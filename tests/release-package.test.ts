import {expect,test} from 'bun:test';
import {releaseLayout,installerNames} from '../scripts/release';
import {releaseNotes} from '../scripts/release-notes';
import {readFileSync} from 'node:fs';
import {join} from 'node:path';
import {windowsTestProcess} from './support/windows-test-process';

test('Windows bootstrap validates unified identities and native architecture without installing',async()=>{
  if(process.platform!=='win32')return;
  const result=await windowsTestProcess('powershell.exe',['-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File',join(import.meta.dir,'./cli-install-unified.test.ps1')]);
  expect(result.exitCode,result.stdout+'\n'+result.stderr).toBe(0);
  expect(result.stdout).toContain('PASS: unified');
},15000);

test('the first official release keeps full ARM64-capable assets and later releases compact them',()=>{
  expect(releaseLayout('0.0.1')).toBe('full');
  expect(releaseLayout('0.0.2')).toBe('compact');
  expect(releaseLayout('1.0.1')).toBe('legacy');
  for(const version of ['1.0.1','1.1.0','1.1.0-rc.1'])expect(releaseLayout(version)).toBe('legacy');
  for(const version of ['1.1.1','1.2.0','2.0.0'])expect(releaseLayout(version)).toBe('compact');
  expect(()=>releaseLayout('invalid')).toThrow();
  expect(installerNames('1.1.1','arm64')).toEqual(['TaskPulse_1.1.1_windows_arm64_setup.exe']);
  expect(installerNames('0.0.1','arm64')).toEqual(['TaskPulse_0.0.1_windows_arm64_setup.exe']);
  expect(installerNames('1.1.1')).toContain('TaskPulse_1.1.1_windows_x64.msi');
});

test('local ARM64 build and release jobs target the same native architecture and NSIS format',()=>{
  const project=JSON.parse(readFileSync(join(import.meta.dir,'../package.json'),'utf8'));
  const args=project.scripts['build:windows:arm64'].split(/\s+/);
  expect(args[args.indexOf('--target')+1]).toBe('aarch64-pc-windows-msvc');
  expect(args[args.indexOf('--bundles')+1]).toBe('nsis');
  expect(args.slice(args.indexOf('--')+1)).toContain('--locked');
  for(const [file,job] of [['release.yml','windows-arm64'],['release-contract.yml','arm64-build']]){
    const workflow=Bun.YAML.parse(readFileSync(join(import.meta.dir,'../.github/workflows',file),'utf8')) as any;
    const build=workflow.jobs[job].steps.find((s:any)=>s.name?.startsWith('Build ')&&(s.run??s.with?.args??'').includes('--bundles nsis'));
    expect(build,`${file}: native ARM64 build required`).toBeDefined();
    expect(build.run??build.with.args).toContain('--target aarch64-pc-windows-msvc');
    expect(build.run??build.with.args).toContain('--locked');
  }
});

test('release notes show actual installation links first and never invent an ARM64 download',()=>{
  const version='1.1.1';
  const assets=[...installerNames(version),...installerNames(version,'arm64'),`TaskPulse_${version}_macos_arm64.dmg`,`TaskPulse_${version}_macos_x64.dmg`];
  const body='# TaskPulse v1.1.1\n\n## Added\n\nChanges\n';
  const text=releaseNotes(body,version,assets);
  expect([...text.matchAll(/https:\/\/github.com\/[^)]+/g)]).toHaveLength(4);
  expect(text.indexOf('windows_arm64_setup.exe')).toBeLessThan(text.indexOf('## Added'));
  expect(text).not.toContain('.app.tar.gz');
  expect(()=>releaseNotes(body,version,assets.filter(n=>!n.includes('windows_arm64')))).toThrow('actual');
  expect(()=>releaseNotes(body,version,assets.filter(n=>!n.includes('macos_x64')))).toThrow('actual');
  expect(()=>releaseNotes(body,version,[])).toThrow('actual');
  expect(()=>releaseNotes(body,'2.0.0',assets)).toThrow('title');
});

test('draft publication requires native acceptance on every included architecture',()=>{
  const workflow=Bun.YAML.parse(readFileSync(join(import.meta.dir,'../.github/workflows/release.yml'),'utf8')) as any;
  const draft=workflow.jobs.draft;
  for(const job of ['windows','installer-smoke','macos','windows-arm64','installer-smoke-arm64']){
    expect(draft.needs).toContain(job);
    expect(draft.if).toContain(`needs.${job}.result == 'success'`);
  }
  expect(workflow.jobs['windows-arm64']['runs-on']).toBe('windows-11-arm');
  const armSmoke=workflow.jobs['installer-smoke-arm64'];
  expect(armSmoke['runs-on']).toBe('windows-11-arm');
  expect(armSmoke.strategy.matrix.round).toEqual([1,2]);
  const armCommands=armSmoke.steps.map((s:any)=>s.run??'').join('\n');
  expect(armCommands).not.toMatch(/\bwinget\s+install\b/i);
  expect(armCommands).toContain('installer-smoke.ps1');
  expect(armCommands).toContain('-Architecture arm64');
  const steps=draft.steps.map((s:any)=>s.run??'').join('\n');
  expect(steps).toContain('release-package.py');expect(steps).toContain('release-notes.ts');
  expect(draft.steps.find((s:any)=>s.uses?.startsWith('softprops/')).with.files).toBe('dist/publish/*');
  const contracts=Bun.YAML.parse(readFileSync(join(import.meta.dir,'../.github/workflows/release-contract.yml'),'utf8')) as any;
  expect(contracts.permissions).toEqual({contents:'read'});
  expect(contracts.jobs['arm64-build'].if).toContain("'test:installers'");
  expect(contracts.jobs['arm64-build'].if).toContain('head.repo.full_name == github.repository');
  expect(contracts.on.pull_request.types).toContain('labeled');
  const armPreview=contracts.jobs['arm64-install'];
  expect(armPreview?.needs).toContain('arm64-build');
  expect(armPreview['runs-on']).toBe('windows-11-arm');
  expect(armPreview.steps.find((step:any)=>step.uses?.startsWith('actions/download-artifact')).with.name)
    .toBe(contracts.jobs['arm64-build'].steps.find((step:any)=>step.uses?.startsWith('actions/upload-artifact')).with.name);
  const previewCommands=armPreview.steps.map((step:any)=>step.run??'').join('\n');
  expect(previewCommands).toContain('pr-preview');
  expect(previewCommands).toContain('-Architecture arm64 -VerifyRenamedExecutable');
});
