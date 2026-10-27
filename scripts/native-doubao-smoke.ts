/** Opt-in, read-only real Doubao -> isolated native TaskPulse GUI acceptance.
 * Does not install hooks, restart Doubao or change the real TaskPulse profile.
 * bun scripts/native-doubao-smoke.ts --exe <unused-built-exe> [--wait-seconds 120]
 * --startup-only explicitly limits acceptance to startup/source reads, NOT task UI.
 */
import assert from 'node:assert/strict';
import {mkdir,copyFile} from 'node:fs/promises';
import {resolve,join,dirname} from 'node:path';
import {spawn,type ChildProcess} from 'node:child_process';
import {withSmokeWorkspace,stopOwnedChild} from './smoke-workspace';
const arg=process.argv.indexOf('--exe');assert(arg>=0,'--exe required');
const original=resolve(process.argv[arg+1]);
const requireRunning=!process.argv.includes('--startup-only');
assert(!(process.argv.includes('--startup-only')&&process.argv.includes('--require-running')),'Conflicting acceptance scopes');
const waitArg=process.argv.indexOf('--wait-seconds');const waitSeconds=waitArg<0?120:Number(process.argv[waitArg+1]);
assert(Number.isFinite(waitSeconds)&&waitSeconds>=5&&waitSeconds<=300,'--wait-seconds must be 5..300');
await withSmokeWorkspace('doubao-native',async({owned,evidenceDir,onStop})=>{
  let app:ChildProcess|undefined;let ws:WebSocket|undefined;
  onStop(async()=>{ws?.close();await stopOwnedChild(app);});
  const home=join(owned,'profile');await mkdir(home);
  const exe=join(owned,'agent-tray.exe');await copyFile(original,exe);
  const loader=join(dirname(original),'WebView2Loader.dll');if(await Bun.file(loader).exists())await copyFile(loader,join(owned,'WebView2Loader.dll'));
  await Bun.write(join(home,'config.toml'),`[dashboard]
version=2
onboarding_complete=true
notifications=false
sound=false
[[dashboard.sources]]
id="doubao"
name="豆包 · 本机（实验）"
kind="doubao"
enabled=true
location=""
projects=["*"]
poll_interval_ms=1000
`);
  const endpoint='http://127.0.0.1:9338/json';
  assert.equal(await fetch(endpoint).then(()=>true).catch(()=>false),false,'CDP port occupied');
  app=spawn(exe,['--show'],{windowsHide:true,stdio:'ignore',env:{...process.env,TASKPULSE_HOME:home,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:'--remote-debugging-port=9338'}});
  let failure:Error|undefined;app.once('error',e=>failure=e);
  let url='';
  for(let i=0;i<200;i++){
    if(failure)throw failure;
    const tabs:any[]=await fetch(endpoint).then(r=>r.json()).catch(()=>[]);
    url=tabs.find(t=>t.url.includes('tauri.localhost')&&!/\/(manage|notice)/.test(t.url))?.webSocketDebuggerUrl??'';
    if(url)break;await Bun.sleep(100);
  }
  assert(url,'Native popup unavailable');ws=new WebSocket(url);
  await new Promise<void>((ok,fail)=>{ws!.onopen=()=>ok();ws!.onerror=()=>fail(Error('CDP connect'));});
  let sequence=0;
  const evaluate=(expression:string)=>new Promise<any>((ok,fail)=>{
    const id=++sequence;const timer=setTimeout(()=>{ws!.removeEventListener('message',handler);fail(Error('Native IPC timeout'));},15000);
    const handler=(e:MessageEvent)=>{const m=JSON.parse(String(e.data));if(m.id!==id)return;clearTimeout(timer);ws!.removeEventListener('message',handler);if(m.error||m.result?.exceptionDetails)fail(Error(String(m.error?.message??m.result.exceptionDetails.exception?.description??m.result.exceptionDetails.text).slice(0,300)));else ok(m.result.result.value);};
    ws!.addEventListener('message',handler);ws!.send(JSON.stringify({id,method:'Runtime.evaluate',params:{expression,awaitPromise:true,returnByValue:true}}));
  });
  let snapshot:any;let visible=false;let firstVisible=0;let streakId='';let runningSamples=0;
  const freshReads=new Set<number>();const progress=new Set<number>();
  const began=Date.now();
  while(Date.now()-began<waitSeconds*1000){
    snapshot=await evaluate(`globalThis.__TAURI_INTERNALS__?.invoke ? __TAURI_INTERNALS__.invoke('get_dashboard_snapshot') : null`);
    const health=snapshot?.providers?.find((p:any)=>p.provider==='doubao');
    const active=snapshot?.tasks?.find((t:any)=>t.source==='doubao'&&t.status==='running'&&!t.stale);
    if(!requireRunning&&health?.status==='connected'&&health.last_success)break;
    visible=!!active&&await evaluate(`(()=>{const row=[...document.querySelectorAll('.task-row')].find(r=>r.dataset.taskId===${JSON.stringify(active?.id)});if(!row)return false;const rect=row.getBoundingClientRect();return rect.width>0&&rect.height>0&&rect.top<innerHeight&&rect.bottom>0&&row.textContent.includes('豆包')&&row.querySelector('.status-label')?.textContent==='运行中';})()`);
    if(visible&&health?.status==='connected'&&health.last_success){
      if(streakId!==active.id){firstVisible=Date.now();streakId=active.id;freshReads.clear();progress.clear();runningSamples=0;}
      freshReads.add(health.last_success);progress.add(active.updated_at);runningSamples++;
      if(Date.now()-firstVisible>=5000&&freshReads.size>=3)break;
    }else{firstVisible=0;streakId='';freshReads.clear();progress.clear();runningSamples=0;}
    await Bun.sleep(500);
  }
  assert(snapshot,'Native IPC did not initialize');
  const tasks=(snapshot.tasks??[]).filter((t:any)=>t.source==='doubao');
  const health=snapshot.providers?.find((p:any)=>p.provider==='doubao');
  assert(health?.status==='connected'&&health.last_success,'Doubao source did not complete a healthy native read');
  const running=tasks.filter((t:any)=>t.status==='running'&&!t.stale);
  if(requireRunning)assert(running.length>0&&visible&&firstVisible>0&&Date.now()-firstVisible>=5000&&freshReads.size>=3,'No sustained real running Doubao task rendered; empty-source reads are not task UI acceptance');
  assert(tasks.every((t:any)=>t.agent_id==='doubao'&&t.session_id&&t.turn_key&&t.source_instance==='doubao'&&!t.input_request_id),'Native identity/provenance');
  // Persist counts only; do not include real session IDs, prompts or messages.
  const build=await evaluate(`__TAURI_INTERNALS__.invoke('get_build_info')`);
  await Bun.write(join(owned,'result.json'),JSON.stringify({passed:true,verification_scope:requireRunning?'live_running_ui':'startup_and_source_read',running_ui_verified:requireRunning,read_only:true,real_source:true,build,tasks:tasks.length,running:running.length,visible,running_samples:runningSamples,distinct_fresh_reads:freshReads.size,progress_observed:progress.size>1,approval_verified:false,completion_verified:false},null,2));
  console.log(JSON.stringify({passed:true,tasks:tasks.length,running:running.length,visible,evidenceDir}));
});
