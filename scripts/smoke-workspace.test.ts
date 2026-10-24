import {test, expect} from 'bun:test';
import {mkdir, writeFile, readFile, readdir, rm, symlink} from 'node:fs/promises';
import {join, resolve, sep} from 'node:path';
import {spawn, type ChildProcess} from 'node:child_process';
import {withSmokeWorkspace, removeWithRetry, stopOwnedChild} from './smoke-workspace';

test('setup failure writes evidence before removing the owned executable directory', async()=>{
  let owned='',evidence='';
  await expect(withSmokeWorkspace('cleanup-test',async ctx=>{
    owned=ctx.owned;evidence=ctx.evidenceDir;
    await writeFile(join(owned,'agent-tray.exe'),'isolated placeholder, never executed');
    throw Error('setup failed before native startup');
  })).rejects.toThrow('setup failed');
  expect(await Bun.file(join(owned,'agent-tray.exe')).exists()).toBe(false);
  const report=JSON.parse(await readFile(join(evidence,'failure.json'),'utf8'));
  expect(report.passed).toBe(false);expect(report.error).toContain('setup failed');
});

test('all top-level screenshot variants survive; profiles and config are not exported',async()=>{
  let owned='',evidence='';
  await withSmokeWorkspace('cleanup-test',async ctx=>{
    owned=ctx.owned;evidence=ctx.evidenceDir;
    for(const name of ['dashboard-100.png','dashboard-150.png','sources-200.png','notice-100.png','management.png','截图示例.png'])await writeFile(join(owned,name),'synthetic image');
    await mkdir(join(owned,'profile'));await writeFile(join(owned,'profile','private.json'),'do not export');
    await writeFile(join(owned,'config.toml'),'do not export');
    await writeFile(join(owned,'result.json'),'{"passed":true}');
  });
  const names=await readdir(evidence);
  expect(names.filter(n=>n.endsWith('.png')).length).toBe(6);
  expect(names).not.toContain('config.toml');expect(names).not.toContain('profile');
  expect(await Bun.file(join(owned,'result.json')).exists()).toBe(false);
});

test('custom failure diagnostics survive rather than being overwritten by the wrapper',async()=>{
  let evidence='';
  await expect(withSmokeWorkspace('cleanup-test',async ctx=>{
    evidence=ctx.evidenceDir;
    await writeFile(join(ctx.owned,'failure.json'),JSON.stringify({passed:false,checks:['early check'],error:'specific failure'}));
    throw Error('specific failure');
  })).rejects.toThrow('specific failure');
  expect(JSON.parse(await readFile(join(evidence,'failure.json'),'utf8')).checks).toEqual(['early check']);
});

async function removeRetainedFixture(owned:string){
  const root=resolve('src-tauri/target');
  if(!resolve(owned).startsWith(root+sep)||!owned.split(sep).at(-1)?.startsWith('cleanup-test-'))throw Error('Fixture path escaped');
  await rm(owned,{recursive:true,force:true});
}

test('evidence copy failure retains originals and does not invoke deletion',async()=>{
  let owned='',removals=0;
  try{
    await expect(withSmokeWorkspace('cleanup-test',async ctx=>{
      owned=ctx.owned;await writeFile(join(owned,'result.json'),'{"passed":true}');
    },{copy:async()=>{throw Error('disk full');},remove:async()=>{removals++;}})).rejects.toThrow('disk full');
    expect(removals).toBe(0);expect(await Bun.file(join(owned,'result.json')).exists()).toBe(true);
  }finally{if(owned)await removeRetainedFixture(owned);}
});

test('failed process stop saves evidence but retains the working directory',async()=>{
  let owned='',evidence='',removals=0;
  try{
    await expect(withSmokeWorkspace('cleanup-test',async ctx=>{
      owned=ctx.owned;evidence=ctx.evidenceDir;
      ctx.onStop(async()=>{throw Error('owned process still active');});
      await writeFile(join(owned,'result.json'),'{"passed":true}');
    },{remove:async()=>{removals++;}})).rejects.toThrow('owned process still active');
    expect(removals).toBe(0);expect(await Bun.file(join(evidence,'result.json')).exists()).toBe(true);
    expect(JSON.parse(await readFile(join(evidence,'cleanup.json'),'utf8')).complete).toBe(false);
  }finally{if(owned)await removeRetainedFixture(owned);}
});

