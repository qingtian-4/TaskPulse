import {test,expect} from 'bun:test';
import {sourceReadState,readTestResult,sourceCapability,sourceCatalogRows,summarizeSources,sourceSetupStep,type SourceDefinition} from '../src/lib/source-setup';
import type {MonitorSource,ProviderHealth,SourceSnapshot} from '../src/lib/types';

const s:MonitorSource={id:'one',kind:'remote_codex',name:'test',enabled:true,location:'public-test',projects:['*'],excluded_projects:[],tmux_sessions:[],tmux_panes:[],poll_interval_ms:3000};
const empty:SourceSnapshot={id:s.id,enabled:true,read_state:'healthy',data_detected:false,discovered_sessions:0,current_tasks:0};
test('support, data detection, first read, health and current work are independent',()=>{
  const ssh:SourceDefinition={id:'remote_codex',kind:'remote_codex',name:'SSH',category:'remote',platforms:[],location_kind:'host',multiple_instances:true};
  const wsl:SourceDefinition={...ssh,id:'deepseek_harness',kind:'deepseek_wsl',platforms:['windows']};
  const wslSource={...s,id:'other',kind:wsl.kind};
  const found={...empty,data_detected:true,discovered_sessions:7,read_state:'partial' as const};
  const rows=sourceCatalogRows([ssh,wsl],[s,wslSource],[], 'remote','windows',true,[found]);
  expect(rows).toHaveLength(1);
  expect(rows[0]).toMatchObject({supported:true,dataDetected:true,source:{id:s.id},observation:{discovered_sessions:7,current_tasks:0}});
  expect(sourceCatalogRows([ssh],[s],[],'remote','windows',true,[empty])).toEqual([]);
  expect(sourceCatalogRows([wsl],[wslSource],[],'remote','macos',false)).toMatchObject([{supported:false,dataDetected:null}]);
  expect(sourceReadState(undefined,true)).toBe('等待首次读取');
  expect(sourceReadState(empty,true)).toBe('来源读取正常');
  expect(sourceReadState(found,true)).toBe('部分会话需检查');
  expect(sourceReadState(found,false)).toBe('未读取');
  expect(summarizeSources([])).toEqual({enabled:0,waiting:0,issues:0});
  expect(summarizeSources([{...empty,read_state:'waiting'}, {...found,id:'other'}])).toEqual({enabled:2,waiting:1,issues:1});
  expect(sourceSetupStep(false,false,false)).toBe(1);expect(sourceSetupStep(true,true,true)).toBe(2);
  expect(sourceSetupStep(true,false,false)).toBe(3);expect(sourceSetupStep(true,false,true)).toBe(4);
});
test('manual test waits for a new matching success, never borrows stale success',()=>{
  const h:ProviderHealth={provider:s.id,status:'connected',last_attempt:110,last_success:90};
  expect(readTestResult(s.id,[h],100)).toBe('failed');
  expect(readTestResult(s.id,[{...h,last_attempt:90}],100)).toBe('pending');
  expect(readTestResult(s.id,[{...h,last_success:110}],100)).toBe('passed');
  expect(readTestResult(s.id,[{...h,provider:'other'}],100)).toBe('pending');
  expect(sourceCapability('generic_event')).not.toMatch(/自动全面支持|无需配置/);
});
