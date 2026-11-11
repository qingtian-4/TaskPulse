/** Native provider/UI acceptance using public native-schema fixtures, not real-host certification. */
import assert from 'node:assert/strict';
import {Database} from 'bun:sqlite';
import {createHash} from 'node:crypto';
import {mkdir,copyFile,appendFile} from 'node:fs/promises';
import {dirname,join,resolve} from 'node:path';
import {spawn,type ChildProcess} from 'node:child_process';
import {withSmokeWorkspace,stopOwnedChild} from '../support/smoke-workspace';

const arg=process.argv.indexOf('--exe'),head=process.argv.indexOf('--commit');
assert.ok(arg>=0&&head>=0,'--exe and full --commit required');
const original=resolve(process.argv[arg+1]),commit=process.argv[head+1];assert.match(commit,/^[a-f0-9]{40}$/);
await withSmokeWorkspace('native-agent-readers',async({owned,onStop})=>{
 let app:ChildProcess|undefined;const sockets:WebSocket[]=[];let stderr='';
 onStop(async()=>{sockets.forEach(s=>s.close());await stopOwnedChild(app);});
 const exe=join(owned,'agent-tray.exe');await copyFile(original,exe);if(await Bun.file(join(dirname(original),'WebView2Loader.dll')).exists())await copyFile(join(dirname(original),'WebView2Loader.dll'),join(owned,'WebView2Loader.dll'));
 const home=join(owned,'profile'),kimi=join(owned,'kimi'),qoder=join(owned,'qoder'),trae=join(owned,'trae'),claude=join(owned,'claude');
 const session=join(kimi,'sessions','public','session-kimi');await mkdir(join(session,'agents','main'),{recursive:true});await mkdir(home);await mkdir(qoder);await mkdir(join(trae,'ModularData','ai-agent'),{recursive:true});
 await mkdir(join(claude,'projects'),{recursive:true});
 await Bun.write(join(trae,'ModularData','ai-agent','database.db'),Buffer.alloc(32,0x91));
 await Bun.write(join(session,'state.json'),JSON.stringify({id:'session-kimi',title:'Kimi 原生会话公开夹具',cwd:'C:/public/Kimi'}));
 const wire=join(session,'agents','main','wire.jsonl'),at=Date.now();
 const event=async(type:string,data:Record<string,unknown>={},delta=0)=>appendFile(wire,JSON.stringify({type,agentId:'main',time:at+delta,...data})+'\n');
 await Bun.write(join(kimi,'config.toml'),'[models."fixture/flash"]\nmodel = "public-flash"\ndisplay_name = "Public Flash"\n');
 await Bun.write(wire,'');await event('metadata',{protocol_version:'1.5',created_at:at});await event('turn.prompt',{turnId:1,origin:'user',input:[{type:'text',text:'Public fixture'}]});await event('llm.request',{kind:'loop',model:'public-flash',modelAlias:'fixture/flash',thinkingEffort:'high'});
 await event('agent.message.appended',{message:{meta:{model:{provider:'agent-loop',model:'agent-loop'}}}});
 const db=new Database(join(qoder,'main.sqlite'));
 db.exec(`CREATE TABLE chat_sessions(session_id TEXT,title TEXT,cwd TEXT,model TEXT,deleted_at TEXT,archived INTEGER,owner_session_id TEXT,updated_at TEXT);
 CREATE TABLE chat_session_messages(session_id TEXT,message_id TEXT,turn_id TEXT,sequence INTEGER,payload_json TEXT,status TEXT,source TEXT,created_at TEXT,updated_at TEXT);`);
 const iso=new Date(at).toISOString();db.query('INSERT INTO chat_sessions VALUES(?,?,?,?,NULL,0,NULL,?)').run('session-qoder','Qoder 原生轮次公开夹具','C:/public/Qoder','qoder-public-model',iso);
 const row=(id:string,seq:number,status:string,value:unknown)=>db.query("INSERT INTO chat_session_messages VALUES('session-qoder',?,'turn-qoder',?,?,?,'host-projection',?,?)").run(id,seq,JSON.stringify(value),status,iso,iso);
 row('user',1,'completed',{id:'user',role:'user',text:'Public fixture'});row('assistant',2,'streaming',{id:'assistant',role:'assistant',turnId:'turn-qoder',turnStartedAt:iso,timestamp:iso,tools:[]});
 const sources=[['public-kimi','kimi_code',kimi],['public-qoder','qoder',qoder],['public-trae','trae',trae],['public-claude','claude_code',claude]];
 await Bun.write(join(home,'config.toml'),`[dashboard]\nversion=2\nonboarding_complete=true\nauto_discover_local=false\nauto_check_updates=false\nnotifications=true\nsound=false\nnotification_mode="popup"\ndefault_view="tasks"\n`+sources.map(([id,kind,location])=>`[[dashboard.sources]]\nid=${JSON.stringify(id)}\nkind=${JSON.stringify(kind)}\nname=${JSON.stringify(id)}\nenabled=true\nlocation=${JSON.stringify(location.replaceAll('\\','/'))}\nprojects=["*"]\nexcluded_projects=[]\ntmux_sessions=[]\ntmux_panes=[]\npoll_interval_ms=500\n`).join(''));
 const env={...process.env,TASKPULSE_HOME:home,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:'--remote-debugging-port=9341'};
 assert.equal(await fetch('http://127.0.0.1:9341/json').then(()=>true).catch(()=>false),false,'CDP port occupied');
 const start=()=>{app=spawn(exe,['--show'],{env,windowsHide:true,stdio:['ignore','ignore','pipe']});app.stderr?.on('data',d=>stderr=(stderr+String(d)).slice(-8000));};
 async function connect(path=''){
  let target:any;for(let n=0;n<200;n++){const tabs:any[]=await fetch('http://127.0.0.1:9341/json').then(r=>r.json()).catch(()=>[]);target=tabs.find(t=>path?t.url.includes(path):t.url.includes('tauri.localhost')&&!t.url.includes('/manage')&&!t.url.includes('/notice'));if(target)break;await Bun.sleep(100);}assert.ok(target,'Native window missing');
  const ws=new WebSocket(target.webSocketDebuggerUrl);sockets.push(ws);await new Promise<void>((ok,fail)=>{ws.onopen=()=>ok();ws.onerror=()=>fail(Error('CDP failed'));});let seq=0;const pending=new Map<number,any>();
  ws.onmessage=e=>{const m=JSON.parse(String(e.data)),p=pending.get(m.id);if(p){pending.delete(m.id);clearTimeout(p.timer);m.error?p.fail(Error(JSON.stringify(m.error))):p.ok(m.result);}};
  const send=(method:string,params:unknown={})=>new Promise<any>((ok,fail)=>{const id=++seq,timer=setTimeout(()=>{pending.delete(id);fail(Error('CDP timed out: '+method));},20000);pending.set(id,{ok,fail,timer});ws.send(JSON.stringify({id,method,params}));});
  const evaluate=async(expression:string)=>{const result=await send('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true,userGesture:true});if(result.exceptionDetails)throw Error(result.exceptionDetails.exception?.description??result.exceptionDetails.text);return result.result.value;};
  const invoke=(command:string,args:unknown={})=>evaluate(`__TAURI_INTERNALS__.invoke(${JSON.stringify(command)},${JSON.stringify(args)})`);
  const wait=async(expression:string)=>{for(let n=0;n<200;n++){if(await evaluate(expression))return;await Bun.sleep(100);}throw Error('Native condition timed out: '+expression);};
  return{send,evaluate,invoke,wait};
 }
 try{
  start();let root=await connect();await root.wait(`!!document.querySelector('.dashboard')`);const build=await root.invoke('get_build_info');assert.equal(build.commit,commit);assert.equal(build.modified,false);assert.equal(build.process_id,app!.pid);
  if(!await root.invoke('get_pinned'))await root.invoke('toggle_pin');
  await root.wait(`__TAURI_INTERNALS__.invoke('get_agents').then(t=>t.filter(t=>t.status==='running').length===2)`);
  const tasks=await root.invoke('get_agents'),k=tasks.find((t:any)=>t.agent_id==='kimi-code'),q=tasks.find((t:any)=>t.agent_id==='qoder-desktop');assert.ok(k&&q);assert.equal(k.model,'Public Flash');assert.equal(k.reasoning_effort,'high');assert.equal(q.model,'qoder-public-model');
  await root.wait(`document.querySelector('[data-task-id="${k.id}"] .model')?.textContent.includes('Public Flash') && document.querySelector('[data-task-id="${k.id}"] .effort')?.textContent==='high'`);
  assert.ok(tasks.every((t:any)=>t.agent_id!=='trae'),'Opaque TRAE store must not create fake tasks');
  await root.wait(`__TAURI_INTERNALS__.invoke('get_providers').then(p=>p.some(p=>p.provider==='public-trae'&&p.status==='degraded'))`);
  await event('interaction.request',{id:'native-question',kind:'question',toolCallId:'native-call',request:{questions:[{id:'choice',question:'Public fixture'}]}},1000);
  await root.wait(`document.querySelector('[data-task-id="${k.id}"] .status-label')?.textContent.includes('等待输入')`);
  await root.wait(`__TAURI_INTERNALS__.invoke('get_task_notice').then(s=>s.current?.task_id==='${k.id}')`);
  let notice=await connect('/notice');await notice.wait(`document.querySelector('.heading')?.textContent==='Kimi 原生会话公开夹具'`);assert.equal((await root.invoke('get_task_notice')).current.auto_dismiss_ms,null);
  await Bun.write(join(owned,'kimi-question-native.png'),Buffer.from((await notice.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
  await event('interaction.resolved',{id:'unrelated',response:{}},2000);await root.wait(`__TAURI_INTERNALS__.invoke('get_agents').then(t=>t.some(t=>t.id==='${k.id}'&&t.updated_at>=${at+2000}&&t.input_request_id==='native-question'))`);
  await event('turn.cancel',{turnId:1,target:'queued'},3000);await root.wait(`__TAURI_INTERNALS__.invoke('get_agents').then(t=>t.some(t=>t.id==='${k.id}'&&t.updated_at>=${at+3000}&&!t.finished_at))`);
  const response=JSON.stringify({type:'interaction.resolved',agentId:'main',id:'native-question',response:{answers:{choice:'A'}},time:at+4000});await appendFile(wire,response);
  await root.wait(`__TAURI_INTERNALS__.invoke('get_providers').then(p=>p.some(p=>p.provider==='public-kimi'&&p.status==='degraded'))`);assert.equal((await root.invoke('get_agents')).find((t:any)=>t.id===k.id).input_request_id,'native-question');
  await appendFile(wire,'\n');await root.wait(`__TAURI_INTERNALS__.invoke('get_agents').then(t=>t.some(t=>t.id==='${k.id}'&&t.status==='running'&&!t.stale&&!t.input_request_id&&!t.finished_at))`);
  await root.wait(`__TAURI_INTERNALS__.invoke('get_task_notice').then(s=>s.current?.task_id!=='${k.id}')`);
  await event('turn.ended',{turnId:1,reason:'completed'},61000);
  await root.wait(`document.querySelector('[data-task-id="${k.id}"] .elapsed')?.textContent==='1m 1s'`);
  db.query("UPDATE chat_session_messages SET status='completed',payload_json=? WHERE message_id='assistant'").run(JSON.stringify({id:'assistant',role:'assistant',turnId:'turn-qoder',turnStartedAt:iso,turnMetrics:{durationMs:42000,turnCount:2}}));
  await root.wait(`document.querySelector('[data-task-id="${q.id}"] .elapsed')?.textContent==='42s'`);
  const ended=await root.invoke('get_agents');assert.ok(ended.every((t:any)=>t.unread&&t.finished_at));
  await root.invoke('update_task_attention',{id:k.id,action:'pin'});
  await Bun.write(join(owned,'native-reader-results.png'),Buffer.from((await root.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
  await root.invoke('open_management',{section:'sources'});const manage=await connect('/manage');
  await manage.wait(`!!document.querySelector('.local-agents')`);
  assert.ok(await manage.evaluate(`!document.querySelector('[data-agent-kind="scanner"], [data-saved-source="local_file"]')`));
  await manage.wait(`document.querySelector('[data-agent-kind="trae"]').textContent.includes('已检测到 · 仅支持会话目录')`);
  await manage.wait(`document.querySelector('[data-agent-kind="claude_code"]').textContent.includes('读取正常 · 暂无近期会话')`);
  assert.equal(await manage.evaluate(`document.querySelector('[data-agent-kind="trae"] strong').textContent`),'Trae');
  await Bun.write(join(owned,'native-reader-settings.png'),Buffer.from((await manage.send('Page.captureScreenshot',{format:'png'})).data,'base64'));
  sockets.forEach(s=>s.close());await stopOwnedChild(app);start();root=await connect();await root.wait(`typeof __TAURI_INTERNALS__!=='undefined'&&!!document.querySelector('.dashboard')`);await root.wait(`__TAURI_INTERNALS__.invoke('get_agents').then(t=>t.some(t=>t.id==='${k.id}'&&t.finished_at&&t.unread&&t.pinned&&t.model==='Public Flash'&&t.reasoning_effort==='high')&&t.some(t=>t.id==='${q.id}'&&t.finished_at&&t.model==='qoder-public-model'))`);
  assert.equal((await root.invoke('get_agents')).length,2);
  // Exercise real provider freshness without making the test wait two
  // minutes: the new native start is already 108 seconds old at admission.
  const quiet=join(kimi,'sessions','public','session-quiet','agents','main');
  await mkdir(quiet,{recursive:true});
  const quietWire=join(quiet,'wire.jsonl');
  const quietAt=Date.now()-108000;
  const quietId='public-kimi:kimi-code:session-quiet';
  await Bun.write(quietWire,JSON.stringify({type:'turn.started',turnId:'quiet',time:quietAt})+'\n');
  if(!await root.invoke('get_pinned'))await root.invoke('toggle_pin');
  await root.wait(`__TAURI_INTERNALS__.invoke('get_dashboard_snapshot').then(s=>s.tasks.some(t=>t.id==='${quietId}'&&t.status==='running'&&!t.stale))`);
  await root.wait(`__TAURI_INTERNALS__.invoke('get_dashboard_snapshot').then(s=>s.view.counts.ongoing===0&&s.tasks.some(t=>t.id==='${quietId}'&&t.stale&&!t.finished_at))`);
  await root.wait(`document.querySelector('[data-task-id="${quietId}"] .status-label')?.textContent==='状态待更新'`);
  const refresh=await root.invoke('request_refresh');
  await root.wait(`__TAURI_INTERNALS__.invoke('get_dashboard_snapshot').then(s=>s.providers.some(p=>p.provider==='public-kimi'&&p.last_attempt>=${refresh}))`);
  assert.equal(await root.evaluate(`document.querySelector('[data-task-id="${quietId}"] .status-label')?.textContent`),'状态待更新');
  await appendFile(quietWire,JSON.stringify({type:'llm.request',model:'kimi-public-model',time:Date.now()})+'\n');
  await root.wait(`__TAURI_INTERNALS__.invoke('get_dashboard_snapshot').then(s=>s.view.counts.ongoing===1&&!s.view.unconfirmed_task_ids.includes('${quietId}')&&s.tasks.filter(t=>t.id==='${quietId}'&&t.status==='running'&&!t.stale).length===1)`);
  await root.wait(`document.querySelector('[data-task-id="${quietId}"] .status-label')?.textContent==='运行中'`);
  await appendFile(quietWire,JSON.stringify({type:'turn.ended',turnId:'quiet',reason:'completed',time:Date.now()})+'\n');
  await root.wait(`__TAURI_INTERNALS__.invoke('get_agents').then(t=>t.some(t=>t.id==='${quietId}'&&t.waiting_reason==='turn_ended'&&t.finished_at))`);
  const hermesPath=join(owned,'hermes.db'),zcodePath=join(owned,'zcode.db');
  const hermes=new Database(hermesPath),zcode=new Database(zcodePath),started=Date.now();
  try {
    hermes.exec(`CREATE TABLE sessions(id TEXT,display_name TEXT,title TEXT,last_activity_at REAL,started_at REAL,cwd TEXT,git_repo_root TEXT,ended_at REAL,end_reason TEXT,archived INTEGER,hidden INTEGER,pinned INTEGER,last_read_at REAL,last_activity_description TEXT,model TEXT,model_config TEXT);
      CREATE TABLE session_turn_leases(conversation_id TEXT,acquired_at REAL,expires_at REAL);
      CREATE TABLE messages(id INTEGER PRIMARY KEY,session_id TEXT,role TEXT,timestamp REAL,finish_reason TEXT,active INTEGER DEFAULT 1,compacted INTEGER DEFAULT 0,display_kind TEXT);`);
    hermes.query("INSERT INTO sessions VALUES('test-hermes',NULL,'Hermes example',?,?,'C:/public/Hermes',NULL,NULL,NULL,0,0,0,NULL,NULL,'hermes-public',?)")
      .run(started/1000,started/1000,JSON.stringify({model:'hermes-public',reasoning_config:{enabled:true,effort:'high'}}));
    hermes.query("INSERT INTO session_turn_leases VALUES('test-hermes',?,?)").run(started/1000,started/1000+300);
    zcode.exec(`CREATE TABLE session(id TEXT,title TEXT,directory TEXT,last_outcome TEXT,time_deleted INTEGER,time_archived INTEGER,time_updated INTEGER);
      CREATE TABLE message(id TEXT,session_id TEXT,sequence INTEGER,data TEXT);
      CREATE TABLE part(id TEXT,message_id TEXT,sequence INTEGER,data TEXT);
      INSERT INTO session VALUES('test-zcode','ZCode example','C:/public/ZCode','completed',NULL,NULL,${started});`);
    zcode.query("INSERT INTO message VALUES('z-user','test-zcode',1,?)").run(JSON.stringify({role:'user',time:{created:started},
      modelSelection:{providerId:'public',modelId:'zcode-public',options:{reasoningLevel:'high'}},
      model:{providerID:'public',modelID:'zcode-public',variant:'max'}}));
    await root.invoke('open_management',{section:'sources'});const settings=await connect('/manage');
    const saved=await settings.invoke('get_monitoring_settings');
    const additions=[['public-hermes','hermes',hermesPath],['public-zcode','zcode',zcodePath]].map(([id,kind,location])=>({
      id,kind,name:id,location,enabled:true,projects:['*'],excluded_projects:[],tmux_sessions:[],tmux_panes:[],poll_interval_ms:500
    }));
    await settings.invoke('save_monitoring_settings',{expected:saved,settings:{...saved,sources:[...saved.sources,...additions]}});
    await root.wait(`document.querySelector('[data-task-id="public-hermes:hermes:test-hermes"] .effort')?.textContent==='high'`);
    await root.wait(`document.querySelector('[data-task-id="public-zcode:zcode:test-zcode"] .effort')?.textContent==='high'`);
    const live=await root.invoke('get_agents');
    assert.equal(live.find((t:any)=>t.source_instance==='public-zcode').model,'zcode-public');
    assert.equal(live.find((t:any)=>t.source_instance==='public-hermes').model,'hermes-public');
    zcode.query("INSERT INTO message VALUES('z-assistant','test-zcode',2,?)").run(JSON.stringify({role:'assistant',parentID:'z-user',providerId:'public',modelId:'zcode-public',time:{created:started+100,completed:started+200},finish:'stop'}));
    await root.wait(`document.querySelector('[data-task-id="public-zcode:zcode:test-zcode"] .status-label')?.textContent==='本轮结束'`);
    assert.equal(await root.evaluate(`document.querySelector('[data-task-id="public-zcode:zcode:test-zcode"]').closest('.task-item').dataset.tone`),'unread');
  } finally {
    hermes.close();zcode.close();
  }
  const result={passed:true,commit,modified:false,executableSha256:createHash('sha256').update(await Bun.file(exe).bytes()).digest('hex'),checks:['Kimi native v1.5 request → panel → independent popup → exact resolution','unrelated response and queued cancel cannot end or resolve active request','partial native write preserves unresolved request and degrades health','explicit Kimi/Qoder end freezes source duration','opaque TRAE data never invents tasks or healthy state','native Agent settings, catalog-only Trae and healthy empty Claude reads','Kimi alias, Hermes session effort and ZCode current-turn effort reach the native renderer','successful ZCode turns use the common end label and unread tone','same native session IDs, unread, pin, model and effort survive restart','Kimi quiet open turn remains visible as unconfirmed through a refresh, recovers the same card, and ends only on an explicit event'],inputs:'Public synthetic native schema; not real Agent-host certification',realAgentConfigurationsChanged:false};
  await Bun.write(join(owned,'result.json'),JSON.stringify(result,null,2));console.log(JSON.stringify(result));
 }catch(error){await Bun.write(join(owned,'failure.json'),JSON.stringify({error:String(error),stderr},null,2));throw error;}finally{db.close();}
});
