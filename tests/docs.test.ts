import {test,expect} from 'bun:test';
import {readFileSync,existsSync} from 'node:fs';
import {resolve} from 'node:path';
import {createHash} from 'node:crypto';
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
    expect(body).toContain('ZCode');expect(body).toContain('WorkBuddy');
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
  expect(manifest.data).toContain('Public synthetic');
  expect(manifest.assets).toHaveLength(10);
  for(const asset of manifest.assets){
    const bytes=readFileSync(resolve(root,'assets/screenshots',asset.file));
    expect(createHash('sha256').update(bytes).digest('hex'),asset.file).toBe(asset.sha256);
    expect(bytes.length,asset.file).toBe(asset.bytes);
    if(asset.kind==='demo'){
      expect(bytes.subarray(0,6).toString(),asset.file).toMatch(/^GIF8[79]a$/);
      expect(bytes.readUInt16LE(6),asset.file).toBe(asset.width);
      expect(bytes.readUInt16LE(8),asset.file).toBe(asset.height);
      expect(bytes.length,asset.file).toBeLessThanOrEqual(5*1024*1024);
    }else{
      expect(bytes.subarray(0,8).toString('hex'),asset.file).toBe('89504e470d0a1a0a');
      expect(bytes.readUInt32BE(16),asset.file).toBe(asset.logical_width*2);
      expect(bytes.readUInt32BE(20),asset.file).toBe(asset.logical_height*2);
    }
  }
  for(const [file,suffix] of [['README.md',''],['README.en.md','-en']]){
    const body=read(file);
    expect(body,file).toContain(`v${manifest.version}`);
    expect(body,file).toContain(`src="assets/screenshots/dashboard${suffix}.png"`);
    expect(body,file).toContain(`src="assets/screenshots/demo${suffix}.gif"`);
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
