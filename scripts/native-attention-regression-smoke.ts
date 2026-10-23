/** Isolated native WebView2 attention/presentation regression, not Agent-host certification. */
import assert from 'node:assert/strict';
import {mkdir, copyFile} from 'node:fs/promises';
import {resolve, join, dirname} from 'node:path';
import {spawn, spawnSync, type ChildProcess} from 'node:child_process';
import {withSmokeWorkspace,stopOwnedChild} from './smoke-workspace';
const at=process.argv.indexOf('--exe');if(at<0)throw Error('--exe required');
const original=resolve(process.argv[at+1]);
await withSmokeWorkspace('lifecycle-native',async({owned,evidenceDir:evidence,onStop})=>{
let app:ChildProcess|undefined,spawnFailure:Error|undefined;const sockets:WebSocket[]=[];
onStop(async()=>{try{for(const ws of sockets)ws.close();}finally{await stopOwnedChild(app);}});
const exe=join(owned,'agent-tray.exe'),home=join(owned,'profile');await mkdir(home);await copyFile(original,exe);
const loader=join(dirname(original),'WebView2Loader.dll');if(await Bun.file(loader).exists())await copyFile(loader,join(owned,'WebView2Loader.dll'));
await Bun.write(join(home,'config.toml'),`[dashboard]
version=2
onboarding_complete=true
notifications=true
notification_mode="popup"
sound=false
default_view="tasks"
[[dashboard.sources]]
id="lifecycle-fixture"
name="隔离合成回归"
kind="generic_event"
enabled=true
location=""
projects=["*"]
excluded_projects=[]
tmux_sessions=[]
tmux_panes=[]
poll_interval_ms=1000
[dashboard.sources.integration]
agent_id="custom-agent"
adapter_id="standard-events"
transport="local"
accept_since_ms=0
`);
const env={...process.env,TASKPULSE_HOME:home,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:'--remote-debugging-port=9339'};
const tabs=()=>fetch('http://127.0.0.1:9339/json').then(r=>r.json()) as Promise<any[]>;
assert.equal(await tabs().then(()=>true).catch(()=>false),false,'Owned CDP port must be free');
app=spawn(exe,['--show'],{env,windowsHide:true,stdio:['ignore','ignore','pipe']});
app.once('error',error=>spawnFailure=error);
let stderr='';app.stderr?.on('data',d=>stderr=(stderr+String(d)).slice(-4000));
async function page(notice=false){
  let tab:any;for(let i=0;i<200;i++){if(spawnFailure)throw spawnFailure;tab=(await tabs().catch(()=>[])).find(t=>t.url.includes('tauri.localhost')&&(notice?t.url.includes('/notice'):!t.url.includes('/notice')&&!t.url.includes('/manage')));if(tab)break;if(app?.exitCode!==null)throw Error('Owned app exited');await Bun.sleep(100);}
  if(!tab)throw Error('Native page unavailable');const ws=new WebSocket(tab.webSocketDebuggerUrl);sockets.push(ws);
  await new Promise<void>((ok,fail)=>{ws.onopen=()=>ok();ws.onerror=()=>fail(Error('CDP connection failed'));});
  let seq=0;const pending=new Map<number,any>();
  ws.onmessage=e=>{const m=JSON.parse(String(e.data)),p=pending.get(m.id);if(!p)return;clearTimeout(p.timer);pending.delete(m.id);m.error?p.fail(Error(JSON.stringify(m.error))):p.ok(m.result);};
  const send=(method:string,params:any={})=>new Promise<any>((ok,fail)=>{const id=++seq,timer=setTimeout(()=>{pending.delete(id);fail(Error('CDP timeout'));},15000);pending.set(id,{ok,fail,timer});ws.send(JSON.stringify({id,method,params}));});
  const evaluate=async(expression:string)=>{const r=await send('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true,userGesture:true});if(r.exceptionDetails)throw Error(r.exceptionDetails.text);return r.result.value;};
  return {send,evaluate,invoke:(cmd:string,args:any={})=>evaluate(`__TAURI_INTERNALS__.invoke(${JSON.stringify(cmd)},${JSON.stringify(args)})`)};
}
async function until(check:()=>Promise<boolean>){for(let i=0;i<200;i++){if(await check())return;await Bun.sleep(100);}throw Error('Native assertion timed out');}
let seq=0;
function event(session:string,turn:string,kind:string,payload:any={},extra:any={}){
  const body={schema_version:1,agent_id:'custom-agent',session_id:session,turn_id:turn,event_id:'native-'+(++seq),kind,payload,...extra};
  const result=spawnSync(exe,['--integration-event','--source','lifecycle-fixture','--adapter','standard-events'],{env:{...env,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:''},input:JSON.stringify(body),encoding:'utf8',windowsHide:true,timeout:10000});
  assert.equal(result.status,0,result.stderr);
}
try{
  const root=await page();await until(()=>root.evaluate(`!!document.querySelector('.dashboard')`));
  assert.equal((await root.invoke('integration_environment')).isolated,true);
  event('one','a','turn.started',{title:'合成：任务一',project:'Fixture Project A'});
  event('two','a','turn.started',{title:'合成：任务二',project:'Fixture Project B'});
  event('one','a','phase.changed',{phase:'thinking'});
  await until(async()=> (await root.invoke('get_agents')).filter((t:any)=>t.status==='running').length===2);
  const size=await root.evaluate(`({height:innerHeight,panel:document.querySelector('.dashboard').getBoundingClientRect().height,list:document.querySelector('.task-scroll').clientHeight,available:screen.availHeight})`);
  assert.ok(size.height>=Math.min(700,Math.floor(size.available*.8))-2,JSON.stringify(size));
  assert.ok(size.list>=200,JSON.stringify(size));
  assert.equal((await root.invoke('get_task_notice')).current,null,'Thinking must not notify');
  // UI boundary injection only: backend Provider→Store→Attention fault/recovery
  // is covered by the Rust WSL continuity pipeline regression. Do not mutate
  // a real WSL source or install Hooks to exercise uncertainty rendering.
  const runningTasks=await root.invoke('get_agents');
  const staleTasks=runningTasks.map((t:any)=>({...t,stale:true}));
  let staleEvidence:any=null;
  // The real fixture worker restores fresh Running every second. Capture a
  // coherent before/after pair; do not freeze IPC, stop workers or make loose
  // assertions against DOM nodes from different observations.
  for(let attempt=0;attempt<5 && !staleEvidence;attempt++) {
    await root.invoke('plugin:event|emit',{event:'agents-updated',payload:staleTasks});
    await until(()=>root.evaluate(`!!document.querySelector('.pending-running')`));
    const before=await root.evaluate(`!!document.querySelector('.pending-running button')`);
    if(!before)continue;
    const shot=await root.send('Page.captureScreenshot',{format:'png'});
    const observed=await root.evaluate(`(()=>{const button=document.querySelector('.pending-running button');if(!button)return null;const snapshot={counts:JSON.parse(document.querySelector('.dashboard').dataset.continuity),ongoing:document.querySelector('.counts .overview b')?.textContent,hint:document.querySelector('.pending-running')?.textContent,rows:document.querySelector('.task-scroll')?.textContent};button.click();return snapshot;})()`);
    if(!observed)continue;
    assert.equal(observed.ongoing,'0');assert.equal(observed.counts.global.ongoing,0);
    assert.equal(observed.counts.global.unconfirmed_running,2);
    assert.ok(observed.hint.includes('上次运行 2'));assert.ok(observed.rows.includes('上次确认为运行中'));
    staleEvidence=observed.counts;
    await Bun.write(join(owned,'uncertain-running.png'),Buffer.from(shot.data,'base64'));
  }
  assert.ok(staleEvidence,'No coherent stale Running observation within five bounded attempts');
  assert.equal(await root.evaluate(`document.querySelector('.counts .overview.active span')?.textContent`),'待更新');
  await root.invoke('plugin:event|emit',{event:'agents-updated',payload:runningTasks});
  await until(()=>root.evaluate(`!document.querySelector('.pending-running')`));
  await root.evaluate(`document.querySelector('.overview.total').click()`);
  assert.equal((await root.invoke('get_task_notice')).current,null,'Uncertainty/recovery must not fabricate a completion');
  event('one','a','input.requested',{request_kind:'question'},{request_id:'q1'});
  await until(async()=> Boolean((await root.invoke('get_task_notice')).current));
  const alert=await page(true);await until(()=>alert.evaluate(`document.querySelector('.summary')?.textContent.includes('需处理：等待输入')`));
  assert.equal((await root.invoke('get_task_notice')).current.auto_dismiss_ms,null);
  assert.equal((await root.invoke('get_task_notice')).current.context.task_name,'合成：任务一');
  assert.equal(await alert.evaluate(`document.querySelector('.summary')?.textContent.includes('本轮结束')`),false,'A question is not a completed turn');
  event('one','a','input.resolved',{}, {request_id:'q1'});
  await until(async()=> !(await root.invoke('get_task_notice')).current);
  event('one','a','turn.completed');
  await until(async()=> Boolean((await root.invoke('get_task_notice')).current));
  await until(()=>alert.evaluate(`document.querySelector('.summary')?.textContent.includes('本轮结束，等待后续输入')`));
  assert.equal((await root.invoke('get_task_notice')).current.auto_dismiss_ms,15000);
  event('one','b','turn.started');
  await until(async()=> !(await root.invoke('get_task_notice')).current);
  await root.invoke('test_notification',{kind:'quota_exhausted'});
  await until(()=>alert.evaluate(`document.querySelector('.summary')?.textContent.includes('额度已用完')`));
  const quota=await root.invoke('get_task_notice');assert.equal(quota.current.test,true);assert.equal(quota.current.task_id,null);assert.equal(quota.current.auto_dismiss_ms,null);
  await Bun.write(join(owned,'quota.png'),Buffer.from((await alert.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
  await root.invoke('dismiss_task_notice',{id:quota.current.id});
  await Bun.write(join(owned,'dashboard.png'),Buffer.from((await root.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
  const result={passed:true,size,staleDiagnostics:staleEvidence,checks:['two running tasks visible','thinking does not notify','stale Running stays visible as unconfirmed, with a working global recovery hint and no fake fresh running count','question says action required and waiting input for the matching task, without a completion label or timer','matching answer clears notice','real turn end has a separate completion label and timer','new turn withdraws obsolete result','quota test is sticky and never creates a task'],realHostTested:false};
  await Bun.write(join(owned,'result.json'),JSON.stringify(result,null,2));console.log(JSON.stringify({...result,evidence}));
}catch(error){await Bun.write(join(owned,'failure.json'),JSON.stringify({passed:false,error:String(error),stderr}));throw error;}
});
