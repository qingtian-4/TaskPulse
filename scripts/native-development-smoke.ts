/** Actual default debug profile + WebView2/IPC; never installs or modifies startup. */
import assert from 'node:assert/strict';
import {copyFile,mkdir,readFile,lstat} from 'node:fs/promises';
import {resolve,join,dirname,sep} from 'node:path';
import {createServer} from 'node:net';
import {spawn,spawnSync,type ChildProcess} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {withSmokeWorkspace,stopOwnedChild,removeWithRetry} from './smoke-workspace';
const at=process.argv.indexOf('--exe');if(at<0)throw Error('--exe required');
const original=resolve(process.argv[at+1]);
await withSmokeWorkspace('development-native',async({owned,onStop})=>{
  const exe=join(owned,'agent-tray.exe');await copyFile(original,exe);
  const loader=join(dirname(original),'WebView2Loader.dll');if(await Bun.file(loader).exists())await copyFile(loader,join(owned,'WebView2Loader.dll'));
  const env={...process.env};delete env.TASKPULSE_HOME;
  const info=spawnSync(exe,['--profile-info'],{env,encoding:'utf8',windowsHide:true,timeout:15000});
  assert.equal(info.status,0,info.stderr);const profile=JSON.parse(info.stdout);
  assert.equal(profile.channel,'development');
  const home=process.env.USERPROFILE??process.env.HOME!;
  const production=join(home,'.taskpulse'),state=resolve(profile.state_directory);
  assert.ok(state.startsWith(join(home,'.taskpulse-dev')+sep));
  const files=['config.toml'];
  const readStable=()=>Promise.all(files.map(name=>readFile(join(production,name)).catch((e:NodeJS.ErrnoException)=>{if(e.code==='ENOENT')return null;throw e;})));
  // The installed app can legitimately persist attention while this check runs;
  // only its settings must remain byte-identical. No test writes a stable file.
  const before=await readStable();
  const forbidden=spawnSync(exe,['--profile-info'],{env:{...env,TASKPULSE_HOME:production},encoding:'utf8',windowsHide:true,timeout:15000});
  assert.notEqual(forbidden.status,0,'Debug must reject a production-directory override');
  assert.match(forbidden.stderr,/Development builds cannot use/);
  await mkdir(dirname(state),{recursive:true});
  await mkdir(state);const identity=await lstat(state,{bigint:true});
  let app:ChildProcess|undefined,ws:WebSocket|undefined,startupEvidence:any;
  onStop(async()=>{
    ws?.close();await stopOwnedChild(app);
    const failure=Bun.file(join(owned,'failure.json'));
    if(startupEvidence && await failure.exists())await Bun.write(failure,JSON.stringify({...await failure.json(),startup:startupEvidence},null,2));
    const current=await lstat(state,{bigint:true});
    assert.ok(!current.isSymbolicLink()&&current.dev===identity.dev&&current.ino===identity.ino,'Refuse cleanup of a replaced profile');
    await removeWithRetry(state);
  });
  const server=createServer();await new Promise<void>(ok=>server.listen(0,'127.0.0.1',ok));
  const port=(server.address() as {port:number}).port;await new Promise<void>(ok=>server.close(()=>ok()));
  let stderr='',spawnError='';app=spawn(exe,['--show'],{env:{...env,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port}`},windowsHide:true,stdio:['ignore','ignore','pipe']});
  app.stderr?.on('data',v=>stderr=(stderr+String(v)).slice(-2000));
  app.once('error',e=>spawnError=String(e));
  const started=performance.now(),deadline=started+24000;
  let target:any,pages:any[]=[],lastHttpStatus:number|undefined,lastCdpError='',attempts=0;
  while(performance.now()<deadline && !spawnError && app.exitCode===null){
    attempts++;
    try{
      const response=await fetch(`http://127.0.0.1:${port}/json`,{signal:AbortSignal.timeout(Math.max(1,Math.min(1000,Math.ceil(deadline-performance.now()))))});
      lastHttpStatus=response.status;const value=await response.json();pages=Array.isArray(value)?value:[];lastCdpError='';
      if(performance.now()<=deadline)target=pages.find(t=>t.url?.includes('tauri.localhost')&&!t.url.includes('/manage')&&!t.url.includes('/notice'));
    }catch(error){lastCdpError=String(error).slice(0,400);}
    if(target)break;await Bun.sleep(100);
  }
  const readinessMs=Math.round(performance.now()-started);
  // Collect after deciding readiness: diagnostics must not extend its deadline.
  let nativeProcess:any={unavailable:true};
  if(process.platform==='win32' && app.pid){
    const probe=spawnSync(join(process.env.SystemRoot??'C:/Windows','System32/WindowsPowerShell/v1.0/powershell.exe'),['-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File',fileURLToPath(new URL('./native-process-evidence.ps1',import.meta.url)),'-ProcessId',String(app.pid),'-Exe',exe,'-DebugPort',String(port)],{encoding:'utf8',windowsHide:true,timeout:20000,maxBuffer:65536});
    try{nativeProcess=probe.status===0?JSON.parse(probe.stdout):{error:probe.error?.message??probe.stderr.slice(-1500),exit_code:probe.status};}catch{nativeProcess={error:'Invalid native process evidence'};}
  }
  let browser:any;
  try{const value:any=await fetch(`http://127.0.0.1:${port}/json/version`,{signal:AbortSignal.timeout(1000)}).then(r=>r.json());browser={browser:value.Browser,protocol:value['Protocol-Version']};}catch{/* The absence of this endpoint is retained in startup evidence. */}
  const startup=startupEvidence={commit:profile.commit,deadline_ms:24000,readiness_ms:readinessMs,elapsed_ms:Math.round(performance.now()-started),attempts,pid:app.pid,exit_code:app.exitCode,spawn_error:spawnError,stderr,last_http_status:lastHttpStatus,last_cdp_error:lastCdpError,pages:pages.slice(0,8).map(t=>({type:String(t.type??'').slice(0,32),route:String(t.url??'').split(/[?#]/)[0].slice(0,256)})),process:nativeProcess,browser};
  if(!target)await Bun.write(join(owned,'failure.json'),JSON.stringify({passed:false,phase:'native-startup',error:'Native development popup is missing',startup},null,2));
  assert.ok(target,'Native development popup is missing');
  ws=new WebSocket(target.webSocketDebuggerUrl);await new Promise<void>((ok,fail)=>{ws!.onopen=()=>ok();ws!.onerror=()=>fail(Error('CDP unavailable'));});
  let id=0;const pending=new Map<number,any>();
  ws.onmessage=e=>{const m=JSON.parse(String(e.data)),p=pending.get(m.id);if(!p)return;clearTimeout(p.timer);pending.delete(m.id);m.error?p.fail(Error(JSON.stringify(m.error))):p.ok(m.result);};
  const evaluate=(expression:string)=>new Promise<any>((ok,fail)=>{const next=++id,timer=setTimeout(()=>{pending.delete(next);fail(Error('CDP timed out'));},15000);pending.set(next,{timer,fail,ok:(r:any)=>r.exceptionDetails?fail(Error(r.exceptionDetails.text)):ok(r.result.value)});ws!.send(JSON.stringify({id:next,method:'Runtime.evaluate',params:{expression,awaitPromise:true,returnByValue:true}}));});
  for(let n=0;n<200;n++){if(await evaluate(`typeof __TAURI_INTERNALS__!=='undefined'&&!!document.querySelector('.dashboard')`))break;await Bun.sleep(100);}
  const result=await evaluate(`(async()=>{const i=__TAURI_INTERNALS__.invoke;return {build:await i('get_build_info'),environment:await i('integration_environment'),startup:await i('get_autostart'),settings:await i('get_monitoring_settings'),tasks:await i('get_agents')}})()`);
  assert.equal(result.build.process_id,app.pid,'CDP must belong to the exact owned native process');
  if(process.platform==='win32')assert.equal(nativeProcess.owned_executable,true,JSON.stringify(nativeProcess));
  const hookDenied=await evaluate(`__TAURI_INTERNALS__.invoke('configure_claude_events',{sourceId:'nonexistent-fixture-source',enabled:true,confirmed:true}).then(()=>'',e=>String(e))`);
  assert.match(hookDenied,/installed TaskPulse/);
  assert.equal(result.environment.isolated,true);assert.equal(result.environment.state_directory,profile.state_directory);
  assert.equal(result.build.channel,'development');assert.equal(result.settings.onboarding_complete,false);
  assert.equal(result.startup.available,false);assert.equal(result.tasks.length,0);
  const after=await readStable();assert.ok(before[0]===null?after[0]===null:after[0]!==null&&before[0].equals(after[0]),'Installed settings changed');
  await Bun.write(join(owned,'result.json'),JSON.stringify({passed:true,commit:result.build.commit,channel:result.build.channel,automaticProfileIsolation:true,onboardingRequired:true,importedTasks:0,startupDisabled:true,realHookConfigurationBlocked:true,productionDirectoryOverrideBlocked:true,productionSettingsUnchanged:true,startup}));
  console.log('Native default development profile, onboarding, IPC and production-settings isolation passed');
});
