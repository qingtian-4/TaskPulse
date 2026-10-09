// Pure presentation regression tests. Run with `bun test scripts/dashboard.test.ts`.
import { test, expect } from 'bun:test';
import { taskSection, taskLabel, projectKey } from '../src/lib/dashboard';
import type { TaskSnapshot } from '../src/lib/types';
const base: TaskSnapshot = {id:'a',name:'Native title',source:'codex',status:'running',updated_at:1,remote:false,confidence:'hook',unread:true};
test('normal turn end is an unread result, not a blocking prompt', () => {
  const ended = {...base,status:'waiting_input' as const,waiting_reason:'turn_ended' as const};
  expect(taskSection(ended)).toBe(2);
  expect(taskLabel(ended)).toBe('本轮结束');
  expect(taskSection({...ended,waiting_reason:'approval'})).toBe(0);
  expect(taskSection({...ended,waiting_reason:'question'})).toBe(0);
  expect(taskSection({...base,stale:true})).toBe(0);
});
test('project identity uses path and source instance, not just the basename', () => {
  const task = {...base,project:'demo',project_path:'C:/demo',source_instance:'codex'};
  expect(projectKey(task)).not.toBe(projectKey({...task,project_path:'D:/demo'}));
  expect(projectKey(task)).not.toBe(projectKey({...task,source_instance:'remote-codex:server'}));
});
