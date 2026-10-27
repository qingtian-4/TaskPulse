import {expect,test} from 'bun:test';
import {readFileSync} from 'node:fs';
import {resolve} from 'node:path';

const root=resolve(import.meta.dir,'..');
const read=(path:string)=>readFileSync(resolve(root,path));

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
