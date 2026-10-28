/** Actual Windows provider -> IPC -> WebView2 acceptance with isolated, labelled
 * Claude transcripts. No real agent settings, hooks or TaskPulse state changes.
 * bun scripts/native-model-smoke.ts --exe <embedded-frontend-executable>
 */
import assert from 'node:assert/strict';
import {mkdir,copyFile,appendFile} from 'node:fs/promises';
import {resolve,join,dirname} from 'node:path';
import {spawn,type ChildProcess} from 'node:child_process';
import {withSmokeWorkspace,stopOwnedChild} from './smoke-workspace';
const arg=process.argv.indexOf('--exe');if(arg<0)throw Error('--exe required');
const original=resolve(process.argv[arg+1]);
await withSmokeWorkspace('model-native',async({owned,evidenceDir,onStop})=>{
  let app:ChildProcess|undefined,ws:WebSocket|undefined;
  onStop(async()=>{ws?.close();await stopOwnedChild(app);});
  const home=join(owned,'profile'),claude=join(owned,'claude'),project=join(claude,'projects','example');
  await mkdir(home);await mkdir(project,{recursive:true});
  const exe=join(owned,'agent-tray.exe');await copyFile(original,exe);
  if(await Bun.file(join(dirname(original),'WebView2Loader.dll')).exists())await copyFile(join(dirname(original),'WebView2Loader.dll'),join(owned,'WebView2Loader.dll'));
  await Bun.write(join(home,'config.toml'),`[dashboard]\nversion=2\nonboarding_complete=true\nnotifications=false\nsound=false\n[[dashboard.sources]]\nid="claude_code"\nname="Claude Code · isolated model test"\nkind="claude_code"\nenabled=true\nlocation=""\nprojects=["*"]\nexcluded_projects=[]\ntmux_sessions=[]\ntmux_panes=[]\npoll_interval_ms=1000\n`);
  const model='custom-provider/very-long-model-name-for-narrow-window-2026-10-28';
  for(const [id,name] of [['known','验收示例：模型信息'],['unknown','验收示例：来源未提供模型']]) {
    const rows:any[]=[{type:'user',uuid:'turn-'+id,timestamp:new Date().toISOString(),cwd:'D:/Public Example',message:{content:name}}];
    if(id==='known')rows.push({type:'assistant',timestamp:new Date().toISOString(),message:{model,content:[{type:'thinking',thinking:'Synthetic test fixture'}]}});
    await Bun.write(join(project,id+'.jsonl'),rows.map(r=>JSON.stringify(r)).join('\n')+'\n');
  }
  const port=9346;
  const tabs=()=>fetch(`http://127.0.0.1:${port}/json`).then(r=>r.json()) as Promise<{url:string;webSocketDebuggerUrl:string}[]>;
  assert.equal(await tabs().then(()=>true).catch(()=>false),false,'CDP port already in use');
  app=spawn(exe,['--show'],{env:{...process.env,TASKPULSE_HOME:home,CLAUDE_CONFIG_DIR:claude,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port}`},windowsHide:true,stdio:['ignore','ignore','pipe']});
  let failure:Error|undefined,stderr='';app.once('error',e=>failure=e);app.stderr?.on('data',d=>{stderr=(stderr+String(d)).slice(-4000);});
  let tab:any;
  for(let n=0;n<200;n++){if(failure)throw failure;tab=(await tabs().catch(()=>[])).find(t=>t.url.includes('tauri.localhost')&&!/\/(manage|notice)/.test(t.url));if(tab)break;if(app.exitCode!==null)throw Error('App exited: '+stderr);await Bun.sleep(100);}
  assert.ok(tab,'Native page missing: '+stderr);
  ws=new WebSocket(tab.webSocketDebuggerUrl);await new Promise<void>((ok,fail)=>{ws!.onopen=()=>ok();ws!.onerror=()=>fail(Error('CDP connection failed'));});
  let seq=0;const pending=new Map<number,{ok:(v:any)=>void,fail:(e:any)=>void,timer:ReturnType<typeof setTimeout>}>();
  const errors:string[]=[];
  ws.onmessage=e=>{const m=JSON.parse(String(e.data));if(m.method==='Runtime.exceptionThrown')errors.push(JSON.stringify(m.params.exceptionDetails));const p=pending.get(m.id);if(!p)return;clearTimeout(p.timer);pending.delete(m.id);m.error?p.fail(Error(JSON.stringify(m.error))):p.ok(m.result);};
  const send=(method:string,params:Record<string,unknown>={})=>new Promise<any>((ok,fail)=>{const id=++seq;const timer=setTimeout(()=>{pending.delete(id);fail(Error('CDP timeout '+method));},15000);pending.set(id,{ok,fail,timer});ws!.send(JSON.stringify({id,method,params}));});
  const evaluate=async(expression:string)=>{const r=await send('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true,userGesture:true});if(r.exceptionDetails)throw Error(JSON.stringify(r.exceptionDetails));return r.result.value;};
  const invoke=(cmd:string)=>evaluate(`__TAURI_INTERNALS__.invoke(${JSON.stringify(cmd)})`);
  const wait=async(expression:string)=>{for(let n=0;n<150;n++){if(await evaluate(expression))return;await Bun.sleep(100);}throw Error('Native assertion timeout '+expression);};
  await send('Runtime.enable');
  await wait(`document.querySelectorAll('.task-row').length===2`);
  const before=(await invoke('get_agents')).find((t:any)=>t.model===model);assert.ok(before);
  assert.equal(before.source,'claude_code');
  await wait(`document.querySelector('.model')?.textContent.includes(${JSON.stringify(model)})`);
  const layouts=[];
  for(const [width,height] of [[440,720],[320,420],[440,420],[320,720]]) {
    await send('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:1,mobile:false});await Bun.sleep(150);
    const layout=await evaluate(`(()=>{const m=document.querySelector('.model'),b=m.getBoundingClientRect();return {width:innerWidth,height:innerHeight,overflow:document.documentElement.scrollWidth>innerWidth,modelClipped:m.scrollWidth>m.clientWidth,right:b.right,bottom:b.bottom};})()`);
    assert.equal(layout.overflow,false);assert.equal(layout.modelClipped,false);assert.ok(layout.right<=width);layouts.push(layout);
    await Bun.write(join(owned,`model-${width}x${height}.png`),Buffer.from((await send('Page.captureScreenshot',{format:'png'})).data,'base64'));
  }
  await send('Emulation.setDeviceMetricsOverride',{width:440,height:720,deviceScaleFactor:1,mobile:false});
  await evaluate(`document.querySelector('.model').closest('button').click()`);
  await wait(`document.querySelector('.model-detail')?.textContent.includes(${JSON.stringify(model)})`);
  await appendFile(join(project,'known.jsonl'),JSON.stringify({type:'assistant',timestamp:new Date().toISOString(),message:{model:'deepseek-v4-flash',content:[]}})+'\n');
  await wait(`document.querySelector('.model-detail')?.textContent.includes('deepseek-v4-flash')`);
  const after=(await invoke('get_agents')).find((t:any)=>t.id===before.id);
  for(const key of ['id','source','turn_key','started_at','status','unread'])assert.equal(after[key],before[key],key+' changed on model update');
  await evaluate(`document.querySelector('[aria-label="关闭详情"]').click();const q=document.querySelector('[aria-label="搜索任务"]');q.value='deepseek';q.dispatchEvent(new Event('input',{bubbles:true}));`);
  await wait(`document.querySelectorAll('.task-row').length===1 && document.querySelector('.model')?.textContent.includes('deepseek')`);
  await evaluate(`document.querySelector('[aria-label="清除搜索"]').click()`);await wait(`document.querySelectorAll('.task-row').length===2`);
  await evaluate(`[...document.querySelectorAll('.task-row')].find(e=>e.textContent.includes('来源未提供模型')).click()`);
  await wait(`document.querySelector('.model-detail')?.textContent.includes('来源未提供')`);
  assert.deepEqual(errors,[]);
  await Bun.write(join(owned,'result.json'),JSON.stringify({passed:true,checks:['real Claude parser + native IPC + WebView2','reported model visible','missing model explicit in details','long names at normal/minimum width and height','model switch preserves lifecycle and start time','model search'],layouts,data:'isolated synthetic transcripts, not host certification'},null,2));
  console.log(JSON.stringify({passed:true,evidenceDir,layouts}));
});
