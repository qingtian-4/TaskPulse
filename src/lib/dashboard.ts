import { SOURCE_LABEL, formatAgo, formatElapsed, type DashboardSettings, type TaskSnapshot, type ProviderHealth } from './types';

export type TaskSection = 'ongoing' | 'results' | 'uncertain' | 'pinned';
export type TaskSignal = 'approval' | 'question' | 'failed' | 'action';
export interface DashboardFilters { source: string; project: string; location: string; section: TaskSection | null; signal?: TaskSignal | null; query?:string; unreadOnly?:boolean }
export const EMPTY_FILTERS: DashboardFilters = {source:'',project:'',location:'',section:null,signal:null,query:'',unreadOnly:false};
export function shouldApplyDashboardSnapshot(currentRevision:number,incomingRevision:number,requestGeneration:number,currentRequestGeneration:number):boolean {
  return requestGeneration===currentRequestGeneration && incomingRevision>=currentRevision;
}
export const SECTION_ORDER: TaskSection[] = ['ongoing', 'results', 'uncertain', 'pinned'];
export const SECTION_NAMES: Record<TaskSection,string> = {ongoing:'进行中',results:'结果待查看',uncertain:'状态待更新',pinned:'固定与已查看'};
// Apply the same projection the native tray used for this revision. Raw
// lifecycle/attention data remains unchanged in the snapshot.
export function projectDashboardTasks(tasks:TaskSnapshot[], unconfirmed:string[]):TaskSnapshot[] {
  const ids=new Set(unconfirmed);
  return tasks.map(task=>ids.has(task.id) ? {...task,stale:true} : task);
}
export function sectionRank(task: TaskSnapshot): number { return SECTION_ORDER.indexOf(taskSection(task)); }
export function matchesSignal(task:TaskSnapshot, signal:TaskSignal):boolean {
  if(signal==='failed')return task.status==='failed' && isEndedResult(task);
  return !task.stale && needsAction(task) && (signal==='action' || task.waiting_reason===signal);
}

export function filterTasks(tasks: TaskSnapshot[], filters: DashboardFilters, settings?: DashboardSettings | null): TaskSnapshot[] {
  const terms=(filters.query??'').trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  return tasks.filter(t => (!filters.source || taskSource(t,settings)?.id === filters.source) &&
    (!filters.project || projectKey(t,settings) === filters.project) &&
    (!filters.location || (t.host ?? '本机') === filters.location) &&
    (filters.section === null || taskSection(t) === filters.section) &&
    (!filters.signal || matchesSignal(t,filters.signal)) && (!filters.unreadOnly || t.unread===true) &&
    (!terms.length || terms.every(word=>[t.name,t.project,t.project_path,t.agent_name,t.model,SOURCE_LABEL[t.source],taskContext(t,settings),t.host,taskActivity(t)].filter(Boolean).join(' ').toLocaleLowerCase().includes(word))));
}
export function compareTasks(a:TaskSnapshot,b:TaskSnapshot):number {
  return sectionRank(a)-sectionRank(b) || Number(matchesSignal(b,'action'))-Number(matchesSignal(a,'action')) || (b.attention_since??b.updated_at)-(a.attention_since??a.updated_at) || a.id.localeCompare(b.id);
}
export function taskAgent(task:TaskSnapshot):string {return task.agent_name ?? SOURCE_LABEL[task.source];}
// Visual semantics only: never derive an end, approval or read acknowledgement.
export function taskTone(task:TaskSnapshot):'running'|'action'|'danger'|'unread'|'stale'|'neutral' {
  if(task.stale || task.status==='unknown' || task.status==='disconnected')return 'stale';
  if(needsAction(task))return 'action';
  if(task.status==='failed')return 'danger';
  if(task.status==='running')return 'running';
  if(isEndedResult(task))return 'unread';
  return 'neutral';
}
export function taskSummary(task:TaskSnapshot):string {
  if(task.stale)return task.status==='running'?'上次确认为运行中，当前采集待更新':'上次观测 · 等待来源更新';
  if(needsAction(task))return task.waiting_reason==='approval'?'等待你在原 Agent 中批准':'等待你在原 Agent 中回答';
  if(task.status==='running')return taskActivity(task);
  if(isEndedResult(task))return task.message || (task.status==='failed'?'执行失败 · 结果尚未查看':'本轮结果尚未查看');
  return task.unread?'尚未查看':'已查看 · 保留关注';
}

