import {test,expect} from 'bun:test';
import {resolve} from 'node:path';
import {windowsTestProcess} from './windows-test-process';

const ready={ready:true,documentReady:'complete',hasDashboard:true,hasTauri:true};
const valid={embedded:true,onboarding:false,tasks:0,prompts:0};
const value=(v:any)=>({result:{result:{value:v}}});
const navigation={error:{code:-32000,message:'Execution context was destroyed.'}};

// Match the production probe's 15s budget; cold PowerShell/JIT can consume 3s.
async function probe(responses:any[],timeout=15000,firstResponseDelay=0,options:{holdConnect?:boolean;holdStage?:'readiness'|'ipc';setupDelay?:number;lateFailure?:boolean}={}){
  let calls=0,ipcCalls=0;
  let delayed:ReturnType<typeof setTimeout>|undefined;
  let releaseConnect:()=>void=()=>{};
  const connectGate=new Promise<void>(ok=>releaseConnect=ok);
  let stopped=false;
  const server=Bun.serve({port:0,hostname:'127.0.0.1',async fetch(req,s){
    if(options.holdConnect)await connectGate;
    if(stopped)return new Response('',{status:503});
    return s.upgrade(req)?undefined:new Response('',{status:400});
  },websocket:{message(ws,message){
    const request=JSON.parse(String(message));calls++;
    const ipc=request.params.expression.includes('get_monitoring_settings');
    if(ipc)ipcCalls++;
    if(options.holdStage===(ipc?'ipc':'readiness'))return;
    const reply=JSON.stringify({id:request.id,...responses[Math.min(calls-1,responses.length-1)]});
    if(calls===1&&firstResponseDelay)delayed=setTimeout(()=>ws.send(reply),firstResponseDelay);
    else ws.send(reply);
  }}});
  const helper=resolve(import.meta.dir,'installer-webview.ps1').replaceAll("'","''");
  // Reproduce timer/clock skew without racing a sub-millisecond scheduler tick.
  // Only object setup is delayed; the real deadline and WebSocket remain live.
  const setup=options.setupDelay?`function New-Object { param([string]$TypeName,[object[]]$ArgumentList) $v=Microsoft.PowerShell.Utility\\New-Object @PSBoundParameters; if($TypeName -eq 'System.Threading.CancellationTokenSource'){Start-Sleep -Milliseconds ${options.setupDelay}}; return ,$v };`:'';
  const lateFailure=options.lateFailure?`function Invoke-InstallerCDPEvaluate { Start-Sleep -Milliseconds ${timeout+100}; throw 'Fixture non-cancellation failure' };`:'';
  try{
    const result=await windowsTestProcess('powershell.exe',['-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-Command',
      `. '${helper}'; ${setup} ${lateFailure} $trace=New-Object 'Collections.Generic.List[object]'; try {$v=Invoke-InstallerWebViewProbe -Url 'ws://127.0.0.1:${server.port}' -Trace $trace -TimeoutMilliseconds ${timeout}; @{value=$v;trace=$trace}|ConvertTo-Json -Depth 12 -Compress} catch {$types=@();$cause=$_.Exception;while($cause){$types+=$cause.GetType().FullName;$cause=$cause.InnerException};@{error=$_.Exception.Message;types=$types;trace=$trace}|ConvertTo-Json -Depth 12 -Compress;exit 1}`],timeout+10000);
    return {...result,calls,ipcCalls,data:JSON.parse(result.stdout.trim())};
  }finally{stopped=true;releaseConnect();clearTimeout(delayed);server.stop(true)}
}

test('native installer probe preserves non-cancellation failures even after the deadline',async()=>{
  if(process.platform!=='win32')return;
  const r=await probe([],1500,0,{lateFailure:true});
  expect(r.exitCode).not.toBe(0);expect(r.ipcCalls).toBe(0);
  expect(r.data.error).toBe('Fixture non-cancellation failure');
  expect(r.data.trace.at(-1).stage).toBe('readiness');
  expect(r.data.trace.at(-1).timeout).not.toBe(true);
},15000);

