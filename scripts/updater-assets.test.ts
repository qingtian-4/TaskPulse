import {expect,test} from 'bun:test';
import {mkdtempSync,mkdirSync,readFileSync,writeFileSync,rmSync,copyFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {stageUpdates,assembleFeed} from './updater-assets';
const version='1.0.0',commit='a'.repeat(40);
const fixtureSig=Buffer.from(`untrusted comment: fixture\n${Buffer.alloc(74).toString('base64')}\ntrusted comment: fixture\tversion:${version}\n${Buffer.alloc(64).toString('base64')}\n`).toString('base64');
test('feed requires matching version, commit and intact signed assets for all platforms',()=>{
  const dir=mkdtempSync(join(tmpdir(),'tp-updates-'));
  try{
    const bundle=join(dir,'bundle'),out=join(dir,'assets');
    for(const [sub,name] of [['nsis','app.exe'],['msi','app.msi'],['macos','app.app.tar.gz']]){mkdirSync(join(bundle,sub),{recursive:true});writeFileSync(join(bundle,sub,name),'fixture payload');writeFileSync(join(bundle,sub,name+'.sig'),fixtureSig);}
    stageUpdates(bundle,out,version,commit,'windows');stageUpdates(bundle,out,version,commit,'macos-arm64');
    expect(()=>assembleFeed(out,version,commit,'Notes')).toThrow();
    stageUpdates(bundle,out,version,commit,'macos-x64');
    expect(()=>assembleFeed(out,version,'b'.repeat(40),'Notes')).toThrow('identity');
    const file=join(out,`TaskPulse_${version}_macos_x64.app.tar.gz`);writeFileSync(file,'corrupt');
    expect(()=>assembleFeed(out,version,commit,'Notes')).toThrow('mismatch');
    copyFileSync(join(bundle,'macos/app.app.tar.gz'),file);
    assembleFeed(out,version,commit,'Notes');
    const feed=JSON.parse(readFileSync(join(out,'latest.json'),'utf8'));
    expect(Object.keys(feed.platforms).sort()).toEqual(['darwin-aarch64','darwin-x86_64','windows-x86_64','windows-x86_64-msi','windows-x86_64-nsis']);
    expect(feed.version).toBe(version);expect(feed.platforms['windows-x86_64-msi'].url).toEndWith('.msi');
    expect(()=>assembleFeed(out,version,commit,'Notes')).toThrow('stale');
    expect(()=>stageUpdates(bundle,out,version,commit,'macos-x64')).toThrow('stale');
  }finally{rmSync(dir,{recursive:true,force:true});}
});
test('release signing is isolated from PR builds and all native packages gate the draft',()=>{
  const release=Bun.YAML.parse(readFileSync(join(import.meta.dir,'../.github/workflows/release.yml'),'utf8')) as any;
  expect(release.jobs.draft.needs).toContain('macos');expect(release.jobs.draft.needs).toContain('installer-smoke');
  const publish=release.jobs.draft.steps.find((s:any)=>s.uses?.startsWith('softprops/')).with;
  expect(publish.draft).toBe(true);expect(publish.files).toContain('latest.json');expect(publish.files).toContain('*.sig');
  for(const job of ['windows','macos'])expect(release.jobs[job].steps.some((s:any)=>s.run?.includes('signed_release_artifacts_match_embedded_key'))).toBe(true);
  const preview=readFileSync(join(import.meta.dir,'../.github/workflows/installation-preview.yml'),'utf8');
  expect(preview).not.toContain('secrets.');expect(preview).not.toContain('tauri.updater.conf.json');
  const config=JSON.parse(readFileSync(join(import.meta.dir,'../src-tauri/tauri.conf.json'),'utf8'));
  expect(config.plugins.updater.endpoints).toEqual(['https://github.com/qingtian-4/TaskPulse/releases/latest/download/latest.json']);
  expect(config.plugins.updater.dangerousInsecureTransportProtocol).not.toBe(true);
  expect(config.plugins.updater.allowDowngrades).not.toBe(true);
  expect(config.plugins.updater.requireSignedVersion).toBe(true);
});