// Global counters mean ALL monitored tasks, not the intersection with a
// forgotten project/source filter. One click must show the counted tasks.
export function overviewFilters(section: TaskSection, currentSection: TaskSection | null): DashboardFilters {
  return {...EMPTY_FILTERS,section:section === currentSection ? null : section};
}

export function runningOutsideView(tasks: TaskSnapshot[], visible: TaskSnapshot[]): TaskSnapshot[] {
  const ids=new Set(visible.map(t=>t.id));
  return tasks.filter(t=>t.status === 'running' && !t.stale && !ids.has(t.id));
}
// Keep uncertainty separate from fresh ongoing counts. The global hint remains
// visible even when search/source/ongoing filters exclude the stale task.
export function unconfirmedRunning(tasks:TaskSnapshot[]):TaskSnapshot[] {
  return tasks.filter(t=>t.status==='running' && t.stale===true);
}
// Read-only DOM diagnostics: real frontend classification/filter counts, no
// task titles, identifiers, search text, activity, paths or user responses.
export function dashboardContinuity(tasks:TaskSnapshot[], visible:TaskSnapshot[]) {
  const count=(list:TaskSnapshot[])=>{
    const sections:Record<TaskSection,number>={ongoing:0,results:0,uncertain:0,pinned:0};
    for(const task of list) sections[taskSection(task)]++;
    return {...sections,total:list.length,unconfirmed_running:unconfirmedRunning(list).length};
  };
  const sources=[...new Set(tasks.map(t=>t.source_instance??t.source))];
  return {global:count(tasks),visible:count(visible),sources:sources.slice(0,128).map(source=>({source_id:source,global:count(tasks.filter(t=>(t.source_instance??t.source)===source)),visible:count(visible.filter(t=>(t.source_instance??t.source)===source))})),sources_truncated:sources.length>128};
}
export function refreshResultMessage(ids:string[], failedIds:string[], labels:Record<string,string>={}):string {
  const failed=ids.filter(id=>failedIds.includes(id));
  return failed.length ? '本次刷新后仍待更新：'+failed.map(id=>labels[id]??id).join('、')+'；保留上次观测，不代表任务结束' : '已收到全部启用来源的新采集结果';
}

export function filterDescription(filters: DashboardFilters, tasks: TaskSnapshot[], settings?: DashboardSettings | null): string {
  return [filters.source && `来源：${settings?.sources.find(s=>s.id===filters.source)?.name ?? filters.source}`,
    filters.project && `项目：${tasks.find(t=>projectKey(t,settings)===filters.project)?.project ?? '已不在当前列表'}`,
    filters.location && `位置：${filters.location}`,
    filters.section !== null && `状态：${SECTION_NAMES[filters.section]}`,
    filters.signal && `提醒：${{approval:'待批准',question:'待回答',failed:'失败结果',action:'需要处理'}[filters.signal]}`,
    filters.query?.trim() && `搜索：${filters.query.trim()}`,filters.unreadOnly && '仅未查看'].filter(Boolean).join(' · ');
}

