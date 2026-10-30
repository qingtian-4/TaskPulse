"""Real installed N→N+1 acceptance. Only disposable hosted CI may build/install.

The source overlay changes transport, signing identity, version and a native test
driver, never the updater's verification/install/restart implementation. No test
binary or private key is exported. Published-channel and OS trust UI are separate.
"""
import argparse
import ctypes
import difflib
import hashlib
import http.server
import json
import os
from pathlib import Path
import platform
import re
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import uuid

REPO = Path(__file__).resolve().parent.parent


def require_ci():
    if (os.environ.get("GITHUB_ACTIONS") != "true"
            or os.environ.get("RUNNER_ENVIRONMENT") != "github-hosted"
            or platform.system() not in ("Windows", "Darwin")):
        raise RuntimeError("Only disposable GitHub-hosted Windows/macOS runners may install")
    if os.environ.get("TASKPULSE_HOME"):
        raise RuntimeError("Must exercise installed-app guards, not TASKPULSE_HOME override")


def run(*args, cwd=None, env=None, timeout=180, capture=True):
    return subprocess.run(list(map(str, args)), cwd=cwd, env=env, check=True,
                          capture_output=capture, timeout=timeout)


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def save(path, value):
    path = Path(path)
    temp = path.with_name(path.name + ".tmp")
    temp.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
    os.replace(temp, path)


def replace_once(path, old, new):
    text = path.read_text(encoding="utf-8")
    if text.count(old) != 1:
        raise RuntimeError(f"Source contract changed: {path.name}: {old[:60]}")
    path.write_text(text.replace(old, new), encoding="utf-8")


def next_version(old):
    match = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)-alpha\.(\d+)", old)
    if not match:
        raise ValueError("Review the upgrade pair when the release version scheme changes")
    return ".".join(match.groups()[:3]) + "-alpha." + str(int(match[4]) + 1)


def overlay(source, public, endpoint, version, original_version, msi_log=None):
    config_path = source / "src-tauri/tauri.conf.json"
    config = json.loads(config_path.read_text(encoding="utf-8"))
    config["version"] = version
    config["bundle"]["windows"]["wix"]["version"] = version.split("-")[0] + "." + str(1000 + int(version.split(".")[-1]))
    config["bundle"]["createUpdaterArtifacts"] = True
    config["plugins"]["updater"].update(pubkey=public, endpoints=[endpoint], dangerousInsecureTransportProtocol=True)
    if msi_log is not None:
        config["plugins"]["updater"]["windows"]["installerArgs"] = ["/L*vx!", '"' + str(msi_log) + '"']
    config_path.write_text(json.dumps(config, indent=2) + "\n")
    package = source / "package.json"
    data = json.loads(package.read_text(encoding="utf-8"))
    data["version"] = version
    package.write_text(json.dumps(data, indent=2) + "\n")
    replace_once(source / "src-tauri/Cargo.toml", f'version     = "{original_version}"', f'version     = "{version}"')
    replace_once(source / "src-tauri/Cargo.lock", f'name = "agent-tray"\nversion = "{original_version}"', f'name = "agent-tray"\nversion = "{version}"')


def windows_payload_exe(package, output, kind):
    output.mkdir()
    if kind == "nsis":
        sevenzip = Path(os.environ["ProgramFiles"]) / "7-Zip/7z.exe"
        run(sevenzip, "x", package, "-o" + str(output), "-y")
    else:
        run_msi(["/a", str(package), "/qn"], {"TARGETDIR": str(output)})
    executables = list(output.rglob("agent-tray.exe"))
    if len(executables) != 1:
        raise RuntimeError("Expected one actual packaged executable")
    return executables[0]


def msi_command(arguments, properties):
    # Windows Installer properties quote the VALUE, not the entire NAME=value.
    # Python's list2cmdline quotes the whole token when a directory has spaces.
    for key, value in properties.items():
        if not re.fullmatch("[A-Z_]+", key) or any(c in value for c in ('"', '\n', '\r')):
            raise ValueError("Invalid MSI property")
    return subprocess.list2cmdline(["msiexec.exe", *arguments]) + "".join(
        f' {key}="{value}"' for key, value in properties.items())


