import { SOURCE_LABEL, type DashboardSettings, type TaskSnapshot } from './types';

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
export function taskSection(task: TaskSnapshot): number {
  if (task.stale || task.status === 'failed' || needsAction(task)) return 0;
  if (task.status === 'running') return 1;
  if (task.unread) return 2;
  return 3;
}
export function taskContext(task: TaskSnapshot, settings?: DashboardSettings | null): string {
  const source = settings?.sources.find(s => s.id === task.source_instance) ?? settings?.sources.find(s => s.id === (task.source === 'deepseek_harness' ? (task.id.startsWith('dsh-native:') ? 'deepseek_harness_desktop' : 'deepseek_harness') : task.source));
  return `${task.project ?? '未关联项目'} · ${source?.name ?? `${SOURCE_LABEL[task.source]} · ${task.host ?? '本机'}`}`;
}
export function projectKey(task: TaskSnapshot): string {
  return `${task.source_instance ?? task.host ?? task.source}:${task.project_path ?? task.project ?? '@unassociated'}`;
}
