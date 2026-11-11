/** Interactive Windows tray/window acceptance. Only a copied executable and disposable profile. */
import assert from 'node:assert/strict';
import {copyFile,mkdir} from 'node:fs/promises';
import {join,resolve,dirname} from 'node:path';
import {spawn,type ChildProcess} from 'node:child_process';
import {createServer} from 'node:net';
import {createReadStream} from 'node:fs';
import {createHash} from 'node:crypto';
import {withSmokeWorkspace,stopOwnedChild} from '../support/smoke-workspace';
import {windowsTestProcess} from '../support/windows-test-process';
import {verifyDevelopmentBuild} from '../../scripts/development-artifact';
const at=process.argv.indexOf('--exe');if(at<0)throw Error('--exe required');
const original=resolve(process.argv[at+1]);
const expectedAt=process.argv.indexOf('--expected-commit');
const expectedCommit=expectedAt>=0?process.argv[expectedAt+1]:'';
assert.match(expectedCommit??'',/^[a-f0-9]{40}$/,'--expected-commit requires the full source SHA');
const version=(await Bun.file(new URL('../../package.json',import.meta.url)).json()).version;
async function fileSha256(path:string){
  const hash=createHash('sha256');
  for await(const chunk of createReadStream(path))hash.update(chunk);
  return hash.digest('hex');
}
const reproduce=process.argv.includes('--reproduce');
const callbacksOnly=process.argv.includes('--callbacks-only');
const physicalOnly=process.argv.includes('--physical-only');
const settingsOnly=process.argv.includes('--settings-only');
assert.ok([reproduce,callbacksOnly,physicalOnly,settingsOnly].filter(Boolean).length<=1,'Choose one native acceptance mode');
await withSmokeWorkspace('tray-native',async({owned,onStop})=>{
  const home=join(owned,'profile');await mkdir(home);
  await Bun.write(join(home,'config.toml'),'[dashboard]\nversion=2\nonboarding_complete=true\nauto_discover_local=false\nauto_check_updates=false\nnotifications=false\nsound=false\n[dashboard.preferences]\nshow_on_start=false\nlanguage="zh-CN"\n');
  const exe=join(owned,'agent-tray.exe');await copyFile(original,exe);
  if(await Bun.file(join(dirname(original),'WebView2Loader.dll')).exists())await copyFile(join(dirname(original),'WebView2Loader.dll'),join(owned,'WebView2Loader.dll'));
  const executableSha256=await fileSha256(original);
  assert.equal(await fileSha256(exe),executableSha256,'Test executable copy must match the supplied file');
  const profileInfo=await windowsTestProcess(exe,['--profile-info'],15000);
  assert.equal(profileInfo.exitCode,0,profileInfo.stderr);
  verifyDevelopmentBuild(JSON.parse(profileInfo.stdout),expectedCommit,version);
  const server=createServer();await new Promise<void>(ok=>server.listen(0,'127.0.0.1',ok));
  const port=(server.address() as {port:number}).port;await new Promise<void>(ok=>server.close(()=>ok()));
  let driver:ChildProcess|undefined;let stderr='',lines='';const sockets:WebSocket[]=[];
  const backendInputs:Record<string,number>={};let releaseInput=async()=>{};
  const app=spawn(exe,['--show'],{env:{...process.env,TASKPULSE_HOME:home,RUST_LOG:'agent_tray::tray=debug',WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port}`},windowsHide:true,stdio:['ignore','ignore','pipe']});
  app.stderr?.on('data',v=>{stderr=(stderr+String(v)).slice(-4000);lines+=String(v);const complete=lines.split(/\r?\n/);lines=complete.pop()??'';for(const line of complete){const name=line.match(/Popup input=(\w+)/)?.[1];if(name)backendInputs[name]=(backendInputs[name]??0)+1}});app.once('error',e=>stderr+=String(e));
  onStop(async()=>{sockets.forEach(s=>s.close());try{await releaseInput()}finally{try{await stopOwnedChild(driver)}finally{await stopOwnedChild(app)}}});
  async function connect(route=''){
    let target:any;
    for(let n=0;n<200;n++){
      const tabs:any[]=await fetch(`http://127.0.0.1:${port}/json`).then(r=>r.json()).catch(()=>[]);
      target=tabs.find(t=>t.url?.includes('tauri.localhost')&&(route?t.url.includes(route):!t.url.includes('/manage')&&!t.url.includes('/notice')));
      if(target)break;await Bun.sleep(100);
    }
    assert.ok(target,'Owned native window unavailable: '+stderr);
    const ws=new WebSocket(target.webSocketDebuggerUrl);sockets.push(ws);
    await new Promise<void>((ok,fail)=>{ws.onopen=()=>ok();ws.onerror=()=>fail(Error('CDP connection failed'))});
    let id=0;const pending=new Map<number,any>();
    ws.onmessage=e=>{const m=JSON.parse(String(e.data)),p=pending.get(m.id);if(!p)return;pending.delete(m.id);clearTimeout(p.timer);m.error?p.fail(Error(JSON.stringify(m.error))):p.ok(m.result)};
    async function evaluate(expression:string){
      const next=++id;
      const r:any=await new Promise((ok,fail)=>{const timer=setTimeout(()=>{pending.delete(next);fail(Error('CDP timed out'))},15000);pending.set(next,{ok,fail,timer});ws.send(JSON.stringify({id:next,method:'Runtime.evaluate',params:{expression,awaitPromise:true,returnByValue:true,userGesture:true}}))});
      if(r.exceptionDetails)throw Error(r.exceptionDetails.exception?.description??r.exceptionDetails.text);return r.result.value;
    }
    const invoke=(command:string,args:Record<string,unknown>={})=>evaluate(`__TAURI_INTERNALS__.invoke(${JSON.stringify(command)},${JSON.stringify(args)})`);
    const wait=async(expression:string)=>{for(let n=0;n<150;n++){if(await evaluate(expression))return;await Bun.sleep(100)}throw Error('Native DOM timeout: '+expression)};
    return {evaluate,invoke,wait};
  }
  const root=await connect();await root.wait(`!!document.querySelector('.dashboard')`);
  const build=await root.invoke('get_build_info');assert.equal(build.process_id,app.pid);
  verifyDevelopmentBuild(build,expectedCommit,version);
  const mode=settingsOnly?'settings-only':physicalOnly?'physical-only':callbacksOnly?'callbacks-only':reproduce?'reproduce':'full';
  async function result(report:Record<string,unknown>){
    assert.equal(await fileSha256(exe),executableSha256,'The tested executable changed during acceptance');
    assert.equal(await fileSha256(original),executableSha256,'The supplied executable changed during acceptance');
    await Bun.write(join(owned,'result.json'),JSON.stringify({...report,build,mode,expected_commit:expectedCommit,executable_sha256:executableSha256},null,2));
  }
  await root.evaluate(`window.__trayProbe=[];window.addEventListener('blur',()=>window.__trayProbe.push({event:'blur',at:performance.now()}));window.addEventListener('focus',()=>window.__trayProbe.push({event:'focus',at:performance.now()}));`);
  const statePath=join(owned,'state.json'),control=join(owned,'control.json');
  driver=spawn('powershell.exe',['-NoProfile','-NonInteractive','-STA','-ExecutionPolicy','Bypass','-File',resolve(import.meta.dir,'./native-tray-driver.ps1'),'-AppProcessId',String(app.pid),'-Exe',exe,'-StatePath',statePath,'-ControlPath',control],{windowsHide:true,stdio:['ignore','ignore','pipe']});
  let driverError='';driver.stderr?.on('data',v=>driverError=(driverError+String(v)).slice(-4000));driver.once('error',e=>driverError+=String(e));
  type State={visible:boolean;focused:boolean;topmost:boolean;request_id:number;rounds:number;events:string[];overflow:boolean;menu:string[];bounds:{x:number;y:number;width:number;height:number};physical_point_verified:boolean;error?:string};
  let last:State|undefined,request=0;
  async function waitState(accept:(s:State)=>boolean,timeout=15000){
    const end=Date.now()+timeout;
    do{
      const s:State|undefined=await Bun.file(statePath).json().catch(()=>undefined);
      if(s){last=s;if(s.error)throw Error(s.error);if(accept(s))return s}
      if(driver?.exitCode!==null||driver?.signalCode!==null)throw Error('Native driver exited: '+driverError);
      await Bun.sleep(25);
    }while(Date.now()<end);
    throw Error('Native tray state timeout: '+JSON.stringify(last)+' '+driverError);
  }
  async function action(action:string,args:Record<string,unknown>={}){
    const id=++request;await Bun.write(control,JSON.stringify({id,action,...args}));
    return waitState(s=>s.request_id===id);
  }
  releaseInput=async()=>{if(driver?.exitCode===null&&driver.signalCode===null&&!last?.error)await action('release-input')};
  async function show(){await root.invoke('plugin:window|show',{label:'popup'});await root.invoke('plugin:window|set_focus',{label:'popup'});await waitState(s=>s.visible&&s.focused)}
  const checks:string[]=[];
  const physicalEvidence:unknown[]=[];
  const eventsSince=(state:State,start:number)=>state.events.slice(start).map(event=>event.split(':')[0]);
  async function open(){const count=(await waitState(()=>true)).events.length;await action('notify-down');await waitState(s=>s.visible&&s.events.length>=count+1);await action('notify-up');return waitState(s=>s.visible&&s.focused)}
  async function hide(){const prior=await waitState(()=>true);await root.invoke('close_popup');return waitState(s=>!s.visible&&(!prior.visible||s.events.length>=prior.events.length+1))}
  async function menu(){await action('menu');return waitState(s=>s.menu.length>=6)}
  async function select(label:string){const current=await menu();const index=current.menu.indexOf(label);assert.ok(index>=0,'Native menu label missing: '+label);await action('menu-select',{index});await waitState(s=>s.menu.length===0)}
  async function restoreSettings(){
    await select('设置');const management=await connect('/manage');await management.wait(`!!document.querySelector('.settings-heading')`);
    await management.evaluate(`window.__trayReuse=true`);
    const managementTitle=await management.invoke('plugin:window|title',{label:'management'});
    const minimized=await windowsTestProcess('powershell.exe',[
      '-NoProfile','-ExecutionPolicy','Bypass','-File',resolve(import.meta.dir,'settings-window-move.ps1'),
      '-ProcessId',String(app.pid),'-Exe',exe,'-Title',managementTitle,'-Action','Minimize',
    ],15000);
    assert.equal(minimized.exitCode,0,minimized.stderr);
    await management.wait(`__TAURI_INTERNALS__.invoke('plugin:window|is_minimized',{label:'management'})`);
    await select('设置');
    await management.wait(`__TAURI_INTERNALS__.invoke('plugin:window|is_minimized',{label:'management'}).then(v=>!v)`);
    await management.wait(`__TAURI_INTERNALS__.invoke('plugin:window|is_focused',{label:'management'})`);
    assert.equal(await management.evaluate(`window.__trayReuse`),true);
    return management;
  }
  if(settingsOnly){
    await waitState(()=>true,30000);
    const management=await restoreSettings();
    await management.invoke('plugin:window|close',{label:'management'});
    await result({passed:true,checks:['Native Settings restores and focuses the same minimized management WebView']});
    return;
  }
  try{
    await waitState(()=>true,30000);
    await show();await Bun.sleep(400);
    const start=(await waitState(()=>true)).events.length;
    await action('notify-down');await action('blur');await waitState(s=>!s.visible);
    const beforeRelease=await waitState(()=>true);
    await action('notify-up');await Bun.sleep(400);
    const afterRelease=await waitState(()=>true);
    assert.equal(afterRelease.visible,false,'Mouse release must not reopen a popup hidden by the same gesture');
    checks.push('down/blur/up has no release rebound');
    if(!reproduce){
      let mark:number,state:State;
      if(!callbacksOnly){
      // SendInput reaches the actual Explorer notification icon. Shell bounds
      // bind it to our owned tray; UI Automation must confirm the hit target.
      mark=(await waitState(()=>true)).events.length;
      await action('mouse-down');
      await Bun.sleep(1000);const whileHeld=await waitState(()=>true);
      physicalEvidence.push({phase:'held',state:whileHeld});
      assert.ok(eventsSince(whileHeld,mark).every(event=>event==='show'),'Holding a hidden popup gesture must not hide a newly opened window');
      physicalEvidence.push({phase:'released',state:await action('mouse-up')});state=await waitState(s=>s.visible&&s.focused&&s.events.length>=mark+1);
      assert.ok(state.physical_point_verified);assert.deepEqual(eventsSince(state,mark),['show']);
      checks.push('real mouse long press and release show once without rebound or flicker');
      await hide();
      for(const initialVisible of [false,true]){
        if(initialVisible)await open();
        mark=(await waitState(()=>true)).events.length;const doubles=backendInputs.TrayDouble??0,ups=backendInputs.TrayUp??0;
        await action('mouse-double');state=await waitState(s=>s.visible!==initialVisible&&s.events.length>=mark+1&&(backendInputs.TrayDouble??0)>doubles&&(backendInputs.TrayUp??0)>=ups+2);
        assert.equal(state.visible,!initialVisible);assert.deepEqual(eventsSince(state,mark),[initialVisible?'hide':'show']);
        assert.ok((backendInputs.TrayDouble??0)>doubles,'The OS must actually report a native double-click');
        if(state.visible)await hide();
      }
      checks.push('real OS double-click is one stable toggle in both initial states');
      console.log('Physical long press and double-click passed');
      }
      if(physicalOnly){
        await result({passed:true,checks,backendInputs,physicalEvidence});
        return;
      }
      mark=(await waitState(()=>true)).events.length;
      const expectedUps=(backendInputs.TrayUp??0)+200;
      await action('rounds');
      state=await waitState(s=>s.rounds===100&&!s.visible&&(backendInputs.TrayUp??0)>=expectedUps,95000);
      assert.deepEqual(eventsSince(state,mark),Array.from({length:100},()=>['show','hide']).flat());
      console.log('Native tray callback rounds: 100');
      checks.push('100 consecutive native tray callback rounds have exactly show/hide and no extra transitions');
      await open();mark=(await waitState(()=>true)).events.length;
      await action('notify-down');await waitState(s=>!s.visible);await Bun.sleep(1000);await action('notify-up');
      assert.deepEqual(eventsSince(await waitState(()=>true),mark),['hide']);
      checks.push('long press on a visible popup hides once and release cannot reopen');
      await open();await action('blur');await waitState(s=>!s.visible);
      checks.push('real external focus loss hides an unpinned popup');
      await open();await root.invoke('toggle_pin');await waitState(s=>s.topmost);
      await action('blur');assert.equal((await waitState(()=>true)).visible,true);assert.equal(await root.invoke('get_pinned'),true);
      await show();await root.invoke('toggle_pin');await waitState(s=>!s.topmost);await action('blur');await waitState(s=>!s.visible);
      checks.push('pin preserves the native window across external focus; unpin restores dismissal');
      await open();mark=(await waitState(()=>true)).events.length;
      await select('显示任务面板');state=await waitState(s=>s.visible);
      assert.deepEqual(eventsSince(state,mark),[],'Show on an already-visible popup must not hide/show it');
      await select('隐藏任务面板');await waitState(s=>!s.visible);mark=(await waitState(()=>true)).events.length;
      await select('隐藏任务面板');assert.equal((await waitState(()=>true)).visible,false);assert.deepEqual(eventsSince(await waitState(()=>true),mark),[]);
      checks.push('native menu Show and Hide are independently idempotent');
      const management=await restoreSettings();
      async function language(language:string){
        const settings=await root.invoke('get_monitoring_settings');
        const expected=Object.fromEntries(['preferences','default_view','recent_days','notifications','sound','notification_mode','auto_check_updates','auto_discover_local'].map(key=>[key,settings[key]]));
        await management.invoke('save_preferences',{expected,values:{...expected,preferences:{...settings.preferences,language}}});
        await management.wait(`document.documentElement.lang===${JSON.stringify(language)}`);
      }
      await language('en');state=await menu();assert.ok(state.menu.includes('Settings'));assert.ok(state.menu.includes('Show task panel'));
      await action('menu-select',{index:state.menu.indexOf('Settings')});await waitState(s=>s.menu.length===0);assert.equal(await management.evaluate(`window.__trayReuse`),true);
      await language('zh-CN');await management.invoke('plugin:window|close',{label:'management'});
      checks.push('Chinese/English native Settings restores and focuses the same management WebView');
      const monitors:any[]=await root.invoke('plugin:window|available_monitors');assert.ok(monitors.length>=2,'Actual multi-monitor acceptance requires at least two displays');
      const monitorEvidence=[];
      await hide();
      for(let monitor=0;monitor<monitors.length;monitor++){
        await action('move',{monitor});state=await open();
        const area=monitors[monitor].workArea,b=state.bounds;
        assert.ok(b.x>=area.position.x&&b.y>=area.position.y&&b.x+b.width<=area.position.x+area.size.width&&b.y+b.height<=area.position.y+area.size.height,'Popup must fit the actual display work area');
        await root.invoke('toggle_pin');await waitState(s=>s.topmost);await action('blur');assert.equal((await waitState(()=>true)).visible,true);
        await root.invoke('close_popup');await waitState(s=>!s.visible&&!s.topmost);
        monitorEvidence.push({workArea:area,popup:b,pinPreserved:true});
      }
      checks.push('actual multi-monitor opening, work-area fitting and pinned focus loss');
      state=await waitState(()=>true);assert.equal(state.overflow,false);assert.deepEqual(await root.invoke('get_agents'),[]);
      await result({passed:true,checks,rounds:100,roundTransitions:['show,hide'],backendInputs,physicalPointVerified:state.physical_point_verified,monitors:monitorEvidence});
    }else await result({passed:true,checks,reproduce,events:afterRelease.events.slice(start),beforeRelease});
    console.log('Native tray acceptance passed:',checks.length,'checks');
  }catch(error){
    await Bun.write(join(owned,'failure.json'),JSON.stringify({passed:false,error:String(error),build,last,backendInputs,physicalEvidence,stderr,frontend:await root.evaluate('window.__trayProbe').catch(()=>[]),driverError},null,2));
    throw error;
  }
});
