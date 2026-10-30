/** Build-time license inventory. Static upstream originals are checksum-pinned;
 * generated reports belong to the ignored output paths, never Git. */
import {createHash} from 'node:crypto';
import {existsSync,mkdirSync,readFileSync,readdirSync,realpathSync,statSync,writeFileSync} from 'node:fs';
import {dirname,isAbsolute,join,relative,resolve,sep} from 'node:path';
import {execFileSync} from 'node:child_process';

type Metadata={name:string;version:string;license?:string;license_file?:string;repository?:unknown;gitHead?:string};
type CargoPackage=Metadata&{id:string;source:string|null;manifest_path:string};
export type CargoMetadata={packages:CargoPackage[];resolve:{root:string|null;nodes:Array<{id:string;deps:Array<{pkg:string}>}>};workspace_default_members?:string[]};
type Source={path:string;url:string;sha256:string;kind:'license'|'notice'|'upstream-declaration'|'canonical-terms';sourceSha256?:string;extract?:{kind:string}};
type Rule={ecosystem:'cargo'|'npm';name:string;version:string;license:string;repository:string;sourceRepository?:string;sourceCommit:string;pathInVcs?:string;integrity?:string;sources:string[];canonicalTerms?:string[]};
export type SourceManifest={format:string;canonicalSource?:{repository:string;commit:string};sources:Record<string,Source>;packages:Rule[]};
export type LicenseText={file:string;text:string;kind:Source['kind'];provenance:{origin:string;url?:string;sha256:string;sourceSha256?:string}};
type NoticePackage={ecosystem:string;name:string;version:string;license:string|null;repository:string|null;texts:LicenseText[];coverage:'package-text'|'upstream-text'|'declaration-and-canonical-terms'|'unresolved';review:string[]};
const hash=(data:Uint8Array)=>createHash('sha256').update(data).digest('hex');
const fileHash=(path:string)=>hash(readFileSync(path));
const label=(path:string)=>path.split(sep).join('/');
const sourceProgram=/\.(?:rs|ts|js|py|c|cpp|h)$/i;

