import {expect,test} from 'bun:test';
import {mkdtempSync,mkdirSync,readFileSync,rmSync,statSync,writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {spawnSync} from 'node:child_process';
import {documentationErrors,gitFiles,headingIds} from './repository-audit';
const root=resolve(import.meta.dir,'..');

test('all tracked Markdown navigation and SVG references resolve offline',()=>{
  expect(documentationErrors(root,gitFiles(root))).toEqual([]);
});

test('audit catches moved docs, broken anchors, SVG assets and repository escapes',()=>{
  const dir=mkdtempSync(join(tmpdir(),'taskpulse-doc-links-'));
  try{
    mkdirSync(join(dir,'docs'));
    writeFileSync(join(dir,'README.md'),'# Product\n[ok](docs/guide.md#hello-1)\n[bad](old.md)\n[bad anchor](docs/guide.md#gone)\n[escape](../outside.md)\n[local only](untracked.md)\n```md\n[example](not-a-real-file.md)\n```\n');
    writeFileSync(join(dir,'untracked.md'),'# Local only');
    writeFileSync(join(dir,'docs/guide.md'),'# Hello\n# Hello\n');
    writeFileSync(join(dir,'diagram.svg'),'<svg><defs><marker id="arrow"/></defs><path marker-end="url(#arrow)"/><image href="missing.png"/></svg>');
    const errors=documentationErrors(dir,['README.md','docs/guide.md','diagram.svg']);
    expect(errors).toHaveLength(5);
    expect(errors.join('\n')).toContain('missing old.md');
    expect(errors.join('\n')).toContain('missing anchor docs/guide.md#gone');
    expect(errors.join('\n')).toContain('link escapes repository');
    expect(errors.join('\n')).toContain('missing missing.png');
    expect(errors.join('\n')).toContain('target is not tracked untracked.md');
    expect(headingIds('# 中文标题\n# 中文标题')).toEqual(new Set(['中文标题','中文标题-1']));
  }finally{rmSync(dir,{recursive:true,force:true});}
});

test('tracked files contain no ignored outputs or large video exports',()=>{
  const ignored=spawnSync('git',['ls-files','-ci','--exclude-standard','-z'],{cwd:root,encoding:'utf8',windowsHide:true});
  expect(ignored.error).toBeUndefined();expect(ignored.status).toBe(0);expect(ignored.stdout).toBe('');
  const disposable=gitFiles(root).filter(f=>/\.(?:mp4|webm|mov|avi|mkv|dmp|mdmp|profraw|heapsnapshot|log)$/i.test(f)||/(?:^|\/)(?:node_modules|target|recordings-local|test-results|__pycache__)\//.test(f));
  expect(disposable).toEqual([]);
  for(const file of gitFiles(root).filter(f=>f.startsWith('assets/')&&f.endsWith('.gif'))){
    expect(statSync(join(root,file)).size,file).toBeLessThanOrEqual(5*1024*1024);
  }
});

test('quality and release gate the inventory; development-target PRs get quality/security',()=>{
  for(const [file,job] of [['quality.yml','quality'],['release.yml','windows']] as const){
    const workflow=Bun.YAML.parse(readFileSync(join(root,'.github/workflows',file),'utf8')) as any;
    const step=workflow.jobs[job].steps.find((s:any)=>s.run==='bun run test:repo');
    expect(step,file).toBeDefined();expect(step?.['continue-on-error']).not.toBe(true);
  }
  for(const file of ['quality.yml','security.yml']){
    const workflow=Bun.YAML.parse(readFileSync(join(root,'.github/workflows',file),'utf8')) as any;
    expect(workflow.on.pull_request.branches,file).toContain('codex/**');
    expect(workflow.permissions.contents,file).toBe('read');
  }
});

test('local reports, captures, credentials and build files cannot enter the public tree',()=>{
  const files=gitFiles(root);
  const forbidden=files.filter(f=>/(?:^|\/)(?:\.claude|\.agents|\.codex|node_modules|target|dist|build|\.tmp)\//.test(f) ||
    /(?:^|\/)(?:CLAUDE\.md|skills-lock\.json)$/.test(f) ||
    /^docs\/(?:internal|maintainers|proposals)\//.test(f) ||
    /^docs\/[^/]*(?:VALIDATION|_STATUS|_FIXES_|_FIX_|_PLAN|TODO|PROPOSAL)[^/]*\.md$/.test(f) ||
    /^assets\/legacy-agenttray\//.test(f) ||
    /\.(?:exe|dll|pdb|rlib|rmeta|o|obj|zip|tar|tgz|7z|pem|key|p12|pfx|sqlite|sqlite3|db|dump|tfstate)$/.test(f) ||
    /(?:^|\/)\.env(?:$|\.)/.test(f) && !/\.env\.(?:example|template)$/.test(f));
  expect(forbidden).toEqual([]);
  const candidates=['docs/session-notes.md','docs/maintainers/review.md','assets/screenshots/raw.png',
    'assets/legacy-agenttray/demo.gif','NOTES.md','.agents/skills/tool/SKILL.md','skills-lock.json',
    'CLAUDE.md','.claude/commands/private.md','.env.local','session.sqlite','debug.exe','output.zip'];
  const ignored=spawnSync('git',['check-ignore','--no-index','--stdin'],{cwd:root,input:candidates.join('\n')+'\n',encoding:'utf8',windowsHide:true});
  expect(ignored.status).toBe(0);
  expect(ignored.stdout.trim().split(/\r?\n/).sort()).toEqual([...candidates].sort());
  const publicFiles=['README.md','README.en.md','AGENTS.md','LICENSE','docs/INSTALL.md','docs/README.md',
    'assets/brand/brand.svg','src-tauri/icons/icon.ico','package.json','bun.lock','src-tauri/Cargo.lock'];
  const allowed=spawnSync('git',['check-ignore','--no-index','--stdin'],{cwd:root,input:publicFiles.join('\n')+'\n',encoding:'utf8',windowsHide:true});
  expect(allowed.status).toBe(1);expect(allowed.stdout).toBe('');
});
