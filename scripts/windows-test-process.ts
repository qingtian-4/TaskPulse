/** Release-test probes: observe child close/error, never accept a null status. */
import {spawn} from 'node:child_process';
export function windowsTestProcess(exe:string,args:string[],timeoutMs=10000):Promise<{exitCode:number;stdout:string;stderr:string}> {
  return new Promise((resolve,reject)=>{
    const child=spawn(exe,args,{windowsHide:true,stdio:['ignore','pipe','pipe']});
    let stdout='',stderr='',settled=false;
    const fail=(error:Error)=>{if(settled)return;settled=true;clearTimeout(timer);reject(error);};
    const timer=setTimeout(()=>{child.kill();fail(Error('Windows test subprocess timed out'));},timeoutMs);
    const collect=(channel:'stdout'|'stderr',bytes:Buffer)=>{
      if(channel==='stdout')stdout+=bytes.toString();else stderr+=bytes.toString();
      if(Buffer.byteLength(stdout)+Buffer.byteLength(stderr)>65536){child.kill();fail(Error('Windows test subprocess output exceeded budget'));}
    };
    child.stdout.on('data',bytes=>collect('stdout',bytes));child.stderr.on('data',bytes=>collect('stderr',bytes));
    child.once('error',fail);
    // close follows stdio drain; a fast exit cannot be inferred from a null
    // synchronous-wrapper status. Preserve the actual code or fail explicitly.
    child.once('close',(code,signal)=>{
      if(settled)return;
      if(code===null){fail(Error('Windows test subprocess has no exit code; signal='+signal));return;}
      settled=true;clearTimeout(timer);resolve({exitCode:code,stdout,stderr});
    });
  });
}
