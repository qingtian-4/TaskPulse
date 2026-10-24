import type {MessageCenterState, MessageEntry, MessageKind} from './types';
export const MESSAGE_LABELS:Record<MessageKind,string>={approval:'需要授权',question:'等待回答',quota:'额度已用完',completed:'本轮结束',failed:'任务失败',interrupted:'本轮已中断'};
export function messageStatus(entry:MessageEntry):string {
  if(entry.state==='unconfirmed')return '上次请求 · 来源待确认';
  if(entry.state==='pending')return '原 Agent 仍待处理';
  if(entry.state==='resolved')return '该请求已不再待处理';
  return '任务结果是否查看单独记录';
}
export function acceptMessageState(current:MessageCenterState,next:MessageCenterState):MessageCenterState {
  return next.revision>=current.revision ? next : current;
}
// Exact snapshot IDs: a concurrent new arrival must never be acknowledged by
// an earlier "all read" click. No Task Unread or Request Pending mutation.
export function unreadMessageIds(state:MessageCenterState):string[] {
  return state.entries.filter(entry=>!entry.seen).map(entry=>entry.id);
}
