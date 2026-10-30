import {expect,test} from 'bun:test';
import {createHash} from 'node:crypto';
import {mkdtempSync,mkdirSync,readFileSync,rmSync,symlinkSync,writeFileSync} from 'node:fs';
import {join} from 'node:path';
import {tmpdir} from 'node:os';
import {execFileSync} from 'node:child_process';
import {collectNotices,localLicenseTexts,matchingRule,pinnedLicenseText,reachableCargoPackages,sourceCommit,targetTriple,wrapperParent,writeNotices,type CargoMetadata,type SourceManifest} from './dependency-notices';

const sha='a'.repeat(40),other='b'.repeat(40);
const empty:SourceManifest={format:'taskpulse-license-sources-v1',sources:{},packages:[]};
function fixture(run:(root:string)=>void){const root=mkdtempSync(join(tmpdir(),'taskpulse-license-'));try{run(root);}finally{rmSync(root,{recursive:true,force:true});}}
function put(path:string,text:string){mkdirSync(join(path,'..'),{recursive:true});writeFileSync(path,text);}
const graph=(root:string):CargoMetadata=>({packages:[{id:'app',name:'agent-tray',version:'1.0.0',source:null,manifest_path:join(root,'Cargo.toml')}],resolve:{root:'app',nodes:[{id:'app',deps:[]}]}});

test('explicit nested license files work; traversal/symlinks and copying.rs cannot masquerade as licenses',()=>fixture(root=>{
  const directory=join(root,'package');mkdirSync(directory);
  put(join(directory,'legal','grant.txt'),'Actual package grant');put(join(root,'outside','LICENSE'),'Outside grant');
  put(join(directory,'THIRD-PARTY-LICENSE'),'Dependency notices');put(join(directory,'copying.rs'),'fn copy() {}');put(join(directory,'src/copying.rs'),'fn copy() {}');
  const link=join(directory,'external');symlinkSync(join(root,'outside'),link,process.platform==='win32'?'junction':'dir');
  const metadata={name:'fixture',version:'1.0.0',license:'MIT',license_file:'legal/grant.txt'};
  expect(localLicenseTexts(metadata,directory).map(t=>t.file).sort()).toEqual(['THIRD-PARTY-LICENSE','legal/grant.txt']);
  for(const license_file of ['../outside/LICENSE','external/LICENSE','src/copying.rs'])expect(localLicenseTexts({...metadata,license_file},directory).map(t=>t.file)).toEqual(['THIRD-PARTY-LICENSE']);
}));

test('native wrapper fallback requires the expected package family, exact version and same repository',()=>fixture(root=>{
  const repo='https://github.com/rolldown/rolldown';
  const metadata={name:'@rolldown/binding-win32-x64-msvc',version:'1.2.14',license:'MIT',repository:{url:'git+'+repo+'.git'}};
  put(join(root,'node_modules/rolldown/package.json'),JSON.stringify({name:'rolldown',version:'1.2.14',repository:repo}));
  expect(wrapperParent(metadata,root)?.metadata.name).toBe('rolldown');
  for(const patch of [{version:'1.2.15'},{repository:'https://github.com/another/project'},{name:'@rolldown/binding-unrelated'}])expect(wrapperParent({...metadata,...patch},root)).toBeNull();
  put(join(root,'node_modules/rolldown/package.json'),JSON.stringify({name:'rolldown',version:'1.2.14',repository:'https://github.com/another/project'}));
  expect(wrapperParent(metadata,root)).toBeNull();
}));

test('Cargo inventory follows only filtered reachable edges, including build nodes and cycles',()=>{
  const packages=['app','common','windows','build','mac-only'].map(id=>({id,name:id,version:'1',source:id==='app'?null:'registry+https://index.crates.io/',manifest_path:'/fixture/'+id+'/Cargo.toml'}));
  const metadata:CargoMetadata={packages,resolve:{root:'app',nodes:[{id:'app',deps:[{pkg:'common'},{pkg:'build'}]},{id:'common',deps:[{pkg:'windows'}]},{id:'windows',deps:[{pkg:'common'}]},{id:'build',deps:[]},{id:'mac-only',deps:[]}]}};
  expect(reachableCargoPackages(metadata).map(p=>p.name)).toEqual(['app','common','windows','build']);
  expect(()=>reachableCargoPackages({...metadata,resolve:{...metadata.resolve,nodes:metadata.resolve.nodes.filter(n=>n.id!=='windows')}})).toThrow('Incomplete');
});

test('pinned upstream text cannot be borrowed by a different revision/version or silently changed',()=>fixture(root=>{
  const directory=join(root,'crate'),license='Copyright (c) an upstream fixture\nPermission is hereby granted.';
  put(join(directory,'.cargo_vcs_info.json'),JSON.stringify({git:{sha1:sha},path_in_vcs:'crate'}));
  put(join(root,'licenses/upstream/LICENSE.txt'),license);
  const manifest:SourceManifest={format:empty.format,sources:{original:{path:'upstream/LICENSE.txt',url:`https://raw.githubusercontent.com/fixture/project/${sha}/LICENSE`,sha256:createHash('sha256').update(license).digest('hex'),kind:'license'}},packages:[{ecosystem:'cargo',name:'fixture',version:'1.0.0',license:'MIT',repository:'https://github.com/fixture/project',sourceCommit:sha,pathInVcs:'crate',sources:['original']}]};
  const metadata={name:'fixture',version:'1.0.0',license:'MIT',repository:'https://github.com/fixture/project'};
  expect(matchingRule('cargo',metadata,directory,root,manifest)).toBeDefined();
  expect(matchingRule('cargo',{...metadata,version:'1.0.1'},directory,root,manifest)).toBeUndefined();
  put(join(directory,'.cargo_vcs_info.json'),JSON.stringify({git:{sha1:other},path_in_vcs:'crate'}));
  expect(matchingRule('cargo',metadata,directory,root,manifest)).toBeUndefined();
  expect(pinnedLicenseText(root,'original',manifest).text).toBe(license);
  put(join(root,'licenses/upstream/LICENSE.txt'),license+' altered');
  expect(()=>pinnedLicenseText(root,'original',manifest)).toThrow('checksum');
}));

