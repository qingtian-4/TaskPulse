/** Isolated native WebView2 acceptance at CDP 9337. Reading the center
 * persists notification seen; never acknowledges results or resolves requests. */
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
assert.ok(process.env.TASKPULSE_HOME,'Use an isolated TASKPULSE_HOME, never the live profile');
assert.notEqual(resolve(process.env.TASKPULSE_HOME).toLowerCase(),resolve(process.env.USERPROFILE!,'.taskpulse').toLowerCase());
const config=join(process.env.TASKPULSE_HOME,'config.toml');
const hash=async()=>createHash('sha256').update(await readFile(config)).digest('hex');
const before=await hash();
type Tab={url:string;webSocketDebuggerUrl:string};
const tabs:Tab[]=await fetch('http://127.0.0.1:9337/json').then(r=>r.json());
const tab=tabs.find(t=>t.url.includes('tauri.localhost')&&!/\/(manage|notice)/.test(t.url));
assert.ok(tab,'Start the actual embedded app with temporary CDP 9337');
const ws=new WebSocket(tab.webSocketDebuggerUrl);
await new Promise<void>((ok,fail)=>{ws.onopen=()=>ok();ws.onerror=()=>fail(Error('CDP unavailable'));});
let id=0;const pending=new Map<number,{ok:(v:any)=>void;fail:(e:any)=>void}>();
ws.onmessage=e=>{const m=JSON.parse(String(e.data));const p=pending.get(m.id);if(!p)return;pending.delete(m.id);m.error?p.fail(m.error):p.ok(m.result);};
const send=(method:string,params:any)=>new Promise<any>((ok,fail)=>{const n=++id;pending.set(n,{ok,fail});ws.send(JSON.stringify({id:n,method,params}));setTimeout(()=>{if(pending.delete(n))fail(Error('CDP timeout'));},10000);});
const evaluate=async(expression:string)=>{const r=await send('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true});if(r.exceptionDetails)throw Error('Native evaluation failed');return r.result.value;};
try {
  for(let n=0;n<100;n++){if(await evaluate(`!!document.querySelector('.dashboard')&&!!globalThis.__TAURI_INTERNALS__`))break;await Bun.sleep(100);}
  const environment=await evaluate(`__TAURI_INTERNALS__.invoke('integration_environment')`);
  assert.equal(resolve(environment.state_directory).toLowerCase(),resolve(process.env.TASKPULSE_HOME!).toLowerCase());
  const tasks=await evaluate(`__TAURI_INTERNALS__.invoke('get_agents')`);
  const prompts=await evaluate(`__TAURI_INTERNALS__.invoke('get_pending_task_prompts')`);
  assert.ok(Array.isArray(tasks)&&Array.isArray(prompts));
  assert.ok(prompts.every((t:any)=>!t.internal&&t.status==='waiting_input'&&['approval','question','quota_exhausted'].includes(t.waiting_reason)));
  await evaluate(`document.querySelector('.all-tasks').click()`);
  await Bun.sleep(300);
  const sections=await evaluate(`[...document.querySelectorAll('main section h2')].map(e=>e.firstChild.textContent.trim())`);
  const order=['进行中','结果待查看','状态待更新','固定与已查看'];
  assert.ok(sections.every((name:string)=>order.includes(name)));
  assert.deepEqual(sections,[...sections].sort((a:string,b:string)=>order.indexOf(a)-order.indexOf(b)));
  await evaluate(`document.querySelector('[aria-label^="消息中心："]').click()`);
  await Bun.sleep(200);
  const rows=await evaluate(`document.querySelectorAll('.pending-item').length`);
  assert.equal(rows,prompts.length);
  assert.ok(await evaluate(`document.querySelector('.message-center')?.textContent.includes('不会批准或解除请求')`));
  const layout=await evaluate(`({width:document.querySelector('.dashboard').getBoundingClientRect().width,height:document.querySelector('.dashboard').getBoundingClientRect().height,overflow:document.documentElement.scrollWidth>innerWidth,errors:[...document.querySelectorAll('[role=alert]')].length})`);
  assert.ok(layout.width<=440.1&&layout.height<=720.1&&!layout.overflow&&!layout.errors);
  await evaluate(`document.querySelector('[aria-label="关闭消息中心"]').click()`);
  assert.equal(await hash(),before,'Read-only inbox acceptance changed config');
  const result={passed:true,taskCount:tasks.length,promptCount:prompts.length,sections,layout,checks:['actual native IPC','semantic section ordering','persistent prompt catalog','inbox rendering','bounded native panel','config unchanged'],notVerified:['real fresh Hook approval/reopen/resolution','editor focus','cross-version installer upgrade','long-run P95 latency']};
  await mkdir('src-tauri/target/ui-smoke',{recursive:true});
  await writeFile('src-tauri/target/ui-smoke/native-inbox-result.json',JSON.stringify(result,null,2));
  console.log(JSON.stringify(result));
}finally{ws.close();}
