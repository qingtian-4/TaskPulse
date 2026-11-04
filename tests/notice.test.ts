import { expect, test } from 'bun:test';
import { noticeContent, noticeTimeout, noticeRotation, deliveryStage } from '../src/lib/notice';
import type { TaskNotice } from '../src/lib/types';

const base: TaskNotice = {
  id: 1, title: 'TaskPulse · 本轮结束，等待后续输入',
  body: 'Codex · Project · Task', created_at: 1, task_id: 'codex:stable', test: false,
};

test('confirmed blockers rotate for fairness but are never auto-dismissed',()=>{
  const blocker={...base,auto_dismiss_ms:null};
  expect(noticeRotation(blocker,1)).toBe(15000);
  expect(noticeRotation(blocker,0)).toBeNull();
  expect(noticeTimeout(blocker)).toBeNull();
  expect(noticeRotation(base,1)).toBeNull();
  expect(deliveryStage('reopened')).toContain('非新事件');
});
test('confirmed blocking reminders have no timer, while legacy completion remains 15 seconds',()=>{
  expect(noticeTimeout({...base,auto_dismiss_ms:null})).toBeNull();
  expect(noticeTimeout(base)).toBe(15000);
  expect(noticeTimeout({...base,auto_dismiss_ms:15000})).toBe(15000);
  expect(deliveryStage('rendered')).toBe('界面已渲染');
  expect(deliveryStage('api_accepted')).not.toContain('已阅读');
});

test('native task title and sender do not split names at punctuation', () => {
  const content = noticeContent({ ...base, context: {
    task_name: '研究 · HyperMem 记忆机制', source: 'codex', project: 'Project · Native', host: 'Server1',
  } });
  expect(content.heading).toBe('研究 · HyperMem 记忆机制');
  expect(content.sender).toBe('Codex · Server1 · Project · Native');
  expect(content.summary).toBe('本轮结束，等待后续输入');
  expect(content.tone).toBe('success');
});

test('non-task and older notification payloads preserve full message', () => {
  const content = noticeContent({ ...base, context: null, title: 'TaskPulse · 远程主机已断开', body: 'Server3 · 最后在线 3 分钟前' });
  expect(content.sender).toBe('TaskPulse');
  expect(content.heading).toBe('远程主机已断开');
  expect(content.summary).toBe('Server3 · 最后在线 3 分钟前');
  expect(content.tone).toBe('danger');
});

test('status accents distinguish blockers, failed tasks and uncertain completion', () => {
  expect(noticeContent({ ...base, title: 'TaskPulse · 需处理：额度已用完' }).tone).toBe('attention');
  expect(noticeContent({ ...base, title: 'TaskPulse · 需要授权' }).tone).toBe('attention');
  expect(noticeContent({ ...base, title: 'TaskPulse · 任务失败，需要处理' }).tone).toBe('danger');
  expect(noticeContent({ ...base, title: 'TaskPulse · 任务可能已结束' }).tone).toBe('attention');
  expect(noticeContent({ ...base, title: 'TaskPulse · 通知测试成功' }).tone).toBe('info');
});
