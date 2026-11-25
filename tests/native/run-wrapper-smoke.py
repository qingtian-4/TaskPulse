"""Linux/WSL isolated foreground-supervisor acceptance, no Agent hooks/config writes."""
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time

repo = Path(__file__).resolve().parents[2]
parent = repo / "src-tauri/target/wrapper-smoke"
parent.mkdir(parents=True, exist_ok=True)
with tempfile.TemporaryDirectory(prefix="linux-", dir=parent) as temporary:
    root = Path(temporary)
    def launch(label, argv, legacy=False, hide_args=False):
        directory = root / label
        directory.mkdir()
        env = dict(os.environ, TASKPULSE_STATE_DIR=str(directory), TASKPULSE_STATUS_DIR=str(directory), TASKPULSE_HEARTBEAT_SECONDS="0.2")
        prefix = ["bash", str(repo / "scripts/wrap.sh")] if legacy else [sys.executable, str(repo / "scripts/tp-run")]
        options = ["--hide-args", "--"] if hide_args else ["--"]
        return subprocess.Popen(prefix + [label] + options + argv, cwd=root, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE), directory

    def state(directory):
        files = list(directory.glob("*.json"))
        assert len(files) == 1, files
        return json.loads(files[0].read_text(encoding="utf-8"))

    private_prompt = "private-prompt-fixture"
    child, directory = launch("private", [sys.executable, "-c",
        "import sys,time;time.sleep(1);sys.exit(7 if sys.argv[1] else 1)", private_prompt],
        legacy=os.name != "nt", hide_args=True)
    for _ in range(100):
        if list(directory.glob("*.status")):
            break
        time.sleep(.01)
    def check_hidden(expected_status):
        run = state(directory)
        local = json.loads(next(directory.glob("*.status")).read_text(encoding="utf-8"))
        assert run["label"] == local["name"] == "private"
        assert run["status"] == local["status"] == expected_status
        assert run["argv"] == [] and run["command"] is None
        assert local["message"] is None and local["activity"] is None
        assert private_prompt not in json.dumps([run, local])
    check_hidden("running")
    _, err = child.communicate(timeout=10)
    assert child.returncode == 7 and not err
    check_hidden("failed")
    assert state(directory)["finished_at_ms"] >= state(directory)["started_at_ms"]
    missing_name = "missing-private-executable-fixture"
    child, directory = launch("private-missing", [missing_name, private_prompt], hide_args=True)
    _, err = child.communicate(timeout=10)
    evidence = err + b"".join(path.read_bytes() for path in directory.iterdir())
    assert child.returncode == 127 and private_prompt.encode() not in evidence and missing_name.encode() not in evidence
    if "--privacy-only" in sys.argv[1:]:
        print("Foreground supervisor privacy, both record formats, exit code and timings PASSED")
        sys.exit(0)

    args = ["", "space here", "中文", 'a"b', "trail\\", "--hide-args"]
    child, directory = launch("参数", [sys.executable, "-c", 'import json,sys;print(json.dumps(sys.argv[1:]));sys.stderr.write("x"*250000);sys.exit(7)'] + args)
    out, err = child.communicate(timeout=15)
    assert child.returncode == 7 and json.loads(out) == args and len(err) == 250000
    assert state(directory)["exit_code"] == 7
    assert len(list(directory.glob("*.status"))) == 1

    child, directory = launch("quiet", [sys.executable, "-c", "import time;time.sleep(1.2)"])
    time.sleep(.45)
    first = state(directory)
    time.sleep(.45)
    second = state(directory)
    assert first["status"] == second["status"] == "running"
    assert second["heartbeat_at_ms"] > first["heartbeat_at_ms"]
    assert second["execution_activity_at_ms"] == first["execution_activity_at_ms"]
    child.communicate(timeout=10)
    assert child.returncode == 0 and state(directory)["status"] == "succeeded"

    child, directory = launch("cancel", [sys.executable, "-c", "import time;time.sleep(20)"])
    time.sleep(.4)
    child.send_signal(signal.SIGTERM)
    child.communicate(timeout=10)
    assert child.returncode == 143 and state(directory)["status"] == "interrupted"

    child, directory = launch("missing", ["/taskpulse-no-such-executable"])
    child.communicate(timeout=10)
    assert child.returncode == 127 and state(directory)["status"] == "failed"

    child, directory = launch("legacy", [sys.executable, "-c", "pass"], legacy=True)
    out, err = child.communicate(timeout=10)
    assert child.returncode == 0, err.decode()
    assert state(directory)["status"] == "succeeded"

    child, directory = launch("killed", [sys.executable, "-c", "import time;time.sleep(1)"])
    time.sleep(.35)
    child.kill()
    child.communicate(timeout=5)
    assert state(directory)["status"] == "running"  # Never fabricated success.
print("Linux wrapper: argv/stdio/heartbeat/success/failure/cancel/SIGKILL/legacy entry PASSED")
