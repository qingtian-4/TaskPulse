import { SOURCE_LABEL, type TaskNotice } from './types';

/** Presentation only: lifecycle status and notification dedup remain in Rust. */
export function noticeContent(notice: TaskNotice) {
  const status = notice.title.replace(/^TaskPulse · /, '');
  const context = notice.context;
  const sender = context
    ? [SOURCE_LABEL[context.source], context.host, context.project].filter(Boolean).join(' · ')
    : 'TaskPulse';
  const tone = /失败|断开/.test(status) ? 'danger'
    : /可能|授权|确认/.test(status) ? 'attention'
    : /完成|本轮结束|恢复/.test(status) ? 'success'
    : /等待输入/.test(status) ? 'attention' : 'info';
  return {
    sender,
    heading: context?.task_name ?? status,
    summary: context ? status : notice.body,
    tone,
  };
}