export function needsAction(task: TaskSnapshot): boolean {
  return task.status === 'waiting_input' && (task.waiting_reason === 'approval' || task.waiting_reason === 'question' || task.waiting_reason === 'quota_exhausted');
}
export function taskLabel(task: TaskSnapshot): string {
  if (task.outcome === 'interrupted' && task.finished_at) return '已中断';
  if (task.stale) return '状态待更新';
  if (needsAction(task)) return task.waiting_reason === 'quota_exhausted' ? '额度已用完' : task.waiting_reason === 'approval' ? '需要授权' : '等待输入';
  if (isInferredCommandEnd(task)) return '观察到结束 · 退出码未知';
  if (task.status === 'running') return '运行中';
  if (task.status === 'waiting_input') return task.waiting_reason === 'turn_ended' ? '本轮结束' : '状态待确认';
  return { succeeded: '已完成', failed: '失败', idle: '空闲', disconnected: '已断开', unknown: '状态未知' }[task.status];
}
// Subphase is presentation, never a lifecycle/notification heuristic. Missing
// phase must not be advertised as thinking, or a completed command as current.
export function taskActivity(task: TaskSnapshot): string {
  if (task.status !== 'running') return task.message ?? '来源未提供最近消息';
  switch (task.phase) {
    case 'thinking': return '正在思考';
    case 'responding': return '正在生成回复';
    case 'awaiting_model': return '等待模型响应 · 本轮仍在运行';
    case 'awaiting_tool': return task.activity ? '等待工具响应 · '+task.activity : '等待工具响应 · 本轮仍在运行';
    case 'tool': return task.activity ?? '正在执行工具 · 来源未提供具体指令';
    default: return task.activity ?? '运行中 · 来源未提供执行阶段';
  }
}
// The Standard Events reducer owns these terminal messages and overwrites any
// producer text. Identical native/user messages must remain untranslated data.
export function taskSummaryIsAppCopy(task:TaskSnapshot):boolean {
  if(!task.message||task.stale||task.status==='running'||needsAction(task))return true;
  if(task.source!=='generic_event'||!isEndedResult(task))return false;
  const messages:Record<string,string>={turn_ended:'本轮结束，尚未查看',failed:'来源明确报告本轮失败',interrupted:'本轮已中断，非成功'};
  return task.message===messages[task.outcome??''];
}
export function taskSection(task: TaskSnapshot): TaskSection {
  if (task.stale) return 'uncertain';
  if (task.status === 'running' || needsAction(task)) return 'ongoing';
  if (isEndedResult(task)) return 'results';
  if (task.status === 'unknown' || task.status === 'disconnected' || task.status === 'waiting_input' && task.waiting_reason !== 'turn_ended') return 'uncertain';
  return 'pinned';
}

// Process detection alone does not establish a named user task. Keep these
// observations in the source catalog without manufacturing attention cards.
// Never classify a native session or a subagent from its display name alone.
export function dashboardCandidates(tasks:TaskSnapshot[]):TaskSnapshot[] {
  return tasks.filter(task=>!task.internal && (task.pinned || !(
    task.confidence==='inferred' && (task.source_instance==='scanner' || task.id.startsWith('scan:')) &&
    !task.session_id?.trim() && !task.project?.trim() && !task.project_path?.trim() &&
    (!task.name.trim() || task.name.trim().toLowerCase()==='unknown')
  )));
}

