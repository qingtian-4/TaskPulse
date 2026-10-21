/** Real executable stdin/exit-code acceptance. Every write stays under an
 * owned temporary TASKPULSE_HOME; no real host configuration or Agent work. */
import { mkdtemp, mkdir, copyFile, readdir, readFile } from 'node:fs/promises';
import { resolve, join, dirname } from 'node:path';
import { spawnSync, spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
const at=process.argv.indexOf('--exe');
if(at<0||!process.argv[at+1])throw Error('--exe is required; never guess a running user executable');
const originalExe=resolve(process.argv[at+1]);
const binary=await readFile(originalExe);
const pe=binary.readUInt32LE(0x3c), subsystem=binary.readUInt16LE(pe+24+68);
if(process.argv.includes('--require-gui')&&subsystem!==2)throw Error('Requires an actual Windows GUI-subsystem release executable');
const owned=await mkdtemp(resolve('src-tauri/target/integration-bridge-'));
const installed=join(owned,"安装 目录 & team's app");await mkdir(installed);
const exe=join(installed,'agent-tray.exe');await copyFile(originalExe,exe);
// GNU development/release builds dynamically import Microsoft's loader;
// MSVC bundles may use the static loader. Copy only the build-owned companion.
const loader=join(dirname(originalExe),'WebView2Loader.dll');
if(await Bun.file(loader).exists())await copyFile(loader,join(installed,'WebView2Loader.dll'));
const home=join(owned,'profile');await mkdir(home);
const env={...process.env,TASKPULSE_HOME:home};delete env.WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS;
const hash=(s:string)=>createHash('sha256').update(s).digest('hex');
const source='bridge-fixture';
const config=`[dashboard]\nversion = 2\nonboarding_complete = true\nnotifications = false\nnotification_mode = "popup"\nsound = false\ndefault_view = "tasks"\n[[dashboard.sources]]\nid = "${source}"\nname = "合成接收测试"\nkind = "generic_event"\nenabled = true\nlocation = ""\nprojects = ["*"]\nexcluded_projects = []\ntmux_sessions = []\ntmux_panes = []\npoll_interval_ms = 1000\n[dashboard.sources.integration]\nagent_id = "custom-agent"\nadapter_id = "standard-events"\ntransport = "local"\naccept_since_ms = 0\n`;
await Bun.write(join(home,'config.toml'),config);
const call=(args:string[],input:string|Buffer)=>{
  const r=spawnSync(exe,args,{input,encoding:'utf8',env,windowsHide:true,timeout:10000,maxBuffer:1024*1024});
  if(r.error)throw r.error;return {code:r.status,stdout:r.stdout,stderr:r.stderr};
};
const start={schema_version:1,agent_id:'custom-agent',session_id:'synthetic/中文:session',turn_id:'turn-a',event_id:'event-start',kind:'turn.started',payload:{title:'合成测试，不是真实宿主任务'}};
const flags=['--integration-event','--source',source,'--adapter','standard-events'];
const valid=call(['--integration-validate'],JSON.stringify(start));if(valid.code!==0||!JSON.parse(valid.stdout).valid)throw Error('valid input rejected');
const bad=call(['--integration-validate'],JSON.stringify({...start,schema_version:99}));if(bad.code===0||JSON.parse(bad.stdout).valid)throw Error('validator hid invalid schema');
const accepted=call(flags,JSON.stringify(start));if(accepted.code!==0||accepted.stdout!=='')throw Error('Hook must be silent and fail-open');
const journalDir=join(home,'integrations',hash(source),'events');
const paths=await readdir(journalDir);if(paths.length!==1||paths[0]!==hash(start.session_id)+'.jsonl')throw Error('Identity escaped controlled encoding');
const journal=join(journalDir,paths[0]);const before=await Bun.file(journal).text();
for(const input of [JSON.stringify({...start,source_instance_id:'spoof'}),JSON.stringify({...start,payload:{prompt:'SYNTHETIC_PRIVATE_DATA'}}),'x'.repeat(65537)]){
  const rejected=call(flags,input);if(rejected.code!==0||rejected.stdout!=='')throw Error('Rejected hook changed host behavior');
}
if(await Bun.file(journal).text()!==before)throw Error('Invalid input reached journal');
const diagnostic=call(['--integration-diagnose','--source',source],'');const report=JSON.parse(diagnostic.stdout);
if(diagnostic.code!==0||report.events!==1||JSON.stringify(report).includes('SYNTHETIC_PRIVATE'))throw Error('Invalid or leaking diagnostics');
const test=call(flags,JSON.stringify({...start,kind:'connection.test',event_id:'test-event'}));
if(test.code!==0||await Bun.file(journal).text()!==before)throw Error('Connection test became a monitored task');
// Holding an OS byte-range lock overlaps Rust's whole-file lock. The child
// must leave silently after its bounded lock timeout, never hang the host.
let lockTimeoutVerified=false;
if(process.platform==='win32'){
  const lockPath=join(home,'integrations',hash(source),'writer.lock');
  const script=`$tpFile=[IO.File]::Open('${lockPath.replaceAll("'","''")}',[IO.FileMode]::OpenOrCreate,[IO.FileAccess]::ReadWrite,[IO.FileShare]::ReadWrite);try{$tpFile.Lock(0,1);[Console]::Out.WriteLine('locked');[Console]::In.ReadLine()|Out-Null}finally{$tpFile.Dispose()}`;
  const holder=spawn('powershell.exe',['-NoLogo','-NoProfile','-NonInteractive','-Command',script],{env,windowsHide:true,stdio:['pipe','pipe','pipe']});
  try{
    await new Promise<void>((ok,fail)=>{const timer=setTimeout(()=>fail(Error('Lock helper timeout')),10000);holder.stdout.once('data',data=>{clearTimeout(timer);String(data).includes('locked')?ok():fail(Error('Lock helper failed'));});holder.once('error',fail);});
    const began=performance.now();const blocked=call(flags,JSON.stringify({...start,event_id:'locked-write'}));const elapsed=performance.now()-began;
    const d=call(['--integration-diagnose','--source',source],'');
    if(blocked.code!==0||blocked.stdout!==''||elapsed>5000||JSON.parse(d.stdout).last_error?.code!=='lock_timeout')throw Error('Bounded fail-open lock handling failed');
    lockTimeoutVerified=true;
  }finally{holder.stdin.end('\n');await new Promise<void>(ok=>{if(holder.exitCode!==null)ok();else{holder.once('exit',()=>ok());setTimeout(()=>{holder.kill();ok();},3000).unref();}});}
}
const result={passed:true,guiSubsystem:subsystem===2,checks:['actual redirected stdin','machine-readable validator failure','silent Hook mode','bounded size and identity rejection','private payload omitted','separate connection-test channel','space/Unicode/apostrophe/metacharacter executable path'],lockTimeoutVerified,realHostConfigurationsChanged:false};
await Bun.write(join(owned,'result.json'),JSON.stringify(result,null,2));
console.log(JSON.stringify({...result,evidence:owned}));
