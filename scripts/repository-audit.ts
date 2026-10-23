/** Offline checks of versioned documentation and media. Never reads user state. */
import {existsSync, readFileSync} from 'node:fs';
import {dirname, extname, isAbsolute, relative, resolve} from 'node:path';
import {spawnSync} from 'node:child_process';

export function gitFiles(root: string): string[] {
  const result=spawnSync('git',['ls-files','-z'],{cwd:root,encoding:'utf8',windowsHide:true});
  if(result.error || result.status!==0)throw Error('Cannot enumerate tracked files');
  return result.stdout.split('\0').filter(Boolean);
}

function prose(body: string): string {
  // Link-like examples inside fenced code are not navigation.
  const lines=body.split(/\r?\n/);let fence='';
  return lines.map(line=>{
    const marker=line.match(/^\s*(`{3,}|~{3,})/);
    if(marker){if(!fence)fence=marker[1][0];else if(fence===marker[1][0])fence='';return '';}
    return fence?'':line;
  }).join('\n');
}

export function headingIds(body: string): Set<string> {
  const counts=new Map<string,number>(),ids=new Set<string>();
  for(const match of prose(body).matchAll(/^#{1,6}\s+(.+?)\s*#*\s*$/gm)){
    const slug=match[1].replace(/<[^>]*>/g,'').toLowerCase().replace(/[`*]/g,'').replace(/[^\p{L}\p{N}\p{M}\s_-]/gu,'').replace(/ /g,'-');
    const n=counts.get(slug)??0;counts.set(slug,n+1);ids.add(slug+(n?'-'+n:''));
  }
  for(const match of body.matchAll(/\b(?:id|name)=["']([^"']+)["']/g))ids.add(match[1]);
  return ids;
}

export function documentationErrors(root: string, files: string[]): string[] {
  const errors:string[]=[];
  const tracked=new Set(files.map(f=>f.replaceAll('\\','/')));
  for(const file of files.filter(f=>['.md','.svg'].includes(extname(f)))){
    const body=readFileSync(resolve(root,file),'utf8');
    const links=extname(file)==='.md'
      ? [...prose(body).matchAll(/\]\(<?([^\s)>]+)>?(?:\s+"[^"]*")?\)|^\s*\[[^\]]+\]:\s*<?([^\s>]+)>?|\b(?:src|href)=["']([^"']+)["']/gm)].map(m=>m[1]??m[2]??m[3])
      : [...body.matchAll(/\b(?:href|xlink:href)=["']([^"']+)["']|url\((#[^)]+)\)/g)].map(m=>m[1]??m[2]);
    for(const link of links){
      if(/^(?:[a-z][a-z\d+.-]*:|\/\/)/i.test(link))continue;
      let decoded:string;
      try{decoded=decodeURIComponent(link);}catch{errors.push(`${file}: invalid URL encoding ${link}`);continue;}
      const hash=decoded.indexOf('#'),path=hash<0?decoded:decoded.slice(0,hash),anchor=hash<0?'':decoded.slice(hash+1);
      const target=path?resolve(root,dirname(file),path):resolve(root,file);
      const rel=relative(root,target);
      if(rel==='..'||rel.startsWith('..\\')||rel.startsWith('../')||isAbsolute(rel)){errors.push(`${file}: link escapes repository: ${link}`);continue;}
      if(!existsSync(target)){errors.push(`${file}: missing ${link}`);continue;}
      const portable=rel.replaceAll('\\','/');
      if(!tracked.has(portable) && ![...tracked].some(f=>f.startsWith(portable.replace(/\/$/,'')+'/'))){
        errors.push(`${file}: target is not tracked ${link}`);continue;
      }
      if(anchor && ['.md','.svg'].includes(extname(target))){
        const contents=readFileSync(target,'utf8');
        const ids=extname(target)==='.md'?headingIds(contents):new Set([...contents.matchAll(/\bid=["']([^"']+)["']/g)].map(m=>m[1]));
        if(!ids.has(anchor))errors.push(`${file}: missing anchor ${link}`);
      }
    }
  }
  return errors;
}
