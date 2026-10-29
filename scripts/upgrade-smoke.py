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


def overlay(source, public, endpoint, version, original_version):
    config_path = source / "src-tauri/tauri.conf.json"
    config = json.loads(config_path.read_text(encoding="utf-8"))
    config["version"] = version
    config["bundle"]["windows"]["wix"]["version"] = version.split("-")[0] + "." + str(1000 + int(version.split(".")[-1]))
    config["bundle"]["createUpdaterArtifacts"] = True
    config["plugins"]["updater"].update(pubkey=public, endpoints=[endpoint], dangerousInsecureTransportProtocol=True)
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
    shutil.copy2(REPO / "scripts/upgrade-probe.rs", evidence / "upgrade-probe.rs.txt")
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
        overlay(source, public, endpoint, version, previous)
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
        files = list(root.glob(f"boot-{version}-*.json"))
        return json.loads(files[-1].read_text(encoding="utf-8")) if files else None

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
        result["installed_exe_sha256_after_failure"] = digest(exe) if exe.is_file() else None
        try:
            result["installed_version_after_failure"] = run(exe, "--version", timeout=15).stdout.decode().strip()
        except Exception as probe_error:
            result["installed_version_probe_error"] = str(probe_error)
        if platform.system() == "Windows":
            diagnostic = run("powershell.exe", "-NoProfile", "-Command",
                "Get-CimInstance Win32_Process | Where-Object { $_.Name -match 'agent-tray|msiexec|TaskPulse.*installer|TaskPulse.*setup' } | Select-Object ProcessId,ParentProcessId,Name,ExecutablePath,CommandLine | ConvertTo-Json -Depth 3")
            (evidence / "failure-processes.json").write_bytes(diagnostic.stdout)
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
    info["installed_exe"] = str(exe)
    save(plan, info)
    if info["kind"] == "macos":
        supervise(plan)
    else:
        run(os.environ["GSUDO_EXE"], "--integrity", "Medium", "--direct", sys.executable,
            Path(__file__).resolve(), "supervise", "--plan", plan, timeout=600, capture=False)


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
