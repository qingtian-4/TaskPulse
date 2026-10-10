/** Opt-in real Tauri/WebView2 acceptance; temporary CDP 9337 required.
 * Reads native Claude session metadata, never sends an Agent prompt.
 * Temporarily enables Claude in TaskPulse, then restores scopes by default.
 * TASKPULSE_KEEP_CLAUDE=1 keeps this explicitly requested source enabled.
 * Never installs real hooks or changes Claude settings / task read state.
 */
import { mkdir, readFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { join } from 'node:path';
const out='src-tauri/target/ui-smoke';
await mkdir(out,{recursive:true});
type Tab={url:string;webSocketDebuggerUrl:string};
const tabs=async():Promise<Tab[]>=>fetch('http://127.0.0.1:9337/json').then(r=>r.json());
async function connect(tab:Tab){
  const ws=new WebSocket(tab.webSocketDebuggerUrl);
  await new Promise<void>((ok,fail)=>{ws.onopen=()=>ok();ws.onerror=()=>fail(Error('CDP failed'));});
  let seq=0;const pending=new Map<number,{ok:(v:any)=>void;fail:(e:any)=>void;timer:ReturnType<typeof setTimeout>}>();
  ws.onmessage=e=>{const m=JSON.parse(String(e.data));const p=pending.get(m.id);if(!p)return;clearTimeout(p.timer);pending.delete(m.id);m.error?p.fail(Error(JSON.stringify(m.error))):p.ok(m.result);};
  const send=(method:string,params:Record<string,unknown>={})=>new Promise<any>((ok,fail)=>{const id=++seq;const timer=setTimeout(()=>{pending.delete(id);fail(Error('CDP timeout: '+method));},15000);pending.set(id,{ok,fail,timer});ws.send(JSON.stringify({id,method,params}));});
  const evaluate=async(expression:string)=>{const r=await send('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true,userGesture:true});if(r.exceptionDetails)throw Error(JSON.stringify(r.exceptionDetails));return r.result.value;};
  const invoke=(cmd:string,args:Record<string,unknown>={})=>evaluate(`__TAURI_INTERNALS__.invoke(${JSON.stringify(cmd)},${JSON.stringify(args)})`);
  const wait=async(expression:string)=>{for(let i=0;i<150;i++){if(await evaluate(expression))return;await Bun.sleep(100);}throw Error('Assertion timeout: '+expression);};
  return {send,evaluate,invoke,wait,close:()=>ws.close()};
}
let popup:Tab|undefined;
for(let i=0;i<100;i++){
  popup=(await tabs().catch(()=>[])).find(t=>t.url.includes('tauri.localhost')&&!t.url.includes('/manage')&&!t.url.includes('/notice'));
  if(popup)break;
  await Bun.sleep(100);
}
if(!popup)throw Error('No actual TaskPulse popup');
const root=await connect(popup);let manage:Awaited<ReturnType<typeof connect>>|undefined;
const settingsPath=join(process.env.CLAUDE_CONFIG_DIR??join(homedir(),'.claude'),'settings.json');
const before=await readFile(settingsPath).catch(()=>null);
let original:any;let saved=false;let success=false;
try{
  original=await root.invoke('get_monitoring_settings');
  const discovery=await root.invoke('discover_sources');
  let source=original.sources.find((s:any)=>s.kind==='claude_code');
  const candidate=discovery.find((s:any)=>s.available&&s.candidate?.kind==='claude_code');
  if(!source&&!candidate)throw Error('No actual installed Claude data source');
  await root.invoke('plugin:window|show',{label:'popup'});
  await root.evaluate(`document.querySelector('[aria-label="管理监控来源"]').click()`);
  let tab:Tab|undefined;
  for(let i=0;i<100;i++){tab=(await tabs()).find(t=>t.url.includes('/manage'));if(tab)break;await Bun.sleep(100);}
  if(!tab)throw Error('Management did not open');
  manage=await connect(tab);
  await manage.wait(`!!document.querySelector('.manage') && document.querySelectorAll('.source-item').length>0`);
  if(!await root.invoke('plugin:window|is_visible',{label:'popup'}))throw Error('Management launch hid dashboard');
  if(!source){
    await manage.wait(`document.body.textContent.includes('发现 Claude Code')`);
    await manage.evaluate(`[...document.querySelectorAll('.message')].find(x=>x.textContent.includes('发现 Claude Code')).querySelector('button').click()`);
    await manage.wait(`document.querySelector('header .primary')?.disabled===false`);
    await manage.evaluate(`document.querySelector('header .primary').click()`);
    await manage.wait(`__TAURI_INTERNALS__.invoke('get_monitoring_settings').then(s=>s.sources.some(x=>x.kind==='claude_code'&&x.enabled))`);
    source=(await root.invoke('get_monitoring_settings')).sources.find((s:any)=>s.kind==='claude_code');
    saved=true;
  }else{
    await manage.evaluate(`[...document.querySelectorAll('.source-item')].find(x=>x.textContent.includes(${JSON.stringify(source.name)})).querySelector('button').click()`);
    if(!source.enabled){
      await manage.evaluate(`document.querySelector('[aria-label=${JSON.stringify('启用 '+source.name)}]').click()`);
      await manage.wait(`__TAURI_INTERNALS__.invoke('get_monitoring_settings').then(s=>s.sources.find(x=>x.kind==='claude_code').enabled)`);
      saved=true;
    }
  }
  await manage.wait(`document.querySelector('button') && [...document.querySelectorAll('button')].some(x=>x.textContent==='启用精确事件…'&&!x.disabled)`);
  await manage.evaluate(`[...document.querySelectorAll('button')].find(x=>x.textContent==='启用精确事件…').click()`);
  await manage.wait(`!!document.querySelector('[aria-labelledby="events-title"]')`);
  await Bun.write(out+'/native-claude-confirm.png',Buffer.from((await manage.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
  await manage.evaluate(`[...document.querySelectorAll('dialog button')].find(x=>x.textContent==='取消').click()`);
  await manage.wait(`!document.querySelector('[aria-labelledby="events-title"]')`);
  const rejection=await manage.evaluate(`__TAURI_INTERNALS__.invoke('configure_claude_events',{sourceId:${JSON.stringify(source.id)},enabled:true,confirmed:false}).then(()=>false,()=>true)`);
  if(!rejection)throw Error('Hook configuration did not require confirmation');
  await root.wait(`__TAURI_INTERNALS__.invoke('get_providers').then(p=>p.some(s=>s.provider===${JSON.stringify(source.id)}&&s.status==='connected'))`);
  const directory=await root.invoke('list_discovered_sessions',{source:source.id,query:'',offset:0,limit:100});
  if(!directory.items.length)throw Error('Native Claude sessions missing');
  if(directory.items.some((v:any)=>!v.task.session_id||!v.task.id.includes('claude:')))throw Error('Unstable Claude session identity');
  const after=await readFile(settingsPath).catch(()=>null);
  if(before?.toString()!==after?.toString())throw Error('Claude settings changed without confirmation');
  const next=await root.invoke('get_monitoring_settings');
  if(JSON.stringify(next.sources.filter((s:any)=>s.kind!=='claude_code'))!==JSON.stringify(original.sources.filter((s:any)=>s.kind!=='claude_code')))throw Error('Other scopes changed');
  const result={passed:true,sessions:directory.items.map((v:any)=>({id:v.task.id,title:v.task.name,project:v.task.project,model:v.task.model,status:v.task.status,stale:v.task.stale})),checks:['actual candidate UI','native source save','management retains popup','hook confirmation and cancellation','unconfirmed IPC rejected','unchanged Claude settings','unchanged other source scopes'],notVerified:['real Claude hook dispatch','exact conversation-tab navigation']};
  await Bun.write(out+'/native-claude-result.json',JSON.stringify(result,null,2));
  console.log(JSON.stringify(result));success=true;
}finally{
  try{
    if(saved&&original&&(!success||process.env.TASKPULSE_KEEP_CLAUDE!=='1'))await root.invoke('save_monitoring_settings',{settings:original});
    if(manage)await manage.invoke('plugin:window|close',{label:'management'});
  }finally{manage?.close();root.close();}
}