// Keep a known open turn in the default panel when its liveness expires.
// Uncertainty remains visible as such and never contributes to live counts.
export function defaultDashboardTasks(tasks:TaskSnapshot[]):TaskSnapshot[] {
  return dashboardCandidates(tasks).filter(task=>task.pinned || taskSection(task)==='ongoing' || isEndedResult(task)
    || (task.stale && task.status==='running' && task.finished_at==null && !!task.session_id?.trim() && !!task.turn_key?.trim()));
}
export function taskSource(task: TaskSnapshot, settings?: DashboardSettings | null) {
  if(!task.remote&&task.id.startsWith('file:'))return settings?.sources.find(s=>s.id==='local_file'&&s.kind==='hooks');
  if(task.source_instance)return settings?.sources.find(s => s.id === task.source_instance);
  const nativeId = task.source === 'deepseek_harness'
    ? (task.id.startsWith('dsh-native:') ? 'deepseek_harness_desktop' : 'deepseek_harness')
    : task.source === 'claude_code' && task.id.startsWith('claude:') ? 'claude_code'
    : task.source === 'claude_code' || task.source === 'gemini' ? 'scanner' : task.source;
  return (task.remote
    ? settings?.sources.find(s => s.location === task.host && s.kind === (task.source === 'codex' ? 'remote_codex' : task.source === 'claude_code' ? 'remote_claude_code' : 'tmux'))
    : settings?.sources.find(s => s.id === nativeId));
}
export function taskProjectLabel(task: TaskSnapshot): string {
  return task.project ?? (task.source === 'doubao' ? '独立会话' : '未关联项目');
}
export function taskContext(task: TaskSnapshot, settings?: DashboardSettings | null): string {
  const source = taskSource(task, settings);
  return `${taskProjectLabel(task)} · ${source?.name ?? `${SOURCE_LABEL[task.source]} · ${task.host ?? '本机'}`}`;
}
export function projectKey(task: TaskSnapshot, settings?: DashboardSettings | null): string {
  return `${taskSource(task, settings)?.id ?? task.source_instance ?? task.host ?? task.source}:${task.project_path ?? task.project ?? '@unassociated'}`;
}

export function sourceFreshness(task: TaskSnapshot, settings: DashboardSettings | null, providers: ProviderHealth[], now: number): string {
  const source = taskSource(task, settings);
  const health = providers.find(p => p.provider === (source?.id ?? task.source_instance));
  if(health?.status==='degraded' && source?.enabled && (source.kind==='codex'||source.kind==='codex_wsl'||source.kind==='remote_claude_code'||source.kind==='kimi_code'||source.kind==='qoder') && !task.stale && (task.status==='running'||needsAction(task)) && health.task_observations?.[task.id])return '该会话已更新；同一来源的其他会话需检查';
  if (!health?.last_success) return '来源尚未成功采集';
  const age = now - health.last_success;
  const stale = health.status === 'degraded' || age > Math.max(30000, (source?.poll_interval_ms ?? 3000) * 3);
  return `${stale ? '来源数据待更新' : '来源采集'} · ${formatAgo(health.last_success, now)}${health.message ? ' · '+health.message : ''}`;
}

export function enabledDashboardSources(settings: DashboardSettings | null) {
  // Retired adapters may still be enabled in saved profiles. Keep the setting
  // but don't show them in source filters or wait for a poll that cannot occur.
  return settings?.sources.filter(s => s.enabled && s.kind !== 'doubao') ?? [];
}

export function refreshProgress(settings: DashboardSettings | null, providers: ProviderHealth[], since: number) {
  const enabled = enabledDashboardSources(settings);
  const pending = enabled.filter(s => (providers.find(p => p.provider === s.id)?.last_attempt ?? 0) < since);
  const failed = enabled.filter(s => { const p=providers.find(p=>p.provider===s.id); return p && (p.last_attempt ?? 0) >= since && p.status==='degraded'; });
  return { total: enabled.length, pending, failed };
}

export function isEndedResult(task: TaskSnapshot): boolean {
  return !task.stale && !needsAction(task) && task.unread === true &&
    (task.status === 'failed' || task.status === 'succeeded' || isInferredCommandEnd(task) || task.status === 'waiting_input' && task.waiting_reason === 'turn_ended');
}

export function isInferredCommandEnd(task:TaskSnapshot):boolean {
  return task.source==='remote_tmux' && task.session_id==null && task.confidence==='inferred' && task.status==='idle' && task.finished_at!==undefined && task.exit_code===undefined;
}

// Elapsed values freeze at the explicit end; never use refresh/observation time.
export function taskElapsed(task:TaskSnapshot,now:number):string|null {
  const start=task.started_at;
  if(start==null||!Number.isFinite(start))return null;
  const ended=isEndedResult(task);
  if(!ended&&task.stale)return null;
  const end=ended?task.finished_at:task.status==='running'?now:undefined;
  if(end==null||!Number.isFinite(end)||end<start)return null;
  return formatElapsed(start,end);
}
