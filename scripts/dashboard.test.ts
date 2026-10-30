// Pure presentation regression tests. Run with `bun test scripts/dashboard.test.ts`.
import { test, expect } from 'bun:test';
import {observedDashboardTasks,shouldApplyDashboardSnapshot,preferredDashboardHeight} from '../src/lib/dashboard';
import {acceptMessageState,unreadMessageIds,messageStatus} from '../src/lib/messages';
import type {MessageCenterState} from '../src/lib/types';
import { taskSection, sectionRank, matchesSignal, taskLabel, taskActivity, projectKey, taskContext, taskSource, sourceFreshness, refreshProgress, isEndedResult, filterTasks, overviewFilters, runningOutsideView, filterDescription, EMPTY_FILTERS, compareTasks,taskSummary,taskAgent } from '../src/lib/dashboard';
import type { TaskSnapshot, DashboardSettings } from '../src/lib/types';
import {taskTone,unconfirmedRunning,refreshResultMessage,dashboardContinuity} from '../src/lib/dashboard';
const base: TaskSnapshot = {id:'a',name:'Native title',source:'codex',status:'running',updated_at:1,remote:false,confidence:'hook',unread:true};

test('Doubao conversation titles remain separate from project membership and filters',()=>{
  const task:TaskSnapshot={...base,source:'doubao',name:'真实会话名称'};
  expect(taskContext(task)).toBe('独立会话 · 豆包 · 本机');
  expect(projectKey(task)).toBe(projectKey({...task,name:'已改名'}));
  expect(filterTasks([task],{...EMPTY_FILTERS,query:'真实会话名称'})).toEqual([task]);
  expect(taskContext({...task,project:'实际项目'})).toBe('实际项目 · 豆包 · 本机');
  expect(task.project).toBeUndefined();
});

test('dashboard snapshots reject late responses from an older request or revision',()=>{
  expect(shouldApplyDashboardSnapshot(4,3,1,2)).toBe(false);
  expect(shouldApplyDashboardSnapshot(4,5,1,2)).toBe(false);
  expect(shouldApplyDashboardSnapshot(4,4,2,2)).toBe(true);
  expect(shouldApplyDashboardSnapshot(4,5,2,2)).toBe(true);
});

test('three headline counts retain task evidence while fresh blockers are an ongoing subset',()=>{
  const settings:DashboardSettings={version:2,onboarding_complete:true,notifications:true,sound:false,default_view:'tasks',sources:[{
    id:'codex',name:'Local',kind:'codex',enabled:true,location:'',projects:['*'],excluded_projects:[],tmux_sessions:[],tmux_panes:[],poll_interval_ms:3000}]};
  const blocker={...base,id:'gate',source_instance:'codex',status:'waiting_input' as const,waiting_reason:'approval' as const,input_request_id:'q'};
  const done={...base,id:'done',status:'failed' as const};
  const health=[{provider:'codex',status:'connected' as const,last_success:100}];
  const initial=observedDashboardTasks([base,blocker,done],health,settings,100);
  expect(initial.filter(t=>taskSection(t)==='ongoing')).toHaveLength(2);
  expect(initial.filter(t=>matchesSignal(t,'action'))).toHaveLength(1);
  expect(initial.filter(isEndedResult)).toHaveLength(1);
  const failed=observedDashboardTasks([base,blocker,done],[{...health[0],status:'degraded'}],settings,200);
  expect(failed.filter(t=>taskSection(t)==='ongoing')).toHaveLength(0);
  expect(failed.filter(t=>matchesSignal(t,'action'))).toHaveLength(0);
  expect(failed[0].status).toBe('running');expect(failed).toHaveLength(3);
  expect(filterTasks(failed,{...EMPTY_FILTERS,query:'Native'})).toHaveLength(3);
  expect(observedDashboardTasks([blocker],health,settings,40000)[0].stale).toBe(true);
  expect(observedDashboardTasks([{...blocker,confidence:'inferred'}],health,settings,100)[0].stale).toBe(true);
  expect(observedDashboardTasks([{...blocker,input_request_id:undefined}],health,settings,100)[0].stale).toBe(true);
  expect(observedDashboardTasks([base,blocker],health,settings,100).every(t=>!t.stale)).toBe(true);
  expect(base.unread&&blocker.unread).toBe(true);
});

