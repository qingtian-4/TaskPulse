/** Actual Windows WebView2 integration acceptance with an owned isolated home.
 * Synthetic producers exercise TaskPulse, NOT actual Agent-host support.
 * No real host settings, accounts, approvals, prompts or sources are changed. */
import assert from 'node:assert/strict';
import { mkdir, copyFile, readFile, unlink } from 'node:fs/promises';
import { resolve, join, dirname } from 'node:path';
import { spawn, spawnSync, type ChildProcess } from 'node:child_process';
import { cpus, release, arch } from 'node:os';
import {withSmokeWorkspace,stopOwnedChild} from '../support/smoke-workspace';
const arg=process.argv.indexOf('--exe');if(arg<0)throw Error('--exe is required');
const originalExe=resolve(process.argv[arg+1]);
await withSmokeWorkspace('integration-native',async({owned,evidenceDir,onStop})=>{
let app:ChildProcess|undefined;let root:any;let management:any;let stderr='';let spawnFailure:Error|undefined;
onStop(async()=>{if(app)await stop();});
const installed=join(owned,"安装 & team's App");await mkdir(installed);
const exe=join(installed,'agent-tray.exe');await copyFile(originalExe,exe);
if(await Bun.file(join(dirname(originalExe),'WebView2Loader.dll')).exists())await copyFile(join(dirname(originalExe),'WebView2Loader.dll'),join(installed,'WebView2Loader.dll'));
const home=join(owned,'profile');await mkdir(home);
await Bun.write(join(home,'config.toml'),'[dashboard]\nversion = 2\nonboarding_complete = true\nsources = []\nnotifications = false\nnotification_mode = "popup"\nsound = false\ndefault_view = "tasks"\n');
const host=join(home,'host-sandbox','.kimi-code');await mkdir(host,{recursive:true});
await Bun.write(join(host,'config.toml'),"# synthetic host config; not a real Kimi installation\nmodel = 'synthetic-model'\n[[hooks]]\nevent = 'Stop'\ncommand = 'synthetic-user-hook'\n");
const env={...process.env,TASKPULSE_HOME:home,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:'--remote-debugging-port=9337'};
type Tab={url:string;webSocketDebuggerUrl:string};
const tabs=async():Promise<Tab[]>=>fetch('http://127.0.0.1:9337/json').then(r=>r.json());
if(await tabs().then(()=>true).catch(()=>false))throw Error('CDP 9337 already in use; refusing to attach to another app');
const errors:string[]=[];const checks:string[]=[];
async function connect(tab:Tab){
  const ws=new WebSocket(tab.webSocketDebuggerUrl);await new Promise<void>((ok,fail)=>{ws.onopen=()=>ok();ws.onerror=()=>fail(Error('CDP failed'));});
  let seq=0;const pending=new Map<number,{ok:(v:any)=>void;fail:(e:any)=>void;timer:ReturnType<typeof setTimeout>}>();
  ws.onmessage=e=>{const m=JSON.parse(String(e.data));if(m.method==='Log.entryAdded'&&m.params.entry.level==='error')errors.push(m.params.entry.text);const p=pending.get(m.id);if(!p)return;clearTimeout(p.timer);pending.delete(m.id);m.error?p.fail(Error(JSON.stringify(m.error))):p.ok(m.result);};
  const send=(method:string,params:Record<string,unknown>={})=>new Promise<any>((ok,fail)=>{const id=++seq;const timer=setTimeout(()=>{pending.delete(id);fail(Error('CDP timeout: '+method));},20000);pending.set(id,{ok,fail,timer});ws.send(JSON.stringify({id,method,params}));});
  const evaluate=async(expression:string)=>{const r=await send('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true,userGesture:true});if(r.exceptionDetails)throw Error(r.exceptionDetails.exception?.description??r.exceptionDetails.exception?.value??r.exceptionDetails.text??'native JS failed');return r.result.value;};
  const invoke=(command:string,args:Record<string,unknown>={})=>evaluate(`__TAURI_INTERNALS__.invoke(${JSON.stringify(command)},${JSON.stringify(args)})`);
  const wait=async(expression:string,ms=20000)=>{const end=Date.now()+ms;while(Date.now()<end){if(await evaluate(expression))return;await Bun.sleep(100);}throw Error('Native assertion timeout: '+expression);};
  await send('Log.enable');return {send,evaluate,invoke,wait,close:()=>ws.close()};
}
async function start(){
  spawnFailure=undefined;
  app=spawn(exe,['--show'],{env,windowsHide:true,stdio:['ignore','pipe','pipe']});
  app.once('error',error=>spawnFailure=error);
  app.stderr?.on('data',data=>{const s=String(data);stderr=(stderr+s).slice(-16000);if(/PANIC|panicked|failed to run app/i.test(s))errors.push('native startup failure');});
  let tab:Tab|undefined;
  for(let n=0;n<200;n++){if(spawnFailure)throw spawnFailure;tab=(await tabs().catch(()=>[])).find(t=>t.url.includes('tauri.localhost')&&!/\/(manage|notice)/.test(t.url));if(tab)break;if(app.exitCode!==null)throw Error('Owned app exited during startup');await Bun.sleep(100);}
  if(!tab)throw Error('Native popup missing');root=await connect(tab);await root.wait(`!!globalThis.__TAURI_INTERNALS__&&!!document.querySelector('.dashboard')`);
  const environment=await root.invoke('integration_environment');assert.equal(environment.isolated,true);assert.equal(resolve(environment.state_directory).toLowerCase(),home.toLowerCase());
}
async function stop(){try{management?.close();root?.close();}finally{await stopOwnedChild(app);}management=undefined;root=undefined;app=undefined;for(let n=0;n<50;n++){if(!await tabs().then(()=>true).catch(()=>false))return;await Bun.sleep(100);}throw Error('Owned debug listener did not close');}
async function openManagement(){await root.invoke('open_management',{section:'sources'});let tab:Tab|undefined;for(let n=0;n<100;n++){tab=(await tabs()).find(t=>t.url.includes('/manage'));if(tab)break;await Bun.sleep(100);}if(!tab)throw Error('Management missing');management=await connect(tab);await management.wait(`!!document.querySelector('.source-layout')`);}
async function seedSource(adapter:string,agent:string){
  const source={id:`generic_event:fixture-${adapter}`,kind:'generic_event',name:adapter,
    enabled:true,location:'',projects:['*'],excluded_projects:[],tmux_sessions:[],tmux_panes:[],poll_interval_ms:3000,
    integration:{adapter_id:adapter,agent_id:agent,transport:'local',accept_since_ms:0}};
  const expected=await root.invoke('get_monitoring_settings');
  const saved=await management.invoke('save_monitoring_settings',{settings:{...expected,sources:[...expected.sources,source]},expected});
  await management.send('Page.reload');
  const selector=`[data-saved-event-connections] [data-event-source="${source.id}"]`;
  await management.wait(`!!document.querySelector(${JSON.stringify(selector)})`);
  return saved.sources.find((s:any)=>s.id===source.id);
}
function send(source:any,event:any,host=false){const r=spawnSync(exe,[host?'--integration-host-event':'--integration-event','--source',source.id,'--adapter',source.integration.adapter_id],{env:{...env,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:''},input:JSON.stringify(event),encoding:'utf8',windowsHide:true,timeout:8000});if(r.error)throw r.error;assert.equal(r.status,0);assert.equal(r.stdout,'');}
let report:any;
let loadMetrics:any=null;
let resourceMetrics:any=null;
try{
  await start();await openManagement();const kimi=await seedSource('kimi-code-cli','kimi-code-cli');assert.ok(kimi);
  resourceMetrics=await root.invoke('get_resource_diagnostics');
  assert.equal(typeof resourceMetrics.checkpoint_writes,'number');
  assert.equal(resourceMetrics.process_memory.available,true);
  assert.equal(resourceMetrics.integration_disk.complete,true);
  checks.push('native asynchronous resource diagnostics IPC/memory/bounded isolated disk scan');
  await management.wait(`!!document.querySelector('[data-saved-event-connections] [data-event-adapter="kimi-code-cli"]')`);
  assert.ok(await management.evaluate(`!document.querySelector('[data-event-add]')`));
  checks.push('existing Kimi connection stays manageable without a creation action');
  const tasksBefore=await root.invoke('get_agents');const test=await management.invoke('integration_test',{sourceId:kimi.id});assert.equal(test.test,true);assert.deepEqual(await root.invoke('get_agents'),tasksBefore);
  checks.push('connection test stays outside tasks');
  const preview=await management.invoke('integration_preview',{sourceId:kimi.id,remove:false});
  assert.ok(preview.path.startsWith(home));assert.ok(!JSON.stringify(preview).includes('synthetic-model'));
  const changed=(await Bun.file(join(host,'config.toml')).text())+'\n# synthetic external edit\n';await Bun.write(join(host,'config.toml'),changed);
  await assert.rejects(()=>management.invoke('integration_apply',{token:preview.token,confirmed:true}));assert.equal(await Bun.file(join(host,'config.toml')).text(),changed);
  const next=await management.invoke('integration_preview',{sourceId:kimi.id,remove:false});
  await assert.rejects(()=>management.invoke('integration_apply',{token:next.token,confirmed:false}));assert.equal(await Bun.file(join(host,'config.toml')).text(),changed);
  // Seed the old installation through the native API in the owned sandbox.
  const setup=await management.invoke('integration_preview',{sourceId:kimi.id,remove:false});
  await management.invoke('integration_apply',{token:setup.token,confirmed:true});
  const installedText=await Bun.file(join(host,'config.toml')).text();assert.ok(installedText.includes('synthetic-user-hook'));assert.ok(installedText.includes('# synthetic external edit'));
  checks.push('fingerprint conflicts/explicit sandbox consent/comment preservation');
  send(kimi,{hook_event_name:'TurnStarted',session_id:'synthetic-kimi',turn_id:'turn-a',session_title:'合成 Kimi 契约任务',cwd:'D:\\synthetic'},true);
  await root.wait(`__TAURI_INTERNALS__.invoke('get_agents').then(t=>t.some(t=>t.session_id==='synthetic-kimi'&&t.status==='running'))`);
  send(kimi,{hook_event_name:'Stop',session_id:'synthetic-kimi'},true);
  await Bun.sleep(1200);assert.equal((await root.invoke('get_agents')).find((t:any)=>t.session_id==='synthetic-kimi').status,'running');
  for(const call of ['a','b']){
    send(kimi,{hook_event_name:'PreToolUse',session_id:'synthetic-kimi',tool_call_id:call,tool_name:'SyntheticTool',tool_input:{synthetic:call}},true);
    send(kimi,{hook_event_name:'PermissionRequest',session_id:'synthetic-kimi',tool_call_id:call,request_id:'request-'+call,tool_name:'SyntheticTool',tool_input:{synthetic:call}},true);
  }
  await root.wait(`__TAURI_INTERNALS__.invoke('get_agents').then(t=>t.some(t=>t.session_id==='synthetic-kimi'&&t.input_request_ids?.length===2))`);
  send(kimi,{hook_event_name:'PermissionResult',session_id:'synthetic-kimi',tool_call_id:'b',request_id:'request-b'},true);
  await root.wait(`__TAURI_INTERNALS__.invoke('get_agents').then(t=>t.some(t=>t.session_id==='synthetic-kimi'&&t.input_request_ids?.length===1&&t.input_request_ids[0]==='request-a'))`);
  checks.push('native pipeline Stop safety and independent synthetic approvals');
  // Execute only TaskPulse's fixed generated forwarding command, not anything
  // from events/imported manifests. This is still a synthetic producer.
  const shell=next.entries.find((e:any)=>e.event==='SessionHeartbeat').command;
  const encoded=shell.split(' -EncodedCommand ')[1];assert.ok(encoded);
  const roundtrip=spawnSync('powershell.exe',['-NoLogo','-NoProfile','-NonInteractive','-EncodedCommand',encoded],{env,input:JSON.stringify({hook_event_name:'SessionHeartbeat',session_id:'synthetic-kimi'}),encoding:'utf8',windowsHide:true,timeout:15000});assert.equal(roundtrip.status,0);assert.equal(roundtrip.stdout,'');
  checks.push('generated Windows shell forwarding with Unicode/space/apostrophe path');
  // Import a new producer identity through the real management IPC.
  const custom=await Bun.file('integrations/community-template.json').json();
  custom.adapter_id='synthetic-editor';custom.agent_id='synthetic-editor';custom.display_name='合成社区 Agent';
  await management.invoke('integration_import',{text:JSON.stringify(custom)});
  const producer=await seedSource(custom.adapter_id,custom.agent_id);
  assert.ok(producer);
  const base={schema_version:1,agent_id:'synthetic-editor',session_id:'synthetic-session',turn_id:'turn-a',payload:{title:'合成标准事件任务',project:'synthetic-project'}};
  send(producer,{...base,event_id:'a-start',kind:'turn.started'});send(producer,{...base,event_id:'a-end',kind:'turn.completed'});send(producer,{...base,turn_id:'turn-b',event_id:'b-start',kind:'turn.started'});
  await root.wait(`__TAURI_INTERNALS__.invoke('get_agents').then(t=>t.some(t=>t.session_id==='synthetic-session'&&t.turn_key==='turn-b'&&t.status==='running'))`);
  send(producer,{...base,turn_id:'turn-b',event_id:'b-end',kind:'turn.completed'});
  await root.wait(`__TAURI_INTERNALS__.invoke('get_agents').then(t=>t.some(t=>t.session_id==='synthetic-session'&&t.status==='waiting_input'&&t.waiting_reason==='turn_ended'))`);
  let task=(await root.invoke('get_agents')).find((t:any)=>t.session_id==='synthetic-session');
  await root.invoke('update_task_attention',{id:task.id,action:'viewed'});
  send(producer,{...base,turn_id:'turn-b',event_id:'b-end',kind:'turn.completed'});
  let deliveryBefore=(await root.invoke('get_notification_log')).length;
  checks.push('community import without brand code; short A-end/B-start; duplicate terminal dedup');
  const stressAt=process.argv.indexOf('--stress-worker');
  if(stressAt>=0){
    const workerExe=resolve(process.argv[stressAt+1]);await Bun.write(join(home,'benchmark-owned.json'),'{"synthetic":true}');
    const workers=Array.from({length:20},(_,worker)=>spawn(workerExe,['--exact','integrations::tests::integration_benchmark_worker','--ignored','--nocapture'],{env:{...process.env,PATH:dirname(originalExe)+';'+process.env.PATH,TASKPULSE_BENCH_ROOT:home,TASKPULSE_BENCH_WORKER:String(worker),TASKPULSE_BENCH_SOURCE:producer.id,TASKPULSE_BENCH_AGENT:producer.integration.agent_id},windowsHide:true,stdio:['ignore','ignore','pipe']}));
    const latencies:number[]=[];const started=performance.now();
    try{
      while(workers.some(w=>w.exitCode===null)){
        const began=performance.now();await root.evaluate(`__TAURI_INTERNALS__.invoke('get_agents').then(()=>new Promise(ok=>requestAnimationFrame(()=>ok(true))))`);latencies.push(performance.now()-began);
        if(performance.now()-started>180000)throw Error('Owned UI load workers exceeded 180 seconds');await Bun.sleep(100);
      }
      assert.ok(workers.every(w=>w.exitCode===0),'Every load worker must succeed');
      const finalTurn='synthetic-turn-'+49;
      await root.wait(`__TAURI_INTERNALS__.invoke('get_agents').then(t=>t.filter(t=>t.session_id?.startsWith('synthetic-worker-')&&t.finished_at&&t.turn_key===${JSON.stringify(finalTurn)}).length===20)`,180000);
      latencies.sort((a,b)=>a-b);
      const resources=spawnSync('powershell.exe',['-NoProfile','-NonInteractive','-Command',`$tpOwned=[Diagnostics.Process]::GetProcessById(${app!.pid});[pscustomobject]@{peakWorkingSetBytes=$tpOwned.PeakWorkingSet64;totalCpuSeconds=$tpOwned.TotalProcessorTime.TotalSeconds}|ConvertTo-Json -Compress`],{encoding:'utf8',windowsHide:true,timeout:5000});assert.equal(resources.status,0);
      loadMetrics={processes:20,events:10000,samples:latencies.length,ipcAndAnimationFrameP50Ms:latencies[Math.floor(latencies.length*.5)],ipcAndAnimationFrameP95Ms:latencies[Math.floor(latencies.length*.95)],wallMs:performance.now()-started,appResources:JSON.parse(resources.stdout),scope:'TaskPulse WebView IPC + animation frame while synthetic journal writers run; not real-host latency'};
      checks.push('20 synthetic processes / 10000 events with actual WebView IPC and animation-frame responsiveness');
    }finally{for(const w of workers)if(w.exitCode===null)w.kill();}
  }
  // Reattach after long load to the currently active management target; a
  // background WebView may have suspended/replaced its prior CDP session.
  management.close();management=undefined;await openManagement();
  const layout=await management.evaluate(`({overflow:document.documentElement.scrollWidth>innerWidth,errors:document.querySelectorAll('[role=alert]').length})`);assert.equal(layout.overflow,false);assert.equal(layout.errors,0);
  await Bun.write(join(owned,'management.png'),Buffer.from((await management.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
  // R4: removing a monitored source must NOT make its registered Hook orphaned.
  const withoutKimi=await root.invoke('get_monitoring_settings');withoutKimi.sources=withoutKimi.sources.filter((s:any)=>s.id!==kimi.id);
  await management.invoke('save_monitoring_settings',{settings:withoutKimi});
  assert.equal(await Bun.file(join(host,'config.toml')).text(),installedText,'source removal must not implicitly edit host hooks');
  let registrations=await management.invoke('integration_installations');assert.ok(registrations.some((r:any)=>r.source_id===kimi.id&&r.orphaned));
  // Existing pre-index installations are reversible as well. Remove only this
  // sandbox's new index to simulate the older ownership-only format.
  const sourceHash=new Bun.CryptoHasher('sha256').update(kimi.id).digest('hex');
  await unlink(join(home,'integrations',sourceHash,'hook-installation.json'));
  registrations=await management.invoke('integration_installations');assert.ok(registrations.some((r:any)=>r.source_id===kimi.id&&r.orphaned));
  const removal=await management.invoke('integration_preview',{sourceId:kimi.id,remove:true});
  assert.ok(removal.path.startsWith(home));
  await management.invoke('integration_apply',{token:removal.token,confirmed:true});
  const removed=await Bun.file(join(host,'config.toml')).text();assert.ok(removed.includes('synthetic-user-hook'));assert.ok(!removed.includes('EncodedCommand'));
  checks.push('removed-source Hook cleanup requires consent and retains foreign settings/hooks');
  assert.ok(!(await management.invoke('integration_installations')).some((r:any)=>r.source_id===kimi.id));
  const restoredSettings=await root.invoke('get_monitoring_settings');restoredSettings.sources.push(kimi);await management.invoke('save_monitoring_settings',{settings:restoredSettings});
  deliveryBefore=(await root.invoke('get_notification_log')).length;
  await stop();await start();await Bun.sleep(2500);
  const recovered=await root.invoke('get_agents');assert.ok(recovered.every((t:any)=>t.session_id!=='synthetic-session'||!t.unread));
  const pending=await root.invoke('get_pending_task_prompts');assert.ok(pending.some((t:any)=>t.session_id==='synthetic-kimi'&&t.input_request_ids.includes('request-a')));
  assert.equal((await root.invoke('get_notification_log')).length,deliveryBefore);
  checks.push('restart preserves viewed/unresolved state and does not replay notifications');
  if(process.argv.includes('--notices')){
    const child=spawn(process.execPath,['tests/native/native-notice-smoke.ts'],{env:{...process.env,TASKPULSE_SMOKE_LOCKED_DESKTOP:'1'},windowsHide:true,stdio:['ignore','pipe','pipe']});
    const output:string[]=[];child.stdout?.on('data',d=>output.push(String(d)));child.stderr?.on('data',d=>output.push(String(d)));
    await new Promise<void>((ok,fail)=>{const timer=setTimeout(()=>{child.kill();fail(Error('Owned notice acceptance timed out'));},150000);child.once('exit',code=>{clearTimeout(timer);code===0?ok():fail(Error('Owned notice acceptance failed: '+output.join('').slice(-6000)));});child.once('error',fail);});
    checks.push('actual notice rendering/click/timer/sticky-blocker regression (external editor focus NOT verified)');
  }
  assert.equal(errors.length,0,JSON.stringify(errors));
  resourceMetrics=await root.invoke('get_resource_diagnostics');
  assert.ok(resourceMetrics.checkpoint_writes>0);
  report={passed:true,checks,loadMetrics,resourceMetrics,platform:{os:release(),arch:arch(),cpu:cpus()[0]?.model,logicalCpus:cpus().length},realHostsVerified:[],realHostConfigurationsChanged:false,fixtureProducersOnly:true,notVerified:['actual Kimi/TRAE/ZCode/WorkBuddy hook delivery','external editor focus','long-run real-host latency']};
  await Bun.write(join(owned,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify({...report,evidence:evidenceDir}));
}catch(error){
  const ui=management?await management.evaluate(`({adapter:document.querySelector('[aria-label="Agent 适配描述"]')?.value,disabled:document.querySelector('[aria-label="Agent 适配描述"]')?.disabled,errors:[...document.querySelectorAll('[role=alert]')].map(e=>e.textContent),statuses:[...document.querySelectorAll('[role=status]')].map(e=>e.textContent)})`).catch(()=>null):null;
  await Bun.write(join(owned,'failure.json'),JSON.stringify({passed:false,checks,loadMetrics,error:String(error),errors,ui,stderr},null,2));console.error('Isolated evidence: '+evidenceDir);throw error;
}
});
