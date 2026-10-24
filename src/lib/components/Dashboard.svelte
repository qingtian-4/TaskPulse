<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { invoke, isTauri } from '@tauri-apps/api/core';
  import { emit, listen } from '@tauri-apps/api/event';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { SOURCE_LABEL, CONFIDENCE_LABEL, formatAgo, type TaskSnapshot, type DashboardSettings, type ProviderHealth, type CodexAccountQuota, type SourceDiscovery } from '$lib/types';
  import { taskSection, taskTone, compareTasks, matchesSignal, taskLabel, taskActivity, taskContext, projectKey, sourceFreshness, refreshProgress, refreshResultMessage, unconfirmedRunning, dashboardContinuity, filterTasks, overviewFilters, runningOutsideView, filterDescription, SECTION_NAMES, SECTION_ORDER, type TaskSection, type TaskSignal } from '$lib/dashboard';
  import TaskList from './TaskList.svelte';
  import MessageCenter from './MessageCenter.svelte';
  import {observedDashboardTasks} from '$lib/dashboard';
  import {acceptMessageState} from '$lib/messages';
  import type {MessageCenterState} from '$lib/types';
  import Settings from '@lucide/svelte/icons/settings';
  import RefreshCw from '@lucide/svelte/icons/refresh-cw';
  import Pin from '@lucide/svelte/icons/pin';
  import PinOff from '@lucide/svelte/icons/pin-off';
  import Search from '@lucide/svelte/icons/search';
  import Activity from '@lucide/svelte/icons/activity';
  import SlidersHorizontal from '@lucide/svelte/icons/sliders-horizontal';
  import Inbox from '@lucide/svelte/icons/inbox';
  import Bell from '@lucide/svelte/icons/bell';

  let tasks = $state<TaskSnapshot[]>([]);
  let archived = $state<TaskSnapshot[]>([]);
  let showArchive = $state(false);
  let showFilters = $state(false);
  let filterSection = $state<TaskSection | null>(null);
  let filterSignal = $state<TaskSignal | null>(null);
  let now = $state(Date.now());
  let providers = $state<ProviderHealth[]>([]);
  let settings = $state<DashboardSettings | null>(null);
  let quota = $state<CodexAccountQuota | null>(null);
  let selected = $state<TaskSnapshot | null>(null);
  let filterSource = $state('');
  let filterProject = $state('');
  let filterLocation = $state('');
  let query = $state('');
  let unreadOnly = $state(false);
  let view = $state<'tasks' | 'projects'>('tasks');
  let error = $state('');
  let note = $state('');
  let refreshing = $state(false);
  let pinned = $state(false);
  let overPopup = $state(false);
  let openingManagement = false;
  let newSources = $state(0);
  let pendingPrompts = $state<TaskSnapshot[]>([]);
  let showPrompts = $state(false);
  let messages = $state<MessageCenterState>({revision:0,unread:0,entries:[],error:null});
  let messagesEpoch=0;
  let panel = $state<HTMLDivElement | null>(null);
  let demo = $state(false);
  let lastHidden = $state<{task:TaskSnapshot;action:'restore'|'unignore'} | null>(null);
  let refreshSince = $state(0);
  let layoutReady = $state(false);
  let sessionHeight = $state(400);
  let quotaOpen = $state(false);
  let resizeQueue:Promise<unknown> = Promise.resolve();
  const progress = $derived(refreshProgress(settings,providers,refreshSince));
  const confirmedTasks = $derived(demo ? tasks : observedDashboardTasks(tasks,providers,settings,now));
  const selectedTask = $derived([...confirmedTasks,...archived].find(t => t.id === selected?.id) ?? selected);
  const displayed = $derived(showArchive ? archived : confirmedTasks);
  const filters = $derived({source:filterSource,project:filterProject,location:filterLocation,section:filterSection,signal:filterSignal,query,unreadOnly});
  const filtered = $derived(filterTasks(displayed,filters,settings));
  const outsideRunning = $derived(runningOutsideView(confirmedTasks,filtered));
  const pendingRunning = $derived(unconfirmedRunning(confirmedTasks));
  const uncertainTasks = $derived(confirmedTasks.filter(t=>taskSection(t)==='uncertain'));
  let refreshSources = $state<string[]>([]);
  const refreshResult = $derived(refreshSources.length ? refreshResultMessage(refreshSources,providers.filter(p=>p.status==='degraded').map(p=>p.provider),Object.fromEntries(settings?.sources.map(s=>[s.id,s.name])??[])) : '');
  const filterText = $derived(filterDescription(filters,displayed,settings));
  const sorted = $derived([...filtered].sort(compareTasks));
  const projects = $derived([...new Map(displayed.map(t => [projectKey(t,settings), { key:projectKey(t,settings), label:taskContext(t,settings), path:t.project_path }])).values()]);
  const locations = $derived([...new Set(displayed.map(t => t.host ?? '本机'))]);
  const projectGroups = $derived([...new Map(sorted.map(t => [projectKey(t,settings), { title:taskContext(t,settings), tasks:sorted.filter(x => projectKey(x,settings) === projectKey(t,settings)) }])).values()]);
  const degraded = $derived(providers.filter(p => p.status === 'degraded'));

  async function snapshots() {
    const epoch=messagesEpoch;
    const [nextTasks,nextProviders,nextSettings,nextQuota,nextArchived,nextPrompts] = await Promise.all([
      invoke<TaskSnapshot[]>('get_agents'), invoke<ProviderHealth[]>('get_providers'), invoke<DashboardSettings>('get_monitoring_settings'), invoke<CodexAccountQuota>('get_codex_quota'), invoke<TaskSnapshot[]>('get_archived_attention'), invoke<TaskSnapshot[]>('get_pending_task_prompts')
    ]);
    tasks=nextTasks; providers=nextProviders; settings=nextSettings; quota=nextQuota; archived=nextArchived; pendingPrompts=nextPrompts;
    const center=await invoke<MessageCenterState>('get_message_center');
    if(epoch===messagesEpoch)messages=acceptMessageState(messages,center);
  }
  async function readMessages(ids:string[]) {
    if(!ids.length||!isTauri())return;
    try{const center=await invoke<MessageCenterState>('mark_messages_seen',{ids});messagesEpoch++;messages=acceptMessageState(messages,center);}
    catch(e){error='消息已读状态未保存：'+String(e);}
  }
  async function selectMessage(id:string) {
    showPrompts=false;
    const task=[...tasks,...archived,...pendingPrompts].find(task=>task.id===id);
    if(task)selectTask(task);
    else if(isTauri())try{await invoke('open_task',{id});}catch(e){error=String(e);}
  }
  async function reopenRequest(id:string,requestId?:string|null,turnKey?:string|null) {
    try{await invoke('reopen_task_prompt',{id,requestId,turnKey});note='已重新打开原提醒；消息已读、任务未读和请求状态均不改变';}
    catch(e){error=String(e);}
  }
  async function refresh() {
    if (!isTauri() || refreshing) return;
    refreshing=true; error=''; refreshSources=[];
    try {
      refreshSince=await invoke<number>('request_refresh'); note='';
      for(let i=0;i<100;i++) {
        await new Promise(r=>setTimeout(r,300)); await snapshots();
        const result=refreshProgress(settings,providers,refreshSince);
        if(!result.pending.length) {refreshSources=settings?.sources.filter(s=>s.enabled).map(s=>s.id)??[];return;}
      }
      error='刷新超时，未返回：'+progress.pending.map(s=>s.name).join('、')+'。保留原状态，不视为已完成。';
    }
    catch(e) { error='刷新失败：'+String(e); } finally { refreshing=false; }
  }
  async function manage() {
    if (!isTauri()) { note='浏览器演示：真实设置请在托盘应用中打开'; return; }
    openingManagement=true;
    try { await invoke('open_management'); } catch(e) { error=String(e);openingManagement=false; }
  }
  async function action(task: TaskSnapshot, name: string, value?: boolean) {
    if (!isTauri()) return;
    try {
      tasks=await invoke<TaskSnapshot[]>('update_task_attention',{id:task.id,action:name,pinned:value});
      if (name==='hide' || name==='ignore') { lastHidden={task,action:name==='hide'?'restore':'unignore'}; selected=null; }
      else if (name==='ignore' || !tasks.some(t=>t.id===selected?.id)) selected=null;
      archived=await invoke<TaskSnapshot[]>('get_archived_attention');
    } catch(e) { error=String(e); }
  }
  async function copy(task:TaskSnapshot) {
    try {await navigator.clipboard.writeText(`${task.name}\n${taskContext(task,settings)}\n${task.project_path ?? ''}\n${task.session_id ?? task.id}`);note='已复制任务标题、项目及会话 ID';}catch(e){error='复制失败：'+String(e);}
  }
  function resetFilters(){filterSource='';filterProject='';filterLocation='';filterSection=null;filterSignal=null;query='';unreadOnly=false;}
  function chooseSection(section:TaskSection){
    const next=overviewFilters(section,filterSection);
    showArchive=false;resetFilters();filterSection=next.section;
  }
  function showRunning(){showArchive=false;resetFilters();filterSection='ongoing';}
  function chooseSignal(signal:TaskSignal){const active=filterSignal===signal;showArchive=false;resetFilters();filterSignal=active?null:signal;}
  function selectTask(task:TaskSnapshot){
    resetFilters();showArchive=archived.some(t=>t.id===task.id);selected=task;
  }
  async function open(task: TaskSnapshot) {
    if (!isTauri()) return;
    try { const outcome=await invoke<string>('open_task',{id:task.id}); note=outcome==='window_focused' ? '已聚焦运行窗口；请在查看结果后标记已查看' : '已定位任务详情；此来源暂不支持精确会话跳转'; }
    catch(e) { error=String(e); }
  }
  // Discrete, grow-only height within one opening. Raw task count is independent
  // of stale status, search and filtering; collection refresh cannot oscillate it.
  $effect(()=>{
    if(!layoutReady)return;
    const expanded=showPrompts || !!selectedTask || showFilters || showArchive || quotaOpen || !!error || !!note;
    const needed=tasks.length>1 || expanded ? 620 : 400;
    if(needed>sessionHeight)sessionHeight=needed;
  });
  $effect(()=>{
    if(!layoutReady || !isTauri())return;
    const height=sessionHeight;
    // Serialize native requests; older small-size requests cannot win a race
    // with opening details. Work-area/DPI fitting happens in the native layer.
    resizeQueue=resizeQueue.then(()=>invoke('resize_dashboard',{height})).catch(e=>{error='窗口尺寸调整失败：'+String(e);});
  });
  onMount(()=>{
    const searchShortcut=(event:KeyboardEvent)=>{
      const target=event.target as HTMLElement;
      if(event.key==='/' && !['INPUT','TEXTAREA','SELECT'].includes(target.tagName) && !target.isContentEditable){event.preventDefault();panel?.querySelector<HTMLInputElement>('[aria-label="搜索任务"]')?.focus();}
    };
    window.addEventListener('keydown',searchShortcut);
    const clock=setInterval(()=>now=Date.now(),1000);
    if (!isTauri()) {
      if (import.meta.env.DEV) {
        demo=true;
        const now=Date.now();
        tasks=[
          {id:'demo-gate',name:'演示：梳理项目流程',source:'codex',status:'waiting_input',waiting_reason:'approval',project:'demo-project',host:'DemoServer',remote:true,updated_at:now,confidence:'hook',unread:true},
          {id:'demo-run',remote:false,name:'演示：研究记忆机制',source:'codex',status:'running',project:'demo-project',source_instance:'codex',activity:'执行测试 · pytest tests/',started_at:now-310000,updated_at:now,confidence:'hook',unread:true},
          {id:'demo-dsh',remote:false,name:'演示：客服任务',source:'deepseek_harness',status:'running',project:'demo-shop',source_instance:'deepseek_harness_desktop',activity:'读取项目文件',started_at:now-120000,updated_at:now,confidence:'hook',unread:true},
          {id:'demo-done',remote:false,name:'演示：界面评审',source:'codex',status:'waiting_input',waiting_reason:'turn_ended',project:'demo-ui',updated_at:now-60000,confidence:'hook',unread:true},
        ];
      }
      return ()=>{clearInterval(clock);window.removeEventListener('keydown',searchShortcut);};
    }
    const cleanups:Array<()=>void>=[]; let mounted=true;
    const discover=async()=>{
      try {
        const found=await invoke<SourceDiscovery[]>('discover_sources');
        if(mounted)newSources=found.filter(s=>s.available && s.candidate && !['hooks','scanner'].includes(s.candidate.kind)).length;
      } catch { /* Candidate hints must not block normal monitoring. */ }
    };
    const discoveryTimer=setInterval(()=>void discover(),30000);
    async function subscribe(promise:Promise<()=>void>) { const cleanup=await promise; if(mounted)cleanups.push(cleanup);else cleanup(); }
    void (async()=>{
      try {
        await subscribe(listen<TaskSnapshot[]>('agents-updated',e=>tasks=e.payload));
        await subscribe(listen<TaskSnapshot[]>('archived-attention-updated',e=>archived=e.payload));
        await subscribe(listen<ProviderHealth[]>('providers-updated',e=>providers=e.payload));
        await subscribe(listen<TaskSnapshot[]>('pending-prompts-updated',e=>pendingPrompts=e.payload));
        await subscribe(listen<MessageCenterState>('messages-updated',e=>{messagesEpoch++;messages=acceptMessageState(messages,e.payload);}));
        await subscribe(listen<CodexAccountQuota>('codex-quota-updated',e=>quota=e.payload));
        await subscribe(listen<DashboardSettings>('monitoring-settings-updated',e=>{settings=e.payload;view=e.payload.default_view;void discover();}));
        await subscribe(listen<TaskSnapshot>('task-selected',e=>selectTask(e.payload)));
        await subscribe(listen<boolean>('pinned-changed',e=>pinned=e.payload));
        await subscribe(listen('popup-opened',()=>{if(!pinned)sessionHeight=400;}));
        await subscribe(listen('management-closed',()=>openingManagement=false));
        await snapshots(); view=settings?.default_view ?? 'tasks';
        void discover();
        error=await invoke<string | null>('get_monitoring_error') ?? '';
        const selection=await invoke<TaskSnapshot | null>('get_selected_task');
        if(selection)selectTask(selection);
        pinned=await invoke<boolean>('get_pinned');
        const win=getCurrentWindow(); let showTime=Date.now();
        await subscribe(win.onFocusChanged(({payload:focused})=>{
          if(focused) {showTime=Date.now();showArchive=false;resetFilters();void snapshots().catch(e=>error=String(e));}
          else if(!pinned && !overPopup && !openingManagement && Date.now()-showTime>300) void invoke('close_popup');
        }));
      } catch(e) { error='无法读取监控数据：'+String(e); }
      finally { layoutReady=true; await tick(); await resizeQueue; await emit('popup-ready',null).catch(()=>{}); }
    })();
    return ()=>{mounted=false;window.removeEventListener('keydown',searchShortcut);clearInterval(clock);clearInterval(discoveryTimer);cleanups.forEach(fn=>fn());};
  });