test('canonical terms remain separate from declarations and never silently rewrite MIT-only grants',()=>fixture(root=>{
  put(join(root,'package.json'),JSON.stringify({version:'1.0.0',license:'MIT'}));put(join(root,'LICENSE'),'Project fixture copyright');
  const directory=join(root,'crate');put(join(directory,'.cargo_vcs_info.json'),JSON.stringify({git:{sha1:sha},path_in_vcs:'crate'}));
  const texts={declaration:'Upstream declares MIT; standalone text omitted.',canonical:'Canonical MIT terms without invented attribution.'};
  const sources=Object.fromEntries(Object.entries(texts).map(([key,text])=>{
    put(join(root,'licenses',key+'.txt'),text);
    return [key==='canonical'?'spdx:MIT':key,{path:key+'.txt',url:`https://raw.githubusercontent.com/${key==='canonical'?'spdx/license-list-data':'fixture/project'}/${sha}/LICENSE`,sha256:createHash('sha256').update(text).digest('hex'),kind:key==='canonical'?'canonical-terms':'upstream-declaration'}];
  }));
  const rule={ecosystem:'cargo' as const,name:'fixture',version:'1.0.0',license:'MIT',repository:'https://github.com/fixture/project',sourceCommit:sha,pathInVcs:'crate',sources:['declaration'],canonicalTerms:['spdx:MIT']};
  const manifest:SourceManifest={format:empty.format,canonicalSource:{repository:'https://github.com/spdx/license-list-data',commit:sha},sources:sources as SourceManifest['sources'],packages:[rule]};
  const cargo=graph(root);cargo.packages.push({id:'dep',...rule,source:'registry+https://index.crates.io/',manifest_path:join(directory,'Cargo.toml')});cargo.resolve.nodes.push({id:'dep',deps:[]});cargo.resolve.nodes[0].deps.push({pkg:'dep'});
  const pkg=collectNotices(root,cargo,manifest).find(p=>p.name==='fixture')!;
  expect(pkg.license).toBe('MIT');expect(pkg.coverage).toBe('declaration-and-canonical-terms');expect(pkg.review.length).toBeGreaterThan(0);
  expect(pkg.texts.map(t=>t.kind)).toEqual(['upstream-declaration','canonical-terms']);
  manifest.sources['spdx:Apache-2.0']=manifest.sources['spdx:MIT'];manifest.packages[0].canonicalTerms=['spdx:Apache-2.0'];
  expect(()=>collectNotices(root,cargo,manifest)).toThrow('declared license');
}));

test('target precedence and source archives work without borrowing an enclosing checkout provenance',()=>fixture(root=>{
  expect(targetTriple('aarch64-apple-darwin','x86_64-pc-windows-msvc',()=>{throw Error('Host must not run');})).toBe('aarch64-apple-darwin');
  expect(targetTriple(undefined,'x86_64-pc-windows-msvc',()=>{throw Error('Host must not run');})).toBe('x86_64-pc-windows-msvc');
  expect(targetTriple(undefined,undefined,()=> 'rustc 1\nhost: x86_64-unknown-linux-gnu\nrelease: 1')).toBe('x86_64-unknown-linux-gnu');
  expect(()=>targetTriple('../invalid',undefined,()=> '')).toThrow('target triple');
  execFileSync('git',['init',root],{stdio:'pipe'});
  execFileSync('git',['-C',root,'-c','commit.gpgsign=false','-c','user.name=TaskPulse Fixture','-c','user.email=fixture@example.invalid','commit','--allow-empty','-m','fixture'],{stdio:'pipe'});
  const archive=join(root,'source-archive');mkdirSync(archive);
  expect(sourceCommit(root)).toMatch(/^[a-f0-9]{40}$/);expect(sourceCommit(archive)).toBeNull();
  put(join(archive,'package.json'),JSON.stringify({version:'1.0.0',license:'MIT'}));put(join(archive,'LICENSE'),'Project fixture copyright');
  put(join(archive,'bun.lock'),'{}');put(join(archive,'src-tauri/Cargo.lock'),'');put(join(archive,'licenses/sources.json'),JSON.stringify(empty));
  const result=writeNotices(archive,join(archive,'licenses'),'x86_64-unknown-linux-gnu',graph(archive));
  expect(result.commit).toBeNull();const output=JSON.parse(readFileSync(join(archive,'licenses/DEPENDENCIES.json'),'utf8'));
  expect(output.commit).toBeNull();expect(output.packages[0].name).toBe('TaskPulse');
  expect(readFileSync(join(archive,'licenses/THIRD_PARTY_NOTICES.txt'),'utf8')).toContain('Project fixture copyright');
}));
