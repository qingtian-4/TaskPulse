/** Windows native replay against independently authored expected cases.
 * bun tests/native/native-request-replay.ts --exe PATH --expected-commit SHA
 * Runs only isolated producers; does not install hooks or answer any request. */
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {createReadStream} from 'node:fs';
import {appendFile,copyFile,mkdir} from 'node:fs/promises';
import {dirname,join,resolve} from 'node:path';
import {spawn,spawnSync,type ChildProcess} from 'node:child_process';
import {createServer} from 'node:net';
import {withSmokeWorkspace,stopOwnedChild} from '../support/smoke-workspace';
import {verifyDevelopmentBuild} from '../../scripts/development-artifact';
import {scoreRequests,type Path,type Target,type Truth} from '../../scripts/request-evaluation';

const option=(name:string)=>{const n=process.argv.indexOf(name);return n<0?undefined:process.argv[n+1]};
const original=resolve(option('--exe')??'');
const expectedCommit=option('--expected-commit');
assert.ok(expectedCommit&&/^[a-f0-9]{40}$/.test(expectedCommit),'Supply the full clean build SHA');
assert.equal(process.platform,'win32');
const version=(await Bun.file('package.json').json()).version;
const fixture=await Bun.file(new URL('../fixtures/request-replay.json',import.meta.url)).json() as {
  version:string;cases:{agent:'codex'|'claude';signal:'approval'|'question';path:Path;negative:string;positive:string;wrongResult:string;restore:boolean;returnTarget:Target;unsupported?:boolean}[]
};
async function sha(path:string){
  const hash=createHash('sha256');
  for await(const part of createReadStream(path))hash.update(part);
  return hash.digest('hex');
}
const hash=await sha(original);
const reports=[];
for(const spec of fixture.cases){
  reports.push(await withSmokeWorkspace(`request-replay-${spec.agent}-${spec.signal}`,async({owned,onStop})=>{
    const home=join(owned,'profile'),codex=join(owned,'codex'),claude=join(owned,'claude');
    await mkdir(join(home,'claude-events'),{recursive:true});await mkdir(join(codex,'sessions'),{recursive:true});await mkdir(join(codex,'taskpulse-events'));await mkdir(claude);
    const exe=join(owned,'agent-tray.exe');await copyFile(original,exe);
    const loader=join(dirname(original),'WebView2Loader.dll');
    if(await Bun.file(loader).exists())await copyFile(loader,join(owned,'WebView2Loader.dll'));
    assert.equal(await sha(exe),hash);
    const source=spec.agent==='codex'?'codex':'claude_code',session='public-fixture',turn='public-turn';
    const taskId=(spec.agent==='codex'?'codex:':'claude:')+session;
    await Bun.write(join(home,'config.toml'),`[dashboard]
version=2
onboarding_complete=true
auto_check_updates=false
auto_discover_local=false
notifications=true
sound=false
notification_mode="popup"
[[dashboard.sources]]
id="${source}"
kind="${source}"
name="Public replay"
enabled=true
poll_interval_ms=1000
`);
    const rollout=join(codex,'sessions','rollout-public.jsonl');
    const stamp=()=>new Date().toISOString();
    const line=(type:string,payload:unknown)=>JSON.stringify({type,timestamp:stamp(),payload})+'\n';
    if(spec.agent==='codex')await Bun.write(rollout,line('session_meta',{id:session,cwd:'D:/Public Fixture'})+line('event_msg',{type:'task_started',turn_id:turn}));
    const reserve=createServer();await new Promise<void>(ok=>reserve.listen(0,'127.0.0.1',ok));
    const port=(reserve.address() as {port:number}).port;
    await new Promise<void>((ok,fail)=>reserve.close(e=>e?fail(e):ok()));
    const env={...process.env,TASKPULSE_HOME:home,CODEX_HOME:codex,CLAUDE_CONFIG_DIR:claude,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port}`};
    let app:ChildProcess|undefined;
    const sockets=new Set<WebSocket>();
    const stop=async()=>{for(const ws of sockets)ws.close();sockets.clear();await stopOwnedChild(app);};
    onStop(stop);
    function start(){app=spawn(exe,['--show'],{env,windowsHide:true,stdio:'ignore'});}
    async function until<T>(message:string,action:()=>Promise<T|undefined>):Promise<T>{
      const deadline=Date.now()+16000;
      while(Date.now()<deadline){const result=await action();if(result!==undefined)return result;await Bun.sleep(80);}
      throw Error(message);
    }
    const tabs=()=>fetch(`http://127.0.0.1:${port}/json`).then(r=>r.json() as Promise<any[]>).catch(()=>[]);
    async function connect(notice=false){
      const url=await until('No WebView for replay',async()=>(await tabs()).find(t=>t.url?.includes('tauri.localhost')&&(notice?t.url.includes('/notice'):!t.url.includes('/notice')&&!t.url.includes('/manage')))?.webSocketDebuggerUrl as string|undefined);
      const ws=new WebSocket(url);sockets.add(ws);
      await new Promise<void>((ok,fail)=>{ws.onopen=()=>ok();ws.onerror=()=>fail(Error('CDP connection failed'));});
      let seq=0;const calls=new Map<number,(r:any)=>void>();
      ws.onmessage=e=>{const r=JSON.parse(String(e.data));calls.get(r.id)?.(r);};
      const evaluate=(expression:string)=>new Promise<any>((ok,fail)=>{
        const id=++seq;
        const timer=setTimeout(()=>{calls.delete(id);fail(Error('Replay CDP timed out'));},10000);
        calls.set(id,r=>{calls.delete(id);clearTimeout(timer);if(r.error||r.result.exceptionDetails)fail(Error(JSON.stringify(r.error??r.result.exceptionDetails)));else ok(r.result.result.value);});
        ws.send(JSON.stringify({id,method:'Runtime.evaluate',params:{expression,awaitPromise:true,returnByValue:true,userGesture:true}}));
      });
      await until('IPC not ready',async()=>await evaluate('!!globalThis.__TAURI_INTERNALS__?.invoke')||undefined);
      return {evaluate,invoke:(cmd:string,args={})=>evaluate(`__TAURI_INTERNALS__.invoke(${JSON.stringify(cmd)},${JSON.stringify(args)})`)};
    }
    function hook(name:string,fields:Record<string,unknown>={}){
      const argv=spec.agent==='claude'?[exe,'--claude-event']:[process.env.TASKPULSE_PYTHON??'python',resolve('scripts/codex-observer.py')];
      const result=spawnSync(argv[0],argv.slice(1),{env,input:JSON.stringify({session_id:session,turn_id:turn,hook_event_name:name,cwd:'D:/Public Fixture',...fields}),encoding:'utf8',windowsHide:true,timeout:10000});
      assert.equal(result.status,0,result.stderr);
      assert.equal(result.stdout,'','Observer must never issue a host decision');
    }
    start();let root=await connect();
    verifyDevelopmentBuild(await root.invoke('get_build_info'),expectedCommit,version);
    assert.equal((await root.invoke('integration_environment')).isolated,true);
    if(spec.agent==='claude')hook('UserPromptSubmit');
    const task=async()=>(await root.invoke('get_agents')).find((t:any)=>t.id===taskId);
    const pending=async()=>(await root.invoke('get_pending_task_prompts')).find((t:any)=>t.id===taskId);
    const running=async()=>{const t=await task();return t?.status==='running'&&!t.stale?t:undefined};
    await until('Initial task not running',running);
    const log=async()=>(await root.invoke('get_notification_log')).filter((r:any)=>r.channel==='popup'&&!r.test);
    const ids=async()=>new Set<number>((await log()).filter((r:any)=>r.stage==='queued').map((r:any)=>r.notice_id));
    const attempts=async()=>(await root.invoke('get_providers')).find((p:any)=>p.provider===source)?.last_attempt??0;
    const waitRead=async(at:number)=>until('Provider did not read fixture',async()=>(await attempts())>at?true:undefined);

    const tool={tool_name:'Bash',tool_input:{command:'public-fixture-command'}};
    const call=(name:string,id:string,args:unknown)=>line('response_item',{type:'function_call',name,call_id:id,arguments:JSON.stringify(args)});
    const result=(id:string)=>line('response_item',{type:'function_call_output',call_id:id,output:'ordinary-result'});
    const reply=(role:string)=>line('response_item',{type:'message',role,content:[{type:'input_text',text:`<send_user_message_question_reply>\n${JSON.stringify([{questionItemId:JSON.stringify(['request_user_input_async','ask',0]),answer:'fixture'}])}\n</send_user_message_question_reply>`}]});
    const beforeResources=await root.invoke('get_resource_diagnostics');
    const beforeNoise=await ids(),noiseAt=Date.now();
    if(spec.agent==='claude'){
      const noise=spec.signal==='question'?{tool_name:'AskUserQuestion',tool_input:{questions:[{question:'not a confirmed dialog'}]}}:tool;
      hook('PreToolUse',{...noise,tool_use_id:'noise'});hook('PostToolUse',{...noise,tool_use_id:'noise'});
    }else if(spec.signal==='approval')hook('PostToolUse',{...tool,tool_use_id:'unrelated'});
    else await appendFile(rollout,call('shell_command','noise',{command:'public-fixture-command'})+result('noise'));
    await waitRead(noiseAt);
    await Bun.sleep(1300);
    const noiseEnd=Date.now();
    const noiseAlerts=[...(await ids())].filter(id=>!beforeNoise.has(id)).map(()=>({}));
    assert.equal(await pending(),undefined);

    if(spec.unsupported){
      assert.equal(noiseAlerts.length,0,'Unsupported source must not synthesize a user request');
      const evidence={cohort:{agent:spec.agent,version:fixture.version,platform:process.platform,signal:spec.signal,lane:'fixed'},unsupported:true,
        negativePassed:true,positives:0,falseNegativeRate:null,buildCommit:expectedCommit,modified:false,executableSha256:hash};
      await Bun.write(join(owned,'result.json'),JSON.stringify(evidence,null,2));
      return evidence;
    }
    const beforeRequest=await ids();
    if(spec.signal==='approval')hook('PreToolUse',{...tool,tool_use_id:'approval-call'});
    // Timestamp before synchronous fixture submission. This upper-bounds
    // observable-to-render latency, including the fixture producer's write.
    const requestAt=Date.now();
    if(spec.signal==='approval')hook('PermissionRequest',tool);
    else await appendFile(rollout,call('request_user_input_async','ask',{questions:[{title:'Fixture',options:['Yes','No']}]})+
      line('response_item',{type:'function_call_output',call_id:'ask',output:[{type:'input_text',text:'{"accepted":true}'}]}));
    const request=await until('Explicit request not confirmed',async()=>{const t=await pending();return t?.waiting_reason===spec.signal&&!t.stale?t:undefined});
    assert.ok(request.input_request_id);
    const notice=await until('Request did not reach native notice',async()=>{const state=await root.invoke('get_task_notice');return state.current?.task_id===taskId?state.current:undefined});
    const rendered=await until('No frame-acknowledged render for this request',async()=>(await log()).find((r:any)=>r.notice_id===notice.id&&r.stage==='rendered'));
    const noticeUi=await connect(true);
    assert.equal(await noticeUi.invoke('open_task_notice',{id:notice.id,preferHost:true}),'details');
    const returned=await until('Fallback details did not select the request task',async()=>{const t=await root.invoke('get_selected_task');return t?.id===taskId?t:undefined});
    assert.equal(returned.unread,request.unread);

    await stop();
    await until('Previous WebView process still active',async()=>(await tabs()).length===0?true:undefined);
    start();root=await connect();
    const restored=await until('Restart did not restore the same request',async()=>{const t=await pending();return t?.input_request_id===request.input_request_id&&!t.stale?t:undefined});
    assert.equal(restored.unread,request.unread);
    const reopen=await root.invoke('reopen_task_prompt',{id:taskId,turnKey:restored.turn_key??null,requestId:restored.input_request_id});
    assert.equal(reopen.current?.task_id,taskId);

    const wrongAt=Date.now();
    if(spec.signal==='approval')hook('PostToolUse',{...tool,tool_use_id:'different-call'});
    else await appendFile(rollout,reply('assistant'));
    await waitRead(wrongAt);await Bun.sleep(1200);
    assert.equal((await pending())?.input_request_id,request.input_request_id,'Unrelated result cleared request');
    if(spec.signal==='approval')hook('PostToolUse',{...tool,tool_use_id:'approval-call'});
    else await appendFile(rollout,reply('user'));
    await until('Matching result did not clear the request',async()=>!(await pending())&&(await running())?true:undefined);
    await until('Matching result did not dismiss the notice',async()=>!(await root.invoke('get_task_notice')).current?true:undefined);
    assert.equal((await task()).unread,request.unread);

    const afterResolve=Date.now();await waitRead(afterResolve);await Bun.sleep(1100);
    const end=Date.now();
    const notices=(await log()).filter((r:any)=>r.stage==='queued'&&!beforeRequest.has(r.notice_id));
    const observeUntil=Math.max(end,rendered.at);
    const truth:Truth={cohort:{agent:spec.agent,version:fixture.version,platform:process.platform,signal:spec.signal,lane:'fixed'},oracle:'authored_replay',cases:[
      {id:'noise',expected:'no_request',observableAt:noiseAt,observeUntil:noiseEnd,path:spec.path,pollIntervalMs:1000},
      {id:'request',expected:'request',observableAt:requestAt,observeUntil,path:spec.path,pollIntervalMs:1000,restart:spec.restore,returnTarget:spec.returnTarget}
    ]};
    const resources=await root.invoke('get_resource_diagnostics');
    const resourceSamples=[beforeResources,resources].flatMap(r=>{
      const tree=r.process_memory?.process_tree;
      return tree?.complete&&tree.includes_webview_children&&Number.isSafeInteger(tree.working_set_bytes)
        ? [{workingSetBytes:tree.working_set_bytes,intervalMs:Math.max(1,end-noiseAt)}] : [];
    });
    const report=scoreRequests(truth,{resourceSamples,cases:[
      {id:'noise',observedUntil:noiseEnd,alerts:noiseAlerts},
      {id:'request',observedUntil:observeUntil,alerts:notices.map((n:any)=>({renderedAt:n.notice_id===notice.id&&n.task_id===taskId&&n.request_id===request.input_request_id?rendered.at:undefined})),
        restored:true,returned:{target:'details',verified:true}}
    ]});
    const evidence={...report,fixture:spec,requestLatencyOrigin:'before fixture write (upper bound)',resourceSampleScope:'Available complete Windows process-tree snapshots with WebView2; fixed replay only',resources,buildCommit:expectedCommit,modified:false,executableSha256:hash};
    await Bun.write(join(owned,'result.json'),JSON.stringify(evidence,null,2));
    assert.ok(report.passed,JSON.stringify(report));
    assert.equal(await sha(exe),hash);
    return evidence;
  }));
}
console.log(JSON.stringify({passed:true,reports},null,2));
