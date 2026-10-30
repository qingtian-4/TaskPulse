/** Native WebView2 settings acceptance with an isolated profile. Never registers login startup or installs Agent hooks. */
import assert from 'node:assert/strict';
import {mkdir,copyFile,chmod} from 'node:fs/promises';
import {resolve,join,dirname} from 'node:path';
import {spawn,type ChildProcess} from 'node:child_process';
import {createServer} from 'node:net';
import {withSmokeWorkspace,stopOwnedChild} from './smoke-workspace';
import {fileURLToPath} from 'node:url';
const arg=process.argv.indexOf('--exe');if(arg<0)throw Error('--exe required');
const original=resolve(process.argv[arg+1]);
await withSmokeWorkspace('settings-native',async({owned,onStop})=>{
  const home=join(owned,'profile');await mkdir(home);
  const exe=join(owned,'agent-tray.exe');await copyFile(original,exe);
  if(await Bun.file(join(dirname(original),'WebView2Loader.dll')).exists())await copyFile(join(dirname(original),'WebView2Loader.dll'),join(owned,'WebView2Loader.dll'));
  await Bun.write(join(home,'config.toml'),`custom_preserved="keep-me"
[dashboard]
version=2
onboarding_complete=true
auto_discover_local=false
auto_check_updates=false
notifications=false
sound=false
notification_mode="popup"
default_view="tasks"
recent_days=7
[dashboard.preferences]
theme="light"
language="zh-CN"
show_on_start=false
show_model=true
show_elapsed=true
panel_density="comfortable"
[[dashboard.sources]]
id="codex"
name="Codex"
kind="codex"
enabled=false
location=""
projects=["protected-project"]
excluded_projects=[]
tmux_sessions=[]
tmux_panes=[]
poll_interval_ms=3000
`);
  const reserve=createServer();await new Promise<void>(ok=>reserve.listen(0,'127.0.0.1',ok));
  const port=(reserve.address() as {port:number}).port;await new Promise<void>((ok,fail)=>reserve.close(e=>e?fail(e):ok()));
  const env={...process.env,TASKPULSE_HOME:home,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port}`};
  let app:ChildProcess|undefined;const sockets:WebSocket[]=[];let stderr='';const errors:string[]=[];
  onStop(async()=>{sockets.forEach(ws=>ws.close());await stopOwnedChild(app);});
  type Tab={url:string;webSocketDebuggerUrl:string};
  async function tab(path:string){for(let n=0;n<200;n++){
    const tabs:Tab[]=await fetch(`http://127.0.0.1:${port}/json`).then(r=>r.json()).catch(()=>[]);
    const found=tabs.find(t=>path?t.url.includes(path):t.url.includes('tauri.localhost')&&!t.url.includes('/manage')&&!t.url.includes('/notice'));
    if(found)return found;await Bun.sleep(100);
  }throw Error('Native window unavailable: '+path+' '+stderr.slice(-1500));}
  async function connect(target:Tab){
    const ws=new WebSocket(target.webSocketDebuggerUrl);sockets.push(ws);await new Promise<void>((ok,fail)=>{ws.onopen=()=>ok();ws.onerror=()=>fail(Error('CDP failed'));});
    let id=0;const pending=new Map<number,{ok:(r:any)=>void;fail:(e:unknown)=>void;timer:ReturnType<typeof setTimeout>}>();
    ws.onmessage=e=>{const m=JSON.parse(String(e.data));if(m.method==='Runtime.exceptionThrown')errors.push(m.params.exceptionDetails.exception?.description??m.params.exceptionDetails.text);const p=pending.get(m.id);if(!p)return;clearTimeout(p.timer);pending.delete(m.id);m.error?p.fail(Error(JSON.stringify(m.error))):p.ok(m.result);};
    const send=(method:string,params:Record<string,unknown>={})=>new Promise<any>((ok,fail)=>{const next=++id;const timer=setTimeout(()=>{pending.delete(next);fail(Error('CDP timeout: '+method+' '+String(params.expression??'').slice(0,180)));},20000);pending.set(next,{ok,fail,timer});ws.send(JSON.stringify({id:next,method,params}));});
    const evaluate=async(expression:string)=>{const r=await send('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true,userGesture:true});if(r.exceptionDetails)throw Error(r.exceptionDetails.exception?.description??r.exceptionDetails.exception?.value??r.exceptionDetails.text);return r.result.value;};
    const invoke=(cmd:string,args:Record<string,unknown>={})=>evaluate(`__TAURI_INTERNALS__.invoke(${JSON.stringify(cmd)},${JSON.stringify(args)})`);
    const wait=async(expression:string)=>{for(let n=0;n<150;n++){if(await evaluate(expression))return;await Bun.sleep(100);}throw Error('Native assertion timeout: '+expression);};
    await send('Runtime.enable');return {send,evaluate,invoke,wait};
  }
  const start=()=>{app=spawn(exe,['--show'],{env,windowsHide:true,stdio:['ignore','ignore','pipe']});app.stderr?.on('data',chunk=>stderr+=String(chunk));app.once('error',e=>stderr+=String(e));};
  console.log('settings smoke: native startup');start();let root=await connect(await tab(''));await root.wait(`!!document.querySelector('.dashboard')`);
  console.log('settings smoke: build',JSON.stringify(await root.invoke('get_build_info')));
  await root.invoke('open_management',{section:'sources'});
  let ui=await connect(await tab('/manage'));await ui.wait(`!!document.querySelector('.settings-heading')`);
  console.log('settings smoke: cold section',await ui.evaluate(`document.querySelector('.settings-heading h1').textContent`));
  await ui.wait(`document.querySelector('.settings-nav a[href="#sources"]')?.getAttribute('aria-current')==='page'`);
  // Both cold creation and warm window reuse must honor every allowed section.
  for(const section of ['general','panel','sources','notifications','updates','about']){
    await root.invoke('open_management',{section});
    await ui.wait(`document.querySelector('.settings-nav a[href="#${section}"]').getAttribute('aria-current')==='page'`);
  }
  await root.invoke('open_management',{section:'invalid?section=updates'});
  assert.equal(await ui.evaluate(`document.querySelector('.settings-nav [aria-current="page"]').getAttribute('href')`),'#about');
  const windowSizes:any[]=[];
  for(const section of ['general','panel','sources','notifications','updates','about']){
    await ui.evaluate(`document.querySelector('.settings-nav a[href="#${section}"]').click()`);
    await ui.wait(`document.querySelector('.settings-nav a[href="#${section}"]').getAttribute('aria-current')==='page'`);
    windowSizes.push(await ui.invoke('plugin:window|inner_size',{label:'management'}));
    assert.equal(await ui.evaluate('document.documentElement.scrollHeight<=innerHeight && document.documentElement.scrollWidth<=innerWidth'),true);
  }
  assert.ok(windowSizes.every(size=>JSON.stringify(size)===JSON.stringify(windowSizes[0])));
  const nativeMonitors:any[]=await ui.invoke('plugin:window|available_monitors');
  const title:string=await ui.invoke('plugin:window|title',{label:'management'});
  const move=async(x:number,y:number)=>{
    const child=Bun.spawn([join(process.env.SystemRoot??'C:/Windows','System32/WindowsPowerShell/v1.0/powershell.exe'),'-NoProfile','-ExecutionPolicy','Bypass','-File',fileURLToPath(new URL('./settings-window-move.ps1',import.meta.url)),'-ProcessId',String(app!.pid),'-Exe',exe,'-Title',title,'-X',String(Math.round(x)),'-Y',String(Math.round(y))],{stdout:'pipe',stderr:'pipe'});
    const [code,,error]=await Promise.all([child.exited,new Response(child.stdout).text(),new Response(child.stderr).text()]);assert.equal(code,0,error);
  };
  const initialMonitor:any=await ui.invoke('plugin:window|current_monitor',{label:'management'});
  const initialPosition:any=await ui.invoke('plugin:window|outer_position',{label:'management'});
  const initialSize:any=await ui.invoke('plugin:window|outer_size',{label:'management'});
  // A small overlap at the bottom edge must not be snapped back on the same
  // monitor. Clamping every Moved event prevents gradual cross-screen dragging.
  const area=initialMonitor.workArea;
  const edgeY=area.position.y+area.size.height-initialSize.height+8;
  await move(initialPosition.x,edgeY);await Bun.sleep(400);
  const edge:any=await ui.invoke('plugin:window|outer_position',{label:'management'});
  assert.ok(Math.abs(edge.y-edgeY)<=2,'same-monitor dragging must not snap back');
  await move(initialPosition.x,initialPosition.y);
  const movementChecks:any[]=[];
  for(const monitor of [...nativeMonitors,...nativeMonitors.slice().reverse(),initialMonitor]){
    const work=monitor.workArea;
    await move(work.position.x+Math.max(0,(work.size.width-initialSize.width)/2),work.position.y+24);
    await Bun.sleep(400);
    const current:any=await ui.invoke('plugin:window|current_monitor',{label:'management'});
    const pos:any=await ui.invoke('plugin:window|outer_position',{label:'management'}),size:any=await ui.invoke('plugin:window|outer_size',{label:'management'});
    const bounds=current.workArea;
    assert.deepEqual(bounds,work,'movement must reach the requested monitor');
    assert.ok(pos.x>=bounds.position.x-2&&pos.y>=bounds.position.y-2);
    assert.ok(pos.x+size.width<=bounds.position.x+bounds.size.width+2&&pos.y+size.height<=bounds.position.y+bounds.size.height+2);
    const inner:any=await ui.invoke('plugin:window|inner_size',{label:'management'});
    const scale:number=await ui.invoke('plugin:window|scale_factor',{label:'management'});
    const css:any=await ui.evaluate('({width:innerWidth,height:innerHeight,ratio:devicePixelRatio})');
    const expectedWidth=Math.min(960,(bounds.size.width-(size.width-inner.width))/scale-32);
    const expectedHeight=Math.min(704,(bounds.size.height-(size.height-inner.height))/scale-32);
    const sample={scale:current.scaleFactor,windowScale:scale,css,size,inner,position:pos,expectedWidth,expectedHeight};
    movementChecks.push(sample);
    assert.ok(Math.abs(inner.width/scale-expectedWidth)<=2,JSON.stringify(sample));
    assert.ok(Math.abs(inner.height/scale-expectedHeight)<=2,JSON.stringify(sample));
    assert.ok(Math.abs(css.width-inner.width/scale)<=2&&Math.abs(css.ratio-scale)<.01,JSON.stringify(sample));
  }
  await move(initialPosition.x,initialPosition.y);
  const catalog:any[]=await ui.invoke('get_source_catalog');assert.equal(catalog.filter(s=>s.category==='local').length,9);
  assert.equal(catalog.find(s=>s.kind==='scanner').capabilities.turn_end,'unsupported');
  console.log('settings smoke: fixed layout and catalog passed');const startup=await ui.invoke('get_autostart');assert.equal(startup.available,false);
  assert.match(await ui.evaluate(`__TAURI_INTERNALS__.invoke('set_autostart',{enabled:true}).then(()=>'',e=>String(e))`),/installed TaskPulse/);
  await ui.evaluate(`document.querySelector('.settings-nav a[href="#sources"]').click()`);
  await ui.wait(`document.querySelectorAll('.source-card').length>=9`);
  await ui.evaluate(`document.querySelector('.source-expand').click()`);
  await ui.wait(`!!document.querySelector('.source-editor textarea')`);
  await ui.evaluate(`(()=>{const field=document.querySelector('.source-editor textarea');field.value='draft-only-project';field.dispatchEvent(new Event('input',{bubbles:true}));})()`);
  await ui.wait(`!!document.querySelector('.source-pending')`);
  console.log('settings smoke: source draft, switching language');await ui.evaluate(`document.querySelector('.settings-nav a[href="#general"]').click()`);
  await ui.evaluate(`(()=>{const s=document.querySelector('.setting-row select');s.value='en';s.dispatchEvent(new Event('change',{bubbles:true}));})()`);
  await ui.wait(`document.querySelector('.settings-heading h1').textContent==='General' && document.querySelector('.settings-status').textContent===''`);
  assert.equal(await root.evaluate(`document.documentElement.lang`),'en');
  console.log('settings smoke: language passed, switching theme');await ui.evaluate(`document.querySelector('[data-preview="dark"] input').click()`);
  await ui.wait(`document.documentElement.dataset.theme==='dark'`);
  await root.wait(`document.documentElement.dataset.theme==='dark'`);
  console.log('settings smoke: theme passed');let saved=await ui.invoke('get_monitoring_settings');assert.deepEqual(saved.sources[0].projects,['protected-project']);
  await ui.evaluate(`document.querySelector('.settings-nav a[href="#sources"]').click()`);
  assert.equal(await ui.evaluate(`document.querySelector('.source-editor textarea').value`),'draft-only-project');
  // Closing a dirty editor must show the actual native close-request confirmation.
  await ui.invoke('plugin:window|close',{label:'management'});
  await ui.wait(`!!document.querySelector('#close-title')`);
  await ui.evaluate(`document.querySelector('.settings-modal button').click()`);
  await ui.evaluate(`[...document.querySelectorAll('.source-editor .settings-actions button')].find(b=>b.textContent==='Cancel changes').click()`);
  await ui.wait(`!document.querySelector('.source-pending')`);
  assert.equal(await ui.evaluate(`document.querySelector('.settings-heading h1').textContent`),'Sources');
  await ui.evaluate(`document.querySelector('.settings-nav a[href="#panel"]').click()`);
  await ui.evaluate(`document.querySelector('[aria-label="Show task model"]').click()`);
  await ui.wait(`document.querySelector('[aria-label="Show task model"]').checked===false && document.querySelector('.settings-status').textContent===''`);
  saved=await root.invoke('get_monitoring_settings');assert.equal(saved.preferences.show_model,false);
  assert.deepEqual(saved.sources[0].projects,['protected-project']);assert.equal(saved.sources[0].enabled,false);
  assert.ok((await Bun.file(join(home,'config.toml')).text()).includes('custom_preserved = "keep-me"'));
  // Exercise a real source commit, then restore the isolated scope.
  await ui.evaluate(`document.querySelector('.settings-nav a[href="#sources"]').click()`);
  for(const project of ['applied-project','protected-project']){
    await ui.evaluate(`(()=>{const field=document.querySelector('.source-editor textarea');field.value=${JSON.stringify(project)};field.dispatchEvent(new Event('input',{bubbles:true}));})()`);
    await ui.evaluate(`[...document.querySelectorAll('.source-editor .settings-actions button')].find(b=>b.textContent==='Apply changes').click()`);
    await ui.wait(`!document.querySelector('.source-pending')`);
    assert.equal(await ui.evaluate(`/[\\u3400-\\u9fff]/.test(document.querySelector('.source-message')?.textContent??'')`),false,'Application save feedback must follow the selected language');
    const current=await ui.invoke('get_monitoring_settings');assert.deepEqual(current.sources[0].projects,[project]);assert.equal(current.preferences.show_model,false);assert.equal(current.preferences.language,'en');
  }
  // Windows read-only replacement fails through the real Rust persistence path.
  await ui.evaluate(`document.querySelector('.settings-nav a[href="#panel"]').click()`);
  await chmod(join(home,'config.toml'),0o444);
  try{
    await ui.evaluate(`document.querySelector('[aria-label="Show task model"]').click()`);
    await ui.wait(`!!document.querySelector('.settings-inline-error') && document.querySelector('.settings-status').textContent===''`);
    assert.equal(await ui.evaluate(`document.querySelector('[aria-label="Show task model"]').checked`),false,'Rejected save must restore the last confirmed control value');
    assert.equal((await root.invoke('get_monitoring_settings')).preferences.show_model,false);
  }finally{await chmod(join(home,'config.toml'),0o666);}
  await ui.evaluate(`document.querySelector('.settings-inline-error button').click()`);
  await ui.wait(`!document.querySelector('.settings-inline-error')`);
  // Add two actual saved SSH instances without connecting to any host.
  await ui.evaluate(`document.querySelector('.settings-nav a[href="#sources"]').click()`);
  await ui.evaluate(`[...document.querySelectorAll('.settings-tabs button')].find(b=>b.textContent==='WSL & remote').click()`);
  const addSelector='.source-card[data-definition="remote_codex"][data-source-id=""] .source-expand';
  for(const host of ['fixture-one','fixture-two']){
    await ui.wait(`!!document.querySelector(${JSON.stringify(addSelector)})`);
    assert.equal(await ui.evaluate(`document.querySelector(${JSON.stringify(addSelector)}).textContent`),'Add source');
    await ui.evaluate(`document.querySelector(${JSON.stringify(addSelector)}).click()`);
    await ui.wait(`!!document.querySelector('.source-editor input')`);
    await ui.evaluate(`(()=>{const card=document.querySelector('.source-editor').closest('.source-card');const toggle=card.querySelector('input[role="switch"]');if(toggle.checked)toggle.click();const inputs=card.querySelectorAll('.source-editor input');const location=inputs[1];location.value=${JSON.stringify(host)};location.dispatchEvent(new Event('input',{bubbles:true}));})()`);
    await ui.evaluate(`[...document.querySelectorAll('.source-editor .settings-actions button')].find(b=>b.textContent==='Apply changes').click()`);
    await ui.wait(`!document.querySelector('.source-pending')`);
  }
  const multiple=await ui.invoke('get_monitoring_settings');
  const hosts=multiple.sources.filter((s:any)=>s.kind==='remote_codex');
  assert.deepEqual(hosts.map((s:any)=>s.location),['fixture-one','fixture-two']);
  assert.equal(new Set(hosts.map((s:any)=>s.id)).size,2);assert.ok(hosts.every((s:any)=>!s.enabled));
  const duplicate=structuredClone(multiple);duplicate.sources.push({...hosts[0],id:'duplicate-fixture'});
  assert.match(await ui.evaluate(`__TAURI_INTERNALS__.invoke('save_monitoring_settings',{settings:${JSON.stringify(duplicate)},expected:${JSON.stringify(multiple)}}).then(()=>'',e=>String(e))`),/同类来源/);
  // Restore the isolated source configuration, preserving current preferences.
  await ui.invoke('save_monitoring_settings',{settings:{...multiple,sources:multiple.sources.filter((s:any)=>s.kind!=='remote_codex')},expected:multiple});
  await ui.evaluate(`document.querySelector('.settings-nav a[href="#general"]').click()`);
  await Bun.write(join(owned,'settings-general-en.png'),Buffer.from((await ui.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
  await ui.evaluate(`document.querySelector('.settings-nav a[href="#sources"]').click()`);
  await Bun.write(join(owned,'settings-sources-en.png'),Buffer.from((await ui.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
  sockets.forEach(ws=>ws.close());await stopOwnedChild(app);start();
  root=await connect(await tab(''));await root.wait(`!!document.querySelector('.dashboard')`);
  const restored=await root.invoke('get_monitoring_settings');assert.equal(restored.preferences.language,'en');assert.equal(restored.preferences.theme,'dark');assert.equal(restored.preferences.show_model,false);assert.deepEqual(restored.sources[0].projects,['protected-project']);
  assert.deepEqual(errors,[]);
  await Bun.write(join(owned,'result.json'),JSON.stringify({passed:true,checks:['cold source entry and warm six-section routing','invalid section rejected','same-monitor drag does not snap back','native monitor transitions fit work area','two saved SSH source instances','duplicate source rejected','native fixed window across six sections','catalog and process capability','isolated startup rejection','source draft retained across navigation and preferences','language and theme synchronized across native windows','native dirty-close confirmation','cancel source draft','apply source scope preserves preferences','failed disk write restores control and settings','unknown config preserved','restart restores preferences and scope'],windowSizes,startup,errors,nativeMonitors:nativeMonitors.length,movementChecks},null,2));
  console.log('Native settings acceptance passed, including cold/warm routing, window movement and two SSH sources.');
});
