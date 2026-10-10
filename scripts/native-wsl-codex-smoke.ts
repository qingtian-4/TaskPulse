/** Actual WebView2/WSL acceptance with temporary CDP 9337. No Agent prompts.
 * Temporarily enables one running WSL distro, restoring TaskPulse config by
 * default. TASKPULSE_KEEP_WSL_CODEX=1 retains it only after full success.
 * Does not alter Agent configuration, session files, hooks or read state.
 */
import { mkdir } from 'node:fs/promises';
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
const root=await connect(popup);let manage:Awaited<ReturnType<typeof connect>>|undefined;
let original:any;let saved=false;let success=false;
try{
  original=await root.invoke('get_monitoring_settings');
  const discovery=await root.invoke('discover_sources');
  let source=original.sources.find((s:any)=>s.kind==='codex_wsl'&&s.location.toLowerCase()===distro.toLowerCase());
  const candidate=discovery.find((s:any)=>s.available&&s.candidate?.kind==='codex_wsl'&&s.candidate.location.toLowerCase()===distro.toLowerCase());
  if(!source&&!candidate)throw Error('No running WSL candidate: '+distro);
  await root.invoke('plugin:window|show',{label:'popup'});
  await root.evaluate(`document.querySelector('[aria-label="管理监控来源"]').click()`);
  let tab:Tab|undefined;
  for(let i=0;i<100;i++){tab=(await tabs()).find(t=>t.url.includes('/manage'));if(tab)break;await Bun.sleep(100);}
  if(!tab)throw Error('Management did not open');
  manage=await connect(tab);
  await manage.wait(`!!document.querySelector('.manage') && document.querySelectorAll('.source-item').length>0`);
  if(!source){
    const label='发现 '+candidate.candidate.name;
    await manage.wait(`document.body.textContent.includes(${JSON.stringify(label)})`);
    await manage.evaluate(`[...document.querySelectorAll('.message')].find(x=>x.textContent.includes(${JSON.stringify(label)})).querySelector('button').click()`);
    await manage.wait(`document.querySelector('header .primary')?.disabled===false`);
    saved=true;
    await manage.evaluate(`document.querySelector('header .primary').click()`);
  }else if(!source.enabled){
    saved=true;
    await manage.evaluate(`document.querySelector('[aria-label=${JSON.stringify('启用 '+source.name)}]').click()`);
  }
  await root.wait(`__TAURI_INTERNALS__.invoke('get_monitoring_settings').then(s=>s.sources.some(x=>x.kind==='codex_wsl'&&x.location.toLowerCase()===${JSON.stringify(distro.toLowerCase())}&&x.enabled))`);
  source=(await root.invoke('get_monitoring_settings')).sources.find((s:any)=>s.kind==='codex_wsl'&&s.location.toLowerCase()===distro.toLowerCase());
  await manage.evaluate(`[...document.querySelectorAll('.source-item')].find(x=>x.textContent.includes(${JSON.stringify(source.name)})).querySelector('button').click()`);
  await manage.wait(`document.body.textContent.includes('独立于 Windows')`);
  await root.wait(`__TAURI_INTERNALS__.invoke('get_providers').then(p=>p.some(s=>s.provider===${JSON.stringify(source.id)}&&s.status==='connected'))`);
  const directory=await root.invoke('list_discovered_sessions',{source:source.id,query:project,offset:0,limit:100});
  if(!directory.items.length)throw Error('WSL native sessions missing');
  if(directory.items.some((v:any)=>!v.task.session_id||v.task.source_instance!==source.id||v.task.remote||v.task.id!==source.id+':codex:'+v.task.session_id))throw Error('WSL identities were aliased to Windows/SSH');
  const sample=title?directory.items.find((v:any)=>v.task.name===title):directory.items[0];
  if(!sample)throw Error('Native sidebar rename/title mismatch');
  if(project&&(sample.task.project!==project||!sample.task.project_path?.startsWith('/')))throw Error('Linux project metadata mismatch');
  const others=(s:any)=>s.sources.filter((v:any)=>v.id!==source.id);
  if(JSON.stringify(others(await root.invoke('get_monitoring_settings')))!==JSON.stringify(others(original)))throw Error('Other source scopes changed');
  // Prove the distro partition can be disabled/restored without editing scopes.
  const enabled=await root.invoke('get_monitoring_settings');
  const disabled=structuredClone(enabled);disabled.sources.find((s:any)=>s.id===source.id).enabled=false;
  saved=true;
  await root.invoke('save_monitoring_settings',{settings:disabled});
  await root.wait(`__TAURI_INTERNALS__.invoke('get_agents').then(t=>!t.some(x=>x.source_instance===${JSON.stringify(source.id)}))`);
  await root.invoke('save_monitoring_settings',{settings:enabled});
  await root.wait(`__TAURI_INTERNALS__.invoke('get_providers').then(p=>p.some(s=>s.provider===${JSON.stringify(source.id)}&&s.status==='connected'))`);
  await manage.invoke('plugin:window|show',{label:'management'});
  await Bun.write(out+'/native-wsl-source.png',Buffer.from((await manage.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
  await manage.evaluate(`[...document.querySelectorAll('nav button')].find(x=>x.textContent==='已发现会话').click()`);
  await manage.evaluate(`const select=document.querySelector('[aria-label="会话来源"]');select.value=${JSON.stringify(source.id)};select.dispatchEvent(new Event('change',{bubbles:true}));`);
  await manage.wait(`document.querySelectorAll('article.session').length>0`);
  await Bun.write(out+'/native-wsl-directory.png',Buffer.from((await manage.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
  const result={passed:true,source:source.id,distro,sessions:directory.total,sample:{id:sample.task.id,title:sample.task.name,project:sample.task.project,path:sample.task.project_path,status:sample.task.status},checks:['actual WSL candidate and save UI','connected native source','stable isolated session IDs','native title and Linux project','independent disable/enable','unchanged Windows and SSH scopes'],notVerified:['real new-turn end-to-end latency','all Linux users or shell-only custom CODEX_HOME']};
  await Bun.write(out+'/native-wsl-result.json',JSON.stringify(result,null,2));
  console.log(JSON.stringify(result));success=true;
}finally{
  try{
    if(saved&&original&&(!success||process.env.TASKPULSE_KEEP_WSL_CODEX!=='1'))await root.invoke('save_monitoring_settings',{settings:original});
    if(manage)await manage.invoke('plugin:window|close',{label:'management'});
  }finally{manage?.close();root.close();}
}
