// Check the emitted desktop entry pages before Tauri embeds the frontend.
import {readFileSync} from 'node:fs';
import {join,resolve} from 'node:path';

const root=resolve(process.argv[2]??'build');
const pages=['index.html','manage/index.html','notice.html'];
for(const page of pages){
  const html=readFileSync(join(root,page),'utf8');
  const payloads=new Set([...html.matchAll(/\b(__sveltekit_[a-z0-9]+)\s*=\s*\{/g)].map(m=>m[1]));
  const assets=new Set([...html.matchAll(/(?:href=|import\()["']\/_app\/immutable\/((?:[\w-]+\/)*[\w.-]+\.js)["']/g)].map(m=>m[1]));
  const runtimes=new Set<string>();
  for(const asset of assets){
    const source=readFileSync(join(root,'_app','immutable',asset),'utf8');
    for(const match of source.matchAll(/\bglobalThis\.(__sveltekit_[a-z0-9]+)\.data\b/g))runtimes.add(match[1]);
  }
  const [payload]=payloads;
  if(payloads.size!==1||runtimes.size!==1||!runtimes.has(payload)){
    throw Error(`Frontend startup payload mismatch in ${page}: HTML=${[...payloads].join(',')} client=${[...runtimes].join(',')}`);
  }
}
console.log('Frontend startup payloads match for dashboard, settings and reminders.');
