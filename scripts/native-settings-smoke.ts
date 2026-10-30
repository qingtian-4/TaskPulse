/** Native WebView2 settings acceptance with an isolated profile. Never registers login startup or installs Agent hooks. */
import assert from 'node:assert/strict';
import {mkdir,copyFile} from 'node:fs/promises';
import {resolve,join,dirname} from 'node:path';
import {spawn,type ChildProcess} from 'node:child_process';
import {createServer} from 'node:net';
import {withSmokeWorkspace,stopOwnedChild} from './smoke-workspace';
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
  await root.invoke('open_management',{section:'general'});
  let ui=await connect(await tab('/manage'));await ui.wait(`!!document.querySelector('.theme-picker')`);
  const windowSizes:any[]=[];
  for(const section of ['general','panel','sources','notifications','updates','about']){
    await ui.evaluate(`document.querySelector('.settings-nav a[href="#${section}"]').click()`);
    await ui.wait(`document.querySelector('.settings-nav a[href="#${section}"]').getAttribute('aria-current')==='page'`);
    windowSizes.push(await ui.invoke('plugin:window|inner_size',{label:'management'}));
    assert.equal(await ui.evaluate('document.documentElement.scrollHeight<=innerHeight && document.documentElement.scrollWidth<=innerWidth'),true);
  }
  assert.ok(windowSizes.every(size=>JSON.stringify(size)===JSON.stringify(windowSizes[0])));
  const catalog:any[]=await ui.invoke('get_source_catalog');assert.equal(catalog.filter(s=>s.category==='local').length,9);
  assert.equal(catalog.find(s=>s.kind==='scanner').capabilities.turn_end,'unsupported');
  console.log('settings smoke: fixed layout and catalog passed');const startup=await ui.invoke('get_autostart');assert.equal(startup.available,false);
  assert.match(await ui.evaluate(`__TAURI_INTERNALS__.invoke('set_autostart',{enabled:true}).then(()=>'',e=>String(e))`),/installed TaskPulse/);
  await ui.evaluate(`document.querySelector('.settings-nav a[href="#sources"]').click()`);
  await ui.wait(`document.querySelectorAll('.source-card').length>=9`);
  await ui.evaluate(`document.querySelector('.source-expand').click()`);
  await ui.wait(`!!document.querySelector('.source-editor textarea')`);
  await ui.evaluate(`(()=>{const field=document.querySelector('.source-editor textarea');field.value='draft-only-project';field.dispatchEvent(new Event('input',{bubbles:true}));})()`);
  await ui.wait(`document.querySelector('.settings-save').textContent.includes('未应用')`);
  console.log('settings smoke: source draft, switching language');await ui.evaluate(`document.querySelector('.settings-nav a[href="#general"]').click()`);
  await ui.evaluate(`(()=>{const s=document.querySelector('.setting-row select');s.value='en';s.dispatchEvent(new Event('change',{bubbles:true}));})()`);
  await ui.wait(`document.querySelector('.settings-heading h1').textContent==='General' && !document.querySelector('.settings-save').textContent.includes('Saving')`);
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
  await ui.wait(`document.querySelector('[aria-label="Show task model"]').checked===false && document.querySelector('.settings-save').textContent.includes('Saved')`);
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
  await ui.evaluate(`document.querySelector('.settings-nav a[href="#general"]').click()`);
  await Bun.write(join(owned,'settings-general-en.png'),Buffer.from((await ui.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
  await ui.evaluate(`document.querySelector('.settings-nav a[href="#sources"]').click()`);
  await Bun.write(join(owned,'settings-sources-en.png'),Buffer.from((await ui.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
  sockets.forEach(ws=>ws.close());await stopOwnedChild(app);start();
  root=await connect(await tab(''));await root.wait(`!!document.querySelector('.dashboard')`);
  const restored=await root.invoke('get_monitoring_settings');assert.equal(restored.preferences.language,'en');assert.equal(restored.preferences.theme,'dark');assert.equal(restored.preferences.show_model,false);assert.deepEqual(restored.sources[0].projects,['protected-project']);
  assert.deepEqual(errors,[]);
  await Bun.write(join(owned,'result.json'),JSON.stringify({passed:true,checks:['native fixed window across six sections','catalog and process capability','isolated startup rejection','source draft retained across navigation and preferences','language and theme synchronized across native windows','native dirty-close confirmation','cancel source draft','apply source scope preserves preferences','unknown config preserved','restart restores preferences and scope'],windowSizes,startup,errors},null,2));
  console.log('Native settings acceptance passed (11 checks).');
});
