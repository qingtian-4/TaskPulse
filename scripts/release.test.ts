import { expect, test } from 'bun:test';
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import {windowsTestProcess} from './windows-test-process';
import './installer-webview.test';
import './cli-install.test';
import { installerNames, msiVersionFor, projectVersions, stageInstallers, validateCandidateCommit, validateMsiVersion, validateRelease, verifyChecksums } from './release';

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
  expect(commands).toContain('bun run audit:frontend');
  expect(commands.some((s:string)=>s.includes('cargo clippy --locked'))).toBe(true);
  expect(commands.every((s:string)=>s.trim().split('\n').length===1)).toBe(true);
});
test('quality and release audit the locked frontend graph without swallowing failures',()=>{
  const root=join(import.meta.dir,'..');
  const pkg=JSON.parse(readFileSync(join(root,'package.json'),'utf8'));
  expect(pkg.scripts['audit:frontend']).toBe('bun audit --audit-level=moderate');
  for(const [file,job] of [['quality.yml','quality'],['release.yml','windows']]) {
    const workflow=Bun.YAML.parse(readFileSync(join(root,'.github/workflows',file),'utf8')) as any;
    const steps=workflow.jobs[job].steps;
    const install=steps.findIndex((s:any)=>s.run==='bun install --frozen-lockfile');
    const audit=steps.findIndex((s:any)=>s.run==='bun run audit:frontend');
    expect(install).toBeGreaterThanOrEqual(0);
    expect(audit).toBeGreaterThan(install);
    expect(steps[audit]['continue-on-error']).not.toBe(true);
    expect(workflow.jobs[job]['continue-on-error']).not.toBe(true);
  }
});
test('MSI numeric metadata is separate from SemVer but derived and bounded',()=>{
  expect(msiVersionFor('0.2.0-alpha.2')).toBe('0.2.0.1002');
  expect(msiVersionFor('0.2.0-beta.2')).toBe('0.2.0.2002');
  expect(msiVersionFor('0.2.0-rc.2')).toBe('0.2.0.3002');
  for(const v of ['256.2.0-alpha.1','0.256.0-alpha.1','0.2.65536-alpha.1','0.2.0-alpha.1000'])expect(()=>msiVersionFor(v)).toThrow();
});
test('installer acceptance de-elevates only the app, keeps native checks mandatory and saves failures',async()=>{
  const root=join(import.meta.dir,'..');
  const workflow=Bun.YAML.parse(readFileSync(join(root,'.github/workflows/release.yml'),'utf8')) as any;
  const smoke=workflow.jobs['installer-smoke'];
  expect(smoke.strategy.matrix.installer).toEqual(['nsis','msi']);
  expect(smoke.strategy.matrix.round).toEqual([1,2]);
  expect(smoke.strategy['fail-fast']).toBe(false);
  expect(smoke.steps.find((s:any)=>s.uses?.startsWith('actions/upload-artifact')).with.name).toContain('matrix.installer');
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
  const probe=readFileSync(join(root,'scripts/installer-webview.ps1'),'utf8');
  expect(probe).toContain('get_pending_task_prompts');
  expect(helper).toContain('last_probe_error=$lastProbeError');
  expect(helper).toContain('-RedirectStandardError');
  expect(helper).toContain('Test-InstallerSameExecutable $live.Path $Exe');
  expect(probe).toContain('[void]$socket.ConnectAsync');
  expect(probe).toContain('[void]$Socket.SendAsync');
  expect(parent).toContain('$native.native_ipc -is [Array]');
  expect(parent).toContain("[ValidateSet('nsis','msi')]");
  expect(parent).toContain('foreach ($kind in @($InstallerKind))');
  for(const name of process.platform==='win32' ? ['installer-smoke.ps1','installer-app-smoke.ps1','installer-process.ps1','installer-process.test.ps1','installer-webview.ps1'] : []) {
    const script=join(root,'scripts',name).replaceAll("'","''");
    const result=await windowsTestProcess('powershell.exe',['-NoProfile','-NonInteractive','-Command',`$parseErrors=$null;[void][System.Management.Automation.Language.Parser]::ParseFile('${script}',[ref]$null,[ref]$parseErrors);if($parseErrors){$parseErrors|Out-String|Write-Error;exit 1}`]);
    expect(result.exitCode, result.stderr).toBe(0);
  }
},15000);
test('Windows process acceptance retains real fast exit codes/output and enforces timeout without installing software',async()=>{
  if(process.platform!=='win32')return;
  const fixture=join(import.meta.dir,'installer-process.test.ps1');
  const result=await windowsTestProcess('powershell.exe',['-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File',fixture]);
  expect(result.exitCode,result.stderr).toBe(0);
  expect(result.stdout).toContain('Fast zero/nonzero exits');
},15000);

