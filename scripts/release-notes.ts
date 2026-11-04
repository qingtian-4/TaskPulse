import {readFileSync,writeFileSync} from 'node:fs';
import {join} from 'node:path';
import {validateRelease} from './release';

export function releaseNotes(body:string,version:string,assets:string[]):string {
  validateRelease(`v${version}`,Array(4).fill(version));
  const packages=[
    ['Windows · x64',`TaskPulse_${version}_windows_x64_setup.exe`],
    ['Windows · ARM64',`TaskPulse_${version}_windows_arm64_setup.exe`],
    ['macOS · Apple Silicon',`TaskPulse_${version}_macos_arm64.dmg`],
    ['macOS · Intel',`TaskPulse_${version}_macos_x64.dmg`],
  ];
  const downloads=packages.map(([label,name])=>assets.includes(name)?`[${label}](https://github.com/qingtian-4/TaskPulse/releases/download/v${version}/${name})`:null).filter(Boolean);
  if(downloads.length<3)throw Error('Release notes require actual Windows and both macOS installers');
  const withoutTitle=body.replace(/^# TaskPulse v[^\r\n]+\r?\n\s*/,'').trim();
  return `# TaskPulse v${version}\n\n${downloads.join(' · ')}\n\n${downloads.length===3?'Windows ARM64 安装包待提供 / Windows ARM64 installer not yet available.\n\n':''}${withoutTitle}\n`;
}
if(import.meta.main){
  const [version,directory,source,out]=process.argv.slice(2);
  const manifest=JSON.parse(readFileSync(join(directory,'release-manifest.json'),'utf8'));
  // Legacy manifests list Windows assets only. DMGs are verified by the
  // packaging stage; use the actual staged directory for every download link.
  const {readdirSync}=await import('node:fs');
  if(manifest.version!==version)throw Error('Release note version mismatch');
  writeFileSync(out,releaseNotes(readFileSync(source,'utf8'),version,readdirSync(directory)));
}
