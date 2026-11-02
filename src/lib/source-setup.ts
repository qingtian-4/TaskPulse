import type { DashboardSettings, MonitorSource, ProviderHealth, SourceKind, SourceDiscovery } from './types';

export interface SourceDefinition {
  description?:string; id:string; name:string; kind:SourceKind; category:string;
  location_kind:string; platforms:string[]; adapter_id?:string; agent_id?:string;
  multiple_instances?:boolean;
}
export function sourceDefinitionFor(catalog:SourceDefinition[],source:MonitorSource,platform:string):SourceDefinition {
  return catalog.find(d=>d.kind===source.kind&&(d.kind!=='generic_event'||d.adapter_id===source.integration?.adapter_id))
    ??{id:source.id,name:source.name,kind:source.kind,category:'custom',location_kind:'none',platforms:[platform]};
}
export function sourceCatalogRows(catalog:SourceDefinition[],sources:MonitorSource[],discovery:SourceDiscovery[],category:string,platform:string,foundOnly:boolean) {
  const saved=sources.filter(s=>sourceDefinitionFor(catalog,s,platform).category===category)
    .map(source=>({definition:sourceDefinitionFor(catalog,source,platform),source,found:discovery.find(d=>d.id===source.id)?.available??false}));
  const available=catalog.filter(d=>d.category===category&&(!d.platforms.length||d.platforms.includes(platform)));
  const additions=available.filter(d=>d.multiple_instances||!saved.some(r=>r.definition.id===d.id))
    .map(definition=>({definition,source:undefined as MonitorSource|undefined,found:discovery.some(d=>d.available&&(d.id===definition.id||d.candidate?.kind===definition.kind&&definition.kind!=='generic_event'))}));
  return [...saved,...additions].filter(r=>!foundOnly||r.found);
}

// Readability is a transport observation, never proof of Agent lifecycle support.
export function sourceReadState(source:MonitorSource, applied:DashboardSettings|null, health:ProviderHealth[], now:number):string {
  const saved=applied?.sources.find(s=>s.id===source.id);
  if(!saved || !applied?.onboarding_complete)return '未保存 · 尚未监控';
  if(!saved.enabled)return '已保存 · 已停用';
  const h=health.find(h=>h.provider===source.id);
  if(!h?.last_attempt)return '已保存 · 等待首次读取';
  if(h.status==='degraded')return (saved.kind==='codex'||saved.kind==='codex_wsl') &&
    now>=h.last_attempt && now-h.last_attempt<=30000 &&
    Object.values(h.task_observations??{}).some(proof=>proof.observed_at===h.last_attempt)?'部分会话需检查':'读取异常 · 需检查';
  if(!h.last_success || now-h.last_success>Math.max(30000,saved.poll_interval_ms*3))return '读取结果待更新';
  return '来源读取正常';
}

export function sourceReadTone(source:MonitorSource, applied:DashboardSettings|null, health:ProviderHealth[], now:number):'neutral'|'stale'|'success' {
  const saved=applied?.sources.find(s=>s.id===source.id);
  if(!saved || !applied?.onboarding_complete || !saved.enabled)return 'neutral';
  const h=health.find(h=>h.provider===source.id);
  return h?.status==='connected' && !!h.last_attempt && !!h.last_success && now-h.last_success<=Math.max(30000,saved.poll_interval_ms*3)?'success':'stale';
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