export function repositoryIdentity(value:unknown):string|null {
  let raw=typeof value==='string'?value:typeof (value as {url?:unknown})?.url==='string'?(value as {url:string}).url:'';
  raw=raw.replace(/^git\+/,'').replace(/^git:\/\//,'https://').replace(/^git@([^:]+):/,'https://$1/');
  if(/^[\w.-]+\/[\w.-]+$/.test(raw))raw='https://github.com/'+raw;
  try {const url=new URL(raw);if(url.username||url.password||!['https:','http:'].includes(url.protocol))return null;return `${url.hostname}${url.pathname.replace(/\.git\/?$/,'').replace(/\/$/,'')}`.toLowerCase();}
  catch{return null;}
}

/** A symlink or license_file cannot read outside its package. */
export function packageLicenseFile(directory:string,path:string):string|null {
  if(!existsSync(path))return null;
  const root=realpathSync(directory),file=realpathSync(path),rel=relative(root,file);
  return !isAbsolute(rel)&&rel!=='..'&&!rel.startsWith('..'+sep)&&statSync(file).isFile()?file:null;
}

export function localLicenseTexts(metadata:Metadata,directory:string):LicenseText[] {
  const paths=new Map<string,string>();
  for(const name of readdirSync(directory)){
    if(/^(?:licen[cs]e|copying|notice)(?:[._-]|$)|^third[-_ ]party[-_ ](?:licen[cs]es?|notices?)(?:[._-]|$)/i.test(name)){
      if(sourceProgram.test(name))continue;
      const file=packageLicenseFile(directory,join(directory,name));if(file&&!sourceProgram.test(file))paths.set(file,name);
    }
  }
  if(metadata.license_file&&!sourceProgram.test(metadata.license_file)){
    const file=packageLicenseFile(directory,resolve(directory,metadata.license_file));
    if(file&&!sourceProgram.test(file))paths.set(file,label(relative(realpathSync(directory),file)));
  }
  return [...paths].flatMap(([file,name])=>{
    const bytes=readFileSync(file);if(bytes.includes(0)||bytes.length>8*1024*1024)return [];
    return [{file:name,text:bytes.toString('utf8'),kind:/^notice|^third[-_ ]party/i.test(name)?'notice' as const:'license' as const,provenance:{origin:'installed-package',sha256:hash(bytes)}}];
  });
}

/** Only known native wrappers share their exact-version parent's license. */
export function wrapperParent(metadata:Metadata,root:string):{metadata:Metadata;directory:string}|null {
  const native='(?:win32-(?:x64|arm64|ia32)-msvc|darwin-(?:x64|arm64)|linux-(?:x64|arm64)-(?:gnu|musl)|freebsd-x64)';
  const family=new RegExp('^@rolldown/binding-'+native+'$').test(metadata.name)
    ? {name:'rolldown',repo:'github.com/rolldown/rolldown'}
    : new RegExp('^@tauri-apps/cli-'+native+'$').test(metadata.name)?{name:'@tauri-apps/cli',repo:'github.com/tauri-apps/tauri'}:null;
  if(!family||repositoryIdentity(metadata.repository)!==family.repo)return null;
  const directory=join(root,'node_modules',family.name),manifest=join(directory,'package.json');if(!existsSync(manifest))return null;
  const parent=JSON.parse(readFileSync(manifest,'utf8')) as Metadata;
  return parent.name===family.name&&parent.version===metadata.version&&repositoryIdentity(parent.repository)===family.repo?{metadata:parent,directory}:null;
}

/** --filter-platform filters edges, not packages. */
export function reachableCargoPackages(metadata:CargoMetadata):CargoPackage[] {
  const roots=metadata.resolve?.root?[metadata.resolve.root]:metadata.workspace_default_members??[];
  if(!roots.length||!Array.isArray(metadata.resolve?.nodes))throw Error('Cargo metadata lacks the target-filtered resolve graph');
  const nodes=new Map(metadata.resolve.nodes.map(n=>[n.id,n]));const seen=new Set<string>(),queue=[...roots];
  for(const id of queue){
    if(seen.has(id))continue;seen.add(id);const node=nodes.get(id);
    if(!node||!Array.isArray(node.deps))throw Error('Incomplete Cargo dependency graph');
    for(const dep of node.deps)queue.push(dep.pkg);
  }
  const packages=metadata.packages.filter(p=>seen.has(p.id));if(packages.length!==seen.size)throw Error('Missing reachable Cargo package metadata');
  return packages;
}

export function targetTriple(requested:string|undefined,environment:string|undefined,host:()=>string):string {
  const target=requested||environment||host().match(/^host:\s*(\S+)$/m)?.[1];
  if(!target||!/^[a-z0-9][a-z0-9_-]+$/.test(target))throw Error('Expected a Rust target triple');return target;
}

/** A source zip nested in another checkout cannot inherit that outer SHA. */
export function sourceCommit(root:string):string|null {
  if(!existsSync(join(root,'.git')))return null;
  try {
    const top=execFileSync('git',['-C',root,'rev-parse','--show-toplevel'],{encoding:'utf8',stdio:['ignore','pipe','ignore']}).trim();
    const a=statSync(top,{bigint:true}),b=statSync(root,{bigint:true});
    if(a.dev!==b.dev||a.ino!==b.ino)return null;
    const sha=execFileSync('git',['-C',root,'rev-parse','HEAD'],{encoding:'utf8',stdio:['ignore','pipe','ignore']}).trim();return /^[a-f0-9]{40}$/.test(sha)?sha:null;
  }catch{return null;}
}

function lockedNpmIntegrity(root:string,metadata:Metadata,integrity:string):boolean {
  const lock=Bun.JSONC.parse(readFileSync(join(root,'bun.lock'),'utf8')) as {packages:Record<string,unknown>};
  return Object.values(lock.packages??{}).some(value=>Array.isArray(value)&&value[0]===`${metadata.name}@${metadata.version}`&&value.at(-1)===integrity);
}

export function matchingRule(ecosystem:'cargo'|'npm',metadata:Metadata,directory:string,root:string,manifest:SourceManifest):Rule|undefined {
  const candidate=manifest.packages.find(rule=>rule.ecosystem===ecosystem&&rule.name===metadata.name&&rule.version===metadata.version&&rule.license===metadata.license&&repositoryIdentity(rule.repository)===repositoryIdentity(metadata.repository));
  if(!candidate)return undefined;
  if(ecosystem==='cargo'){
    const file=join(directory,'.cargo_vcs_info.json');if(!existsSync(file))return undefined;
    const vcs=JSON.parse(readFileSync(file,'utf8'));
    if(vcs.git?.sha1!==candidate.sourceCommit||vcs.path_in_vcs!==candidate.pathInVcs)return undefined;
  }else if(!candidate.integrity||!lockedNpmIntegrity(root,metadata,candidate.integrity)||metadata.gitHead&&metadata.gitHead!==candidate.sourceCommit)return undefined;
  return candidate;
}

export function pinnedLicenseText(root:string,key:string,manifest:SourceManifest):LicenseText {
  const source=manifest.sources[key];if(!source)throw Error('Missing pinned license source: '+key);
  const directory=join(root,'licenses'),path=packageLicenseFile(directory,resolve(directory,source.path));
  const url=new URL(source.url);
  if(!path||url.protocol!=='https:'||url.hostname!=='raw.githubusercontent.com'||!/^\/[^/]+\/[^/]+\/[a-f0-9]{40}\//.test(url.pathname)||url.username||url.password||url.search||url.hash)throw Error('Invalid pinned license source: '+key);
  const bytes=readFileSync(path);if(!/^[a-f0-9]{64}$/.test(source.sha256)||hash(bytes)!==source.sha256)throw Error('Pinned license checksum mismatch: '+key);
  return {file:source.path,text:bytes.toString('utf8'),kind:source.kind,provenance:{origin:'pinned-upstream',url:source.url,sha256:source.sha256,...(source.sourceSha256?{sourceSha256:source.sourceSha256}:{})}};
}

export function collectNotices(root:string,cargo:CargoMetadata,manifest:SourceManifest):NoticePackage[] {
  if(manifest.format!=='taskpulse-license-sources-v1')throw Error('Unknown license source manifest format');
  const packages:NoticePackage[]=[];const seen=new Set<string>();
  function add(ecosystem:'cargo'|'npm',metadata:Metadata,directory:string){
    metadata={...metadata,license:typeof metadata.license==='string'?metadata.license:undefined};
    const key=`${ecosystem}:${metadata.name}@${metadata.version}`;if(seen.has(key))return;seen.add(key);
    let texts=localLicenseTexts(metadata,directory),coverage:NoticePackage['coverage']='unresolved';const review:string[]=[];
    if(texts.some(t=>t.kind==='license'))coverage='package-text';
    if(coverage==='unresolved'&&ecosystem==='npm'){
      const parent=wrapperParent(metadata,root);
      if(parent){
        texts.push(...localLicenseTexts(parent.metadata,parent.directory).map(text=>({...text,file:parent.metadata.name+'/'+text.file,provenance:{...text.provenance,origin:'exact-version-wrapper-parent'}})));
        if(texts.some(t=>t.kind==='license'))coverage='package-text';
      }
    }
    if(coverage==='unresolved'){
      const rule=matchingRule(ecosystem,metadata,directory,root,manifest);
      if(rule){
        for(const key of rule.sources){
          const source=manifest.sources[key],url=source&&new URL(source.url);
          if(!url||url.pathname.split('/')[3]!==rule.sourceCommit||repositoryIdentity('https://github.com/'+url.pathname.split('/').slice(1,3).join('/'))!==repositoryIdentity(rule.sourceRepository??rule.repository))throw Error('License source is not tied to the matched package revision: '+key);
        }
        for(const key of rule.canonicalTerms??[]){
          const source=manifest.sources[key],url=source&&new URL(source.url),identifier=key.replace(/^spdx:/,'');
          if(!source||source.kind!=='canonical-terms'||!manifest.canonicalSource||url?.pathname.split('/')[3]!==manifest.canonicalSource.commit||repositoryIdentity('https://github.com/'+url.pathname.split('/').slice(1,3).join('/'))!==repositoryIdentity(manifest.canonicalSource.repository)||!rule.license.split(/\s+|[()/]/).includes(identifier))throw Error('Canonical terms do not match the declared license: '+key);
        }
        texts.push(...rule.sources.map(key=>pinnedLicenseText(root,key,manifest)),...(rule.canonicalTerms??[]).map(key=>pinnedLicenseText(root,key,manifest)));
        if(texts.some(t=>t.kind==='license'))coverage='upstream-text';
        else if(texts.some(t=>t.kind==='canonical-terms')){
          coverage='declaration-and-canonical-terms';
          review.push('Upstream declares the license but omits a standalone full license; canonical terms supplement the original notice without inventing copyright attribution.');
        }
      }
    }
    if(!metadata.license)review.push('Missing declared license');
    if(coverage==='unresolved')review.push('No verified license text for this exact package/source version');
    packages.push({ecosystem,name:metadata.name,version:metadata.version,license:metadata.license??null,repository:repositoryIdentity(metadata.repository),texts,coverage,review});
  }
  for(const metadata of reachableCargoPackages(cargo))if(metadata.source)add('cargo',metadata,dirname(metadata.manifest_path));
  const visited=new Set<string>();
  function nodeModules(directory:string){
    if(!existsSync(directory))return;
    const canonical=realpathSync(directory);if(visited.has(canonical))return;visited.add(canonical);
    for(const entry of readdirSync(directory)){
      if(entry.startsWith('.'))continue;const path=join(directory,entry);
      if(entry.startsWith('@')){for(const name of readdirSync(path))nodePackage(join(path,name));}else nodePackage(path);
    }
  }
  function nodePackage(directory:string){
    const file=join(directory,'package.json');if(!existsSync(file))return;
    add('npm',JSON.parse(readFileSync(file,'utf8')),directory);nodeModules(join(directory,'node_modules'));
  }
  nodeModules(join(root,'node_modules'));
  const project=JSON.parse(readFileSync(join(root,'package.json'),'utf8')) as Metadata;
  const bytes=readFileSync(join(root,'LICENSE'));
  packages.push({ecosystem:'application',name:'TaskPulse',version:project.version,license:typeof project.license==='string'?project.license:null,repository:repositoryIdentity(project.repository),texts:[{file:'LICENSE',text:bytes.toString('utf8'),kind:'license',provenance:{origin:'project',sha256:hash(bytes)}}],coverage:'package-text',review:[]});
  return packages.sort((a,b)=>`${a.ecosystem}:${a.name}:${a.version}`.localeCompare(`${b.ecosystem}:${b.name}:${b.version}`));
}

export function writeNotices(root:string,output:string,target:string,cargo:CargoMetadata){
  const manifest=JSON.parse(readFileSync(join(root,'licenses/sources.json'),'utf8')) as SourceManifest;
  const packages=collectNotices(root,cargo,manifest),commit=sourceCommit(root);
  const reviewRequired=packages.filter(p=>p.review.length).map(p=>`${p.ecosystem}:${p.name}@${p.version}`);
  const scope='Target-filtered reachable Cargo graph (including build/test dependencies) and installed frontend/build packages; OS frameworks and native dependencies embedded in third-party build-tool binaries are not independently enumerated.';
  const project=JSON.parse(readFileSync(join(root,'package.json'),'utf8'));
  const locks={'bun.lock':fileHash(join(root,'bun.lock')),'Cargo.lock':fileHash(join(root,'src-tauri/Cargo.lock'))};
  mkdirSync(output,{recursive:true});
  writeFileSync(join(output,'DEPENDENCIES.json'),JSON.stringify({format:'taskpulse-build-inventory-v2',commit,sourceProvenance:commit?'git-checkout':'unavailable-source-archive',target,scope,lockfiles:locks,reviewRequired,packages:packages.map(({texts,...p})=>({...p,licenseFiles:texts.map(t=>({file:t.file,kind:t.kind,provenance:t.provenance}))}))},null,2)+'\n');
  writeFileSync(join(output,'SBOM.cdx.json'),JSON.stringify({bomFormat:'CycloneDX',specVersion:'1.6',version:1,metadata:{component:{type:'application',name:'TaskPulse',version:project.version},properties:[{name:'taskpulse:source-commit',value:commit??'unavailable'},{name:'taskpulse:target',value:target},{name:'taskpulse:bun-lock-sha256',value:locks['bun.lock']},{name:'taskpulse:cargo-lock-sha256',value:locks['Cargo.lock']},{name:'taskpulse:scope',value:scope}]},components:packages.filter(p=>p.ecosystem!=='application').map(p=>({type:'library','bom-ref':`${p.ecosystem}:${p.name}@${p.version}`,name:p.name,version:p.version,...(p.license?{licenses:[{license:{name:p.license}}]}:{})}))},null,2)+'\n');
  writeFileSync(join(output,'THIRD_PARTY_NOTICES.txt'),'TaskPulse license and dependency notices\nSource: '+(commit??'unavailable (source archive)')+'\nTarget: '+target+'\nScope: '+scope+'\nDeclared license expressions remain unchanged. Review items are retained in DEPENDENCIES.json. Canonical terms are identified separately from package-authored copyright and license notices.\n\n'+packages.map(p=>`${p.ecosystem}:${p.name}@${p.version}\nDeclared license: ${p.license??'UNKNOWN'}\nLicense coverage: ${p.coverage}\n${p.review.map(r=>'Review: '+r+'\n').join('')}${p.texts.map(t=>`--- ${t.file} (${t.kind}) ---\nProvenance: ${t.provenance.url??t.provenance.origin}\nSHA-256: ${t.provenance.sha256}\n${t.text}`).join('\n')||'No verified license text is available for this exact source.'}`).join('\n\n'));
  return {packages:packages.length,reviewRequired:reviewRequired.length,target,commit};
}

if(import.meta.main){
  const root=resolve(import.meta.dir,'..');
  const target=targetTriple(process.argv[3],process.env.TAURI_ENV_TARGET_TRIPLE,()=>execFileSync('rustc',['-vV'],{encoding:'utf8'}));
  // Fresh clones can fetch locked metadata; --offline fails before Cargo has a cache.
  const cargo=JSON.parse(execFileSync('cargo',['metadata','--locked','--format-version','1','--manifest-path',join(root,'src-tauri/Cargo.toml'),'--filter-platform',target],{cwd:root,encoding:'utf8',maxBuffer:64*1024*1024})) as CargoMetadata;
  console.log(JSON.stringify(writeNotices(root,resolve(process.argv[2]??join(root,'licenses')),target,cargo)));
}
