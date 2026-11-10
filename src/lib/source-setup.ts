import type { DashboardSettings, MonitorSource, ProviderHealth, SourceKind, SourceDiscovery, SourceSnapshot, SourceReadState } from './types';

export type CapabilityEvidence='unsupported'|'explicit'|'inferred'|'hook'|'producer';
export interface SourceCapabilities {
  discovery:CapabilityEvidence; model:CapabilityEvidence; turn_start:CapabilityEvidence;
  turn_end:CapabilityEvidence; approval:CapabilityEvidence; question:CapabilityEvidence;
  return_target:'details'|'host_window';
}
export interface SourceDefinition {
  description?:string; id:string; name:string; kind:SourceKind; category:string;
  location_kind:string; platforms:string[]; adapter_id?:string; agent_id?:string;
  multiple_instances?:boolean; auto_enable_on_discovery?:boolean;
  capabilities?:SourceCapabilities; verified_platforms?:string[]; default_source?:MonitorSource;
}
export function sourceDefinitionFor(catalog:SourceDefinition[],source:MonitorSource):SourceDefinition {
  return catalog.find(d=>d.kind===source.kind&&(d.kind!=='generic_event'||d.adapter_id===source.integration?.adapter_id))
    ??{id:source.id,name:source.name,kind:source.kind,category:'custom',location_kind:'none',platforms:[]};
}
// Use the same reviewable rows on the panel and on the settings landing page.
export function localDiscoveryCandidates(catalog:SourceDefinition[], found:SourceDiscovery[], settings:DashboardSettings|null){
  if(!settings?.onboarding_complete)return [];
  const seen=new Set(settings.auto_discovery_seen??[]);
  return catalog.filter(d=>d.category==='local'&&d.kind!=='scanner').flatMap(definition=>{
    if(settings.sources.some(s=>s.kind===definition.kind))return [];
    const row=found.find(d=>d.data_location_detected===true && d.candidate?.kind===definition.kind
      && !seen.has(d.candidate.id) && !settings.sources.some(s=>s.id===d.candidate?.id));
    return row?.candidate?[{definition,candidate:row.candidate}]:[];
  });
}
export interface SourceCatalogRow {
  definition:SourceDefinition; source?:MonitorSource; candidate?:MonitorSource; observation?:SourceSnapshot;
  supported:boolean; locationDetected:boolean|null; dataDetected:boolean|null;
}
// Short labels are presentation only. Manifests and persisted source IDs
// continue to carry the original adapter identity.
const LOCAL_HOOKS:Readonly<Record<string,SourceKind>>={
  'kimi-code-cli':'kimi_code','trae-cn-ide':'trae','zcode-desktop':'zcode',
};
export function localHookKind(adapterId:string|undefined):SourceKind|undefined {
  return adapterId&&Object.hasOwn(LOCAL_HOOKS,adapterId)?LOCAL_HOOKS[adapterId]:undefined;
}
export function eventConnections(catalog:SourceDefinition[],sources:MonitorSource[]){
  return catalog.flatMap(definition=>{
    const connections=sources.filter(s=>s.kind==='generic_event'&&s.integration?.adapter_id===definition.adapter_id);
    return connections.length?[{definition,connections}]:[];
  });
}
export function isDefaultEventName(name:string,definition:SourceDefinition):boolean {
  return name===definition.name||name===eventAdapterLabel(definition.adapter_id??'',definition.name)||
    definition.adapter_id==='standard-events'&&[
      '其他 Agent','其他 Agent · 通用事件','其他 Agent（标准事件）','Other Agent','Other agents (standard events)','Generic events (MCP / scripts)','通用事件（MCP / 脚本）','MCP or scripts',
    ].includes(name);
}
export function eventAdapterLabel(adapterId:string,displayName:string):string {
  return ({
    'standard-events':'MCP 或脚本',
    'kimi-code-cli':'Kimi Code',
    'trae-cn-ide':'Trae',
    'zcode-desktop':'ZCode',
    'workbuddy-candidate':'WorkBuddy',
  } as Record<string,string>)[adapterId]??displayName;
}
export function createEventSource(definition:SourceDefinition,name=definition.name):MonitorSource {
  if(definition.kind!=='generic_event'||!definition.adapter_id||!definition.agent_id)throw Error('invalid_event_definition');
  return {id:`generic_event:${crypto.randomUUID()}`,kind:'generic_event',name,location:'',enabled:false,projects:['*'],excluded_projects:[],tmux_sessions:[],tmux_panes:[],poll_interval_ms:3000,integration:{agent_id:definition.agent_id,adapter_id:definition.adapter_id,transport:'local',accept_since_ms:0}};
}
export function sourceCatalogRows(catalog:SourceDefinition[],sources:MonitorSource[],discovery:SourceDiscovery[],category:string,platform:string,foundOnly:boolean,live:SourceSnapshot[] = []):SourceCatalogRow[] {
  const saved=sources.filter(s=>sourceDefinitionFor(catalog,s).category===category)
    .map(source=>{
      const definition=sourceDefinitionFor(catalog,source);
      const observation=live.find(row=>row.id===source.id&&row.kind===source.kind&&row.location===source.location);
      return {definition,source,observation,candidate:undefined as MonitorSource|undefined,supported:catalog.includes(definition) && (!definition.platforms.length || definition.platforms.includes(platform)),
        locationDetected:discovery.find(d=>d.id===source.id&&d.location===source.location)?.data_location_detected??null,
        dataDetected:observation?.data_detected??null};
    });
  const supported=catalog.filter(d=>d.category===category&&(!d.platforms.length||d.platforms.includes(platform)));
  const additions=supported.filter(d=>d.multiple_instances||!saved.some(r=>r.definition.id===d.id))
    .flatMap<SourceCatalogRow>(definition=>{
      const matches=discovery.filter(d=>d.candidate && (d.id===definition.id || d.candidate.kind===definition.kind && definition.kind!=='generic_event'));
      const candidates=matches.filter(d=>d.data_location_detected===true&&d.candidate&&
        !sources.some(s=>s.id===d.candidate!.id||s.kind===d.candidate!.kind&&s.location.toLowerCase()===d.candidate!.location.toLowerCase()));
      const detected:SourceCatalogRow[]=candidates.map(d=>({definition,supported:true,locationDetected:true,dataDetected:null,candidate:d.candidate}));
      if(detected.length&&!definition.multiple_instances)return detected;
      const locationDetected=matches.some(d=>d.data_location_detected===false) ? false : null;
      return [...detected,{definition,supported:true,locationDetected,dataDetected:null}];
    });
  return [...saved,...additions].filter(r=>!foundOnly||r.dataDetected===true||r.locationDetected===true);
}

