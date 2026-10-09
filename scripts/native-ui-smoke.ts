/** Opt-in real Windows Tauri smoke, never mocked IPC.
 * Start an embedded-frontend build with temporary localhost CDP port 9337.
 * Briefly disables/re-enables local Codex and restores scopes in finally.
 * Sends ONE real test notification. Does not change Agent sessions or read state.
 * Close the debug-enabled app and restart normally after this test.
 */
import { mkdir } from 'node:fs/promises';

const out = 'src-tauri/target/ui-smoke';
await mkdir(out, { recursive: true });
type Tab = { url: string; webSocketDebuggerUrl: string };
const tabs = async (): Promise<Tab[]> => fetch('http://127.0.0.1:9337/json').then(r => r.json());

async function client(tab: Tab) {
  const ws = new WebSocket(tab.webSocketDebuggerUrl);
  await new Promise<void>((ok, fail) => { ws.onopen = () => ok(); ws.onerror = () => fail(Error('CDP failed')); });
  let id = 0;
  const errors: string[] = [];
  const pending = new Map<number, { ok: (value: any) => void; fail: (error: any) => void; timer: ReturnType<typeof setTimeout> }>();
  ws.onmessage = event => {
    const message = JSON.parse(String(event.data));
    if (message.method === 'Log.entryAdded' && message.params.entry.level === 'error') errors.push(message.params.entry.text);
    if (!message.id) return;
    const call = pending.get(message.id);
    if (!call) return;
    clearTimeout(call.timer);
    pending.delete(message.id);
    message.error ? call.fail(message.error) : call.ok(message.result);
  };
  const send = (method: string, params: Record<string, unknown> = {}) => new Promise<any>((ok, fail) => {
    const current = ++id;
    const timer = setTimeout(() => { pending.delete(current); fail(Error('CDP timeout ' + method)); }, 15000);
    pending.set(current, { ok, fail, timer });
    ws.send(JSON.stringify({ id: current, method, params }));
  });
  const evaluate = async (expression: string) => {
    const result = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true, userGesture: true });
    if (result.exceptionDetails) throw Error(JSON.stringify(result.exceptionDetails));
    return result.result.value;
  };
  const invoke = (command: string, args: Record<string, unknown> = {}) =>
    evaluate(`__TAURI_INTERNALS__.invoke(${JSON.stringify(command)},${JSON.stringify(args)})`);
  const waitFor = async (expression: string) => {
    for (let n = 0; n < 100; n++) {
      if (await evaluate(expression)) return;
      await Bun.sleep(100);
    }
    throw Error('Native UI assertion timeout: ' + expression);
  };
  await send('Log.enable');
  return { send, evaluate, invoke, waitFor, errors, close: () => ws.close() };
}

let popup: Tab | undefined;
for (let n = 0; n < 100; n++) {
  popup = (await tabs().catch(() => [])).find(t => t.url.includes('tauri.localhost') && !t.url.includes('manage'));
  if (popup) break;
  await Bun.sleep(100);
}
if (!popup) throw Error('No live TaskPulse popup at CDP 9337');
const native = await client(popup);
let original: any;
let changed = false;
try {
  await native.waitFor(`!!globalThis.__TAURI_INTERNALS__&&!!document.querySelector('.dashboard')`);
  original = await native.invoke('get_monitoring_settings');
  if (!original.onboarding_complete) throw Error('Smoke requires already configured scopes');
  await native.waitFor(`__TAURI_INTERNALS__.invoke('get_providers').then(p=>p.some(s=>s.provider==='codex'&&s.status==='connected'))`);
  console.log('Native IPC connected');
  const initial = await native.invoke('get_agents');
  const providers = await native.invoke('get_providers');
  const directory = await native.invoke('list_discovered_sessions', { source: null, query: '', offset: 0, limit: 100 });
  if (initial.some((t: any) => t.internal) || directory.items.some((v: any) => v.task.internal)) throw Error('Internal service task leaked');
  const layout = await native.evaluate(`(()=>{const p=document.querySelector('.dashboard');return {width:p?.getBoundingClientRect().width,height:p?.getBoundingClientRect().height,errors:[...document.querySelectorAll('[role=alert]')].map(x=>x.textContent),isTauri:globalThis.isTauri};})()`);
  if (layout.errors.length || layout.width > 440.1 || layout.height > 720.1 || !layout.isTauri) throw Error(JSON.stringify(layout));
  await native.invoke('open_management');
  let management: Tab | undefined;
  for (let n = 0; n < 100; n++) {
    management = (await tabs()).find(t => t.url.includes('manage'));
    if (management) break;
    await Bun.sleep(100);
  }
  if (!management) throw Error('Management window not created');
  const manage = await client(management);
  try {
    await manage.waitFor(`document.querySelectorAll('.source-item').length===${original.sources.length}`);
    if ((await manage.evaluate(`document.querySelectorAll('[role=alert]').length`)) || manage.errors.length) throw Error('Management UI error: ' + manage.errors.join(';'));
    await Bun.write(out + '/native-management.png', Buffer.from((await manage.send('Page.captureScreenshot', { format: 'png' })).data, 'base64'));
  } finally { manage.close(); }
  console.log('Native management UI rendered');
  const disabled = structuredClone(original);
  const codex = disabled.sources.find((s: any) => s.kind === 'codex' && s.enabled);
  if (!codex) throw Error('No enabled local Codex source');
  codex.enabled = false;
  changed = true;
  await native.invoke('save_monitoring_settings', { settings: disabled });
  await Bun.sleep(1400);
  const afterDisable = await native.invoke('get_agents');
  if (afterDisable.some((t: any) => t.source_instance === codex.id || t.source === 'codex' && !t.remote)) throw Error('Disabled Codex leaked through scanner');
  await native.invoke('save_monitoring_settings', { settings: original });
  await native.invoke('request_refresh');
  await native.waitFor(`__TAURI_INTERNALS__.invoke('get_providers').then(p=>p.some(s=>s.provider==='codex'&&s.status==='connected'))`);
  const restored = await native.invoke('get_agents');
  const before = initial.find((t: any) => t.source === 'codex' && !t.remote && t.status === 'running');
  if (before && !restored.some((t: any) => t.id === before.id)) throw Error('Active Codex identity not restored');
  if (JSON.stringify(await native.invoke('get_monitoring_settings')) !== JSON.stringify(original)) throw Error('Scope changed after restore');
  const detail = restored.find((t: any) => !t.can_focus);
  if (detail && await native.invoke('open_task', { id: detail.id }) !== 'details') throw Error('Detail fallback failed');
  await native.invoke('test_notification');
  if (native.errors.length) throw Error('Popup console/CSP errors: ' + native.errors.join(';'));
  const result = {
    passed: true, layout, initialTasks: initial.length, discoveredSessions: directory.total,
    providers: providers.map((p: any) => ({ provider: p.provider, status: p.status })),
    checks: ['native embedded popup', 'management rendered with native IPC', 'unchanged scopes after save/restore', 'disable + scanner exclusion', 'restore stable Codex identity', 'internal service exclusion', 'no CSP console errors', 'native notification API accepted'],
    notVerified: ['user clicked delivered banner', 'real-agent P95 latency']
  };
  await Bun.write(out + '/native-result.json', JSON.stringify(result, null, 2));
  await Bun.write(out + '/native-dashboard.png', Buffer.from((await native.send('Page.captureScreenshot', { format: 'png' })).data, 'base64'));
  console.log(JSON.stringify(result));
} finally {
  try { if (changed && original) await native.invoke('save_monitoring_settings', { settings: original }); }
  finally { native.close(); }
}
