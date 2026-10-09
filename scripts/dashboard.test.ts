// Pure presentation regression tests. Run with `bun test scripts/dashboard.test.ts`.
import { test, expect } from 'bun:test';
import { taskSection, taskLabel, projectKey, taskContext, taskSource, sourceFreshness, refreshProgress, isEndedResult } from '../src/lib/dashboard';
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
  expect(taskSource(task,settings)?.id).toBe('remote-codex:server3');
  expect(projectKey(task,settings)).toBe(projectKey({...task,source_instance:'remote-codex:server3'},settings));
  expect(taskContext({...task,host:'Other'},settings)).toBe('HyperMem · Codex · Other');
});

test('source freshness does not borrow another provider success or task activity',()=>{
  const task={...base,source_instance:'remote-codex:server3',remote:true,host:'Server3'};
  const providers=[{provider:'codex',status:'connected' as const,last_success:100000},{provider:'remote-codex:server3',status:'degraded' as const,last_success:1000,last_attempt:99900,message:'SSH failed'}];
  expect(sourceFreshness(task,null,providers,100000)).toContain('来源数据待更新 · 1 分钟前');
  expect(sourceFreshness(task,null,providers,100000)).toContain('SSH failed');
  expect(sourceFreshness({...task,source_instance:'missing'},null,providers,100000)).toBe('来源尚未成功采集');
});

test('refresh waits for actual attempts, failures count as returned, disabled sources do not',()=>{
  const common={name:'source',enabled:true,kind:'codex' as const,location:'',projects:['*'],excluded_projects:[],tmux_sessions:[],tmux_panes:[],poll_interval_ms:3000};
  const settings:DashboardSettings={version:2,onboarding_complete:true,notifications:true,sound:false,default_view:'tasks',sources:[{...common,id:'codex'},{...common,id:'remote'},{...common,id:'off',enabled:false}]};
  const providers=[{provider:'codex',status:'connected' as const,last_success:101,last_attempt:101},{provider:'remote',status:'degraded' as const,last_success:20,last_attempt:99}];
  expect(refreshProgress(settings,providers,100).pending.map(s=>s.id)).toEqual(['remote']);
  providers[1].last_attempt=102;
  expect(refreshProgress(settings,providers,100).pending).toEqual([]);
  expect(refreshProgress(settings,providers,100).failed.map(s=>s.id)).toEqual(['remote']);
});

test('quick viewed action is only offered for explicit ended results',()=>{
  expect(isEndedResult({...base,status:'waiting_input',waiting_reason:'turn_ended'})).toBe(true);
  expect(isEndedResult({...base,status:'succeeded'})).toBe(true);
  for(const status of ['running','failed','idle'] as const)expect(isEndedResult({...base,status})).toBe(false);
  expect(isEndedResult({...base,status:'waiting_input',waiting_reason:'question'})).toBe(false);
  expect(isEndedResult({...base,status:'succeeded',stale:true})).toBe(false);
});