test('message revisions and exact read snapshot IDs preserve concurrent unseen arrivals',()=>{
  const entry={id:'a',at:100,seen:false,kind:'approval' as const,task_id:'task',task_name:'native',source:'codex' as const,state:'pending' as const,can_reopen:true};
  const current:MessageCenterState={revision:2,unread:1,entries:[entry],error:null};
  const rendered=unreadMessageIds(current);
  const incoming={...current,revision:3,unread:2,entries:[{...entry,id:'b'},entry]};
  expect(rendered).toEqual(['a']);expect(unreadMessageIds(incoming)).toEqual(['b','a']);
  expect(acceptMessageState(incoming,current)).toBe(incoming);
  expect(messageStatus({...entry,seen:true})).toContain('仍待处理');
  expect(messageStatus({...entry,state:'unconfirmed',can_reopen:false})).toContain('待确认');
});

test('WSL fresh ongoing 1 → 0 → 1 is uncertainty, not a vanished or ended task',()=>{
  const running={...base,source_instance:'codex-wsl:ubuntu',session_id:'stable',turn_key:'one',pinned:true};
  const stale={...running,stale:true};
  const samples=[running,stale,stale,running];
  expect(samples.map(t=>Number(taskSection(t)==='ongoing'))).toEqual([1,0,0,1]);
  expect(samples.map(t=>unconfirmedRunning([t]).length)).toEqual([0,1,1,0]);
  expect(taskSummary(stale)).toBe('上次确认为运行中，当前采集待更新');
  expect(isEndedResult(stale)).toBe(false);
  const other={...base,id:'claude',source:'claude_code' as const};
  const list=[stale,other];
  const hidden=filterTasks(list,{...EMPTY_FILTERS,section:'ongoing',query:'nonexistent'});
  expect(hidden).toEqual([]);
  const diagnostics=dashboardContinuity(list,hidden);
  expect(diagnostics.global.ongoing).toBe(1);expect(diagnostics.global.uncertain).toBe(1);
  expect(diagnostics.visible.total).toBe(0);expect(diagnostics.global.unconfirmed_running).toBe(1);
  expect(JSON.stringify(diagnostics)).not.toContain('Native title');expect(JSON.stringify(diagnostics)).not.toContain('stable');
  expect(unconfirmedRunning(list)).toEqual([stale]);
  expect(filterTasks(list,overviewFilters('uncertain',null))).toEqual([stale]);
  expect(stale.unread && stale.pinned).toBe(true);
  const ended={...running,status:'waiting_input' as const,waiting_reason:'turn_ended' as const};
  expect(unconfirmedRunning([ended])).toEqual([]);
  expect(taskSection(ended)).toBe('results');
});

test('refresh collection fault reflects recovery instead of persisting a historical failure',()=>{
  expect(refreshResultMessage(['WSL'],['WSL'])).toContain('不代表任务结束');
  expect(refreshResultMessage(['WSL'],[])).toBe('已收到全部启用来源的新采集结果');
  expect(refreshResultMessage(['wsl-id'],['wsl-id'],{'wsl-id':'重命名后的 Ubuntu'})).toContain('重命名后的 Ubuntu');
  expect(refreshResultMessage(['other-id'],['wsl-id'],{'other-id':'WSL'})).not.toContain('仍待更新');
});

