<script lang="ts">
  import {t} from '#lib/i18n.ts';
  import {DEFAULT_TASK_FONT_SIZE,preferences} from '#lib/preferences.ts';
  import { onMount, tick } from 'svelte';
  import { invoke, isTauri } from '@tauri-apps/api/core';
  import { emit, listen } from '@tauri-apps/api/event';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { type TaskSnapshot, type DashboardSettings, type ProviderHealth, type SourceDiscovery, type DashboardView, type DashboardSnapshot } from '#lib/types.ts';
  import { taskSection, compareTasks, matchesSignal, taskContext, taskModel, projectKey, enabledDashboardSources, refreshProgress, refreshResultMessage, dashboardCandidates, defaultDashboardTasks, dashboardContinuity, filterTasks, overviewFilters, runningOutsideView, filterDescription, shouldApplyDashboardSnapshot, type TaskSection, type TaskSignal } from '#lib/dashboard.ts';
  import TaskList from './TaskList.svelte';
  import TaskDetails from './TaskDetails.svelte';
  import MessageCenter from './MessageCenter.svelte';
  import {projectDashboardTasks} from '#lib/dashboard.ts';
  import {measureDashboard} from '#lib/dashboard-layout.ts';
  import {localDiscoveryCandidates,type SourceDefinition} from '#lib/source-setup.ts';
  import {acceptMessageState} from '#lib/messages.ts';
  import type {MessageCenterState} from '#lib/types.ts';
  import Settings from '@lucide/svelte/icons/settings';
  import RefreshCw from '@lucide/svelte/icons/refresh-cw';
  import Pin from '@lucide/svelte/icons/pin';
  import PinOff from '@lucide/svelte/icons/pin-off';
  import Search from '@lucide/svelte/icons/search';
  import BrandMark from '../BrandMark.svelte';
  import SlidersHorizontal from '@lucide/svelte/icons/sliders-horizontal';
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
  let selected = $state<TaskSnapshot | null>(null);
  let filterSource = $state('');
  let filterProject = $state('');
  let filterLocation = $state('');
  let query = $state('');
  let searchInput: HTMLInputElement;
  let unreadOnly = $state(false);
  let view = $state<'tasks' | 'projects'>('tasks');
  let error = $state('');
  let note = $state('');
  let refreshing = $state(false);
  let pinned = $state(false);
  let discovery=$state<SourceDiscovery[]>([]);
  let sourceCatalog=$state<SourceDefinition[]>([]);
  const newSources=$derived(localDiscoveryCandidates(sourceCatalog,discovery,settings));
  let pendingPrompts = $state<TaskSnapshot[]>([]);
  let showPrompts = $state(false);
  let messages = $state<MessageCenterState>({revision:0,unread:0,entries:[],error:null});
  let snapshotView=$state<DashboardView|null>(null);
  let dashboardRevision=$state(0);
  let snapshotRequestGeneration=0;
  let messagesEpoch=0;
  let settingsEpoch=0;
  let panel = $state<HTMLDivElement | null>(null);
  let demo = $state(false);
  let lastHidden = $state<{task:TaskSnapshot;action:'restore'|'unignore'} | null>(null);
  let refreshSince = $state(0);
  let layoutReady = $state(false);
  let sessionHeight = $state(400);
  let resizeQueue:Promise<unknown> = Promise.resolve();
  let layoutQueue:Promise<void> = Promise.resolve();
  let layoutEpoch=$state(0);let layoutTicket=0,lastWidth=0;
  const progress = $derived(refreshProgress(settings,providers,refreshSince));
  const confirmedTasks = $derived(projectDashboardTasks(tasks,snapshotView?.unconfirmed_task_ids??[]));
  const selectedTask = $derived([...confirmedTasks,...archived].find(t => t.id === selected?.id) ?? selected);
  const candidates = $derived(dashboardCandidates(confirmedTasks));
  const archiveTasks = $derived(dashboardCandidates(archived));
  const attentionTasks = $derived(defaultDashboardTasks(candidates));
  const inspecting = $derived(filterSection!==null || !!filterSignal || !!filterSource || !!filterProject || !!filterLocation || !!query.trim() || unreadOnly);
  const displayed = $derived(showArchive ? archiveTasks : inspecting ? candidates : attentionTasks);
  const filters = $derived({source:filterSource,project:filterProject,location:filterLocation,section:filterSection,signal:filterSignal,query,unreadOnly});
  const filtered = $derived(filterTasks(displayed,filters,settings));
  const outsideRunning = $derived(runningOutsideView(candidates,filtered));
  const uncertainTasks = $derived(candidates.filter(t=>taskSection(t)==='uncertain'));
  let refreshSources = $state<string[]>([]);
  const refreshResult = $derived(refreshSources.length ? refreshResultMessage(refreshSources,providers.filter(p=>p.status==='degraded').map(p=>p.provider),Object.fromEntries(settings?.sources.map(s=>[s.id,s.name])??[])) : '');
  const filterText = $derived(filterDescription(filters,displayed,settings));
  const sorted = $derived([...filtered].sort(compareTasks));
  const projects = $derived([...new Map(displayed.map(task => [projectKey(task,settings), { key:projectKey(task,settings), label:taskContext(task,settings,$t), path:task.project_path }])).values()]);
  const locations = $derived([...new Set(displayed.map(t => t.host ?? '本机'))]);
  const projectGroups = $derived.by(() => {
    const groups = new Map<string, {title:string;tasks:TaskSnapshot[]}>();
    for (const task of sorted) {
      const key = projectKey(task,settings);
      let group = groups.get(key);
      if (!group) { group = {title:taskContext(task,settings,$t),tasks:[]}; groups.set(key,group); }
      group.tasks.push(task);
    }
    return [...groups.values()];
  });
  const overview=$derived(snapshotView?.counts??{
    ongoing:candidates.filter(t=>taskSection(t)==='ongoing').length,
    action:candidates.filter(t=>matchesSignal(t,'action')).length,
    results:candidates.filter(t=>taskSection(t)==='results').length,
    uncertain:uncertainTasks.length,
  });

  function applyDashboardSnapshot(snapshot:DashboardSnapshot) {
    // A collection can finish between the one-second display ticks. Compare
    // its proof against receipt time, never the previous tick's cached time.
    now=Date.now();
    dashboardRevision=snapshot.revision; tasks=snapshot.tasks; providers=snapshot.providers; snapshotView=snapshot.view;
  }

  async function snapshots() {
    const requestGeneration=++snapshotRequestGeneration;
    const epoch=messagesEpoch;
    const settingsAtRequest=settingsEpoch;
    const [snapshot,nextSettings,nextArchived,nextPrompts] = await Promise.all([
      invoke<DashboardSnapshot>('get_dashboard_snapshot'), invoke<DashboardSettings>('get_monitoring_settings'), invoke<TaskSnapshot[]>('get_archived_attention'), invoke<TaskSnapshot[]>('get_pending_task_prompts')
    ]);
    if(requestGeneration!==snapshotRequestGeneration)return;
    if(shouldApplyDashboardSnapshot(dashboardRevision,snapshot.revision,requestGeneration,snapshotRequestGeneration)){
      applyDashboardSnapshot(snapshot); archived=nextArchived; pendingPrompts=nextPrompts;
    }
    if(settingsEpoch===settingsAtRequest)settings=nextSettings;
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
        if(!result.pending.length) {refreshSources=enabledDashboardSources(settings).map(s=>s.id);return;}
      }
      error='刷新超时，未返回：'+progress.pending.map(s=>s.name).join('、')+'。保留原状态，不视为已完成。';
    }
    catch(e) { error='刷新失败：'+String(e); } finally { refreshing=false; }
  }
  let availableUpdate=$state('');
  function manage(){return openManagement("general");}
  function manageSources(){return openManagement("sources");}
  function reviewSources(){return openManagement("sources",true);}
  function manageUpdates(){return openManagement("updates");}
  async function openManagement(section:string,reviewSources=false) {
    if (!isTauri()) { note='浏览器演示：真实设置请在托盘应用中打开'; return; }
    try { await invoke('open_management',{section,reviewSources}); } catch(e) { error=String(e); }
  }
  async function action(task: TaskSnapshot, name: string, value?: boolean) {
    if (!isTauri()) return;
    try {
      await invoke('update_task_attention',{id:task.id,action:name,pinned:value});
      await snapshots();
      if (name==='hide' || name==='ignore') { lastHidden={task,action:name==='hide'?'restore':'unignore'}; selected=null; }
      else if (!tasks.some(t=>t.id===selected?.id)) selected=null;
    } catch(e) { error=String(e); }
  }
  async function copy(task:TaskSnapshot) {
    try {await navigator.clipboard.writeText(`${task.name}\n${taskContext(task,settings,$t)}\n${$t('上报模型：')}${taskModel(task) ?? $t('来源未提供')}\n${task.project_path ?? ''}\n${task.session_id ?? task.id}`);note='已复制任务标题、项目、模型及会话 ID';}catch(e){error='复制失败：'+String(e);}
  }
  function resetFilters(){filterSource='';filterProject='';filterLocation='';filterSection=null;filterSignal=null;query='';unreadOnly=false;}
  function chooseSection(section:TaskSection){
    const next=overviewFilters(section,filterSection);
    showArchive=false;resetFilters();filterSection=next.section;
  }
  function showRunning(){showArchive=false;resetFilters();filterSection='ongoing';}
  function chooseSignal(signal:TaskSignal){const active=filterSignal===signal;showArchive=false;resetFilters();filterSignal=active?null:signal;}
  function selectTask(task:TaskSnapshot){
    resetFilters();showArchive=archived.some(t=>t.id===task.id);if(!showArchive && taskSection(task)==='uncertain')filterSection='uncertain';selected=task;
  }
  async function open(task: TaskSnapshot) {
    if (!isTauri()) return;
    try {
      const outcome=await invoke<string>('open_task',{id:task.id});
      if(outcome==='window_focused')note='已聚焦运行窗口；请在查看结果后标记已查看';
      else if(outcome==='terminal_target_requested')note='已请求返回终端，未确认窗口或会话';
      else if(outcome==='focus_unavailable')note='无法聚焦原窗口，已打开任务详情';
      else note='已定位任务详情；此来源暂不支持精确会话跳转';
    }
    catch(e) { error=String(e); }
  }
  // Measure natural card content after rendering, including status/filter chrome.
  $effect(()=>{
    if(!layoutReady)return;
    // Track layout inputs without serializing complete task payloads on every clock tick.
    void [sorted,view,showPrompts,selectedTask,showFilters,showArchive,error,note,$preferences,$t,layoutEpoch,now];
    const ticket=++layoutTicket;
    layoutQueue=tick().then(()=>{if(ticket===layoutTicket&&panel)sessionHeight=measureDashboard(panel,showPrompts||!!selectedTask);}).then(()=>tick());
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
      if(!showPrompts && event.key==='/' && !['INPUT','TEXTAREA','SELECT'].includes(target.tagName) && !target.isContentEditable){event.preventDefault();searchInput?.focus();}
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
          {id:'demo-done',remote:false,name:'演示：界面评审',source:'codex',status:'waiting_input',waiting_reason:'turn_ended',project:'demo-ui',model:'demo-model',reasoning_effort:'high',started_at:now-121000,finished_at:now-60000,updated_at:now-60000,confidence:'hook',unread:true},
        ];
      }
      return ()=>{clearInterval(clock);window.removeEventListener('keydown',searchShortcut);};
    }
    const cleanups:Array<()=>void>=[]; let mounted=true;
    const discover=async()=>{
      try {
        const found=await invoke<SourceDiscovery[]>('discover_sources');
        if(mounted)discovery=found;
      } catch { /* Candidate hints must not block normal monitoring. */ }
    };
    const discoveryTimer=setInterval(()=>void discover(),30000);
    async function subscribe(promise:Promise<()=>void>) { const cleanup=await promise; if(mounted)cleanups.push(cleanup);else cleanup(); }
    void (async()=>{
      try {
        await subscribe(listen<DashboardSnapshot>('dashboard-snapshot-updated',e=>{if(e.payload.revision>=dashboardRevision)applyDashboardSnapshot(e.payload);}));
        await subscribe(listen<TaskSnapshot[]>('archived-attention-updated',e=>archived=e.payload));
        await subscribe(listen<TaskSnapshot[]>('pending-prompts-updated',e=>pendingPrompts=e.payload));
        await subscribe(listen<MessageCenterState>('messages-updated',e=>{messagesEpoch++;messages=acceptMessageState(messages,e.payload);}));
        await subscribe(listen<DashboardSettings>('monitoring-settings-updated',e=>{settingsEpoch++;settings=e.payload;view=e.payload.default_view;void discover();}));
        await subscribe(listen<TaskSnapshot>('task-selected',e=>selectTask(e.payload)));
        await subscribe(listen<boolean>('pinned-changed',e=>pinned=e.payload));
        await subscribe(listen('popup-opened',()=>{layoutEpoch++;}));
        await subscribe(listen<{phase:string;version:string|null;can_install:boolean}>('application-update',e=>availableUpdate=e.payload.phase==='available'&&e.payload.can_install ? e.payload.version ?? '' : ''));
        void invoke<{phase:string;version:string|null;can_install:boolean}>('get_update_status').then(s=>availableUpdate=s.phase==='available'&&s.can_install?s.version ?? '':'');
        void invoke<SourceDefinition[]>('get_source_catalog').then(value=>{if(mounted)sourceCatalog=value;}).catch(()=>{});
        await snapshots(); view=settings?.default_view ?? 'tasks';
        void discover();
        error=await invoke<string | null>('get_monitoring_error') ?? '';
        const selection=await invoke<TaskSnapshot | null>('get_selected_task');
        if(selection)selectTask(selection);
        pinned=await invoke<boolean>('get_pinned');
        const win=getCurrentWindow();
        await subscribe(win.onResized(({payload})=>{if(payload.width!==lastWidth){lastWidth=payload.width;layoutEpoch++;}}));
        void document.fonts.ready.then(()=>{if(mounted)layoutEpoch++;});
        await subscribe(win.onFocusChanged(({payload:focused})=>{
          if(focused)void snapshots().catch(e=>error=String(e));
        }));
      } catch(e) { error='无法读取监控数据：'+String(e); }
      finally { layoutReady=true; await tick(); await layoutQueue; await resizeQueue; await emit('popup-ready',null).catch(()=>{}); }
    })();
    return ()=>{mounted=false;window.removeEventListener('keydown',searchShortcut);clearInterval(clock);clearInterval(discoveryTimer);cleanups.forEach(fn=>fn());};
  });
