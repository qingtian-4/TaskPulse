/** Actual default debug profile + WebView2/IPC; never installs or modifies startup. */
import assert from 'node:assert/strict';
import {copyFile,mkdir,readFile,lstat} from 'node:fs/promises';
import {resolve,join,dirname,sep} from 'node:path';
import {createServer} from 'node:net';
import {spawn,spawnSync,type ChildProcess} from 'node:child_process';
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
  let app:ChildProcess|undefined,ws:WebSocket|undefined;
  onStop(async()=>{
    ws?.close();await stopOwnedChild(app);
    const current=await lstat(state,{bigint:true});
    assert.ok(!current.isSymbolicLink()&&current.dev===identity.dev&&current.ino===identity.ino,'Refuse cleanup of a replaced profile');
    await removeWithRetry(state);
  });
  const server=createServer();await new Promise<void>(ok=>server.listen(0,'127.0.0.1',ok));
  const port=(server.address() as {port:number}).port;await new Promise<void>(ok=>server.close(()=>ok()));
  let stderr='';app=spawn(exe,['--show'],{env:{...env,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port}`},windowsHide:true,stdio:['ignore','ignore','pipe']});
  app.stderr?.on('data',v=>stderr=(stderr+String(v)).slice(-2000));
  let target:any;
  for(let n=0;n<240;n++){
    if(app.exitCode!==null)throw Error('Development app exited: '+app.exitCode+' '+stderr);
    target=(await fetch(`http://127.0.0.1:${port}/json`).then(r=>r.json()).catch(()=>[]) as any[]).find(t=>t.url.includes('tauri.localhost')&&!t.url.includes('/manage')&&!t.url.includes('/notice'));
    if(target)break;await Bun.sleep(100);
  }
  assert.ok(target,'Native development popup is missing');
  ws=new WebSocket(target.webSocketDebuggerUrl);await new Promise<void>((ok,fail)=>{ws!.onopen=()=>ok();ws!.onerror=()=>fail(Error('CDP unavailable'));});
  let id=0;const pending=new Map<number,any>();
  ws.onmessage=e=>{const m=JSON.parse(String(e.data)),p=pending.get(m.id);if(!p)return;clearTimeout(p.timer);pending.delete(m.id);m.error?p.fail(Error(JSON.stringify(m.error))):p.ok(m.result);};
  const evaluate=(expression:string)=>new Promise<any>((ok,fail)=>{const next=++id,timer=setTimeout(()=>{pending.delete(next);fail(Error('CDP timed out'));},15000);pending.set(next,{timer,fail,ok:(r:any)=>r.exceptionDetails?fail(Error(r.exceptionDetails.text)):ok(r.result.value)});ws!.send(JSON.stringify({id:next,method:'Runtime.evaluate',params:{expression,awaitPromise:true,returnByValue:true}}));});
  for(let n=0;n<200;n++){if(await evaluate(`typeof __TAURI_INTERNALS__!=='undefined'&&!!document.querySelector('.dashboard')`))break;await Bun.sleep(100);}
  const result=await evaluate(`(async()=>{const i=__TAURI_INTERNALS__.invoke;return {build:await i('get_build_info'),environment:await i('integration_environment'),startup:await i('get_autostart'),settings:await i('get_monitoring_settings'),tasks:await i('get_agents')}})()`);
  const hookDenied=await evaluate(`__TAURI_INTERNALS__.invoke('configure_claude_events',{sourceId:'nonexistent-fixture-source',enabled:true,confirmed:true}).then(()=>'',e=>String(e))`);
  assert.match(hookDenied,/installed TaskPulse/);
  assert.equal(result.environment.isolated,true);assert.equal(result.environment.state_directory,profile.state_directory);
  assert.equal(result.build.channel,'development');assert.equal(result.settings.onboarding_complete,false);
  assert.equal(result.startup.available,false);assert.equal(result.tasks.length,0);
  const after=await readStable();assert.deepEqual(after[0],before[0],'Installed settings changed');
  await Bun.write(join(owned,'result.json'),JSON.stringify({passed:true,commit:result.build.commit,channel:result.build.channel,automaticProfileIsolation:true,onboardingRequired:true,importedTasks:0,startupDisabled:true,realHookConfigurationBlocked:true,productionDirectoryOverrideBlocked:true,productionSettingsUnchanged:true}));
  console.log('Native default development profile, onboarding, IPC and production-settings isolation passed');
});
