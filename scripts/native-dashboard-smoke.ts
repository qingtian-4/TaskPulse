/** Windows WebView2 dashboard/notice acceptance using an isolated profile and synthetic task records.
 * Run: bun scripts/native-dashboard-smoke.ts --exe <unused-built-exe>
 * No native process scanning, login registration, Agent hooks or real user state. */
import assert from 'node:assert/strict';
import {mkdir,copyFile,rename} from 'node:fs/promises';
import {resolve,join,dirname} from 'node:path';
import {spawn,type ChildProcess} from 'node:child_process';
import {createServer} from 'node:net';
import {withSmokeWorkspace,stopOwnedChild} from './smoke-workspace';
const arg=process.argv.indexOf('--exe');if(arg<0)throw Error('--exe required');
const original=resolve(process.argv[arg+1]);
await withSmokeWorkspace('dashboard-polish',async({owned,onStop})=>{
  const home=join(owned,'profile');await mkdir(home);
  const exe=join(owned,'agent-tray.exe');await copyFile(original,exe);
  if(await Bun.file(join(dirname(original),'WebView2Loader.dll')).exists())await copyFile(join(dirname(original),'WebView2Loader.dll'),join(owned,'WebView2Loader.dll'));
  const focusState=join(owned,'focus-window.json'),focusControl=join(owned,'focus-control.json');
  const focusWindow=spawn('powershell.exe',['-NoProfile','-NonInteractive','-STA','-ExecutionPolicy','Bypass','-File',resolve(import.meta.dir,'native-focus-window.ps1'),'-StatePath',focusState,'-ControlPath',focusControl],{windowsHide:true,stdio:['ignore','ignore','pipe']});
  let focusError='';focusWindow.stderr?.on('data',chunk=>focusError+=String(chunk));
  focusWindow.once('error',error=>focusError+=String(error));
  onStop(()=>stopOwnedChild(focusWindow));
  type FocusState={pid:number;identity:string;hwnd:number;foreground:boolean;minimized:boolean;request_id:number;visible:boolean;identity_matches:boolean};
  async function waitFocus(accept:(state:FocusState)=>boolean){
    let last:FocusState|undefined;
    for(let n=0;n<150;n++){
      if(focusWindow.exitCode!==null||focusWindow.signalCode!==null)throw Error('Test-owned focus window exited: '+JSON.stringify({exitCode:focusWindow.exitCode,signal:focusWindow.signalCode,state:await Bun.file(focusState).json().catch(()=>undefined)})+' '+focusError);
      const state:FocusState|undefined=await Bun.file(focusState).json().catch(()=>undefined);
      if(state)last=state;
      if(state&&accept(state))return state;
      await Bun.sleep(100);
    }
    throw Error('Test-owned foreground/minimized state not reached: '+JSON.stringify(last)+' '+focusError);
  }
  const focusTarget=await waitFocus(state=>!!state.identity&&state.visible&&state.identity_matches);
  assert.equal(focusTarget.pid,focusWindow.pid,'The fixture must own the observed window process');
  const now=Date.now();
  const tasks=[
    {id:'file:running',name:'验收：运行中的任务',source:'local_file',source_instance:'local_file',status:'running',turn_key:'run-1',confidence:'hook',remote:false,updated_at:now,started_at:now-60000,unread:true,pinned:false},
    {id:'file:done',name:'验收：未查看的结果',source:'local_file',source_instance:'local_file',status:'succeeded',turn_key:'done-1',confidence:'exact',remote:false,updated_at:now,finished_at:now,exit_code:0,unread:true,pinned:false},
    {id:'file:offline',name:'验收：离线任务',source:'local_file',source_instance:'local_file',status:'running',turn_key:'offline-1',confidence:'hook',remote:false,updated_at:now,unread:true,pinned:false},
    {id:'file:pinned',name:'验收：手动固定',source:'local_file',source_instance:'local_file',status:'unknown',turn_key:'pinned-1',confidence:'hook',remote:false,updated_at:now,unread:true,pinned:true,can_focus:true,terminal:{kind:'windows_native',focus_id:String(focusTarget.pid),outer_id:focusTarget.identity,label:'Test-owned focus window'}},
    {id:'scan:codex:anonymous',name:'unknown',source:'codex',source_instance:'scanner',status:'running',confidence:'inferred',remote:false,updated_at:now,unread:true,pinned:false},
  ];
  const records=Object.fromEntries(tasks.map(task=>[task.id,{task,unread:task.unread,pinned:task.pinned,pin_override:task.pinned?true:null,last_execution_at:now,last_turn_key:task.turn_key??null,last_status:task.status,hidden:false,ignored:false}]));
  await Bun.write(join(home,'attention.json'),JSON.stringify({version:4,records,baselines:{}}));
  for(const task of tasks.slice(0,2))await Bun.write(join(home,task.id.slice(5)+'.status'),JSON.stringify({v:2,name:task.name,status:task.status,turn_key:task.turn_key,execution_activity_at:now,started_at:task.started_at,finished_at:task.finished_at,exit_code:task.exit_code,confidence:task.confidence}));
  // Both configured transports read only this disposable status directory.
  // The persisted scanner fixture never enables native process enumeration.
  await Bun.write(join(home,'config.toml'),`[dashboard]
version=2
onboarding_complete=true
auto_discover_local=false
auto_check_updates=false
notifications=true
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
${['local_file','scanner'].map(id=>`[[dashboard.sources]]
id="${id}"
name="${id}"
kind="${id==='scanner'?'generic_event':'hooks'}"
enabled=true
location=""
projects=["*"]
excluded_projects=[]
tmux_sessions=[]
tmux_panes=[]
poll_interval_ms=3000
${id==='scanner'?`[dashboard.sources.integration]
agent_id="custom-agent"
adapter_id="standard-events"
transport="local"
accept_since_ms=${now}`:''}`).join('\n')}
`);
  const reserve=createServer();await new Promise<void>(ok=>reserve.listen(0,'127.0.0.1',ok));
  const port=(reserve.address() as {port:number}).port;await new Promise<void>((ok,fail)=>reserve.close(e=>e?fail(e):ok()));
  const codex=join(owned,'codex');await mkdir(join(codex,'sessions'),{recursive:true});
  const env={...process.env,CODEX_HOME:codex,TASKPULSE_HOME:home,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port}`};
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
  start();const root=await connect(await tab(''));await root.wait(`!!document.querySelector('.dashboard')`);
  const build=await root.invoke('get_build_info');

  await root.wait(`__TAURI_INTERNALS__.invoke('get_dashboard_snapshot').then(s=>s.tasks.length>=5 && s.tasks.some(t=>t.id==='file:running'&&!t.stale) && s.providers.some(p=>p.provider==='local_file' && p.status==='connected'))`);
  await root.wait(`document.querySelectorAll('.task-item').length>=3`);
  const before=await root.invoke('get_dashboard_snapshot');
  const titles=()=>root.evaluate(`[...document.querySelectorAll('.task-item .row-title')].map(e=>e.textContent.trim())`);
  const initial=await titles();
  console.log('default cards:',JSON.stringify(initial));
  assert.deepEqual([...initial].sort(),['验收：运行中的任务','验收：未查看的结果','验收：手动固定'].sort());
  assert.equal(await root.evaluate(`!!document.querySelector('.pending-running,.filter-note,.quota')`),false,'Red-box content must be removed');
  const checks=['default task set excludes stale unpinned and anonymous scanner rows','removed red-box content'];
  const focusEvidence=[];
  for(const minimized of [false,true]){
    console.log('Focus acceptance: initiallyMinimized='+minimized);
    if(minimized){
      await Bun.write(focusControl,JSON.stringify({id:1,action:'minimize'}));
      await waitFocus(state=>state.request_id===1&&state.minimized);
    }
    await root.invoke('plugin:window|show',{label:'popup'});
    await root.invoke('plugin:window|set_focus',{label:'popup'});
    await root.wait(`__TAURI_INTERNALS__.invoke('plugin:window|is_focused',{label:'popup'})`);
    await waitFocus(state=>!state.foreground&&state.minimized===minimized);
    const priorSelection=await root.invoke('get_selected_task');
    const outcome=await root.invoke('open_task',{id:'file:pinned'});
    assert.equal(outcome,'window_focused',JSON.stringify({build,target:focusTarget,current:await waitFocus(()=>true),task:(await root.invoke('get_agents')).find((t:any)=>t.id==='file:pinned'),outcome}));
    const actual=await waitFocus(state=>state.foreground&&!state.minimized);
    assert.equal(actual.hwnd,focusTarget.hwnd,'The originally observed window must actually become foreground');
    assert.equal((await root.invoke('get_agents')).find((t:any)=>t.id==='file:pinned').unread,true);
    const persisted=await Bun.file(join(home,'attention.json')).json();
    assert.equal(persisted.records['file:pinned'].unread,true,'Successful focus must preserve durable unread state');
    assert.deepEqual(await root.invoke('get_selected_task'),priorSelection,'Successful focus must not fall back to details');
    focusEvidence.push({initiallyMinimized:minimized,foreground:actual.foreground,minimized:actual.minimized,unread:true});
  }
  checks.push('test-owned normal window actually becomes foreground without acknowledging its task',
    'test-owned minimized window restores and actually becomes foreground without acknowledging its task');
  await stopOwnedChild(focusWindow);
  await root.invoke('plugin:window|show',{label:'popup'});
  await Bun.write(join(owned,'dashboard-light.png'),Buffer.from((await root.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
  await root.evaluate(`[...document.querySelectorAll('.view-bar button')].find(b=>b.textContent.includes('状态待更新')).click()`);
  await root.wait(`document.querySelector('.task-scroll').textContent.includes('验收：离线任务')`);
  assert.ok(!(await titles()).includes('unknown'));
  assert.equal(await root.evaluate(`!!document.querySelector('.filter-note')`),false);
  await root.evaluate(`document.querySelector('.all-tasks').click()`);
  await root.wait(`!document.querySelector('.task-scroll').textContent.includes('验收：离线任务')`);
  const after=await root.invoke('get_dashboard_snapshot');
  for(const id of ['file:offline','file:pinned','scan:codex:anonymous']){
    const a=before.tasks.find((t:any)=>t.id===id),b=after.tasks.find((t:any)=>t.id===id);
    assert.ok(a&&b);assert.equal(b.unread,a.unread);assert.equal(b.pinned,a.pinned);assert.equal(b.status,a.status);
  }
  checks.push('uncertainty is reachable without acknowledging or deleting tasks');
  await root.invoke('open_management',{section:'general'});
  const ui=await connect(await tab('/manage'));await ui.wait(`!!document.querySelector('.settings-heading')`);
  assert.equal(await ui.evaluate(`!!document.querySelector('.settings-save')`),false);
  await Bun.write(join(owned,'settings-light.png'),Buffer.from((await ui.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
  checks.push('no persistent saved badge');
  const contrasts:any[]=[];
  for(const theme of ['light','dark'])for(const language of ['zh-CN','en']){
    const settings=await root.invoke('get_monitoring_settings');
    await ui.invoke('save_preferences',{expected:{preferences:settings.preferences,default_view:settings.default_view,recent_days:settings.recent_days,notifications:settings.notifications,sound:settings.sound,notification_mode:settings.notification_mode,auto_check_updates:settings.auto_check_updates,auto_discover_local:settings.auto_discover_local},values:{preferences:{...settings.preferences,theme,language},default_view:settings.default_view,recent_days:settings.recent_days,notifications:settings.notifications,sound:settings.sound,notification_mode:settings.notification_mode,auto_check_updates:settings.auto_check_updates,auto_discover_local:settings.auto_discover_local}});
    await root.wait(`document.documentElement.dataset.theme==='${theme}' && document.documentElement.lang==='${language}'`);
    await ui.invoke('test_notification',{kind:'turn_ended'});
    const notice=await connect(await tab('/notice'));await notice.wait(`!!document.querySelector('.summary')`);
    await notice.wait(`document.documentElement.dataset.theme==='${theme}' && document.documentElement.lang==='${language}'`);
    const colors=await notice.evaluate(String.raw`(()=>{const rgb=s=>s.match(/\d+(?:\.\d+)?/g).slice(0,3).map(Number);const lum=c=>c.map(v=>{v/=255;return v<=.04045?v/12.92:((v+.055)/1.055)**2.4}).reduce((s,v,i)=>s+v*[.2126,.7152,.0722][i],0);const bg=getComputedStyle(document.querySelector('footer')).backgroundColor;const b=lum(rgb(bg));return {background:bg,items:['.hint','.later','.primary'].map(sel=>{const el=document.querySelector(sel),fg=getComputedStyle(el).color,a=lum(rgb(fg));return {selector:sel,text:el.textContent,color:fg,contrast:(Math.max(a,b)+.05)/(Math.min(a,b)+.05)};})};})()`);
    for(const item of colors.items)assert.ok(item.contrast>=4.5,`${theme}/${language} ${item.selector}: ${item.contrast}`);
    contrasts.push({theme,language,...colors});
    for(const [width,height] of [[320,132],[368,132]]){
      await notice.send('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:1,mobile:false});
      assert.equal(await notice.evaluate(`document.documentElement.scrollWidth>innerWidth || document.documentElement.scrollHeight>innerHeight`),false);
      assert.equal(await notice.evaluate(`document.querySelector('.summary').getBoundingClientRect().bottom<=document.querySelector('footer').getBoundingClientRect().top`),true);
    }
    await Bun.write(join(owned,`notice-${theme}-${language}.png`),Buffer.from((await notice.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
    const current=await root.invoke('get_task_notice');await notice.invoke('dismiss_task_notice',{id:current.current.id,reason:'user'});
  }
  checks.push('light/dark notice footer contrast >= 4.5 for hints and both actions','Chinese/English notice layouts at 320 and 368 pixels');
  await root.evaluate(`document.querySelector('.all-tasks').click()`);
  for(const [width,height] of [[320,400],[368,840],[460,560]]){
    await root.send('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:1,mobile:false});
    assert.equal(await root.evaluate(`document.documentElement.scrollWidth>innerWidth || document.documentElement.scrollHeight>innerHeight`),false);
    const box=await root.evaluate(`(()=>{const r=document.querySelector('footer').getBoundingClientRect();return {top:r.top,bottom:r.bottom,viewport:innerHeight};})()`);
    assert.ok(box.bottom<=height+1);
  }
  checks.push('compact and standard dashboard layouts');
  // A valid current session must survive an unrelated parser gap. This uses
  // the real Rust collector/IPC/renderer, never a mocked dashboard response.
  const rollout=join(codex,'sessions/rollout-public-good.jsonl');
  const stamp=new Date().toISOString();
  const lines=(id:string)=>[
    {type:'session_meta',payload:{id,source:'vscode',cwd:'D:/Public Fixture'}},
    {timestamp:stamp,type:'event_msg',payload:{type:'task_started',turn_id:'turn-'+id}},
    {timestamp:stamp,type:'event_msg',payload:{type:'user_message',message:'Public native partial-read fixture'}}
  ];
  await Bun.write(rollout,lines('public-good').map(v=>JSON.stringify(v)).join('\n')+'\n');
  await Bun.write(join(codex,'sessions/rollout-public-second.jsonl'),lines('public-second').map(v=>JSON.stringify(v)).join('\n')+'\n');
  const previous=await root.invoke('get_monitoring_settings');
  await ui.invoke('save_monitoring_settings',{expected:previous,settings:{...previous,sources:[...previous.sources,
    {id:'codex',name:'Public Codex fixture',kind:'codex',enabled:true,location:'',projects:['*'],excluded_projects:[],tmux_sessions:[],tmux_panes:[],poll_interval_ms:1000}]}});
  const ongoing=`JSON.parse(document.querySelector('.dashboard').dataset.continuity).sources.find(s=>s.source_id==='codex')?.global.ongoing`;
  await root.wait(`__TAURI_INTERNALS__.invoke('get_providers').then(p=>p.some(p=>p.provider==='codex'&&p.status==='connected'))`);
  await root.wait(`${ongoing}===2`);
  const bad=lines('public-bad');
  const badRollout=join(codex,'sessions/rollout-public-bad.jsonl');
  await Bun.write(badRollout,bad.map(v=>JSON.stringify(v)).join('\n')+'\n'+
    JSON.stringify({timestamp:stamp,type:'event_msg',payload:{type:'item_completed',data:'x'.repeat(4*1024*1024+64)}})+'\n');
  await root.wait(`__TAURI_INTERNALS__.invoke('get_providers').then(p=>p.some(p=>p.provider==='codex'&&p.status==='degraded'&&p.task_observations?.['codex:public-good']))`);
  await root.wait(`${ongoing}===2`);
  assert.ok((await root.invoke('get_agents')).some((t:any)=>t.id==='codex:public-good'&&t.status==='running'&&!t.stale));
  await ui.evaluate(`document.querySelector('.settings-nav a[href="#sources"]').click()`);
  const readState=`document.querySelector('[data-source-id="codex"] .read-state')`;
  await ui.wait(`${readState}?.textContent==='Some sessions need attention'`);
  // The source badge has its own receiver and clock. Observe it independently
  // of the dashboard so a steady headline cannot hide a settings regression.
  await ui.evaluate(`(()=>{const badge=${readState};window.__sourceReadSamples=[badge.textContent];window.__sourceReadObserver=new MutationObserver(()=>window.__sourceReadSamples.push(badge.textContent));window.__sourceReadObserver.observe(badge,{childList:true,characterData:true,subtree:true});})()`);
  // A one-shot wait can pass on a clock tick while every following provider
  // delivery briefly hides both sessions. Observe all rendered count changes
  // across real collection cycles, including arrivals between clock ticks.
  await root.evaluate(`(()=>{window.__ongoingSamples=[${ongoing}];window.__ongoingObserver=new MutationObserver(()=>window.__ongoingSamples.push(${ongoing}));window.__ongoingObserver.observe(document.querySelector('.dashboard'),{attributes:true,attributeFilter:['data-continuity']});})()`);
  const attempts:number[]=[];
  for(let poll=0;poll<8;poll++){
    const last=(await root.invoke('get_providers')).find((p:any)=>p.provider==='codex').last_attempt;
    await root.wait(`__TAURI_INTERNALS__.invoke('get_providers').then(p=>p.some(p=>p.provider==='codex'&&p.last_attempt>${last}))`);
    attempts.push((await root.invoke('get_providers')).find((p:any)=>p.provider==='codex').last_attempt);
  }
  const ongoingSamples:number[]=await root.evaluate(`(()=>{window.__ongoingObserver.disconnect();return window.__ongoingSamples;})()`);
  const sourceReadSamples:string[]=await ui.evaluate(`(()=>{window.__sourceReadObserver.disconnect();return window.__sourceReadSamples;})()`);
  assert.equal(new Set(attempts).size,8,'Eight distinct native collection attempts are required');
  assert.ok(ongoingSamples.length>0);
  assert.ok(ongoingSamples.every(count=>count===2),`Running count flickered between clock ticks: ${JSON.stringify(ongoingSamples)}`);
  assert.ok(sourceReadSamples.every(label=>label==='Some sessions need attention'),`Source badge flickered between clock ticks: ${JSON.stringify(sourceReadSamples)}`);
  checks.push('two running sessions stay visible across eight partial-source collections between UI clock ticks');
  checks.push('settings partial-source badge stays stable across the same eight collections');
  // Once that session itself is no longer verified, its previous proof must
  // disappear even though the same partial source continues polling.
  await rename(rollout,join(codex,'moved-good.jsonl'));
  await root.wait(`__TAURI_INTERNALS__.invoke('get_providers').then(p=>p.some(p=>p.provider==='codex'&&p.status==='degraded'&&!p.task_observations?.['codex:public-good']))`);
  await root.wait(`${ongoing}===1`);
  await ui.wait(`${readState}?.textContent==='Some sessions need attention'`);
  checks.push('partial Codex collection preserves an independently verified ongoing task; missing task proof is revoked');
  await rename(join(codex,'sessions/rollout-public-second.jsonl'),join(codex,'moved-second.jsonl'));
  await root.wait(`__TAURI_INTERNALS__.invoke('get_providers').then(p=>p.some(p=>p.provider==='codex'&&p.status==='degraded'&&!Object.keys(p.task_observations??{}).length))`);
  await root.wait(`${ongoing}===0`);
  await ui.wait(`${readState}?.textContent==='Source access failed'`);
  await rename(badRollout,join(codex,'moved-bad.jsonl'));
  await root.wait(`__TAURI_INTERNALS__.invoke('get_providers').then(p=>p.some(p=>p.provider==='codex'&&p.status==='connected'))`);
  await ui.wait(`${readState}?.textContent==='Source access healthy'`);
  checks.push('source badge still reports unavailable session evidence and real source recovery');
  assert.equal(await root.invoke('open_task',{id:'file:pinned'}),'focus_unavailable');
  const selected=await root.invoke('get_selected_task');
  assert.equal(selected.id,'file:pinned');
  assert.equal(selected.unread,true,'Falling back to details must preserve unread state');
  checks.push('expired native window reports failed focus and opens task details without marking it read');
  assert.deepEqual(errors,[]);
  await Bun.write(join(owned,'result.json'),JSON.stringify({passed:true,build,checks,contrasts,focus:focusEvidence,partialCodex:{collections:new Set(attempts).size,ongoingCounts:[...new Set(ongoingSamples)],sourceReadStates:[...new Set(sourceReadSamples)]},errors},null,2));
  console.log('Dashboard polish acceptance passed:',checks.length,'checks');
});