test('explicit pending question is waiting input, never a completed result',()=>{
  const question={...base,status:'waiting_input' as const,waiting_reason:'question' as const,input_request_id:'q1'};
  expect(taskLabel(question)).toBe('等待输入');
  expect(taskSection(question)).toBe('ongoing');
  expect(taskTone(question)).toBe('action');
  expect(isEndedResult(question)).toBe(false);
  expect(taskLabel({...question,stale:true})).toBe('状态待更新');
});
test('explicit exhausted quota is actionable and thinking is never a blocking request',()=>{
  const quota:TaskSnapshot={...base,status:'waiting_input',waiting_reason:'quota_exhausted'};
  expect(matchesSignal(quota,'action')).toBe(true);
  expect(taskLabel(quota)).toBe('额度已用完');
  expect(taskSection(quota)).toBe('ongoing');
  expect(taskTone(quota)).toBe('action');
  expect(isEndedResult(quota)).toBe(false);
  expect(matchesSignal({...base,phase:'thinking'},'action')).toBe(false);
  expect(matchesSignal({...quota,stale:true},'action')).toBe(false);
});

test('visual tones prioritize uncertainty without inventing completion or human blockers',()=>{
  expect(taskTone({...base,phase:'thinking'})).toBe('running');
  expect(taskTone({...base,phase:'awaiting_model'})).toBe('running');
  expect(taskTone({...base,status:'waiting_input',waiting_reason:'approval'})).toBe('action');
  expect(taskTone({...base,status:'failed',stale:true})).toBe('stale');
  expect(taskTone({...base,status:'waiting_input',waiting_reason:'approval',stale:true})).toBe('stale');
  expect(taskTone({...base,status:'failed'})).toBe('danger');
  expect(taskTone({...base,status:'waiting_input',waiting_reason:'turn_ended'})).toBe('unread');
  expect(taskTone({...base,status:'waiting_input',waiting_reason:'unknown'})).toBe('neutral');
  expect(taskTone({...base,status:'idle'})).toBe('neutral');
});
test('search and unread filters compose without mutating attention or hiding the outside-running hint',()=>{
  const task={...base,name:'修复 Long Title',project:'Atlas',project_path:'D:/Atlas',agent_name:'Coding Agent'};
  const other={...task,id:'b',unread:false};
  expect(filterTasks([task,other],{...EMPTY_FILTERS,query:'ATLAS 修复',unreadOnly:true})).toEqual([task]);
  expect(filterTasks([task],{...EMPTY_FILTERS,query:'Coding Agent'})).toEqual([task]);
  expect(filterTasks([task],{...EMPTY_FILTERS,query:'missing'})).toEqual([]);
  expect(runningOutsideView([task],[])).toEqual([task]);expect(task.unread).toBe(true);
  expect(filterDescription({...EMPTY_FILTERS,unreadOnly:true,query:'Atlas'},[task])).toContain('仅未查看');
});
test('fresh explicit blockers sort first within ongoing; stale approval remains uncertain',()=>{
  const approval={...base,id:'p',status:'waiting_input' as const,waiting_reason:'approval' as const};
  const stale={...approval,id:'s',stale:true};
  expect(matchesSignal(approval,'action')).toBe(true);expect(matchesSignal(stale,'action')).toBe(false);
  expect(matchesSignal(stale,'approval')).toBe(false);expect(taskSection(stale)).toBe('uncertain');
  expect([stale,base,approval].sort(compareTasks).map(t=>t.id)).toEqual(['p','a','s']);
  expect(taskSummary(approval)).toContain('原 Agent');expect(taskSummary({...base,stale:true,activity:'old command'})).not.toContain('old command');
  expect(taskAgent({...base,agent_name:'Community Agent'})).toBe('Community Agent');
});

