/** Read-only native WebView2/WSL acceptance over a test-owned CDP 9337.
 * The isolated TaskPulse profile must already contain the enabled WSL source.
 * This script never saves settings or writes Agent data.
 */
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
const distro=process.env.TASKPULSE_WSL_DISTRO??'Ubuntu';
const project=process.env.TASKPULSE_WSL_PROJECT??'';
const title=process.env.TASKPULSE_WSL_TITLE??'';
let popup:Tab|undefined;
for(let i=0;i<100;i++){
  popup=(await tabs().catch(()=>[])).find(t=>t.url.includes('tauri.localhost')&&!t.url.includes('/manage')&&!t.url.includes('/notice'));
  if(popup)break;
  await Bun.sleep(100);
}
if(!popup)throw Error('No actual TaskPulse popup');
const root=await connect(popup);
try {
  const settings=await root.invoke('get_monitoring_settings');
  const source=settings.sources.find((s:any)=>s.kind==='codex_wsl'&&s.location.toLowerCase()===distro.toLowerCase());
  if(!source?.enabled)throw Error('Configure the WSL source in the isolated profile before starting this smoke test');
  await root.wait(`__TAURI_INTERNALS__.invoke('get_providers').then(p=>p.some(s=>s.provider===${JSON.stringify(source.id)}&&s.status==='connected'))`);
  const directory=await root.invoke('list_discovered_sessions',{source:source.id,query:project,offset:0,limit:100});
  if(!directory.items.length)throw Error('WSL native sessions missing');
  if(directory.items.some((v:any)=>!v.task.session_id||v.task.source_instance!==source.id||v.task.remote||v.task.id!==source.id+':codex:'+v.task.session_id))throw Error('WSL identities were aliased to Windows/SSH');
  const sample=title?directory.items.find((v:any)=>v.task.name===title):directory.items[0];
  if(!sample)throw Error('Native sidebar rename/title mismatch');
  if(project&&(sample.task.project!==project||!sample.task.project_path?.startsWith('/')))throw Error('Linux project metadata mismatch');
  console.log(JSON.stringify({passed:true,source:source.id,distro,sessions:directory.total,
    checks:['connected native source','stable isolated session IDs','native title and Linux project']}));
} finally {
  root.close();
}
