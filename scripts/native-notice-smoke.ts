/** Real WebView2 alert acceptance, CDP 9337; no mocked IPC.
 * Temporarily selects silent popup reminders, restores config in finally.
 * Test alerts are explicit and never enter attention/history/Agent sessions.
 */
import { mkdir } from 'node:fs/promises';
const out='src-tauri/target/ui-smoke';
await mkdir(out,{recursive:true});
type Tab={url:string;webSocketDebuggerUrl:string};
const tabs=async():Promise<Tab[]>=>fetch('http://127.0.0.1:9337/json').then(r=>r.json());
async function connect(tab:Tab){
  const ws=new WebSocket(tab.webSocketDebuggerUrl);
  await new Promise<void>((ok,fail)=>{ws.onopen=()=>ok();ws.onerror=()=>fail(Error('CDP connection failed'));});
  let id=0;const pending=new Map<number,{ok:(v:any)=>void;fail:(e:any)=>void;timer:ReturnType<typeof setTimeout>}>();
  const errors:string[]=[];
  ws.onmessage=e=>{const m=JSON.parse(String(e.data));if(m.method==='Log.entryAdded' && m.params.entry.level==='error')errors.push(m.params.entry.text);if(!m.id)return;const p=pending.get(m.id);if(!p)return;clearTimeout(p.timer);pending.delete(m.id);m.error?p.fail(m.error):p.ok(m.result);};
  const send=(method:string,params:Record<string,unknown>={})=>new Promise<any>((ok,fail)=>{const n=++id;const timer=setTimeout(()=>{pending.delete(n);fail(Error('CDP timeout: '+method));},15000);pending.set(n,{ok,fail,timer});ws.send(JSON.stringify({id:n,method,params}));});
  const evaluate=async(expression:string)=>{const r=await send('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true,userGesture:true});if(r.exceptionDetails)throw Error(JSON.stringify(r.exceptionDetails));return r.result.value;};
  const invoke=(cmd:string,args:Record<string,unknown>={})=>evaluate(`__TAURI_INTERNALS__.invoke(${JSON.stringify(cmd)},${JSON.stringify(args)})`);
  const wait=async(expression:string,timeout=10000)=>{for(let n=0;n<timeout/100;n++){if(await evaluate(expression))return;await Bun.sleep(100);}throw Error('Assertion timeout: '+expression);};
  await send('Log.enable');
  return {send,evaluate,invoke,wait,errors,close:()=>ws.close()};
}
let popup:Tab|undefined;
for(let n=0;n<100;n++){popup=(await tabs().catch(()=>[])).find(t=>t.url.includes('tauri.localhost')&&!t.url.includes('/manage')&&!t.url.includes('/notice'));if(popup)break;await Bun.sleep(100);}
if(!popup)throw Error('Start real embedded TaskPulse with temporary CDP 9337');
const root=await connect(popup);let alert:Awaited<ReturnType<typeof connect>>|undefined;let original:any;
let phase='startup';
try{
  await root.wait(`!!globalThis.__TAURI_INTERNALS__ && !!document.querySelector('.dashboard')`);
  original=await root.invoke('get_monitoring_settings');
  const before=await root.invoke('get_agents');
  const existing=await root.invoke('get_task_notice');
  if(existing.current)throw Error('A real user reminder is pending; refusing to dismiss it for tests');
  await root.invoke('save_monitoring_settings',{settings:{...original,notifications:true,notification_mode:'popup',sound:false}});
  await root.invoke('plugin:window|show',{label:'popup'});
  await root.invoke('plugin:window|set_focus',{label:'popup'});
  await root.wait(`__TAURI_INTERNALS__.invoke('plugin:window|is_focused',{label:'popup'})`);
  await root.invoke('test_notification',{kind:'turn_ended'});
  let tab:Tab|undefined;
  for(let n=0;n<100;n++){tab=(await tabs()).find(t=>t.url.includes('/notice'));if(tab)break;await Bun.sleep(100);}
  if(!tab)throw Error('Actual alert window not created');
  alert=await connect(tab);
  await alert.wait(`document.querySelector('.summary')?.textContent.includes('本轮结束，等待后续输入')`);
  const first=await root.invoke('get_task_notice');
  if(!first.current?.test || first.current.task_id!==null)throw Error('Test created fake monitored task');
  const focusPreserved=await root.invoke('plugin:window|is_focused',{label:'popup'});
  if(!focusPreserved)throw Error('Alert stole dashboard focus');
  const layout=await alert.evaluate(`(()=>{const p=document.querySelector('.notice-window');return {width:p.getBoundingClientRect().width,height:p.getBoundingClientRect().height,background:getComputedStyle(p).backgroundColor,overflow:document.documentElement.scrollWidth>400||document.documentElement.scrollHeight>180,errors:[...document.querySelectorAll('[role=alert]')].map(e=>e.textContent)};})()`);
  if(layout.width>400.1||layout.height>180.1||layout.background!=='rgb(255, 255, 255)'||layout.overflow||layout.errors.length)throw Error(JSON.stringify(layout));
  const name=await alert.evaluate(`document.querySelector('.heading')?.textContent`);
  if(name!==first.current.context?.task_name)throw Error('Native task title not used as message heading');
  await alert.send('Input.dispatchMouseEvent',{type:'mouseMoved',x:50,y:90});
  await alert.wait(`document.querySelector('footer')?.textContent.includes('鼠标移开')`);
  const paused=await alert.evaluate(`document.querySelector('.progress').style.width`);
  await Bun.sleep(1200);
  if(await alert.evaluate(`document.querySelector('.progress').style.width`)!==paused)throw Error('Hover timer did not pause');
  await Bun.write(out+'/native-completion-notice.png',Buffer.from((await alert.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
  await root.invoke('test_notification',{kind:'failed'});
  await root.invoke('test_notification',{kind:'approval'});
  const queued=await root.invoke('get_task_notice');
  if(queued.pending!==2||queued.current.id!==first.current.id)throw Error('Separate alert queue failed');
  await alert.wait(`document.querySelector('.close')?.disabled===false`);
  await alert.evaluate(`document.querySelector('[aria-label="关闭提醒（保持未查看）"]').click()`);
  await alert.wait(`document.querySelector('.summary')?.textContent.includes('任务失败')`);
  const second=await root.invoke('get_task_notice');
  await root.invoke('dismiss_task_notice',{id:first.current.id});
  if((await root.invoke('get_task_notice')).current.id!==second.current.id)throw Error('Late old timer closed the next alert');
  await alert.wait(`document.querySelector('.later')?.disabled===false`);
  await Bun.sleep(200);
  await Bun.write(out+'/native-failure-notice.png',Buffer.from((await alert.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
  await alert.evaluate(`[...document.querySelectorAll('.actions button')].find(b=>b.textContent==='稍后处理').click()`);
  await alert.wait(`document.querySelector('.summary')?.textContent.includes('需要授权')`);
  await alert.wait(`document.querySelector('.primary')?.disabled===false`);
  await Bun.sleep(200); // Wait for entrance animation before visual capture.
  await Bun.write(out+'/native-approval-notice.png',Buffer.from((await alert.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
  phase='open approval details';
  await alert.evaluate(`document.querySelector('.primary').click()`);
  await root.wait(`__TAURI_INTERNALS__.invoke('get_task_notice').then(s=>!s.current)`);
  if(!await root.invoke('plugin:window|is_visible',{label:'popup'}))throw Error('View details did not show dashboard');
  if(await root.invoke('plugin:window|is_visible',{label:'task-notice'}))throw Error('Dismissed alert still visible');
  await root.invoke('test_notification');
  await alert.wait(`document.querySelector('.heading')?.textContent.includes('通知测试成功')`);
  await alert.send('Input.dispatchMouseEvent',{type:'mouseMoved',x:410,y:260});
  await alert.wait(`document.querySelector('footer')?.textContent.includes('秒后收起')`);
  const start=Date.now();
  phase='automatic timeout';
  await root.wait(`__TAURI_INTERNALS__.invoke('get_task_notice').then(s=>!s.current)`,20000);
  const autoDismissMs=Date.now()-start;
  if(autoDismissMs<14000)throw Error('Alert disappeared too early');
  await root.invoke('test_notification',{kind:'long_title'});
  await alert.wait(`document.querySelector('.heading')?.textContent.includes('长标题排版验收（测试）')`);
  await alert.wait(`document.querySelector('.message')?.disabled===false`);
  await alert.send('Input.dispatchMouseEvent',{type:'mouseMoved',x:100,y:80});
  await Bun.sleep(200);
  const longTitleLayout=await alert.evaluate(`(()=>{const title=document.querySelector('.heading');const summary=document.querySelector('.summary');const footer=document.querySelector('footer');const sender=document.querySelector('.sender');return {headingHeight:title.getBoundingClientRect().height,summaryBottom:summary.getBoundingClientRect().bottom,footerTop:footer.getBoundingClientRect().top,fullTitle:title.title,senderTitle:sender.title,overflow:document.documentElement.scrollWidth>400||document.documentElement.scrollHeight>180};})()`);
  if(longTitleLayout.headingHeight>42.1||longTitleLayout.summaryBottom>longTitleLayout.footerTop||longTitleLayout.overflow||!longTitleLayout.senderTitle.includes('Test-server'))throw Error('Long title layout regression: '+JSON.stringify(longTitleLayout));
  await Bun.write(out+'/native-long-title-notice.png',Buffer.from((await alert.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
  // Real pointer input exercises the message-card hit target, not just DOM.click().
  phase='message pointer click';
  await alert.send('Input.dispatchMouseEvent',{type:'mousePressed',x:100,y:80,button:'left',clickCount:1});
  await alert.send('Input.dispatchMouseEvent',{type:'mouseReleased',x:100,y:80,button:'left',clickCount:1});
  await root.wait(`__TAURI_INTERNALS__.invoke('get_task_notice').then(s=>!s.current)`);
  await root.invoke('test_notification');
  await root.invoke('save_monitoring_settings',{settings:{...original,notifications:false,notification_mode:'popup',sound:false}});
  if((await root.invoke('get_task_notice')).current)throw Error('Disabling reminders did not clear popup');
  const after=await root.invoke('get_agents');
  if(after.some((t:any)=>t.id==='notification-test'))throw Error('Test polluted attention');
  for(const old of before.filter((t:any)=>t.unread)){const next=after.find((t:any)=>t.id===old.id);if(next && !next.unread)throw Error('Alert dismissed unread task state');}
  if(root.errors.length||alert.errors.length)throw Error('Native console/CSP error: '+[...root.errors,...alert.errors].join(';'));
  const result={passed:true,layout,longTitleLayout,focusPreserved,autoDismissMs,checks:['actual completion/failure/approval windows rendered','native task title is primary heading','compact light card and long title layout','message card pointer click opens dashboard','separate queued alerts','late callback safe','hover pauses timer','view details opens dashboard','15s auto-hide','master off clears popup','no fake task or viewed-state mutations','no native console/CSP errors'],notVerified:['fresh real Agent task finish during acceptance','OS system banner delivery']};
  await Bun.write(out+'/native-notice-result.json',JSON.stringify(result,null,2));
  console.log(JSON.stringify(result));
}catch(error){
  console.error('Failed phase: '+phase);
  if(alert)console.error(JSON.stringify(await alert.evaluate(`(async()=>({footer:document.querySelector('footer')?.textContent,disabled:document.querySelector('.primary')?.disabled,progress:document.querySelector('.progress')?.style.width,state:await __TAURI_INTERNALS__.invoke('get_task_notice')}))()`).catch(e=>String(e))));
  await Bun.write(out+'/native-notice-result.json',JSON.stringify({passed:false,phase,error:String(error)},null,2));
  throw error;
}finally{
  try{
    for(let n=0;n<4;n++){const state=await root.invoke('get_task_notice').catch(()=>null);if(!state?.current?.test)break;await root.invoke('dismiss_task_notice',{id:state.current.id});}
    if(original)await root.invoke('save_monitoring_settings',{settings:original});
  }finally{alert?.close();root.close();}
}
