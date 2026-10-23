/** Actual WebView2 interaction with isolated synthetic producers, never real Agent hooks. */
import assert from 'node:assert/strict';
import {mkdir,copyFile} from 'node:fs/promises';
import {resolve,join,dirname} from 'node:path';
import {spawn,spawnSync,type ChildProcess} from 'node:child_process';
import {withSmokeWorkspace,stopOwnedChild} from './smoke-workspace';
const arg=process.argv.indexOf('--exe');assert.ok(arg>=0,'--exe required');
const original=resolve(process.argv[arg+1]);
await withSmokeWorkspace('message-center-native',async({owned,evidenceDir,onStop})=>{
  const exe=join(owned,'agent-tray.exe'),home=join(owned,'profile');await mkdir(home);await copyFile(original,exe);
  const loader=join(dirname(original),'WebView2Loader.dll');if(await Bun.file(loader).exists())await copyFile(loader,join(owned,'WebView2Loader.dll'));
  const env={...process.env,TASKPULSE_HOME:home,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:'--remote-debugging-port=9337'};
  const source=(id:string)=>`[[dashboard.sources]]\nid="${id}"\nname="合成来源 ${id}"\nkind="generic_event"\nenabled=true\nlocation=""\nprojects=["*"]\nexcluded_projects=[]\ntmux_sessions=[]\ntmux_panes=[]\npoll_interval_ms=1000\n[dashboard.sources.integration]\nagent_id="${id==='a'?'kimi-code-cli':'custom-agent'}"\nadapter_id="${id==='a'?'kimi-code-cli':'standard-events'}"\ntransport="local"\naccept_since_ms=0\n`;
  await Bun.write(join(home,'config.toml'),'[dashboard]\nversion=2\nonboarding_complete=true\nnotifications=true\nnotification_mode="popup"\nsound=false\ndefault_view="tasks"\n'+source('a')+source('b'));
  const tabs=async():Promise<any[]>=>fetch('http://127.0.0.1:9337/json').then(r=>r.json());
  assert.equal(await tabs().then(()=>true).catch(()=>false),false,'Refusing occupied CDP port');
  let app:ChildProcess|undefined,root:any;
  onStop(async()=>{root?.close();await stopOwnedChild(app);});
  async function connect(tab:any){
    const ws=new WebSocket(tab.webSocketDebuggerUrl);await new Promise<void>((ok,fail)=>{ws.onopen=()=>ok();ws.onerror=()=>fail(Error('CDP failed'));});
    let seq=0;const pending=new Map<number,any>();
    ws.onmessage=e=>{const m=JSON.parse(String(e.data)),p=pending.get(m.id);if(!p)return;clearTimeout(p.timer);pending.delete(m.id);m.error?p.fail(Error('CDP transport error')):p.ok(m.result);};
    const send=(method:string,params:any={})=>new Promise<any>((ok,fail)=>{const id=++seq,timer=setTimeout(()=>{pending.delete(id);fail(Error('CDP timeout'));},15000);pending.set(id,{ok,fail,timer});ws.send(JSON.stringify({id,method,params}));});
    const evaluate=async(expression:string)=>{const r=await send('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true,userGesture:true});if(r.exceptionDetails)throw Error(r.exceptionDetails.exception?.description??'Native JS error');return r.result.value;};
    const invoke=(cmd:string,args:any={})=>evaluate(`__TAURI_INTERNALS__.invoke(${JSON.stringify(cmd)},${JSON.stringify(args)})`);
    const wait=async(expression:string,timeout=25000)=>{const caller=new Error().stack;const end=Date.now()+timeout;while(Date.now()<end){if(await evaluate(expression))return;await Bun.sleep(100);}await Bun.write(join(owned,'failure.json'),JSON.stringify({expression,caller,visible:await invoke('plugin:window|is_visible',{label:'popup'}),ui:await evaluate(`({center:!!document.querySelector('.message-center'),visibility:document.visibilityState,buttons:[...document.querySelectorAll('.message-center button')].map(b=>({text:b.textContent,disabled:b.disabled})),height:innerHeight})`),center:await invoke('get_message_center')},null,2));throw Error('Native assertion timeout: '+expression+'\n'+caller);};
    return {send,evaluate,invoke,wait,close:()=>ws.close()};
  }
  async function start(){
    app=spawn(exe,['--show'],{env,windowsHide:true,stdio:'ignore'});let failure=false;app.once('error',()=>failure=true);
    let tab:any;for(let n=0;n<200;n++){assert.ok(!failure&&app.exitCode===null,'Owned app exited');tab=(await tabs().catch(()=>[])).find(t=>t.url.includes('tauri.localhost')&&!/\/(manage|notice)/.test(t.url));if(tab)break;await Bun.sleep(100);}
    assert.ok(tab,'Native popup missing');root=await connect(tab);await root.wait(`!!globalThis.__TAURI_INTERNALS__&&!!document.querySelector('.dashboard')`);
    const environment=await root.invoke('integration_environment');assert.equal(resolve(environment.state_directory).toLowerCase(),home.toLowerCase());
    if(!await root.invoke('get_pinned'))await root.invoke('toggle_pin');
    await root.invoke('plugin:window|show',{label:'popup'});
    await root.wait(`__TAURI_INTERNALS__.invoke('get_providers').then(p=>p.length===2&&p.every(s=>s.status==='connected'))`);
  }
  async function stop(){root?.close();await stopOwnedChild(app);root=undefined;app=undefined;for(let n=0;n<100;n++){if(!await tabs().then(()=>true).catch(()=>false))return;await Bun.sleep(100);}throw Error('Owned debug listener did not close');}
  let sequence=0;
  const send=(session:string,kind:string,extra:any={},source='a')=>{
    const host=source==='a'&&['request','legacy-request'].includes(session);
    const hostEvent=({'turn.started':'TurnStarted','input.requested':'PermissionRequest','input.resolved':'PermissionResult'} as Record<string,string>)[kind];
    const event=host?{hook_event_name:hostEvent,session_id:session,turn_id:'turn',request_id:extra.request_id,tool_call_id:extra.request_id,session_title:'合成并行审批'}:
      {schema_version:1,agent_id:source==='a'?'kimi-code-cli':'custom-agent',session_id:session,turn_id:'turn',event_id:'event-'+(++sequence),kind,payload:{},...extra};
    const r=spawnSync(exe,[host?'--integration-host-event':'--integration-event','--source',source,'--adapter',source==='a'?'kimi-code-cli':'standard-events'],{env:{...env,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:''},input:JSON.stringify(event),encoding:'utf8',windowsHide:true,timeout:10000});
    assert.ifError(r.error);assert.equal(r.status,0);assert.equal(r.stdout,'');
  };
  const click=async(selector:string)=>{
    await root.wait(`!!document.querySelector(${JSON.stringify(selector)})`);
    await root.evaluate(`document.querySelector(${JSON.stringify(selector)}).click()`);
  };
  const center=()=>root.invoke('get_message_center');
  const readDisplayedMessages=async()=>{
    for(const entry of (await center()).entries.filter((e:any)=>!e.seen)){
      await root.evaluate(`document.querySelector('[data-message-id="${entry.id}"]').scrollIntoView({block:'center'})`);
      await root.wait(`__TAURI_INTERNALS__.invoke('get_message_center').then(c=>c.entries.find(e=>e.id==='${entry.id}')?.seen)`);
    }
  };
  const checks:string[]=[];
  await start();assert.equal((await center()).unread,0);
  await root.wait('innerHeight===400');
  const outer=await root.invoke('plugin:window|outer_size',{label:'popup'});
  const inner=await root.invoke('plugin:window|inner_size',{label:'popup'});
  assert.deepEqual(outer,inner,'Undecorated popup must have no native non-client frame');
  checks.push('actual native outer/client dimensions match; no Windows shadow frame');
  send('result','turn.started',{payload:{title:'合成结果 · 非真实 Agent'}});
  send('result','turn.completed');
  await root.wait(`__TAURI_INTERNALS__.invoke('get_message_center').then(c=>c.unread===1)`);
  await root.wait(`document.querySelector('[aria-label="消息中心：1 条未读"] span')?.textContent==='1'`);
  assert.equal(await root.evaluate(`document.querySelectorAll('.counts .overview').length`),3);
  assert.equal(await root.evaluate(`document.querySelector('.counts').textContent.includes('待更新')`),false);
  const done=(await root.invoke('get_agents')).find((task:any)=>task.session_id==='result');assert.ok(done.unread);
  await click('[aria-label="消息中心：1 条未读"]');
  await readDisplayedMessages();
  await root.wait(`!!document.querySelector('.message-center')&&document.querySelector('[aria-label="消息中心：0 条未读"]')`);
  assert.equal((await center()).unread,0);assert.ok((await root.invoke('get_agents')).find((t:any)=>t.id===done.id).unread);
  await click('.message-center .task-link');
  await root.wait(`!!document.querySelector('[aria-label="任务详情"]')&&!document.querySelector('.message-center')`);
  assert.ok((await root.invoke('get_agents')).find((t:any)=>t.id===done.id).unread);
  await click('[aria-label="关闭详情"]');checks.push('new completion badge 1; visible reading clears only notification seen; opening task details does not view result');

  send('request','turn.started',{payload:{title:'合成并行审批'}});
  for(const request_id of ['q1','q2'])send('request','input.requested',{request_id,payload:{request_kind:'approval'}});
  await root.wait(`__TAURI_INTERNALS__.invoke('get_message_center').then(c=>c.unread===2)`);
  await click('[aria-label="消息中心：2 条未读"]');
  await readDisplayedMessages();
  await root.wait(`__TAURI_INTERNALS__.invoke('get_message_center').then(c=>c.unread===0)`);
  assert.equal(await root.evaluate(`document.querySelector('.counts [data-tone="action"] b').textContent`),'1');
  assert.equal(await root.evaluate(`document.querySelector('.counts [data-tone="running"] b').textContent`),'1');
  let c=await center();assert.equal(c.entries.filter((e:any)=>e.state==='pending').length,2);
  const q1=c.entries.find((entry:any)=>entry.request_id==='q1'),requestTask=(await root.invoke('get_agents')).find((t:any)=>t.session_id==='request');
  await click(`[data-message-id="${q1.id}"] .state-line .link`);
  await root.wait(`__TAURI_INTERNALS__.invoke('get_task_notice').then(n=>n.current?.task_id===${JSON.stringify(requestTask.id)}&&n.current.auto_dismiss_ms===null)`);
  assert.equal((await center()).unread,0);assert.equal((await center()).entries.length,3);
  const notice=await root.invoke('get_task_notice');assert.ok(notice.current);assert.equal(notice.current.auto_dismiss_ms,null);
  await root.invoke('dismiss_task_notice',{id:notice.current.id});
  assert.equal((await center()).unread,0);assert.ok((await root.invoke('get_agents')).find((t:any)=>t.id===requestTask.id).input_request_ids.includes('q1'));
  send('request','input.resolved',{request_id:'q1'});
  await root.wait(`__TAURI_INTERNALS__.invoke('get_message_center').then(c=>c.entries.filter(e=>e.state==='pending').length===1)`);
  await assert.rejects(()=>root.invoke('reopen_task_prompt',{id:requestTask.id,requestId:'q1',turnKey:'turn'}));
  assert.equal(await root.evaluate(`document.querySelector('.counts [data-tone="action"] b').textContent`),'1');
  send('request','input.resolved',{request_id:'q2'});
  await root.wait(`document.querySelector('.counts [data-tone="action"] b').textContent==='0'`);
  checks.push('read parallel requests remain actionable; exact request resolution/reopen identity; dismiss never resolves or creates new unread');

  // A background arrival is shown/read while the center stays open.
  send('failure','turn.started',{payload:{title:'合成明确失败'}});send('failure','turn.failed');
  await root.wait(`__TAURI_INTERNALS__.invoke('get_message_center').then(c=>c.entries.length===4)`);
  await readDisplayedMessages();
  await root.wait(`__TAURI_INTERNALS__.invoke('get_message_center').then(c=>c.entries.length===4&&c.unread===0)`);
  // Mounting/receiving while the native popup is hidden must not acknowledge.
  await root.invoke('plugin:window|hide',{label:'popup'});
  send('hidden','turn.started',{payload:{title:'合成后台结果'}});send('hidden','turn.completed');
  await root.wait(`__TAURI_INTERNALS__.invoke('get_message_center').then(c=>c.entries.length===5&&c.unread===1)`);
  await Bun.sleep(1200);assert.equal((await center()).unread,1);
  await root.invoke('plugin:window|show',{label:'popup'});
  await root.invoke('plugin:window|set_focus',{label:'popup'});
  await readDisplayedMessages();
  await root.wait(`__TAURI_INTERNALS__.invoke('get_message_center').then(c=>c.unread===0)`);
  checks.push('open center updates for a background event; hidden native window is not seen; visible viewport reads new row');

  send('request','input.requested',{request_id:'q3',payload:{request_kind:'question'}});
  await root.wait(`__TAURI_INTERNALS__.invoke('get_message_center').then(c=>c.entries.length===6)`);
  await readDisplayedMessages();
  await root.wait(`__TAURI_INTERNALS__.invoke('get_message_center').then(c=>c.entries.length===6&&c.unread===0)`);
  const settings=await root.invoke('get_monitoring_settings'),failed=structuredClone(settings);
  Object.assign(failed.sources[0],{kind:'codex_wsl',location:'TaskPulse-Absent-Distro-Probe'});delete failed.sources[0].integration;
  await root.invoke('save_monitoring_settings',{settings:failed});
  await root.wait(`__TAURI_INTERNALS__.invoke('get_providers').then(p=>p.some(x=>x.provider==='a'&&x.status==='degraded'))`);
  await root.wait(`document.querySelector('.counts [data-tone="action"] b').textContent==='0'`);
  c=await center();assert.equal(c.unread,0);assert.equal(c.entries.find((e:any)=>e.request_id==='q3').state,'unconfirmed');
  assert.ok((await root.invoke('get_agents')).some((t:any)=>t.id===requestTask.id));
  assert.ok(await root.evaluate(`document.querySelector('.signals')?.textContent.includes('来源待更新')`));
  await root.invoke('save_monitoring_settings',{settings});
  await root.wait(`__TAURI_INTERNALS__.invoke('get_providers').then(p=>p.some(x=>x.provider==='a'&&x.status==='connected'))`);
  send('request','input.requested',{request_id:'q3',payload:{request_kind:'question'}});
  await root.wait(`document.querySelector('.counts [data-tone="action"] b').textContent==='1'`);
  assert.equal((await center()).entries.length,6);assert.equal((await center()).unread,0);
  checks.push('source fault demotes stale pending without resolving or losing task; recovery/reconfirmation cannot revive read messages');

  for(const [width,height] of [[412,620],[380,640],[320,480],[440,720]])for(const scale of [1,1.5,2]){
    await root.send('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:scale,mobile:false});await Bun.sleep(200);
    const layout=await root.evaluate(`(()=>{const p=document.querySelector('.message-center').getBoundingClientRect(),s=document.querySelector('.message-scroll');return {width:document.documentElement.scrollWidth,height:innerHeight,top:p.top,bottom:p.bottom,right:p.right,left:p.left,scroll:s.scrollHeight>s.clientHeight};})()`);
    assert.ok(layout.width<=width&&layout.bottom<=height&&layout.left>=0&&layout.right<=width);
    await Bun.write(join(owned,`messages-${width}x${height}-${scale*100}.png`),Buffer.from((await root.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
    await click('[aria-label="关闭消息中心"]');
    assert.equal(await root.evaluate(`document.querySelectorAll('.counts .overview').length`),3);
    assert.ok(await root.evaluate(`document.documentElement.scrollWidth<=innerWidth`));
    await Bun.write(join(owned,`dashboard-${width}x${height}-${scale*100}.png`),Buffer.from((await root.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
    await click('[aria-label^="消息中心："]');
  }
  // Offscreen rows are not auto-read; explicit all-read covers the snapshot.
  for(let i=0;i<9;i++){send('parallel-'+i,'turn.started',{payload:{title:'合成并发结果 '+i}},i%2?'a':'b');send('parallel-'+i,'turn.completed',{},i%2?'a':'b');}
  await root.wait(`__TAURI_INTERNALS__.invoke('get_message_center').then(c=>c.entries.length===15)`);
  const ids=(await center()).entries.map((entry:any)=>entry.id);assert.equal(new Set(ids).size,15);
  await click('.message-center header .link');await root.wait(`__TAURI_INTERNALS__.invoke('get_message_center').then(c=>c.unread===0)`);
  await click('[aria-label="关闭消息中心"]');
  await root.invoke('update_task_attention',{id:done.id,action:'viewed'});
  await root.invoke('update_task_attention',{id:requestTask.id,action:'pin',pinned:true});
  await stop();await start();
  await root.wait(`__TAURI_INTERNALS__.invoke('get_providers').then(p=>p.every(s=>s.status==='connected'))`);
  c=await center();assert.equal(c.unread,0);assert.equal(c.entries.length,15);assert.deepEqual(c.entries.map((e:any)=>e.id),ids);
  const pending=await root.invoke('get_pending_task_prompts');assert.ok(pending.some((t:any)=>t.id===requestTask.id&&t.input_request_ids.includes('q3')));
  const known=await root.invoke('list_discovered_sessions',{offset:0,limit:100});assert.equal(known.items.find((i:any)=>i.task.id===done.id).task.unread,false);
  assert.ok((await root.invoke('get_agents')).some((t:any)=>t.id===requestTask.id&&t.pinned));
  const persisted=await Bun.file(join(home,'messages.json')).text();assert.ok(!persisted.includes('合成')&&!persisted.includes('q3')&&!persisted.includes('custom-agent'));
  // A pending fact that has no inbox event (e.g. pre-upgrade or reminders off)
  // must remain manually reopenable without backfilling new unread messages.
  const preferences=await root.invoke('get_monitoring_settings');
  await root.invoke('save_monitoring_settings',{settings:{...preferences,notifications:false}});
  send('legacy-request','turn.started');send('legacy-request','input.requested',{request_id:'legacy-q'});
  await root.wait(`__TAURI_INTERNALS__.invoke('get_pending_task_prompts').then(p=>p.some(t=>t.session_id==='legacy-request'&&!t.stale))`);
  await root.invoke('save_monitoring_settings',{settings:preferences});
  await click('[aria-label^="消息中心："]');await click('.pending-history summary');
  const legacy=(await root.invoke('get_pending_task_prompts')).find((t:any)=>t.session_id==='legacy-request');
  await click(`[data-prompt-id="${legacy.id}"] .link`);
  await root.wait(`__TAURI_INTERNALS__.invoke('get_task_notice').then(n=>n.current?.task_id===${JSON.stringify(legacy.id)})`);
  const historicalNotice=await root.invoke('get_task_notice');assert.ok(historicalNotice.current);assert.equal(historicalNotice.current.task_id,legacy.id);
  await root.invoke('dismiss_task_notice',{id:historicalNotice.current.id});
  assert.equal((await center()).entries.length,15);assert.equal((await center()).unread,0);
  checks.push('fresh historical request without an inbox event remains manually reopenable without creating unread');
  checks.push('412x620/380x640/320x480/440x720 at DPR 1/1.5/2 real layout; explicit all-read; parallel source identity; restart preserves seen/viewed/pin/pending without replay; disk payload minimal');
  await Bun.write(join(owned,'result.json'),JSON.stringify({passed:true,checks,renderer:'Windows native Tauri WebView2',inputs:'isolated synthetic events',realHostConfigurationsChanged:false,notVerified:['actual Agent lifecycle','installer acceptance','OS mixed-DPI monitor migration']},null,2));
  console.log(JSON.stringify({passed:true,checks,evidence:evidenceDir}));
});
