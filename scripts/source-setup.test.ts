import {test,expect} from 'bun:test';
import {sourceReadState,readTestResult,sourceCapability,sourceReadTone,sourceSetupStep} from '../src/lib/source-setup';
import type {MonitorSource,DashboardSettings,ProviderHealth} from '../src/lib/types';
const s:MonitorSource={id:'one',kind:'codex',name:'test',enabled:true,location:'',projects:['*'],excluded_projects:[],tmux_sessions:[],tmux_panes:[],poll_interval_ms:3000};
const config:DashboardSettings={version:2,onboarding_complete:true,sources:[s],notifications:true,sound:false,default_view:'tasks'};
test('setup step and read badges do not promote discovery or another provider to host verification',()=>{
  expect(sourceSetupStep(false,false,false)).toBe(1);expect(sourceSetupStep(true,true,true)).toBe(2);
  expect(sourceSetupStep(true,false,false)).toBe(3);expect(sourceSetupStep(true,false,true)).toBe(4);
  expect(sourceReadTone(s,null,[],100000)).toBe('neutral');
  const h:ProviderHealth={provider:s.id,status:'connected',last_attempt:99000,last_success:99000};
  expect(sourceReadTone(s,config,[h],100000)).toBe('success');
  expect(sourceReadTone(s,config,[h],140000)).toBe('stale');
  expect(sourceReadTone(s,config,[{...h,status:'degraded'}],100000)).toBe('stale');
  expect(sourceReadTone(s,config,[{...h,provider:'another'}],100000)).toBe('stale');
  expect(sourceReadTone(s,{...config,sources:[{...s,enabled:false}]},[h],100000)).toBe('neutral');
});
test('saved/discovered/readable are distinct and scoped to the matching source',()=>{
  expect(sourceReadState(s,null,[],100000)).toContain('未保存');
  expect(sourceReadState(s,config,[],100000)).toContain('等待首次读取');
  const h:ProviderHealth={provider:s.id,status:'connected',last_attempt:99000,last_success:99000};
  expect(sourceReadState(s,config,[{...h,provider:'other'}],100000)).not.toContain('读取正常');
  expect(sourceReadState(s,config,[h],100000)).toBe('来源读取正常');
  expect(sourceReadState(s,config,[h],140000)).toContain('待更新');
  expect(sourceReadState(s,config,[{...h,status:'degraded'}],100000)).toContain('异常');
  expect(sourceReadState(s,{...config,sources:[{...s,enabled:false}]},[h],100000)).toContain('已停用');
});
test('manual test waits for a new matching success, never borrows stale success',()=>{
  const h:ProviderHealth={provider:s.id,status:'connected',last_attempt:110,last_success:90};
  expect(readTestResult(s.id,[h],100)).toBe('failed');
  expect(readTestResult(s.id,[{...h,last_attempt:90}],100)).toBe('pending');
  expect(readTestResult(s.id,[{...h,last_success:110}],100)).toBe('passed');
  expect(readTestResult(s.id,[{...h,provider:'other'}],100)).toBe('pending');
  expect(sourceCapability('generic_event')).not.toMatch(/自动全面支持|无需配置/);
});

test('partial Codex health remains a warning while identifying verified sessions',()=>{
  const health:ProviderHealth={provider:s.id,status:'degraded',last_attempt:99000,task_observations:{good:{observed_at:99000,state:'fixture'}}};
  expect(sourceReadState(s,config,[health],100000)).toBe('部分会话需检查');
  expect(sourceReadTone(s,config,[health],100000)).toBe('stale');
  expect(sourceReadState(s,config,[{...health,task_observations:{good:{observed_at:98000,state:'fixture'}}}],100000)).toContain('读取异常');
  expect(sourceReadState(s,config,[health],140000)).toContain('读取异常');
});
