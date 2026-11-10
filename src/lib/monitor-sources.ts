import type {DashboardSettings,MonitorSource,SourceSnapshot,TaskSnapshot,SourceKind} from './types';

export type AgentKind=SourceKind;
export interface AgentProbe {state:'readable'|'empty'|'missing'|'read_error'|'unsupported_format'|'not_checked'|'unsupported';data_detected:boolean;projects:string[]}
export interface SshProbe {host:string;checked_at:number;connection:'connected'|'disconnected';codex:AgentProbe;claude_code:AgentProbe;message:string}
export interface ServerGroup {host:string;name:string;sources:MonitorSource[];codex?:MonitorSource;paused:boolean}
export const remoteSource=(s:MonitorSource)=>s.kind==='remote_codex'||s.kind==='tmux';
export const canRead=(p?:AgentProbe)=>p?.state==='readable'||p?.state==='empty';
export function servers(settings:DashboardSettings):ServerGroup[]{
  const rows=new Map<string,MonitorSource[]>();
  for(const s of settings.sources.filter(remoteSource)){const list=rows.get(s.location)??[];list.push(s);rows.set(s.location,list);}
  return [...rows].map(([host,sources])=>{
    const codex=sources.find(s=>s.kind==='remote_codex');
    return {host,sources,codex,name:codex?.name.replace(/^Codex · /,'')||host,paused:sources.some(s=>(settings.ssh_paused_sources??[]).includes(s.id))};
  });
}
export function createSource(kind:AgentKind|'remote_codex',location='',name?:string):MonitorSource {
  return {id:kind==='codex'?'codex':kind==='scanner'?'scanner':kind==='hooks'?'local_file':['kimi_code','qoder','trae'].includes(kind)?kind:`${kind}:${crypto.randomUUID()}`,kind,name:name??(kind==='remote_codex'?location:kind==='codex'?'Codex':kind==='claude_code'?'Claude Code':kind),location,enabled:false,projects:['*'],excluded_projects:[],tmux_sessions:[],tmux_panes:[],poll_interval_ms:3000};
}
export function pauseServer(settings:DashboardSettings,host:string,pause:boolean):DashboardSettings {
  const next=structuredClone(settings);const restore=new Set(next.ssh_paused_sources??[]);
  for(const s of next.sources.filter(s=>remoteSource(s)&&s.location===host)){
    if(pause){if(s.enabled){restore.add(s.id);s.enabled=false;}}
    else if(restore.has(s.id)){s.enabled=true;restore.delete(s.id);}
  }
  next.ssh_paused_sources=[...restore];return next;
}
export function removeServer(settings:DashboardSettings,host:string):DashboardSettings {
  const next=structuredClone(settings);const ids=new Set(next.sources.filter(s=>remoteSource(s)&&s.location===host).map(s=>s.id));
  next.sources=next.sources.filter(s=>!ids.has(s.id));next.ssh_paused_sources=(next.ssh_paused_sources??[]).filter(id=>!ids.has(id));return next;
}
export function projectChoices(source:MonitorSource,tasks:TaskSnapshot[],extra:string[]=[]):string[]{
  return [...new Set([...source.projects.filter(p=>p!=='*'),...extra,...tasks.filter(t=>t.source_instance===source.id).map(t=>t.project_path||t.project||'')].filter(Boolean))];
}
export function sourceStatus(source:MonitorSource|undefined,observed?:SourceSnapshot):string {
  if(!source?.enabled)return '未启用';
  if(!observed||observed.read_state==='waiting')return '等待首次读取';
  return ({healthy:observed.data_detected?'读取正常':'读取正常 · 暂无近期会话',partial:'部分数据待更新',failed:'读取异常',stale:'数据待更新',disabled:'已暂停',waiting:'等待首次读取'} as const)[observed.read_state];
}
export function localSourceStatus(
  source:MonitorSource|undefined,observed:SourceSnapshot|undefined,locationDetected:boolean|null|undefined,
  diagnostic?:string|null,
):string {
  if(source?.kind==='doubao'&&source.enabled&&observed?.read_state==='failed'){
    if(diagnostic==='豆包桌面端未运行')return diagnostic;
    if(diagnostic==='未找到支持的豆包工作任务记录；普通聊天不在监控范围内')return '暂无可读取的豆包工作任务';
  }
  if(!source?.enabled && locationDetected===true)return '已检测到 · 未启用';
  if(locationDetected===false && !observed?.data_detected
    && (!source?.enabled || observed?.read_state==='healthy'))return '未检测到本机数据';
  return sourceStatus(source,observed);
}
export function scopeLabel(projects:string[]):string{return projects.includes('*')?'全部项目':projects.length?'指定项目':'尚未选择项目';}