test('explicit interruption is a viewable result, never advertised as success',()=>{
  const interrupted={...base,source:'generic_event' as const,status:'waiting_input' as const,waiting_reason:'turn_ended' as const,finished_at:2,outcome:'interrupted'};
  expect(taskLabel(interrupted)).toBe('已中断');expect(taskSection(interrupted)).toBe('results');expect(isEndedResult(interrupted)).toBe(true);
});
test('a failed result can be marked viewed without turning it into success',()=>{
  const failed = {...base,status:'failed' as const};
  expect(isEndedResult(failed)).toBe(true);
  expect(isEndedResult({...failed,unread:false})).toBe(false);
  expect(isEndedResult({...failed,stale:true})).toBe(false);
});
test('obsolete approval hints cannot create a blocker label, action section or completed result',()=>{
  const pending={...base,phase:'awaiting_tool' as const,approval_hint:{request_id:'c',requested_at:1,check_due:true}};
  expect(taskLabel(pending)).toBe('运行中');expect(taskSection(pending)).toBe('ongoing');
  expect(taskActivity(pending)).toBe('等待工具响应 · 本轮仍在运行');expect(isEndedResult(pending)).toBe(false);
  expect(taskActivity(pending)).not.toContain('授权');
  const confirmed={...base,status:'waiting_input' as const,waiting_reason:'approval' as const,input_request_id:'q1'};
  expect(taskLabel(confirmed)).toBe('需要授权');expect(taskSection(confirmed)).toBe('ongoing');
  expect(taskLabel({...base,phase:'thinking'})).toBe('运行中');
});

test('a newly detected project outside explicit filters is visible via the global running counter',()=>{
  const old={...base,id:'old',project:'old-project',project_path:'C:/old',source_instance:'codex'};
  const fresh={...base,id:'new',project:'new-project',project_path:'D:/new',source_instance:'codex'};
  const filtered=filterTasks([old,fresh],{...EMPTY_FILTERS,project:projectKey(old)});
  expect(filtered.map(t=>t.id)).toEqual(['old']);
  expect(runningOutsideView([old,fresh],filtered).map(t=>t.id)).toEqual(['new']);
  const global=overviewFilters('ongoing',null);
  expect(filterTasks([old,fresh],global).map(t=>t.id)).toEqual(['old','new']);
  expect(runningOutsideView([old,fresh],filterTasks([old,fresh],global))).toEqual([]);
});

test('zero matching rows is explicitly a filter state, not a lack of collected executions',()=>{
  const filters={...EMPTY_FILTERS,project:'obsolete-project',location:'Old server',section:'uncertain' as const};
  expect(filterTasks([base],filters)).toEqual([]);
  expect(runningOutsideView([base],[])).toEqual([base]);
  expect(filterDescription(filters,[base])).toContain('项目：已不在当前列表');
  expect(filterDescription(filters,[base])).toContain('状态：状态待更新');
  expect(overviewFilters('uncertain','uncertain')).toEqual(EMPTY_FILTERS);
  expect(filterDescription(EMPTY_FILTERS,[base])).toBe('');
});
test('thinking and model waits are running, not command execution or task completion', () => {
  expect(taskActivity({...base,phase:'thinking',activity:'old command'})).toBe('正在思考');
  expect(taskActivity({...base,phase:'awaiting_model',activity:'old command'})).toContain('本轮仍在运行');
  expect(taskActivity({...base,phase:'tool',activity:'cargo test'})).toBe('cargo test');
  expect(taskActivity(base)).toContain('来源未提供执行阶段');
  expect(taskSection({...base,phase:'thinking'})).toBe('ongoing');
});
test('normal turn end is an unread result, not a blocking prompt', () => {
  const ended = {...base,status:'waiting_input' as const,waiting_reason:'turn_ended' as const};
  expect(taskSection(ended)).toBe('results');
  expect(taskLabel(ended)).toBe('本轮结束');
  expect(taskSection({...ended,waiting_reason:'approval'})).toBe('ongoing');
  expect(taskSection({...ended,waiting_reason:'question'})).toBe('ongoing');
  expect(taskSection({...base,stale:true})).toBe('uncertain');
});