</script>

<div bind:this={panel} role="region" aria-label={$t("TaskPulse 任务监控")} class="dashboard tp-theme" style={`--tp-task-font-size:${$preferences.task_font_size??DEFAULT_TASK_FONT_SIZE}px`} data-continuity={JSON.stringify(dashboardContinuity(candidates,filtered))}>
  <header data-tauri-drag-region>
    <span class="brand" data-tauri-drag-region><BrandMark />TaskPulse</span>
    <div class="header-actions">
      <button class="tp-btn tp-icon" data-variant="ghost" onclick={()=>showPrompts=!showPrompts} aria-expanded={showPrompts} title={$t('消息中心 · {count} 条未读',{count:messages.unread})} aria-label={$t('消息中心：{count} 条未读',{count:messages.unread})}><Bell size={15}/>{#if messages.unread}<span>{messages.unread}</span>{/if}</button>
      <button class="tp-btn tp-icon" data-variant="ghost" onclick={manage} title={$t("设置")} aria-label={$t("设置")}><Settings size={15}/></button>
      <button class="tp-btn tp-icon" data-variant="ghost" onclick={refresh} disabled={refreshing || demo} title={$t("立即刷新")} aria-label={$t("立即刷新")}><RefreshCw size={15} class={refreshing?'animate-spin':''}/></button>
      <button class="tp-btn tp-icon" data-variant="ghost" disabled={demo} onclick={()=>invoke('toggle_pin')} title={pinned?$t('取消固定窗口'):$t('固定窗口')} aria-label={pinned?$t('取消固定窗口'):$t('固定窗口')}>{#if pinned}<Pin size={15}/>{:else}<PinOff size={15}/>{/if}</button>
    </div>
  </header>
  <div class="counts" aria-label={$t("任务概览")} aria-describedby="count-help">
    <button class="overview tp-tone" data-tone="running" class:active={filterSection==='ongoing'} aria-pressed={filterSection==='ongoing'} title={$t("来源新鲜的运行任务，包含明确待处理请求")} onclick={()=>chooseSection('ongoing')}><span>{$t("进行中")}</span><b>{overview.ongoing}</b></button>
    <button class="overview tp-tone" data-tone="action" class:active={filterSignal==='action'} aria-pressed={filterSignal==='action'} title={$t("进行中的子集：明确待批准、待回答或额度耗尽")} onclick={()=>chooseSignal('action')}><span>{$t("待处理")}</span><b>{overview.action}</b></button>
    <button class="overview tp-tone" data-tone="unread" class:active={filterSection==='results'} aria-pressed={filterSection==='results'} title={$t("尚未查看的本轮结果，包括失败")} onclick={()=>chooseSection('results')}><span>{$t("待查看")}</span><b>{overview.results}</b></button>
  </div>
  <p id="count-help" class="sr-only">{$t("待处理包含在进行中，计数不相加。")}</p>
  {#if candidates.some(t=>matchesSignal(t,'approval') || matchesSignal(t,'question') || matchesSignal(t,'failed'))}<div class="signals">
    {#if candidates.some(t=>matchesSignal(t,'approval'))}<button class="tp-chip tp-tone" data-tone="action" class:active={filterSignal==='approval'} aria-pressed={filterSignal==='approval'} onclick={()=>chooseSignal('approval')}>{$t("待批准")} {candidates.filter(t=>matchesSignal(t,'approval')).length}</button>{/if}
    {#if candidates.some(t=>matchesSignal(t,'question'))}<button class="tp-chip tp-tone" data-tone="action" class:active={filterSignal==='question'} aria-pressed={filterSignal==='question'} onclick={()=>chooseSignal('question')}>{$t("待回答")} {candidates.filter(t=>matchesSignal(t,'question')).length}</button>{/if}
    {#if candidates.some(t=>matchesSignal(t,'failed'))}<button class="tp-chip tp-tone" data-tone="danger" class:active={filterSignal==='failed'} aria-pressed={filterSignal==='failed'} onclick={()=>chooseSignal('failed')}>{$t("失败结果")} {candidates.filter(t=>matchesSignal(t,'failed')).length}</button>{/if}
  </div>{/if}
  <div class="view-bar">
    <label class="search"><Search size={15}/><input bind:this={searchInput} aria-label={$t("搜索任务")} placeholder={$t("搜索任务、项目、Agent 或模型   /")} bind:value={query} onkeydown={e=>{if(e.key==='Escape'){query='';e.stopPropagation();}}}/>{#if query}<button onclick={()=>query=''} aria-label={$t("清除搜索")}>×</button>{/if}</label>
    <button class="filter-toggle tp-btn tp-icon" class:active={!!filterText||showArchive||showFilters} aria-label={$t('筛选 / 分组')} title={$t('筛选 / 分组')} aria-expanded={showFilters} onclick={()=>showFilters=!showFilters}><SlidersHorizontal size={16}/></button>
    {#if showFilters}<div class="filters">
      <select bind:value={filterSource} aria-label={$t("筛选来源")}><option value="">{$t("全部来源")}</option>{#each enabledDashboardSources(settings) as s}<option value={s.id}>{s.name}</option>{/each}</select>
      <select bind:value={filterProject} aria-label={$t("筛选项目")}><option value="">{$t("全部项目")}</option>{#each projects as p}<option value={p.key}>{p.label}</option>{/each}</select>
      {#if locations.length>1}<select bind:value={filterLocation} aria-label={$t("筛选位置")}><option value="">{$t("全部位置")}</option>{#each locations as location}<option>{location}</option>{/each}</select>{/if}
      {#if uncertainTasks.length && !showArchive}<button class="view-toggle uncertain-filter" aria-pressed={filterSection==='uncertain'} onclick={()=>{chooseSection('uncertain');showFilters=false;}}>{$t('状态待更新')}</button>{/if}
      {#if archiveTasks.length || showArchive}<button class="view-toggle archive-filter" aria-pressed={showArchive} onclick={()=>{showArchive=!showArchive;resetFilters();showFilters=false;}}>{showArchive?$t('当前关注'):$t('较早关注')}</button>{/if}
      <button class="view-toggle group-toggle" onclick={()=>view=view==='tasks'?'projects':'tasks'} title={$t("切换任务/项目视图")}>{view==='tasks'?$t('按项目'):$t('按任务')}</button>
      <button class="unread-filter tp-chip tp-tone" data-tone="unread" class:active={unreadOnly} aria-pressed={unreadOnly} onclick={()=>unreadOnly=!unreadOnly} title={$t("筛选所有尚未查看的任务，含运行中任务；不同于上方的结果待查看计数")}>{$t("仅未查看")} {displayed.filter(t=>t.unread).length}</button>
      <button class="view-toggle all-tasks" disabled={!filterText&&!showArchive} onclick={()=>{showArchive=false;resetFilters();showFilters=false;}}>{$t("清除筛选")}</button>
    </div>{/if}
  </div>
  {#if demo}<p class="notice">{$t("浏览器演示 · 以下均为示例，不是真实采集结果")}</p>{/if}
  {#if showPrompts}<MessageCenter center={messages} prompts={pendingPrompts} {now} onRead={readMessages} onClose={()=>showPrompts=false} onSelect={id=>void selectMessage(id)} onReopen={entry=>void reopenRequest(entry.task_id,entry.request_id,entry.turn_key)} onReopenPrompt={task=>void reopenRequest(task.id,task.input_request_id,task.turn_key)} onPrompt={task=>{showPrompts=false;selectTask(task);}} />{/if}
  {#if error}<p class="notice error" role="alert">{error}</p>{/if}
  {#if note}<p class="notice" role="status">{$t(note)}<button class="dismiss" onclick={()=>note=''} aria-label={$t("关闭提示")}>×</button></p>{/if}
  {#if refreshResult}<p class="notice" role="status">{refreshResult}<button class="dismiss" onclick={()=>refreshSources=[]} aria-label={$t("关闭刷新结果")}>×</button></p>{/if}
  {#if availableUpdate}<p class="notice">TaskPulse {availableUpdate} {$t("可更新")}<button class="link" onclick={manageUpdates}>{$t("查看更新")}</button></p>{/if}
  {#if settings && !settings.onboarding_complete}<div class="onboarding"><strong>{$t("选择你要关注的 Agent")}</strong><p>{$t("确认本机来源，或添加远程服务器。新任务会自动出现。")}</p><button onclick={manageSources}>{$t("设置监控来源")}</button></div>{/if}
  {#if newSources.length}<p class="notice" data-source-review><span>{$t("新发现：{names}",{names:newSources.slice(0,2).map(row=>$t(row.definition.name)).join('、')})}{#if newSources.length>2} +{newSources.length-2}{/if}</span><button class="link" onclick={reviewSources}>{$t("选择添加")}</button></p>{/if}
  {#if refreshing}<p class="notice" role="status">{$t("刷新中：")}{progress.total-progress.pending.length} / {progress.total} {$t("个来源已返回")}{#if progress.failed.length} · {progress.failed.length} {$t("个失败")}{/if}</p>{/if}
  {#if lastHidden}<div class="notice">{lastHidden.action==='restore'?$t('已隐藏本轮'):$t('已停止监控此会话')}：{lastHidden.task.name}<button class="link" onclick={async()=>{const previous=lastHidden!;await action(previous.task,previous.action);lastHidden=null;}}>{$t("撤销")}</button></div>{/if}
  {#if outsideRunning.length}<p class="notice outside-running" role="status">{outsideRunning.length} {$t("个运行任务在当前视图之外")}<button class="link" onclick={showRunning}>{$t("查看全部运行任务")}</button></p>{/if}
  <main class="glass-scroll task-scroll" aria-label={$t("关注任务")}>
    {#if sorted.length===0}<div class="empty"><strong>{displayed.length?$t('当前筛选没有任务'):$t('当前没有需要关注的任务')}</strong><p>{displayed.length?$t('任务已采集，但不符合上方筛选条件。'):$t('已启用范围内的新执行会自动加入；旧项目再次执行也会回来。')}</p>{#if filterText}<button onclick={()=>{showArchive=false;resetFilters();}}>{$t("清除筛选，查看全部任务")}</button>{:else}<button onclick={manageSources}>{$t("查看来源与已发现会话")}</button>{/if}</div>
    {:else if view==='tasks'}
      <TaskList fitFour={sorted.length>=4} tasks={sorted} {settings} {now} selectedId={selected?.id ?? null} onSelect={t=>selected=t}/>
    {:else}{#each projectGroups as group}<section><h2>{group.title}</h2><TaskList fitFour={sorted.length>=4} tasks={group.tasks} {settings} {now} selectedId={selected?.id ?? null} onSelect={t=>selected=t}/></section>{/each}{/if}
  </main>
  {#if selectedTask}
    <TaskDetails task={selectedTask} {settings} {providers} {now} archived={showArchive}
      onClose={() => selected = null} onOpen={open} onCopy={copy} onAction={action}/>
  {/if}

</div>
<style>
  .dashboard{position:relative;height:100dvh;display:flex;flex-direction:column;overflow:hidden;border:1px solid var(--tp-border-strong);border-radius:var(--tp-radius-panel);background:var(--tp-panel)}
  header{display:flex;justify-content:space-between;align-items:center;padding:10px 12px 10px 16px;border-bottom:1px solid var(--tp-border);flex:none;min-height:52px}
  .brand{display:flex;align-items:center;gap:8px;font-size:14px;font-weight:650;color:var(--tp-text)}
  .header-actions{display:flex;gap:4px}.header-actions button{border:0;position:relative}.header-actions span{position:absolute;right:0;top:0;font-size:10px;min-width:14px;line-height:14px;text-align:center;border-radius:4px;background:var(--tp-card);color:var(--tp-action)}
  .counts{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:8px;padding:12px 12px 8px;flex:none}
  .overview{display:flex;flex-direction:column;align-items:flex-start;justify-content:space-between;text-align:start;padding:8px 12px;border:1px solid var(--tp-border);border-radius:8px;background:var(--tp-card);min-width:0}
  .overview span{display:block;font-size:12px;color:var(--tp-muted);line-height:1.35;overflow-wrap:anywhere}.overview b{display:block;font-size:24px;line-height:1.25;font-weight:600;letter-spacing:-.03em;color:var(--tone);margin-top:4px;font-variant-numeric:tabular-nums}
  .overview:hover{border-color:var(--tone-line);background:var(--tp-raised)}.overview.active{border-color:var(--tone-line);background:var(--tone-soft)}.overview.active span{color:var(--tp-text)}
  .signals{display:flex;gap:8px;flex-wrap:wrap;padding:0 12px 4px;max-height:68px;overflow:auto;flex:none}.signals button{font-size:11px;padding:2px 8px;border-radius:16px}.signals .active{box-shadow:inset 0 0 0 1px var(--tone)}
  .view-bar{position:relative;display:flex;align-items:center;gap:8px;padding:2px 12px 8px;flex:none}
  .view-bar .filter-toggle{flex:none;width:36px;height:36px;min-height:36px;color:var(--tp-muted);background:var(--tp-card);border-color:var(--tp-border-strong);border-radius:8px}
  .view-bar .filter-toggle.active{color:var(--tp-accent);background:var(--tp-accent-soft);border-color:var(--tp-accent-line)}
  .search{display:flex;gap:8px;align-items:center;min-height:36px;border:1px solid var(--tp-border-strong);background:var(--tp-input);border-radius:8px;padding:6px 9px;min-width:0;flex:1;color:var(--tp-muted)}
  .search:focus-within{border-color:var(--tp-accent-line)}.search input{width:100%;min-width:0;border:0;outline:none;background:transparent;color:var(--tp-text);font-size:13px;line-height:1.5;padding:0;border-radius:0}.search input:focus-visible{outline:none}.search input::placeholder{color:var(--tp-quiet)}.search>button{font-size:16px;width:24px;line-height:24px}
  .unread-filter{font-size:11px;white-space:nowrap;padding:4px 8px}.unread-filter.active{border-color:var(--tone-line);background:var(--tone-soft);color:var(--tone)}
  .task-scroll{min-height:80px;overflow-y:auto;scrollbar-gutter:stable;flex:1;padding:0 12px 8px;overscroll-behavior:contain}
  h2{font-size:13px;font-weight:600;color:var(--tp-text);padding:8px 0 0;margin:0;overflow-wrap:anywhere}
  .empty{display:flex;flex-direction:column;align-items:center;text-align:center;padding:16px 12px;color:var(--tp-text)}
  .empty strong{font-size:14px;font-weight:550}.empty p{font-size:12px;line-height:1.6;color:var(--tp-muted);margin:8px 0 12px}.empty button,.onboarding button{border:1px solid var(--tp-accent-line);padding:6px 12px;border-radius:8px;color:var(--tp-accent);background:var(--tp-accent-soft);font-size:12px}
  .notice{margin:0;padding:6px 12px;font-size:11px;line-height:1.6;background:var(--tp-accent-soft);color:var(--tp-accent);display:flex;align-items:center;gap:8px;max-height:60px;overflow:auto;flex:none;overflow-wrap:anywhere}.notice.error{color:var(--tp-danger);background:#f09ba70b}.dismiss{margin-left:auto;flex:none}.link{margin-left:auto;color:var(--tp-accent);white-space:nowrap;font-size:11px}
  .filters{position:absolute;z-index:3;left:12px;right:12px;top:calc(100% - 2px);display:flex;gap:8px;padding:12px;flex-wrap:wrap;max-height:42dvh;overflow:auto;background:var(--tp-raised);border:1px solid var(--tp-border-strong);border-radius:8px;box-shadow:var(--tp-shadow-window)}
  .filters select{font-size:12px;flex:1;padding:6px;min-width:110px;max-width:100%}.view-toggle{font-size:12px;border:1px solid var(--tp-border-strong);border-radius:6px;padding:4px 8px}.view-toggle[aria-pressed=true]{color:var(--tp-accent);background:var(--tp-accent-soft);border-color:var(--tp-accent-line)}
  .onboarding{padding:12px;border-bottom:1px solid var(--tp-border)}.onboarding strong{font-size:14px}.onboarding p{font-size:12px;color:var(--tp-muted);line-height:1.6;margin:8px 0}
  @media(max-width:380px){header{min-height:46px;padding:6px 10px 6px 12px}.counts{gap:6px;padding:8px 10px 6px}.overview{padding:6px 10px}.overview b{font-size:22px;margin-top:2px}.header-actions{gap:2px}}
  @media(max-width:350px){header{padding:8px}.brand{gap:4px}.header-actions{gap:0}.counts{gap:4px;padding:8px}.overview{padding:8px}.overview b{font-size:23px}.task-scroll{padding:0 8px 8px}.view-bar{gap:8px;padding-left:8px;padding-right:8px}.filters{left:8px;right:8px}}

  @media(max-height:500px){.dashboard:has(:global(.detail)) header{min-height:44px;padding-top:6px;padding-bottom:6px}.dashboard:has(:global(.detail)) .counts{padding-top:8px}.dashboard:has(:global(.detail)) .overview{padding-top:4px;padding-bottom:4px}.task-scroll{min-height:40px}}
</style>
