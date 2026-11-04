import {expect,test} from 'bun:test';
import {releaseLayout,installerNames} from './release';
import {releaseNotes} from './release-notes';
import {readFileSync} from 'node:fs';
import {join} from 'node:path';
import {windowsTestProcess} from './windows-test-process';

test('Windows bootstrap validates unified identities and native architecture without installing',async()=>{
  if(process.platform!=='win32')return;
  const result=await windowsTestProcess('powershell.exe',['-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File',join(import.meta.dir,'cli-install-unified.test.ps1')]);
  expect(result.exitCode,result.stdout+'\n'+result.stderr).toBe(0);
  expect(result.stdout).toContain('PASS: unified');
},15000);

test('current version keeps legacy attachments and future releases retain MSI compatibility',()=>{
  for(const version of ['1.0.1','1.1.0','1.1.0-rc.1'])expect(releaseLayout(version)).toBe('legacy');
  for(const version of ['1.1.1','1.2.0','2.0.0'])expect(releaseLayout(version)).toBe('compact');
  expect(()=>releaseLayout('invalid')).toThrow();
  expect(installerNames('1.1.1','arm64')).toEqual(['TaskPulse_1.1.1_windows_arm64_setup.exe']);
  expect(installerNames('1.1.1')).toContain('TaskPulse_1.1.1_windows_x64.msi');
});

test('release notes show actual installation links first and never invent an ARM64 download',()=>{
  const version='1.1.1';
  const assets=[...installerNames(version),...installerNames(version,'arm64'),`TaskPulse_${version}_macos_arm64.dmg`,`TaskPulse_${version}_macos_x64.dmg`];
  const body='# TaskPulse v1.1.1\n\n## Added\n\nChanges\n';
  const text=releaseNotes(body,version,assets);
  expect([...text.matchAll(/https:\/\/github.com\/[^)]+/g)]).toHaveLength(4);
  expect(text.indexOf('windows_arm64_setup.exe')).toBeLessThan(text.indexOf('## Added'));
  expect(text).not.toContain('.app.tar.gz');
  const legacy=releaseNotes(body,version,assets.filter(n=>!n.includes('windows_arm64')));
  expect(legacy).not.toContain('/windows_arm64');
  expect(legacy).toContain('not yet available');
  expect(()=>releaseNotes(body,version,[])).toThrow('actual');
});

test('draft publication requires native acceptance on every included architecture',()=>{
  const workflow=Bun.YAML.parse(readFileSync(join(import.meta.dir,'../.github/workflows/release.yml'),'utf8')) as any;
  const draft=workflow.jobs.draft;
  for(const job of ['windows','installer-smoke','macos','windows-arm64','installer-smoke-arm64']){
    expect(draft.needs).toContain(job);
    expect(draft.if).toContain(`needs.${job}.result == 'success'`);
  }
  expect(workflow.jobs['windows-arm64']['runs-on']).toBe('windows-11-arm');
  expect(workflow.jobs['installer-smoke-arm64']['runs-on']).toBe('windows-11-arm');
  const steps=draft.steps.map((s:any)=>s.run??'').join('\n');
  expect(steps).toContain('release-package.py');expect(steps).toContain('release-notes.ts');
  expect(draft.steps.find((s:any)=>s.uses?.startsWith('softprops/')).with.files).toBe('dist/publish/*');
});
