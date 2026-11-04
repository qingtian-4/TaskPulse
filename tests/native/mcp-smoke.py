"""Exercise the actual executable's MCP stdio server in an isolated profile.
No host configuration, real Agent tasks or network listener is involved.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile


def smoke(executable, directory):
    profile = directory / "profile"
    profile.mkdir()
    configuration = '''[dashboard]
version = 2
onboarding_complete = true
notifications = false
sound = false
[[dashboard.sources]]
id = "mcp-smoke"
name = "Synthetic MCP host"
kind = "generic_event"
enabled = true
projects = ["*"]
[dashboard.sources.integration]
agent_id = "custom-agent"
adapter_id = "standard-events"
transport = "local"
accept_since_ms = 0
'''
    config = profile / "config.toml"
    config.write_text(configuration, encoding="utf-8")
    env = dict(os.environ, TASKPULSE_HOME=str(profile))
    env.pop("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", None)
    command = [str(executable), "--integration-mcp", "--source", "mcp-smoke"]

    def exchange(messages):
        data = b"".join((json.dumps(message, ensure_ascii=False) + "\n").encode() for message in messages)
        result = subprocess.run(command, input=data, capture_output=True, env=env, timeout=15)
        assert result.returncode == 0, f"MCP exited {result.returncode}"
        assert result.stderr == b"", "Successful MCP must not emit diagnostics"
        return [json.loads(line) for line in result.stdout.splitlines()]

    def request(number, method, params=None):
        return dict(jsonrpc="2.0", id=number, method=method, **({"params": params} if params is not None else {}))

    handshake = [request(1, "initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                "clientInfo": {"name": "isolated-smoke", "version": "1"}}),
                 {"jsonrpc": "2.0", "method": "notifications/initialized"}]

    def report(number, kind, event_id, **extra):
        arguments = dict(session_id="synthetic-session", turn_id="turn-a", event_id=event_id, kind=kind, **extra)
        return request(number, "tools/call", {"name": "taskpulse_report", "arguments": arguments})

    probe = exchange([request(0, "server/discover")])
    assert probe[0]["error"]["code"] == -32601, "Newer clients must be able to fall back to a handshake"
    replies = exchange(handshake + [request(2, "tools/list"), report(3, "connection.test", "test")])
    assert replies[0]["result"]["protocolVersion"] == "2025-11-25"
    assert [tool["name"] for tool in replies[1]["result"]["tools"]] == ["taskpulse_report"]
    assert replies[2]["result"]["isError"] is False
    assert not list(profile.glob("integrations/*/events/*.jsonl")), "Connection test created a task"

    reports = [report(2, "turn.started", "start", payload={"title": "合成 MCP 会话", "project": "fixture"}),
               report(3, "input.requested", "request-a", request_id="request-a", payload={"request_kind": "approval"}),
               report(4, "input.requested", "request-b", request_id="request-b", payload={"request_kind": "question"}),
               report(5, "input.resolved", "resolved-a", request_id="request-a"),
               report(6, "phase.changed", "thinking", payload={"phase": "thinking"})]
    replies = exchange(handshake + reports)
    assert all(reply["result"].get("isError") is False for reply in replies[1:])
    journals = list(profile.glob("integrations/*/events/*.jsonl"))
    assert len(journals) == 1
    journal = journals[0]
    events = [json.loads(line) for line in journal.read_text(encoding="utf-8").splitlines()]
    assert all(item["evidence"] == "self_reported" for item in events)
    assert [item["sequence"] for item in events] == list(range(1, 6))
    assert events[-1]["context"]["admission"]["pending"][0]["request"] == "request-b"
    assert len(events[-1]["context"]["admission"]["pending"]) == 1
    assert not any(item["event"]["kind"] == "turn.completed" for item in events), "EOF completed a task"
    before = journal.read_bytes()
    rejected = [report(2, "input.resolved", "no-request"),
                report(3, "turn.completed", "spoof", source_id="other-source"),
                report(4, "phase.changed", "private", payload={"prompt": "DO_NOT_STORE"}),
                report(5, "turn.completed", "wrong-agent", agent_id="other-agent")]
    replies = exchange(handshake + rejected)
    assert all(reply["result"]["isError"] for reply in replies[1:])
    assert journal.read_bytes() == before, "Rejected calls changed the journal"
    assert b"DO_NOT_STORE" not in json.dumps(replies).encode()

    # A long-lived server must recheck source enablement after initialization.
    child = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env)
    try:
        # ping synchronization makes sure the server read its original binding first.
        import queue
        import threading
        received = queue.Queue()
        def read_output():
            for line in child.stdout:
                received.put(line)
        reader = threading.Thread(target=read_output, daemon=True)
        reader.start()
        child.stdin.write(b"".join((json.dumps(m) + "\n").encode() for m in handshake + [request(2, "ping")]))
        child.stdin.flush()
        assert json.loads(received.get(timeout=10))["id"] == 1
        assert json.loads(received.get(timeout=10))["id"] == 2
        config.write_text(configuration.replace("enabled = true", "enabled = false"), encoding="utf-8")
        child.stdin.write((json.dumps(report(3, "turn.completed", "disabled")) + "\n").encode())
        child.stdin.flush()
        denied = json.loads(received.get(timeout=10))
        assert denied["result"]["isError"] is True
        assert denied["result"]["content"][0]["text"] == "source_disabled"
        child.stdin.close()
        child.wait(timeout=5)
        reader.join(timeout=2)
        assert journal.read_bytes() == before
    finally:
        if child.poll() is None:
            child.kill()
            child.wait(timeout=5)
        for pipe in (child.stdin, child.stdout, child.stderr):
            pipe.close()

    config.write_text(configuration, encoding="utf-8")
    replies = exchange(handshake + [report(2, "input.resolved", "resolved-b", request_id="request-b"),
                                    report(3, "turn.completed", "ended")])
    assert all(reply["result"]["isError"] is False for reply in replies[1:])
    events = [json.loads(line) for line in journal.read_text(encoding="utf-8").splitlines()]
    assert events[-1]["event"]["kind"] == "turn.completed"
    assert events[-1]["context"]["admission"]["pending"] == []
    before = journal.read_bytes()
    result = subprocess.run(command, input=b"x" * (64 * 1024 + 1), capture_output=True, env=env, timeout=10)
    assert len(result.stdout) < 300 and json.loads(result.stdout)["error"]["code"] == -32600
    assert journal.read_bytes() == before
    assert config.read_text(encoding="utf-8") == configuration
    return {"passed": True, "platform": os.name, "transport": "stdio", "evidence": "self_reported",
            "checks": ["handshake and tools", "connection creates no task", "two pending requests and matched resolution",
                       "EOF never ends task", "source binding and private-field rejection", "live disablement",
                       "explicit completion after restart", "bounded frame"],
            "real_host_verified": False, "executable_sha256": hashlib.sha256(executable.read_bytes()).hexdigest()}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True, type=Path)
    parser.add_argument("--out", type=Path)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="taskpulse-mcp-") as temporary:
        result = smoke(args.exe.resolve(strict=True), Path(temporary))
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result))
