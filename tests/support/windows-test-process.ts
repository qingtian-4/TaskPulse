/** Release-test probes: observe child close/error, never accept a null status. */
import {spawn} from 'node:child_process';
type ProcessTimings={spawn_ms:number;first_stdout_ms?:number;first_stderr_ms?:number;stdout_close_ms?:number;stderr_close_ms?:number;exit_ms?:number;error_ms?:number};
export function windowsTestProcess(exe:string,args:string[],timeoutMs=10000):Promise<{exitCode:number;stdout:string;stderr:string;timings:ProcessTimings}> {
  return new Promise((resolve,reject)=>{
    const started=performance.now();
    const child=spawn(exe,args,{windowsHide:true,stdio:['ignore','pipe','pipe']});
    const timings:ProcessTimings={spawn_ms:Math.round(performance.now()-started)};
    let stdout='',stderr='',settled=false,stdoutSeen=false,stderrSeen=false;
    const elapsed=()=>Math.round(performance.now()-started);
    const fail=(error:Error)=>{
      if(settled)return;
      settled=true;clearTimeout(timer);
      const probeStages=stderr.split(/\r?\n/).filter(line=>line.includes('TaskPulseProbe stage=')).slice(-32).join('|');
      reject(Error(`${error.message}; timings=${JSON.stringify(timings)}${probeStages?`; probe_stages=${probeStages}`:''}`));
    };
    const timer=setTimeout(()=>{child.kill();fail(Error(`Windows test subprocess timed out after ${timeoutMs}ms`));},timeoutMs);
    const collect=(channel:'stdout'|'stderr',bytes:Buffer)=>{
      if(channel==='stdout') { if(!stdoutSeen){stdoutSeen=true;timings.first_stdout_ms=elapsed();} stdout+=bytes.toString(); }
      else { if(!stderrSeen){stderrSeen=true;timings.first_stderr_ms=elapsed();} stderr+=bytes.toString(); }
      if(Buffer.byteLength(stdout)+Buffer.byteLength(stderr)>65536){child.kill();fail(Error('Windows test subprocess output exceeded budget'));}
    };
    child.stdout.on('data',bytes=>collect('stdout',bytes));child.stderr.on('data',bytes=>collect('stderr',bytes));
    child.stdout.once('close',()=>{timings.stdout_close_ms=elapsed();});child.stderr.once('close',()=>{timings.stderr_close_ms=elapsed();});
    child.once('error',error=>{timings.error_ms=elapsed();fail(error);});
    // close follows stdio drain; a fast exit cannot be inferred from a null
    // synchronous-wrapper status. Preserve the actual code or fail explicitly.
    child.once('close',(code,signal)=>{
      if(settled)return;
      timings.exit_ms=elapsed();
      if(code===null){fail(Error('Windows test subprocess has no exit code; signal='+signal));return;}
      settled=true;clearTimeout(timer);resolve({exitCode:code,stdout,stderr,timings});
    });
  });
}
