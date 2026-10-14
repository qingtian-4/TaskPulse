import { expect, test } from 'bun:test';
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { installerNames, projectVersions, stageInstallers, validateRelease, verifyChecksums } from './release';

const version='0.2.0-alpha.1';
test('repository versions and release workflow keep publication draft-only and separate from experimental platforms',()=>{
  const root=join(import.meta.dir,'..');
  const versions=projectVersions(root);
  expect(validateRelease(`v${versions[0]}`,versions)).toBe(versions[0]);
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
