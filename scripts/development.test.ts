import {test,expect} from 'bun:test';
import {mkdtempSync,writeFileSync,mkdirSync,readFileSync,rmSync,symlinkSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {developmentConfig} from './develop';
import {stageDevelopment,verifyDevelopmentBuild} from './development-artifact';
import {sha256} from './release';
import {windowsTestProcess} from './windows-test-process';
const read=(path:string)=>readFileSync(new URL('../'+path,import.meta.url),'utf8');

test('native acceptance rejects stale, dirty or unidentifiable builds',()=>{
  const commit='a'.repeat(40);
  const build={channel:'development',commit,modified:false,version:'1.0.2'};
  expect(()=>verifyDevelopmentBuild(build,commit,'1.0.2')).not.toThrow();
  for(const change of [{modified:true},{modified:undefined},{commit:'b'.repeat(40)},{channel:'release'},{version:'1.0.1'}]){
    expect(()=>verifyDevelopmentBuild({...build,...change},commit,'1.0.2')).toThrow('does not match');
  }
  expect(()=>verifyDevelopmentBuild(build,'main','1.0.2')).toThrow('full source SHA');
});

test('parallel worktrees use separate development identities and exact loopback HMR origins',()=>{
  const a=developmentConfig('/worktree-a',5333),b=developmentConfig('/worktree-b',5444);
  expect(a.identifier).not.toBe(b.identifier);expect(a.bundle.active).toBe(false);
  expect(a.build.beforeDevCommand.script).toContain('run dev');
  expect(a.build.devUrl).toBe('http://127.0.0.1:5333');
  expect(a.app.security.devCsp).toContain('ws://127.0.0.1:5333');
  expect(a.app.security.devCsp).not.toContain('*');
  for(const port of [0,1,65536,1.2,NaN])expect(()=>developmentConfig('/worktree',port)).toThrow();
  expect(JSON.parse(read('src-tauri/tauri.conf.json')).identifier).toBe('app.taskpulse.desktop');
  expect(JSON.parse(read('src-tauri/tauri.conf.json')).app.security.csp).not.toContain('ws:');
});
test('native standard-account setup refuses execution outside disposable hosted CI',async()=>{
  if(process.platform!=='win32')return;
  const helper=new URL('./native-standard-user.ps1',import.meta.url);
  const {fileURLToPath}=await import('node:url');
  const result=await windowsTestProcess('powershell.exe',['-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-Command',`$env:GITHUB_ACTIONS='';$env:RUNNER_ENVIRONMENT=''; & '${fileURLToPath(helper).replaceAll("'","''")}' -Bun missing.exe -Exe missing.exe`]);
  expect(result.exitCode).not.toBe(0);expect(result.stderr).toContain('Disposable hosted CI only');
},15000);
test('native account mode parameters cannot alias the process instance variable',async()=>{
  if(process.platform!=='win32')return;
  const source=read('scripts/native-standard-user.ps1');
  const parameters=source.split(/\r?\n/)[0];
  const allocation=source.match(/(\$\w+)=\[Diagnostics\.Process\]::new\(\)/);
  expect(allocation).not.toBeNull();
  const root=mkdtempSync(join(tmpdir(),'taskpulse-native-parameters-'));
  try{
    const fixture=join(root,'parameters.ps1');
    writeFileSync(fixture,`${parameters}\n$ErrorActionPreference='Stop';${allocation![0]};if(${allocation![1]} -isnot [Diagnostics.Process]){throw 'Process allocation failed'};${allocation![1]}.Dispose();Write-Output 'native-parameters-ok'`);
    const result=await windowsTestProcess('powershell.exe',['-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File',fixture,'-Bun','unused.exe','-Exe','unused.exe']);
    expect(result.exitCode,result.stderr).toBe(0);expect(result.stdout).toContain('native-parameters-ok');
  }finally{rmSync(root,{recursive:true,force:true});}
},15000);
test('development artifacts allowlist runtime files, pin checksums and reject stale or incorrect packages',()=>{
  const root=mkdtempSync(join(tmpdir(),'taskpulse-dev-stage-'));
  try{
    const bin=join(root,'bin');mkdirSync(bin);
    const exe=join(bin,'agent-tray.exe'),out=join(root,'out');
    const bytes=Buffer.alloc(128);bytes.write('MZ');bytes.writeUInt32LE(64,60);bytes.write('PE\0\0',64);bytes.writeUInt16LE(0x8664,68);
    writeFileSync(exe,bytes);writeFileSync(join(bin,'WebView2Loader.dll'),'loader');
    writeFileSync(join(bin,'attention.json'),'private');writeFileSync(join(bin,'config.toml'),'private');
    stageDevelopment(exe,out,'a'.repeat(40),'1.0.1');
    const manifest=JSON.parse(readFileSync(join(out,'development-manifest.json'),'utf8'));
    expect(manifest).toMatchObject({tag:null,build_kind:'development',commit:'a'.repeat(40)});
    expect(manifest.assets).toEqual(['agent-tray.exe','WebView2Loader.dll']);
    expect(readFileSync(join(out,'SHA256SUMS.txt'),'utf8')).toContain(sha256(bytes)+'  agent-tray.exe');
    expect(()=>readFileSync(join(out,'attention.json'))).toThrow();
    expect(()=>stageDevelopment(exe,out,'a'.repeat(40),'1.0.1')).toThrow('already exists');
    expect(()=>stageDevelopment(exe,join(root,'bad-sha'),'main','1.0.1')).toThrow('full source SHA');
    writeFileSync(exe,'not a native executable');
    expect(()=>stageDevelopment(exe,join(root,'bad-exe'),'a'.repeat(40),'1.0.1')).toThrow('Windows x64');
    if(process.platform!=='win32'){
      const link=join(root,'link.exe');symlinkSync(exe,link);
      expect(()=>stageDevelopment(link,join(root,'bad-link'),'a'.repeat(40),'1.0.1')).toThrow('regular');
    }
  }finally{rmSync(root,{recursive:true,force:true});}
});
test('fast CI produces expiring SHA-bound development artifacts; installer and upgrade suites require explicit opt-in',()=>{
  const quality=Bun.YAML.parse(read('.github/workflows/quality.yml')) as any;
  expect(quality.jobs.quality.steps.some((s:any)=>s.run==='bun run test:workflow')).toBe(true);
  const artifact=quality.jobs.quality.steps.find((s:any)=>s.id==='development');
  expect(artifact.with['retention-days']).toBe(7);expect(artifact.with.name).toContain('head.sha || github.sha');
  expect(artifact.with.path).toBe('dist/development/');
  for(const [file,job,label] of [['installation-preview.yml','prepare','test:installers'],['upgrade-acceptance.yml','security','test:upgrade']]){
    const w=Bun.YAML.parse(read('.github/workflows/'+file)) as any;
    expect(w.on.pull_request.types).toContain('labeled');expect(w.jobs[job].if).toContain(label);
    expect(w.concurrency.group).toContain("github.event.label.name != '"+label+"'");expect(w.concurrency.group).toContain('github.run_id');
    expect(w.permissions).toEqual({contents:'read'});
    for(const j of Object.values(w.jobs) as any[])for(const s of j.steps??[])if(s.uses?.startsWith('actions/upload-artifact'))expect(s.with['retention-days']).toBe(7);
  }
  const release=read('.github/workflows/release.yml');
  expect(release).toContain('Formal releases require a stable version tag');
  const w=Bun.YAML.parse(release) as any;
  const publish=w.jobs.draft.steps.find((s:any)=>s.uses?.startsWith('softprops/action-gh-release'));
  expect(publish.with.draft).toBe(true);expect(publish.with.prerelease).toBe(false);
});