test('native installer probe waits only for reproduced navigation before invoking IPC once',async()=>{
  if(process.platform!=='win32')return;
  const r=await probe([navigation,navigation,value({...ready,ready:false,documentReady:'loading'}),value(ready),value(valid)],undefined,3500);
  expect(r.exitCode,r.stderr).toBe(0);expect(r.ipcCalls).toBe(1);expect(r.calls).toBe(5);
  expect(r.data.value).toEqual(valid);
  expect(r.data.trace.find((e:any)=>e.stage==='readiness').elapsed_ms).toBeGreaterThanOrEqual(3500);
  expect(r.data.trace.filter((e:any)=>e.protocol_error).map((e:any)=>e.stage)).toEqual(['readiness','readiness']);
  expect(r.data.trace.at(-1).validated).toBe(true);
},30000);

test('native installer probe never retries actual IPC failures, unexpected protocol errors or JS exceptions',async()=>{
  if(process.platform!=='win32')return;
  for(const failure of [navigation,{error:{code:-32000,message:'IPC denied'}},{result:{exceptionDetails:{exception:{description:'Error: permission denied'}}}},{result:{exceptionDetails:{text:'Uncaught (in promise)',exception:{value:'permission denied'}}}}]){
    const r=await probe([value(ready),failure]);
    expect(r.exitCode).not.toBe(0);expect(r.calls).toBe(2);expect(r.ipcCalls).toBe(1);
    expect(r.data.trace.some((e:any)=>e.stage==='ipc'&&(e.protocol_error||e.js_exception))).toBe(true);
    const js=r.data.trace.find((e:any)=>e.js_exception)?.js_exception;
    if(js)expect(js.description).toContain('permission denied');
  }
  const unknown=await probe([{error:{code:-32000,message:'Unknown browser error'}}]);
  expect(unknown.exitCode).not.toBe(0);expect(unknown.calls).toBe(1);expect(unknown.ipcCalls).toBe(0);
},30000);

test('native installer probe rejects missing/null values and imported tasks',async()=>{
  if(process.platform!=='win32')return;
  for(const invalid of [null,{...valid,onboarding:null},{...valid,onboarding:'false'},{...valid,tasks:null},{...valid,prompts:1},{...valid,tasks:1},{...valid,embedded:false}]){
    const r=await probe([value(ready),value(invalid)]);
    expect(r.exitCode).not.toBe(0);expect(r.calls).toBe(2);expect(r.ipcCalls).toBe(1);
  }
},30000);

test('native installer probe classifies the reached phase at its unchanged 600ms total deadline',async()=>{
  if(process.platform!=='win32')return;
  const timed=await probe([value({...ready,ready:false,documentReady:'loading'})],600);
  expect(timed.exitCode).not.toBe(0);expect(timed.ipcCalls).toBe(0);
  // The total budget also includes cold connection setup. Require its exact
  // reached phase rather than assuming a connection always finishes in 600ms.
  const terminal=timed.data.trace.at(-1);
  expect(['connect','readiness']).toContain(terminal.stage);
  expect(terminal.timeout).toBe(true);
  expect(timed.data.trace.filter((entry:any)=>entry.timeout)).toHaveLength(1);
  expect(timed.data.error).toBe(terminal.stage==='connect'?'Native CDP connection timed out':'Native frontend readiness timed out');
},15000);

for(const stage of ['connect','readiness','ipc'] as const){
  test(`native installer probe identifies ${stage} deadline without retrying IPC`,async()=>{
    if(process.platform!=='win32')return;
    const r=await probe([value(ready)],3000,0,{holdConnect:stage==='connect',holdStage:stage==='connect'?undefined:stage,setupDelay:stage==='readiness'?1800:0});
    expect(r.exitCode,r.stderr).not.toBe(0);
    expect(r.data.error,JSON.stringify(r.data)).toBe(stage==='readiness'?'Native frontend readiness timed out':`Native ${stage==='connect'?'CDP connection':'WebView IPC'} timed out`);
    expect(r.ipcCalls).toBe(stage==='ipc'?1:0);
    if(stage==='connect')expect(r.calls).toBe(0);
    else expect(r.calls).toBe(stage==='ipc'?2:1);
    expect(r.data.trace.at(-1)).toMatchObject({stage,timeout:true});
  },15000);
}
