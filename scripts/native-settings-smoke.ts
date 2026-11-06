/** Native WebView2 settings acceptance with an isolated profile. Never registers login startup or installs Agent hooks. */
import assert from 'node:assert/strict';
import {mkdir,copyFile,chmod} from 'node:fs/promises';
import {createReadStream} from 'node:fs';
import {createHash} from 'node:crypto';
import {release as osRelease,arch} from 'node:os';
import {resolve,join,dirname} from 'node:path';
import {spawn,type ChildProcess} from 'node:child_process';
import {createServer} from 'node:net';
import {withSmokeWorkspace,stopOwnedChild} from './smoke-workspace';
import {fileURLToPath} from 'node:url';
const arg=process.argv.indexOf('--exe');if(arg<0)throw Error('--exe required');
const original=resolve(process.argv[arg+1]);
const sourcesOnly=process.argv.includes('--sources-only');
const captureScreenshots=!process.argv.includes('--no-screenshots');
const sampleResources=process.argv.includes('--sample-resources');
const commitAt=process.argv.indexOf('--commit');
const expectedCommit=commitAt<0?undefined:process.argv[commitAt+1];
if(commitAt>=0 || sampleResources)assert.match(expectedCommit??'',/^[a-f0-9]{40}$/,'Resource evidence requires the full expected commit');
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
    let paused:((params:any)=>Promise<void>)|undefined;
    let response:((params:any)=>void)|undefined;
    ws.onmessage=e=>{const m=JSON.parse(String(e.data));if(m.method==='Network.responseReceived')response?.(m.params);if(m.method==='Fetch.requestPaused')void paused?.(m.params).catch(error=>errors.push(String(error)));if(m.method==='Runtime.exceptionThrown')errors.push(m.params.exceptionDetails.exception?.description??m.params.exceptionDetails.text);const p=pending.get(m.id);if(!p)return;clearTimeout(p.timer);pending.delete(m.id);m.error?p.fail(Error(JSON.stringify(m.error))):p.ok(m.result);};
    const send=(method:string,params:Record<string,unknown>={})=>new Promise<any>((ok,fail)=>{const next=++id;const timer=setTimeout(()=>{pending.delete(next);fail(Error('CDP timeout: '+method+' '+String(params.expression??'').slice(0,180)));},20000);pending.set(next,{ok,fail,timer});ws.send(JSON.stringify({id:next,method,params}));});
    const evaluate=async(expression:string)=>{const r=await send('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true,userGesture:true});if(r.exceptionDetails)throw Error(r.exceptionDetails.exception?.description??r.exceptionDetails.exception?.value??r.exceptionDetails.text);return r.result.value;};
    const invoke=(cmd:string,args:Record<string,unknown>={})=>evaluate(`__TAURI_INTERNALS__.invoke(${JSON.stringify(cmd)},${JSON.stringify(args)})`);
    const wait=async(expression:string)=>{for(let n=0;n<150;n++){if(await evaluate(expression))return;await Bun.sleep(100);}throw Error('Native assertion timeout: '+expression+'; runtime errors: '+JSON.stringify(errors));};
    await send('Runtime.enable');return {send,evaluate,invoke,wait,onRequestPaused:(handler:typeof paused)=>{paused=handler;},onResponse:(handler:typeof response)=>{response=handler;}};
  }
  const start=()=>{app=spawn(exe,['--show'],{env,windowsHide:true,stdio:['ignore','ignore','pipe']});app.stderr?.on('data',chunk=>stderr+=String(chunk));app.once('error',e=>stderr+=String(e));};
  console.log('settings smoke: native startup');start();let root=await connect(await tab(''));await root.wait(`!!document.querySelector('.dashboard')`);
  const build=await root.invoke('get_build_info');
  if(expectedCommit){assert.equal(build.commit,expectedCommit);assert.equal(build.modified,false);assert.equal(build.process_id,app!.pid);}
  const hasher=createHash('sha256');for await(const chunk of createReadStream(exe))hasher.update(chunk);
  const buildEvidence={commit:build.commit,modified:build.modified,executableSha256:hasher.digest('hex')};
  const product=build.channel==='development'?'TaskPulse Dev':'TaskPulse';
  console.log('settings smoke: build',JSON.stringify(build));
  await root.evaluate(`document.querySelector('[aria-label="设置"]').click()`);
  let ui=await connect(await tab('/manage'));await ui.wait(`!!document.querySelector('.settings-main h1')`);
  await ui.wait(`document.querySelector('.settings-nav a[href="#general"]')?.getAttribute('aria-current')==='page'`);
  assert.equal(await ui.evaluate(`!!document.querySelector('.settings-footnote')`),false);
  await root.invoke('open_management',{section:'sources'});
  await ui.wait(`!!document.querySelector('.ms-head')`);
  console.log('settings smoke: cold section',await ui.evaluate(`document.querySelector('.settings-main h1').textContent`));
  await ui.wait(`document.querySelector('.settings-nav a[href="#sources"]')?.getAttribute('aria-current')==='page'`);
  // Both cold creation and warm window reuse must honor every allowed section.
  for(const section of ['general','panel','sources','notifications','updates','about']){
    await root.invoke('open_management',{section});
    await ui.wait(`document.querySelector('.settings-nav a[href="#${section}"]').getAttribute('aria-current')==='page'`);
  }
  await root.invoke('open_management',{section:'invalid?section=updates'});
  assert.equal(await ui.evaluate(`document.querySelector('.settings-nav [aria-current="page"]').getAttribute('href')`),'#about');
  await root.invoke('open_management');
  await ui.wait(`document.querySelector('.settings-nav a[href="#general"]').getAttribute('aria-current')==='page'`);
  const resourceSamples=[];
  let resourceSampling: {intervalMs:number;durationMs:number;attempts:number;omitted:number}|null=null;
  if(sampleResources){
    const started=performance.now(), attempts=10, intervalMs=500;
    for(let n=0;n<attempts;n++){
      const {process_memory:memory}=await ui.invoke('get_resource_diagnostics');
      const tree=memory.process_tree;
      if(tree?.complete&&tree.includes_webview_children){
        assert.ok(tree.processes>1&&tree.webview_processes>0&&tree.webview_working_set_bytes>0);
        assert.ok(tree.working_set_bytes>=memory.working_set_bytes+tree.webview_working_set_bytes);
        resourceSamples.push({elapsedMs:performance.now()-started,rootWorkingSetBytes:memory.working_set_bytes,...tree});
      }
      if(n+1<attempts)await Bun.sleep(intervalMs);
    }
    resourceSampling={intervalMs,durationMs:performance.now()-started,attempts,omitted:attempts-resourceSamples.length};
    assert.ok(resourceSamples.length>=8,'Too few complete samples with verified WebView2 children');
  }
  const browserProduct=(await root.send('Browser.getVersion').catch(()=>null))?.product??null;
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
  assert.equal(title,`${product} · 设置`,'Initial window title must retain the build identity');
  const movementChecks:any[]=[];
  if(!sourcesOnly){
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
  }
  const catalog:any[]=await ui.invoke('get_source_catalog');assert.equal(catalog.filter(s=>s.category==='local').length,9);
  assert.equal(catalog.find(s=>s.kind==='scanner').capabilities.turn_end,'unsupported');
  assert.match(await root.evaluate(`__TAURI_INTERNALS__.invoke('probe_ssh_source',{host:'taskpulse-unreachable.invalid'}).then(()=>'',e=>String(e))`),/not allowed|denied/i,'SSH probing is restricted to management');
  const unreachable=await ui.invoke('probe_ssh_source',{host:'taskpulse-unreachable.invalid'});
  assert.equal(unreachable.connection,'disconnected');assert.equal(unreachable.codex.state,'not_checked');assert.equal(unreachable.claude_code.state,'unsupported');
  console.log('settings smoke: fixed layout and catalog passed');const startup=await ui.invoke('get_autostart');assert.equal(startup.available,false);
  assert.match(await ui.evaluate(`__TAURI_INTERNALS__.invoke('set_autostart',{enabled:true}).then(()=>'',e=>String(e))`),/installed TaskPulse/);
  if(!sourcesOnly){
  // Exercise the packaged startup UI while the native isolated-profile guard
  // stays intact. Only these two IPC responses are simulated through CDP; no
  // real login entry, security setting or production test override is used.
  await ui.evaluate(`document.querySelector('.settings-nav a[href="#general"]').click()`);
  await ui.wait(`!!document.querySelector('[aria-label="登录时自动启动"]')`);
  let startupEnabled=false,startupWrites=0,startupDenied=false;
  ui.onRequestPaused(async({requestId,request})=>{
    const command=new URL(request.url).pathname.slice(1);
    let denied=command==='discover_sources' && request.method==='POST';
    if(request.method==='POST' && command==='set_autostart'){
      startupWrites++;denied=startupDenied;
      if(!denied)startupEnabled=JSON.parse(request.postData).enabled;
    }
    await ui.send('Fetch.fulfillRequest',{requestId,responseCode:200,responseHeaders:[
      {name:'Content-Type',value:'application/json'},
      {name:'Tauri-Response',value:denied?'error':'ok'},
      {name:'Access-Control-Expose-Headers',value:'Tauri-Response'},
      {name:'Access-Control-Allow-Origin',value:'http://tauri.localhost'},
      {name:'Access-Control-Allow-Methods',value:'POST, OPTIONS'},
      {name:'Access-Control-Allow-Headers',value:'Content-Type, Tauri-Callback, Tauri-Error, Tauri-Invoke-Key'}
    ],body:Buffer.from(JSON.stringify(denied?(command==='discover_sources'?'Source discovery fixture failure':'Access is denied.'):{enabled:startupEnabled,available:true})).toString('base64')});
  });
  try{
    await ui.send('Fetch.enable',{patterns:[{urlPattern:'http://ipc.localhost/get_autostart'},{urlPattern:'http://ipc.localhost/set_autostart'}]});
    await ui.evaluate(`window.dispatchEvent(new Event('focus'))`);
    await ui.wait(`!document.querySelector('[aria-label="登录时自动启动"]').disabled`);
    await ui.evaluate(`document.querySelector('[aria-label="登录时自动启动"]').click()`);
    await ui.wait(`document.querySelector('[aria-label="登录时自动启动"]').disabled`);
    await ui.invoke('plugin:window|close',{label:'management'});
    assert.equal(await ui.evaluate(`!!document.querySelector('[aria-label="登录时自动启动"]') && !document.querySelector('#source-confirm-title')`),true,'Pending startup verification must retain the settings window');
    await ui.wait(`document.querySelector('[aria-label="登录时自动启动"]').checked && !document.querySelector('[aria-label="登录时自动启动"]').disabled`);
    assert.equal(await ui.evaluate(`!!document.querySelector('.startup-feedback')`),false,'Positive control: a retained enable must not show a failure');
    startupEnabled=false;
    await ui.evaluate(`document.querySelector('.settings-nav a[href="#panel"]').click()`);
    await ui.evaluate(`document.querySelector('.settings-nav a[href="#general"]').click()`);
    await ui.wait(`!document.querySelector('[aria-label="登录时自动启动"]').checked`);
    assert.match(await ui.evaluate(`document.querySelector('[role="alert"]')?.textContent??''`),/登录启动未能保持开启/,'External removal must have visible feedback, not silently revert');
    await ui.evaluate(`window.dispatchEvent(new Event('focus'))`);
    await ui.wait(`!!document.querySelector('.startup-feedback')`);
    assert.equal(startupWrites,1,'Readback and tab navigation must never retry an enable');
    startupDenied=true;
    await ui.evaluate(`document.querySelector('[aria-label="登录时自动启动"]').click()`);
    await ui.wait(`document.querySelector('.startup-feedback')?.textContent.includes('Access is denied.') && !document.querySelector('[aria-label="登录时自动启动"]').disabled`);
    assert.equal(await ui.evaluate(`document.querySelector('[aria-label="登录时自动启动"]').checked`),false);
    await ui.evaluate(`window.dispatchEvent(new Event('focus'))`);
    await ui.wait(`document.querySelector('.startup-feedback')?.textContent.includes('Access is denied.')`);
    assert.equal(startupWrites,2);
    // Clear only through explicit successful user actions, then remove the mock.
    startupDenied=false;
    for(const enabled of [true,false]){
      await ui.evaluate(`document.querySelector('[aria-label="登录时自动启动"]').click()`);
      await ui.wait(`document.querySelector('[aria-label="登录时自动启动"]').checked===${enabled} && !document.querySelector('[aria-label="登录时自动启动"]').disabled`);
    }
    assert.equal(await ui.evaluate(`!!document.querySelector('.startup-feedback')`),false);
    // A failed source initialization must not skip the window's single close
    // owner. Reload to exercise registration ordering, not an already-ready UI.
    await ui.send('Fetch.enable',{patterns:[{urlPattern:'http://ipc.localhost/get_autostart'},{urlPattern:'http://ipc.localhost/set_autostart'},{urlPattern:'http://ipc.localhost/discover_sources'}]});
    await ui.evaluate(`window.__smokeReloadMarker=true`);
    await ui.send('Page.reload');
    await ui.wait(`!window.__smokeReloadMarker && document.querySelector('.ms-error')?.textContent.includes('Source discovery fixture failure')`);
    await ui.evaluate(`document.querySelector('.settings-nav a[href="#general"]').click()`);
    await ui.wait(`document.querySelector('[aria-label="登录时自动启动"]')?.disabled===false`);
    await ui.evaluate(`document.querySelector('[aria-label="登录时自动启动"]').click()`);
    await ui.wait(`document.querySelector('[aria-label="登录时自动启动"]').disabled`);
    await ui.invoke('plugin:window|close',{label:'management'});
    await ui.wait(`document.querySelector('[aria-label="登录时自动启动"]').checked && !document.querySelector('[aria-label="登录时自动启动"]').disabled`);
    await ui.evaluate(`document.querySelector('[aria-label="登录时自动启动"]').click()`);
    await ui.wait(`!document.querySelector('[aria-label="登录时自动启动"]').checked && !document.querySelector('[aria-label="登录时自动启动"]').disabled`);
    console.log('settings smoke: startup retention, rollback feedback and no automatic writes passed');
  }finally{await ui.send('Fetch.disable');ui.onRequestPaused(undefined);}
  await ui.evaluate(`window.__smokeReloadMarker=true`);
  await ui.send('Page.reload');
  await ui.wait(`!window.__smokeReloadMarker && !!document.querySelector('.settings-nav a[href="#general"]')`);
  await ui.evaluate(`document.querySelector('.settings-nav a[href="#general"]').click()`);
  await ui.wait(`document.querySelector('[aria-label="登录时自动启动"]')?.disabled===true`);
  assert.equal((await ui.invoke('get_autostart')).available,false);
  }
  await ui.evaluate(`document.querySelector('.settings-nav a[href="#sources"]').click()`);
  await ui.wait(`!!document.querySelector('.local-card')`);
  assert.equal(await ui.evaluate(`document.querySelectorAll('.local-card .mini-tag').length`),catalog.filter(s=>s.category==='local').length);
  assert.equal(await ui.evaluate(`/Hermes/.test(document.querySelector('.local-card').textContent)&&/OpenCode/.test(document.querySelector('.local-card').textContent)`),true,'All catalog local Agents remain configurable');
  assert.equal(await ui.evaluate(`!!document.querySelector('.settings-footnote')`),false);
  await ui.evaluate(`document.querySelectorAll('.ms-tabs button')[1].click()`);
  await ui.wait(`document.querySelectorAll('.local-agents .agent-row').length===9`);
  assert.equal(await ui.evaluate(`!!document.querySelector('.local-card')`),false,'The Local tab opens the Agent list directly');
  await ui.evaluate(`document.querySelector('[data-agent-kind="codex"] .agent-row button').click()`);
  assert.equal(await ui.evaluate(`document.querySelector('[data-source-id="codex"]').closest('[data-agent-kind]').dataset.agentKind`),'codex');
  await ui.wait(`!!document.querySelector('[data-source-id="codex"] .project-scope')`);
  assert.equal(await ui.evaluate(`document.querySelector('.ms-advanced').open`),false,'Advanced settings start collapsed');
  if(captureScreenshots)await Bun.write(join(owned,'settings-local-inline-zh.png'),Buffer.from((await ui.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
  const setProject=async(project:string)=>{
    await ui.evaluate(`(()=>{const scope=document.querySelector('[data-source-id="codex"] .project-scope');for(const field of scope.querySelectorAll('.project-options input:checked'))field.click();const field=scope.querySelector('.add-path input');field.value=${JSON.stringify(project)};field.dispatchEvent(new Event('input',{bubbles:true}));})()`);
    await ui.evaluate(`document.querySelector('[data-source-id="codex"] .add-path button').click()`);
    await ui.wait(`!!document.querySelector('[data-source-id="codex"] .project-options input:checked')`);
  };
  await setProject('draft-only-project');
  await ui.wait(`!![...document.querySelectorAll('.ms-foot button')].find(b=>b.textContent.includes('应用更改'))`);
  console.log('settings smoke: source draft, switching language');await ui.evaluate(`document.querySelector('.settings-nav a[href="#general"]').click()`);
  await ui.evaluate(`(()=>{const s=document.querySelector('.setting-row select');s.value='en';s.dispatchEvent(new Event('change',{bubbles:true}));})()`);
  await ui.wait(`document.querySelector('.settings-heading h1').textContent==='General' && document.querySelector('.settings-status').textContent===''`);
  assert.equal(await root.evaluate(`document.documentElement.lang`),'en');
  assert.equal(await ui.invoke('plugin:window|title',{label:'management'}),`${product} · Settings`,'Language changes must retain the build identity');
  console.log('settings smoke: language passed, switching theme');await ui.evaluate(`document.querySelector('[data-preview="dark"] input').click()`);
  await ui.wait(`document.documentElement.dataset.theme==='dark'`);
  await root.wait(`document.documentElement.dataset.theme==='dark'`);
  assert.equal(await ui.invoke('plugin:window|title',{label:'management'}),`${product} · Settings`,'Theme changes must retain the build identity');
  console.log('settings smoke: theme passed');let saved=await ui.invoke('get_monitoring_settings');assert.deepEqual(saved.sources[0].projects,['protected-project']);
  await ui.evaluate(`document.querySelector('.settings-nav a[href="#sources"]').click()`);
  await ui.wait(`document.querySelector('[data-source-id="codex"] .project-options input:checked')?.closest('label').textContent.includes('draft-only-project')`);
  assert.equal(await ui.evaluate('document.documentElement.scrollWidth<=innerWidth'),true);
  assert.ok(await ui.evaluate(`document.querySelector('.agent-row small').textContent.includes('Disabled')`));
  // Closing a dirty editor must show the actual native close-request confirmation.
  await ui.invoke('plugin:window|close',{label:'management'});
  await ui.wait(`!!document.querySelector('#source-confirm-title')`);
  await ui.evaluate(`document.querySelector('.confirm-modal button').click()`);
  await ui.evaluate(`[...document.querySelectorAll('.ms-foot button')].find(b=>b.textContent==='Cancel changes').click()`);
  await ui.wait(`![...document.querySelectorAll('.ms-foot button')].some(b=>b.textContent==='Apply changes')`);
  assert.equal(await ui.evaluate(`document.querySelector('.ms-head h1').textContent`),'Sources');
  await ui.evaluate(`document.querySelector('.settings-nav a[href="#panel"]').click()`);
  await ui.evaluate(`document.querySelector('[aria-label="Show task model"]').click()`);
  await ui.wait(`document.querySelector('[aria-label="Show task model"]').checked===false && document.querySelector('.settings-status').textContent===''`);
  saved=await root.invoke('get_monitoring_settings');assert.equal(saved.preferences.show_model,false);
  assert.deepEqual(saved.sources[0].projects,['protected-project']);assert.equal(saved.sources[0].enabled,false);
  assert.ok((await Bun.file(join(home,'config.toml')).text()).includes('custom_preserved = "keep-me"'));
  // Exercise a real source commit, then restore the isolated scope.
  await ui.evaluate(`document.querySelector('.settings-nav a[href="#sources"]').click()`);
  for(const project of ['applied-project','protected-project']){
    await setProject(project);
    await ui.evaluate(`[...document.querySelectorAll('.ms-foot button')].find(b=>b.textContent==='Apply changes').click()`);
    await ui.wait(`![...document.querySelectorAll('.ms-foot button')].some(b=>b.textContent==='Apply changes')`);
    assert.equal(await ui.evaluate(`/[\\u3400-\\u9fff]/.test(document.querySelector('.ms-success')?.textContent??'')`),false,'Application save feedback must follow the selected language');
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
  // Other catalog Agents remain editable, with their own stable source IDs.
  await ui.evaluate(`document.querySelector('.settings-nav a[href="#sources"]').click()`);
  await ui.evaluate(`[...document.querySelectorAll('.agent-row')].find(row=>row.textContent.includes('Hermes')).querySelector('button').click()`);
  await ui.evaluate(`document.querySelectorAll('[data-source-id] .scope-modes input')[1].click()`);
  await ui.evaluate(`(()=>{const field=document.querySelector('[data-source-id] .add-path input');field.value='/fixture/hermes';field.dispatchEvent(new Event('input',{bubbles:true}));})()`);
  await ui.evaluate(`document.querySelector('[data-source-id] .add-path button').click()`);
  await ui.evaluate(`[...document.querySelectorAll('.ms-foot button')].find(b=>b.textContent==='Apply changes').click()`);
  await ui.wait(`![...document.querySelectorAll('.ms-foot button')].some(b=>b.textContent==='Apply changes')`);
  const withHermes=await ui.invoke('get_monitoring_settings'),hermes=withHermes.sources.find((s:any)=>s.kind==='hermes');
  assert.equal(await ui.evaluate(`document.querySelector('[data-source-id]').closest('[data-agent-kind]').dataset.agentKind`),'hermes');
  await ui.evaluate(`document.querySelectorAll('.ms-tabs button')[0].click()`);
  await ui.evaluate(`document.querySelectorAll('.ms-tabs button')[1].click()`);
  assert.equal(await ui.evaluate(`document.querySelector('[data-source-id]').closest('[data-agent-kind]').dataset.agentKind`),'hermes','Returning to Local retains the selected Agent and its own editor');
  assert.ok(hermes?.id);assert.deepEqual(hermes.projects,['/fixture/hermes']);assert.equal(hermes.enabled,false);
  await ui.invoke('save_monitoring_settings',{settings:{...withHermes,sources:withHermes.sources.filter((s:any)=>s.id!==hermes.id)},expected:withHermes});
  // The project URL is dispatched through the real OS browser, never a WebView popup.
  await ui.evaluate(`document.querySelector('.settings-nav a[href="#about"]').click()`);
  assert.match(await root.evaluate(`__TAURI_INTERNALS__.invoke('open_project_link',{target:'project'}).then(()=>'',e=>String(e))`),/not allowed|denied/i);
  let projectResponse:any;
  ui.onResponse(({response})=>{if(response.url==='http://ipc.localhost/open_project_link')projectResponse=response;});
  try{
    await ui.send('Network.enable');
    await ui.evaluate(`document.querySelector('.settings-actions a').click()`);
    for(let n=0;n<150&&!projectResponse;n++)await Bun.sleep(100);
    assert.ok(projectResponse,'The About anchor must issue the native browser command');
    assert.equal(projectResponse.status,200);
    assert.equal(Object.entries(projectResponse.headers).find(([key])=>key.toLowerCase()==='tauri-response')?.[1],'ok');
  }finally{ui.onResponse(undefined);await ui.send('Network.disable');}
  assert.equal(await ui.evaluate(`!!document.querySelector('.diagnostic')`),false);
  assert.equal(await ui.evaluate(`document.querySelector('.settings-actions a').href`),'https://github.com/qingtian-4/TaskPulse');
  // Return to the selected Codex editor for the SSH wizard flow.
  await ui.evaluate(`document.querySelector('.settings-nav a[href="#sources"]').click()`);
  await ui.evaluate(`[...document.querySelectorAll('.agent-row')].find(row=>row.querySelector('strong')?.textContent==='Codex').querySelector('button').click()`);
  await ui.evaluate(`document.querySelectorAll('.ms-tabs button')[2].click()`);
  await ui.wait(`!!document.querySelector('.ms-head .ms-btn')`);
  // Native wizard with controlled read-only capability responses. Actual Rust
  // save/CAS/config persistence remains live; no production probe override exists.
  await ui.evaluate(`document.querySelector('.settings-nav a[href="#sources"]').click()`);
  let probeCase='empty';
  ui.onRequestPaused(async({requestId,request})=>{
    const command=new URL(request.url).pathname.slice(1);
    const host=request.method==='POST'&&request.postData?JSON.parse(request.postData).host:'';
    const body=command==='discover_ssh_hosts'?['fixture-one','fixture-two']:{host,checked_at:Date.now(),connection:'connected',codex:{state:probeCase,data_detected:probeCase==='readable',projects:probeCase==='readable'?['/srv/public/alpha','/srv/public/beta']:[]},claude_code:{state:'unsupported',data_detected:false,projects:[]},message:'SSH 已连接；Agent 数据读取能力单独检测'};
    await ui.send('Fetch.fulfillRequest',{requestId,responseCode:200,responseHeaders:[{name:'Content-Type',value:'application/json'},{name:'Tauri-Response',value:'ok'},{name:'Access-Control-Expose-Headers',value:'Tauri-Response'},{name:'Access-Control-Allow-Origin',value:'http://tauri.localhost'},{name:'Access-Control-Allow-Methods',value:'POST, OPTIONS'},{name:'Access-Control-Allow-Headers',value:'Content-Type, Tauri-Callback, Tauri-Error, Tauri-Invoke-Key'}],body:Buffer.from(JSON.stringify(body)).toString('base64')});
  });
  try{
    await ui.send('Fetch.enable',{patterns:[{urlPattern:'http://ipc.localhost/discover_ssh_hosts'},{urlPattern:'http://ipc.localhost/probe_ssh_source'}]});
    for(const host of ['fixture-one','fixture-two']){
      probeCase=host==='fixture-one'?'readable':'read_error';
      await ui.evaluate(`document.querySelector('.ms-head .ms-btn').click()`);
      await ui.wait(`document.querySelectorAll('.alias-choice').length===2`);
      if(captureScreenshots&&host==='fixture-one')await Bun.write(join(owned,'settings-wizard-host-en.png'),Buffer.from((await ui.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
      await ui.evaluate(`(()=>{const input=document.querySelector('.ssh-wizard [aria-label="SSH Host"]');input.value=${JSON.stringify(host)};input.dispatchEvent(new Event('input',{bubbles:true}));})()`);
      await ui.evaluate(`document.querySelector('.ssh-wizard .modal-footer .primary').click()`);
      assert.equal(await ui.evaluate(`document.querySelector('.ssh-wizard .modal-footer .primary').disabled`),true,'Detection is required before project selection');
      await ui.evaluate(`document.querySelector('.test-action button').click()`);
      await ui.wait(`document.querySelector('.test-action button').textContent==='Detect again'`);
      if(captureScreenshots&&host==='fixture-two')await Bun.write(join(owned,'settings-wizard-detection-en.png'),Buffer.from((await ui.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
      await ui.evaluate(`document.querySelector('.ssh-wizard .modal-footer .primary').click()`);
      await ui.wait(`!!document.querySelector('.wizard-agents')`);
      if(captureScreenshots)await Bun.write(join(owned,`settings-wizard-${host==='fixture-one'?'projects':'read-error'}-en.png`),Buffer.from((await ui.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
      assert.equal(await ui.evaluate(`document.querySelectorAll('.wizard-agents input')[1].disabled`),true,'Claude SSH cannot be enabled');
      if(host==='fixture-one'){
        assert.equal(await ui.evaluate(`document.querySelector('.wizard-agents input').checked`),true);
        await ui.evaluate(`document.querySelector('.wizard-agents input').click()`);
        await ui.evaluate(`document.querySelectorAll('.ssh-wizard .scope-modes input')[1].click()`);
        await ui.evaluate(`document.querySelector('.ssh-wizard .project-options input').click()`);
      }else{
        assert.equal(await ui.evaluate(`document.querySelector('.wizard-agents input').disabled`),true,'SSH connectivity cannot enable unreadable Agent data');
        assert.equal(await ui.evaluate(`document.querySelector('.wizard-agents input').checked`),false);
      }
      await ui.evaluate(`document.querySelector('.ssh-wizard .modal-footer .primary').click()`);
      await ui.wait(`!document.querySelector('.ssh-wizard') && !!document.querySelector('.detail-content')`);
    }
  }finally{await ui.send('Fetch.disable');ui.onRequestPaused(undefined);}
  if(captureScreenshots){
    // Reset ephemeral fixture reports before capturing the home/detail page:
    // persisted configuration alone must never paint a successful SSH badge.
    await ui.send('Page.reload');
    await ui.wait(`document.querySelectorAll('.server-card').length===2`);
    assert.equal(await ui.evaluate(`document.querySelector('.server-controls .ms-pill').textContent.includes('not checked')`),true);
    await Bun.write(join(owned,'settings-home-en.png'),Buffer.from((await ui.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
    await ui.evaluate(`document.querySelector('.settings-nav a[href="#general"]').click()`);
    await ui.evaluate(`(()=>{const s=document.querySelector('.setting-row select');s.value='zh-CN';s.dispatchEvent(new Event('change',{bubbles:true}));})()`);
    await ui.wait(`document.documentElement.lang==='zh-CN' && document.querySelector('.settings-status').textContent===''`);
    await ui.evaluate(`document.querySelector('[data-preview="light"] input').click()`);
    await ui.wait(`document.documentElement.dataset.theme==='light' && document.querySelector('.settings-status').textContent===''`);
    await ui.evaluate(`document.querySelector('.settings-nav a[href="#sources"]').click()`);
    await Bun.write(join(owned,'settings-home-zh.png'),Buffer.from((await ui.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
    await ui.evaluate(`document.querySelector('.server-card .ms-btn').click()`);
    await ui.wait(`!!document.querySelector('.detail-content')`);
    await Bun.write(join(owned,'settings-server-zh.png'),Buffer.from((await ui.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
    await ui.evaluate(`document.querySelector('.ms-scroll').scrollTop=10000`);
    await Bun.write(join(owned,'settings-projects-zh.png'),Buffer.from((await ui.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
    const bounds=await ui.evaluate(`({width:innerWidth,height:innerHeight,foot:document.querySelector('.ms-foot').getBoundingClientRect().bottom,overflow:document.documentElement.scrollWidth>innerWidth})`);
    assert.deepEqual(bounds,{width:960,height:704,foot:704,overflow:false});
    await ui.evaluate(`document.querySelector('.settings-nav a[href="#general"]').click()`);
    await ui.evaluate(`(()=>{const s=document.querySelector('.setting-row select');s.value='en';s.dispatchEvent(new Event('change',{bubbles:true}));})()`);
    await ui.wait(`document.documentElement.lang==='en' && document.querySelector('.settings-status').textContent===''`);
    await ui.evaluate(`document.querySelector('[data-preview="dark"] input').click()`);
    await ui.wait(`document.documentElement.dataset.theme==='dark' && document.querySelector('.settings-status').textContent===''`);
    await ui.evaluate(`document.querySelector('.settings-nav a[href="#sources"]').click()`);
  }
  const multiple=await ui.invoke('get_monitoring_settings');
  const hosts=multiple.sources.filter((s:any)=>s.kind==='remote_codex');
  assert.deepEqual(hosts.map((s:any)=>s.location),['fixture-one','fixture-two']);
  assert.equal(new Set(hosts.map((s:any)=>s.id)).size,2);assert.ok(hosts.every((s:any)=>!s.enabled));
  const duplicate=structuredClone(multiple);duplicate.sources.push({...hosts[0],id:'duplicate-fixture'});
  assert.match(await ui.evaluate(`__TAURI_INTERNALS__.invoke('save_monitoring_settings',{settings:${JSON.stringify(duplicate)},expected:${JSON.stringify(multiple)}}).then(()=>'',e=>String(e))`),/同类来源/);
  // Restore the isolated source configuration, preserving current preferences.
  await ui.invoke('save_monitoring_settings',{settings:{...multiple,sources:multiple.sources.filter((s:any)=>s.kind!=='remote_codex')},expected:multiple});
  if(captureScreenshots){
    await ui.evaluate(`document.querySelector('.settings-nav a[href="#general"]').click()`);
    await Bun.write(join(owned,'settings-general-en.png'),Buffer.from((await ui.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
    await ui.evaluate(`document.querySelector('.settings-nav a[href="#sources"]').click()`);
    await ui.evaluate(`document.querySelector('.ms-back')?.click()`);
    await Bun.write(join(owned,'settings-sources-en.png'),Buffer.from((await ui.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
  }
  sockets.forEach(ws=>ws.close());await stopOwnedChild(app);start();
  root=await connect(await tab(''));await root.wait(`!!document.querySelector('.dashboard')`);
  const restored=await root.invoke('get_monitoring_settings');assert.equal(restored.preferences.language,'en');assert.equal(restored.preferences.theme,'dark');assert.equal(restored.preferences.show_model,false);assert.deepEqual(restored.sources[0].projects,['protected-project']);
  assert.deepEqual(errors,[]);
  await Bun.write(join(owned,'result.json'),JSON.stringify({passed:true,...buildEvidence,platform:{version:osRelease(),arch:arch(),browserProduct},resourceSamples,resourceSampling,resourceScope:sampleResources?'Isolated Windows settings: root and live descendants; working sets sum shared pages; CPU is cumulative for live processes':null,checks:['panel gear and default settings open General; explicit sources routing preserved','invalid section rejected',...(!sourcesOnly?['same-monitor drag does not snap back','native monitor transitions fit work area']:[]),'catalog-driven local Agents and collapsed advanced settings','three-step SSH wizard with independent SSH/data states','management-only SSH permission and real unreachable-host probe','bilingual light/dark native layout with internal scroll and sticky footer','non-Codex local source save preserves ID and disabled state','project link delegates to OS and popup permission is denied','two saved SSH source instances','duplicate source rejected','native fixed window across six sections','catalog and process capability','isolated startup rejection','source draft retained across navigation and preferences','language and theme synchronized across native windows','native dirty-close confirmation','cancel source draft','apply source scope preserves preferences','failed disk write restores control and settings','unknown config preserved','restart restores preferences and scope'],windowSizes,startup,errors,nativeMonitors:nativeMonitors.length,movementChecks},null,2));
  console.log('Native settings acceptance passed, including cold/warm routing, source drafts and two SSH sources.');
});
