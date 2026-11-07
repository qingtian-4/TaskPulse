import {scopeOptions} from '../src/lib/project-options';
import {test,expect} from 'bun:test';
import {servers,pauseServer,removeServer,projectChoices,sourceStatus,createSource,canRead} from '../src/lib/monitor-sources';
import {reconcileSourceDraft} from '../src/lib/settings-draft';
import type {DashboardSettings} from '../src/lib/types';
const make=():DashboardSettings=>({version:2,onboarding_complete:true,notifications:true,sound:false,default_view:'tasks',sources:[
  {...createSource('codex'),id:'local',enabled:true,projects:['local-only']},
  {...createSource('remote_codex','host-a','Host A'),id:'a',enabled:true,projects:['/srv/alpha']},
  {...createSource('remote_codex','host-a'),id:'tmux-a',kind:'tmux',enabled:false,tmux_sessions:['preserve']},
  {...createSource('remote_codex','host-b'),id:'b',enabled:true},
  {...createSource('codex'),id:'other',kind:'hermes',enabled:true},
]});
test('host grouping preserves source IDs, hidden providers and tmux as transport',()=>{
  const s=make(),groups=servers(s);expect(groups.map(g=>g.host)).toEqual(['host-a','host-b']);expect(groups[0].sources.map(s=>s.id)).toEqual(['a','tmux-a']);expect(groups[0].name).toBe('Host A');expect(s.sources[4].enabled).toBe(true);
});
test('pause/resume is restart-safe and restores only previously enabled sources',()=>{
  const original=make();const paused=pauseServer(original,'host-a',true);
  expect(paused.sources[1].enabled).toBe(false);expect(paused.sources[2].enabled).toBe(false);expect(paused.sources[3].enabled).toBe(true);
  const resumed=pauseServer(JSON.parse(JSON.stringify(paused)),'host-a',false);
  expect(resumed.sources).toEqual(original.sources);expect(resumed.ssh_paused_sources).toEqual([]);expect(original.sources[1].enabled).toBe(true);
});
test('removing one server preserves other servers, local configuration and disabled legacy filters',()=>{
  const s=make(),next=removeServer(pauseServer(s,'host-a',true),'host-a');
  expect(next.sources.map(s=>s.id)).toEqual(['local','b','other']);expect(next.sources[0].projects).toEqual(['local-only']);expect(next.ssh_paused_sources).toEqual([]);
});
test('SSH connectivity is never used as Agent data readiness',()=>{
  expect(canRead({state:'read_error',projects:[],data_detected:false})).toBe(false);expect(canRead({state:'unsupported',projects:[],data_detected:false})).toBe(false);
  expect(canRead({state:'empty',projects:[],data_detected:false})).toBe(true);
  const s=make().sources[1];expect(sourceStatus(s)).toBe('等待首次读取');
  expect(sourceStatus(s,{id:s.id,kind:s.kind,location:s.location,enabled:true,read_state:'healthy',data_detected:false,discovered_sessions:0,current_tasks:0})).toBe('等待新会话');
});
test('project scopes and preference broadcasts preserve drafts and source identity',()=>{
  const applied=make(),draft=structuredClone(applied);draft.sources[1].projects=['/manual/project'];const incoming=structuredClone(applied);incoming.sound=true;
  expect(reconcileSourceDraft(draft,applied,incoming).settings.sources[1].projects).toEqual(['/manual/project']);
  expect(projectChoices(draft.sources[1],[],['/manual/project','/discovered'])).toEqual(['/manual/project','/discovered']);
});

test('new singleton source drafts keep backend-required fixed IDs',()=>{
  expect(createSource('codex').id).toBe('codex');expect(createSource('scanner').id).toBe('scanner');expect(createSource('hooks').id).toBe('local_file');
  expect(createSource('remote_codex','host').id).not.toBe(createSource('remote_codex','host').id);
});

test('project options use verified alias groups and preserve raw scopes and distinct repositories',()=>{
  const configured=['/mnt/d/Project/agenttray'];
  const aliases=[{id:'codex:project',path:'D:/Project/agenttray',aliases:['D:\\Project\\agenttray','/mnt/d/Project/agenttray']}];
  const choices=scopeOptions(configured,['D:/Project/agenttray','/mnt/d/Project/agenttray','D:/other/agenttray'],aliases);
  expect(choices).toHaveLength(2);expect(choices[0].aliases).toContain(configured[0]);expect(configured).toEqual(['/mnt/d/Project/agenttray']);
});
