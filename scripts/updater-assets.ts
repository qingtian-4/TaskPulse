// Stage only assets produced by this build. Signature verification runs with the
// official minisign verifier in signed_release_artifacts_match_embedded_key.
import {copyFileSync,existsSync,mkdirSync,readFileSync,readdirSync,writeFileSync} from 'node:fs';
import {basename,join,resolve} from 'node:path';
import {createHash} from 'node:crypto';
import {installerNames,validateRelease} from './release';
type Entry={url:string;signature:string;sha256:string};
type Manifest={version:string;commit:string;platforms:Record<string,Entry>};
const hash=(file:string)=>createHash('sha256').update(readFileSync(file)).digest('hex');
function signature(file:string,version:string):string {
  const text=readFileSync(file,'utf8').trim();
  if(!/^[A-Za-z0-9+/]+={0,2}$/.test(text))throw Error('Invalid signature encoding');
  const lines=Buffer.from(text,'base64').toString('utf8').trim().split(/\r?\n/);
  if(lines.length!==4 || !lines[0].startsWith('untrusted comment: ') || !lines[2].startsWith('trusted comment: ') || Buffer.from(lines[1],'base64').length!==74 || Buffer.from(lines[3],'base64').length!==64)throw Error('Invalid minisign signature');
  if(lines[2].slice(17).split('\t').find(field=>field.startsWith('version:'))!==`version:${version}`)throw Error('Signature must bind the release version');
  return text;
}
function only(directory:string,extension:string):string {
  const files=readdirSync(directory).filter(n=>n.endsWith(extension));
  if(files.length!==1)throw Error(`Expected exactly one ${extension} asset`);
  return join(directory,files[0]);
}
export function stageUpdates(bundle:string,out:string,version:string,commit:string,platform:string):Manifest {
  validateRelease(`v${version}`,Array(4).fill(version));
  if(!/^[a-f0-9]{40}$/.test(commit))throw Error('Exact commit required');
  const pairs:Array<[string,string,string]>=platform==='windows' ? [
    ['windows-x86_64-nsis',only(join(bundle,'nsis'),'.exe'),installerNames(version)[0]],
    ['windows-x86_64-msi',only(join(bundle,'msi'),'.msi'),installerNames(version)[1]],
  ] : platform==='macos-arm64' || platform==='macos-x64' ? [[
    platform==='macos-arm64'?'darwin-aarch64':'darwin-x86_64',only(join(bundle,'macos'),'.app.tar.gz'),`TaskPulse_${version}_${platform.replace('-','_')}.app.tar.gz`,
  ]] : (()=>{throw Error('Unsupported update platform');})();
  mkdirSync(out,{recursive:true});
  const manifest:Manifest={version,commit,platforms:{}};
  for(const [target,original,name] of pairs){
    const sig=signature(original+'.sig',version);
    const destination=join(out,name);
    if(existsSync(destination) && hash(destination)!==hash(original))throw Error('Staged installer differs from signed payload');
    if(existsSync(destination+'.sig'))throw Error('Refusing stale signature');
    if(!existsSync(destination))copyFileSync(original,destination);
    copyFileSync(original+'.sig',destination+'.sig');
    manifest.platforms[target]={url:`https://github.com/qingtian-4/TaskPulse/releases/download/v${version}/${name}`,signature:sig,sha256:hash(destination)};
  }
  if(platform==='windows')manifest.platforms['windows-x86_64']=manifest.platforms['windows-x86_64-nsis'];
  const path=join(out,`updater-${platform}.json`);if(existsSync(path))throw Error('Refusing stale update manifest');
  writeFileSync(path,JSON.stringify(manifest,null,2)+'\n');return manifest;
}
export function assembleFeed(directory:string,version:string,commit:string,notes:string):void {
  validateRelease(`v${version}`,Array(4).fill(version));
  const platforms:Record<string,Entry>={};
  for(const platform of ['windows','macos-arm64','macos-x64']){
    const manifest=JSON.parse(readFileSync(join(directory,`updater-${platform}.json`),'utf8')) as Manifest;
    if(manifest.version!==version || manifest.commit!==commit)throw Error('Platform build identity mismatch');
    const expected=platform==='windows'?['windows-x86_64','windows-x86_64-nsis','windows-x86_64-msi']:[platform==='macos-arm64'?'darwin-aarch64':'darwin-x86_64'];
    if(Object.keys(manifest.platforms).sort().join(',')!==expected.sort().join(','))throw Error('Incomplete platform coverage');
    for(const [target,entry] of Object.entries(manifest.platforms)){
      const url=new URL(entry.url),name=basename(url.pathname);
      if(url.href!==`https://github.com/qingtian-4/TaskPulse/releases/download/v${version}/${name}` || !name.startsWith(`TaskPulse_${version}_`))throw Error('Unexpected artifact URL');
      if(hash(join(directory,name))!==entry.sha256 || signature(join(directory,name+'.sig'),version)!==entry.signature)throw Error('Asset or signature mismatch');
      platforms[target]=entry;
    }
  }
  const feed=join(directory,'latest.json');if(existsSync(feed))throw Error('Refusing stale update feed');
  writeFileSync(feed,JSON.stringify({version,notes,pub_date:new Date().toISOString(),platforms:Object.fromEntries(Object.entries(platforms).map(([key,{url,signature}])=>[key,{url,signature}]))},null,2)+'\n');
}
if(import.meta.main){
  const [command,...args]=process.argv.slice(2);const arg=(name:string)=>{const i=args.indexOf(name);if(i<0 || !args[i+1])throw Error(`Missing ${name}`);return args[i+1];};
  if(command==='stage')stageUpdates(resolve(arg('--bundle-root')),resolve(arg('--out')),arg('--version'),arg('--commit'),arg('--platform'));
  else if(command==='feed')assembleFeed(resolve(arg('--out')),arg('--version'),arg('--commit'),readFileSync(resolve(arg('--notes')),'utf8'));
  else throw Error('Usage: stage or feed with exact version, commit and paths');
}