test('Windows test launcher retains actual fast errors and rejects startup/timeout failures',async()=>{
  if(process.platform!=='win32')return;
  for(let i=0;i<5;i++){
    const result=await windowsTestProcess('cmd.exe',['/d','/c','echo TaskPulse-launch-ok']);
    expect(result.exitCode).toBe(0);expect(result.stdout.trim()).toBe('TaskPulse-launch-ok');
  }
  expect((await windowsTestProcess('cmd.exe',['/d','/c','exit 7'])).exitCode).toBe(7);
  await expect(windowsTestProcess('taskpulse-missing-test-executable.exe',[])).rejects.toThrow();
  await expect(windowsTestProcess('powershell.exe',['-NoProfile','-Command','Start-Sleep -Seconds 10'],100)).rejects.toThrow('timed out');
},15000);
test('release tag, app versions and lockfile must agree; stable publication is disabled',()=>{
  expect(validateRelease(`v${version}`,Array(4).fill(version))).toBe(version);
  for (const tag of ['main','app-v'+version,'v0.2.0','v0.2.0-alpha.01','v0.2.0-alpha.1\nattack','v$(whoami)']) {
    expect(()=>validateRelease(tag,Array(4).fill(version))).toThrow();
  }
  expect(()=>validateRelease(`v${version}`,[version,version,'0.1.11',version])).toThrow();
});

test('untagged candidate pins the exact main SHA and cannot enter the Release job',()=>{
  const sha='a'.repeat(40),other='b'.repeat(40);
  expect(()=>validateCandidateCommit(sha,sha,sha)).not.toThrow();
  for(const args of [[sha.slice(0,7),sha,sha],[sha,other,sha],[sha,sha,other],['main',sha,sha]])
    expect(()=>validateCandidateCommit(...args as [string,string,string])).toThrow('full SHA');
  const workflow=Bun.YAML.parse(readFileSync(join(import.meta.dir,'../.github/workflows/release.yml'),'utf8')) as any;
  expect(workflow.on.workflow_dispatch.inputs.commit.required).toBe(false);
  expect(workflow.jobs.draft.if).toBe("${{ needs.preflight.outputs.tag != '' }}");
  expect(workflow.jobs.preflight.steps.find((s:any)=>s.id==='version').run).toContain('candidate --commit "$CANDIDATE_COMMIT"');
  expect(workflow.jobs.windows.needs).toContain('security');
  expect(workflow.jobs['installer-smoke'].strategy.matrix.installer).toEqual(['nsis','msi']);
  const preflight=workflow.jobs.preflight.steps.find((s:any)=>s.id==='version').run;
  expect(preflight).toContain('"$PROBE_COMMIT" != "$GITHUB_SHA"');
  expect(preflight).toContain('-z "$CANDIDATE_COMMIT"');
  expect(workflow.jobs.windows.steps[0].with.ref).toBe('${{ needs.preflight.outputs.commit }}');
  expect(workflow.jobs['installer-smoke'].steps[0].with.ref).toBe('${{ needs.preflight.outputs.probe_commit || needs.preflight.outputs.commit }}');
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
    const candidate=join(root,'candidate');stageInstallers(bundle,candidate,version,'a'.repeat(40),{tagged:false});
    const manifest=JSON.parse(readFileSync(join(candidate,'release-manifest.json'),'utf8'));
    expect(manifest.tag).toBeNull();expect(manifest.build_kind).toBe('commit-candidate');verifyChecksums(candidate,version);
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
