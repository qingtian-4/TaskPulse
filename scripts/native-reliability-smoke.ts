import assert from 'node:assert/strict';
import {copyFile, mkdir, rename, readFile} from 'node:fs/promises';
import {dirname, join, resolve} from 'node:path';
import {spawn, spawnSync, type ChildProcess} from 'node:child_process';
import {withSmokeWorkspace, stopOwnedChild} from './smoke-workspace';

const index=process.argv.indexOf('--exe');
if(index<0)throw Error('--exe required');
const original=resolve(process.argv[index+1]);
await withSmokeWorkspace('reliability-native',async({owned,evidenceDir,onStop})=>{
  const home=join(owned,'profile');await mkdir(home);
  const exe=join(owned,'agent-tray.exe');await copyFile(original,exe);
  const loader=join(dirname(original),'WebView2Loader.dll');
  if(await Bun.file(loader).exists())await copyFile(loader,join(owned,'WebView2Loader.dll'));
  const config=`[dashboard]
version=2
onboarding_complete=true
notifications=false
sound=false
notification_mode="popup"
default_view="tasks"
[[dashboard.sources]]
id="review-public"
name="审查隔离合成来源"
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
accept_since_ms=${Date.now()}
`;
  await Bun.write(join(home,'config.toml'),config);
  const port=9345;
  assert.equal(await fetch(`http://127.0.0.1:${port}/json`).then(()=>true).catch(()=>false),false,'CDP port occupied');
  const env={...process.env,TASKPULSE_HOME:home,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port}`};
  let app:ChildProcess|undefined,second:ChildProcess|undefined,ws:WebSocket|undefined;
  onStop(async()=>{ws?.close();await stopOwnedChild(second);await stopOwnedChild(app);});
  app=spawn(exe,['--show'],{env,windowsHide:true,stdio:['ignore','ignore','pipe']});
  let startError:Error|undefined;app.once('error',e=>startError=e);
  let tab:any;
  for(let n=0;n<160;n++){
    if(startError)throw startError;
    tab=(await fetch(`http://127.0.0.1:${port}/json`).then(r=>r.json()).catch(()=>[])).find((t:any)=>t.url.includes('tauri.localhost')&&!/\/(manage|notice)/.test(t.url));
    if(tab)break;await Bun.sleep(150);
  }
  if(!tab)throw Error('No native WebView2 target; child exit='+app.exitCode);
  ws=new WebSocket(tab.webSocketDebuggerUrl);
  await new Promise<void>((ok,fail)=>{ws!.onopen=()=>ok();ws!.onerror=()=>fail(Error('CDP connect failed'));});
  let seq=0;const pending=new Map<number,{ok:(v:any)=>void;fail:(e:any)=>void;timer:ReturnType<typeof setTimeout>}>();
  ws.onmessage=e=>{const m=JSON.parse(String(e.data)),p=pending.get(m.id);if(!p)return;pending.delete(m.id);clearTimeout(p.timer);m.error?p.fail(Error(JSON.stringify(m.error))):p.ok(m.result);};
  const send=(method:string,params:Record<string,unknown>={})=>new Promise<any>((ok,fail)=>{const id=++seq;pending.set(id,{ok,fail,timer:setTimeout(()=>{pending.delete(id);fail(Error('CDP timeout '+method));},10000)});ws!.send(JSON.stringify({id,method,params}));});
  const evaluate=async(expression:string)=>{const r=await send('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true,userGesture:true});if(r.exceptionDetails)throw Error(JSON.stringify(r.exceptionDetails));return r.result.value;};
  const invoke=(cmd:string,args:Record<string,unknown>={})=>evaluate(`__TAURI_INTERNALS__.invoke(${JSON.stringify(cmd)},${JSON.stringify(args)})`);
  const until=async(expression:string)=>{for(let n=0;n<120;n++){if(await evaluate(expression))return;await Bun.sleep(100);}throw Error('Native condition timed out: '+expression);};
  await until(`!!document.querySelector('.dashboard')&&!!globalThis.__TAURI_INTERNALS__`);
  if(!await invoke('get_pinned'))await invoke('toggle_pin');
  assert.equal((await invoke('integration_environment')).isolated,true);
  function event(kind:string,eventId:string){
    const data={schema_version:1,agent_id:'custom-agent',session_id:'review-result',turn_id:'turn-1',event_id:eventId,kind,payload:kind==='turn.started'?{title:'审查合成结果：持久化故障验证',project:'Public Review'}:{}};
    const r=spawnSync(exe,['--integration-event','--source','review-public','--adapter','standard-events'],{env:{...env,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:''},input:JSON.stringify(data),encoding:'utf8',windowsHide:true,timeout:10000});
    assert.equal(r.status,0,r.stderr);
  }
  event('turn.started','review-start');event('turn.completed','review-done');
  await until(`__TAURI_INTERNALS__.invoke('get_agents').then(t=>t.some(t=>t.session_id==='review-result'&&t.waiting_reason==='turn_ended'))`);
  const task=(await invoke('get_agents')).find((t:any)=>t.session_id==='review-result');
  assert.equal(task.unread,true);
  await evaluate(`(()=>{const bell=document.querySelector('[aria-label^="消息中心："]');bell.focus();bell.click();})()`);
  await until(`!!document.querySelector('.message-center')`);
  const focusAfterOpen=await evaluate(`({inside:document.querySelector('.message-center').contains(document.activeElement),label:document.activeElement?.getAttribute('aria-label')})`);
  await send('Input.dispatchKeyEvent',{type:'keyDown',key:'Tab',code:'Tab',windowsVirtualKeyCode:9});
  await send('Input.dispatchKeyEvent',{type:'keyUp',key:'Tab',code:'Tab',windowsVirtualKeyCode:9});
  const tabAfterOpen=await evaluate(`({inside:document.querySelector('.message-center').contains(document.activeElement),label:document.activeElement?.getAttribute('aria-label')})`);
  await send('Input.dispatchKeyEvent',{type:'keyDown',key:'Escape',code:'Escape',windowsVirtualKeyCode:27});
  await send('Input.dispatchKeyEvent',{type:'keyUp',key:'Escape',code:'Escape',windowsVirtualKeyCode:27});
  const messageStillOpen=await evaluate(`!!document.querySelector('.message-center')`);
  await Bun.write(join(owned,'message-keyboard.png'),Buffer.from((await send('Page.captureScreenshot',{format:'png'})).data,'base64'));
  assert.equal(focusAfterOpen.inside,true);
  assert.equal(tabAfterOpen.inside,true);
  assert.equal(messageStillOpen,false);
  assert.equal(await evaluate(`document.activeElement?.getAttribute('aria-label')?.startsWith('消息中心：')`),true);
  const build=await invoke('get_build_info');
  assert.ok(build.commit&&build.executable);
  const before=await invoke('get_dashboard_snapshot');
  assert.equal(await invoke('update_task_attention',{id:task.id,action:'pin',pinned:true}),null);
  const after=await invoke('get_dashboard_snapshot');
  assert.ok(after.revision>before.revision);
  const responseRace={versionedSnapshot:true,revision:after.revision};
  // Same configuration directory: a second GUI process should hand off or refuse.
  second=spawn(exe,[],{env:{...env,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:''},windowsHide:true,stdio:'ignore'});
  let secondError:string|null=null;second.once('error',e=>secondError=String(e));
  await Bun.sleep(2500);
  const duplicateInstance={firstPid:app.pid,secondPid:second.pid,secondExit:second.exitCode,secondError,bothAlive:app.exitCode===null&&second.exitCode===null&&!secondError};
  assert.equal(duplicateInstance.bothAlive,false);
  assert.equal(duplicateInstance.secondExit,0);
  await stopOwnedChild(second);
  // Force atomic-replacement failure exclusively inside this owned fixture.
  await invoke('update_task_attention',{id:task.id,action:'restore'});
  const attention=join(home,'attention.json'),backup=join(home,'attention-before-fault.json');
  const persistedBefore=JSON.parse(await readFile(attention,'utf8'));
  assert.equal(persistedBefore.records[task.id].unread,true);
  await rename(attention,backup);await mkdir(attention);
  let actionRejected=false,actionError:string|null=null,response:any;
  try{response=await invoke('update_task_attention',{id:task.id,action:'viewed'});}catch(e){actionRejected=true;actionError=String(e);}
  const disk=JSON.parse(await readFile(backup,'utf8'));
  const persistenceFault={actionRejected,actionError,returnedTaskCount:Array.isArray(response)?response.length:null,persistedUnread:disk.records[task.id].unread,note:'Atomic persist destination replaced by directory in isolated TASKPULSE_HOME; user state untouched.'};
  assert.equal(actionRejected,true);
  assert.equal((await invoke('get_agents')).find((t:any)=>t.id===task.id).unread,true);
  const report={passed:true,build,renderer:'native Windows Tauri WebView2',syntheticOnly:true,evidenceDir,focusAfterOpen,tabAfterOpen,messageStillOpen,responseRace,duplicateInstance,persistenceFault};
  await Bun.write(join(owned,'result.json'),JSON.stringify(report,null,2));
  console.log(JSON.stringify(report,null,2));
});
