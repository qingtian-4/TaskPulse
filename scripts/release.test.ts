import { expect, test } from 'bun:test';
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { installerNames, msiVersionFor, projectVersions, stageInstallers, validateMsiVersion, validateRelease, verifyChecksums } from './release';

const version='0.2.0-alpha.1';
test('repository versions and release workflow keep publication draft-only and separate from experimental platforms',()=>{
  const root=join(import.meta.dir,'..');
  const versions=projectVersions(root);
  expect(validateRelease(`v${versions[0]}`,versions)).toBe(versions[0]);
  expect(()=>validateMsiVersion(root,versions[0])).not.toThrow();
  const workflow=Bun.YAML.parse(readFileSync(join(root,'.github/workflows/release.yml'),'utf8')) as any;
  expect(workflow.permissions.contents).toBe('read');
  expect(workflow.jobs.windows['runs-on']).toBe('windows-latest');
  expect(workflow.jobs['installer-smoke'].needs).toContain('windows');
  expect(workflow.jobs.draft.needs).toContain('installer-smoke');
  expect(workflow.jobs.draft.permissions.contents).toBe('write');
  const publication=workflow.jobs.draft.steps.find((s:any)=>s.uses?.startsWith('softprops/')).with;
  expect(publication.draft).toBe(true);expect(publication.prerelease).toBe(true);
  expect(publication.make_latest).toBe(false);expect(publication.fail_on_unmatched_files).toBe(true);
  expect(publication.tag_name).toContain('preflight.outputs.tag');
  expect(publication.target_commitish).toContain('preflight.outputs.commit');
  expect(Object.keys(workflow.jobs)).not.toContain('build-macos');
  const commands=workflow.jobs.windows.steps.filter((s:any)=>s.run).map((s:any)=>s.run);
  expect(commands).toContain('bun install --frozen-lockfile');
  expect(commands.some((s:string)=>s.includes('cargo clippy --locked'))).toBe(true);
  expect(commands.every((s:string)=>s.trim().split('\n').length===1)).toBe(true);
});
test('MSI numeric metadata is separate from SemVer but derived and bounded',()=>{
  expect(msiVersionFor('0.2.0-alpha.2')).toBe('0.2.0.1002');
  expect(msiVersionFor('0.2.0-beta.2')).toBe('0.2.0.2002');
  expect(msiVersionFor('0.2.0-rc.2')).toBe('0.2.0.3002');
  for(const v of ['256.2.0-alpha.1','0.256.0-alpha.1','0.2.65536-alpha.1','0.2.0-alpha.1000'])expect(()=>msiVersionFor(v)).toThrow();
});
test('installer acceptance de-elevates only the app, keeps native checks mandatory and saves failures',()=>{
  const root=join(import.meta.dir,'..');
  const workflow=Bun.YAML.parse(readFileSync(join(root,'.github/workflows/release.yml'),'utf8')) as any;
  const smoke=workflow.jobs['installer-smoke'];
  expect(smoke.steps.some((s:any)=>s.run?.includes('gerardog.gsudo'))).toBe(true);
  expect(smoke.steps.some((s:any)=>s.run?.includes('-Gsudo "$env:GSUDO_EXE"'))).toBe(true);
  const parent=readFileSync(join(root,'scripts/installer-smoke.ps1'),'utf8');
  const helper=readFileSync(join(root,'scripts/installer-app-smoke.ps1'),'utf8');
  expect(parent).toContain('--integrity Medium --direct');
  expect(parent).toContain('Medium-integrity native acceptance failed');
  expect(parent).toContain('results=@($results)');
  expect(helper).toContain('WindowsBuiltInRole]::Administrator');
  expect(helper).toContain('WaitForExit(10000)');
  expect(helper).toContain("@('first-launch','restart')");
  expect(helper).toContain('get_pending_task_prompts');
  expect(helper).toContain('last_probe_error=$lastProbeError');
  expect(helper).toContain('-RedirectStandardError');
  for(const name of process.platform==='win32' ? ['installer-smoke.ps1','installer-app-smoke.ps1','installer-process.ps1','installer-process.test.ps1'] : []) {
    const script=join(root,'scripts',name).replaceAll("'","''");
    const result=Bun.spawnSync(['powershell.exe','-NoProfile','-Command',`$parseErrors=$null;[void][System.Management.Automation.Language.Parser]::ParseFile('${script}',[ref]$null,[ref]$parseErrors);if($parseErrors){$parseErrors|Out-String|Write-Error;exit 1}`]);
    expect(result.exitCode, result.stderr.toString()).toBe(0);
  }
},15000);
test('Windows process acceptance retains real fast exit codes/output and enforces timeout without installing software',()=>{
  if(process.platform!=='win32')return;
  const fixture=join(import.meta.dir,'installer-process.test.ps1');
  const result=Bun.spawnSync(['powershell.exe','-NoProfile','-ExecutionPolicy','Bypass','-File',fixture]);
  expect(result.exitCode,result.stderr.toString()).toBe(0);
  expect(result.stdout.toString()).toContain('Fast zero/nonzero exits');
},15000);
test('release tag, app versions and lockfile must agree; stable publication is disabled',()=>{
  expect(validateRelease(`v${version}`,Array(4).fill(version))).toBe(version);
  for (const tag of ['main','app-v'+version,'v0.2.0','v0.2.0-alpha.01','v0.2.0-alpha.1\nattack','v$(whoami)']) {
    expect(()=>validateRelease(tag,Array(4).fill(version))).toThrow();
  }
  expect(()=>validateRelease(`v${version}`,[version,version,'0.1.11',version])).toThrow();
});
test('installer assets are flattened, hashed and versioned, and corruption or stale files fail closed',()=>{
  const root=mkdtempSync(join(tmpdir(),'taskpulse-release-'));
  try {
    const bundle=join(root,'bundle'), out=join(root,'assets');
    mkdirSync(join(bundle,'nsis'),{recursive:true});mkdirSync(join(bundle,'msi'),{recursive:true});
    writeFileSync(join(bundle,'nsis','native.exe'),Buffer.from('4d5a0000','hex'));
    writeFileSync(join(bundle,'msi','native.msi'),Buffer.from('d0cf11e0a1b11ae10000','hex'));
    stageInstallers(bundle,out,version,'a'.repeat(40));
    verifyChecksums(out,version);
    expect(readFileSync(join(out,'SHA256SUMS.txt'),'utf8')).not.toContain('./');
    expect(JSON.parse(readFileSync(join(out,'release-manifest.json'),'utf8')).commit).toBe('a'.repeat(40));
    expect(()=>stageInstallers(bundle,out,version,'a'.repeat(40))).toThrow('empty');
    const exe=join(out,installerNames(version)[0]);
    writeFileSync(exe,'corrupted');expect(()=>verifyChecksums(out,version)).toThrow('Checksum mismatch');
    writeFileSync(join(out,'old.exe'),'old');expect(()=>verifyChecksums(out,version)).toThrow('assets');
  } finally { rmSync(root,{recursive:true,force:true}); }
});
test('missing/duplicate installers and manifest traversal are rejected',()=>{
  const root=mkdtempSync(join(tmpdir(),'taskpulse-release-'));
  try {
    const bundle=join(root,'bundle'),out=join(root,'out');
    mkdirSync(join(bundle,'nsis'),{recursive:true});mkdirSync(join(bundle,'msi'),{recursive:true});
    expect(()=>stageInstallers(bundle,out,version,'a'.repeat(40))).toThrow('exactly one');
    mkdirSync(out);for(const name of installerNames(version))writeFileSync(join(out,name),'fixture');
    writeFileSync(join(out,'SHA256SUMS.txt'),`${'a'.repeat(64)}  ../secret\n${'b'.repeat(64)}  file\n`);
    expect(()=>verifyChecksums(out,version)).toThrow('Invalid checksum');
  } finally { rmSync(root,{recursive:true,force:true}); }
});
