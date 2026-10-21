import {test,expect} from 'bun:test';
import {readFileSync,existsSync} from 'node:fs';
import {resolve,dirname} from 'node:path';
const documents=['README.md','README.en.md','docs/INSTALL.md','docs/AGENT_COMPATIBILITY.md','docs/PRODUCT_DASHBOARD.md','assets/README.md','docs/UI_UX_VALIDATION_2026-10-21.md','docs/DESIGN_SYSTEM.md','docs/VISUAL_POLISH_VALIDATION_2026-10-21.md','docs/ALPHA7_CANDIDATE_STATUS.md','docs/ALPHA7_CANDIDATE_VALIDATION_2026-10-22.md','docs/RELEASING.md','docs/OPEN_SOURCE_CHECKLIST.md'];
test('product docs use resolvable local links and images',()=>{
  for(const path of documents){
    const body=readFileSync(path,'utf8');
    for(const match of body.matchAll(/\]\(([^\s)]+)\)/g)){
      const link=match[1];if(/^(https?:|mailto:)/.test(link))continue;
      const [file,anchor]=link.split('#');const target=file?resolve(dirname(path),file):resolve(path);
      expect(existsSync(target),`${path}: ${link}`).toBe(true);
      if(anchor&&target.endsWith('.md')){
        const headings=[...readFileSync(target,'utf8').matchAll(/^#{1,6}\s+(.+)$/gm)].map(m=>m[1].toLowerCase().replace(/[`*]/g,'').replace(/[^\p{L}\p{N}\s_-]/gu,'').replace(/\s+/g,'-'));
        expect(headings,`${path}: #${anchor}`).toContain(decodeURIComponent(anchor));
      }
    }
  }
});
test('bilingual landing pages lead with the product and disclose alpha availability and evidence',()=>{
  const cn=readFileSync('README.md','utf8'),en=readFileSync('README.en.md','utf8');
  expect(en.split('##')[0]).toContain('Less window switching');
  expect(en.split('##')[0]).not.toContain('v1 protocol');
  expect(cn).toContain('尚无公开可下载的 Alpha.7 安装包');
  expect(en).toContain('No Alpha.7 installer is publicly available');
  for(const body of [cn,en]){
    expect(body).toContain('0.2.0-alpha.7');expect(body).toContain('fixture-tested');
    expect(body).toContain('sprklai/agenttray');expect(body).toContain('(LICENSE)');
    expect(body).not.toMatch(/!\[[^\]]*\]\([^)]*(agenttray-screenshot|agenttray-demo|taskpulse-overview)/);
  }
});

test('current product materials retain dimensions and explicit synthetic native provenance',()=>{
  for(const [name,width,height] of [['dashboard-alpha7',440,720],['sources-alpha7',1040,900],['notice-alpha7',320,132]] as const){
    const path=`assets/screenshots/${name}.png`,png=readFileSync(path);
    expect(png.subarray(1,4).toString()).toBe('PNG');
    expect(png.readUInt32BE(16)).toBe(width);expect(png.readUInt32BE(20)).toBe(height);
    for(const doc of ['README.md','README.en.md'])expect(readFileSync(doc,'utf8')).toContain(path);
  }
  expect(readFileSync('assets/README.md','utf8')).toContain('合成');
  expect(readFileSync('README.en.md','utf8')).toContain('synthetic standard events');
});

test('post-merge source instructions select main instead of the old candidate branch',()=>{
  for(const file of ['README.md','README.en.md']){
    const body=readFileSync(file,'utf8');
    const commands=[...body.matchAll(/^git clone .+$/gm)].map(match=>match[0].trim());
    expect(commands,file).toEqual(['git clone --branch main --single-branch https://github.com/qingtian-4/TaskPulse.git']);
    expect(body,file).not.toContain('尚未合入默认');
    expect(body,file).not.toContain('still on the candidate branch');
  }
  const install=readFileSync('docs/INSTALL.md','utf8');
  expect(install).toContain('`--branch main`');
  expect(install).not.toContain('--branch codex/desktop-discovery-visibility');
  expect(install).not.toContain('尚未合入 `main`');
});

test('candidate status separates verified alpha7 installers from deferred host acceptance',()=>{
  const status=readFileSync('docs/ALPHA7_CANDIDATE_STATUS.md','utf8');
  expect(status).toContain('PR #8 已合并');
  expect(status).toContain('ea21c85fc6b81992de72cf45729cfe6abcc1c15f');
  expect(status).toContain('PR #9 已合并');
  expect(status).toContain('b5dc64a760bae916c9d40b147616f9720405887b');
  expect(status).toContain('39633895368');
  expect(status).toContain('39633895341');
  expect(status).toContain('39636404358');
  expect(status).toContain('419811221');
  expect(status).not.toContain('仍 open，未合并');
  expect(status).not.toContain('安装器与实机验收仍未完成');
  expect(status).not.toContain('尚未生成');
  expect(status).toContain('暂缓，不计为通过');
  expect(status).toContain('维护者单独确认');
});

test('release decision documents retain draft status and separate deferred manual gates',()=>{
  for(const path of ['docs/ALPHA7_CANDIDATE_STATUS.md','docs/RELEASING.md','docs/OPEN_SOURCE_CHECKLIST.md']){
    const body=readFileSync(path,'utf8');
    expect(body,path).toContain('ALPHA7_CANDIDATE_VALIDATION_2026-10-22.md');
    expect(body,path).toContain('39636404358');
    expect(body,path).toContain('Draft');
    expect(body,path).toContain('暂缓，不计为通过');
    expect(body,path).toContain('未公开');
    expect(body,path).not.toContain('尚未创建');
    expect(body,path).not.toContain('未发现 `v0.2.0-alpha.7`');
  }
  const releasing=readFileSync('docs/RELEASING.md','utf8');
  expect(releasing).toContain('- [x] Alpha.7 准确候选提交');
  expect(releasing).toContain('- [ ] 干净 Windows 10/11 VM');
  expect(releasing).toContain('- [ ] 真 Agent 新任务');
  expect(releasing).toContain('- [ ] 记录系统/Agent 版本');
});

test('repository test scripts select exact root files rather than nested worktree filters',()=>{
  const manifest=JSON.parse(readFileSync('package.json','utf8'));
  expect(manifest.scripts['test:docs']).toBe('bun test ./scripts/docs.test.ts');
  for(const name of ['test:docs','test:ui-model','test:release','test:security','test:integration-contract']){
    const command=manifest.scripts[name] as string;
    expect(command,name).toStartWith('bun test ');
    const files=command.slice('bun test '.length).split(/\s+/);
    for(const file of files){
      expect(file,name).toMatch(/^\.\/scripts\/[^/]+\.test\.ts$/);
      expect(existsSync(file),`${name}: ${file}`).toBe(true);
    }
  }
});

test('Windows quality and release actually gate on product documentation regressions',()=>{
  for(const [file,job] of [['quality.yml','quality'],['release.yml','windows']]){
    const workflow=Bun.YAML.parse(readFileSync('.github/workflows/'+file,'utf8')) as any;
    const step=workflow.jobs[job].steps.find((s:any)=>s.run==='bun run test:docs');
    expect(step,file).toBeDefined();expect(step?.['continue-on-error']).not.toBe(true);
  }
});
