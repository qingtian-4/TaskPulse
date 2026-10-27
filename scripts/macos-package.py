"""Stage a traceable DMG; opt-in CI acceptance uses only an owned temporary install."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import re
import shutil
import subprocess
import sys
import tempfile
import time


def stage(bundle_root, output, version, commit, arch):
    if not re.fullmatch(r"\d+\.\d+\.\d+-(alpha|beta|rc)\.\d+", version):
        raise ValueError("An explicit pre-release version is required")
    if not re.fullmatch(r"[a-f0-9]{40}", commit) or arch not in ("arm64", "x64"):
        raise ValueError("Invalid source commit or architecture")
    files = list(bundle_root.glob("*.dmg"))
    if len(files) != 1:
        raise ValueError("Expected exactly one DMG")
    with files[0].open("rb") as image:
        image.seek(0, 2)
        if image.tell() < 512:
            raise ValueError("Invalid DMG trailer")
        image.seek(-512, 2)
        if image.read(4) != b"koly":
            raise ValueError("Invalid DMG trailer")
    if output.exists() and any(output.iterdir()):
        raise ValueError("Output directory must be empty; refusing stale assets")
    output.mkdir(parents=True, exist_ok=True)
    name = f"TaskPulse_{version}_macos_{arch}.dmg"
    destination = output / name
    shutil.copy2(files[0], destination)
    digest = hashlib.sha256(destination.read_bytes()).hexdigest()
    (output / f"macos-{arch}-SHA256SUMS.txt").write_text(f"{digest}  {name}\n", encoding="utf-8")
    (output / f"macos-{arch}-manifest.json").write_text(json.dumps({
        "version": version, "commit": commit, "platform": "macos", "architecture": arch,
        "signing": "ad-hoc", "notarized": False, "assets": [name], "sha256": digest,
        "acceptance": "See separate CI evidence; user desktop acceptance still required",
    }, indent=2) + "\n", encoding="utf-8")
    return destination


def smoke(dmg, version, arch, evidence):
    if platform.system() != "Darwin" or os.environ.get("GITHUB_ACTIONS") != "true":
        raise RuntimeError("Installed-app smoke is restricted to disposable macOS CI runners")
    evidence.mkdir(parents=True, exist_ok=True)
    result = {"passed": False, "checks": [], "os": platform.mac_ver()[0],
              "architecture": platform.machine(), "ui_clicks_verified": False,
              "gatekeeper_download_verified": False, "real_agent_events_verified": False}

    def run(*args, **kwargs):
        return subprocess.run(args, check=True, capture_output=True, timeout=60, **kwargs)

    try:
        with tempfile.TemporaryDirectory(prefix="taskpulse-macos-") as temporary:
            root = Path(temporary)
            probe = root / "window-probe"
            run("xcrun", "swiftc", str(Path(__file__).with_name("macos-window-probe.swift")), "-o", str(probe))
            mount = root / "volume"
            mount.mkdir()
            attached = False
            try:
                run("hdiutil", "attach", str(dmg.resolve()), "-readonly", "-nobrowse", "-mountpoint", str(mount))
                attached = True
                apps = list(mount.glob("*.app"))
                if len(apps) != 1:
                    raise RuntimeError("DMG must contain one application")
                installed = root / "Applications" / "测试 install" / "TaskPulse.app"
                installed.parent.mkdir(parents=True)
                run("ditto", str(apps[0]), str(installed))
                result["checks"].append("DMG mounted read-only and copied to owned install directory")
            finally:
                if attached:
                    run("hdiutil", "detach", str(mount))

            run("codesign", "--verify", "--deep", "--strict", "--verbose=2", str(installed))
            signature = run("codesign", "-dv", "--verbose=4", str(installed)).stderr.decode()
            if "Signature=adhoc" not in signature:
                raise RuntimeError("Expected documented ad-hoc signature")
            (evidence / "signature.txt").write_text(signature, encoding="utf-8")
            info = plistlib.loads((installed / "Contents/Info.plist").read_bytes())
            if info.get("CFBundleIdentifier") != "app.taskpulse.desktop":
                raise RuntimeError("Incorrect bundle identity")
            exe = installed / "Contents/MacOS" / info["CFBundleExecutable"]
            if not exe.resolve().is_relative_to(installed.resolve()):
                raise RuntimeError("Executable escapes installed bundle")
            architectures = run("lipo", "-archs", str(exe)).stdout.decode().strip().split()
            if architectures != ["arm64" if arch == "arm64" else "x86_64"]:
                raise RuntimeError(f"Unexpected executable architecture: {architectures}")
            resources = installed / "Contents/Resources/scripts/hooks"
            if not resources.is_dir() or not any(resources.iterdir()):
                raise RuntimeError("Packaged hook resources missing")
            profile = root / "profile"
            profile.mkdir()
            config = ('[dashboard]\nversion = 2\nonboarding_complete = true\n'
                      'notifications = false\nsound = false\nsources = []\n')
            (profile / "config.toml").write_text(config, encoding="utf-8")
            # Finder-style PATH: successful startup must not depend on shell rc files.
            env = dict(os.environ, TASKPULSE_HOME=str(profile), PATH="/usr/bin:/bin:/usr/sbin:/sbin")
            found = run(str(exe), "--version", env=env).stdout.decode().strip()
            if found != f"TaskPulse {version}":
                raise RuntimeError(f"Version mismatch: {found}")
            result["checks"].extend(["strict ad-hoc signature", "bundle identity and native architecture",
                                     "packaged hook resources", "actual installed executable version"])
            run(sys.executable, str(Path(__file__).with_name("mcp-smoke.py")), "--exe", str(exe),
                "--out", str(evidence / "mcp.json"), env=env)
            result["checks"].append("installed app MCP stdio source binding and lifecycle smoke")
            for cycle in ("first-launch", "restart"):
                with (evidence / f"{cycle}.stdout.txt").open("wb") as out, (evidence / f"{cycle}.stderr.txt").open("wb") as err:
                    child = subprocess.Popen([str(exe), "--show"], env=env, stdout=out, stderr=err)
                    try:
                        deadline = time.monotonic() + 10
                        while time.monotonic() < deadline:
                            if child.poll() is not None:
                                raise RuntimeError(f"{cycle} exited early: {child.returncode}")
                            time.sleep(.2)
                        if not (profile / "gui.lock").is_file():
                            raise RuntimeError("GUI instance did not acquire its isolated lock")
                        windows = run(str(probe), str(child.pid)).stdout
                        (evidence / f"{cycle}.windows.json").write_bytes(windows)
                        # CoreGraphics reports logical points, independently of Retina.
                        if not any(abs(window["width"] - 371) <= 1 and abs(window["height"] - 360) <= 1
                                   for window in json.loads(windows)["windows"]):
                            raise RuntimeError("Empty dashboard did not use the compact macOS size")
                        run(str(exe), "--show", env=env)
                        if not (profile / "gui.activate").is_file() or child.poll() is not None:
                            raise RuntimeError("Second launch did not activate the existing GUI")
                        if (profile / "config.toml").read_text(encoding="utf-8") != config:
                            raise RuntimeError("Startup changed the explicitly empty source configuration")
                        result["checks"].append(f"{cycle}: native window visible, survives 10s, one GUI writer, secondary activation")
                    finally:
                        if child.poll() is None:
                            child.terminate()
                            try:
                                child.wait(timeout=10)
                            except subprocess.TimeoutExpired:
                                child.kill()
                                child.wait(timeout=5)
            result["passed"] = True
    except Exception as error:
        result["error"] = str(error)
        raise
    finally:
        (evidence / "result.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle-root", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--arch", required=True, choices=["arm64", "x64"])
    parser.add_argument("--smoke", action="store_true")
    args = parser.parse_args()
    repo = Path(__file__).resolve().parent.parent
    version = json.loads((repo / "package.json").read_text(encoding="utf-8"))["version"]
    image = stage(args.bundle_root, args.out, version, args.commit, args.arch)
    if args.smoke:
        smoke(image, version, args.arch, args.out.parent / "macos-evidence")
    print(json.dumps({"dmg": str(image), "commit": args.commit, "smoke_passed": args.smoke}))
