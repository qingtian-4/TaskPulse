/** Native WebView2 product acceptance. Isolated synthetic public inputs, NOT Agent-host certification.
 * Run: bun scripts/native-product-smoke.ts --exe <unused-built-exe>
 * --capture writes three public PNGs only after privacy/layout assertions. Never installs Hooks. */
import assert from 'node:assert/strict';
import {mkdir,copyFile} from 'node:fs/promises';
import {resolve,join,dirname} from 'node:path';
import {spawn,spawnSync,type ChildProcess} from 'node:child_process';
import {withSmokeWorkspace,stopOwnedChild} from './smoke-workspace';
const arg=process.argv.indexOf('--exe');if(arg<0)throw Error('--exe required');
const original=resolve(process.argv[arg+1]);
await withSmokeWorkspace('product-native',async({owned,evidenceDir,onStop})=>{
let app:ChildProcess|undefined,root:any,management:any,notice:any,spawnFailure:Error|undefined;
onStop(async()=>{try{notice?.close();management?.close();root?.close();}finally{await stopOwnedChild(app);}});
const home=join(owned,'profile');await mkdir(home);
const exe=join(owned,'agent-tray.exe');await copyFile(original,exe);
if(await Bun.file(join(dirname(original),'WebView2Loader.dll')).exists())await copyFile(join(dirname(original),'WebView2Loader.dll'),join(owned,'WebView2Loader.dll'));
const source={id:'public-example',name:'标准事件 · 公开合成示例',kind:'generic_event',enabled:true,location:'',projects:['*'],excluded_projects:[],tmux_sessions:[],tmux_panes:[],poll_interval_ms:1000,integration:{agent_id:'custom-agent',adapter_id:'standard-events',transport:'local',accept_since_ms:Date.now()}};
await Bun.write(join(home,'config.toml'),`[dashboard]
version=2
onboarding_complete=true
notifications=false
sound=false
notification_mode="popup"
default_view="tasks"
[[dashboard.sources]]
id="public-example"
name="标准事件 · 公开合成示例"
kind="generic_event"
enabled=true
location=""
projects=["*"]
excluded_projects=[]
tmux_sessions=[]
tmux_panes=[]
poll_interval_ms=1000
[dashboard.sources.integration]
agent_id="custom-agent"
adapter_id="standard-events"
transport="local"
accept_since_ms=${source.integration.accept_since_ms}
`);
const env={...process.env,TASKPULSE_HOME:home,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:'--remote-debugging-port=9337'};
type Tab={url:string;webSocketDebuggerUrl:string};
const tabs=async():Promise<Tab[]>=>fetch('http://127.0.0.1:9337/json').then(r=>r.json());
assert.equal(await tabs().then(()=>true).catch(()=>false),false,'Refusing occupied CDP port');
const errors:string[]=[];const checks:string[]=[];const scaleChecks:unknown[]=[];
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

app=spawn(exe,['--show'],{env,windowsHide:true,stdio:['ignore','ignore','pipe']});
app.once('error',error=>spawnFailure=error);
let stderr='';app.stderr?.on('data',d=>stderr=(stderr+String(d)).slice(-12000));
async function page(part:string){for(let n=0;n<200;n++){if(spawnFailure)throw spawnFailure;const tab=(await tabs().catch(()=>[])).find(t=>part?t.url.includes(part):t.url.includes('tauri.localhost')&&!/\/(manage|notice)/.test(t.url));if(tab)return connect(tab);if(app?.exitCode!==null)throw Error('Owned native app exited');await Bun.sleep(100);}throw Error('Page missing: '+part);}
let sequence=0;
function event(session:string,kind:string,payload:Record<string,string>={},extra:Record<string,string>={}){
  const data={schema_version:1,agent_id:'custom-agent',session_id:session,turn_id:'turn-1',event_id:session+'-'+(++sequence),kind,payload,...extra};
  const r=spawnSync(exe,['--integration-event','--source',source.id,'--adapter','standard-events'],{env:{...env,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:''},input:JSON.stringify(data),encoding:'utf8',windowsHide:true,timeout:8000});
  if(r.error)throw r.error;assert.equal(r.status,0,r.stderr);
}
async function input(client:any,selector:string,value:string){await client.evaluate(`(()=>{const e=document.querySelector(${JSON.stringify(selector)});e.value=${JSON.stringify(value)};e.dispatchEvent(new Event('input',{bubbles:true}));})()`);}
async function click(client:any,selector:string){await client.wait(`!!document.querySelector(${JSON.stringify(selector)})`);await client.evaluate(`document.querySelector(${JSON.stringify(selector)}).click()`);}
// Native WebView2 DPR emulation, not a claim that Windows display settings or
// mixed-DPI monitor migration were changed/tested. Logical window sizes stay fixed.
async function checkScales(client:any,kind:'dashboard'|'sources'|'notice',width:number,height:number){
  for(const scale of [1,1.5,2]){
    await client.send('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:scale,mobile:false});
    await Bun.sleep(150);
    const layout=await client.evaluate(`(()=>{
      const box=e=>{const r=e.getBoundingClientRect();return {left:r.left,right:r.right,top:r.top,bottom:r.bottom,width:r.width,height:r.height};};
      const buttons=[...document.querySelectorAll('button:not(:disabled),input:not(:disabled),select:not(:disabled),summary')].filter(e=>{
        // Chromium retains layout boxes for some closed disclosure content.
        for(let p=e.parentElement;p;p=p.parentElement){if(p.tagName==='DETAILS'&&!p.open&&!p.querySelector(':scope > summary')?.contains(e))return false;}
        const r=e.getBoundingClientRect(),x=r.left+r.width/2,y=r.top+r.height/2;
        if(!(r.width>0&&r.height>0&&r.top>=0&&r.bottom<=innerHeight))return false;
        // Offscreen descendants of a scrolling list are not occluded controls.
        // Actual visible controls must still pass the elementFromPoint hit test.
        for(let p=e.parentElement;p;p=p.parentElement){const s=getComputedStyle(p),b=p.getBoundingClientRect();if(/auto|scroll|hidden|clip/.test(s.overflowY)&&(y<b.top||y>=b.bottom))return false;if(/auto|scroll|hidden|clip/.test(s.overflowX)&&(x<b.left||x>=b.right))return false;}
        return true;
      });
      const occluded=buttons.filter(e=>{const r=e.getBoundingClientRect();const hit=document.elementFromPoint(r.left+r.width/2,r.top+r.height/2);return !hit||!(e===hit||e.contains(hit));}).map(e=>e.getAttribute('aria-label')||e.textContent?.trim());
      const titles=[...document.querySelectorAll('.row-title,.heading')].map(box);
      const labels=[...document.querySelectorAll('.status-label')].map(box);
      const overlapping=titles.some(t=>labels.some(s=>s.top<t.bottom&&s.bottom>t.top&&s.left<t.right&&s.right>t.left));
      const panel=document.querySelector('.dashboard,.notice-window,.manage');
      return {dpr:devicePixelRatio,bodyWidth:document.documentElement.scrollWidth,horizontal:document.documentElement.scrollWidth>innerWidth,panel:box(panel),occluded,overlapping,scrollbar:getComputedStyle(document.querySelector('.task-scroll')??document.body).overflowY};
    })()`);
    assert.equal(layout.dpr,scale);assert.equal(layout.horizontal,false,kind+' '+scale+' horizontal overflow');
    assert.equal(layout.overlapping,false,kind+' '+scale+' title / state overlap');
    assert.deepEqual(layout.occluded,[],kind+' '+scale+' inaccessible controls');
    assert.ok(layout.panel.bottom<=height+1,kind+' '+scale+' panel outside logical window');
    if(kind==='notice')assert.ok(await client.evaluate(`document.querySelector('.summary').getBoundingClientRect().bottom<=document.querySelector('footer').getBoundingClientRect().top`),'Notice summary overlaps actions');
    await Bun.write(join(owned,kind+'-'+width+'x'+height+'-'+Math.round(scale*100)+'.png'),Buffer.from((await client.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
    scaleChecks.push({kind,scale,logicalSize:[width,height],...layout});
  }
  await client.send('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:1,mobile:false});
}
async function screenshot(client:any,name:string,width:number,height:number){
  if(!process.argv.includes('--capture'))return;
  await client.send('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:1,mobile:false});
  await Bun.sleep(300);
  const visible=await client.evaluate('document.body.innerText');
  assert.ok(!/[A-Z]:[\\/]Users[\\/]|Administrator|qingtian|Server[13]|wanghy/i.test(visible),'Public capture must not expose private host data');
  await Bun.write(join(owned,name+'.png'),Buffer.from((await client.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
}
try{
  root=await page('');await root.wait(`!!document.querySelector('.dashboard')`);
  assert.equal((await root.invoke('integration_environment')).isolated,true);
  if(!await root.invoke('get_pinned'))await root.invoke('toggle_pin');
  await root.wait(`innerWidth===412&&innerHeight===400`);
  await Bun.write(join(owned,'empty-412x400.png'),Buffer.from((await root.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
  event('done','turn.started',{title:'示例：整理新用户快速开始指南',project:'Docs Demo'});
  event('done','turn.completed');
  await root.wait(`document.querySelectorAll('.task-row').length===1&&innerHeight===400`);
  await Bun.write(join(owned,'one-result-412x400.png'),Buffer.from((await root.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
  const firstResult=(await root.invoke('get_agents'))[0];
  await root.invoke('update_task_attention',{id:firstResult.id,action:'hide'});
  // The generic adapter name is honest: these are NOT native Codex/Claude fixtures.
  event('thinking','turn.started',{title:'示例：评审跨平台任务面板与多来源项目通知消息的长中文标题排版稳定性',project:'TaskPulse Demo'});
  event('thinking','phase.changed',{phase:'thinking'});
  event('question','turn.started',{title:'示例：确认下一轮迭代范围',project:'TaskPulse Demo'});
  event('question','input.requested',{request_kind:'question'},{request_id:'question-1'});
  await root.wait(`document.querySelectorAll('.task-row').length===2&&innerHeight===620`);
  await Bun.write(join(owned,'running-and-request-412x620.png'),Buffer.from((await root.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
  const requestTask=(await root.invoke('get_agents')).find((t:any)=>t.session_id==='question');
  await root.invoke('update_task_attention',{id:requestTask.id,action:'hide'});
  await root.wait(`document.querySelectorAll('.task-row').length===1`);
  assert.equal(await root.evaluate('innerHeight'),620,'Pinned refresh must not shrink a grown window');
  await root.invoke('update_task_attention',{id:requestTask.id,action:'restore'});
  await root.invoke('update_task_attention',{id:firstResult.id,action:'restore'});
  event('tool','turn.started',{title:'Example: verify-session-renaming-and-unread-result-preservation-with-a-long-English-title',project:'UI Sandbox / extremely-long-project-name-for-width-validation'});
  event('tool','tool.started',{activity:'运行回归测试 · C:/public/examples/very-long-project-path/packages/integration/verify-taskpulse-notification-layout.test.ts'},{call_id:'test-call'});
  await root.wait(`document.querySelectorAll('.task-row').length===4`);
  const initial=await root.invoke('get_agents');
  const thinking=initial.find((t:any)=>t.session_id==='thinking'),done=initial.find((t:any)=>t.session_id==='done');
  assert.equal(await root.evaluate(`document.querySelector('.task-row').dataset.taskId`),initial.find((t:any)=>t.session_id==='question').id);
  const fontSizes=await root.evaluate(`({title:getComputedStyle(document.querySelector('.row-title')).fontSize,context:getComputedStyle(document.querySelector('.context')).fontSize,activity:getComputedStyle(document.querySelector('.activity')).fontSize,number:getComputedStyle(document.querySelector('.overview b')).fontSize})`);
  assert.deepEqual(fontSizes,{title:'14px',context:'11px',activity:'12px',number:'24px'});
  for(const [width,height] of [[412,620],[371,558],[371,360],[380,640],[320,480],[440,720]])await checkScales(root,'dashboard',width,height);
  await root.send('Emulation.setDeviceMetricsOverride',{width:412,height:620,deviceScaleFactor:1,mobile:false});
  checks.push('empty and one-result use 400px; two tasks grow to 620px; long Chinese/English typography and Windows/macOS-sized viewports at DPR 1/1.5/2');
  await root.evaluate(`[...document.querySelectorAll('.counts .overview')].find(b=>b.textContent.includes('待处理')).click()`);
  await root.wait(`document.querySelectorAll('.task-row').length===1&&document.querySelector('.task-row').textContent.includes('等待输入')`);
  await click(root,'.all-tasks');
  await screenshot(root,'dashboard-current',412,620);
  event('failed','turn.started',{title:'示例：失败结果展示回归',project:'State Demo'});
  event('failed','turn.failed');
  await root.wait(`document.querySelectorAll('.task-row').length===5&&!!document.querySelector('.task-item[data-tone="danger"] .status-label')`);
  assert.equal(await root.evaluate(`getComputedStyle(document.querySelector('.task-item[data-tone="danger"] .status-label')).color`),'rgb(240, 155, 167)');
  await click(root,'.signals [data-tone="danger"]');
  await root.wait(`document.querySelectorAll('.task-row').length===1&&document.querySelector('.status-label').textContent==='失败'`);
  await click(root,'[aria-label="标记已查看：示例：失败结果展示回归"]');
  await click(root,'.all-tasks');
  await root.wait(`document.querySelectorAll('.task-row').length===4`);
  checks.push('explicit failed-result chip/filter/viewed action remain separate from running and uncertainty');
  await input(root,'[aria-label="搜索任务"]','UI Sandbox');
  await root.wait(`document.querySelectorAll('.task-row').length===1`);
  assert.ok(await root.evaluate(`!!document.querySelector('.outside-running')`));
  await click(root,'.all-tasks');
  await root.wait(`document.querySelectorAll('.task-row').length===4`);
  await input(root,'[aria-label="搜索任务"]','no-such-public-task');
  await root.wait(`document.querySelector('.empty')?.textContent.includes('当前筛选没有任务')`);
  await click(root,'.all-tasks');
  assert.equal(await root.evaluate(`(()=>{const rows=document.querySelectorAll('.task-row');rows[0].focus();rows[0].dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowDown',bubbles:true}));return document.activeElement===rows[1];})()`),true);
  await click(root,'[data-task-id="'+thinking.id+'"]');
  await root.wait(`document.querySelector('.command')?.textContent.includes('正在思考')`);
  const focus=await root.evaluate(`(()=>{const input=document.querySelector('[aria-label="搜索任务"]');document.querySelector('.dashboard').dispatchEvent(new KeyboardEvent('keydown',{key:'/',bubbles:true}));return document.activeElement===input&&getComputedStyle(input.closest('.search')).borderColor!=='rgba(0, 0, 0, 0)';})()`);
  assert.equal(focus,true,'Search keyboard shortcut remains usable');
  assert.equal((await root.invoke('get_agents')).find((t:any)=>t.id===thinking.id).unread,thinking.unread);
  await input(root,'[aria-label="搜索任务"]','评审');
  await root.wait(`document.querySelectorAll('.task-row').length===1`);
  const compact=await root.evaluate(`({list:document.querySelector('.task-scroll').clientHeight,footer:document.querySelector('footer').getBoundingClientRect().bottom,height:innerHeight})`);
  await Bun.write(join(owned,'search-detail-412x620.png'),Buffer.from((await root.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
  assert.ok(compact.list>=80 && compact.footer<=compact.height,'Search + details must retain a usable list and visible footer: '+JSON.stringify(compact));
  await click(root,'[aria-label="清除搜索"]');
  await root.send('Emulation.setDeviceMetricsOverride',{width:320,height:480,deviceScaleFactor:2,mobile:false});
  await Bun.sleep(150);
  const shortWindow=await root.evaluate(`({panel:document.querySelector('.dashboard').getBoundingClientRect().height,list:document.querySelector('.task-scroll').clientHeight,footer:document.querySelector('footer').getBoundingClientRect().bottom})`);
  await Bun.write(join(owned,'search-detail-320x480.png'),Buffer.from((await root.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
  assert.ok(shortWindow.panel<=480&&shortWindow.list>=45&&shortWindow.footer<=480,'Compact logical-height window must retain visible tasks/footer: '+JSON.stringify(shortWindow));
  await root.send('Emulation.setDeviceMetricsOverride',{width:412,height:620,deviceScaleFactor:1,mobile:false});
  checks.push('native search, global reset, blockers first, thinking phase and selection never acknowledges');
  await click(root,'[aria-label="关闭详情"]');
  await click(root,'.filter-toggle');
  await click(root,'.unread-filter');
  await root.wait(`document.querySelector('.unread-filter').getAttribute('aria-pressed')==='true'`);
  assert.ok(await root.evaluate(`[...document.querySelectorAll('.task-row')].every(r=>!!r.querySelector('.unread'))`));
  await input(root,'[aria-label="搜索任务"]','UI Sandbox');
  await root.wait(`document.querySelectorAll('.task-row').length===1&&document.querySelector('.filter-note').textContent.includes('仅未查看')`);
  await Bun.write(join(owned,'search-filter-412x620.png'),Buffer.from((await root.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
  await click(root,'.all-tasks');
  await click(root,'.filter-toggle');
  await click(root,'[aria-label="标记已查看：'+done.name+'"]');
  await root.wait(`!document.querySelector('[data-task-id="${done.id}"]')`);
  assert.ok((await root.invoke('get_agents')).some((t:any)=>t.id===thinking.id && t.status==='running'));
  checks.push('unread intersection and explicit viewed action preserve other running tasks');
  for(let i=0;i<24;i++)event('scroll-'+i,'turn.started',{title:'示例长列表 '+i+'：标题不会隐藏其他任务，也不会截断实际采集结果',project:'Scroll Demo'});
  await root.wait(`document.querySelectorAll('.task-row').length===27`);
  const layout=await root.evaluate(`(()=>{const e=document.querySelector('.task-scroll');return {scroll:e.scrollHeight>e.clientHeight,height:document.querySelector('.dashboard').getBoundingClientRect().height,horizontal:document.documentElement.scrollWidth>innerWidth};})()`);
  assert.equal(layout.scroll,true);assert.ok(layout.height<=620);assert.equal(layout.horizontal,false);
  for(const [width,height] of [[412,620],[371,558],[371,360],[380,640],[320,480],[440,720]])await checkScales(root,'dashboard',width,height);
  await root.send('Emulation.clearDeviceMetricsOverride');
  await root.wait(`innerWidth===412`);
  const nativeBounds=await root.invoke('plugin:window|outer_size',{label:'popup'}),nativePosition=await root.invoke('plugin:window|outer_position',{label:'popup'}),monitor=await root.invoke('plugin:window|current_monitor',{label:'popup'});
  assert.deepEqual(nativeBounds,await root.invoke('plugin:window|inner_size',{label:'popup'}));
  assert.ok(nativePosition.x>=monitor.workArea.position.x&&nativePosition.y>=monitor.workArea.position.y&&nativePosition.x+nativeBounds.width<=monitor.workArea.position.x+monitor.workArea.size.width&&nativePosition.y+nativeBounds.height<=monitor.workArea.position.y+monitor.workArea.size.height);
  await click(root,'.quota summary');await root.wait(`document.querySelector('.quota').open`);
  const quotaBounds=await root.evaluate(`(()=>{const r=document.querySelector('.quota p').getBoundingClientRect();return {top:r.top,bottom:r.bottom,right:r.right,left:r.left,height:innerHeight};})()`);
  assert.ok(quotaBounds.top>=0&&quotaBounds.bottom<=quotaBounds.height&&quotaBounds.left>=0&&quotaBounds.right<=412);
  await Bun.write(join(owned,'quota-expanded-412x620.png'),Buffer.from((await root.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
  await click(root,'.quota summary');
  await root.invoke('toggle_pin');assert.equal(await root.invoke('get_pinned'),false);
  await root.invoke('plugin:window|hide',{label:'popup'});await root.invoke('plugin:window|show',{label:'popup'});
  await root.invoke('plugin:window|set_focus',{label:'popup'});await root.invoke('toggle_pin');
  assert.equal(await root.invoke('get_pinned'),true);assert.deepEqual(await root.invoke('plugin:window|outer_size',{label:'popup'}),nativeBounds);
  checks.push('pin/grow-only refresh, unpin/hide/reopen; real native work-area bounds/client equality; quota expansion within panel');
  checks.push('27 native tasks remain rendered with bounded vertical scrolling and no horizontal overflow');
  await root.invoke('open_management');management=await page('/manage');
  await management.wait(`!!document.querySelector('.setup-flow')`);
  // Settings render before asynchronous discovery selects the source/editor.
  // Wait for the actual controls; keep the folded-state/capability assertions.
  await management.wait(`!!document.querySelector('.integration-advanced') && !!document.querySelector('.hook-registry') && !!document.querySelector('.capability-summary')`);
  assert.equal(await management.evaluate(`document.querySelector('.integration-advanced').open`),false);
  assert.equal(await management.evaluate(`document.querySelector('.hook-registry').open`),false);
  assert.ok(await management.evaluate(`document.querySelector('.capability-summary').textContent.includes('仍需真实宿主验收')`));
  await input(management,'[aria-label="来源显示名称"]','标准事件 · 公开合成示例');
  await click(management,'header .primary');
  await management.wait(`document.querySelector('header .primary').disabled`);
  await management.wait(`[...document.querySelectorAll('.source-tests button')].some(b=>b.textContent==='测试读取来源'&&!b.disabled)`);
  await management.evaluate(`[...document.querySelectorAll('button')].find(b=>b.textContent==='测试读取来源').click()`);
  await management.wait(`document.querySelector('.read-result')?.textContent.includes('本次读取正常')`);
  await management.wait(`document.querySelector('.setup-flow [aria-current="step"]')?.textContent.includes('等待真实任务事件')`);
  const count=(await root.invoke('get_agents')).length;
  await management.evaluate(`[...document.querySelectorAll('button')].find(b=>b.textContent==='测试事件接收（不创建任务）').click()`);
  await management.wait(`document.body.textContent.includes('尚未验证真实 Agent')`);
  assert.equal((await root.invoke('get_agents')).length,count);
  await management.evaluate(`[...document.querySelectorAll('button')].find(b=>b.textContent==='生成本机 MCP 配置').click()`);
  await management.wait(`!!document.querySelector('[aria-label="MCP 接入配置"]')`);
  const mcpConfig=await management.evaluate(`JSON.parse(document.querySelector('[aria-label="MCP 接入配置"]').textContent)`);
  assert.equal(resolve(mcpConfig.mcpServers.taskpulse.command),resolve(exe));
  assert.deepEqual(mcpConfig.mcpServers.taskpulse.args,['--integration-mcp','--source',source.id]);
  assert.equal(resolve(mcpConfig.mcpServers.taskpulse.env.TASKPULSE_HOME),resolve(home));
  assert.equal((await root.invoke('get_agents')).length,count,'Export must not create a task');
  checks.push('native MCP configuration export binds the saved source and isolated profile without host writes or fake tasks');
  await checkScales(management,'sources',1040,900);
  await management.send('Emulation.setDeviceMetricsOverride',{width:640,height:600,deviceScaleFactor:2,mobile:false});
  await Bun.sleep(150);
  assert.ok(await management.evaluate(`document.documentElement.scrollWidth<=innerWidth&&document.querySelector('main').scrollHeight>document.querySelector('main').clientHeight`),'Compact source manager must scroll without horizontal overflow');
  checks.push('compact logical windows at DPR 2 retain usable Dashboard and source manager');
  await screenshot(management,'sources-current',1040,900);
  checks.push('native source workflow/save/read test/event channel test; advanced Hook controls folded and no lifecycle overclaim');
  // Separate test notice is real renderer delivery, never a monitored Agent completion.
  const config=await root.invoke('get_monitoring_settings');config.notifications=true;await root.invoke('save_monitoring_settings',{settings:config});
  await root.invoke('test_notification');notice=await page('/notice');
  await notice.wait(`document.body.innerText.includes('测试')`);
  await checkScales(notice,'notice',320,132);
  await root.invoke('dismiss_task_notice',{id:(await root.invoke('get_task_notice')).current.id});
  await root.invoke('test_notification',{kind:'long_title'});
  await notice.wait(`document.querySelector('.heading')?.textContent.includes('长标题排版验收（测试）')`);
  await checkScales(notice,'notice',300,124);
  const compactNoticeId=(await root.invoke('get_task_notice')).current.id;
  const bounds=await notice.evaluate(`(()=>{const r=document.querySelector('.message').getBoundingClientRect();return {x:r.x+30,y:r.y+30};})()`);
  await notice.send('Input.dispatchMouseEvent',{type:'mousePressed',...bounds,button:'left',clickCount:1});
  await notice.send('Input.dispatchMouseEvent',{type:'mouseReleased',...bounds,button:'left',clickCount:1});
  await root.wait(`__TAURI_INTERNALS__.invoke('get_task_notice').then(s=>s.current?.id!==${compactNoticeId})`);
  await root.invoke('test_notification');
  await notice.wait(`document.querySelector('.heading')?.textContent.includes('通知测试成功')`);
  await screenshot(notice,'notice-current',320,132);
  await root.wait(`__TAURI_INTERNALS__.invoke('get_notification_log').then(r=>r.some(r=>r.test&&r.stage==='rendered'))`);
  assert.equal((await root.invoke('get_agents')).length,count);
  checks.push('native test-notice delivery and compact 300x124 long-title layout/pointer click without task import');
  // Drive the actual MCP entrypoint, then observe its events through the GUI's
  // real provider/store/attention pipeline. All input belongs to this test home.
  const mcpServer=mcpConfig.mcpServers.taskpulse;
  const reportMcp=(kind:string,eventId:string,extra:Record<string,unknown>={})=>{
    const messages=[
      {jsonrpc:'2.0',id:1,method:'initialize',params:{protocolVersion:'2025-11-25',capabilities:{},clientInfo:{name:'native-public-fixture',version:'1'}}},
      {jsonrpc:'2.0',method:'notifications/initialized'},
      {jsonrpc:'2.0',id:2,method:'tools/call',params:{name:'taskpulse_report',arguments:{session_id:'mcp-native-public',turn_id:'mcp-turn',event_id:eventId,kind,...extra}}}
    ];
    const child=spawnSync(mcpServer.command,mcpServer.args,{input:messages.map(m=>JSON.stringify(m)+'\n').join(''),encoding:'utf8',env:{...env,...mcpServer.env,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:''},windowsHide:true,timeout:10000});
    if(child.error)throw child.error;assert.equal(child.status,0,child.stderr);
    const replies=child.stdout.trim().split('\n').map(line=>JSON.parse(line));
    assert.equal(replies.length,2);assert.equal(replies[1].result.isError,false,child.stdout);
  };
  const mcpTask=`__TAURI_INTERNALS__.invoke('get_agents').then(t=>t.find(t=>t.session_id==='mcp-native-public'))`;
  reportMcp('turn.started','mcp-start',{payload:{title:'公开合成 MCP 任务',project:'MCP Demo'}});
  await root.wait(`${mcpTask}.then(t=>t?.status==='running')`);
  const mcpId=(await root.evaluate(mcpTask)).id;
  reportMcp('input.requested','mcp-a',{request_id:'request-a',payload:{request_kind:'approval'}});
  reportMcp('input.requested','mcp-b',{request_id:'request-b',payload:{request_kind:'question'}});
  reportMcp('input.resolved','mcp-resolve-a',{request_id:'request-a'});
  reportMcp('phase.changed','mcp-thinking',{payload:{phase:'thinking'}});
  await root.wait(`${mcpTask}.then(t=>t?.status==='waiting_input'&&t.input_request_id==='request-b')`);
  reportMcp('input.resolved','mcp-resolve-b',{request_id:'request-b'});
  await root.wait(`${mcpTask}.then(t=>t?.status==='running')`);
  reportMcp('turn.completed','mcp-end');
  await root.wait(`${mcpTask}.then(t=>t?.waiting_reason==='turn_ended'&&t.unread)`);
  // Viewed terminal tasks leave attention unless explicitly pinned.
  await root.invoke('update_task_attention',{id:mcpId,action:'pin',pinned:true});
  await root.invoke('update_task_attention',{id:mcpId,action:'viewed'});
  reportMcp('turn.completed','mcp-end');
  reportMcp('session.renamed','mcp-rename',{payload:{title:'公开合成 MCP 任务 · 已改名'}});
  await root.wait(`${mcpTask}.then(t=>t?.name==='公开合成 MCP 任务 · 已改名')`);
  const observedMcp=await root.evaluate(mcpTask);
  assert.equal(observedMcp.id,mcpId);assert.equal(observedMcp.unread,false);
  assert.equal(observedMcp.evidence_channel,'self_reported');
  checks.push('real MCP events reach native task UI: stable identity, two explicit requests, matching resolution, thinking remains blocked, explicit end, rename and duplicate after viewed');
  assert.equal(errors.length,0,JSON.stringify(errors));
  if(process.argv.includes('--capture')){
    await mkdir('assets/screenshots',{recursive:true});
    for(const name of ['dashboard-current','sources-current','notice-current'])await copyFile(join(owned,name+'.png'),join('assets/screenshots',name+'.png'));
  }
  const report={passed:true,checks,layout,scaleChecks,scalingMethod:'WebView2 CDP deviceScaleFactor 1/1.5/2 with fixed logical windows; not OS mixed-monitor DPI acceptance',screenshots:process.argv.includes('--capture'),renderer:'Windows native Tauri WebView2',inputs:'public synthetic standard events, not real Agent-host support',realHostsVerified:[],realHostConfigurationsChanged:false,notVerified:['external editor focus on locked desktop','real Agent-host lifecycle','installer acceptance','OS display-scale changes and mixed-DPI monitor migration'],evidence:evidenceDir};
  await Bun.write(join(owned,'result.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report));
}catch(error){const ui=await management?.evaluate(`({read:document.querySelector('.read-result')?.textContent,buttons:[...document.querySelectorAll('.source-tests button')].map(b=>({text:b.textContent,disabled:b.disabled})),errors:[...document.querySelectorAll('[role=alert]')].map(e=>e.textContent)})`).catch(()=>null);const providers=await root?.invoke('get_providers').catch(()=>null);await Bun.write(join(owned,'failure.json'),JSON.stringify({passed:false,error:String(error),stderr,errors,ui,providers},null,2));console.error('Evidence: '+evidenceDir);throw error;}
});
