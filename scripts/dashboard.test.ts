// Pure presentation regression tests. Run with `bun test scripts/dashboard.test.ts`.
import { test, expect } from 'bun:test';
import { taskSection, taskLabel, projectKey, taskContext } from '../src/lib/dashboard';
import type { TaskSnapshot, DashboardSettings } from '../src/lib/types';
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
test('legacy remote tasks never borrow a local source label', () => {
  const common={enabled:true,projects:['*'],excluded_projects:[],tmux_sessions:[],tmux_panes:[],poll_interval_ms:3000};
  const settings: DashboardSettings={version:2,onboarding_complete:true,notifications:true,sound:false,default_view:'tasks',sources:[
    {...common,id:'codex',name:'Codex · 本机',kind:'codex',location:''},
    {...common,id:'remote-codex:server3',name:'Codex · Server3',kind:'remote_codex',location:'Server3'}
  ]};
  const task={...base,remote:true,host:'Server3',project:'HyperMem'};
  expect(taskContext(task,settings)).toBe('HyperMem · Codex · Server3');
  expect(taskContext({...task,host:'Other'},settings)).toBe('HyperMem · Codex · Other');
});
