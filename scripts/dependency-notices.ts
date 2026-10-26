/** Export an inspectable build inventory and locally supplied license texts.
 * Run after frozen dependency installation; no registry scripts are executed. */
import {createHash} from 'node:crypto';
import {existsSync,mkdirSync,readFileSync,readdirSync,writeFileSync} from 'node:fs';
import {dirname,join,resolve} from 'node:path';
import {execFileSync} from 'node:child_process';

const output=resolve(process.argv[2]??'src-tauri/target/dependency-notices');
const target=process.argv[3]??'x86_64-pc-windows-gnu';
const hash=(path:string)=>createHash('sha256').update(readFileSync(path)).digest('hex');
const packages:Array<{ecosystem:string;name:string;version:string;license:string|null;repository:unknown;texts:Array<{file:string;text:string}>}>=[];
function add(ecosystem:string,metadata:any,directory:string){
  if(packages.some(p=>p.ecosystem===ecosystem&&p.name===metadata.name&&p.version===metadata.version))return;
  const names=readdirSync(directory).filter(n=>/^(licen[cs]e|copying|notice)([._-]|$)/i.test(n));
  const texts=names.flatMap(file=>{try{return [{file,text:readFileSync(join(directory,file),'utf8')}];}catch{return [];}});
  packages.push({ecosystem,name:metadata.name,version:metadata.version,license:typeof metadata.license==='string'?metadata.license:null,repository:metadata.repository??null,texts});
}
function nodeModules(directory:string){
  if(!existsSync(directory))return;
  for(const entry of readdirSync(directory,{withFileTypes:true})){
    if(entry.name.startsWith('.'))continue;
    const path=join(directory,entry.name);
    if(entry.name.startsWith('@')){for(const name of readdirSync(path))nodePackage(join(path,name));}
    else nodePackage(path);
  }
}
function nodePackage(directory:string){
  const manifest=join(directory,'package.json');
  if(!existsSync(manifest))return;
  add('npm',JSON.parse(readFileSync(manifest,'utf8')),directory);
  nodeModules(join(directory,'node_modules'));
}
const cargo=JSON.parse(execFileSync('cargo',['metadata','--locked','--offline','--format-version','1','--manifest-path','src-tauri/Cargo.toml','--filter-platform',target],{encoding:'utf8',maxBuffer:32*1024*1024}));
for(const metadata of cargo.packages){if(metadata.source)add('cargo',metadata,dirname(metadata.manifest_path));}
nodeModules('node_modules');
packages.sort((a,b)=>`${a.ecosystem}:${a.name}:${a.version}`.localeCompare(`${b.ecosystem}:${b.name}:${b.version}`));
const commit=execFileSync('git',['rev-parse','HEAD'],{encoding:'utf8'}).trim();
const missing=packages.filter(p=>!p.license||!p.texts.length).map(p=>`${p.ecosystem}:${p.name}@${p.version}`);
mkdirSync(output,{recursive:true});
writeFileSync(join(output,'DEPENDENCIES.json'),JSON.stringify({format:'taskpulse-build-inventory-v1',commit,target,scope:'Cargo metadata for target and all installed frontend/build packages; generated native/OS components excluded',lockfiles:{'bun.lock':hash('bun.lock'),'Cargo.lock':hash('src-tauri/Cargo.lock')},reviewRequired:missing,packages:packages.map(({texts,...p})=>({...p,licenseFiles:texts.map(t=>t.file)}))},null,2)+'\n');
writeFileSync(join(output,'SBOM.cdx.json'),JSON.stringify({bomFormat:'CycloneDX',specVersion:'1.6',version:1,metadata:{component:{type:'application',name:'TaskPulse',version:JSON.parse(readFileSync('package.json','utf8')).version},properties:[{name:'taskpulse:source-commit',value:commit},{name:'taskpulse:target',value:target},{name:'taskpulse:bun-lock-sha256',value:hash('bun.lock')},{name:'taskpulse:cargo-lock-sha256',value:hash('src-tauri/Cargo.lock')},{name:'taskpulse:scope',value:'Installed build dependencies; includes development tools; excludes OS/native bundled components'}]},components:packages.map(p=>({type:'library','bom-ref':`${p.ecosystem}:${p.name}@${p.version}`,name:p.name,version:p.version,...(p.license?{licenses:[{license:{name:p.license}}]}:{})}))},null,2)+'\n');
writeFileSync(join(output,'THIRD_PARTY_NOTICES.txt'),'TaskPulse third-party build dependencies\nSource: '+commit+'\nTarget: '+target+'\nLicense expressions do not replace license texts. Review required entries in DEPENDENCIES.json before release.\n\n'+packages.map(p=>`${p.ecosystem}:${p.name}@${p.version}\nDeclared license: ${p.license??'UNKNOWN'}\n${p.texts.map(t=>`--- ${t.file} ---\n${t.text}`).join('\n')||'License text unavailable in the installed package.'}`).join('\n\n'));
console.log(JSON.stringify({packages:packages.length,reviewRequired:missing.length,output}));
