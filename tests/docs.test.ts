import {test,expect} from 'bun:test';
import {readFileSync,existsSync} from 'node:fs';
import {resolve} from 'node:path';
import {createHash} from 'node:crypto';
import {spawnSync} from 'node:child_process';
import {documentationErrors,gitFiles} from '../scripts/repository-audit';
const root=resolve(import.meta.dir,'..');
const read=(name:string)=>readFileSync(resolve(root,name),'utf8');

test('public documentation links resolve using tracked files, not local reports',()=>{
  expect(documentationErrors(root,gitFiles(root))).toEqual([]);
});

test('both READMEs expose the actual one-line installation commands before isolated development',()=>{
  const windows='irm https://raw.githubusercontent.com/qingtian-4/TaskPulse/main/install.ps1 | iex';
  const mac='curl -fsSL https://raw.githubusercontent.com/qingtian-4/TaskPulse/main/install.sh | bash';
  for(const file of ['README.md','README.en.md']){
    const body=read(file);
    expect(body).toContain(windows);expect(body).toContain(mac);
    expect(body.indexOf(windows)).toBeLessThan(body.indexOf('git clone'));
    expect(body.indexOf(mac)).toBeLessThan(body.indexOf('git clone'));
    expect(body).not.toContain('docs/CANDIDATE_TESTING.md');
    expect(body).toContain('- [ ]');
    expect(body).toContain('Codex');expect(body).toContain('Claude Code');
    expect(body).toContain('docs/AGENT_COMPATIBILITY.md');
    expect(body).not.toMatch(/1\.0\.[01x]|版本迁移|version migration|numbering scheme/i);
    expect(body).toContain('Apple Silicon');expect(body).toContain('Intel');
    expect(body).toContain('bun run dev:desktop');
    expect(body).toContain('(CONTRIBUTING.md)');
    expect(body).toContain('sprklai/agenttray');expect(body).toContain('(LICENSE)');
    expect(body).not.toMatch(/assets\/legacy-agenttray\//);
    expect(body).not.toMatch(/docs\/(?:internal|maintainers)\//);
    expect([...body.matchAll(/^git clone .+$/gm)].map(m=>m[0])).toEqual(['git clone --branch main --single-branch https://github.com/qingtian-4/TaskPulse.git']);
  }
  expect(read('README.md').indexOf('## 本地开发')).toBeLessThan(read('README.md').indexOf('## 贡献与许可'));
  expect(read('README.en.md').indexOf('## Develop locally')).toBeLessThan(read('README.en.md').indexOf('## Contributing and license'));
  expect(read('README.md')).not.toMatch(/实验支持|测试版本|尚未公开发布|README 怎么呈现/);
  expect(read('README.en.md')).not.toMatch(/experimental|test candidate/);
  // Preserve real command contracts: the Mac bootstrap requires Bash.
  expect(read('install.sh')).toStartWith('#!/bin/bash');
  expect(read('README.md')).not.toContain('install.sh | sh');
  expect(read('README.en.md')).not.toContain('install.sh | sh');
});

test('both READMEs separate installer builds from isolated development',()=>{
  for(const [file,dev,build,contribute] of [
    ['README.md','本地开发','从源码构建','贡献与许可'],
    ['README.en.md','Develop locally','Build from source','Contributing and license'],
  ]){
    const body=read(file).replaceAll('\r\n','\n');
    const devAt=body.indexOf('## '+dev+'\n');
    const buildAt=body.indexOf('## '+build+'\n');
    const contributeAt=body.indexOf('## '+contribute+'\n');
    expect(devAt,file).toBeGreaterThanOrEqual(0);
    expect(buildAt,file).toBeGreaterThan(devAt);
    expect(contributeAt,file).toBeGreaterThan(buildAt);
    expect(body.slice(devAt,buildAt),file).toContain('bun run dev:desktop');
    expect(body.slice(buildAt,contributeAt),file).toContain('bun run tauri build');
    expect(body.slice(buildAt,contributeAt),file).toContain('src-tauri/target/release/bundle/');
  }
});

test('versioned native media has intact provenance and three reminders share one responsive row',()=>{
  const manifest=JSON.parse(read('assets/screenshots/manifest.json'));
  expect(manifest.capture_commit).toMatch(/^[a-f0-9]{40}$/);
  expect(manifest.executable_sha256).toMatch(/^[a-f0-9]{64}$/);
  expect(manifest.modified).toBe(false);
  for(const [field,path] of [['frontend_tree','src'],['runtime_tree','src-tauri/src'],['locale_tree','locales']]){
    const tree=spawnSync('git',['rev-parse',`HEAD:${path}`],{cwd:root,encoding:'utf8',windowsHide:true});
    expect(tree.status).toBe(0);
    expect(manifest[field],`${path}: screenshots must match the current application source`).toBe(tree.stdout.trim());
  }
  for(const [file,expected] of [
    ['src-tauri/tauri.conf.json',manifest.application_config_sha256],
    ...Object.entries(manifest.lockfiles),
  ]){
    const bytes=readFileSync(resolve(root,file));
    expect(bytes.includes(13),`${file}: checkout must use LF for screenshot provenance`).toBe(false);
    expect(createHash('sha256').update(bytes).digest('hex'),file).toBe(expected);
  }
  expect(Object.keys(manifest.lockfiles).sort()).toEqual(['bun.lock','src-tauri/Cargo.lock']);
  expect(manifest.data).toContain('Example tasks');
  expect(manifest.version).toBe(JSON.parse(read('package.json')).version);
  expect(manifest.assets).toHaveLength(8);
  for(const asset of manifest.assets){
    const bytes=readFileSync(resolve(root,'assets/screenshots',asset.file));
    expect(createHash('sha256').update(bytes).digest('hex'),asset.file).toBe(asset.sha256);
    expect(bytes.length,asset.file).toBe(asset.bytes);
    expect(bytes.subarray(0,8).toString('hex'),asset.file).toBe('89504e470d0a1a0a');
    expect(bytes.readUInt32BE(16),asset.file).toBe(asset.logical_width*2);
    expect(bytes.readUInt32BE(20),asset.file).toBe(asset.logical_height*2);
  }
  for(const [file,suffix] of [['README.md',''],['README.en.md','-en']]){
    const body=read(file);
    expect(body,file).toContain(`v${manifest.version}`);
    expect(body,file).toContain(`src="assets/screenshots/dashboard${suffix}.png"`);
    expect(body,file).not.toMatch(/assets\/screenshots\/demo[^"\s]*\.gif|\.(?:mp4|webm)\b|<video\b/);
    const rows=[...body.matchAll(/<p\b[^>]*>[\s\S]*?<\/p>/g)].map(m=>m[0]);
    const row=rows.find(r=>r.includes(`src="assets/screenshots/notice-input${suffix}.png"`));
    expect(row,file).toBeDefined();
    const images=[...row!.matchAll(/<img\b[^>]*>/g)].map(m=>m[0]);
    expect(images,file).toHaveLength(3);
    for(const kind of ['input','approval','completed'])expect(row,file).toContain(`src="assets/screenshots/notice-${kind}${suffix}.png"`);
    const widths=images.map(img=>Number(img.match(/width="(\d+)%"/)?.[1]));
    expect(widths.every(n=>n>0),file).toBe(true);
    expect(widths.reduce((a,b)=>a+b,0),file).toBeLessThanOrEqual(100);
  }
});

test('the supplied eight-second promotion stays separate from native capture evidence',()=>{
  const manifest=JSON.parse(read('assets/promo/manifest.json'));
  const bytes=readFileSync(resolve(root,'assets/promo',manifest.file));
  expect(bytes.subarray(0,6).toString()).toMatch(/^GIF8[79]a$/);
  expect(createHash('sha256').update(bytes).digest('hex')).toBe(manifest.sha256);
  expect(bytes.length).toBe(manifest.bytes);
  expect(manifest.duration_ms).toBe(8000);
  expect(manifest.native_capture).toBe(false);
  for(const file of ['README.md','README.en.md']){
    const body=read(file);
    expect(body).toContain('src="assets/promo/teaser-8s.gif"');
    expect(Math.abs(body.indexOf('assets/promo/teaser-8s.gif')-body.indexOf('src="assets/screenshots/dashboard'))).toBeLessThan(1800);
  }
  expect(read('README.md')).toContain('assets/architecture/architecture-zh.svg');
  expect(read('README.en.md')).toContain('assets/architecture/architecture-en.svg');
});

test('repository tests use exact root paths rather than collecting nested worktrees',()=>{
  const manifest=JSON.parse(read('package.json'));
  for(const name of ['test:docs','test:ui-model','test:release','test:security','test:integration-contract']){
    const command=manifest.scripts[name] as string;
    expect(command,name).toStartWith('bun test ');
    for(const file of command.slice('bun test '.length).split(/\s+/)){
      expect(file,name).toMatch(/^\.\/tests\/[^/]+\.test\.ts$/);
      expect(existsSync(resolve(root,file)),`${name}: ${file}`).toBe(true);
    }
  }
});

test('quality and release gate documentation checks',()=>{
  for(const [file,job] of [['quality.yml','quality'],['release.yml','windows']]){
    const workflow=Bun.YAML.parse(read('.github/workflows/'+file)) as any;
    const step=workflow.jobs[job].steps.find((s:any)=>s.run==='bun run test:docs');
    expect(step,file).toBeDefined();expect(step?.['continue-on-error']).not.toBe(true);
  }
});
