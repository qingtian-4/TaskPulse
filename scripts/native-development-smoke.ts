/** Actual default debug profile + WebView2/IPC; never installs or modifies startup. */
import assert from 'node:assert/strict';
import {copyFile,mkdir,readFile,lstat} from 'node:fs/promises';
import {resolve,join,dirname,sep} from 'node:path';
import {createServer} from 'node:net';
import {spawn,spawnSync,type ChildProcess} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {windowsTestProcess} from './windows-test-process';
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
  let app:ChildProcess|undefined,startupEvidence:any;
  const sockets:WebSocket[]=[];
  onStop(async()=>{
    sockets.forEach(ws=>ws.close());await stopOwnedChild(app);
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
  let childDiagnostics:unknown;
  if(process.platform==='win32' && app.pid){
    const powershell=join(process.env.SystemRoot??'C:/Windows','System32/WindowsPowerShell/v1.0/powershell.exe');
    const script=fileURLToPath(new URL('./native-process-evidence.ps1',import.meta.url));
    const args=['-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File',script,'-ProcessId',String(app.pid),'-Exe',exe];
    async function collect(extra:string[],timeout:number){
      try{
        const probe=await windowsTestProcess(powershell,[...args,...extra],timeout);
        if(probe.exitCode!==0)return {error:probe.stderr.slice(-1500),exit_code:probe.exitCode,timings:probe.timings};
        try{return {...JSON.parse(probe.stdout),timings:probe.timings};}
        catch{return {error:'Invalid native process evidence',timings:probe.timings};}
      }catch(error){return {error:String(error)};}
    }
    nativeProcess=await collect([],20000);
    // CIM is only useful for diagnosing a missing browser target. Run it in
    // its own bounded process so it cannot suppress the identity evidence.
    if(!target)childDiagnostics=await collect(['-ChildrenOnly','-DebugPort',String(port)],5000);
  }
  let browser:any;
  try{const value:any=await fetch(`http://127.0.0.1:${port}/json/version`,{signal:AbortSignal.timeout(1000)}).then(r=>r.json());browser={browser:value.Browser,protocol:value['Protocol-Version']};}catch{/* The absence of this endpoint is retained in startup evidence. */}
  const startup=startupEvidence={commit:profile.commit,deadline_ms:24000,readiness_ms:readinessMs,elapsed_ms:Math.round(performance.now()-started),attempts,pid:app.pid,exit_code:app.exitCode,spawn_error:spawnError,stderr,last_http_status:lastHttpStatus,last_cdp_error:lastCdpError,pages:pages.slice(0,8).map(t=>({type:String(t.type??'').slice(0,32),route:String(t.url??'').split(/[?#]/)[0].slice(0,256)})),process:nativeProcess,child_diagnostics:childDiagnostics,browser};
  if(!target)await Bun.write(join(owned,'failure.json'),JSON.stringify({passed:false,phase:'native-startup',error:'Native development popup is missing',startup},null,2));
  assert.ok(target,'Native development popup is missing');
  async function connect(page:{webSocketDebuggerUrl:string}){
    const ws=new WebSocket(page.webSocketDebuggerUrl);sockets.push(ws);
    await new Promise<void>((ok,fail)=>{
      const timer=setTimeout(()=>fail(Error('CDP connection timed out')),5000);
      ws.onopen=()=>{clearTimeout(timer);ok();};
      ws.onerror=()=>{clearTimeout(timer);fail(Error('CDP unavailable'));};
    });
    let id=0;const pending=new Map<number,any>();
    ws.onmessage=e=>{const m=JSON.parse(String(e.data)),p=pending.get(m.id);if(!p)return;clearTimeout(p.timer);pending.delete(m.id);m.error?p.fail(Error(JSON.stringify(m.error))):p.ok(m.result);};
    return (expression:string)=>new Promise<any>((ok,fail)=>{
      const next=++id,timer=setTimeout(()=>{pending.delete(next);fail(Error('CDP timed out'));},15000);
      pending.set(next,{timer,fail,ok:(r:any)=>r.exceptionDetails?fail(Error(r.exceptionDetails.exception?.description??r.exceptionDetails.exception?.value??r.exceptionDetails.text)):ok(r.result.value)});
      ws.send(JSON.stringify({id:next,method:'Runtime.evaluate',params:{expression,awaitPromise:true,returnByValue:true}}));
    });
  }
  async function connectRoute(route:string){
    for(let n=0;n<150;n++){
      const entries=await fetch(`http://127.0.0.1:${port}/json`,{signal:AbortSignal.timeout(1000)}).then(r=>r.json()).catch(()=>[]) as any[];
      const page=entries.find(t=>new URL(t.url).hostname==='tauri.localhost'&&new URL(t.url).pathname.startsWith(route));
      if(page){
        const view=await connect(page);
        for(let attempt=0;attempt<100;attempt++){
          if(await view(`typeof __TAURI_INTERNALS__!=='undefined' && !!document.querySelector(${JSON.stringify(route==='/notice'?'.notice-window .message':'.settings-heading')})`))return view;
          await Bun.sleep(100);
        }
        throw Error('Native route did not finish loading: '+route);
      }
      await Bun.sleep(100);
    }
    throw Error('Native route unavailable: '+route);
  }
  const evaluate=await connect(target);
  for(let n=0;n<200;n++){if(await evaluate(`typeof __TAURI_INTERNALS__!=='undefined'&&!!document.querySelector('.dashboard')`))break;await Bun.sleep(100);}
  const result=await evaluate(`(async()=>{const i=__TAURI_INTERNALS__.invoke;return {build:await i('get_build_info'),environment:await i('integration_environment'),startup:await i('get_autostart'),settings:await i('get_monitoring_settings'),tasks:await i('get_agents')}})()`);
  assert.equal(result.build.process_id,app.pid,'CDP must belong to the exact owned native process');
  if(process.platform==='win32')assert.equal(nativeProcess.owned_executable,true,JSON.stringify(nativeProcess));
  assert.equal(result.environment.isolated,true);assert.equal(result.environment.state_directory,profile.state_directory);
  assert.equal(result.build.channel,'development');assert.equal(result.settings.onboarding_complete,false);
  assert.equal(result.startup.available,false);assert.equal(result.tasks.length,0);

  // Exercise the real IPC ACL with valid synthetic arguments. Inspect the
  // framework error so business-rule failures cannot make the test pass.
  const reply=(view:typeof evaluate,command:string,args:unknown={})=>view(
    `__TAURI_INTERNALS__.invoke(${JSON.stringify(command)},${JSON.stringify(args)}).then(value=>({ok:true,value}),error=>({ok:false,error:String(error)}))`);
  const allowed=async(view:typeof evaluate,command:string,args:unknown={})=>{
    const result=await reply(view,command,args);assert.equal(result.ok,true,`${command}: ${result.error}`);return result.value;
  };
  const denied=async(view:typeof evaluate,command:string,args:unknown={},removed=false)=>{
    const result=await reply(view,command,args);assert.equal(result.ok,false,command);
    assert.match(result.error,removed?/not allowed|not found|denied/i:/not allowed|denied/i,command);
  };
  const settings=result.settings;
  const edit={...settings,notifications:true,notification_mode:'popup',auto_discover_local:false,sources:[]};
  const fixtureManifest=await Bun.file(new URL('../integrations/community-template.json',import.meta.url)).text();
  const configure={sourceId:'nonexistent-fixture-source',enabled:true,confirmed:true};
  for(const [command,args] of [
    ['save_monitoring_settings',{settings:edit,expected:settings}],
    ['save_preferences',{}],['set_autostart',{enabled:true}],
    ['integration_import',{text:fixtureManifest}],['integration_apply',{token:'fixture',confirmed:true}],
    ['install_application_update',{version:'99.0.0'}],['configure_claude_events',configure],
  ] as const)await denied(evaluate,command,args);
  await denied(evaluate,'focus_terminal',{req:{kind:'windows_native',focusId:'0',outerId:''}},true);
  await allowed(evaluate,'open_management',{section:'sources'});
  const management=await connectRoute('/manage');
  await denied(management,'install_hooks',{cli:'fixture',uninstall:false},true);
  const hook=await reply(management,'configure_claude_events',configure);
  assert.equal(hook.ok,false);assert.match(hook.error,/installed TaskPulse/);
  const imported=await allowed(management,'integration_import',{text:fixtureManifest});
  assert.equal(imported.adapter_id,'example-editor');
  assert.ok((await allowed(management,'integration_catalog')).some((item:any)=>item.adapter_id===imported.adapter_id));
  const saved=await allowed(management,'save_monitoring_settings',{settings:edit,expected:settings});
  assert.equal(saved.notifications,true);
  assert.equal((await allowed(evaluate,'get_monitoring_settings')).notifications,true);
  const stale=await reply(management,'save_monitoring_settings',{settings,expected:settings});
  assert.equal(stale.ok,false);assert.match(stale.error,/配置已更新/);
  await allowed(management,'test_notification',{kind:'turn_ended'});
  const notice=await connectRoute('/notice');
  const current=(await allowed(notice,'get_task_notice')).current;
  assert.equal(current.test,true);
  await notice(`globalThis.forgedNoticeEvents=0; window.__TAURI__.event.listen('task-notice-updated', e=>{
    if(e.payload.syntheticSecurityFixture)globalThis.forgedNoticeEvents++;
  }).then(()=>true)`);
  await allowed(evaluate,'plugin:event|emit',{event:'task-notice-updated',payload:{current:{...current,id:current.id+1},pending:0,syntheticSecurityFixture:true}});
  await allowed(evaluate,'plugin:event|emit',{event:'task-notice-updated',payload:{current:{...current,auto_dismiss_ms:1},pending:0,syntheticSecurityFixture:true}});
  for(let n=0;n<50 && await notice('globalThis.forgedNoticeEvents')!==2;n++)await Bun.sleep(20);
  assert.equal(await notice('globalThis.forgedNoticeEvents'),2,'Both forged events reached the notice renderer');
  await Bun.sleep(450);
  assert.equal((await allowed(notice,'get_task_notice')).current?.id,current.id,'Forged events cannot dismiss the notice through its timer');
  await denied(notice,'save_monitoring_settings',{settings,expected:saved});
  await denied(notice,'mark_messages_seen',{ids:[]});
  await denied(notice,'update_task_attention',{id:'fixture',action:'pin',pinned:true});
  await denied(notice,'open_management',{section:'sources'});
  await denied(evaluate,'dismiss_task_notice',{id:current.id,reason:'user'});
  await denied(management,'dismiss_task_notice',{id:current.id,reason:'user'});
  assert.equal((await allowed(notice,'dismiss_task_notice',{id:current.id,reason:'user'})).current,null);
  const after=await readStable();assert.ok(before[0]===null?after[0]===null:after[0]!==null&&before[0].equals(after[0]),'Installed settings changed');
  await Bun.write(join(owned,'result.json'),JSON.stringify({passed:true,commit:result.build.commit,channel:result.build.channel,automaticProfileIsolation:true,onboardingRequired:true,importedTasks:0,startupDisabled:true,realHookConfigurationBlocked:true,productionDirectoryOverrideBlocked:true,productionSettingsUnchanged:true,windowCommandAclVerified:true,startup}));
  console.log('Native default development profile, onboarding, IPC and production-settings isolation passed');
});
