import { SOURCE_LABEL, type TaskNotice } from './types';

/** Presentation only: lifecycle status and notification dedup remain in Rust. */
export function noticeContent(notice: TaskNotice) {
  const status = notice.title.replace(/^TaskPulse · /, '');
  const context = notice.context;
  const sender = context
    ? [context.agent_name ?? SOURCE_LABEL[context.source], context.host, context.project].filter(Boolean).join(' · ')
    : 'TaskPulse';
  const tone = /失败|断开/.test(status) ? 'danger'
    : /可能|授权|确认|中断|额度|需处理/.test(status) ? 'attention'
    : /完成|本轮结束|恢复/.test(status) ? 'success'
    : /等待输入/.test(status) ? 'attention' : 'info';
  return {
    sender,
    heading: context?.task_name ?? status,
    summary: context ? status : notice.body,
    tone,
  };
}

export function noticeTimeout(notice: TaskNotice): number | null {
  return notice.auto_dismiss_ms===null ? null : (notice.auto_dismiss_ms ?? 15000);
}
// Rotate, never dismiss, confirmed blockers so normal results cannot starve.
export function noticeRotation(notice:TaskNotice, pending:number):number | null {
  return noticeTimeout(notice)===null && pending>0 ? 15000 : null;
}
export function deliveryStage(stage:string):string {
  return ({detected:'已识别事件',queued:'已入队',reopened:'用户重新打开（非新事件）',queue_overflow:'队列已满，可在面板回查',window_visible:'原生窗口可见',rendered:'界面已渲染',api_accepted:'系统 API 已接受',auto_dismissed:'计时自动收起',user_dismissed:'用户收起',details_opened:'打开详情（不等于已查看）',request_resolved:'请求已处理/返回',failed:'投递失败',suppressed_disabled:'提醒开关关闭',suppressed_scope:'不在监控范围'} as Record<string,string>)[stage] ?? stage;
}
