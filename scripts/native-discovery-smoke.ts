/** Read-only actual Windows WebView2 acceptance on temporary localhost CDP 9337.
 * Verifies real Codex sidebar names, active-task rendering and filter escape.
 * No Agent prompts, fake tasks, config saves or attention mutations.
 */
import { Database } from 'bun:sqlite';
import { join } from 'node:path';
const tabs=await fetch('http://127.0.0.1:9337/json').then(r=>r.json()) as Array<{url:string;webSocketDebuggerUrl:string}>;
const tab=tabs.find(t=>t.url.includes('tauri.localhost') && !t.url.includes('/notice') && !t.url.includes('/manage'));
if(!tab)throw Error('No real TaskPulse popup at temporary CDP 9337');
const ws=new WebSocket(tab.webSocketDebuggerUrl);
await new Promise<void>((ok,fail)=>{ws.onopen=()=>ok();ws.onerror=()=>fail(Error('CDP unavailable'));});
let serial=0;
const pending=new Map<number,{ok:(r:any)=>void;fail:(e:Error)=>void;timer:ReturnType<typeof setTimeout>}>();
ws.onmessage=e=>{const r=JSON.parse(String(e.data));const p=pending.get(r.id);if(!p)return;pending.delete(r.id);clearTimeout(p.timer);r.error?p.fail(Error(JSON.stringify(r.error))):p.ok(r.result);};
const send=(method:string,params:Record<string,unknown>={})=>new Promise<any>((ok,fail)=>{
  const id=++serial;const timer=setTimeout(()=>{pending.delete(id);fail(Error('CDP timeout'));},10000);
  pending.set(id,{ok,fail,timer});ws.send(JSON.stringify({id,method,params}));
});
const evaluate=(expression:string)=>send('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true,userGesture:true}).then(r=>{if(r.exceptionDetails)throw Error(JSON.stringify(r.exceptionDetails));return r.result.value;});
const invoke=(command:string)=>evaluate(`__TAURI_INTERNALS__.invoke(${JSON.stringify(command)})`);
async function waitFor(expression:string){for(let n=0;n<150;n++){if(await evaluate(expression))return;await Bun.sleep(100);}throw Error('Native assertion timed out: '+expression);}
try{
  await waitFor(`!!document.querySelector('.dashboard') && !!globalThis.__TAURI_INTERNALS__`);
  const original=await invoke('get_monitoring_settings');
  await waitFor(`__TAURI_INTERNALS__.invoke('get_providers').then(p=>p.some(s=>s.provider==='codex' && s.status==='connected'))`);
  await evaluate(`document.querySelector('.counts .total').click()`);
  const tasks=await invoke('get_agents');
  if(!tasks.some((t:any)=>t.status==='running' && !t.stale))throw Error('Requires an actual running native Agent');
  for(const task of tasks){
    await waitFor(`!!document.querySelector('[data-task-id='+CSS.escape(${JSON.stringify(task.id)})+']')`);
  }
  const root=join(process.env.USERPROFILE!,'.codex');
  const titles=new Map<string,string>();
  for(const line of (await Bun.file(join(root,'session_index.jsonl')).text()).split('\n')){
    try{const row=JSON.parse(line);if(row.id && row.thread_name)titles.set(row.id,row.thread_name);}catch{ /* partial final lines */ }
  }
  const db=new Database(join(root,'state_5.sqlite'),{readonly:true});
  const names=new Map((db.query('SELECT id,name FROM threads WHERE archived=0').all() as any[]).map(r=>[r.id,r.name]));
  db.close();
  let named=0;
  for(const task of tasks.filter((t:any)=>t.source_instance==='codex')){
    const expected=names.get(task.session_id) || titles.get(task.session_id);
    if(!expected || expected.length>56)continue;
    if(task.name!==expected)throw Error('Native sidebar title mismatch for stable ID '+task.id);
    await waitFor(`document.querySelector('[data-task-id='+CSS.escape(${JSON.stringify(task.id)})+'] .row-title')?.getAttribute('title')===${JSON.stringify(expected)}`);
    named++;
  }
  if(!named)throw Error('No native sidebar title was verified');
  await evaluate(`(()=>{if(!document.querySelector('[aria-label="筛选来源"]'))[...document.querySelectorAll('.view-bar button')].find(b=>b.textContent.includes('筛选 / 分组')).click();})()`);
  // A hidden, collapsed filter can leave the list empty despite live tasks.
  // Use an enabled source without any fresh running task, never alter scopes.
  const running=tasks.filter((t:any)=>t.status==='running' && !t.stale);
  const other=original.sources.find((s:any)=>s.enabled && !running.some((t:any)=>t.source_instance===s.id));
  if(!other)throw Error('Requires a second enabled source for filter acceptance');
  await evaluate(`(()=>{const s=document.querySelector('[aria-label="筛选来源"]');if(!s)throw Error('Filter select not visible');s.value=${JSON.stringify(other.id)};s.dispatchEvent(new Event('change',{bubbles:true}));})()`);
  await waitFor(`!!document.querySelector('.outside-running')`);
  await waitFor(`document.querySelector('.filter-note')?.textContent.includes(${JSON.stringify(other.name)})`);
  await evaluate(`[...document.querySelectorAll('.counts button')].find(b=>b.textContent.trim().startsWith('运行')).click()`);
  for(const task of running)await waitFor(`!!document.querySelector('[data-task-id='+CSS.escape(${JSON.stringify(task.id)})+']')`);
  await waitFor(`!document.querySelector('.outside-running')`);
  if(JSON.stringify(await invoke('get_monitoring_settings'))!==JSON.stringify(original))throw Error('Read-only acceptance changed monitoring configuration');
  await evaluate(`document.querySelector('.counts .total').click()`);
  const layout=await evaluate(`(()=>{const p=document.querySelector('.dashboard');return {width:p.getBoundingClientRect().width,height:p.getBoundingClientRect().height,overflow:document.documentElement.scrollWidth>innerWidth,errors:[...document.querySelectorAll('[role=alert]')].map(n=>n.textContent)};})()`);
  if(layout.width>440.1 || layout.height>720.1 || layout.overflow || layout.errors.length)throw Error('Native dashboard layout or IPC errors');
  await Bun.write('src-tauri/target/ui-smoke/native-discovery.png',Buffer.from((await send('Page.captureScreenshot',{format:'png'})).data,'base64'));
  const result={passed:true,taskRowsVerified:tasks.length,nativeSidebarNamesVerified:named,activeRowsVerified:running.length,layout,checks:['actual native IPC and rows','sidebar titles read from real native metadata','hidden running task warning','global running counter reveals every fresh running task','bounded real popup without overflow','monitoring configuration unchanged'],notVerified:['a new user-created Desktop thread beginning during acceptance','long-run P95 discovery latency']};
  await Bun.write('src-tauri/target/ui-smoke/native-discovery-result.json',JSON.stringify(result,null,2));
  console.log(JSON.stringify(result));
}finally{
  await evaluate(`document.querySelector('.counts .total')?.click()`).catch(()=>{});
  ws.close();
}
