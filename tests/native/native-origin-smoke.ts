/** Read-only real WebView2 origin acceptance. No test events, scope saves,
 * hook installation, native task actions or viewed-state mutations.
 * Start embedded TaskPulse with temporary localhost CDP 9337, then restart
 * without CDP afterwards. Required IDs refer to actual opted-in sessions.
 */
import assert from 'node:assert/strict';
import { readFile, mkdir } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import path from 'node:path';

const internalId = process.env.TASKPULSE_EXPECT_INTERNAL_TASK_ID;
const parentId = process.env.TASKPULSE_EXPECT_PARENT_TASK_ID;
assert.ok(internalId && parentId, 'Set actual internal and parent task IDs');
const stateDirectory = path.join(process.env.USERPROFILE!, '.taskpulse');
const configFile = path.join(stateDirectory, 'config.toml');
const hash = (bytes: Buffer) => createHash('sha256').update(bytes).digest('hex');
const beforeConfig = hash(await readFile(configFile));
const started = Date.now();
type Tab = { url: string; webSocketDebuggerUrl: string };
let tab: Tab | undefined;
for (let n = 0; n < 100; n++) {
  const tabs: Tab[] = await fetch('http://127.0.0.1:9337/json').then(r => r.json()).catch(() => []);
  tab = tabs.find(t => t.url.includes('tauri.localhost') && !t.url.includes('/notice') && !t.url.includes('/manage'));
  if (tab) break;
  await Bun.sleep(100);
}
assert.ok(tab, 'Start actual embedded TaskPulse with temporary CDP 9337');
const socket = new WebSocket(tab.webSocketDebuggerUrl);
await new Promise<void>((resolve, reject) => { socket.onopen = () => resolve(); socket.onerror = () => reject(Error('CDP connection failed')); });
let sequence = 0;
const pending = new Map<number, { resolve: (result: any) => void; reject: (error: Error) => void; timer: ReturnType<typeof setTimeout> }>();
socket.onmessage = event => {
  const message = JSON.parse(String(event.data));
  const waiting = pending.get(message.id);
  if (!waiting) return;
  clearTimeout(waiting.timer); pending.delete(message.id);
  message.error ? waiting.reject(Error(JSON.stringify(message.error))) : waiting.resolve(message.result);
};
const send = (method: string, params: Record<string, unknown>) => new Promise<any>((resolve, reject) => {
  const id = ++sequence;
  const timer = setTimeout(() => { pending.delete(id); reject(Error('CDP timeout')); }, 15000);
  pending.set(id, { resolve, reject, timer }); socket.send(JSON.stringify({ id, method, params }));
});
const evaluate = async (expression: string) => {
  const result = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
  assert.ok(!result.exceptionDetails, JSON.stringify(result.exceptionDetails)); return result.result.value;
};
const invoke = (command: string, args: Record<string, unknown> = {}) => evaluate(`__TAURI_INTERNALS__.invoke(${JSON.stringify(command)},${JSON.stringify(args)})`);
try {
  let saved: any;
  for (let n = 0; n < 80; n++) {
    saved = JSON.parse(await readFile(path.join(stateDirectory, 'attention.json'), 'utf8'));
    if (saved.records[internalId]?.task.internal === true) break;
    await Bun.sleep(250);
  }
  const record = saved.records[internalId];
  assert.equal(record?.task.internal, true, 'Persisted wrong origin has not been reconciled');
  const agents = await invoke('get_agents');
  const parent = agents.find((t: any) => t.id === parentId && !t.internal);
  assert.ok(parent, 'Actual parent missing from attention UI');
  assert.ok(!agents.some((t: any) => t.id === internalId || t.internal), 'Internal service leaked into attention UI');
  const catalog = await invoke('list_discovered_sessions', { source: parent.source_instance, offset: 0, limit: 100 });
  assert.ok(!catalog.items.some((item: any) => item.task.internal || item.task.id === internalId), 'Internal service is selectable in discovered sessions');
  assert.ok(await evaluate(`!!document.querySelector('.dashboard')`), 'Actual dashboard not rendered');
  const notice = await invoke('get_task_notice');
  assert.notEqual(notice.current?.task_id, internalId, 'Old internal notice still visible');
  const log = JSON.parse(await readFile(path.join(stateDirectory, 'notification-log.json'), 'utf8'));
  const internalIds = new Set(Object.values(saved.records).filter((r: any) => r.task.internal).map((r: any) => r.task.id));
  assert.ok(!log.entries.some((e: any) => e.at >= started && !e.test && internalIds.has(e.task_id)), 'Internal service produced a new notification');
  assert.equal(hash(await readFile(configFile)), beforeConfig, 'Read-only acceptance changed configuration');
  const result = { passed: true, internalId, parentId, internalRecordRetained: true, unread: record.unread,
    pinned: record.pinned, checks: ['real parent retained', 'internal sessions absent from attention and selection',
      'old origin reconciled and record retained', 'no current internal alert or new internal delivery', 'config unchanged'],
    notVerified: ['a newly spawned real Guardian completing during acceptance', 'editor focus preservation'] };
  await mkdir('src-tauri/target/ui-smoke', { recursive: true });
  await Bun.write('src-tauri/target/ui-smoke/native-origin-result.json', JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result));
} finally { socket.close(); }
