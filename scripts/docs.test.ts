import {test,expect} from 'bun:test';
import {readFileSync,existsSync} from 'node:fs';
import {resolve,dirname} from 'node:path';
const documents=['README.md','README.en.md','docs/INSTALL.md','docs/AGENT_COMPATIBILITY.md','docs/PRODUCT_DASHBOARD.md','assets/README.md','docs/UI_UX_VALIDATION_2026-10-21.md','docs/DESIGN_SYSTEM.md','docs/VISUAL_POLISH_VALIDATION_2026-10-21.md'];
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
