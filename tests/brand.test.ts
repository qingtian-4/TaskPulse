import {expect,test} from 'bun:test';
import {readFileSync,readdirSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {resolve} from 'node:path';

const root=resolve(import.meta.dir,'..');
const read=(path:string)=>readFileSync(resolve(root,path));

test('all active SVG marks use TP-03 geometry, including both README wordmarks',()=>{
  const manifest=JSON.parse(read('assets/brand/manifest.json').toString());
  expect(manifest.version).toBe('design-handoff-2026-10-27-tp03');
  const geometry=manifest.shared_geometry;
  expect(geometry.t_path).toBe('M20 30 H36 M36 30 V74');
  expect(geometry.p_path).toBe('M36 30 H57 C73 30 79 39 79 48 C79 59 71 64 58 64 H50');
  for(const name of readdirSync(resolve(root,'assets/brand')).filter(f=>f.endsWith('.svg'))){
    const svg=read('assets/brand/'+name).toString();
    expect(svg,name).toContain(`d="${geometry.t_path}"`);
    expect(svg,name).toContain(`d="${geometry.p_path}"`);
    expect([...svg.matchAll(/stroke-width="([^"]+)"/g)].map(m=>Number(m[1])),name).toEqual([11.5,11.5]);
    expect(svg,name).not.toContain('M29 74 V30');
  }
  for(const file of ['README.md','README.en.md']){
    const readme=read(file).toString();
    for(const wordmark of Object.values(manifest.wordmarks))expect(readme).toContain('assets/brand/'+wordmark);
  }
});

test('desktop binaries match the TP-03 asset inventory without mixed legacy exports',()=>{
  const manifest=JSON.parse(read('assets/brand/manifest.json').toString());
  const expected=readdirSync(resolve(root,'src-tauri/icons')).filter(f=>/\.(png|ico|icns)$/.test(f)).map(f=>'src-tauri/icons/'+f);
  expected.push('assets/brand/social-preview.png');
  expect(Object.keys(manifest.binary_assets_sha256).sort()).toEqual(expected.sort());
  for(const [path,hash] of Object.entries(manifest.binary_assets_sha256)){
    expect(createHash('sha256').update(read(path)).digest('hex'),path).toBe(hash);
  }
});

test('macOS ICNS is complete and includes native and Retina PNG representations',()=>{
  const bytes=read('src-tauri/icons/icon.icns');
  expect(bytes.subarray(0,4).toString()).toBe('icns');
  expect(bytes.readUInt32BE(4)).toBe(bytes.length);
  const expected:Record<string,number>={ic07:128,ic08:256,ic09:512,ic10:1024,ic11:32,ic12:64,ic13:256,ic14:512};
  const found:string[]=[];
  let offset=8;
  while(offset<bytes.length){
    expect(offset+8).toBeLessThanOrEqual(bytes.length);
    const kind=bytes.subarray(offset,offset+4).toString(),size=bytes.readUInt32BE(offset+4);
    expect(size).toBeGreaterThan(8);expect(offset+size).toBeLessThanOrEqual(bytes.length);
    expect(found).not.toContain(kind);found.push(kind);
    if(expected[kind]){
      const png=bytes.subarray(offset+8,offset+size);
      expect(png.subarray(1,4).toString(),kind).toBe('PNG');
      expect([png.readUInt32BE(16),png.readUInt32BE(20)],kind).toEqual([expected[kind],expected[kind]]);
      expect(png[25],kind).toBe(6);
    }
    offset+=size;
  }
  expect(offset).toBe(bytes.length);
  for(const kind of ['is32','il32',...Object.keys(expected)])expect(found).toContain(kind);
});

test('packaged desktop ICO contains valid multiresolution 32-bit images',()=>{
  const ico=read('src-tauri/icons/icon.ico');
  expect(ico.readUInt16LE(0)).toBe(0);
  expect(ico.readUInt16LE(2)).toBe(1);
  const count=ico.readUInt16LE(4),sizes:number[]=[];
  expect(count).toBeGreaterThanOrEqual(6);
  for(let i=0;i<count;i++){
    const entry=6+i*16,width=ico[entry]||256,height=ico[entry+1]||256;
    const length=ico.readUInt32LE(entry+8),offset=ico.readUInt32LE(entry+12);
    expect(width).toBe(height);expect(ico.readUInt16LE(entry+6)).toBe(32);
    expect(offset).toBeGreaterThanOrEqual(6+count*16);expect(length).toBeGreaterThan(0);
    expect(offset+length).toBeLessThanOrEqual(ico.length);
    sizes.push(width);
  }
  for(const size of [16,24,32,48,64,256])expect(sizes).toContain(size);
});

test('UI master and installer resources use the fixed brand; tray PNGs remain embedded',()=>{
  expect(read('src/lib/assets/brand.svg')).toEqual(read('assets/brand/brand.svg'));
  const config=JSON.parse(read('src-tauri/tauri.conf.json').toString());
  expect(config.bundle.icon).toContain('icons/icon.ico');
  expect(config.bundle.windows.nsis.installerIcon).toBe('icons/icon.ico');
  expect(JSON.stringify(config.bundle.resources)).not.toContain('tray-');
  for(const path of config.bundle.icon){
    const bytes=read('src-tauri/'+path);
    expect(bytes.length).toBeGreaterThan(100);
    if(path.endsWith('.png')){
      expect(bytes.subarray(1,4).toString()).toBe('PNG');
      expect(bytes.readUInt32BE(16)).toBe(bytes.readUInt32BE(20));
      expect(bytes[25]).toBe(6); // RGBA, not opaque indexed exports.
    }
  }
  const social=read('assets/brand/social-preview.png');
  expect([social.readUInt32BE(16),social.readUInt32BE(20)]).toEqual([1280,640]);
});
