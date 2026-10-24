import {test,expect} from 'bun:test';
import {resolve} from 'node:path';
import {windowsTestProcess} from './windows-test-process';

const ready={ready:true,documentReady:'complete',hasDashboard:true,hasTauri:true};
const valid={embedded:true,onboarding:false,tasks:0,prompts:0};
const value=(v:any)=>({result:{result:{value:v}}});
const navigation={error:{code:-32000,message:'Execution context was destroyed.'}};

async function probe(responses:any[],timeout=3000){
  let calls=0,ipcCalls=0;
  const server=Bun.serve({port:0,hostname:'127.0.0.1',fetch(req,s){return s.upgrade(req)?undefined:new Response('',{status:400})},websocket:{message(ws,message){
    const request=JSON.parse(String(message));calls++;
    if(request.params.expression.includes('get_monitoring_settings'))ipcCalls++;
    ws.send(JSON.stringify({id:request.id,...responses[Math.min(calls-1,responses.length-1)]}));
  }}});
  const helper=resolve(import.meta.dir,'installer-webview.ps1').replaceAll("'","''");
  try{
    const result=await windowsTestProcess('powershell.exe',['-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-Command',
      `. '${helper}'; $trace=New-Object 'Collections.Generic.List[object]'; try {$v=Invoke-InstallerWebViewProbe -Url 'ws://127.0.0.1:${server.port}' -Trace $trace -TimeoutMilliseconds ${timeout}; @{value=$v;trace=$trace}|ConvertTo-Json -Depth 12 -Compress} catch {@{error=$_.Exception.Message;trace=$trace}|ConvertTo-Json -Depth 12 -Compress;exit 1}`],10000);
    return {...result,calls,ipcCalls,data:JSON.parse(result.stdout.trim())};
  }finally{server.stop(true)}
}

test('native installer probe waits only for reproduced navigation before invoking IPC once',async()=>{
  if(process.platform!=='win32')return;
  const r=await probe([navigation,navigation,value({...ready,ready:false,documentReady:'loading'}),value(ready),value(valid)]);
  expect(r.exitCode,r.stderr).toBe(0);expect(r.ipcCalls).toBe(1);expect(r.calls).toBe(5);
  expect(r.data.value).toEqual(valid);
  expect(r.data.trace.filter((e:any)=>e.protocol_error).map((e:any)=>e.stage)).toEqual(['readiness','readiness']);
  expect(r.data.trace.at(-1).validated).toBe(true);
},15000);

test('native installer probe never retries actual IPC failures, unexpected protocol errors or JS exceptions',async()=>{
  if(process.platform!=='win32')return;
  for(const failure of [navigation,{error:{code:-32000,message:'IPC denied'}},{result:{exceptionDetails:{exception:{description:'Error: permission denied'}}}}]){
    const r=await probe([value(ready),failure]);
    expect(r.exitCode).not.toBe(0);expect(r.calls).toBe(2);expect(r.ipcCalls).toBe(1);
    expect(r.data.trace.some((e:any)=>e.stage==='ipc'&&(e.protocol_error||e.js_exception))).toBe(true);
  }
  const unknown=await probe([{error:{code:-32000,message:'Unknown browser error'}}]);
  expect(unknown.exitCode).not.toBe(0);expect(unknown.calls).toBe(1);expect(unknown.ipcCalls).toBe(0);
},30000);

test('native installer probe rejects missing/null values, imported tasks and readiness timeout',async()=>{
  if(process.platform!=='win32')return;
  for(const invalid of [null,{...valid,onboarding:null},{...valid,onboarding:'false'},{...valid,tasks:null},{...valid,prompts:1},{...valid,tasks:1},{...valid,embedded:false}]){
    const r=await probe([value(ready),value(invalid)]);
    expect(r.exitCode).not.toBe(0);expect(r.calls).toBe(2);expect(r.ipcCalls).toBe(1);
  }
  const timed=await probe([value({...ready,ready:false,documentReady:'loading'})],600);
  expect(timed.exitCode).not.toBe(0);expect(timed.ipcCalls).toBe(0);expect(timed.data.error).toContain('readiness timed out');
},30000);
