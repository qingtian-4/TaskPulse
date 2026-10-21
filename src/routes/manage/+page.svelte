<script lang="ts">
  import { onMount } from 'svelte';
  import { Cable, Radio, Bell, Info } from '@lucide/svelte';
  import IntegrationEditor from '$lib/components/IntegrationEditor.svelte';
  import IntegrationHookCleanup from '$lib/components/IntegrationHookCleanup.svelte';
  import { invoke, isTauri } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { taskContext, taskLabel } from '$lib/dashboard';
  import { sourceReadState, sourceReadTone, sourceSetupStep, readTestResult, sourceCapability } from '$lib/source-setup';
  import { deliveryStage } from '$lib/notice';
  import type { DeliveryRecord } from '$lib/types';
  import { formatAgo, type DashboardSettings, type MonitorSource, type SourceDiscovery, type SourceKind, type SessionPage, type ProviderHealth } from '$lib/types';
  let settings=$state<DashboardSettings | null>(null);
  let discovery=$state<SourceDiscovery[]>([]);
  let health=$state<ProviderHealth[]>([]);
  let tab=$state<'sources'|'sessions'|'notifications'>('sources');
  let selected=$state('');
  let query=$state('');
  let sessionSource=$state('');
  let offset=$state(0);
  let sessions=$state<SessionPage>({items:[],total:0});
  let error=$state('');
  let notice=$state('');
  let saving=$state(false);
  let loadingSessions=$state(false);
  let notificationError=$state('');
  let deliveryRecords=$state<DeliveryRecord[]>([]);
  let deliveryError=$state('');
  async function loadDelivery(){try{deliveryRecords=await invoke<DeliveryRecord[]>('get_notification_log');deliveryError='';}catch(e){deliveryError=String(e);}}
  let addKind=$state<SourceKind>('codex');
  let eventPrompt=$state<{sourceId:string;enabled:boolean} | null>(null);
  let dirty=$state(false);
  let applied=$state<DashboardSettings | null>(null);
  let conflict=$state(false);
  let closePrompt=$state(false);
  let closing=false;
  let now=$state(Date.now());
  let reading=$state(false), readTest=$state<{id:string;message:string;passed?:boolean}|null>(null);
  let notificationTest=$state(false);
  const testStage=$derived(deliveryRecords.some(r=>r.test && r.stage==='rendered')?'已有测试渲染记录（非已读）':notificationTest?'已提交 · 请确认是否看到':'本窗口暂无测试记录');
  async function testRead(){
    if(!editor || dirty || reading)return;
    const id=editor.id;reading=true;readTest=null;error='';
    try{
      const since=await invoke<number>('request_refresh');
      for(let n=0;n<100;n++){
        await new Promise(r=>setTimeout(r,300));
        health=await invoke<ProviderHealth[]>('get_providers');
        const result=readTestResult(id,health,since);
        if(result!=='pending'){readTest={id,passed:result==='passed',message:result==='passed'?'本次读取正常 · 不等于真实生命周期已验收':'本次读取失败 · 查看高级诊断'};return;}
      }
      readTest={id,message:'读取超时 · 保留原状态，不视为任务结束'};
    }catch(e){error=String(e);}finally{reading=false;}
  }
  let editor=$derived(settings?.sources.find(s=>s.id===selected) ?? null);
  const setupStep=$derived(sourceSetupStep(!!editor,dirty || !applied?.onboarding_complete,readTest?.id===editor?.id && readTest?.passed===true));
  const kinds:Array<{kind:SourceKind;name:string}>=[{kind:'generic_event',name:'其他 Agent · 通用事件'},{kind:'codex',name:'Codex · 本机'},{kind:'codex_wsl',name:'Codex · WSL'},{kind:'claude_code',name:'Claude Code · 本机'},{kind:'hermes',name:'Hermes · 本机'},{kind:'deepseek_desktop',name:'DeepSeek · Windows'},{kind:'deepseek_wsl',name:'DeepSeek · WSL'},{kind:'remote_codex',name:'Codex · 远程服务器'},{kind:'tmux',name:'tmux · 远程服务器'},{kind:'hooks',name:'通用 Hooks'},{kind:'scanner',name:'进程扫描'}];
  const candidates=$derived(discovery.filter(d=>d.available && d.candidate && !settings?.sources.some(s=>s.id===d.candidate!.id || s.kind===d.candidate!.kind && s.location.toLowerCase()===d.candidate!.location.toLowerCase())));
  function addCandidate(candidate:MonitorSource) {
    if(!settings)return;
    settings.sources.push({...candidate,enabled:true});selected=candidate.id;dirty=true;notice='已添加 '+candidate.name+'；保存后开始关注新执行。';
  }
  async function confirmEvents(){
    if(!eventPrompt)return;
    const choice=eventPrompt;eventPrompt=null;saving=true;error='';
    try{notice=await invoke<string>('configure_claude_events',{sourceId:choice.sourceId,enabled:choice.enabled,confirmed:true});}catch(e){error=String(e);}finally{saving=false;}
  }
  function list(value:string):string[]{return value.split('\n').map(s=>s.trim()).filter(Boolean);}
  function changed(){dirty=true;readTest=null;}
  function add(){
    if(!settings)return;
    const builtin:Partial<Record<SourceKind,string>>={codex:'codex',claude_code:'claude_code',hermes:'hermes',deepseek_desktop:'deepseek_harness_desktop',deepseek_wsl:'deepseek_harness',hooks:'local_file',scanner:'scanner'};
    const candidate=builtin[addKind];
    if(['codex','hooks','scanner'].includes(addKind) && settings.sources.some(s=>s.kind===addKind)){selected=settings.sources.find(s=>s.kind===addKind)!.id;notice='本机来源已存在，请在左侧启用或编辑。';return;}
    const id=candidate && !settings.sources.some(s=>s.id===candidate) ? candidate : addKind+':'+crypto.randomUUID();
    const source:MonitorSource={id,name:kinds.find(k=>k.kind===addKind)?.name ?? addKind,kind:addKind,enabled:true,location:['deepseek_wsl','codex_wsl'].includes(addKind)?'Ubuntu':'',projects:['*'],excluded_projects:[],tmux_sessions:[],tmux_panes:[],poll_interval_ms:3000};
    if(addKind==='generic_event')source.integration={agent_id:'custom-agent',adapter_id:'standard-events',transport:'local',accept_since_ms:0};
    settings.sources.push(source);selected=id;dirty=true;
  }
  function remove(id:string){if(!settings)return;settings.sources=settings.sources.filter(s=>s.id!==id);selected=settings.sources[0]?.id ?? '';dirty=true;}
  async function save(){
    if(!settings || !isTauri() || conflict)return;
    saving=true;error='';notice='';
    try { settings.onboarding_complete=true;settings=await invoke<DashboardSettings>('save_monitoring_settings',{settings:$state.snapshot(settings)});applied=structuredClone($state.snapshot(settings));discovery=await invoke<SourceDiscovery[]>('discover_sources');dirty=false;notice='已保存并即时生效。受影响来源正在重新采集。'; }
    catch(e){error=String(e);}finally{saving=false;}
  }
  async function toggleSource(source:MonitorSource,enabled:boolean){
    if(!settings || saving || conflict)return;
    // First setup and newly added sources remain a clearly labelled draft.
    if(!applied?.onboarding_complete || !applied.sources.some(s=>s.id===source.id)){source.enabled=enabled;changed();return;}
    const previous=source.enabled;
    saving=true;error='';
    try {
      const current=await invoke<DashboardSettings>('get_monitoring_settings');
      const target=current.sources.find(s=>s.id===source.id);
      if(!target)throw Error('来源已被移除，请重新打开配置');
      target.enabled=enabled;
      const next=await invoke<DashboardSettings>('save_monitoring_settings',{settings:current});
      applied=structuredClone(next);source.enabled=enabled;
      notice=(enabled?'已启用：':'已停用：')+source.name+'。开关即时生效；其他编辑仍需保存。';
    }catch(e){source.enabled=previous;error=String(e);}finally{saving=false;}
  }
  function sourceState(source:MonitorSource){return sourceReadState(source,applied,health,now);}
  async function saveAndClose(){await save();if(!dirty && !error){closing=true;await getCurrentWindow().close();}}
  async function discardAndClose(){closing=true;await getCurrentWindow().close();}
  async function reloadSettings(){
    try{settings=await invoke<DashboardSettings>('get_monitoring_settings');applied=structuredClone($state.snapshot(settings));dirty=false;conflict=false;error='';notice='已放弃未保存编辑，并加载最新配置';}catch(e){error=String(e);}
  }
  function focusTrap(dialog:HTMLDialogElement){
    const previous=document.activeElement as HTMLElement | null;
    dialog.querySelector<HTMLElement>('button')?.focus();
    function keydown(e:KeyboardEvent){
      if(e.key==='Escape'){e.preventDefault();closePrompt=false;eventPrompt=null;}
      if(e.key!=='Tab')return;
      const buttons=[...dialog.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')];
      const first=buttons[0],last=buttons.at(-1);
      if(e.shiftKey && document.activeElement===first){e.preventDefault();last?.focus();}
      else if(!e.shiftKey && document.activeElement===last){e.preventDefault();first?.focus();}
    }
    dialog.addEventListener('keydown',keydown);
    return {destroy(){dialog.removeEventListener('keydown',keydown);if(previous?.isConnected)previous.focus();}};
  }
  let request=0;
  async function loadSessions(){
    if(!isTauri())return;
    const current=++request;loadingSessions=true;
    try {const result=await invoke<SessionPage>('list_discovered_sessions',{source:sessionSource || null,query,offset,limit:30});if(current===request)sessions=result;}
    catch(e){error=String(e);}finally{if(current===request)loadingSessions=false;}
  }
  async function sessionAction(id:string,action:string){
    try {await invoke('update_task_attention',{id,action});notice=action==='follow'?'已加入关注，不补发历史通知':'会话设置已更新';await loadSessions();}
    catch(e){error=String(e);}
  }
  async function testNotification(){try{await invoke('test_notification');notificationTest=true;await loadDelivery();notice='测试已提交；请确认是否看见提醒。投递成功不代表真实任务生命周期已验收。';}catch(e){error=String(e);}}
  onMount(()=>{
    if(!isTauri()){error='管理配置需要在 TaskPulse 托盘应用中打开。浏览器页面不会写入真实配置。';return;}
    const cleanups:Array<()=>void>=[];let mounted=true;
    void(async()=>{
      try {
        settings=await invoke<DashboardSettings>('get_monitoring_settings');
        applied=structuredClone($state.snapshot(settings));
        error=await invoke<string | null>('get_monitoring_error') ?? '';
        discovery=await invoke<SourceDiscovery[]>('discover_sources');
        health=await invoke<ProviderHealth[]>('get_providers');
        notificationError=await invoke<string | null>('get_notification_error') ?? '';
        if(!settings.onboarding_complete){for(const source of settings.sources){source.enabled=discovery.find(d=>d.id===source.id)?.available ?? false;}dirty=true;}
        selected=settings.sources[0]?.id ?? '';
        const cleanup=await listen<ProviderHealth[]>('providers-updated',e=>health=e.payload);if(mounted)cleanups.push(cleanup);else cleanup();
        const notificationCleanup=await listen<string>('notification-error',e=>notificationError=e.payload);if(mounted)cleanups.push(notificationCleanup);else notificationCleanup();
        const settingsCleanup=await listen<DashboardSettings>('monitoring-settings-updated',e=>{
          if(saving)return;
          if(dirty){conflict=true;error='监控配置已在其他窗口更新，暂不允许覆盖。请加载最新配置后编辑。';return;}
          settings=structuredClone(e.payload);applied=structuredClone(e.payload);
        });if(mounted)cleanups.push(settingsCleanup);else settingsCleanup();
        const closeCleanup=await getCurrentWindow().onCloseRequested(e=>{if(dirty && !closing){e.preventDefault();closePrompt=true;}});if(mounted)cleanups.push(closeCleanup);else closeCleanup();
      } catch(e){error=String(e);}
    })();
    const interval=setInterval(()=>{if(tab==='sessions')void loadSessions();if(notificationTest)void loadDelivery();},3000);
    const discoveryTimer=setInterval(()=>{if(tab==='sources')void invoke<SourceDiscovery[]>('discover_sources').then(value=>discovery=value).catch(e=>error=String(e));},30000);
    const clock=setInterval(()=>now=Date.now(),1000);
    return()=>{mounted=false;clearInterval(interval);clearInterval(discoveryTimer);clearInterval(clock);cleanups.forEach(fn=>fn());};
  });
</script>

<div class="manage tp-theme">
  <header><div class="page-title"><span class="page-icon" aria-hidden="true"><Cable size={21}/></span><div><span class="eyebrow">TASKPULSE / SOURCES</span><h1>管理你的关注范围</h1><p>选择任务来源，交给 TaskPulse 跟踪新执行。</p></div></div><button class="primary tp-btn" data-variant="primary" disabled={saving || !dirty || !settings || conflict} onclick={save}>{saving?'保存中…':dirty?'保存并生效':'配置已保存'}</button></header>
  <nav aria-label="管理页面"><button class="tp-btn" class:active={tab==='sources'} onclick={()=>tab='sources'}>监控来源</button><button class="tp-btn" class:active={tab==='sessions'} onclick={()=>{tab='sessions';void loadSessions();}}>已发现会话</button><button class="tp-btn" class:active={tab==='notifications'} onclick={()=>{tab='notifications';void loadDelivery();}}>通知与显示</button></nav>
  {#if error}<div class="message error" role="alert">{error}<button class="tp-btn" onclick={()=>error=''} aria-label="关闭错误">×</button></div>{/if}
  {#if conflict}<div class="message"><button class="tp-btn" onclick={reloadSettings}>放弃未保存编辑，加载最新配置</button></div>{/if}
  {#if notice}<div class="message" role="status">{notice}<button class="tp-btn" onclick={()=>notice=''} aria-label="关闭提示">×</button></div>{/if}
  {#if settings && !settings.onboarding_complete}<div class="welcome">首次设置：已预选发现的本机来源。确认需要的来源后，点击“保存并生效”。远程服务器由你主动添加。</div>{/if}
  <main class="glass-scroll">
    {#if settings && tab==='sources'}
      <ol class="setup-flow" aria-label="来源接入步骤">{#each ['选择 Agent','能力与配置范围','保存并测试','等待真实任务事件'] as label,i}<li class:current={setupStep===i+1} aria-current={setupStep===i+1?'step':undefined}><span class="step-number">{i+1}</span><span class="step-label">{label}</span></li>{/each}</ol>
      <div class="source-layout"><aside aria-label="监控来源列表"><h2 class="sidebar-title">已保存与草稿来源 <span class="tp-chip">{settings.sources.length}</span></h2><div class="add"><select bind:value={addKind} aria-label="新增来源类型">{#each kinds as kind}<option value={kind.kind}>{kind.name}</option>{/each}</select><button class="tp-btn" onclick={add}>＋ 添加</button></div>
      {#each settings.sources as source (source.id)}
        <div class="source-item" class:chosen={selected===source.id}>
          <input type="checkbox" checked={source.enabled} disabled={saving} onchange={e=>void toggleSource(source,e.currentTarget.checked)} aria-label={'启用 '+source.name}/>
          <button class="source-select" aria-pressed={selected===source.id} onclick={()=>selected=source.id}><strong>{source.name}</strong><span class="source-reading tp-chip tp-tone" data-tone={sourceReadTone(source,applied,health,now)}>{sourceState(source)}</span>{#if applied?.sources.find(s=>s.id===source.id)?.enabled && health.find(h=>h.provider===source.id)?.last_success}<small>最近读取 {formatAgo(health.find(h=>h.provider===source.id)!.last_success!,now)}</small>{/if}</button>
        </div>
      {/each}
      <details class="discovered tp-fold"><summary>已发现 Agent · {candidates.length} 个待选择</summary><p class="help">发现安装痕迹不等于已启用或完整支持；添加后查看能力并保存。</p>{#each candidates as candidate}<div class="candidate"><strong>{candidate.candidate!.name}</strong><button class="tp-btn" onclick={()=>addCandidate(candidate.candidate!)}>添加到监控来源</button></div>{:else}<p class="help">没有新的自动发现项。可在上方手动选择来源类型。</p>{/each}</details>
      </aside><section class="editor">
        {#if editor}
          <h2>{editor.name}</h2>
          <div class="setup-status" aria-label="来源接入状态"><span class="tp-chip tp-tone" data-tone={sourceReadTone(editor,applied,health,now)}>{sourceState(editor)}</span><span class="tp-chip tp-tone" data-tone={notificationTest?'brand':'neutral'}>提醒通道（全局）：{testStage}</span><span class="tp-chip tp-tone" data-tone="stale">真实生命周期：需按宿主版本单独验收</span></div>
          <div class="capability-note tp-surface"><Info size={15} aria-hidden="true"/><p>{sourceCapability(editor.kind)}</p></div>
          <div class="source-tests"><button class="secondary tp-btn" title="只检查所选来源本次读取，不验收 Agent 结束事件" disabled={dirty || saving || reading || !editor.enabled} onclick={testRead}><Radio size={14}/>{reading?'正在读取…':'测试读取来源'}</button><button class="secondary tp-btn" title="只测试全局通知通道，不创建任务" disabled={dirty || saving} onclick={testNotification}><Bell size={14}/>发送测试提醒</button></div>
          {#if readTest?.id===editor.id}<p class="read-result" role="status">{readTest.message}</p>{/if}
          {#if editor.kind==='generic_event'}{#key editor.id}<IntegrationEditor source={editor} {dirty} changed={changed}/>{/key}{/if}
          <label>显示名称<input aria-label="来源显示名称" bind:value={editor.name} oninput={changed}/></label>
          {#if editor.kind==='remote_codex' || editor.kind==='tmux'}<label>SSH 别名<input bind:value={editor.location} oninput={changed} placeholder="例如 dev-server"/></label><p class="help">使用系统 SSH 配置和已有登录方式，不在 TaskPulse 保存密码。</p><label>轮询周期（毫秒）<input type="number" min="500" max="60000" bind:value={editor.poll_interval_ms} oninput={changed}/></label>
          {:else if editor.kind==='claude_code'}<label>Claude 数据目录<input bind:value={editor.location} oninput={changed} placeholder="留空：使用 CLAUDE_CONFIG_DIR 或本机 .claude"/></label><p class="help">自动观测 VS Code / CLI 原生会话、标题与上报模型；未启用精确事件时，内部格式的状态显示为“推断”。</p>
          {:else if editor.kind==='deepseek_desktop'}<label>桌面版数据目录<input bind:value={editor.location} oninput={changed} placeholder="留空：使用 DSH_HOME 或本机 .dsh"/></label>
          {:else if editor.kind==='deepseek_wsl' || editor.kind==='codex_wsl'}<label>WSL 发行版<input bind:value={editor.location} oninput={changed} placeholder="例如 Ubuntu"/></label>{#if editor.kind==='codex_wsl'}<p class="help">本机 WSL 的 Codex 独立于 Windows 和 SSH 来源。只读当前发行版默认用户的 CODEX_HOME / .codex；不导入空闲历史，不自动唤醒已停止的发行版。</p>{/if}
          {:else if editor.kind==='hermes'}<label>状态数据库<input bind:value={editor.location} oninput={changed} placeholder="留空：自动发现 Hermes state.db"/></label>{/if}
          <h3>项目范围</h3>
          <label class="radio"><input type="radio" name="scope" checked={editor.projects.includes('*')} onchange={()=>{editor!.projects=['*'];changed();}}/>自动关注所有项目的新任务</label>
          <label class="radio"><input type="radio" name="scope" checked={!editor.projects.includes('*')} onchange={()=>{editor!.projects=[];changed();}}/>只关注指定项目</label>
          {#if !editor.projects.includes('*')}<label>项目名称或完整路径（每行一个）<textarea value={editor.projects.join('\n')} oninput={e=>{editor!.projects=list(e.currentTarget.value);changed();}} placeholder="agenttray&#10;D:\Project\my-project&#10;@unassociated"></textarea></label>{/if}
          <p class="help">推荐自动关注所有项目的新任务：只要来源启用，旧项目再次执行也会自动回来。主面板按最近 {settings.recent_days ?? 7} 天的执行活动显示，不复制历史会话。指定项目限制不会自动改写，可在上方取消。排除规则优先。</p>
          {#if !editor.projects.includes('*')}<label class="radio"><input type="checkbox" checked={editor.projects.includes('@unassociated')} onchange={e=>{editor!.projects=e.currentTarget.checked ? [...editor!.projects,'@unassociated'] : editor!.projects.filter(p=>p!=='@unassociated');changed();}}/>也包括未关联项目的会话</label>{/if}
          {#if editor.kind==='tmux'}<label>tmux session（留空表示全部，每行一个）<textarea value={editor.tmux_sessions.join('\n')} oninput={e=>{editor!.tmux_sessions=list(e.currentTarget.value);changed();}}></textarea></label><label>tmux pane（可选，每行一个）<textarea value={editor.tmux_panes.join('\n')} oninput={e=>{editor!.tmux_panes=list(e.currentTarget.value);changed();}}></textarea></label>{/if}
          <p class="help">保存后，范围内的新执行会自动加入。测试只验证读取 / 提醒通道；请在 Agent 中启动一轮真实任务，核对开始、阶段、结束和提醒。没有真实宿主证据时，仍以能力矩阵的验证等级为准。</p>
          <details class="advanced tp-fold"><summary>高级设置 · 排除、诊断与来源移除</summary>
          <label>排除项目（可选，每行一个）<textarea value={editor.excluded_projects.join('\n')} oninput={e=>{editor!.excluded_projects=list(e.currentTarget.value);changed();}}></textarea></label>
          {#if discovery.find(d=>d.id===editor!.id)}<p class="path">{discovery.find(d=>d.id===editor!.id)?.detail}</p>{/if}
          {#if health.find(h=>h.provider===editor!.id)?.message}<p class="message error">{health.find(h=>h.provider===editor!.id)?.message}</p>{/if}
          {#if editor.kind==='claude_code'}<h3>自愿接入精确事件</h3><p class="help">不会覆盖 statusLine 或其他 Hook；需逐次明确确认，不自动修改审批策略。</p><button class="tp-btn" disabled={dirty || saving} onclick={()=>eventPrompt={sourceId:editor!.id,enabled:true}}>启用精确事件…</button><button class="tp-btn" disabled={dirty || saving} onclick={()=>eventPrompt={sourceId:editor!.id,enabled:false}}>移除精确事件…</button>{/if}
          <p class="path">来源 ID：{editor.id} · 类型：{editor.kind}</p>
          <button class="remove tp-btn" data-variant="danger" onclick={()=>remove(editor!.id)}>移除此监控来源</button><p class="help">移除只影响 TaskPulse，关注记录保留；宿主 Hook 不会自动删除。保存后仍可在下方“宿主 Hook 登记”预览并确认撤销。</p></details>
        {:else}<div class="setup-empty tp-surface"><h2>先选择你使用的 Agent</h2><p>添加已发现的安装，或手动选择类型。无需预先挑选每个对话：来源启用后，范围内的新执行会自动进入关注列表。</p></div>{/if}
      </section></div>
    {:else if settings && tab==='sessions'}
      <div class="session-filters"><select bind:value={sessionSource} onchange={()=>{offset=0;void loadSessions();}} aria-label="会话来源"><option value="">全部来源</option>{#each settings.sources as source}<option value={source.id}>{source.name}</option>{/each}</select><input bind:value={query} placeholder="搜索任务标题、项目或路径" aria-label="搜索会话" oninput={()=>{offset=0;void loadSessions();}}/><button class="tp-btn" onclick={()=>loadSessions()} disabled={loadingSessions}>刷新目录</button></div>
      <p class="help">显示采集器已发现的会话，不是原生历史的完整镜像。尚未启用的来源不会持续采集。目录不等于关注列表。</p>
      {#each sessions.items as item (item.task.id)}<article class="session"><div><strong>{item.task.name}</strong><p>{taskContext(item.task,settings)}</p><small>{taskLabel(item.task)} · {item.reason} · 最后活动 {formatAgo(item.task.updated_at)}</small></div><div class="session-actions">{#if item.reason==='已设为不再监控'}<button class="tp-btn" onclick={()=>sessionAction(item.task.id,'unignore')}>恢复监控</button>{:else if item.reason==='正在关注'}<span>已关注</span>{:else}<button class="tp-btn" disabled={item.reason==='来源未启用' || item.reason==='来源已移除' || item.reason==='项目不在监控范围' || item.reason==='tmux 不在监控范围'} onclick={()=>sessionAction(item.task.id,'follow')}>加入关注</button>{/if}{#if item.reason!=='已设为不再监控'}<button class="tp-btn" onclick={()=>sessionAction(item.task.id,'ignore')}>不再监控</button>{/if}</div></article>{/each}
      {#if !sessions.items.length}<div class="empty">{loadingSessions?'正在读取…':'暂无已发现会话。启用来源后开始一个任务，或检查来源连接。'}</div>{/if}
      <div class="pagination"><span>共 {sessions.total} 项 · 当前 {sessions.total?offset+1:0}–{Math.min(offset+30,sessions.total)}</span><button class="tp-btn" disabled={offset===0} onclick={()=>{offset=Math.max(0,offset-30);void loadSessions();}}>上一页</button><button class="tp-btn" disabled={offset+30>=sessions.total} onclick={()=>{offset+=30;void loadSessions();}}>下一页</button></div>
    {:else if settings && tab==='notifications'}
      <section class="preferences"><h2>近期活动</h2><label>普通关注任务保留时间<select value={settings.recent_days ?? 7} onchange={e=>{settings!.recent_days=Number(e.currentTarget.value);changed();}}>{#each [3,7,14,30,0] as days}<option value={days}>{days ? '最近 '+days+' 天' : '全部关注，不按时间归档'}</option>{/each}</select></label><p class="help">按执行活动计算，不按刷新或重命名计算。运行中的任务、重新手动固定的任务始终显示；其他过期关注移入“较早关注”，保留未读状态，不删除记录。旧版本的固定标记会保留在归档中，可重新固定使其常驻。也可以切换到“全部关注”。</p><h2>任务提醒</h2>{#if notificationError}<p class="message error" role="alert">最近通知发送失败：{notificationError}</p>{/if}<label class="radio"><input type="checkbox" bind:checked={settings.notifications} onchange={changed}/>任务结束、失败或需要确认时提醒</label><label class="radio"><input type="checkbox" bind:checked={settings.sound} onchange={changed}/>播放通知声音</label><label>提醒方式<select value={settings.notification_mode ?? 'popup'} onchange={e=>{settings!.notification_mode=e.currentTarget.value as 'popup'|'system'|'both';changed();}}><option value="popup">TaskPulse 独立弹窗（推荐）</option><option value="system">Windows / 系统通知</option><option value="both">独立弹窗 + 系统通知</option></select></label><p class="help">普通结果在右下角显示 15 秒，悬停暂停；已确认的授权请求和结构化提问提醒持续保留，可切换下一条，不抢编辑焦点；关闭不等于已查看。每个任务分别提醒，不合并；同一事件只提醒一次。通知消失后，未查看结果仍保留在关注记录；铃铛可回查未解决请求，只有新鲜明确状态才能手动重开。系统勿扰设置可能隐藏横幅。</p><button class="secondary tp-btn" disabled={dirty} onclick={testNotification}>发送测试提醒</button><h2>默认视图</h2><label class="radio"><input type="radio" name="view" checked={settings.default_view==='tasks'} onchange={()=>{settings!.default_view='tasks';changed();}}/>任务优先：进行中、结果待查看、状态待更新</label><label class="radio"><input type="radio" name="view" checked={settings.default_view==='projects'} onchange={()=>{settings!.default_view='projects';changed();}}/>按项目分组</label></section>
    {/if}
    {#if tab==='notifications'}
      <section class="delivery"><h2>提醒行为与投递记录</h2><p class="help">完成提醒 15 秒后收起；已确认的授权请求和结构化提问提醒不会自动收起，可切换下一条，匹配请求处理后自动移除。不根据提权参数或命令耗时猜测授权状态，也不替你批准。来源不提供明确审批事件时，授权提醒可能无法检测。</p><button class="secondary tp-btn" onclick={()=>void loadDelivery()}>刷新投递记录</button><p class="help">“界面已渲染”不代表你已阅读；系统 API 接受也不证明横幅已出现。保存最近 500 条记录，不包含命令或提示词。</p>{#if deliveryError}<p role="alert" class="message error">{deliveryError}</p>{/if}
      {#each deliveryRecords.slice(0,25) as row}<div class="delivery-row"><time>{new Date(row.at).toLocaleString()}</time><strong title={row.task_id ?? ''}>{row.test?'测试 · ':''}{row.title}</strong><span>{row.channel} · {deliveryStage(row.stage)}</span></div>{/each}
      {#if !deliveryRecords.length && !deliveryError}<p class="help">尚无投递记录；升级前的弹窗无法追溯补录。</p>{/if}</section>
    {/if}
    {#if settings && tab==='sources'}<details class="hook-registry tp-fold"><summary>高级 · 宿主 Hook 登记与撤销（含已移除来源）</summary><IntegrationHookCleanup revision={applied?.sources.map(s=>s.id).join(',')??''}/></details>{/if}
  </main>
  {#if closePrompt}<div class="modal"><dialog open use:focusTrap aria-modal="true" aria-labelledby="close-title"><h2 id="close-title">还有未保存的配置</h2><p>启用开关已即时生效；项目范围和其他编辑尚未保存。</p><div><button class="tp-btn" onclick={()=>closePrompt=false}>继续编辑</button><button class="tp-btn" onclick={discardAndClose}>放弃未保存编辑</button><button class="primary tp-btn" data-variant="primary" disabled={saving || conflict} onclick={saveAndClose}>保存并关闭</button></div></dialog></div>{/if}
  {#if eventPrompt}<div class="modal"><dialog open use:focusTrap aria-modal="true" aria-labelledby="events-title"><h2 id="events-title">{eventPrompt.enabled?'启用 Claude 精确事件':'移除 Claude 精确事件'}</h2><p>这会修改所选 Claude 数据目录中的 settings.json，仅添加或移除 TaskPulse Hook。首次修改会备份；已有 Hook、statusLine、代理和模型设置保留。可随时移除。</p><div><button class="tp-btn" onclick={()=>eventPrompt=null}>取消</button><button class="primary tp-btn" data-variant="primary" onclick={confirmEvents}>确认{eventPrompt.enabled?'启用':'移除'}</button></div></dialog></div>{/if}
</div>
<style>
  .manage{height:100vh;display:flex;flex-direction:column;background:var(--tp-bg)}
  header{padding:20px 24px 18px;display:flex;justify-content:space-between;align-items:center;gap:16px;border-bottom:1px solid var(--tp-border);background:linear-gradient(100deg,#76d6c407,transparent 50%),var(--tp-panel);flex:none}
  .page-title{display:flex;gap:12px;align-items:center;min-width:0}.page-title>div{min-width:0}.page-icon{width:37px;height:37px;flex:none;display:grid;place-items:center;border:1px solid var(--tp-accent-line);background:var(--tp-accent-soft);border-radius:var(--tp-radius-md);color:var(--tp-accent)}
  .eyebrow{color:var(--tp-accent);letter-spacing:.13em;font-size:9px}h1{font-size:19px;line-height:1.5;font-weight:600;margin:3px 0}header p{color:var(--tp-muted);font-size:11px;margin:0}
  header>.primary{flex:none;font-size:11px}
  nav{display:flex;padding:0 24px;gap:20px;border-bottom:1px solid var(--tp-border);background:var(--tp-panel);flex:none}
  nav .tp-btn{border:0;border-bottom:2px solid transparent;box-shadow:none;border-radius:0;background:transparent;min-height:42px;padding:8px 0;font-size:12px;color:var(--tp-muted)}nav .active{border-bottom-color:var(--tp-accent);color:var(--tp-accent)}
  main{flex:1;min-height:0;overflow:auto;padding:20px 24px;overscroll-behavior:contain}
  .setup-flow{list-style:none;margin:0 0 20px;padding:0;display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:1px;border:1px solid var(--tp-border);background:var(--tp-border);border-radius:var(--tp-radius-md);overflow:hidden}
  .setup-flow li{display:flex;gap:9px;align-items:center;padding:11px 12px;background:var(--tp-panel);min-width:0}
  .step-number{display:grid;place-items:center;width:21px;height:21px;flex:none;border:1px solid var(--tp-border-strong);border-radius:50%;font-size:10px;color:var(--tp-quiet);font-variant-numeric:tabular-nums}
  .step-label{font-size:11px;color:var(--tp-muted);line-height:1.6;overflow-wrap:anywhere}
  .setup-flow li.current{background:linear-gradient(110deg,var(--tp-accent-soft),transparent),var(--tp-panel);box-shadow:inset 0 -2px var(--tp-accent)}
  .current .step-number{color:var(--tp-on-accent);background:var(--tp-accent);border-color:var(--tp-accent);font-weight:650}.current .step-label{color:var(--tp-text)}
  .source-layout{display:grid;grid-template-columns:235px minmax(0,1fr);gap:20px}
  .source-layout>aside{border-right:1px solid var(--tp-border);padding-right:16px;min-width:0}
  .sidebar-title{font-size:10px;color:var(--tp-muted);display:flex;align-items:center;gap:8px;justify-content:space-between;margin:0 0 12px;font-weight:500}.sidebar-title .tp-chip{padding:0 5px;font-size:9px}
  .add{display:flex;gap:5px;margin-bottom:12px}.add select{min-width:0;flex:1;font-size:11px;padding:6px}.add>.tp-btn{font-size:11px;padding:5px 7px;white-space:nowrap}
  .source-item{display:flex;align-items:flex-start;gap:9px;padding:10px 9px;border:1px solid transparent;border-radius:var(--tp-radius-md);margin:5px 0}
  .source-item input{margin-top:4px;flex:none}.source-item.chosen{border-color:var(--tp-accent-line);background:var(--tp-accent-soft);box-shadow:inset 2px 0 var(--tp-accent)}
  .source-item .source-select{text-align:left;flex:1;min-width:0;display:block;padding:0;border:0;background:transparent;border-radius:3px}
  .source-select strong{display:block;font-size:12px;line-height:1.6;font-weight:600;overflow-wrap:anywhere}
  .source-reading{font-size:9px;margin-top:6px;padding:1px 5px}.source-select small{display:block;font-size:10px;color:var(--tp-quiet);margin-top:5px}
  .editor{min-width:0;background:var(--tp-panel);border:1px solid var(--tp-border);border-radius:var(--tp-radius-lg);padding:18px;box-shadow:var(--tp-shadow-card)}
  h2{font-size:15px;line-height:1.6;font-weight:600;margin:0 0 12px;overflow-wrap:anywhere}h3{font-size:12px;color:var(--tp-text);margin:20px 0 10px;font-weight:600}
  .setup-status{display:flex;flex-wrap:wrap;gap:6px;margin:0 0 14px}.setup-status .tp-chip{font-size:10px}
  .capability-note{display:flex;align-items:flex-start;gap:9px;padding:10px 12px;border-radius:var(--tp-radius-md);background:var(--tp-input);color:var(--tp-muted)}
  .capability-note :global(svg){color:var(--tp-accent);flex:none;margin-top:3px}.capability-note p{font-size:11px;line-height:1.8;margin:0}
  .source-tests{display:flex;gap:8px;flex-wrap:wrap;margin:12px 0 8px}.source-tests>.tp-btn{font-size:11px;min-height:31px;padding:6px 9px}
  .read-result{font-size:11px;color:var(--tp-accent);line-height:1.8;border-left:2px solid var(--tp-accent-line);padding-left:10px;margin:8px 0}
  label{display:flex;flex-direction:column;gap:7px;font-size:11px;color:var(--tp-muted);margin:14px 0}
  .radio{flex-direction:row;align-items:center;gap:9px;margin:10px 0;line-height:1.8}.radio input{flex:none}
  textarea{resize:vertical;min-height:65px}.help{font-size:11px;line-height:1.9;color:var(--tp-muted);margin:9px 0}.path{font-size:10px;overflow-wrap:anywhere;color:var(--tp-quiet)}
  .remove{margin-top:12px;font-size:11px}.advanced,.discovered,.hook-registry{margin-top:18px;font-size:11px;color:var(--tp-muted)}
  .advanced>p,.advanced>label,.advanced>h3{margin-left:12px;margin-right:12px}.advanced>.tp-btn{margin-left:12px;margin-bottom:10px;font-size:11px}
  .candidate{margin:10px;padding:10px 0 0;border-top:1px solid var(--tp-border)}.candidate strong{display:block;font-size:11px;font-weight:550;overflow-wrap:anywhere}.candidate>.tp-btn{font-size:10px;min-height:28px;padding:4px 7px;margin-top:7px}.discovered>.help{margin:10px 12px;font-size:10px}
  .setup-empty{padding:24px;background:var(--tp-card);border-style:dashed}.setup-empty p{font-size:12px;color:var(--tp-muted);line-height:1.9}
  .message,.welcome{padding:9px 24px;font-size:11px;line-height:1.8;color:var(--tp-accent);background:var(--tp-accent-soft);display:flex;align-items:center;gap:10px;flex:none;max-height:100px;overflow:auto}
  .message>.tp-btn{margin-left:auto;font-size:11px;min-height:26px;padding:2px 6px;background:transparent;border:0}.message.error{color:var(--tp-danger);background:#f09ba70b}.editor .message{padding:8px 10px}
  .session-filters{display:flex;gap:8px;flex-wrap:wrap}.session-filters input{flex:1;min-width:130px}.session-filters select{max-width:230px}.session-filters>.tp-btn{font-size:11px}
  .session{padding:14px;background:var(--tp-panel);border:1px solid var(--tp-border);border-radius:var(--tp-radius-md);margin:8px 0;display:flex;gap:12px;justify-content:space-between}.session>div:first-child{min-width:0}.session strong{font-size:12px;font-weight:550;overflow-wrap:anywhere}.session p{font-size:11px;color:var(--tp-muted);margin:5px 0;overflow-wrap:anywhere}.session small{font-size:10px;color:var(--tp-quiet)}.session-actions{display:flex;align-items:center;gap:4px;flex-shrink:0}.session-actions>.tp-btn{font-size:10px;min-height:28px;padding:4px 7px}.session-actions span{font-size:11px;color:var(--tp-accent)}
  .pagination{display:flex;align-items:center;gap:8px;padding-top:16px;font-size:11px;color:var(--tp-muted);flex-wrap:wrap}.pagination span{margin-right:auto}.pagination>.tp-btn{font-size:11px}
  .empty{text-align:center;padding:36px;font-size:12px;color:var(--tp-muted)}.preferences,.delivery{max-width:680px;background:var(--tp-panel);border:1px solid var(--tp-border);border-radius:var(--tp-radius-lg);padding:18px}
  .preferences h2:not(:first-child){margin-top:26px}.preferences>.tp-btn,.delivery>.tp-btn{font-size:11px}
  .delivery{margin-top:20px}.delivery-row{padding:10px 0;border-bottom:1px solid var(--tp-border);display:flex;flex-direction:column;gap:4px;font-size:11px;overflow-wrap:anywhere}.delivery-row time,.delivery-row span{color:var(--tp-muted)}.delivery-row strong{font-weight:550;color:var(--tp-text)}
  .modal{position:fixed;inset:0;display:grid;place-items:center;background:#0009;z-index:10;padding:20px}
  .modal dialog{position:static;margin:0;color:var(--tp-text);background:var(--tp-raised);border:1px solid var(--tp-border-strong);border-radius:var(--tp-radius-lg);padding:20px;max-width:500px;box-shadow:var(--tp-shadow-window)}
  .modal p{font-size:12px;color:var(--tp-muted);margin:12px 0 20px;line-height:1.8}.modal dialog>div{display:flex;gap:8px;justify-content:flex-end;flex-wrap:wrap}.modal .tp-btn{font-size:11px}
  @media(max-width:800px){header,main{padding-left:16px;padding-right:16px}nav{padding:0 16px}.source-layout{grid-template-columns:200px minmax(0,1fr);gap:14px}.editor{padding:14px}.setup-flow li{padding:10px 8px;gap:6px}.step-label{font-size:10px}}
  @media(max-width:600px){.source-layout{grid-template-columns:1fr}.source-layout>aside{border-right:0;border-bottom:1px solid var(--tp-border);padding:0 0 14px}.setup-flow{grid-template-columns:repeat(2,minmax(0,1fr))}.session{flex-direction:column}.page-icon{display:none}.page-title{gap:0}h1{font-size:17px}header{padding-top:14px;padding-bottom:14px}header>.tp-btn{padding:7px;font-size:10px}}
</style>
