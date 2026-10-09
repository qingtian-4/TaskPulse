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
  const archived = await native.invoke('get_archived_attention');
  if (archived.some((t:any)=>t.status==='running' || t.internal) || archived.some((t:any)=>initial.some((v:any)=>v.id===t.id))) throw Error('Archive/main overlap or live task archived');
  if (original.recent_days && archived.some((t:any)=>t.updated_at < Date.now()-original.recent_days*86400000)) {
    await native.evaluate(`[...document.querySelectorAll('.view-bar button')].find(b=>b.textContent.includes('较早关注')).click()`);
    await native.waitFor(`document.querySelectorAll('.task-row').length===${archived.length}`);
    await Bun.write(out + '/native-archive.png', Buffer.from((await native.send('Page.captureScreenshot', {format:'png'})).data,'base64'));
    await native.evaluate(`[...document.querySelectorAll('.view-bar button')].find(b=>b.textContent.includes('近期活动')).click()`);
    await native.waitFor(`document.querySelectorAll('.task-row').length===${initial.length}`);
  }
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
    // Actual UI switch saves immediately; restore before continuing. No task read/pin mutations.
    changed=true;
    await manage.evaluate(`document.querySelector('[aria-label="启用 Codex · 本机"]').click()`);
    await manage.waitFor(`__TAURI_INTERNALS__.invoke('get_monitoring_settings').then(s=>!s.sources.find(x=>x.id==='codex').enabled)`);
    await manage.waitFor(`document.body.textContent.includes('开关即时生效')`);
    await manage.evaluate(`document.querySelector('[aria-label="启用 Codex · 本机"]').click()`);
    await manage.waitFor(`__TAURI_INTERNALS__.invoke('get_monitoring_settings').then(s=>s.sources.find(x=>x.id==='codex').enabled)`);
    await manage.waitFor(`document.querySelector('[aria-label="启用 Codex · 本机"]').disabled===false`);
    await manage.evaluate(`document.querySelector('.editor input').value='Unsaved smoke draft';document.querySelector('.editor input').dispatchEvent(new Event('input',{bubbles:true}));`);
    await manage.invoke('plugin:window|close',{label:'management'});
    await manage.waitFor(`!!document.querySelector('[aria-labelledby="close-title"]')`);
    await manage.evaluate(`[...document.querySelectorAll('dialog button')].find(b=>b.textContent==='放弃未保存编辑').click()`);
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
  await native.evaluate(`document.querySelector('[aria-label="立即刷新"]').click()`);
  await native.waitFor(`document.querySelector('[aria-label="立即刷新"]').disabled===true`);
  for(let n=0;n<350;n++) {
    if(await native.evaluate(`document.querySelector('[aria-label="立即刷新"]').disabled===false`))break;
    if(n===349)throw Error('Manual refresh never completed');
    await Bun.sleep(100);
  }
  const refreshFeedback=await native.evaluate(`document.querySelector('[role="status"]')?.textContent ?? document.querySelector('[role="alert"]')?.textContent ?? ''`);
  if(!refreshFeedback.includes('刷新') && !refreshFeedback.includes('采集结果'))throw Error('No actual refresh feedback');
  await native.invoke('test_notification');
  if (native.errors.length) throw Error('Popup console/CSP errors: ' + native.errors.join(';'));
  const result = {
    passed: true, layout, initialTasks: initial.length, archivedTasks:archived.length, discoveredSessions: directory.total, refreshFeedback,
    providers: providers.map((p: any) => ({ provider: p.provider, status: p.status })),
    checks: ['native embedded popup', 'archive separate from live main', 'management rendered with native IPC', 'immediate source toggle UI', 'unsaved close prevention', 'manual refresh actual source results', 'unchanged scopes after save/restore', 'disable + scanner exclusion', 'restore stable Codex identity', 'internal service exclusion', 'no CSP console errors', 'native notification API accepted'],
    notVerified: ['user clicked delivered banner', 'real-agent P95 latency']
  };
  await Bun.write(out + '/native-result.json', JSON.stringify(result, null, 2));
  await Bun.write(out + '/native-dashboard.png', Buffer.from((await native.send('Page.captureScreenshot', { format: 'png' })).data, 'base64'));
  console.log(JSON.stringify(result));
} finally {
  try { if (changed && original) await native.invoke('save_monitoring_settings', { settings: original }); }
  finally { native.close(); }
}
