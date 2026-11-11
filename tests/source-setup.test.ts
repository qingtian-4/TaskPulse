import {test,expect} from 'bun:test';
import {configuredSource,localDiscoveryCandidates,type SourceDefinition} from '../src/lib/source-setup';
import type {DashboardSettings,MonitorSource} from '../src/lib/types';

const s:MonitorSource={id:'one',kind:'remote_codex',name:'test',enabled:true,location:'public-test',projects:['*'],excluded_projects:[],tmux_sessions:[],tmux_panes:[],poll_interval_ms:3000};
test('the review list contains only new local sources the user can add',()=>{
  const definition=(kind:SourceDefinition['kind'],category='local'):SourceDefinition=>({id:kind,kind,category,name:kind,location_kind:'none',platforms:[]});
  const catalog=[definition('trae'),definition('doubao'),definition('hermes'),definition('codex_wsl','remote'),definition('scanner')];
  const candidates=catalog.map(d=>({id:d.id,location:'',data_location_detected:true,detail:'',candidate:{...s,id:d.id,kind:d.kind,enabled:false,location:''}}));
  const settings:DashboardSettings={version:2,onboarding_complete:true,notifications:true,sound:false,default_view:'tasks',sources:[{...s,id:'saved-hermes',kind:'hermes'}],auto_discovery_seen:['doubao']};
  expect(localDiscoveryCandidates(catalog,candidates,settings).map(row=>row.candidate.id)).toEqual(['trae']);
  expect(localDiscoveryCandidates(catalog,candidates,{...settings,sources:[...settings.sources,candidates[0].candidate]})).toEqual([]);
});

test('configured disabled sources remain visible but untouched seeds can collapse',()=>{
  const seed:MonitorSource={...s,id:'codex',kind:'codex',name:'Codex',enabled:false,location:''};
  const definition:SourceDefinition={id:seed.id,kind:seed.kind,name:seed.name,category:'local',
    location_kind:'none',platforms:['windows','macos'],default_source:seed};
  expect(configuredSource(seed,definition)).toBe(false);
  expect(configuredSource(seed,definition,[seed.id])).toBe(true);
  expect(configuredSource({...seed,projects:['public-project']},definition)).toBe(true);
  expect(configuredSource({...seed,id:'existing-instance'},definition)).toBe(true);
});
