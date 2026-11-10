import {test,expect} from 'bun:test';
import {localDiscoveryCandidates,createEventSource,sourceSections,sourceReadState,readTestResult,sourceCapability,sourceCatalogRows,summarizeSources,sourceSetupStep,type SourceDefinition} from '../src/lib/source-setup';
import type {DashboardSettings,MonitorSource,ProviderHealth,SourceSnapshot} from '../src/lib/types';

const s:MonitorSource={id:'one',kind:'remote_codex',name:'test',enabled:true,location:'public-test',projects:['*'],excluded_projects:[],tmux_sessions:[],tmux_panes:[],poll_interval_ms:3000};
const empty:SourceSnapshot={id:s.id,kind:s.kind,location:s.location,enabled:true,read_state:'healthy',data_detected:false,discovered_sessions:0,current_tasks:0};

test('the review list contains only new local sources the user can add',()=>{
  const definition=(kind:SourceDefinition['kind'],category='local'):SourceDefinition=>({id:kind,kind,category,name:kind,location_kind:'none',platforms:[]});
  const catalog=[definition('trae'),definition('doubao'),definition('hermes'),definition('codex_wsl','remote'),definition('scanner')];
  const candidates=catalog.map(d=>({id:d.id,location:'',data_location_detected:true,detail:'',candidate:{...s,id:d.id,kind:d.kind,enabled:false,location:''}}));
  const settings:DashboardSettings={version:2,onboarding_complete:true,notifications:true,sound:false,default_view:'tasks',sources:[{...s,id:'saved-hermes',kind:'hermes'}],auto_discovery_seen:['doubao']};
  expect(localDiscoveryCandidates(catalog,candidates,settings).map(row=>row.candidate.id)).toEqual(['trae']);
  expect(localDiscoveryCandidates(catalog,candidates,{...settings,sources:[...settings.sources,candidates[0].candidate]})).toEqual([]);
});

test('event source drafts keep manifest identity, require opt-in and support multiple instances',()=>{
  for(const adapter of ['kimi-code-cli','trae-cn-ide','standard-events']){
    const definition:SourceDefinition={id:`integration:${adapter}`,kind:'generic_event',category:'custom',name:adapter,location_kind:'none',platforms:[],adapter_id:adapter,agent_id:'declared-agent'};
    const a=createEventSource(definition),b=createEventSource(definition);
    expect(a.id).not.toBe(b.id);expect(a.enabled).toBe(false);expect(a.location).toBe('');
    expect(a.integration).toEqual({adapter_id:adapter,agent_id:'declared-agent',transport:'local',accept_since_ms:0});
    expect(a.projects).toEqual(['*']);expect(definition.agent_id).toBe('declared-agent');
  }
  expect(()=>createEventSource({id:'native',kind:'codex',category:'local',name:'Codex',location_kind:'none',platforms:[]})).toThrow();
});
test('support, data detection, first read, health and current work are independent',()=>{
  const ssh:SourceDefinition={id:'remote_codex',kind:'remote_codex',name:'SSH',category:'remote',platforms:[],location_kind:'host',multiple_instances:true};
  const wsl:SourceDefinition={...ssh,id:'deepseek_harness',kind:'deepseek_wsl',platforms:['windows']};
  const wslSource={...s,id:'other',kind:wsl.kind};
  const found={...empty,data_detected:true,discovered_sessions:7,read_state:'partial' as const};
  const rows=sourceCatalogRows([ssh,wsl],[s,wslSource],[], 'remote','windows',true,[found]);
  expect(rows).toHaveLength(1);
  expect(rows[0]).toMatchObject({supported:true,dataDetected:true,source:{id:s.id},observation:{discovered_sessions:7,current_tasks:0}});
  expect(sourceCatalogRows([ssh],[s],[],'remote','windows',true,[empty])).toEqual([]);
  const edited=sourceCatalogRows([ssh],[{...s,location:'other-host'}],[],'remote','windows',false,[found])[0];
  expect(edited.observation).toBeUndefined();expect(edited.dataDetected).toBeNull();
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

test('default sources keep configured connections visible and defer dormant catalog entries',()=>{
  const active={...s,id:'claude_code',kind:'claude_code' as const,location:'',name:'Claude'};
  const seed={...s,id:'codex',kind:'codex' as const,location:'',name:'Codex',enabled:false};
  const off={...s,id:'ssh-off',enabled:false};
  const make=(s:MonitorSource,category:string):SourceDefinition=>({id:s.id,kind:s.kind,name:s.name,category,platforms:['windows'],location_kind:'none'});
  const windowsOnly={...make(s,'remote'),id:'codex_wsl',kind:'codex_wsl' as const,multiple_instances:true,location_kind:'distro'};
  const catalog=[make(active,'local'),{...make(seed,'local'),default_source:seed},make(off,'remote'),windowsOnly];
  const settings:DashboardSettings={version:2,onboarding_complete:true,sources:[active,seed,off],notifications:true,sound:false,default_view:'tasks'};
  const candidates=['PublicA','PublicB'].map(location=>{
    const candidate={...s,id:'wsl:'+location,kind:'codex_wsl' as const,location,name:location};
    return {id:candidate.id,location,candidate,data_location_detected:true,detail:''};
  });
  const sections=sourceSections(catalog,settings,candidates,'windows',[],'');
  expect(sections.primary.map(r=>r.source?.id)).toEqual([active.id,off.id]);
  expect(sections.supported.map(r=>r.source?.id)).toEqual([seed.id]);
  expect(sections.advanced.filter(r=>r.candidate).map(r=>r.candidate?.location)).toEqual(['PublicA','PublicB']);
  const detected=sourceSections(catalog,settings,[{id:seed.id,location:'',data_location_detected:true,detail:''}],'windows',[],'');
  expect(detected.primary.some(r=>r.source?.id===seed.id)).toBe(true);
  const offAfterRestart=sourceSections(catalog,{...settings,auto_discovery_seen:[seed.id]},[],'windows',[],'');
  expect(offAfterRestart.primary.some(r=>r.source?.id===seed.id&&r.source.enabled===false)).toBe(true);
});