export function sourceSections(catalog:SourceDefinition[],settings:DashboardSettings,discovery:SourceDiscovery[],platform:string,live:SourceSnapshot[],selected:string) {
  const result:{primary:SourceCatalogRow[];supported:SourceCatalogRow[];advanced:SourceCatalogRow[]}={primary:[],supported:[],advanced:[]};
  for(const category of ['local','remote','custom']){
    for(const row of sourceCatalogRows(catalog,settings.sources,discovery,category,platform,false,live)){
      const source=row.source, seed=row.definition.default_source;
      const configured=source && (source.id===selected || source.enabled || settings.auto_discovery_seen?.includes(source.id) ||
        !seed || source.id!==seed.id || source.name!==seed.name || source.location!==seed.location ||
        source.poll_interval_ms!==seed.poll_interval_ms || JSON.stringify(source.projects)!==JSON.stringify(seed.projects) ||
        !!(source.excluded_projects.length||source.tmux_sessions.length||source.tmux_panes.length||source.integration));
      if(configured || category==='local'&&(row.locationDetected===true||row.dataDetected===true))result.primary.push(row);
      else if(category==='local')result.supported.push(row);
      else result.advanced.push(row);
    }
  }
  return result;
}

export const CAPABILITY_NAMES = {
  discovery:'发现会话',model:'模型信息',turn_start:'本轮开始',turn_end:'本轮结束',approval:'审批请求',question:'输入请求',
} as const;
export const CAPABILITY_EVIDENCE:Record<CapabilityEvidence,string>={
  unsupported:'不支持',explicit:'明确证据',inferred:'有限推断',hook:'需自愿接入 Hook',producer:'由事件生产者提供',
};