def run_msi(arguments, properties):
    subprocess.run(msi_command(arguments, properties), check=True, timeout=180,
                   stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def installed_products():
    import winreg
    products = []
    for hive_name, hive in (("user", winreg.HKEY_CURRENT_USER), ("machine", winreg.HKEY_LOCAL_MACHINE)):
        # HKCU Software is shared between WOW64 views; enumerate it exactly once.
        for view in ((0,) if hive_name == "user" else (winreg.KEY_WOW64_64KEY, winreg.KEY_WOW64_32KEY)):
            try:
                parent = winreg.OpenKey(hive, r"Software\Microsoft\Windows\CurrentVersion\Uninstall", 0, winreg.KEY_READ | view)
            except FileNotFoundError:
                continue
            with parent:
                for index in range(winreg.QueryInfoKey(parent)[0]):
                    name = winreg.EnumKey(parent, index)
                    with winreg.OpenKey(parent, name, 0, winreg.KEY_READ | view) as child:
                        def value(key):
                            try:
                                return winreg.QueryValueEx(child, key)[0]
                            except FileNotFoundError:
                                return None
                        if value("DisplayName") == "TaskPulse":
                            products.append(dict(hive=hive_name, view=view, key=name,
                                                 version=value("DisplayVersion"), installer=value("WindowsInstaller")))
    return products


def owned_plan(plan):
    info = json.loads(plan.read_text(encoding="utf-8"))
    root = Path(info["root"]).resolve()
    runner = Path(os.environ["RUNNER_TEMP"]).resolve()
    if not root.is_relative_to(runner) or root == runner or not root.name.startswith("taskpulse-upgrade-"):
        raise RuntimeError("Upgrade plan is outside its CI-owned temporary root")
    marker = json.loads((root / "ci-owned.json").read_text(encoding="utf-8"))
    if marker.get("commit") != info["commit"] or plan.resolve() != root / "plan.json":
        raise RuntimeError("Upgrade plan ownership mismatch")
    if "installed_exe" in info and not Path(info["installed_exe"]).resolve().is_relative_to(root):
        raise RuntimeError("Installed executable escapes CI-owned root")
    return info


def prepare(kind, target, evidence):
    require_ci()
    evidence.mkdir(parents=True, exist_ok=True)
    root = Path(tempfile.mkdtemp(prefix="taskpulse-upgrade-", dir=os.environ["RUNNER_TEMP"]))
    if platform.system() == "Windows":
        # The hosted runner creates this root elevated. MSI's SYSTEM service
        # otherwise inherits administrator-only access in its output directory.
        # Grant only this CI account on this newly owned tree, before installing.
        account = run("whoami.exe").stdout.decode().strip()
        run("icacls.exe", root, "/grant", account + ":(OI)(CI)M")
        (evidence / "owned-root-acl.txt").write_bytes(run("icacls.exe", root).stdout)
    source = root / "source"
    source.mkdir()
    # Copy only git-listed source files. Secrets, caches and build products cannot enter.
    tracked = run("git", "ls-files", "-z", cwd=REPO).stdout.decode().split("\0")
    hashes = {}
    for name in filter(None, tracked):
        src = REPO / name
        if src.is_file():
            hashes[name] = digest(src)
            dst = source / name
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(src, dst)
    save(evidence / "source-hashes.json", hashes)
    if platform.system() == "Windows":
        run("cmd", "/c", "mklink", "/J", source / "node_modules", REPO / "node_modules")
    else:
        (source / "node_modules").symlink_to(REPO / "node_modules", target_is_directory=True)
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    route = uuid.uuid4().hex
    endpoint = f"http://127.0.0.1:{port}/{route}/latest.json"
    key = root / "ephemeral.key"
    # The signer prints its key: never forward stdout/stderr, even on failure.
    result = subprocess.run(["bun", "run", "tauri", "signer", "generate", "-w", str(key), "--ci"],
                            cwd=REPO, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=60)
    if result.returncode:
        raise RuntimeError("Ephemeral signing-key generation failed")
    key.chmod(0o600)
    public = key.with_suffix(".key.pub").read_text(encoding="utf-8").strip()
    updates = source / "src-tauri/src/updates.rs"
    replace_once(updates, 'url.scheme() == "https"', 'url.scheme() == "http"')
    replace_once(updates, 'url.host_str() == Some("github.com")', 'url.host_str() == Some("127.0.0.1")')
    replace_once(updates, 'url.port().is_none()', f'url.port() == Some({port})')
    replace_once(updates, '/qingtian-4/TaskPulse/releases/download/v', f'/{route}/v')
    main = source / "src-tauri/src/main.rs"
    replace_once(main, 'mod updates;', 'mod updates;\nmod upgrade_probe;')
    replace_once(main, 'fn main() {', 'fn main() {\n    upgrade_probe::record_entry("main");')
    replace_once(main, 'Ok(None) => return,', 'Ok(None) => { upgrade_probe::record_entry("lock-busy"); return; },')
    replace_once(main, 'updates::start(app.handle().clone());', 'updates::start(app.handle().clone());\n            upgrade_probe::start(app.handle().clone());')
    shutil.copy2(REPO / "scripts/upgrade-probe.rs", source / "src-tauri/src/upgrade_probe.rs")
    replace_once(source / "src-tauri/src/upgrade_probe.rs", 'const DIAGNOSTIC_ROOT: &str = "";',
                 'const DIAGNOSTIC_ROOT: &str = ' + json.dumps(str(root), ensure_ascii=False) + ';')
    shutil.copy2(source / "src-tauri/src/upgrade_probe.rs", evidence / "upgrade-probe.rs.txt")
    old = json.loads((REPO / "package.json").read_text(encoding="utf-8"))["version"]
    new = next_version(old)
    info = dict(root=str(root), source=str(source), kind=kind, target=target, port=port,
                route=route, endpoint=endpoint, old=old, new=new,
                commit=run("git", "rev-parse", "HEAD", cwd=REPO).stdout.decode().strip(),
                private_key=str(key), evidence=str(evidence.resolve()))
    env = dict(os.environ, TAURI_SIGNING_PRIVATE_KEY=str(key), TAURI_SIGNING_PRIVATE_KEY_PASSWORD="",
               CARGO_TARGET_DIR=str(REPO / "src-tauri/target"), APPLE_SIGNING_IDENTITY="-")
    env.pop("TAURI_CONFIG", None)
    for label, version, previous in [("old", old, old), ("new", new, old)]:
        overlay(source, public, endpoint, version, previous,
                evidence.resolve() / "msi-update.log" if kind == "msi" else None)
        # Save exact overlays as reviewable evidence; public key only.
        changed = []
        allowed = {"package.json", "src-tauri/Cargo.toml", "src-tauri/Cargo.lock", "src-tauri/tauri.conf.json", "src-tauri/src/main.rs", "src-tauri/src/updates.rs"}
        for name, original_hash in hashes.items():
            path = source / name
            if digest(path) != original_hash:
                if name not in allowed:
                    raise RuntimeError(f"Unexpected source overlay: {name}")
                changed.extend(difflib.unified_diff((REPO / name).read_text(encoding="utf-8").splitlines(True), path.read_text(encoding="utf-8").splitlines(True), fromfile=name, tofile=name))
        (evidence / f"{label}-overlay.diff").write_text("".join(changed), encoding="utf-8")
        bundles = kind if kind in ("nsis", "msi") else "app,dmg"
        run("bun", "run", "tauri", "build", "--target", target, "--features", "custom-protocol",
            "--bundles", bundles, "--", "--profile", "ci-release", "--locked", cwd=source, env=env,
            timeout=2400, capture=False)
        bundle = REPO / "src-tauri/target" / target / "ci-release/bundle"
        output = root / label
        output.mkdir()
        if kind == "macos":
            candidates = list((bundle / "macos").glob("*.app.tar.gz"))
            dmgs = list((bundle / "dmg").glob(f"*{version}*.dmg"))
            if len(dmgs) != 1:
                raise RuntimeError("Expected one version-matched DMG")
            shutil.copy2(dmgs[0], output / "TaskPulse.dmg")
            app = bundle / "macos/TaskPulse.app"
            info[label + "_exe_sha256"] = digest(app / "Contents/MacOS/agent-tray")
        else:
            suffix = "exe" if kind == "nsis" else "msi"
            candidates = list((bundle / kind).glob(f"*{version}*.{suffix}"))
        if len(candidates) != 1:
            raise RuntimeError(f"Expected exactly one {label} updater payload: {candidates}")
        payload = output / ("TaskPulse_" + version + (".app.tar.gz" if kind == "macos" else "." + suffix))
        shutil.copy2(candidates[0], payload)
        shutil.copy2(str(candidates[0]) + ".sig", str(payload) + ".sig")
        if kind != "macos":
            info[label + "_exe_sha256"] = digest(windows_payload_exe(payload, output / "extracted", kind))
        if kind == "msi":
            metadata = run("powershell.exe", "-NoProfile", "-Command", r'''
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[System.Text.UTF8Encoding]::new()
$installer=New-Object -ComObject WindowsInstaller.Installer
$database=$installer.OpenDatabase($env:TASKPULSE_MSI_PACKAGE,0)
$properties=@{}
foreach($name in @('ProductCode','UpgradeCode','ProductVersion','ProductLanguage','ALLUSERS')){
  $view=$database.OpenView("SELECT Value FROM Property WHERE Property='$name'")
  $view.Execute();$row=$view.Fetch()
  if($row){$properties[$name]=$row.StringData(1)}
  $view.Close()
}
$upgrades=@()
$view=$database.OpenView('SELECT UpgradeCode,VersionMin,VersionMax,Attributes,ActionProperty FROM Upgrade')
$view.Execute()
while($row=$view.Fetch()){
  $upgrades+=@{code=$row.StringData(1);min=$row.StringData(2);max=$row.StringData(3);attributes=$row.IntegerData(4);property=$row.StringData(5)}
}
$view.Close()
@{properties=$properties;upgrade_table=$upgrades}|ConvertTo-Json -Depth 5
''', env=dict(os.environ, TASKPULSE_MSI_PACKAGE=str(payload)))
            (evidence / f"{label}-msi-metadata.json").write_bytes(metadata.stdout)
        info[label + "_payload"] = str(payload)
        info[label + "_sha256"] = digest(payload)
    if any(digest(REPO / name) != value for name, value in hashes.items()):
        raise RuntimeError("Production checkout was modified during fixture build")
    # Negative cases use independently signed bytes; do not expose private keys.
    bad = root / "invalid-package.bin"
    bad.write_bytes(b"This is a signed but invalid installer/archive.\n")
    wrong_key = root / "wrong.key"
    subprocess.run(["bun", "run", "tauri", "signer", "generate", "-w", str(wrong_key), "--ci"], cwd=REPO,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True, timeout=60)
    signatures = {}
    for scenario, file, signer, version in [("invalid-package", bad, key, new),
                                           ("wrong-key", Path(info["new_payload"]), wrong_key, new),
                                           ("wrong-version", Path(info["new_payload"]), key, old)]:
        # Sign a copy so the good detached signature remains unchanged.
        copy = root / (scenario + ".bin")
        if file != copy:
            shutil.copy2(file, copy)
        run("bun", "run", "tauri", "signer", "sign", "-f", signer, "--app-version", version, copy, cwd=REPO)
        signatures[scenario] = Path(str(copy) + ".sig").read_text(encoding="utf-8").strip()
    info["negative_signatures"] = signatures
    save(root / "ci-owned.json", {"purpose": "TaskPulse isolated installed upgrade acceptance", "commit": info["commit"]})
    save(root / "plan.json", info)
    # Upload manifest deliberately excludes local key paths/signatures and generated binaries.
    save(evidence / "build.json", {k: v for k, v in info.items() if k in ("kind", "target", "old", "new", "commit", "old_sha256", "new_sha256", "old_exe_sha256", "new_exe_sha256")})
    return root / "plan.json"


def seed_profile():
    state = Path.home() / ".taskpulse"
    if state.exists():
        raise RuntimeError("Refusing to replace an existing TaskPulse profile")
    state.mkdir()
    config = ('custom_upgrade_marker = "preserve-unknown-field"\n[dashboard]\nversion = 2\n'
              'onboarding_complete = true\nauto_discover_local = false\nauto_check_updates = false\n'
              'notifications = false\nsound = false\nrecent_days = 0\n'
              '[[dashboard.sources]]\nid = "hermes"\nkind = "hermes"\n'
              'name = "Disabled fixture"\nenabled = false\nprojects = ["fixture-project"]\n'
              '[[dashboard.sources]]\nid = "codex"\nkind = "codex"\n'
              'name = "Retained fixture"\nenabled = true\nprojects = ["fixture-project"]\n')
    (state / "config.toml").write_text(config, encoding="utf-8")
    now = int(time.time() * 1000)
    task = dict(id="upgrade-fixture:completed", name="Synthetic upgrade state", source="codex",
                source_instance="codex", status="succeeded", confidence="exact",
                turn_key="upgrade-turn", updated_at=now, finished_at=now, project="fixture-project",
                unread=True, pinned=True)
    save(state / "attention.json", dict(version=4, baselines={}, records={task["id"]: dict(
        task=task, unread=True, pinned=True, pin_override=True, last_execution_at=now, followed_at=now,
        last_turn_key=task["turn_key"], last_status="succeeded")}))
    save(state / "recent.json", dict(version=1, tasks=[task]))
    save(state / "notifications.json", dict(version=1, prev={task["id"]: "succeeded"},
         prev_turns={task["id"]: task["turn_key"]}, prev_hosts={},
         notified=[[task["id"], "Succeeded"]], input_notifications=[],
         integration_delivered=["upgrade-fixture-delivered"]))
    (state / "upgrade-marker.txt").write_text("preserve-user-data\n")
    return state, config


def assert_state(state, config):
    if (state / "config.toml").read_text(encoding="utf-8") != config:
        raise AssertionError("Source configuration or unknown fields changed")
    record = json.loads((state / "attention.json").read_text(encoding="utf-8"))["records"]["upgrade-fixture:completed"]
    assert record["unread"] and record["pinned"] and record["pin_override"]
    assert record["task"]["id"] == "upgrade-fixture:completed"
    ledger = json.loads((state / "notifications.json").read_text(encoding="utf-8"))
    assert "upgrade-fixture-delivered" in ledger["integration_delivered"]
    assert ["upgrade-fixture:completed", "Succeeded"] in ledger["notified"]
    assert (state / "upgrade-marker.txt").read_text(encoding="utf-8") == "preserve-user-data\n"


class Feed(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        shared = self.server.shared
        info = shared["info"]
        with shared["lock"]:
            scenario = shared["scenario"]
            shared["requests"].append({"path": self.path, "scenario": scenario})
        base = "/" + info["route"]
        path = base + "/v" + info["new"] + "/" + Path(info["new_payload"]).name
        if self.path == base + "/latest.json":
            signature = info["negative_signatures"].get(scenario, Path(info["new_payload"] + ".sig").read_text(encoding="utf-8").strip())
            item = {"signature": signature, "url": f"http://127.0.0.1:{info['port']}{path}"}
            platforms = {key: item for key in ("windows-x86_64-nsis", "windows-x86_64-msi", "darwin-aarch64", "darwin-x86_64")}
            body = json.dumps(dict(version=info["new"], notes="Isolated upgrade acceptance", platforms=platforms)).encode()
        elif self.path == path:
            body = (Path(info["root"]) / "invalid-package.bin").read_bytes() if scenario == "invalid-package" else Path(info["new_payload"]).read_bytes()
            if scenario == "tampered":
                body += b"tampered"
            if scenario == "interrupted":
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body[:64])
                self.wfile.flush()
                self.close_connection = True
                return
        else:
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def wait_for(predicate, timeout=90):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(.2)
    raise TimeoutError("Timed out waiting for native upgrade evidence")


def automatic_boot(root, version):
    records = [json.loads(path.read_text(encoding="utf-8")) for path in root.glob(f"boot-{version}-*.json")]
    automatic = [record for record in records if not record.get("diagnostic_launch", False)]
    if len(automatic) > 1:
        raise RuntimeError("Multiple automatic boots; refusing ambiguous restart evidence")
    return automatic[0] if automatic else None


class NativeProcess:
    """Hold Windows process identity; on macOS verify PID/start/path before signalling."""
    def __init__(self, pid, exe):
        self.pid, self.exe = pid, Path(exe).resolve()
        if platform.system() == "Windows":
            from ctypes import wintypes
            self.kernel = ctypes.WinDLL("kernel32", use_last_error=True)
            self.kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
            self.kernel.OpenProcess.restype = wintypes.HANDLE
            self.kernel.QueryFullProcessImageNameW.argtypes = [wintypes.HANDLE, wintypes.DWORD, wintypes.LPWSTR, ctypes.POINTER(wintypes.DWORD)]
            self.kernel.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
            self.kernel.TerminateProcess.argtypes = [wintypes.HANDLE, wintypes.UINT]
            self.kernel.CloseHandle.argtypes = [wintypes.HANDLE]
            self.handle = self.kernel.OpenProcess(0x100000 | 0x1000 | 1, False, pid)
            if not self.handle:
                raise OSError(ctypes.get_last_error(), "Cannot retain restarted process handle")
            name, length = ctypes.create_unicode_buffer(32768), wintypes.DWORD(32768)
            if not self.kernel.QueryFullProcessImageNameW(self.handle, 0, name, ctypes.byref(length)):
                raise OSError(ctypes.get_last_error(), "Cannot verify restarted process executable")
            if not os.path.samefile(name.value, self.exe):
                raise RuntimeError("Restarted process executable identity mismatch")
        else:
            self.start = self.identity()
            if not self.start:
                raise RuntimeError("Restarted native process already exited")

    def identity(self):
        proc = subprocess.run(["ps", "-p", str(self.pid), "-o", "lstart=", "-o", "comm="], capture_output=True, text=True)
        if proc.returncode:
            return None
        # macOS comm is the full executable path, including spaces.
        output = proc.stdout.strip()
        if not output.endswith(str(self.exe)):
            raise RuntimeError("Refusing to signal unrelated restarted process")
        return output

    def exited(self):
        if platform.system() == "Windows":
            result = self.kernel.WaitForSingleObject(self.handle, 0)
            if result not in (0, 258):
                raise OSError("Cannot wait for restarted process")
            return result == 0
        found = self.identity()
        if found is not None and found != self.start:
            raise RuntimeError("Restarted PID was reused")
        return found is None

    def close(self):
        try:
            if not self.exited():
                if platform.system() == "Windows":
                    if not self.kernel.TerminateProcess(self.handle, 1):
                        raise OSError("Cannot stop owned restarted process")
                else:
                    os.kill(self.pid, signal.SIGTERM)
                wait_for(self.exited, timeout=15)
        finally:
            if platform.system() == "Windows":
                self.kernel.CloseHandle(self.handle)


def windows_dialogs(pids):
    from ctypes import wintypes
    user = ctypes.WinDLL("user32", use_last_error=True)
    callback = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    user.EnumWindows.argtypes = [callback, wintypes.LPARAM]
    user.EnumChildWindows.argtypes = [wintypes.HWND, callback, wintypes.LPARAM]
    user.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
    user.GetWindowTextW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
    user.IsWindowVisible.argtypes = [wintypes.HWND]
    result = []
    def title(hwnd):
        buffer = ctypes.create_unicode_buffer(2048)
        user.GetWindowTextW(hwnd, buffer, len(buffer))
        return buffer.value
    @callback
    def window(hwnd, _):
        pid = wintypes.DWORD()
        user.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
        if pid.value in pids and user.IsWindowVisible(hwnd):
            children = []
            @callback
            def child(control, _):
                text = title(control)
                if text:
                    children.append(text)
                return True
            user.EnumChildWindows(hwnd, child, 0)
            result.append(dict(pid=pid.value, title=title(hwnd), text=children))
        return True
    user.EnumWindows(window, 0)
    return result


def diagnostic_shell_launch(exe, arguments):
    """Diagnostic-only ShellExecute with a retained handle; never counts as restart."""
    from ctypes import wintypes
    class ShellInfo(ctypes.Structure):
        _fields_ = [("cbSize", wintypes.DWORD), ("fMask", wintypes.ULONG), ("hwnd", wintypes.HWND),
                    ("lpVerb", wintypes.LPCWSTR), ("lpFile", wintypes.LPCWSTR),
                    ("lpParameters", wintypes.LPCWSTR), ("lpDirectory", wintypes.LPCWSTR),
                    ("nShow", ctypes.c_int), ("hInstApp", wintypes.HINSTANCE), ("lpIDList", ctypes.c_void_p),
                    ("lpClass", wintypes.LPCWSTR), ("hkeyClass", wintypes.HKEY), ("dwHotKey", wintypes.DWORD),
                    ("hIcon", wintypes.HANDLE), ("hProcess", wintypes.HANDLE)]
    shell = ctypes.WinDLL("shell32", use_last_error=True)
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    shell.ShellExecuteExW.argtypes = [ctypes.POINTER(ShellInfo)]
    kernel.GetProcessId.argtypes = [wintypes.HANDLE]
    kernel.GetProcessId.restype = wintypes.DWORD
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    info = ShellInfo(cbSize=ctypes.sizeof(ShellInfo), fMask=0x40, lpVerb="open", lpFile=str(exe),
                     lpParameters=subprocess.list2cmdline(arguments), nShow=1)
    if not shell.ShellExecuteExW(ctypes.byref(info)):
        raise OSError(ctypes.get_last_error(), "Diagnostic ShellExecute failed")
    if not info.hProcess:
        raise RuntimeError("Diagnostic ShellExecute returned no process handle")
    try:
        return kernel.GetProcessId(info.hProcess)
    finally:
        kernel.CloseHandle(info.hProcess)


def diagnostic_nsis_launch(root, exe, arguments, evidence):
    """Isolate the exact official launch plugin; performs no installation."""
    tools = Path(os.environ["LOCALAPPDATA"]) / "tauri/NSIS"
    compiler = tools / "makensis.exe"
    plugin = tools / "Plugins/x86-unicode/additional/nsis_tauri_utils.dll"
    # SHA-256 of the files in Tauri's checksum-verified NSIS 3.13 archive
    # and its checksum-verified nsis_tauri_utils 0.5.3 release.
    expected = {compiler: "4ec390d0f96f9728507fbc2f4e91311dd90bfc6c8ddfe2159357b3651ae151c0",
                tools / "Bin/makensis.exe": "65343c225ec88393cc4453b0963caf63f763f42f48cb55736d51893ff2e29d4a",
                plugin: "5ba143b5db4a87d32d6e7802e033330aae56cbceabe0d1e3ba41948385ad4709"}
    for file, checksum in expected.items():
        if digest(file) != checksum:
            raise RuntimeError("NSIS diagnostic tool differs from Tauri CLI's pinned tool")
    quote = lambda value: '"' + str(value).replace('$', '$$').replace('"', '$\\"') + '"'
    log = evidence / "nsis-launch-plugin.txt"
    output = root / "launch-plugin-probe.exe"
    text = f'''Unicode true
RequestExecutionLevel user
SilentInstall silent
Name "TaskPulse launch diagnostic"
OutFile {quote(output)}
!addplugindir /x86-unicode {quote(plugin.parent)}
Section
  FileOpen $9 {quote(log)} w
  System::Call 'shell32::IsUserAnAdmin() i .r0'
  FileWrite $9 "admin=$0$\\r$\\n"
  System::Call 'kernel32::GetCurrentProcess() p .r0'
  System::Call 'advapi32::OpenProcessToken(p r0, i 8, *p .r1) i .r2'
  FileWrite $9 "open_token=$2$\\r$\\n"
  System::Call 'advapi32::GetTokenInformation(p r1, i 20, *i .r2, i 4, *i .r3) i .r4'
  FileWrite $9 "elevation_query=$4;token_elevated=$2$\\r$\\n"
  System::Call 'advapi32::GetTokenInformation(p r1, i 18, *i .r2, i 4, *i .r3) i .r4'
  FileWrite $9 "elevation_type_query=$4;elevation_type=$2$\\r$\\n"
  System::Call 'advapi32::GetTokenInformation(p r1, i 12, *i .r2, i 4, *i .r3) i .r4'
  FileWrite $9 "session_query=$4;session=$2$\\r$\\n"
  System::Call 'kernel32::CloseHandle(p r1)'
  System::Call 'user32::GetShellWindow() p .r5'
  FileWrite $9 "shell_window=$5$\\r$\\n"
  FileClose $9
  nsis_tauri_utils::RunAsUser {quote(exe)} {quote(subprocess.list2cmdline(arguments))}
  Pop $8
  FileOpen $9 {quote(log)} a
  FileWrite $9 "plugin_result=$8$\\r$\\n"
  FileClose $9
SectionEnd
'''
    script = root / "launch-plugin-probe.nsi"
    script.write_text(text, encoding="utf-8")
    run(compiler, "-INPUTCHARSET", "UTF8", "-V2", script)
    run(output, timeout=30)
    # NSIS FileWrite uses the current ANSI page; this report is ASCII only.
    if "plugin_result=0" not in log.read_text(encoding="ascii", errors="replace"):
        raise RuntimeError("Official NSIS RunAsUser reported launch failure; see token evidence")


def supervise(plan):
    require_ci()
    info = owned_plan(plan)
    root, evidence, exe = Path(info["root"]), Path(info["evidence"]), Path(info["installed_exe"])
    state, config = seed_profile()
    result = {"passed": False, "checks": [], "kind": info["kind"], "source_commit": info["commit"],
              "old": info["old"], "new": info["new"], "os": platform.platform(),
              "transport": "controlled-loopback", "signing": "ephemeral", "driver": "native management commands",
              "historical_release_migration": False, "production_feed_verified": False,
              "os_trust_prompts_verified": False, "mid_replacement_rollback_verified": False}
    shared = dict(info=info, scenario="good", requests=[], lock=threading.Lock())
    server = http.server.ThreadingHTTPServer(("127.0.0.1", info["port"]), Feed)
    server.shared = shared
    threading.Thread(target=server.serve_forever, daemon=True).start()
    serial = 0
    child = None
    restarted = None

    def command(action, version="", wait=True):
        nonlocal serial
        serial += 1
        save(root / "control.json", dict(id=serial, action=action, version=version))
        if wait:
            path = root / f"response-{serial}.json"
            wait_for(path.exists)
            response = json.loads(path.read_text(encoding="utf-8"))
            save(evidence / f"response-{serial}-{action}.json", response)
            return response

    def boot(version):
        return automatic_boot(root, version)

    try:
        old_hash = digest(exe)
        assert old_hash == info["old_exe_sha256"], "Installed old payload differs from bundled executable"
        result["old_exe_sha256"] = old_hash
        stdout = (evidence / "native.stdout.log").open("wb")
        stderr = (evidence / "native.stderr.log").open("wb")
        child = subprocess.Popen([str(exe), "--show", "--upgrade-probe-root", str(root)], stdout=stdout, stderr=stderr)
        initial = wait_for(lambda: boot(info["old"]))
        save(evidence / "old-boot.json", initial)
        assert initial["status"]["can_install"] and not initial["elevated"]
        assert os.path.samefile(initial["exe"], exe)
        if info["kind"] != "macos":
            assert info["kind"].lower() in initial["bundle"].lower()
            old_products = installed_products()
            assert len(old_products) == 1, old_products
            save(evidence / "old-products.json", old_products)
        assert_state(state, config)
        for case in ("wrong-key", "wrong-version", "tampered", "interrupted", "invalid-package"):
            shared["scenario"] = case
            count = sum("/latest.json" not in r["path"] for r in shared["requests"])
            checked = command("check")
            assert checked["status"]["phase"] == "available", checked
            retained = [task for task in checked["dashboard"]["tasks"] if task["id"] == "upgrade-fixture:completed"]
            assert len(retained) == 1 and retained[0]["unread"] and retained[0]["pinned"], checked["dashboard"]
            time.sleep(.5)
            assert sum("/latest.json" not in r["path"] for r in shared["requests"]) == count
            if case == "wrong-key":
                assert not command("popup-denied", info["new"])["ok"]
                assert not command("install", "0.0.0")["ok"]
                assert sum("/latest.json" not in r["path"] for r in shared["requests"]) == count
            events_file = root / "events.jsonl"
            event_offset = len(events_file.read_text(encoding="utf-8").splitlines())
            failed = command("install", info["new"])
            assert not failed["ok"] and failed["status"]["phase"] == "error", failed
            phases = [json.loads(line)["phase"] for line in events_file.read_text(encoding="utf-8").splitlines()[event_offset:]]
            assert "downloading" in phases and "error" in phases and failed["status"]["downloaded"] > 0
            assert sum("/latest.json" not in r["path"] for r in shared["requests"]) == count + 1
            if case == "interrupted":
                assert "verifying" not in phases and "installing" not in phases
            else:
                assert "verifying" in phases
                assert ("installing" in phases) == (case == "invalid-package")
            assert child.poll() is None and digest(exe) == old_hash
            assert_state(state, config)
            result["checks"].append(case + ": rejected, old app alive/unchanged, state retained")
        shared["scenario"] = "good"
        assert command("check")["status"]["phase"] == "available"
        command("install", info["new"], wait=False)
        updated = wait_for(lambda: boot(info["new"]), timeout=180)
        save(evidence / "new-boot.json", updated)
        assert updated["pid"] != initial["pid"] and not updated["elevated"]
        assert updated["status"]["can_install"] and updated["bundle"] == initial["bundle"]
        assert os.path.samefile(updated["exe"], exe)
        restarted = NativeProcess(updated["pid"], exe)
        assert child.wait(timeout=15) == 0
        assert digest(exe) != old_hash and digest(exe) == info["new_exe_sha256"]
        if info["kind"] == "macos":
            assert digest(exe) == info["new_exe_sha256"]
            run("codesign", "--verify", "--deep", "--strict", exe.parents[2])
        else:
            products = installed_products()
            save(evidence / "new-products.json", products)
            assert len(products) == 1 and products[0]["version"] != old_products[0]["version"], products
            if info["kind"] == "msi":
                assert products[0]["installer"] == 1 and products[0]["key"] != old_products[0]["key"]
            result["checks"].append("one updated product registration; no old MSI product retained")
        actual = run(exe, "--version").stdout.decode().strip()
        assert actual == "TaskPulse " + info["new"], actual
        assert_state(state, config)
        result["new_exe_sha256"] = digest(exe)
        result["checks"].extend(["check-only did not download", "popup/stale version rejected",
                                "real installed N→N+1 replacement", "same path and bundle format",
                                "automatic restart without elevation", "settings/unread/pin/dedup preserved"])
        # A second check must see current; no repeated install after automatic restart.
        current = command("check")
        assert current["status"]["phase"] == "current" and current["pid"] == updated["pid"], current
        assert current["status"]["current_version"] == info["new"]
        retained = [task for task in current["dashboard"]["tasks"] if task["id"] == "upgrade-fixture:completed"]
        assert len(retained) == 1 and retained[0]["unread"] and retained[0]["pinned"]
        command("exit", wait=False)
        wait_for(restarted.exited, timeout=20)
        result["checks"].append("new-process commands and clean shutdown verified")
        hashes = json.loads((evidence / "source-hashes.json").read_text(encoding="utf-8"))
        assert all(digest(REPO / name) == value for name, value in hashes.items())
        result["checks"].append("production checkout unchanged")
        result["passed"] = True
    except Exception as error:
        result["error"] = repr(error)
        result["old_process_exit"] = child.poll() if child else None
        try:
            result["installed_exe_sha256_after_failure"] = digest(exe) if exe.is_file() else None
        except OSError as probe_error:
            result["installed_hash_probe_error"] = str(probe_error)
        try:
            result["installed_version_after_failure"] = run(exe, "--version", timeout=15).stdout.decode().strip()
        except Exception as probe_error:
            result["installed_version_probe_error"] = str(probe_error)
        if platform.system() == "Windows":
            save(evidence / "failure-products.json", installed_products())
            diagnostic = run("powershell.exe", "-NoProfile", "-Command",
                "[Console]::OutputEncoding=[System.Text.UTF8Encoding]::new(); Get-CimInstance Win32_Process | Where-Object { $_.Name -match 'agent-tray|msiexec|consent|TaskPulse.*installer|TaskPulse.*setup' } | Select-Object ProcessId,ParentProcessId,Name,ExecutablePath,CommandLine | ConvertTo-Json -Depth 3")
            (evidence / "failure-processes.json").write_bytes(diagnostic.stdout)
            processes = json.loads(diagnostic.stdout.decode("utf-8-sig") or "[]")
            if isinstance(processes, dict):
                processes = [processes]
            save(evidence / "failure-dialogs.json", windows_dialogs({p["ProcessId"] for p in processes}))
            # Preserve the original FAILED verdict. These manual launches only
            # distinguish payload startup from automatic installer relaunch.
            if info["kind"] == "nsis" and result.get("installed_exe_sha256_after_failure") == info["new_exe_sha256"]:
                attempts = []
                for method in ("direct", "shell", "nsis-plugin"):
                    record = {"method": method, "automatic_restart": False}
                    diagnostic_process = None
                    try:
                        arguments = ["--show", "--upgrade-probe-root", str(root), "--diagnostic-launch=" + method]
                        if method == "direct":
                            with (evidence / "diagnostic-direct.stderr.log").open("wb") as errors:
                                launched = subprocess.Popen([str(exe), *arguments], stdout=subprocess.DEVNULL, stderr=errors)
                            pid = launched.pid
                        elif method == "shell":
                            pid = diagnostic_shell_launch(exe, arguments)
                        else:
                            diagnostic_nsis_launch(root, exe, arguments, evidence)
                            def plugin_entry():
                                for path in root.glob(f"entry-{info['new']}-*-main.json"):
                                    value = json.loads(path.read_text(encoding="utf-8"))
                                    if "--diagnostic-launch=nsis-plugin" in value.get("args", []):
                                        return value["pid"]
                                return None
                            pid = wait_for(plugin_entry, timeout=30)
                        record["pid"] = pid
                        diagnostic_process = NativeProcess(pid, exe)
                        observed = root / f"boot-{info['new']}-{pid}.json"
                        wait_for(observed.exists, timeout=30)
                        record["boot_observed"] = True
                        command("exit", wait=False)
                        wait_for(diagnostic_process.exited, timeout=15)
                    except Exception as diagnostic_error:
                        record["error"] = repr(diagnostic_error)
                    finally:
                        if diagnostic_process is not None:
                            diagnostic_process.close()
                        attempts.append(record)
                save(evidence / "manual-launch-diagnostics.json", attempts)
        else:
            diagnostic = run("ps", "ax", "-o", "pid,ppid,stat,command").stdout.decode()
            (evidence / "failure-processes.txt").write_text("\n".join(line for line in diagnostic.splitlines() if "agent-tray" in line or "TaskPulse.app" in line))
        raise
    finally:
        cleanup_errors = []
        try:
            if restarted is None:
                observed = boot(info["new"])
                if observed:
                    restarted = NativeProcess(observed["pid"], exe)
            if restarted is not None:
                restarted.close()
            if child is not None and child.poll() is None:
                command("exit", wait=False)
                try:
                    child.wait(timeout=15)
                except subprocess.TimeoutExpired:
                    # Only the exact Popen-owned child, never a process-name kill.
                    child.terminate()
                    child.wait(timeout=10)
        except Exception as error:
            cleanup_errors.append(repr(error))
        finally:
            server.shutdown()
            server.server_close()
            save(evidence / "requests.json", shared["requests"])
            for path in root.glob("boot-*.json"):
                shutil.copy2(path, evidence / path.name)
            for path in root.glob("entry-*.json"):
                shutil.copy2(path, evidence / path.name)
            for path in root.glob("panic-*.txt"):
                shutil.copy2(path, evidence / path.name)
            if (root / "events.jsonl").exists():
                shutil.copy2(root / "events.jsonl", evidence / "events.jsonl")
            if cleanup_errors:
                result["passed"] = False
                result["cleanup_errors"] = cleanup_errors
            save(evidence / "result.json", result)
        if cleanup_errors:
            raise RuntimeError("Owned process cleanup failed; see result.json")


def install_and_test(plan):
    require_ci()
    info = owned_plan(plan)
    root = Path(info["root"])
    install = root / "Applications" / "测试 install"
    install.mkdir(parents=True)
    if info["kind"] == "macos":
        mount = root / "volume"
        mount.mkdir()
        run("hdiutil", "attach", root / "old/TaskPulse.dmg", "-readonly", "-nobrowse", "-mountpoint", mount)
        try:
            run("ditto", mount / "TaskPulse.app", install / "TaskPulse.app")
        finally:
            run("hdiutil", "detach", mount)
        exe = install / "TaskPulse.app/Contents/MacOS/agent-tray"
        assert digest(exe) == info["old_exe_sha256"]
    else:
        package = info["old_payload"]
        if info["kind"] == "nsis":
            # NSIS /D is a raw, final tail; quoting the whole /D=argument changes parsing.
            subprocess.run(subprocess.list2cmdline([package, "/S"]) + " /D=" + str(install),
                           check=True, timeout=180)
        else:
            run_msi(["/i", package, "/qn", "/norestart", "/L*v",
                     str(Path(info["evidence"]) / "msi-initial.log")], {"INSTALLDIR": str(install)})
        exe = install / "agent-tray.exe"
    assert exe.is_file(), str(exe)
    if info["kind"] != "macos":
        (Path(info["evidence"]) / "installed-acl.txt").write_bytes(run("icacls.exe", exe).stdout)
    info["installed_exe"] = str(exe)
    save(plan, info)
    if info["kind"] == "macos":
        supervise(plan)
    else:
        try:
            run(os.environ["GSUDO_EXE"], "--integrity", "Medium", "--direct", sys.executable,
                Path(__file__).resolve(), "supervise", "--plan", plan, timeout=600, capture=False)
        finally:
            if info["kind"] == "msi":
                # Read from the privileged CI parent: the live app remains medium.
                events = run("powershell.exe", "-NoProfile", "-Command",
                    "[Console]::OutputEncoding=[System.Text.UTF8Encoding]::new(); Get-WinEvent -FilterHashtable @{LogName='Application';ProviderName='MsiInstaller';StartTime=(Get-Date).AddMinutes(-15)} -ErrorAction SilentlyContinue | Select-Object TimeCreated,Id,Message | ConvertTo-Json -Depth 4")
                (Path(info["evidence"]) / "msi-system-events.json").write_bytes(events.stdout)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["build", "test", "supervise"])
    parser.add_argument("--kind", choices=["nsis", "msi", "macos"])
    parser.add_argument("--target")
    parser.add_argument("--plan", type=Path)
    parser.add_argument("--evidence", type=Path, default=REPO / "dist/upgrade-evidence")
    options = parser.parse_args()
    if options.action == "build":
        plan = prepare(options.kind, options.target, options.evidence)
        with open(os.environ["GITHUB_ENV"], "a", encoding="utf-8") as output:
            output.write(f"TASKPULSE_UPGRADE_PLAN={plan}\n")
    elif options.action == "test":
        install_and_test(options.plan)
    else:
        supervise(options.plan)