test('running and explicit blockers come first; failed results are viewable, source faults are not failures',()=>{
  const failed={...base,status:'failed' as const};
  expect(taskSection(failed)).toBe('results');
  expect(taskSection({...failed,unread:false,pinned:true})).toBe('pinned');
  expect(taskSection({...base,status:'unknown'})).toBe('uncertain');
  expect(sectionRank(base)).toBeLessThan(sectionRank(failed));
  expect(matchesSignal(failed,'failed')).toBe(true);
  expect(matchesSignal({...failed,unread:false},'failed')).toBe(false);
  expect(matchesSignal({...base,stale:true},'approval')).toBe(false);
  expect(runningOutsideView([{...base,stale:true}],[])).toEqual([]);
  expect(filterTasks([base,failed],{...EMPTY_FILTERS,signal:'failed'})).toEqual([failed]);
  expect(overviewFilters('ongoing',null).signal).toBeNull();
});

test('only a recorded non-Agent shell return is an inferred result; never advertise exit success',()=>{
  const ended={...base,source:'remote_tmux' as const,confidence:'inferred' as const,status:'idle' as const,finished_at:100};
  expect(taskLabel(ended)).toBe('观察到结束 · 退出码未知');
  expect(isEndedResult(ended)).toBe(true);
  expect(taskSection(ended)).toBe('results');
  expect(isEndedResult({...ended,finished_at:undefined})).toBe(false);
  expect(isEndedResult({...ended,source:'codex'})).toBe(false);
  expect(isEndedResult({...ended,stale:true})).toBe(false);
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

test('local WSL Codex has a separate source label and project identity', () => {
  const common={enabled:true,projects:['*'],excluded_projects:[],tmux_sessions:[],tmux_panes:[],poll_interval_ms:3000};
  const settings:DashboardSettings={version:2,onboarding_complete:true,notifications:true,sound:false,default_view:'tasks',sources:[
    {...common,id:'codex',name:'Codex · 本机',kind:'codex',location:''},
    {...common,id:'codex-wsl:ubuntu',name:'Codex · WSL Ubuntu',kind:'codex_wsl',location:'Ubuntu'}
  ]};
  const task={...base,source_instance:'codex-wsl:ubuntu',project:'E-commerce Agent',project_path:'/home/user/projects/E-commerce Agent'};
  expect(taskContext(task,settings)).toBe('E-commerce Agent · Codex · WSL Ubuntu');
  expect(taskSource(task,settings)?.id).toBe('codex-wsl:ubuntu');
  expect(projectKey(task,settings)).not.toBe(projectKey({...task,source_instance:'codex'},settings));
  expect(sourceFreshness(task,settings,[{provider:'codex',status:'connected',last_success:100}],100)).toBe('来源尚未成功采集');
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
  expect(isEndedResult({...base,status:'failed'})).toBe(true);
  for(const status of ['running','idle'] as const)expect(isEndedResult({...base,status})).toBe(false);
  expect(isEndedResult({...base,status:'waiting_input',waiting_reason:'question'})).toBe(false);
  expect(isEndedResult({...base,status:'succeeded',stale:true})).toBe(false);
});

test('reported models are searchable without changing Agent identity or task ordering',()=>{
  const task={...base,source:'claude_code' as const,model:'deepseek-v4-flash'};
  expect(taskAgent(task)).toBe(taskAgent({...task,model:undefined}));
  expect(filterTasks([task],{...EMPTY_FILTERS,query:'DEEPSEEK flash'})).toEqual([task]);
  expect(filterTasks([base],{...EMPTY_FILTERS,query:'deepseek'})).toEqual([]);
  expect(taskSection(task)).toBe(taskSection({...task,model:'another-model'}));
  expect(compareTasks(task,{...task,model:'another-model'})).toBe(0);
});

test('panel grows for four tasks while empty and single-task views remain compact',()=>{
  expect([0,1,2,3,4,27].map(count=>preferredDashboardHeight(count,false))).toEqual([400,400,560,680,840,840]);
  expect(preferredDashboardHeight(0,true)).toBe(840);
  expect(preferredDashboardHeight(1,true)).toBe(840);
});