</script>

<div bind:this={panel} role="region" aria-label="TaskPulse 任务监控" class="dashboard tp-theme" data-continuity={JSON.stringify(dashboardContinuity(tasks,filtered))} onpointerenter={()=>overPopup=true} onpointerleave={()=>overPopup=false}>
  <header data-tauri-drag-region>
    <span class="brand" data-tauri-drag-region><span class="brand-mark" aria-hidden="true"><Activity size={17}/></span>TaskPulse</span>
    <div class="header-actions">
      <button class="tp-btn tp-icon" data-variant="ghost" onclick={()=>showPrompts=!showPrompts} aria-expanded={showPrompts} title={'消息中心 · '+messages.unread+' 条未读'} aria-label={'消息中心：'+messages.unread+' 条未读'}><Bell size={15}/>{#if messages.unread}<span>{messages.unread}</span>{/if}</button>
      <button class="tp-btn tp-icon" data-variant="ghost" onclick={manage} title="管理监控来源" aria-label="管理监控来源"><Settings size={15}/></button>
      <button class="tp-btn tp-icon" data-variant="ghost" onclick={refresh} disabled={refreshing || demo} title="立即刷新" aria-label="立即刷新"><RefreshCw size={15} class={refreshing?'animate-spin':''}/></button>
      <button class="tp-btn tp-icon" data-variant="ghost" disabled={demo} onclick={()=>invoke('toggle_pin')} title={pinned?'取消固定窗口':'固定窗口'} aria-label={pinned?'取消固定窗口':'固定窗口'}>{#if pinned}<Pin size={15}/>{:else}<PinOff size={15}/>{/if}</button>
    </div>
  </header>
  <div class="counts" aria-label="任务概览" aria-describedby="count-help">
    <button class="overview tp-tone" data-tone="running" class:active={filterSection==='ongoing'} aria-pressed={filterSection==='ongoing'} title="来源新鲜的运行任务，包含明确待处理请求" onclick={()=>chooseSection('ongoing')}><span>进行中</span><b>{confirmedTasks.filter(t=>taskSection(t)==='ongoing').length}</b></button>
    <button class="overview tp-tone" data-tone="action" class:active={filterSignal==='action'} aria-pressed={filterSignal==='action'} title="进行中的子集：明确待批准、待回答或额度耗尽" onclick={()=>chooseSignal('action')}><span>待处理</span><b>{confirmedTasks.filter(t=>matchesSignal(t,'action')).length}</b></button>
    <button class="overview tp-tone" data-tone="unread" class:active={filterSection==='results'} aria-pressed={filterSection==='results'} title="尚未查看的本轮结果，包括失败" onclick={()=>chooseSection('results')}><span>待查看</span><b>{confirmedTasks.filter(t=>taskSection(t)==='results').length}</b></button>
  </div>
  <p id="count-help" class="sr-only">待处理包含在进行中，计数不相加。</p>
  {#if pendingRunning.length}<p class="notice pending-running" role="status">上次运行 {pendingRunning.length} · 当前采集待更新，不代表结束<button class="link" onclick={()=>{showArchive=false;resetFilters();filterSection='uncertain';}}>查看待确认任务</button></p>{/if}
  {#if confirmedTasks.some(t=>matchesSignal(t,'approval') || matchesSignal(t,'question') || matchesSignal(t,'failed')) || degraded.length || uncertainTasks.length}<div class="signals">
    {#if confirmedTasks.some(t=>matchesSignal(t,'approval'))}<button class="tp-chip tp-tone" data-tone="action" class:active={filterSignal==='approval'} aria-pressed={filterSignal==='approval'} onclick={()=>chooseSignal('approval')}>待批准 {confirmedTasks.filter(t=>matchesSignal(t,'approval')).length}</button>{/if}
    {#if confirmedTasks.some(t=>matchesSignal(t,'question'))}<button class="tp-chip tp-tone" data-tone="action" class:active={filterSignal==='question'} aria-pressed={filterSignal==='question'} onclick={()=>chooseSignal('question')}>待回答 {confirmedTasks.filter(t=>matchesSignal(t,'question')).length}</button>{/if}
    {#if confirmedTasks.some(t=>matchesSignal(t,'failed'))}<button class="tp-chip tp-tone" data-tone="danger" class:active={filterSignal==='failed'} aria-pressed={filterSignal==='failed'} onclick={()=>chooseSignal('failed')}>失败结果 {confirmedTasks.filter(t=>matchesSignal(t,'failed')).length}</button>{/if}
    {#if degraded.length}<button class="tp-chip tp-tone" data-tone="stale" onclick={manage}>来源待更新 {degraded.length}</button>{/if}
    {#if uncertainTasks.length && !pendingRunning.length}<button class="tp-chip tp-tone" data-tone="stale" onclick={()=>chooseSection('uncertain')}>状态待确认 {uncertainTasks.length}</button>{/if}
  </div>{/if}
  <div class="view-bar"><button class="all-tasks" class:active={!showArchive && !filterText} onclick={()=>{showArchive=false;resetFilters();}}>全部 {confirmedTasks.length}</button>{#if archived.length}<button class:active={showArchive} onclick={()=>{showArchive=!showArchive;resetFilters();}}>较早关注 {archived.length}</button>{/if}<button class="filter-toggle tp-btn" data-variant="ghost" aria-expanded={showFilters} onclick={()=>showFilters=!showFilters}><SlidersHorizontal size={12}/>{showFilters?'收起筛选':'筛选 / 分组'}</button></div>
  <div class="search-bar"><label class="search"><Search size={14}/><input aria-label="搜索任务" placeholder="搜索任务、项目或 Agent   /" bind:value={query} onkeydown={e=>{if(e.key==='Escape'){query='';e.stopPropagation();}}}/>{#if query}<button onclick={()=>query=''} aria-label="清除搜索">×</button>{/if}</label></div>
  {#if demo}<p class="notice">浏览器演示 · 以下均为示例，不是真实采集结果</p>{/if}
  {#if showPrompts}<MessageCenter center={messages} prompts={pendingPrompts} {now} onRead={readMessages} onClose={()=>showPrompts=false} onSelect={id=>void selectMessage(id)} onReopen={entry=>void reopenRequest(entry.task_id,entry.request_id,entry.turn_key)} onReopenPrompt={task=>void reopenRequest(task.id,task.input_request_id,task.turn_key)} onPrompt={task=>{showPrompts=false;selectTask(task);}} />{/if}
  {#if error}<p class="notice error" role="alert">{error}</p>{/if}
  {#if note}<p class="notice" role="status">{note}<button class="dismiss" onclick={()=>note=''} aria-label="关闭提示">×</button></p>{/if}
  {#if refreshResult}<p class="notice" role="status">{refreshResult}<button class="dismiss" onclick={()=>refreshSources=[]} aria-label="关闭刷新结果">×</button></p>{/if}
  {#if settings && !settings.onboarding_complete}<div class="onboarding"><strong>选择你要关注的 Agent</strong><p>确认本机来源，或添加远程服务器。新任务会自动出现。</p><button onclick={manage}>设置监控来源</button></div>{/if}
  {#if settings?.onboarding_complete && newSources}<p class="notice">发现 {newSources} 个可监控来源<button class="link" onclick={manage}>选择添加</button></p>{/if}
  {#if refreshing}<p class="notice" role="status">刷新中：{progress.total-progress.pending.length} / {progress.total} 个来源已返回{#if progress.failed.length} · {progress.failed.length} 个失败{/if}</p>{/if}
  {#if showArchive}<p class="notice">较早关注保留原状态和未读标记，不占近期面板；可重新加入或固定。</p>{/if}
  {#if lastHidden}<div class="notice">{lastHidden.action==='restore'?'已隐藏本轮':'已停止监控此会话'}：{lastHidden.task.name}<button class="link" onclick={async()=>{const previous=lastHidden!;await action(previous.task,previous.action);lastHidden=null;}}>撤销</button></div>{/if}
  {#if showFilters}
  <div class="filters">
    <select bind:value={filterSource} aria-label="筛选来源"><option value="">全部来源</option>{#each settings?.sources.filter(s=>s.enabled) ?? [] as s}<option value={s.id}>{s.name}</option>{/each}</select>
    <select bind:value={filterProject} aria-label="筛选项目"><option value="">全部项目</option>{#each projects as p}<option value={p.key}>{p.label}</option>{/each}</select>
    {#if locations.length>1}<select bind:value={filterLocation} aria-label="筛选位置"><option value="">全部位置</option>{#each locations as location}<option>{location}</option>{/each}</select>{/if}
    <button class="view-toggle" onclick={()=>view=view==='tasks'?'projects':'tasks'} title="切换任务/项目视图">{view==='tasks'?'按项目':'按任务'}</button>
    <button class="unread-filter tp-chip tp-tone" data-tone="unread" class:active={unreadOnly} aria-pressed={unreadOnly} onclick={()=>unreadOnly=!unreadOnly} title="筛选所有尚未查看的任务，含运行中任务；不同于上方的结果待查看计数">仅未查看 {displayed.filter(t=>t.unread).length}</button>
  </div>
  {/if}
  {#if filterSource || filterProject || filterLocation || filterSection!==null || filterSignal || query.trim() || unreadOnly}<div class="filter-note"><div><span>当前筛选 {filtered.length} / {displayed.length} 项</span><p>{filterText}</p></div><button class="link" onclick={resetFilters}>清除筛选</button></div>{/if}
  {#if outsideRunning.length}<p class="notice outside-running" role="status">{outsideRunning.length} 个运行任务在当前视图之外<button class="link" onclick={showRunning}>查看全部运行任务</button></p>{/if}
  <main class="glass-scroll task-scroll" aria-label="关注任务">
    {#if sorted.length===0}<div class="empty"><span class="empty-icon" aria-hidden="true"><Inbox size={24}/></span><strong>{displayed.length?'当前筛选没有任务':'当前没有需要关注的任务'}</strong><p>{displayed.length?'任务已采集，但不符合上方筛选条件。':'已启用范围内的新执行会自动加入；旧项目再次执行也会回来。'}</p>{#if filterText}<button onclick={()=>{showArchive=false;resetFilters();}}>清除筛选，查看全部任务</button>{:else}<button onclick={manage}>查看来源与已发现会话</button>{/if}</div>
    {:else if view==='tasks'}
      {#each SECTION_ORDER as section}{@const group=sorted.filter(t=>taskSection(t)===section)}{#if group.length}<section><h2>{SECTION_NAMES[section]}<span>{group.length}</span></h2><TaskList tasks={group} {settings} {providers} {now} selectedId={selected?.id ?? null} onSelect={t=>selected=t} onViewed={t=>void action(t,'viewed')}/></section>{/if}{/each}
    {:else}{#each projectGroups as group}<section><h2>{group.title}<span>{group.tasks.length}</span></h2><TaskList tasks={group.tasks} {settings} {providers} {now} selectedId={selected?.id ?? null} onSelect={t=>selected=t} onViewed={t=>void action(t,'viewed')}/></section>{/each}{/if}
  </main>
  {#if selectedTask}
    <aside class="detail glass-scroll tp-tone" data-tone={taskTone(selectedTask)} aria-label="任务详情">
      <div class="detail-heading"><strong title={selectedTask.name}>{selectedTask.name}</strong><span class="tp-chip">{taskLabel(selectedTask)}</span><button class="tp-icon" onclick={()=>selected=null} aria-label="关闭详情">×</button></div>
      <p>{taskContext(selectedTask,settings)}</p>
      <details class="evidence tp-fold"><summary>状态依据与采集时间</summary><p>状态依据：{CONFIDENCE_LABEL[selectedTask.confidence]}{#if selectedTask.model} · 上报模型：{selectedTask.model}{/if}</p><p>{sourceFreshness(selectedTask,settings,providers,now)} · 最后活动 {formatAgo(selectedTask.updated_at,now)}</p></details>
      <div class="command glass-scroll"><span>{selectedTask.stale?'上次观测活动（非实时）':selectedTask.status==='running'?'当前活动':'最近消息'}</span><pre>{taskActivity(selectedTask)}</pre></div>
      <div class="detail-actions">
        {#if selectedTask.can_focus}<button class="tp-btn" title="只聚焦宿主窗口，不标记已查看" onclick={()=>open(selectedTask!)}>{selectedTask.source==='claude_code'?'聚焦项目窗口':'聚焦运行窗口'}</button>{/if}
        <button class="tp-btn" data-variant="ghost" onclick={()=>copy(selectedTask!)}>复制任务信息</button>
        {#if selectedTask.unread}<button class="tp-btn" data-variant="primary" title="更新本轮已查看标记，不代表批准或项目完成" onclick={()=>action(selectedTask!,'viewed')}>已查看</button>{/if}
        {#if showArchive}<button class="tp-btn" onclick={()=>action(selectedTask!,'follow')}>重新加入</button>{/if}
        <button class="tp-btn" data-variant="ghost" onclick={()=>action(selectedTask!,'pin',showArchive ? true : !selectedTask!.pinned)}>{showArchive?'固定到近期面板':selectedTask.pinned?'取消固定':'固定'}</button>
        <details class="more tp-fold"><summary>更多关注操作</summary>{#if selectedTask.status!=='running'}<button onclick={()=>action(selectedTask!,'hide')}>隐藏本轮（新执行会恢复）</button>{/if}<button onclick={()=>action(selectedTask!,'ignore')}>停止监控此会话（需手动恢复）</button></details>
      </div>
    </aside>
  {/if}
  <footer>
    <button class:degraded={degraded.length>0} onclick={manage} title="管理来源；任务详情可查看采集时间">{settings?.sources.filter(s=>s.enabled).length ?? 0} 个来源{degraded.length ? ' · '+degraded.length+' 个待检查' : ' · 正常'}</button>
    {#if !demo}<details class="quota" bind:open={quotaOpen}><summary title="本机 Codex 账号额度，不代表 WSL 或远程账号">本地额度 <span>{quota?.refreshing?'更新中…':quota?.quota ? '5h '+Math.round(100-quota.quota.used_5h_pct)+'%'+(quota.stale?' · 待更新':''):quota?.error?'待检查':'暂无数据'}</span></summary><p>{#if quota?.quota}5h 剩余 {Math.round(100-quota.quota.used_5h_pct)}% · 7天剩余 {Math.round(100-quota.quota.used_1w_pct)}%{#if quota.quota.resets_5h_at}<br/>5h 重置：{new Date(quota.quota.resets_5h_at*1000).toLocaleString()}{/if}{#if quota.quota.resets_1w_at}<br/>7天重置：{new Date(quota.quota.resets_1w_at*1000).toLocaleString()}{/if}{#if quota.observed_at}<br/>读取于 {formatAgo(quota.observed_at,now)}{/if}{:else}尚未获得本机账号额度{/if}{#if quota?.error}<br/>{quota.error}{/if}<br/>不代表 WSL 或远程服务器账号额度。</p></details>{/if}
  </footer>
</div>
<style>
  .dashboard{position:relative;height:100dvh;display:flex;flex-direction:column;overflow:hidden;border:1px solid var(--tp-border-strong);border-radius:var(--tp-radius-panel);background:var(--tp-panel)}
  header{display:flex;justify-content:space-between;align-items:center;padding:10px 12px 10px 16px;border-bottom:1px solid var(--tp-border);flex:none;min-height:52px}
  .brand{display:flex;align-items:center;gap:8px;font-size:14px;font-weight:650;color:var(--tp-text)}
  .brand-mark{width:24px;height:24px;border:1px solid var(--tp-accent-line);border-radius:6px;display:grid;place-items:center;background:var(--tp-accent-soft);color:var(--tp-accent)}
  .header-actions{display:flex;gap:4px}.header-actions button{border:0;position:relative}.header-actions span{position:absolute;right:0;top:0;font-size:10px;min-width:14px;line-height:14px;text-align:center;border-radius:4px;background:var(--tp-card);color:var(--tp-action)}
  .counts{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:8px;padding:12px 12px 8px;flex:none}
  .overview{text-align:left;padding:8px 12px;border:1px solid var(--tp-border);border-radius:8px;background:var(--tp-card);min-width:0}
  .overview span{font-size:11px;color:var(--tp-muted);white-space:nowrap}.overview b{display:block;font-size:24px;line-height:1.25;font-weight:600;letter-spacing:-.03em;color:var(--tone);margin-top:4px;font-variant-numeric:tabular-nums}
  .overview:hover{border-color:var(--tone-line);background:var(--tp-raised)}.overview.active{border-color:var(--tone-line);background:var(--tone-soft)}.overview.active span{color:var(--tp-text)}
  .signals{display:flex;gap:8px;flex-wrap:wrap;padding:0 12px 4px;max-height:68px;overflow:auto;flex:none}.signals button{font-size:11px;padding:2px 8px;border-radius:16px}.signals .active{box-shadow:inset 0 0 0 1px var(--tone)}
  .view-bar{display:flex;align-items:center;gap:12px;padding:4px 12px 8px;flex:none}.view-bar>button{font-size:11px;color:var(--tp-muted);border-radius:4px;padding:4px 0}.view-bar>button.active{color:var(--tp-accent)}.view-bar .filter-toggle{margin-left:auto;min-height:28px;padding:4px 0;gap:4px;font-size:11px}
  .search-bar{display:flex;padding:0 12px 8px;flex:none}.search{display:flex;gap:8px;align-items:center;min-height:36px;border:1px solid var(--tp-border-strong);background:var(--tp-input);border-radius:8px;padding:6px 8px;min-width:0;flex:1;color:var(--tp-muted)}
  .search:focus-within{border-color:var(--tp-accent-line)}.search input{width:100%;min-width:0;border:0;outline:none;background:transparent;color:var(--tp-text);font-size:12px;line-height:1.5;padding:0;border-radius:0}.search input:focus-visible{outline:none}.search input::placeholder{color:var(--tp-quiet)}.search>button{font-size:16px;width:24px;line-height:24px}
  .unread-filter{font-size:11px;white-space:nowrap;padding:4px 8px}.unread-filter.active{border-color:var(--tone-line);background:var(--tone-soft);color:var(--tone)}
  .task-scroll{min-height:80px;overflow-y:auto;flex:1;padding:0 12px 8px;overscroll-behavior:contain}
  h2{font-size:11px;font-weight:550;color:var(--tp-muted);padding:8px 0 0;margin:0;display:flex;gap:8px;min-width:0;overflow-wrap:anywhere}h2 span{font-size:11px;color:var(--tp-quiet);font-variant-numeric:tabular-nums}
  .empty{display:flex;flex-direction:column;align-items:center;text-align:center;padding:16px 12px;color:var(--tp-text)}.empty-icon{display:none}
  .empty strong{font-size:14px;font-weight:550}.empty p{font-size:12px;line-height:1.6;color:var(--tp-muted);margin:8px 0 12px}.empty button,.onboarding button{border:1px solid var(--tp-accent-line);padding:6px 12px;border-radius:8px;color:var(--tp-accent);background:var(--tp-accent-soft);font-size:12px}
  .notice{margin:0;padding:6px 12px;font-size:11px;line-height:1.6;background:var(--tp-accent-soft);color:var(--tp-accent);display:flex;align-items:center;gap:8px;max-height:60px;overflow:auto;flex:none;overflow-wrap:anywhere}.notice.error{color:var(--tp-danger);background:#f09ba70b}.dismiss{margin-left:auto;flex:none}.link{margin-left:auto;color:var(--tp-accent);white-space:nowrap;font-size:11px}
  .filter-note{padding:0 12px 8px;font-size:11px;color:var(--tp-muted);display:flex;align-items:flex-start;gap:8px;flex:none;max-height:60px;overflow:auto}.filter-note p{margin:4px 0 0;overflow-wrap:anywhere;font-size:11px;line-height:1.5}
  .filters{position:absolute;z-index:3;left:12px;right:12px;top:188px;display:flex;gap:8px;padding:12px;flex-wrap:wrap;max-height:42dvh;overflow:auto;background:var(--tp-raised);border:1px solid var(--tp-border-strong);border-radius:8px;box-shadow:var(--tp-shadow-window)}
  .filters select{font-size:12px;flex:1;padding:6px;min-width:110px;max-width:100%}.view-toggle{font-size:11px;border:1px solid var(--tp-border-strong);border-radius:4px;padding:4px 8px}
  .detail{margin:0 12px 8px;padding:12px;border:1px solid var(--tone-line);border-radius:8px;background:var(--tp-card);max-height:32dvh;min-height:96px;overflow:auto;flex:0 1 auto}
  .detail-heading{display:flex;gap:8px;align-items:flex-start}.detail-heading strong{flex:1;min-width:0;display:-webkit-box;-webkit-line-clamp:2;line-clamp:2;-webkit-box-orient:vertical;overflow:hidden;overflow-wrap:anywhere;font-size:14px;font-weight:600;line-height:1.5}.detail-heading>.tp-chip{flex:none;max-width:100px}.detail-heading>button{height:24px;width:24px;flex:none}
  .detail p{font-size:11px;color:var(--tp-muted);margin:4px 0;line-height:1.6;overflow-wrap:anywhere}
  .command{max-height:72px;overflow:auto;margin:8px 0;padding:8px;background:var(--tp-input);border:1px solid var(--tp-border);border-radius:4px}.command span{font-size:11px;color:var(--tp-quiet)}.command pre{white-space:pre-wrap;overflow-wrap:anywhere;font-family:inherit;font-size:12px;line-height:1.5;color:var(--tp-text);margin:4px 0}
  .evidence{font-size:11px;color:var(--tp-muted);margin:8px 0}.evidence>summary{padding:4px 8px}.evidence p{padding:0 8px}
  .detail-actions{display:flex;gap:8px;flex-wrap:wrap}.detail-actions>.tp-btn{font-size:11px;min-height:28px;padding:4px 8px}.more{font-size:11px;color:var(--tp-muted);width:100%}.more summary{padding:4px 8px}.more button{display:block;padding:8px;white-space:normal;text-align:left;font-size:11px}
  footer{display:flex;gap:8px;align-items:center;justify-content:space-between;border-top:1px solid var(--tp-border);padding:8px 12px;font-size:11px;color:var(--tp-muted);flex:none;min-height:40px}footer>button{font-size:11px;white-space:nowrap;color:var(--tp-muted)}footer>button.degraded{color:var(--tp-action)}
  .quota{font-size:11px;min-width:0;max-width:65%}.quota summary{cursor:pointer;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.quota summary span{color:var(--tp-quiet)}
  .quota p{position:absolute;z-index:3;bottom:44px;left:12px;right:12px;max-height:44dvh;overflow:auto;padding:12px;background:var(--tp-raised);border:1px solid var(--tp-border-strong);border-radius:8px;font-size:11px;line-height:1.7;overflow-wrap:anywhere;margin:0}
  .onboarding{padding:12px;border-bottom:1px solid var(--tp-border)}.onboarding strong{font-size:14px}.onboarding p{font-size:12px;color:var(--tp-muted);line-height:1.6;margin:8px 0}
  @media(max-width:350px){header{padding:8px}.brand{gap:4px}.header-actions{gap:0}.counts{gap:4px;padding:8px}.overview{padding:8px}.overview b{font-size:23px}.task-scroll{padding:0 8px 8px}.view-bar{gap:8px;padding-left:8px;padding-right:8px}.search-bar{padding-left:8px;padding-right:8px}footer{padding:8px;font-size:10px}footer>button,.quota{font-size:10px}.detail-heading{flex-wrap:wrap}}
  @media(max-height:500px){header{min-height:44px;padding-top:6px;padding-bottom:6px}.counts{padding-top:8px}.overview{padding-top:4px;padding-bottom:4px}.view-bar{padding-bottom:4px}.notice{max-height:44px}.filter-note{max-height:32px}.detail{max-height:30dvh;min-height:80px;padding:8px}.task-scroll{min-height:40px}}
</style>
