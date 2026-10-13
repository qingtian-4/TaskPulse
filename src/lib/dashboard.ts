import { SOURCE_LABEL, formatAgo, type DashboardSettings, type TaskSnapshot, type ProviderHealth } from './types';

export interface DashboardFilters { source: string; project: string; location: string; section: number | null }
export const EMPTY_FILTERS: DashboardFilters = {source:'',project:'',location:'',section:null};
export const SECTION_NAMES = ['需要处理', '正在运行', '结果待查看', '固定的空闲任务'];

export function filterTasks(tasks: TaskSnapshot[], filters: DashboardFilters, settings?: DashboardSettings | null): TaskSnapshot[] {
  return tasks.filter(t => (!filters.source || taskSource(t,settings)?.id === filters.source) &&
    (!filters.project || projectKey(t,settings) === filters.project) &&
    (!filters.location || (t.host ?? '本机') === filters.location) &&
    (filters.section === null || taskSection(t) === filters.section));
}

// Global counters mean ALL monitored tasks, not the intersection with a
// forgotten project/source filter. One click must show the counted tasks.
export function overviewFilters(section: number, currentSection: number | null): DashboardFilters {
  return {...EMPTY_FILTERS,section:section === currentSection ? null : section};
}

export function runningOutsideView(tasks: TaskSnapshot[], visible: TaskSnapshot[]): TaskSnapshot[] {
  const ids=new Set(visible.map(t=>t.id));
  return tasks.filter(t=>taskSection(t) === 1 && !ids.has(t.id));
}

export function filterDescription(filters: DashboardFilters, tasks: TaskSnapshot[], settings?: DashboardSettings | null): string {
  return [filters.source && `来源：${settings?.sources.find(s=>s.id===filters.source)?.name ?? filters.source}`,
    filters.project && `项目：${tasks.find(t=>projectKey(t,settings)===filters.project)?.project ?? '已不在当前列表'}`,
    filters.location && `位置：${filters.location}`,
    filters.section !== null && `状态：${SECTION_NAMES[filters.section]}`].filter(Boolean).join(' · ');
}

export function needsAction(task: TaskSnapshot): boolean {
  return task.status === 'waiting_input' && (task.waiting_reason === 'approval' || task.waiting_reason === 'question');
}
export function taskLabel(task: TaskSnapshot): string {
  if (task.stale) return '状态待更新';
  if (needsAction(task)) return task.waiting_reason === 'approval' ? '需要授权' : '需要你确认';
  if (task.status === 'running') return '运行中';
  if (task.status === 'waiting_input') return task.waiting_reason === 'turn_ended' ? '本轮结束' : '等待输入';
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
export function taskSection(task: TaskSnapshot): number {
  if (task.stale || task.status === 'failed' || needsAction(task)) return 0;
  if (task.status === 'running') return 1;
  if (task.unread) return 2;
  return 3;
}
export function taskSource(task: TaskSnapshot, settings?: DashboardSettings | null) {
  const explicit = settings?.sources.find(s => s.id === task.source_instance);
  const nativeId = task.source === 'deepseek_harness'
    ? (task.id.startsWith('dsh-native:') ? 'deepseek_harness_desktop' : 'deepseek_harness')
    : task.source === 'claude_code' && task.id.startsWith('claude:') ? 'claude_code'
    : task.source === 'claude_code' || task.source === 'gemini' ? 'scanner' : task.source;
  return explicit ?? (task.remote
    ? settings?.sources.find(s => s.location === task.host && s.kind === (task.source === 'codex' ? 'remote_codex' : 'tmux'))
    : settings?.sources.find(s => s.id === nativeId));
}
export function taskContext(task: TaskSnapshot, settings?: DashboardSettings | null): string {
  const source = taskSource(task, settings);
  return `${task.project ?? '未关联项目'} · ${source?.name ?? `${SOURCE_LABEL[task.source]} · ${task.host ?? '本机'}`}`;
}
export function projectKey(task: TaskSnapshot, settings?: DashboardSettings | null): string {
  return `${taskSource(task, settings)?.id ?? task.source_instance ?? task.host ?? task.source}:${task.project_path ?? task.project ?? '@unassociated'}`;
}

export function sourceFreshness(task: TaskSnapshot, settings: DashboardSettings | null, providers: ProviderHealth[], now: number): string {
  const source = taskSource(task, settings);
  const health = providers.find(p => p.provider === (source?.id ?? task.source_instance));
  if (!health?.last_success) return '来源尚未成功采集';
  const age = now - health.last_success;
  const stale = health.status === 'degraded' || age > Math.max(30000, (source?.poll_interval_ms ?? 3000) * 3);
  return `${stale ? '来源数据待更新' : '来源采集'} · ${formatAgo(health.last_success, now)}${health.message ? ' · '+health.message : ''}`;
}

export function refreshProgress(settings: DashboardSettings | null, providers: ProviderHealth[], since: number) {
  const enabled = settings?.sources.filter(s => s.enabled) ?? [];
  const pending = enabled.filter(s => (providers.find(p => p.provider === s.id)?.last_attempt ?? 0) < since);
  const failed = enabled.filter(s => { const p=providers.find(p=>p.provider===s.id); return p && (p.last_attempt ?? 0) >= since && p.status==='degraded'; });
  return { total: enabled.length, pending, failed };
}

export function isEndedResult(task: TaskSnapshot): boolean {
  return !task.stale && !needsAction(task) && task.unread === true &&
    (task.status === 'failed' || task.status === 'succeeded' || task.status === 'waiting_input' && task.waiting_reason === 'turn_ended');
}