test('no report means no destructive cleanup or misleading success',async()=>{
  let owned='';
  try{
    await expect(withSmokeWorkspace('cleanup-test',async ctx=>{owned=ctx.owned;})).rejects.toThrow('report');
    expect(await readdir(owned)).toEqual([]);
  }finally{if(owned)await removeRetainedFixture(owned);}
});

test('Windows lock errors retry explicitly until release, independently of Bun fs.rm retries',async()=>{
  let attempts=0,clock=0;
  await removeWithRetry('unused fixture path',{remove:async()=>{if(++attempts<4)throw Object.assign(Error('locked'),{code:'EBUSY'});},now:()=>clock,sleep:async ms=>{clock+=ms;},timeoutMs:3000});
  expect(attempts).toBe(4);expect(clock).toBe(1750);
});

test('a remaining/recreated profile after successful rm must be removed again before cleanup is complete',async()=>{
  for(const existsAt of [1,2])for(const missingAtFirstRemove of [false,true]){
    let attempts=0,clock=0,checks=0;
    await removeWithRetry('unused fixture path',{remove:async()=>{if(++attempts===1&&missingAtFirstRemove)throw Object.assign(Error('already absent'),{code:'ENOENT'});},exists:async()=>++checks===existsAt,now:()=>clock,sleep:async ms=>{clock+=ms;},timeoutMs:5000});
    expect(attempts).toBe(2);expect(clock).toBeGreaterThanOrEqual(1000);
  }
});

test('persistent lock retries are bounded and non-lock failures fail immediately',async()=>{
  let attempts=0,clock=0;
  await expect(removeWithRetry('unused fixture path',{remove:async()=>{attempts++;throw Object.assign(Error('locked'),{code:'EPERM'});},now:()=>clock,sleep:async ms=>{clock+=ms;},timeoutMs:1000})).rejects.toThrow('locked');
  expect(clock).toBe(1000);expect(attempts).toBe(5);
  attempts=0;
  await expect(removeWithRetry('unused fixture path',{remove:async()=>{attempts++;throw Object.assign(Error('bad IO'),{code:'EIO'});}})).rejects.toThrow('bad IO');
  expect(attempts).toBe(1);
});

test('workspace names cannot target existing build trees or paths outside target',async()=>{
  for(const name of ['../debug','debug/child','D:\\data',''])await expect(withSmokeWorkspace(name,async()=>{})).rejects.toThrow('name');
});

test('all owned CLI smoke setups enter the lifecycle before allocating copies or processes',async()=>{
  for(const script of ['integration-bridge-smoke.ts','native-product-smoke.ts','native-integration-smoke.ts','native-attention-regression-smoke.ts']){
    const text=await readFile(join('scripts',script),'utf8'),boundary=text.indexOf('await withSmokeWorkspace(');
    expect(boundary,script).toBeGreaterThan(0);
    expect(text,script).not.toContain('mkdtemp(');
    expect(text,script).not.toContain("process.on('exit'");
    for(const operation of ['copyFile(','spawn('])expect(text.indexOf(operation),script+' '+operation).toBeGreaterThan(boundary);
  }
});

test('local verify and both Windows CI workflows gate on cleanup regression tests',async()=>{
  const manifest=JSON.parse(await readFile('package.json','utf8'));
  expect(manifest.scripts['test:smoke-cleanup']).toBe('bun test ./scripts/smoke-workspace.test.ts');
  for(const path of ['scripts/verify.ps1','.github/workflows/quality.yml','.github/workflows/release.yml'])expect(await readFile(path,'utf8'),path).toContain('run test:smoke-cleanup');
});

test('profile junctions are not exported or followed into another owned fixture',async()=>{
  await withSmokeWorkspace('cleanup-test',async outer=>{
    const outside=join(outer.owned,'outside');await mkdir(outside);
    await writeFile(join(outside,'keep.json'),'preserve sibling fixture');
    let evidence='';
    await withSmokeWorkspace('cleanup-test',async inner=>{
      evidence=inner.evidenceDir;
      await symlink(outside,join(inner.owned,'profile-link'),process.platform==='win32'?'junction':'dir');
      await writeFile(join(inner.owned,'result.json'),'{"passed":true}');
    });
    expect(await readFile(join(outside,'keep.json'),'utf8')).toBe('preserve sibling fixture');
    expect(await readdir(evidence)).not.toContain('profile-link');
    await writeFile(join(outer.owned,'result.json'),'{"passed":true}');
  });
});

