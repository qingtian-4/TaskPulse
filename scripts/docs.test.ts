import {test,expect} from 'bun:test';
import {readFileSync,existsSync} from 'node:fs';
import {resolve} from 'node:path';
import {documentationErrors,gitFiles} from './repository-audit';
const root=resolve(import.meta.dir,'..');
const read=(name:string)=>readFileSync(resolve(root,name),'utf8');

test('public documentation links resolve using tracked files, not local reports',()=>{
  expect(documentationErrors(root,gitFiles(root))).toEqual([]);
});

test('both READMEs expose the actual one-line installation commands before source builds',()=>{
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
    expect(body).toContain('bun run tauri build');
    expect(body).toContain('sprklai/agenttray');expect(body).toContain('(LICENSE)');
    expect(body).not.toMatch(/assets\/legacy-agenttray\//);
    expect(body).not.toMatch(/docs\/(?:internal|maintainers)\//);
    expect([...body.matchAll(/^git clone .+$/gm)].map(m=>m[0])).toEqual(['git clone --branch main --single-branch https://github.com/qingtian-4/TaskPulse.git']);
  }
  expect(read('README.md').indexOf('## 从源码构建')).toBeLessThan(read('README.md').indexOf('## 贡献与许可'));
  expect(read('README.en.md').indexOf('## Build from source')).toBeLessThan(read('README.en.md').indexOf('## Contributing and license'));
  expect(read('README.md')).not.toMatch(/实验支持|测试版本|尚未公开发布|README 怎么呈现/);
  expect(read('README.en.md')).not.toMatch(/experimental|test candidate/);
  // Preserve real command contracts: the Mac bootstrap requires Bash.
  expect(read('install.sh')).toStartWith('#!/bin/bash');
  expect(read('README.md')).not.toContain('install.sh | sh');
  expect(read('README.en.md')).not.toContain('install.sh | sh');
});

test('both READMEs show four native figures at half their actual PNG resolution',()=>{
  const figures=[['dashboard',368,840],['notice-input',320,132],['notice-approval',320,132],['notice-completed',320,132]] as const;
  for(const [name,width,height] of figures){
    const path=`assets/screenshots/${name}.png`;
    const bytes=readFileSync(resolve(root,path));
    expect(bytes.subarray(0,8).toString('hex'),path).toBe('89504e470d0a1a0a');
    expect(bytes.readUInt32BE(16),path).toBe(width*2);
    expect(bytes.readUInt32BE(20),path).toBe(height*2);
    for(const file of ['README.md','README.en.md'])expect(read(file),file).toContain(`src="${path}" width="${width}"`);
  }
  expect(read('README.md')).toContain('Windows 界面 · 示例任务');
  expect(read('README.en.md')).toContain('Windows interface with sample tasks.');
});

test('repository tests use exact root paths rather than collecting nested worktrees',()=>{
  const manifest=JSON.parse(read('package.json'));
  for(const name of ['test:docs','test:ui-model','test:release','test:security','test:integration-contract']){
    const command=manifest.scripts[name] as string;
    expect(command,name).toStartWith('bun test ');
    for(const file of command.slice('bun test '.length).split(/\s+/)){
      expect(file,name).toMatch(/^\.\/scripts\/[^/]+\.test\.ts$/);
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
