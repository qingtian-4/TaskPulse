/** Stage only the verified Windows debug executable, never a profile or target tree. */
import {copyFileSync,existsSync,lstatSync,mkdirSync,readFileSync,writeFileSync} from 'node:fs';
import {dirname,join,resolve} from 'node:path';
import {spawnSync} from 'node:child_process';
import {sha256} from './release';

export function verifyDevelopmentBuild(build:any,commit:string,version:string) {
  if(!/^[a-f0-9]{40}$/.test(commit)) {
    throw Error('A full source SHA is required');
  }
  if(build?.channel!=='development' || build.commit!==commit ||
      build.modified!==false || build.version!==version) {
    throw Error('Stale or non-development executable does not match the committed source');
  }
}

export function stageDevelopment(executable:string,out:string,commit:string,version:string) {
  if(!/^[a-f0-9]{40}$/.test(commit))throw Error('A full source SHA is required');
  if(existsSync(out))throw Error('Development output already exists; use a fresh directory');
  const regular=(path:string)=>{const s=lstatSync(path);if(!s.isFile()||s.isSymbolicLink())throw Error('Only regular build files can be staged');};
  regular(executable);
  const bytes=readFileSync(executable);
  const pe=bytes.length>=64?bytes.readUInt32LE(60):0;
  if(bytes.subarray(0,2).toString()!=='MZ'||pe+6>bytes.length||bytes.subarray(pe,pe+4).toString()!=='PE\0\0'||bytes.readUInt16LE(pe+4)!==0x8664)throw Error('Expected a Windows x64 executable');
  const loader=join(dirname(executable),'WebView2Loader.dll');if(existsSync(loader))regular(loader);
  mkdirSync(out,{recursive:true});
  const assets=['TaskPulse.exe'];copyFileSync(executable,join(out,assets[0]));
  if(existsSync(loader)){assets.push('WebView2Loader.dll');copyFileSync(loader,join(out,'WebView2Loader.dll'));}
  writeFileSync(join(out,'development-manifest.json'),JSON.stringify({version,commit,tag:null,build_kind:'development',platform:'windows',architecture:'x64',state:'isolated; ~/.taskpulse-dev/<build-directory-id>',assets},null,2)+'\n');
  writeFileSync(join(out,'SHA256SUMS.txt'),assets.map(name=>`${sha256(readFileSync(join(out,name)))}  ${name}\n`).join(''));
  writeFileSync(join(out,'START-HERE.txt'),`TaskPulse Dev — temporary test build\nSource: ${commit}\n\nExtract the whole ZIP into a new folder, then run TaskPulse.exe. No installation is needed.\nState is separate from the installed application. Startup registration and updates are disabled.\nConfirm the read-only sources in this development profile before monitoring real tasks.\nVerify the SHA-256 values in SHA256SUMS.txt; GitHub also provides the ZIP artifact digest.\nThis artifact expires after 7 days. It is not a release or an update package.\n`);
}
if(import.meta.main){
  const root=resolve(import.meta.dir,'..'),args=process.argv.slice(2);
  const arg=(key:string)=>{const i=args.indexOf(key);if(i<0||!args[i+1])throw Error(`Missing ${key}`);return args[i+1];};
  const commit=arg('--commit');
  const git=spawnSync('git',['rev-parse','HEAD'],{cwd:root,encoding:'utf8',windowsHide:true});
  if(git.status!==0||git.stdout.trim()!==commit)throw Error('Artifact SHA does not match checkout HEAD');
  const clean=spawnSync('git',['status','--porcelain','--untracked-files=normal'],{cwd:root,encoding:'utf8',windowsHide:true});
  if(clean.status!==0||clean.stdout.trim())throw Error('Only a clean committed checkout can produce a shared artifact');
  const executable=resolve(arg('--exe'));
  const native=spawnSync(executable,['--profile-info'],{encoding:'utf8',windowsHide:true,timeout:15000});
  if(native.status!==0)throw Error('Cannot verify the built executable');
  const build=JSON.parse(native.stdout);
  verifyDevelopmentBuild(build,commit,JSON.parse(readFileSync(join(root,'package.json'),'utf8')).version);
  stageDevelopment(resolve(arg('--exe')),resolve(arg('--out')),commit,JSON.parse(readFileSync(join(root,'package.json'),'utf8')).version);
  console.log(`Staged isolated Windows development build ${commit}`);
}
