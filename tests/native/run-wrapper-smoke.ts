// Isolated Windows process-boundary acceptance; never writes monitored folders.
import { mkdir, readdir, readFile } from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';

const repo = path.resolve(import.meta.dir, '../..');
const root = path.join(repo, 'src-tauri', 'target', 'wrapper-smoke', randomUUID(), '中文 space');
await mkdir(root, { recursive: true });
const node = Bun.which('node.exe');
assert.ok(node, 'The wrapper smoke requires Node on PATH');
const quote = (s: string) => "'" + s.replaceAll("'", "''") + "'";
async function launch(label: string, args: string[]) {
  const directory = path.join(root, label);
  await mkdir(directory);
  const code = `$ProgressPreference='SilentlyContinue'; $jobArgs=@(${args.map(quote).join(',')}); & ${quote(path.join(repo, 'scripts', 'wrap.ps1'))} -AgentName ${quote(label)} -Command $jobArgs; exit $LASTEXITCODE`;
  const child = Bun.spawn(['powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-EncodedCommand', Buffer.from(code, 'utf16le').toString('base64')], {
    cwd: root, env: { ...process.env, TASKPULSE_STATUS_DIR: directory }, stdin: 'pipe', stdout: 'pipe', stderr: 'pipe',
  });
  child.stdin.write('stdin 验收\n');
  child.stdin.end();
  const stdout = new Response(child.stdout).text(), stderr = new Response(child.stderr).text();
  const state = async () => {
    // Cold PowerShell startup and the stdin relay compilation can take several seconds.
    const deadline = Date.now() + 15_000;
    let files = (await readdir(directory)).filter(f => f.endsWith('.status'));
    while (!files.length && child.exitCode === null && Date.now() < deadline) {
      await Bun.sleep(50);
      files = (await readdir(directory)).filter(f => f.endsWith('.status'));
    }
    assert.equal(files.length, 1);
    return JSON.parse(await readFile(path.join(directory, files[0]), 'utf8'));
  };
  return { child, stdout, stderr, state };
}
const argumentsToKeep = ['', 'with space', '中文', 'a"b', 'trail\\', 'two\\"quotes', '--hide-args'];
const argv = await launch('参数验证', [node, '-e', 'const fs=require("fs");fs.writeSync(1,JSON.stringify(process.argv.slice(1))+"\\n");fs.writeSync(2,"x".repeat(250000));process.exit(7)', '--', ...argumentsToKeep]);
const argvCode = await argv.child.exited;
assert.equal(argvCode, 7, (await argv.stderr).slice(-4000));
assert.deepEqual(JSON.parse((await argv.stdout).trim()), argumentsToKeep);
assert.equal((await argv.stderr).length, 250000);
const failed = await argv.state();
assert.equal(failed.status, 'failed'); assert.equal(failed.exit_code, 7); assert.equal(failed.name, '参数验证');
const input = await launch('标准输入', [node, '-e', 'process.stdout.write(require("fs").readFileSync(0))']);
assert.equal(await input.child.exited, 0); await input.stderr;
assert.equal(await input.stdout, 'stdin 验收\n');
const privatePrompt = 'private-prompt-fixture';
const quiet = await launch('静默心跳', ['--hide-args', node, '-e', 'setTimeout(()=>process.exit(process.argv[1] ? 0 : 1),4200)', privatePrompt]);
const first = await quiet.state();
await Bun.sleep(1600);
const next = await quiet.state();
assert.equal(first.status, 'running'); assert.equal(next.status, 'running');
assert.ok(next.heartbeat_at > first.heartbeat_at);
assert.equal(next.execution_activity_at, first.execution_activity_at);
assert.equal(await quiet.child.exited, 0); await quiet.stdout; await quiet.stderr;
const ended = await quiet.state(); assert.equal(ended.status, 'succeeded'); assert.equal(ended.exit_code, 0); assert.ok(ended.finished_at);
assert.ok(ended.finished_at >= ended.started_at);
for (const status of [first, next, ended]) {
  assert.equal(status.name, '静默心跳');
  assert.equal(status.message, null);
  assert.equal(status.activity, null);
  assert.ok(!JSON.stringify(status).includes(privatePrompt));
}
const missing = await launch('启动失败', ['taskpulse-nonexistent-executable-' + randomUUID()]);
assert.equal(await missing.child.exited, 127); await missing.stdout; await missing.stderr;
assert.equal((await missing.state()).status, 'failed');
const privateExecutable = 'taskpulse-unavailable-' + randomUUID();
const hiddenMissing = await launch('隐藏的启动失败', ['--hide-args', privateExecutable, privatePrompt]);
assert.equal(await hiddenMissing.child.exited, 127);
const diagnostics = JSON.stringify(await hiddenMissing.state()) + await hiddenMissing.stderr;
assert.ok(!diagnostics.includes(privateExecutable) && !diagnostics.includes(privatePrompt));
console.log('Windows wrapper: argv/Unicode/empty strings/stdin/large stderr/exit status/quiet heartbeat/start failure PASSED');
console.log('Isolated evidence:', root);