const READ_LABELS:Record<SourceReadState,string>={
  disabled:'未读取',waiting:'等待首次读取',healthy:'来源读取正常',
  partial:'部分会话需检查',failed:'读取异常 · 需检查',stale:'读取结果待更新',
};
export function sourceReadState(observation:SourceSnapshot|undefined, enabled:boolean):string {
  return READ_LABELS[enabled ? observation?.enabled ? observation.read_state : 'waiting' : 'disabled'];
}
export function summarizeSources(sources:SourceSnapshot[]) {
  const enabled=sources.filter(s=>s.enabled);
  return {enabled:enabled.length,waiting:enabled.filter(s=>s.read_state==='waiting').length,
    issues:enabled.filter(s=>['partial','failed','stale'].includes(s.read_state)).length};
}

// A visual hint only. Reaching step 4 is NOT host lifecycle certification.
export function sourceSetupStep(hasSource:boolean,draft:boolean,readTestPassed:boolean):number {
  return !hasSource?1:draft?2:readTestPassed?4:3;
}

export function readTestResult(id:string, health:ProviderHealth[], since:number):'pending'|'passed'|'failed' {
  const h=health.find(h=>h.provider===id);
  if(!h?.last_attempt || h.last_attempt<since)return 'pending';
  return h.status==='connected' && (h.last_success??0)>=since?'passed':'failed';
}

const notes:Record<SourceKind,string>={
  kimi_code:'只读 Kimi 原生 wire 日志；能力随日志版本而异。',
  qoder:'只读 Qoder Desktop 轮次投影及 IDE/CLI 转录；原生审批和提问尚未提供。',
  trae:'公开会话发现；当前非标准会话库的任务状态读取受限。',
  opencode:'只读本机 OpenCode 会话、模型和运行记录；明确本轮结束才提醒，不从工具返回推断完成。提问与审批事件尚不提供。',
  workbuddy:'只读 WorkBuddy 本地数据库与会话记录；关注新执行，不导入空闲历史，不包含云端任务。提问与审批事件尚不提供。',
  zcode:'自动读取本机 ZCode 会话、项目和模型；工具执行完成不会当作整轮结束。',
  doubao:'读取豆包原生执行检查点；已验证 Windows 2.26.10。检查点移除只标为待更新，不推断完成；审批提醒尚未适配。',
  codex:'原生标题、项目与 rollout 执行阶段；仅明确结束事件提醒。命令审批需可选的官方观察 Hook，安装后还需在 Codex /hooks 中信任。',
  codex_wsl:'独立读取已运行发行版的 Codex 数据；不唤醒 WSL，不借用 Windows 额度。命令审批需在该发行版安装并信任观察 Hook。',
  remote_codex:'读取已配置 SSH 主机的 Codex 会话；依赖主机可达、权限与远端数据格式。',
  claude_code:'原生会话读取为尽力推断；精确生命周期需自愿配置官方 Hook，与代理模型无关。',
  hermes:'原生标题、项目和运行租约；没有明确结束证据时不猜测任务完成。',
  deepseek_desktop:'读取原生 Desktop / CLI 存储，支持旧版与 v4 日志；状态能力取决于宿主记录。',
  deepseek_wsl:'独立读取已启用 WSL 安装，不与 Windows 桌面会话混合。',
  tmux:'普通 shell 返回仅为推断结束，退出码未知；前台 tp-run 可报告直接子进程退出。',
  hooks:'读取兼容状态文件；状态准确性由写入方决定，不自动安装宿主 Hook。',
  scanner:'只提供受限进程证据；不能用 CPU、静默日志或进程消失证明本轮结束。',
  generic_event:'通过已连接的 Agent 事件跟踪任务；具体支持的状态见接入说明。',
};
export function sourceCapability(kind:SourceKind):string{return notes[kind];}
