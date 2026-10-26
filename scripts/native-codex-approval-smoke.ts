import assert from 'node:assert/strict';
import {copyFile, mkdir, rename, readFile} from 'node:fs/promises';
import {dirname, join, resolve} from 'node:path';
import {spawn, spawnSync, type ChildProcess} from 'node:child_process';
import {withSmokeWorkspace, stopOwnedChild} from './smoke-workspace';

const index=process.argv.indexOf('--exe');
if(index<0)throw Error('--exe required');
const original=resolve(process.argv[index+1]);
await withSmokeWorkspace('codex-approval-native',async({owned,evidenceDir,onStop})=>{
  const home=join(owned,'profile');await mkdir(home);
  const exe=join(owned,'agent-tray.exe');await copyFile(original,exe);
  const loader=join(dirname(original),'WebView2Loader.dll');
  if(await Bun.file(loader).exists())await copyFile(loader,join(owned,'WebView2Loader.dll'));
  const config=`[dashboard]
version=2
onboarding_complete=true
notifications=true
sound=false
notification_mode="popup"
[[dashboard.sources]]
id="codex"
name="Public Codex fixture"
kind="codex"
enabled=true
location=""
projects=["*"]
excluded_projects=[]
tmux_sessions=[]
tmux_panes=[]
poll_interval_ms=1000
`
  await Bun.write(join(home,'config.toml'),config);
  const port=9346;
  assert.equal(await fetch(`http://127.0.0.1:${port}/json`).then(()=>true).catch(()=>false),false,'CDP port occupied');
  const codex=join(owned,'codex');await mkdir(join(codex,'sessions'),{recursive:true});
  const rollout=join(codex,'sessions/rollout-public-approval.jsonl');
  const initial=[{type:'session_meta',payload:{id:'public-approval',cwd:'D:/Public Fixture'}},{timestamp:new Date().toISOString(),type:'event_msg',payload:{type:'task_started',turn_id:'public-turn'}}].map(v=>JSON.stringify(v)).join('\n')+'\n';
  await Bun.write(rollout,initial);
  const env={...process.env,CODEX_HOME:codex,TASKPULSE_HOME:home,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port}`};
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
  await until(`__TAURI_INTERNALS__.invoke('get_agents').then(t=>t.some(t=>t.session_id==='public-approval'&&t.status==='running'))`);
  const python=process.env.TASKPULSE_PYTHON??'python';
  function hook(kind:string,call?:string){
    const input={session_id:'public-approval',turn_id:'public-turn',hook_event_name:kind,tool_use_id:call,tool_name:'Bash',tool_input:{command:'public-command'}};
    const result=spawnSync(python,[resolve('scripts/codex-observer.py')],{env,input:JSON.stringify(input),encoding:'utf8',windowsHide:true,timeout:10000});
    assert.equal(result.status,0,result.stderr);
    assert.equal(result.stdout,'','Observer must never send an approval decision');
  }
  hook('PreToolUse','public-call');hook('PermissionRequest');
  await until(`__TAURI_INTERNALS__.invoke('get_agents').then(t=>t.some(t=>t.session_id==='public-approval'&&t.waiting_reason==='approval'))`);
  await until(`__TAURI_INTERNALS__.invoke('get_task_notice').then(n=>n.current?.task_id==='codex:public-approval')`);
  const notice=await invoke('get_task_notice');
  assert.equal(notice.current.auto_dismiss_ms,null);
  await until(`__TAURI_INTERNALS__.invoke('get_notification_log').then(rows=>rows.some(r=>r.task_id==='codex:public-approval'&&r.stage==='rendered'))`);
  hook('PostToolUse','unrelated');
  await Bun.sleep(1600);
  assert.equal((await invoke('get_agents')).find((t:any)=>t.session_id==='public-approval').waiting_reason,'approval');
  hook('PostToolUse','public-call');
  await until(`__TAURI_INTERNALS__.invoke('get_agents').then(t=>t.some(t=>t.session_id==='public-approval'&&t.status==='running'))`);
  await until(`__TAURI_INTERNALS__.invoke('get_task_notice').then(n=>!n.current)`);
  const report={passed:true,syntheticOnly:true,realAgentAcceptance:false,renderer:'native WebView2',approvalRendered:true,unmatchedReturnPreserved:true,matchingReturnCleared:true,build:await invoke('get_build_info'),evidenceDir};
  await Bun.write(join(owned,'result.json'),JSON.stringify(report,null,2));
  console.log(JSON.stringify(report));
});
