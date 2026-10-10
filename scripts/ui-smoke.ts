/** Read-only UI smoke via an isolated Edge CDP browser and mocked IPC.
 * Start Vite on 5189 and Edge headless with --remote-debugging-port=9227.
 * Run `bun scripts/ui-smoke.ts`. Never changes real TaskPulse/Agent data.
 */
import { mkdir } from 'node:fs/promises';
const out = 'src-tauri/target/ui-smoke';
await mkdir(out,{recursive:true});
const target = await fetch('http://127.0.0.1:9227/json/new?about:blank',{method:'PUT'}).then(r=>r.json()) as {webSocketDebuggerUrl:string};
if(!target) throw Error('No CDP page');
const socket=new WebSocket(target.webSocketDebuggerUrl);
await new Promise<void>((ok,fail)=>{socket.onopen=()=>ok();socket.onerror=()=>fail(Error('CDP connection failed'));});
let seq=0;const pending=new Map<number,{ok:(v:any)=>void;fail:(e:unknown)=>void}>();
socket.onmessage=e=>{const msg=JSON.parse(String(e.data));if(msg.id){const p=pending.get(msg.id);pending.delete(msg.id);msg.error?p?.fail(msg.error):p?.ok(msg.result);}};
const send=(method:string,params:Record<string,unknown>={})=>new Promise<any>((ok,fail)=>{const id=++seq;pending.set(id,{ok,fail});socket.send(JSON.stringify({id,method,params}));setTimeout(()=>{if(pending.delete(id))fail(Error('CDP timeout '+method));},30000);});
const evaluate=async(expression:string)=>{const result=await send('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true});if(result.exceptionDetails)throw Error(JSON.stringify(result.exceptionDetails));return result.result.value;};
const waitFor=async(expression:string)=>{for(let n=0;n<80;n++){if(await evaluate(expression))return;await Bun.sleep(100);}throw Error('UI assertion timeout: '+expression);};
const fixture=String.raw`
(()=>{
const now=Date.now();
let settings={version:2,onboarding_complete:true,notifications:true,sound:false,default_view:'tasks',sources:[{id:'codex',name:'Codex · 本机',kind:'codex',enabled:true,location:'',projects:['*'],excluded_projects:[],tmux_sessions:[],tmux_panes:[],poll_interval_ms:3000},{id:'deepseek_harness_desktop',name:'DeepSeek · Windows',kind:'deepseek_desktop',enabled:true,location:'',projects:['*'],excluded_projects:[],tmux_sessions:[],tmux_panes:[],poll_interval_ms:3000}]};
let tasks=Array.from({length:28},(_,i)=>({id:'test-'+i,name:i===0?'验收：授权阻塞':i===1?'验收：已结束':'验收任务 '+i,source:'codex',source_instance:'codex',project:'test-project',project_path:'D:/test/project',status:i<2?'waiting_input':'running',waiting_reason:i===0?'approval':i===1?'turn_ended':null,updated_at:now,started_at:now-60000,confidence:'hook',remote:false,unread:true,activity:'cargo test --bin agent-tray'}));
globalThis.isTauri=true;
const callbacks=new Map();const listeners=[];let cb=0;globalThis.__smokeCalls=[];
globalThis.__TAURI_INTERNALS__={metadata:{currentWindow:{label:location.pathname.startsWith('/manage')?'management':'popup'},currentWebview:{label:'popup'}},transformCallback(fn){callbacks.set(++cb,fn);return cb;},unregisterCallback(id){callbacks.delete(id);},async invoke(cmd,args={}){globalThis.__smokeCalls.push({cmd,args});
if(cmd==='get_agents')return tasks;if(cmd==='get_archived_attention')return [];if(cmd==='request_refresh')return Date.now();if(cmd==='get_providers')return settings.sources.filter(s=>s.enabled).map(s=>({provider:s.id,status:'connected',last_success:Date.now(),last_attempt:Date.now()}));if(cmd==='get_monitoring_settings')return settings;if(cmd==='get_selected_task'||cmd==='get_notification_error')return null;if(cmd==='get_codex_quota')return {quota:null,observed_at:null,stale:true,error:null,refreshing:false};if(cmd==='discover_sources')return settings.sources.map(s=>({id:s.id,available:true,detail:'Read-only test fixture'}));
if(cmd==='save_monitoring_settings'){settings=structuredClone(args.settings);return settings;}if(cmd==='list_discovered_sessions')return {total:1,items:[{task:tasks[1],reason:'历史会话，尚未加入关注'}]};if(cmd==='update_task_attention'){if(args.action==='viewed'||args.action==='hide'||args.action==='ignore')tasks=tasks.filter(t=>t.id!==args.id);return tasks;}if(cmd==='open_task')return 'details';
if(cmd==='plugin:event|listen'){listeners.push({id:listeners.length+1,...args});return listeners.length;}return null;}};
globalThis.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener(){}};
globalThis.__smokeEmit=(event,payload)=>listeners.filter(l=>l.event===event).forEach(l=>callbacks.get(l.handler)?.({event,id:l.id,payload}));
})();`;
try {
  await send('Page.enable');
  await send('Page.addScriptToEvaluateOnNewDocument',{source:fixture});
  await send('Emulation.setDeviceMetricsOverride',{width:440,height:900,deviceScaleFactor:1,mobile:false,screenWidth:1440,screenHeight:900});
  await send('Page.navigate',{url:'http://127.0.0.1:5189/'});
  await waitFor(`document.querySelectorAll('.task-row').length===28`);
  const layout=await evaluate(`(()=>{const p=document.querySelector('.dashboard'),m=document.querySelector('main');return {width:p.getBoundingClientRect().width,height:p.getBoundingClientRect().height,scroll:m.scrollHeight>m.clientHeight,overflow:document.documentElement.scrollWidth>440,errors:[...document.querySelectorAll('[role=alert]')].map(n=>n.textContent),rows:document.querySelectorAll('.task-row').length};})()`);
  if(layout.width>440||layout.height>720||!layout.scroll||layout.overflow||layout.errors.length)throw Error(JSON.stringify(layout));
  await evaluate(`[...document.querySelectorAll('.task-row')].find(row=>row.textContent.includes('验收：已结束')).click()`);
  await waitFor(`document.querySelector('.detail')?.textContent.includes('本轮结束')`);
  await evaluate(`[...document.querySelectorAll('.detail button')].find(b=>b.textContent==='已查看').click()`);
  await waitFor(`document.querySelectorAll('.task-row').length===27`);
  await evaluate(`document.querySelector('[aria-label="管理监控来源"]').click()`);
  await waitFor(`__smokeCalls.some(c=>c.cmd==='open_management')`);
  await Bun.write(out+'/dashboard.png',Buffer.from((await send('Page.captureScreenshot',{format:'png'})).data,'base64'));
  await send('Emulation.setDeviceMetricsOverride',{width:900,height:640,deviceScaleFactor:1,mobile:false});
  await send('Page.navigate',{url:'http://127.0.0.1:5189/manage/'});
  await waitFor(`document.querySelectorAll('.source-item').length===2`);
  await evaluate(`document.querySelector('.source-item input').click()`);
  await waitFor(`document.body.textContent.includes('开关即时生效')`);
  await evaluate(`document.querySelector('.editor input').value='Edited name';document.querySelector('.editor input').dispatchEvent(new Event('input',{bubbles:true}));`);
  await evaluate(`document.querySelector('header .primary').click()`);
  await waitFor(`document.body.textContent.includes('已保存并即时生效')`);
  await evaluate(`[...document.querySelectorAll('nav button')].find(b=>b.textContent==='已发现会话').click()`);
  await waitFor(`document.querySelector('.session')?.textContent.includes('验收：已结束')`);
  await evaluate(`[...document.querySelectorAll('nav button')].find(b=>b.textContent==='通知与显示').click()`);
  await waitFor(`document.body.textContent.includes('同一事件只提醒一次')`);
  await Bun.write(out+'/management.png',Buffer.from((await send('Page.captureScreenshot',{format:'png'})).data,'base64'));
  const result={passed:true,layout,checks:['28 tasks, scroll, bounded popup','ended task viewed action','management open IPC','source toggle + save','discovered sessions','notification preferences'],data:'isolated mocked IPC; not native live acceptance'};
  await Bun.write(out+'/result.json',JSON.stringify(result,null,2));
  console.log(JSON.stringify(result));
}finally{socket.close();}