test.skipIf(process.platform!=='win32')('actual Windows profile handle releases during bounded cleanup retries',async()=>{
  let holder:ChildProcess|undefined,owned='',attempts=0;
  try{
    await withSmokeWorkspace('cleanup-test',async ctx=>{
      owned=ctx.owned;
      await mkdir(join(owned,'profile'));
      const path=join(owned,'profile','held.bin');await writeFile(path,'isolated lock fixture');
      const script=`$tpLock=[IO.File]::Open('${path.replaceAll("'","''")}',[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::ReadWrite);try{[Console]::Out.WriteLine('locked');Start-Sleep -Milliseconds 1200}finally{$tpLock.Dispose()}`;
      holder=spawn('powershell.exe',['-NoLogo','-NoProfile','-NonInteractive','-Command',script],{windowsHide:true,stdio:['ignore','pipe','pipe']});
      await new Promise<void>((ok,fail)=>{
        const timer=setTimeout(()=>fail(Error('Windows lock fixture did not start')),5000);
        holder!.once('error',error=>{clearTimeout(timer);fail(error);});
        holder!.stdout!.once('data',data=>{clearTimeout(timer);String(data).includes('locked')?ok():fail(Error('Lock not acquired'));});
      });
      // Models a profile handle outliving the parent, not an Agent process.
      await writeFile(join(owned,'result.json'),'{"passed":true}');
    },{remove:async path=>{attempts++;await rm(path,{recursive:true,force:true,maxRetries:0});}});
    expect(attempts).toBeGreaterThan(1);
    expect(await readdir(owned).then(()=>true).catch(()=>false)).toBe(false);
  }finally{await stopOwnedChild(holder);if(owned)await removeRetainedFixture(owned);}
},20000);

test.skipIf(!process.env.TASKPULSE_SMOKE_EXE)('real CLI early failures retain reports without leaked executable/profile directories',async()=>{
  const exe=resolve(process.env.TASKPULSE_SMOKE_EXE!);
  const run=async(script:string,args:string[],message:RegExp)=>{
    const child=Bun.spawn([process.execPath,resolve('scripts',script),...args],{stdout:'pipe',stderr:'pipe',env:process.env});
    const timer=setTimeout(()=>child.kill(),30000);
    let code:number,stdout:string,stderr:string;
    try{[code,stdout,stderr]=await Promise.all([child.exited,new Response(child.stdout).text(),new Response(child.stderr).text()]);}
    finally{clearTimeout(timer);}
    expect(code).not.toBe(0);
    const path=stdout.match(/^Evidence preserved: (.+)$/m)?.[1]?.trim();
    expect(path,stdout+'\n'+stderr).toBeTruthy();
    const report=JSON.parse(await readFile(join(path!,'failure.json'),'utf8'));
    expect(report.passed).toBe(false);expect(report.error).toMatch(message);
    const cleanup=JSON.parse(await readFile(join(path!,'cleanup.json'),'utf8'));
    expect(cleanup.complete).toBe(true);expect(cleanup.retained).toBeNull();
    expect(await readdir(cleanup.working_directory).then(()=>true).catch(()=>false)).toBe(false);
  };
  const missing=join(resolve('src-tauri/target'),'smoke-nonexistent-'+crypto.randomUUID()+'.exe');
  await run('integration-bridge-smoke.ts',['--exe',missing],/ENOENT/);
  for(const port of [9337,9339]){
    // Refuse occupied ports; never attach to, kill or reconfigure another app.
    const server=Bun.serve({hostname:'127.0.0.1',port,fetch:()=>Response.json([])});
    try{
      const scripts=port===9337?['native-product-smoke.ts','native-integration-smoke.ts']:['native-attention-regression-smoke.ts'];
      for(const script of scripts)await run(script,['--exe',exe],/CDP|occupied/);
    }finally{await server.stop(true);}
  }
  await withSmokeWorkspace('cleanup-test',async ctx=>{
    const invalid=join(ctx.owned,'invalid-fixture.exe');await writeFile(invalid,'not an executable');
    for(const script of ['native-product-smoke.ts','native-integration-smoke.ts','native-attention-regression-smoke.ts'])await run(script,['--exe',invalid],/spawn|exec|exited|ENOEXEC|EINVAL|EACCES/i);
    await writeFile(join(ctx.owned,'result.json'),'{"passed":true}');
  });
},60000);
