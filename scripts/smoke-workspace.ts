/** Owned smoke-test artifacts only. Never scans/deletes existing caches or user state. */
import {mkdtemp, mkdir, readdir, copyFile, writeFile, access, realpath, lstat, rm} from 'node:fs/promises';
import {resolve, join, dirname, basename} from 'node:path';
import {fileURLToPath} from 'node:url';
import type {ChildProcess} from 'node:child_process';

type Operations={
  remove?:(path:string)=>Promise<void>;
  copy?:(from:string,to:string)=>Promise<void>;
  sleep?:(ms:number)=>Promise<void>;
  now?:()=>number;
  exists?:(path:string)=>Promise<boolean>;
  timeoutMs?:number;
};
export type SmokeWorkspace={owned:string;evidenceDir:string;onStop:(stop:()=>Promise<void>)=>void};
const target=resolve(dirname(fileURLToPath(import.meta.url)),'../src-tauri/target');
const delay=(ms:number)=>new Promise<void>(ok=>setTimeout(ok,ms));
const removeTree=(path:string)=>rm(path,{recursive:true,force:true,maxRetries:0});

/** Bun/Windows may ignore fs.rm maxRetries. Retry transient locks ourselves, for ~10s. */
export async function removeWithRetry(path:string,ops:Operations={}){
  const now=ops.now??Date.now, sleep=ops.sleep??delay, remove=ops.remove??removeTree;
  const exists=ops.exists??(async(path:string)=>{try{await lstat(path);return true;}catch(error){if((error as NodeJS.ErrnoException).code==='ENOENT')return false;throw error;}});
  const deadline=now()+(ops.timeoutMs??10000);
  for(;;){
    try{
      try{await remove(path);}catch(error){if((error as NodeJS.ErrnoException).code!=='ENOENT')throw error;}
      // A successful fs.rm return is not sufficient on Windows: a partial
      // removal or a late WebView2 profile write can leave the directory.
      // Verify immediate absence and a full second of quiescence.
      for(let check=0;check<=4;check++){
        if(check)await sleep(250);
        if(await exists(path))throw Object.assign(Error('Smoke directory remains/reappeared after removal'),{code:'EBUSY'});
      }
      return;
    }
    catch(error){
      const code=(error as NodeJS.ErrnoException).code;
      if(!['EBUSY','EPERM','ENOTEMPTY','EACCES'].includes(code??'') || now()>=deadline)throw error;
      await sleep(Math.min(250,deadline-now()));
    }
  }
}

/** Only the exact ChildProcess handle supplied by the test, never a process-name kill. */
export async function stopOwnedChild(child:ChildProcess|undefined){
  if(!child?.pid || child.exitCode!==null || child.signalCode!==null)return;
  await new Promise<void>((ok,fail)=>{
    const done=()=>{clearTimeout(timer);child.removeListener('exit',exited);child.removeListener('error',errored);};
    const exited=()=>{done();ok();},errored=(error:Error)=>{done();fail(error);};
    const timer=setTimeout(()=>{done();fail(Error('Owned smoke process did not exit within 10s'));},10000);
    child.once('exit',exited);child.once('error',errored);
    if(!child.kill()){done();fail(Error('Unable to stop owned smoke process'));}
  });
}

/** Setup, test, shutdown, evidence and removal share one error boundary. */
export async function withSmokeWorkspace<T>(name:string,run:(ctx:SmokeWorkspace)=>Promise<T>,ops:Operations={}):Promise<T>{
  if(!/^[a-z][a-z0-9-]{0,60}$/.test(name))throw Error('Invalid smoke workspace name');
  await mkdir(target,{recursive:true});
  const owned=await mkdtemp(join(target,name+'-'));
  const evidenceDir=join(target,'ui-smoke/evidence',name+'-'+new Date().toISOString().replace(/[:.]/g,'-')+'-'+basename(owned).slice(name.length+1));
  const stops:Array<()=>Promise<void>>=[];
  let value:T|undefined,failed=false,failure:unknown,reportFailure:unknown;
  try{value=await run({owned,evidenceDir,onStop:stop=>stops.push(stop)});}
  catch(error){
    failed=true;failure=error;
    try{
      const path=join(owned,'failure.json');
      try{await access(path);}catch(missing){
        if((missing as NodeJS.ErrnoException).code!=='ENOENT')throw missing;
        await writeFile(path,JSON.stringify({passed:false,error:String(error)},null,2));
      }
    }catch(error){reportFailure=error;}
  }
  const cleanupErrors:unknown[]=[];
  if(reportFailure)cleanupErrors.push(reportFailure);
  for(const stop of stops.reverse())try{await stop();}catch(error){cleanupErrors.push(error);}
  let preserved=false,removed=false;
  const artifacts:string[]=[];
  try{
    const entries=await readdir(owned,{withFileTypes:true});
    for(const entry of entries){
      // Do not export profiles, configuration, journals, symlinks or arbitrary JSON.
      if(entry.isFile() && (['result.json','failure.json'].includes(entry.name)||/\.png$/i.test(entry.name)))artifacts.push(entry.name);
    }
    if(!artifacts.some(n=>n==='result.json'||n==='failure.json'))throw Error('No smoke result/failure report; preserving working directory');
    await mkdir(evidenceDir,{recursive:true});
    for(const name of artifacts)await (ops.copy??copyFile)(join(owned,name),join(evidenceDir,name));
    preserved=true;
    // Record the retained path before attempting deletion, not after data is gone.
    await writeFile(join(evidenceDir,'cleanup.json'),JSON.stringify({complete:false,working_directory:owned,retained:owned,artifacts},null,2));
    if(cleanupErrors.length===0){
      const root=await realpath(target),actual=await realpath(owned);
      if((await lstat(owned)).isSymbolicLink() || dirname(actual).toLowerCase()!==root.toLowerCase() || basename(actual)!==basename(owned))throw Error('Owned smoke directory escaped target; refusing removal');
      await removeWithRetry(owned,ops);removed=true;
    }
  }catch(error){cleanupErrors.push(error);}
  if(preserved){
    try{await writeFile(join(evidenceDir,'cleanup.json'),JSON.stringify({complete:removed,working_directory:owned,retained:removed?null:owned,artifacts,errors:cleanupErrors.map(String)},null,2));}
    catch(error){cleanupErrors.push(error);}
    console.log('Evidence preserved: '+evidenceDir);
  }
  if(!removed)console.error('Smoke working directory retained: '+owned);
  if(cleanupErrors.length)throw new AggregateError(failed?[failure,...cleanupErrors]:cleanupErrors,'Smoke failed: '+[...(failed?[failure]:[]),...cleanupErrors].map(String).join('; '));
  if(failed)throw failure;
  return value as T;
}
