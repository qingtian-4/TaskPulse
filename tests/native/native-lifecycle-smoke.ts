/** Read-only real IPC acceptance. No fake tasks, Agent prompts or config saves.
 * Run against a temporary localhost WebView2 CDP instance on port 9337.
 * Samples actual native turns and notices for 30 seconds, validates phase/UI
 * consistency, and records only minimal metadata (never reasoning or commands).
 */
const tabs = await fetch('http://127.0.0.1:9337/json').then(r => r.json()) as Array<{url:string;webSocketDebuggerUrl:string}>;
const tab = tabs.find(t => t.url.includes('tauri.localhost') && !t.url.includes('/notice') && !t.url.includes('/manage'));
if (!tab) throw Error('No native TaskPulse popup');
const ws = new WebSocket(tab.webSocketDebuggerUrl);
await new Promise<void>((ok,fail)=>{ws.onopen=()=>ok();ws.onerror=()=>fail(Error('CDP connection failed'));});
let serial = 0;
const pending = new Map<number,{ok:(value:any)=>void;fail:(error:Error)=>void;timer:ReturnType<typeof setTimeout>}>();
ws.onmessage=e=>{const r=JSON.parse(String(e.data));if(!r.id)return;const p=pending.get(r.id);if(!p)return;pending.delete(r.id);clearTimeout(p.timer);r.error?p.fail(Error(JSON.stringify(r.error))):p.ok(r.result);};
const evaluate=(expression:string)=>new Promise<any>((ok,fail)=>{
  const id=++serial;
  const timer=setTimeout(()=>{pending.delete(id);fail(Error('CDP timeout'));},10000);
  pending.set(id,{ok,fail,timer});
  ws.send(JSON.stringify({id,method:'Runtime.evaluate',params:{expression,awaitPromise:true,returnByValue:true}}));
}).then(r=>{if(r.exceptionDetails)throw Error(JSON.stringify(r.exceptionDetails));return r.result.value;});
const invoke=(command:string)=>evaluate(`__TAURI_INTERNALS__.invoke(${JSON.stringify(command)})`);
const observed=new Map<string,Set<string>>();
let samples=0, running=0, rendered=0;
try {
  for(let n=0;n<60;n++){
    const tasks=await invoke('get_agents');
    const notice=await invoke('get_task_notice');
    if(tasks.some((t:any)=>Object.hasOwn(t,'approval_hint')))throw Error('Removed approval hint leaked into actual task snapshots');
    if(notice.current && !notice.current.test && notice.current.title.includes('授权状态待确认'))throw Error('A real task received a removed inferred approval reminder');
    if(notice.current && !notice.current.test && notice.current.context?.source!=='remote_tmux' && notice.current.body.includes('任务可能已结束'))throw Error('Native Agent received an inferred finish notice');
    for(const t of tasks.filter((t:any)=>t.status==='running' && t.session_id && !t.stale)){
      running++;
      const key=observed.get(t.id)??new Set<string>();key.add(t.phase??'unavailable');observed.set(t.id,key);
      if(['thinking','responding','awaiting_model'].includes(t.phase) && t.activity)throw Error('Ended command retained as current activity: '+t.id);
      if(t.phase==='tool' && !t.activity)throw Error('Tool phase lost current activity: '+t.id);
      const expected=({thinking:'正在思考',responding:'正在生成回复',awaiting_model:'等待模型响应'} as Record<string,string>)[t.phase];
      if(expected){
        const row=await evaluate(`Array.from(document.querySelectorAll('.task-row')).find(r=>r.querySelector('.row-title')?.getAttribute('title')===${JSON.stringify(t.name)})?.textContent`);
        // IPC and UI can race at phase boundaries; retry the next sample.
        if(row?.includes(expected))rendered++;
      }
    }
    samples++;await Bun.sleep(500);
  }
  if(!running)throw Error('No fresh native running turn; acceptance requires a real active Agent');
  if(!rendered)throw Error('No model/thinking phase rendered in actual task rows');
  const current=await invoke('get_agents');
  const archive=await invoke('get_archived_attention');
  if(archive.some((t:any)=>current.some((v:any)=>v.id===t.id) || t.status==='running'))throw Error('Archive contains current/live attention');
  const result={passed:true,samples,runningSamples:running,phaseRenderedSamples:rendered,phases:Object.fromEntries([...observed].map(([id,p])=>[id,[...p]])),checks:['real native task phases sampled','no stale command in model/thinking phases','no inferred Agent completion notice'],notVerified:['new real Agent finish to popup latency','long wall-clock cloud thinking']};
  await Bun.write('src-tauri/target/ui-smoke/native-lifecycle-result.json',JSON.stringify(result,null,2));
  console.log(JSON.stringify(result));
} finally {ws.close();}
