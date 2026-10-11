<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke, isTauri } from '@tauri-apps/api/core';
  import { emit, listen } from '@tauri-apps/api/event';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { LogicalSize } from '@tauri-apps/api/dpi';
  import { SOURCE_LABEL, CONFIDENCE_LABEL, formatAgo, type TaskSnapshot, type DashboardSettings, type ProviderHealth, type CodexAccountQuota, type SourceDiscovery } from '$lib/types';
  import { taskSection, taskLabel, taskActivity, taskContext, projectKey, sourceFreshness, refreshProgress, filterTasks, overviewFilters, runningOutsideView, filterDescription, SECTION_NAMES } from '$lib/dashboard';
  import TaskList from './TaskList.svelte';
  import Settings from '@lucide/svelte/icons/settings';
  import RefreshCw from '@lucide/svelte/icons/refresh-cw';
  import Pin from '@lucide/svelte/icons/pin';
  import PinOff from '@lucide/svelte/icons/pin-off';

  let tasks = $state<TaskSnapshot[]>([]);
  let archived = $state<TaskSnapshot[]>([]);
  let showArchive = $state(false);
  let showFilters = $state(false);
  let filterSection = $state<number | null>(null);
  let now = $state(Date.now());
  let providers = $state<ProviderHealth[]>([]);
  let settings = $state<DashboardSettings | null>(null);
  let quota = $state<CodexAccountQuota | null>(null);
  let selected = $state<TaskSnapshot | null>(null);
  let selectedTask = $derived([...tasks,...archived].find(t => t.id === selected?.id) ?? selected);
  let filterSource = $state('');
  let filterProject = $state('');
  let filterLocation = $state('');
  let view = $state<'tasks' | 'projects'>('tasks');
  let error = $state('');
  let note = $state('');
  let refreshing = $state(false);
  let pinned = $state(false);
  let overPopup = $state(false);
  let openingManagement = false;
  let newSources = $state(0);
  let panel = $state<HTMLDivElement | null>(null);
  let demo = $state(false);
  let lastHidden = $state<{task:TaskSnapshot;action:'restore'|'unignore'} | null>(null);
  let refreshSince = $state(0);
  const progress = $derived(refreshProgress(settings,providers,refreshSince));
  const maxHeight = typeof screen === 'undefined' ? 720 : Math.min(720, Math.floor(screen.availHeight * .8));
  const width = typeof screen === 'undefined' ? 440 : Math.min(440,screen.availWidth - 16);
  const sectionNames = SECTION_NAMES;
  const displayed = $derived(showArchive ? archived : tasks);
  const filters = $derived({source:filterSource,project:filterProject,location:filterLocation,section:filterSection});
  const filtered = $derived(filterTasks(displayed,filters,settings));
  const outsideRunning = $derived(runningOutsideView(tasks,filtered));
  const filterText = $derived(filterDescription(filters,displayed,settings));
  const sorted = $derived([...filtered].sort((a,b) => taskSection(a)-taskSection(b) || (b.attention_since ?? b.updated_at)-(a.attention_since ?? a.updated_at) || a.id.localeCompare(b.id)));
  const projects = $derived([...new Map(displayed.map(t => [projectKey(t,settings), { key:projectKey(t,settings), label:taskContext(t,settings), path:t.project_path }])).values()]);
  const locations = $derived([...new Set(displayed.map(t => t.host ?? '本机'))]);
  const projectGroups = $derived([...new Map(sorted.map(t => [projectKey(t,settings), { title:taskContext(t,settings), tasks:sorted.filter(x => projectKey(x,settings) === projectKey(t,settings)) }])).values()]);
  const degraded = $derived(providers.filter(p => p.status === 'degraded'));

  async function snapshots() {
    const [nextTasks,nextProviders,nextSettings,nextQuota,nextArchived] = await Promise.all([
      invoke<TaskSnapshot[]>('get_agents'), invoke<ProviderHealth[]>('get_providers'), invoke<DashboardSettings>('get_monitoring_settings'), invoke<CodexAccountQuota>('get_codex_quota'), invoke<TaskSnapshot[]>('get_archived_attention')
    ]);
    tasks=nextTasks; providers=nextProviders; settings=nextSettings; quota=nextQuota; archived=nextArchived;
  }
  async function refresh() {
    if (!isTauri() || refreshing) return;
    refreshing=true; error='';
    try {
      refreshSince=await invoke<number>('request_refresh'); note='';
      for(let i=0;i<100;i++) {
        await new Promise(r=>setTimeout(r,300)); await snapshots();
        const result=refreshProgress(settings,providers,refreshSince);
        if(!result.pending.length) {note=result.failed.length ? '刷新返回；采集失败：'+result.failed.map(s=>s.name).join('、') : '已收到全部启用来源的新采集结果';return;}
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
  function resetFilters(){filterSource='';filterProject='';filterLocation='';filterSection=null;}
  function chooseSection(section:number){
    const next=overviewFilters(section,filterSection);
    showArchive=false;resetFilters();filterSection=next.section;
  }
  function showRunning(){showArchive=false;resetFilters();filterSection=1;}
  function selectTask(task:TaskSnapshot){
    resetFilters();showArchive=archived.some(t=>t.id===task.id);selected=task;
  }
  async function open(task: TaskSnapshot) {
    if (!isTauri()) return;
    try { const outcome=await invoke<string>('open_task',{id:task.id}); note=outcome==='window_focused' ? '已聚焦运行窗口；请在查看结果后标记已查看' : '已定位任务详情；此来源暂不支持精确会话跳转'; }
    catch(e) { error=String(e); }
  }
  $effect(()=>{
    if (!panel || !isTauri()) return;
    let previous=0;
    const observer=new ResizeObserver(([entry])=>{
      const height=Math.min(maxHeight,Math.ceil(entry.borderBoxSize[0]?.blockSize ?? entry.contentRect.height));
      if (height!==previous) { previous=height; void getCurrentWindow().setSize(new LogicalSize(width,height)); }
    });
    observer.observe(panel); return ()=>observer.disconnect();
  });
  onMount(()=>{
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
      return ()=>clearInterval(clock);
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
        await subscribe(listen<CodexAccountQuota>('codex-quota-updated',e=>quota=e.payload));
        await subscribe(listen<DashboardSettings>('monitoring-settings-updated',e=>{settings=e.payload;view=e.payload.default_view;void discover();}));
        await subscribe(listen<TaskSnapshot>('task-selected',e=>selectTask(e.payload)));
        await subscribe(listen<boolean>('pinned-changed',e=>pinned=e.payload));
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
      finally { await emit('popup-ready',null).catch(()=>{}); }
    })();
    return ()=>{mounted=false;clearInterval(clock);clearInterval(discoveryTimer);cleanups.forEach(fn=>fn());};
  });
</script>

<div bind:this={panel} role="region" aria-label="TaskPulse 任务监控" class="glass-panel glass-noise dashboard" style:max-height={maxHeight+'px'} onpointerenter={()=>overPopup=true} onpointerleave={()=>overPopup=false}>
  <header data-tauri-drag-region>
    <span class="brand" data-tauri-drag-region>TaskPulse</span>
    <div class="header-actions">
      <button onclick={manage} title="管理监控来源" aria-label="管理监控来源"><Settings size={15}/></button>
      <button onclick={refresh} disabled={refreshing || demo} title="立即刷新" aria-label="立即刷新"><RefreshCw size={15} class={refreshing?'animate-spin':''}/></button>
      <button disabled={demo} onclick={()=>invoke('toggle_pin')} title={pinned?'取消固定窗口':'固定窗口'} aria-label={pinned?'取消固定窗口':'固定窗口'}>{#if pinned}<Pin size={15}/>{:else}<PinOff size={15}/>{/if}</button>
    </div>
  </header>
  <div class="counts"><button class:active={filterSection===0} onclick={()=>chooseSection(0)}>需处理 <b>{tasks.filter(t=>taskSection(t)===0).length}</b></button><button class:active={filterSection===1} onclick={()=>chooseSection(1)}>运行 <b>{tasks.filter(t=>taskSection(t)===1).length}</b></button><button class:active={filterSection===2} onclick={()=>chooseSection(2)}>待查看 <b>{tasks.filter(t=>taskSection(t)===2).length}</b></button><button class="total" onclick={()=>{showArchive=false;resetFilters();}}>全部 {tasks.length}</button></div>
  <div class="view-bar"><button class:active={!showArchive} onclick={()=>{showArchive=false;resetFilters();}}>{settings?.recent_days===0?'全部关注':'近期活动 · '+(settings?.recent_days ?? 7)+'天'}</button>{#if archived.length}<button class:active={showArchive} onclick={()=>{showArchive=!showArchive;resetFilters();}}>较早关注 {archived.length}</button>{/if}<button class="link" onclick={()=>showFilters=!showFilters}>{showFilters?'收起筛选':'筛选 / 分组'}</button></div>
  {#if demo}<p class="notice">浏览器演示 · 以下均为示例，不是真实采集结果</p>{/if}
  {#if error}<p class="notice error" role="alert">{error}</p>{/if}
  {#if note}<p class="notice" role="status">{note}<button class="dismiss" onclick={()=>note=''} aria-label="关闭提示">×</button></p>{/if}
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
  </div>
  {/if}
  {#if filterSource || filterProject || filterLocation || filterSection!==null}<div class="filter-note"><div><span>当前筛选 {filtered.length} / {displayed.length} 项</span><p>{filterText}</p></div><button class="link" onclick={resetFilters}>清除筛选</button></div>{/if}
  {#if outsideRunning.length}<p class="notice outside-running" role="status">{outsideRunning.length} 个运行任务在当前视图之外<button class="link" onclick={showRunning}>查看全部运行任务</button></p>{/if}
  <main class="glass-scroll task-scroll" aria-label="关注任务">
    {#if sorted.length===0}<div class="empty"><strong>{displayed.length?'当前筛选没有任务':'当前没有需要关注的任务'}</strong><p>{displayed.length?'任务已采集，但不符合上方筛选条件。':'已启用范围内的新执行会自动加入；旧项目再次执行也会回来。'}</p>{#if filterText}<button onclick={()=>{showArchive=false;resetFilters();}}>清除筛选，查看全部任务</button>{:else}<button onclick={manage}>查看来源与已发现会话</button>{/if}</div>
    {:else if view==='tasks'}
      {#each sectionNames as name,index}{@const group=sorted.filter(t=>taskSection(t)===index)}{#if group.length}<section><h2>{name}<span>{group.length}</span></h2><TaskList tasks={group} {settings} {providers} {now} selectedId={selected?.id ?? null} onSelect={t=>selected=t} onViewed={t=>void action(t,'viewed')}/></section>{/if}{/each}
    {:else}{#each projectGroups as group}<section><h2>{group.title}<span>{group.tasks.length}</span></h2><TaskList tasks={group.tasks} {settings} {providers} {now} selectedId={selected?.id ?? null} onSelect={t=>selected=t} onViewed={t=>void action(t,'viewed')}/></section>{/each}{/if}
  </main>
  {#if selectedTask}
    <aside class="detail" aria-label="任务详情">
      <div class="detail-heading"><strong title={selectedTask.name}>{selectedTask.name}</strong><span>{taskLabel(selectedTask)}</span><button onclick={()=>selected=null} aria-label="关闭详情">×</button></div>
      <p>{taskContext(selectedTask,settings)}</p>
      <p>状态依据：{CONFIDENCE_LABEL[selectedTask.confidence]}{#if selectedTask.model} · 上报模型：{selectedTask.model}{/if}</p>
      <p>{sourceFreshness(selectedTask,settings,providers,now)} · 最后活动 {formatAgo(selectedTask.updated_at,now)}</p>
      <div class="command glass-scroll"><span>{selectedTask.stale?'上次观测活动（非实时）':selectedTask.status==='running'?'当前活动':'最近消息'}</span><pre>{taskActivity(selectedTask)}</pre></div>
      <div class="detail-actions">
        {#if selectedTask.can_focus}<button onclick={()=>open(selectedTask!)}>{selectedTask.source==='claude_code'?'聚焦项目窗口':'聚焦运行窗口'}</button>{/if}
        <button onclick={()=>copy(selectedTask!)}>复制任务信息</button>
        {#if selectedTask.unread}<button onclick={()=>action(selectedTask!,'viewed')}>已查看</button>{/if}
        {#if showArchive}<button onclick={()=>action(selectedTask!,'follow')}>重新加入</button>{/if}
        <button onclick={()=>action(selectedTask!,'pin',showArchive ? true : !selectedTask!.pinned)}>{showArchive?'固定到近期面板':selectedTask.pinned?'取消固定':'固定'}</button>
        <details class="more"><summary>更多</summary>{#if selectedTask.status!=='running'}<button onclick={()=>action(selectedTask!,'hide')}>隐藏本轮（新执行会恢复）</button>{/if}<button onclick={()=>action(selectedTask!,'ignore')}>停止监控此会话（需手动恢复）</button></details>
      </div>
    </aside>
  {/if}
  <footer><button onclick={manage}>{settings?.sources.filter(s=>s.enabled).length ?? 0} 个来源{degraded.length ? ' · '+degraded.length+' 个待检查' : ''}</button><span>按来源显示采集时间</span></footer>
  {#if !demo}<details class="quota"><summary>本地 Codex 账号额度<span>{quota?.refreshing?'更新中…':quota?.quota ? (quota.stale?'数据待更新':'已更新'):'暂无数据'}</span></summary><p>{#if quota?.quota}5h 剩余 {Math.round(100-quota.quota.used_5h_pct)}% · 7天剩余 {Math.round(100-quota.quota.used_1w_pct)}%{#if quota.quota.resets_5h_at}<br/>5h 重置：{new Date(quota.quota.resets_5h_at*1000).toLocaleString()}{/if}{#if quota.quota.resets_1w_at}<br/>7天重置：{new Date(quota.quota.resets_1w_at*1000).toLocaleString()}{/if}{#if quota.observed_at}<br/>读取于 {formatAgo(quota.observed_at,now)}{/if}{:else}尚未获得本机账号额度{/if}{#if quota?.error}<br/>{quota.error}{/if}<br/>不代表远程服务器账号额度。</p></details>{/if}
</div>
<style>
  .filter-note p{margin:4px 0 0;overflow-wrap:anywhere;font-size:10px;line-height:1.5}.filter-note{align-items:flex-start;gap:10px}.outside-running{flex:none}
  .counts button{padding:0;font-size:12px}.counts .active,.view-bar .active{color:#a9d8f4}.view-bar{display:flex;align-items:center;gap:12px;padding:10px 16px}.view-bar button{font-size:11px}.detail-actions button,.detail-actions summary{font-size:12px}.more{font-size:12px;color:#aab8bd}.more summary{cursor:pointer}.more button{display:block;padding:8px 0}h2{font-size:11px}.detail p{line-height:1.6}
  .dashboard{position:relative;border-radius:14px;display:flex;flex-direction:column;color:#deded8;overflow:hidden;border:1px solid #ffffff0b}header{display:flex;justify-content:space-between;align-items:center;padding:14px 16px;border-bottom:1px solid #ffffff0b}.brand{font-size:12px;letter-spacing:.14em;font-weight:650}.header-actions{display:flex;gap:12px}button{cursor:pointer;background:transparent;border:0;color:#b2b5b4;font-size:11px}button:hover{color:#eef2f0}button:disabled{opacity:.45;cursor:default}.counts{display:flex;gap:19px;padding:12px 16px;font-size:11px;border-bottom:1px solid #ffffff09;color:#adbac3}.counts b{margin-left:4px;color:#e7e5dc;font-weight:550}.total{margin-left:auto;color:#898b85}.filters{display:flex;gap:6px;padding:11px 14px}select{font-size:11px;background:#ffffff07;color:#c3c7c5;border:1px solid #ffffff0d;border-radius:5px;padding:5px;min-width:0;max-width:145px;flex:1}option{background:#25272a}.view-toggle{white-space:nowrap;font-size:10px;flex:none}.task-scroll{min-height:0;overflow-y:auto;flex:1;padding:0 5px 6px}h2{font-size:11px;letter-spacing:.06em;font-weight:550;color:#8f999b;padding:10px 12px 3px;display:flex;gap:8px}h2 span{color:#707778}.empty{text-align:center;padding:24px 24px 30px;color:#b8bab3}.empty strong{font-size:12px;font-weight:500}.empty p{font-size:11px;line-height:1.8;color:#898e88;margin:10px 0}.empty button,.onboarding button{border:1px solid #8eb3c530;padding:7px 10px;border-radius:6px;color:#a1c6dd}.notice{padding:8px 15px;font-size:11px;line-height:1.6;background:#78aaca0c;color:#a4c3d7;display:flex;align-items:center;gap:8px}.notice.error{color:#e7a6a0;background:#dc7d710d}.dismiss{margin-left:auto}.link{margin-left:auto;color:#9bc5e1;white-space:nowrap}.filter-note{padding:0 16px 8px;font-size:10px;color:#8a9ea9;display:flex}.detail{padding:11px 16px;border-top:1px solid #ffffff0d;background:#80b8df06;flex:none}.detail-heading{display:flex;gap:9px;align-items:center}.detail-heading strong{flex:1;min-width:0;white-space:nowrap;text-overflow:ellipsis;overflow:hidden;font-size:12px;font-weight:550}.detail-heading span{font-size:10px;color:#a4b3b4}.detail p{font-size:10px;color:#898f91;margin:5px 0}.command{max-height:86px;overflow:auto;margin:9px 0}.command span{font-size:9px;color:#7c8c94}.command pre{white-space:pre-wrap;overflow-wrap:anywhere;font-family:inherit;font-size:11px;line-height:1.7;color:#b8c0c3;margin:4px 0}.detail-actions{display:flex;gap:12px;flex-wrap:wrap}.detail-actions button{font-size:12px}footer{display:flex;gap:8px;justify-content:space-between;border-top:1px solid #ffffff09;padding:10px 16px;font-size:9px;color:#7f8a8a}footer button{font-size:10px}.quota{font-size:10px;padding:9px 16px;border-top:1px solid #ffffff09;color:#909b9b;flex:none}.quota summary{cursor:pointer;display:flex;justify-content:space-between}.quota p{font-size:10px;line-height:1.7;margin:7px 0 2px;overflow-wrap:anywhere}.onboarding{padding:15px 16px;border-bottom:1px solid #ffffff09}.onboarding strong{font-size:13px}.onboarding p{font-size:11px;color:#98a5a6;line-height:1.7;margin:8px 0}
</style>
