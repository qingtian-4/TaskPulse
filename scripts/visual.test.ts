import {test,expect} from 'bun:test';
import {readFileSync} from 'node:fs';
const css=readFileSync('src/app.css','utf8');
test('dashboard retains the tall viewport instead of shrinking to its own content height',()=>{
  const dashboard=readFileSync('src/lib/components/Dashboard.svelte','utf8');
  expect(dashboard).toContain("style:height={'min('+maxHeight+'px, 100dvh)'}");
  expect(readFileSync('src-tauri/src/tray.rs','utf8')).toContain('.inner_size(440.0, 720.0)');
});
const hex=(token:string)=>{const value=css.match(new RegExp('--tp-'+token+': (#[0-9a-f]{6});'))?.[1];expect(value,'missing '+token).toBeDefined();return value!;};
function luminance(hex:string){const [r,g,b]=[1,3,5].map(i=>{const v=parseInt(hex.slice(i,i+2),16)/255;return v<=.04045?v/12.92:((v+.055)/1.055)**2.4;});return .2126*r+.7152*g+.0722*b;}
function contrast(a:string,b:string){const x=luminance(a),y=luminance(b);return (Math.max(x,y)+.05)/(Math.min(x,y)+.05);}
test('shared desktop theme retains readable foregrounds and distinct uncertainty colors',()=>{
  for(const surface of ['bg','panel','card','raised'])for(const text of ['text','muted','accent','running','action','stale','danger','success'])expect(contrast(hex(surface),hex(text)),surface+' / '+text).toBeGreaterThanOrEqual(4.5);
  expect(contrast(hex('accent'),hex('on-accent'))).toBeGreaterThanOrEqual(4.5);
  expect(hex('stale')).not.toBe(hex('danger'));expect(hex('action')).not.toBe(hex('running'));
});
test('shared controls retain focus, reduced motion and constrained lightweight surface variants',()=>{
  for(const rule of ['.tp-btn','.tp-chip','.tp-field','.tp-fold','.tp-surface',':focus-visible','prefers-reduced-motion','forced-colors'])expect(css).toContain(rule);
  for(const scale of ['space-1','space-6','radius-sm','radius-panel','shadow-card','glow'])expect(css).toContain('--tp-'+scale+':');
});
