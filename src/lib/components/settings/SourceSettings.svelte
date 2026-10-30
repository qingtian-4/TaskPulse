<script lang="ts">
  import { onMount } from 'svelte';
  import {get} from 'svelte/store';
  import { Radio, Bell, Info } from '@lucide/svelte';
  import {t} from '$lib/i18n';
  import {appSettings,savePreference,preferencePending} from '$lib/preferences';
  import {reconcileSourceDraft} from '$lib/settings-draft';
  import SettingRow from './SettingRow.svelte';

  import IntegrationEditor from '$lib/components/IntegrationEditor.svelte';
  import IntegrationHookCleanup from '$lib/components/IntegrationHookCleanup.svelte';
  import { invoke, isTauri } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { taskContext, taskLabel } from '$lib/dashboard';
  import { sourceReadState, readTestResult, sourceCapability } from '$lib/source-setup';
  import { deliveryStage } from '$lib/notice';
  import type { DeliveryRecord } from '$lib/types';
  import { formatAgo, type DashboardSettings, type MonitorSource, type SourceDiscovery, type SourceKind, type SessionPage, type ProviderHealth } from '$lib/types';
  let {onDirtyChange,onAttention}:{onDirtyChange:(value:boolean)=>void;onAttention:()=>void}=$props();
  $effect(()=>onDirtyChange(dirty||saving||conflict));
  type Definition={description?:string;id:string;name:string;kind:SourceKind;category:string;location_kind:string;platforms:string[];adapter_id?:string;agent_id?:string};
  let catalog=$state<Definition[]>([]),category=$state('local'),foundOnly=$state(false),platform=$state('windows');
  let settings=$state<DashboardSettings | null>(null);
  let discovery=$state<SourceDiscovery[]>([]);
  let health=$state<ProviderHealth[]>([]);
  let tab=$state<'sources'|'sessions'|'notifications'|'updates'>('sources');
  let selected=$state('');
  let query=$state('');
  let sessionSource=$state('');
  let offset=$state(0);
  let sessions=$state<SessionPage>({items:[],total:0});
  let error=$state('');
  let notice=$state('');
  let buildInfo=$state<{version:string;commit:string;modified:boolean;profile:string;executable:string}|null>(null);
  let saving=$state(false);
  let updating=$state(false);
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
        if(result!=='pending'){readTest={id,passed:result==='passed',message:result==='passed'?'已连接，等待新任务':'本次读取失败 · 查看高级诊断'};return;}
      }
      readTest={id,message:'读取超时 · 保留原状态，不视为任务结束'};
    }catch(e){error=String(e);}finally{reading=false;}
  }
  let editor=$derived(settings?.sources.find(s=>s.id===selected) ?? null);
  const kinds=$derived(catalog.map(d=>({kind:d.kind,name:d.name})));
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
    const builtin:Partial<Record<SourceKind,string>>={opencode:'opencode',workbuddy:'workbuddy',zcode:'zcode',doubao:'doubao',codex:'codex',claude_code:'claude_code',hermes:'hermes',deepseek_desktop:'deepseek_harness_desktop',deepseek_wsl:'deepseek_harness',hooks:'local_file',scanner:'scanner'};
    const candidate=builtin[addKind];
    if(['codex','hooks','scanner'].includes(addKind) && settings.sources.some(s=>s.kind===addKind)){selected=settings.sources.find(s=>s.kind===addKind)!.id;notice='本机来源已存在，请在左侧启用或编辑。';return;}
    const id=candidate && !settings.sources.some(s=>s.id===candidate) ? candidate : addKind+':'+crypto.randomUUID();
    const source:MonitorSource={id,name:kinds.find(k=>k.kind===addKind)?.name ?? addKind,kind:addKind,enabled:true,location:['deepseek_wsl','codex_wsl'].includes(addKind)?'Ubuntu':'',projects:['*'],excluded_projects:[],tmux_sessions:[],tmux_panes:[],poll_interval_ms:3000};
    if(addKind==='generic_event')source.integration={agent_id:'custom-agent',adapter_id:'standard-events',transport:'local',accept_since_ms:0};
    settings.sources.push(source);selected=id;dirty=true;
  }
  function remove(id:string){if(!settings)return;settings.sources=settings.sources.filter(s=>s.id!==id);selected=settings.sources[0]?.id ?? '';dirty=true;}
  function configError(e:unknown){error=String(e);if(error==='配置已更新，请重新加载后保存')conflict=true;}
  async function save(){
    if(!settings || !isTauri() || conflict)return;
    saving=true;error='';notice='';
    try { settings.onboarding_complete=true;settings=await invoke<DashboardSettings>('save_monitoring_settings',{settings:$state.snapshot(settings),expected:applied ? $state.snapshot(applied) : null});applied=structuredClone($state.snapshot(settings));discovery=await invoke<SourceDiscovery[]>('discover_sources');dirty=false;notice='已保存并即时生效。受影响来源正在重新采集。'; }
    catch(e){configError(e);}finally{saving=false;}
  }
  async function toggleSource(source:MonitorSource,enabled:boolean){
    if(!settings || saving || conflict)return;
    // First setup and newly added sources remain a clearly labelled draft.
    if(!applied?.onboarding_complete || !applied.sources.some(s=>s.id===source.id)){source.enabled=enabled;changed();return;}
    const previous=source.enabled;
    saving=true;error='';
    try {
      const expected=structuredClone($state.snapshot(applied));
      const current=structuredClone(expected);
      const target=current.sources.find(s=>s.id===source.id);
      if(!target)throw Error('来源已被移除，请重新打开配置');
      target.enabled=enabled;
      const next=await invoke<DashboardSettings>('save_monitoring_settings',{settings:current,expected});
      applied=structuredClone(next);source.enabled=enabled;
      if(!dirty)settings=structuredClone(next);
      notice=(enabled?'已启用：':'已停用：')+source.name+'。开关即时生效；其他编辑仍需保存。';
    }catch(e){source.enabled=previous;configError(e);}finally{saving=false;}
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
  async function testNotification(){try{await invoke('test_notification');notificationTest=true;await loadDelivery();notice='测试提醒已发送。';}catch(e){error=String(e);}}
  onMount(()=>{
    if(!isTauri()){error='管理配置需要在 TaskPulse 托盘应用中打开。浏览器页面不会写入真实配置。';return;}

    const cleanups:Array<()=>void>=[];let mounted=true;
    void(async()=>{
      try {

        catalog=await invoke<Definition[]>('get_source_catalog');
        const catalogCleanup=await listen('integration-catalog-updated',()=>{void invoke<Definition[]>('get_source_catalog').then(value=>catalog=value).catch(e=>error=String(e));});if(mounted)cleanups.push(catalogCleanup);else catalogCleanup();
        platform=(await invoke<{platform:string}>('integration_environment')).platform;
        buildInfo=await invoke('get_build_info');
        settings=await invoke<DashboardSettings>('get_monitoring_settings');
        applied=structuredClone($state.snapshot(settings));
        error=await invoke<string | null>('get_monitoring_error') ?? '';
        discovery=await invoke<SourceDiscovery[]>('discover_sources');
        health=await invoke<ProviderHealth[]>('get_providers');
        notificationError=await invoke<string | null>('get_notification_error') ?? '';
        if(!settings.onboarding_complete){for(const source of settings.sources){source.enabled=['codex','claude_code','hermes','deepseek_desktop','doubao','zcode','opencode','workbuddy'].includes(source.kind) && (discovery.find(d=>d.id===source.id)?.available ?? false);}dirty=true;}
        selected='';
        const updateCleanup=await listen<{phase:string}>('application-update',e=>updating=['downloading','verifying','installing'].includes(e.payload.phase));if(mounted)cleanups.push(updateCleanup);else updateCleanup();
        const currentUpdate=await invoke<{phase:string}>('get_update_status');updating=['downloading','verifying','installing'].includes(currentUpdate.phase);
        const cleanup=await listen<ProviderHealth[]>('providers-updated',e=>health=e.payload);if(mounted)cleanups.push(cleanup);else cleanup();
        const notificationCleanup=await listen<string>('notification-error',e=>notificationError=e.payload);if(mounted)cleanups.push(notificationCleanup);else notificationCleanup();
        const settingsCleanup=await listen<DashboardSettings>('monitoring-settings-updated',e=>{
          if(saving)return;
          if(dirty && applied && settings){
            const next=reconcileSourceDraft($state.snapshot(settings),$state.snapshot(applied),e.payload);
            if(next.conflict){conflict=true;error='监控配置已在其他窗口更新，暂不允许覆盖。请加载最新配置后编辑。';return;}
            settings=next.settings;applied=structuredClone(e.payload);return;
          }
          settings=structuredClone(e.payload);applied=structuredClone(e.payload);
        });if(mounted)cleanups.push(settingsCleanup);else settingsCleanup();
        const closeCleanup=await getCurrentWindow().onCloseRequested(e=>{if(get(preferencePending)){e.preventDefault();onAttention();error='正在保存设置，请稍候再关闭。';return;}if((dirty || saving) && !closing){e.preventDefault();onAttention();closePrompt=true;}});if(mounted)cleanups.push(closeCleanup);else closeCleanup();
      } catch(e){error=String(e);}
    })();
    const interval=setInterval(()=>{if(tab==='sessions')void loadSessions();if(notificationTest)void loadDelivery();},3000);
    const discoveryTimer=setInterval(()=>{if(tab==='sources')void invoke<SourceDiscovery[]>('discover_sources').then(value=>discovery=value).catch(e=>error=String(e));},30000);
    const clock=setInterval(()=>now=Date.now(),1000);
    return()=>{mounted=false;clearInterval(interval);clearInterval(discoveryTimer);clearInterval(clock);cleanups.forEach(fn=>fn());};
  });

  function definitionFor(source:MonitorSource):Definition {
    return catalog.find(d=>d.kind===source.kind&&(d.kind!=='generic_event'||d.adapter_id===source.integration?.adapter_id))??{id:source.id,name:source.name,kind:source.kind,category:'custom',location_kind:'none',platforms:[platform]};
  }
  function rows(){
    const available=catalog.filter(d=>d.category===category&&(!d.platforms.length||d.platforms.includes(platform)));
    const saved=(settings?.sources??[]).filter(s=>definitionFor(s).category===category).map(source=>({definition:definitionFor(source),source,found:discovery.find(d=>d.id===source.id)?.available??false}));
    const missing=available.filter(d=>!saved.some(r=>r.definition.id===d.id)).map(definition=>({definition,source:undefined as MonitorSource|undefined,found:discovery.some(d=>d.available&&(d.id===definition.id||d.candidate?.kind===definition.kind&&definition.kind!=='generic_event'))}));
    return [...saved,...missing].filter(r=>!foundOnly||r.found);
  }
  function choose(definition:Definition,source?:MonitorSource){
    if(!settings)return;
    if(dirty){notice='配置来源前请先应用或取消当前更改。';return;}
    if(source){selected=selected===source.id?'':source.id;return;}
    const id=['codex','local_file','scanner'].includes(definition.id)?definition.id:definition.id+':'+crypto.randomUUID();
    const created:MonitorSource={id,name:definition.name,kind:definition.kind,enabled:true,location:definition.location_kind==='distro'?'Ubuntu':'',projects:['*'],excluded_projects:[],tmux_sessions:[],tmux_panes:[],poll_interval_ms:3000};
    if(definition.kind==='generic_event')created.integration={agent_id:definition.agent_id!,adapter_id:definition.adapter_id!,transport:'local',accept_since_ms:0};
    settings.sources.push(created);selected=id;dirty=true;
  }
  function autoDiscover(value:boolean){void savePreference({auto_discover_local:value}).catch(()=>{});}
  async function detect(){try{discovery=await invoke<SourceDiscovery[]>('discover_sources');notice='检测完成';}catch(e){error=String(e);}}
  function showSessions(){tab=tab==='sessions'?'sources':'sessions';if(tab==='sessions')void loadSessions();}
</script>

<div class="source-settings">
  <div class="settings-tabs" role="group" aria-label={$t("来源类型")}>{#each [['local',$t('本机 Agent')],['remote',platform==='windows'?$t('WSL 与远程'):$t('远程来源')],['custom',$t('自定义接入')]] as [id,label]}<button class="settings-tab" aria-pressed={category===id&&tab==='sources'} onclick={()=>{category=id;tab='sources';}}>{label}</button>{/each}<button class="settings-tab" aria-pressed={tab==='sessions'} onclick={showSessions}>{$t("已发现会话")}</button></div>
  {#if error}<div class="settings-inline-error" role="alert">{error}<button class="tp-btn" onclick={()=>error=''} aria-label={$t("关闭错误")}>×</button></div>{/if}
  {#if conflict}<button class="tp-btn" onclick={reloadSettings}>{$t("放弃未保存编辑，加载最新配置")}</button>{/if}
  {#if notice}<p class="source-message" role="status">{$t(notice)}</p>{/if}
  {#if settings && !settings.onboarding_complete}<div class="welcome"><p>{$t("首次确认后，开始关注所选来源的新执行。")}</p><button class="tp-btn" data-variant="primary" disabled={saving||conflict} onclick={save}>{$t("开始监控")}</button></div>{/if}
  <fieldset disabled={updating}>
  {#if tab==='sources'&&settings}
    {#if category==='local'}<div class="settings-group"><SettingRow label={$t("自动发现本机 Agent")} description={$t("识别新的只读本机来源，保留已关闭来源和项目范围。")}><input class="settings-switch" type="checkbox" role="switch" aria-label={$t("自动发现本机 Agent")} checked={$appSettings?.auto_discover_local??false} disabled={$preferencePending} onchange={e=>autoDiscover(e.currentTarget.checked)}/></SettingRow></div>{/if}
    <div class="source-toolbar"><div class="settings-actions"><button class="settings-tab" aria-pressed={!foundOnly} onclick={()=>foundOnly=false}>{$t("全部接入")}</button><button class="settings-tab" aria-pressed={foundOnly} onclick={()=>foundOnly=true}>{$t("已发现")}</button></div><button class="tp-btn" onclick={detect}>{$t("重新检测")}</button></div>
    <div class="source-list">{#each rows() as row (row.definition.id+':'+(row.source?.id??''))}
      <article class="source-card"><div class="source-row"><div class="source-identity"><strong>{row.source?.name??$t(row.definition.name)}</strong><small>{row.definition.kind==='scanner'?$t('仅进程识别'):row.definition.kind==='generic_event'?$t('事件接入'):row.definition.category==='remote'?$t('独立连接'):$t('原生会话读取')} · {row.found?$t('已发现'):$t('尚未发现数据')}</small></div>{#if row.source}<span class="read-state">{$t(sourceState(row.source))}</span><input class="settings-switch" type="checkbox" role="switch" aria-label={'启用 '+row.source.name} checked={row.source.enabled} disabled={saving} onchange={e=>void toggleSource(row.source!,e.currentTarget.checked)}/>{/if}<button class="tp-btn source-expand" aria-label={(row.source?.name??$t(row.definition.name))+' 设置'} aria-expanded={!!row.source&&selected===row.source.id} onclick={()=>choose(row.definition,row.source)}>{row.source&&selected===row.source.id?$t('收起'):$t('配置')}</button></div>
      {#if editor&&row.source?.id===editor.id}<div class="source-editor"><label>{$t('来源名称')}<input bind:value={editor.name} oninput={changed}/></label>
          <div class="capability-note tp-surface"><Info size={15} aria-hidden="true"/><p>{$t(row.definition.description??sourceCapability(editor.kind))}</p></div>
          <div class="source-tests"><button class="secondary tp-btn" title={$t("只检查所选来源本次读取，不验收 Agent 结束事件")} disabled={dirty || saving || reading || !editor.enabled} onclick={testRead}><Radio size={14}/>{reading?$t('正在读取…'):$t('测试读取来源')}</button><button class="secondary tp-btn" title={$t("只测试全局通知通道，不创建任务")} disabled={dirty || saving} onclick={testNotification}><Bell size={14}/>{$t("发送测试提醒")}</button></div>
          {#if readTest?.id===editor.id}<p class="read-result" role="status">{$t(readTest.message)}</p>{/if}
          {#if editor.kind==='generic_event'}{#key editor.id}<IntegrationEditor source={editor} {dirty} changed={changed}/>{/key}
          {:else if definitionFor(editor).location_kind!=='none'}<label>{definitionFor(editor).location_kind==='host'?$t('SSH 配置别名'):definitionFor(editor).location_kind==='distro'?$t('WSL 发行版'):$t('数据位置')}<input bind:value={editor.location} oninput={changed} placeholder={$t("留空使用自动发现的位置")}/></label>
          {:else}<p class="help">{discovery.find(d=>d.id===editor!.id)?.detail??$t('使用本机默认配置')}</p>{/if}
          <h3>{$t("项目范围")}</h3>
          <label class="radio"><input type="radio" name="scope" checked={editor.projects.includes('*')} onchange={()=>{editor!.projects=['*'];changed();}}/>{$t("自动关注所有项目的新任务")}</label>
          <label class="radio"><input type="radio" name="scope" checked={!editor.projects.includes('*')} onchange={()=>{editor!.projects=[];changed();}}/>{$t("只关注指定项目")}</label>
          {#if !editor.projects.includes('*')}<label>{$t("项目名称或完整路径（每行一个）")}<textarea value={editor.projects.join('\n')} oninput={e=>{editor!.projects=list(e.currentTarget.value);changed();}} placeholder="agenttray&#10;D:\Project\my-project&#10;@unassociated"></textarea></label>{/if}
          <p class="help">{$t("推荐自动关注所有项目的新任务：只要来源启用，旧项目再次执行也会自动回来。主面板按最近")} {settings.recent_days ?? 7} {$t("天的执行活动显示，不复制历史会话。指定项目限制不会自动改写，可在上方取消。排除规则优先。")}</p>
          {#if !editor.projects.includes('*')}<label class="radio"><input type="checkbox" checked={editor.projects.includes('@unassociated')} onchange={e=>{editor!.projects=e.currentTarget.checked ? [...editor!.projects,'@unassociated'] : editor!.projects.filter(p=>p!=='@unassociated');changed();}}/>{$t("也包括未关联项目的会话")}</label>{/if}
          {#if editor.kind==='tmux'}<label>{$t("tmux session（留空表示全部，每行一个）")}<textarea value={editor.tmux_sessions.join('\n')} oninput={e=>{editor!.tmux_sessions=list(e.currentTarget.value);changed();}}></textarea></label><label>{$t("tmux pane（可选，每行一个）")}<textarea value={editor.tmux_panes.join('\n')} oninput={e=>{editor!.tmux_panes=list(e.currentTarget.value);changed();}}></textarea></label>{/if}
          <p class="help">{$t("保存后，范围内的新任务会自动出现。可以回到 Agent 开始一轮对话，随后在托盘查看状态。")}</p>
          <details class="advanced tp-fold"><summary>{$t("高级设置 · 排除、诊断与来源移除")}</summary>
          <label>{$t("排除项目（可选，每行一个）")}<textarea value={editor.excluded_projects.join('\n')} oninput={e=>{editor!.excluded_projects=list(e.currentTarget.value);changed();}}></textarea></label>
          {#if discovery.find(d=>d.id===editor!.id)}<p class="path">{discovery.find(d=>d.id===editor!.id)?.detail}</p>{/if}
          {#if health.find(h=>h.provider===editor!.id)?.message}<p class="message error">{health.find(h=>h.provider===editor!.id)?.message}</p>{/if}
          {#if editor.kind==='claude_code'}<h3>{$t("自愿接入精确事件")}</h3><p class="help">{$t("不会覆盖 statusLine 或其他 Hook；需逐次明确确认，不自动修改审批策略。")}</p><button class="tp-btn" disabled={dirty || saving} onclick={()=>eventPrompt={sourceId:editor!.id,enabled:true}}>{$t("启用精确事件…")}</button><button class="tp-btn" disabled={dirty || saving} onclick={()=>eventPrompt={sourceId:editor!.id,enabled:false}}>{$t("移除精确事件…")}</button>{/if}
          <p class="path">{$t("来源 ID：")}{editor.id} {$t("· 类型：")}{editor.kind}</p>
          <button class="remove tp-btn" data-variant="danger" onclick={()=>remove(editor!.id)}>{$t("移除此监控来源")}</button><p class="help">{$t("移除只影响 TaskPulse，关注记录保留；宿主 Hook 不会自动删除。保存后仍可在下方“宿主 Hook 登记”预览并确认撤销。")}</p></details>

      <div class="settings-actions"><button class="tp-btn" data-variant="primary" disabled={!dirty||saving||conflict} onclick={save}>{saving?$t('保存中…'):$t('应用更改')}</button><button class="tp-btn" disabled={!dirty||saving} onclick={reloadSettings}>{$t("取消更改")}</button></div></div>{/if}</article>
    {:else}<p class="help">{$t("没有符合条件的来源。")}</p>{/each}</div>
    {#if dirty}<div class="source-pending"><span>{$t("监控来源有未应用更改")}</span><div class="settings-actions"><button class="tp-btn" disabled={saving||conflict} onclick={save}>{$t("应用更改")}</button><button class="tp-btn" disabled={saving||$preferencePending} onclick={reloadSettings}>{$t("取消更改")}</button></div></div>{/if}
    {#if category==='custom'}<details class="tp-fold"><summary>{$t("高级 · 宿主 Hook 登记与撤销（含已移除来源）")}</summary><IntegrationHookCleanup revision={applied?.sources.map(s=>s.id).join(',')??''}/></details>{/if}
  {:else if tab==='sessions'&&settings}
      <div class="session-filters"><select bind:value={sessionSource} onchange={()=>{offset=0;void loadSessions();}} aria-label={$t("会话来源")}><option value="">{$t("全部来源")}</option>{#each settings.sources as source}<option value={source.id}>{source.name}</option>{/each}</select><input bind:value={query} placeholder={$t("搜索任务标题、项目或路径")} aria-label={$t("搜索会话")} oninput={()=>{offset=0;void loadSessions();}}/><button class="tp-btn" onclick={()=>loadSessions()} disabled={loadingSessions}>{$t("刷新目录")}</button></div>
      <p class="help">{$t("显示采集器已发现的会话，不是原生历史的完整镜像。尚未启用的来源不会持续采集。目录不等于关注列表。")}</p>
      {#each sessions.items as item (item.task.id)}<article class="session"><div><strong>{item.task.name}</strong><p>{taskContext(item.task,settings)}</p><small>{$t(taskLabel(item.task))} · {item.reason} {$t("· 最后活动")} {$t(formatAgo(item.task.updated_at))}</small></div><div class="session-actions">{#if item.reason==='已设为不再监控'}<button class="tp-btn" onclick={()=>sessionAction(item.task.id,'unignore')}>{$t("恢复监控")}</button>{:else if item.reason==='正在关注'}<span>{$t("已关注")}</span>{:else}<button class="tp-btn" disabled={item.reason==='来源未启用' || item.reason==='来源已移除' || item.reason==='项目不在监控范围' || item.reason==='tmux 不在监控范围'} onclick={()=>sessionAction(item.task.id,'follow')}>{$t("加入关注")}</button>{/if}{#if item.reason!=='已设为不再监控'}<button class="tp-btn" onclick={()=>sessionAction(item.task.id,'ignore')}>{$t("不再监控")}</button>{/if}</div></article>{/each}
      {#if !sessions.items.length}<div class="empty">{loadingSessions?$t('正在读取…'):$t('暂无已发现会话。启用来源后开始一个任务，或检查来源连接。')}</div>{/if}
      <div class="pagination"><span>{$t("共")} {sessions.total} {$t("项 · 当前")} {sessions.total?offset+1:0}–{Math.min(offset+30,sessions.total)}</span><button class="tp-btn" disabled={offset===0} onclick={()=>{offset=Math.max(0,offset-30);void loadSessions();}}>{$t("上一页")}</button><button class="tp-btn" disabled={offset+30>=sessions.total} onclick={()=>{offset+=30;void loadSessions();}}>{$t("下一页")}</button></div>
    {/if}
  </fieldset>
  {#if closePrompt}<div class="settings-modal"><dialog open use:focusTrap aria-modal="true" aria-labelledby="close-title"><h2 id="close-title">{$t("还有未保存的配置")}</h2><p>{$t("启用开关已即时生效；项目范围和其他编辑尚未保存。")}</p><div><button class="tp-btn" onclick={()=>closePrompt=false}>{$t("继续编辑")}</button><button class="tp-btn" onclick={discardAndClose}>{$t("放弃未保存编辑")}</button><button class="primary tp-btn" data-variant="primary" disabled={saving || conflict} onclick={saveAndClose}>{$t("保存并关闭")}</button></div></dialog></div>{/if}
  {#if eventPrompt}<div class="settings-modal"><dialog open use:focusTrap aria-modal="true" aria-labelledby="events-title"><h2 id="events-title">{eventPrompt.enabled?$t('启用 Claude 精确事件'):$t('移除 Claude 精确事件')}</h2><p>{$t("这会修改所选 Claude 数据目录中的 settings.json，仅添加或移除 TaskPulse Hook。首次修改会备份；已有 Hook、statusLine、代理和模型设置保留。可随时移除。")}</p><div><button class="tp-btn" onclick={()=>eventPrompt=null}>{$t("取消")}</button><button class="primary tp-btn" data-variant="primary" onclick={confirmEvents}>{$t("确认")}{eventPrompt.enabled?$t('启用'):$t('移除')}</button></div></dialog></div>{/if}
</div>
<style>
.source-settings{min-width:0}.source-toolbar{display:flex;justify-content:space-between;align-items:center;gap:10px;flex-wrap:wrap;margin:18px 0 10px}.source-card{border-bottom:1px solid var(--tp-border)}.source-row{display:flex;align-items:center;gap:10px;padding:16px 0;min-width:0}.source-identity{flex:1;min-width:0}.source-identity strong{font-size:13px;font-weight:500;overflow-wrap:anywhere}.source-identity small{display:block;font-size:11px;color:var(--tp-muted);margin-top:4px}.read-state{font-size:11px;color:var(--tp-muted);max-width:130px;overflow-wrap:anywhere}.source-expand{padding:5px 8px}.source-editor{padding:18px;margin:0 0 16px 22px;border-radius:8px;background:var(--tp-card);font-size:12px}.source-editor label{display:grid;gap:7px;margin:12px 0}.source-editor label.radio{display:flex;align-items:center;gap:8px}.source-editor input:not([type=checkbox]):not([type=radio]),.source-editor textarea{width:100%;max-width:100%}.source-editor textarea{min-height:80px;resize:vertical}.source-editor h3{font-size:13px;margin:20px 0 10px}.help,.path{font-size:12px;color:var(--tp-muted);line-height:1.7;overflow-wrap:anywhere}.capability-note{display:flex;gap:8px;background:transparent;border:0;box-shadow:none}.capability-note p{margin:0}.capability-note :global(svg){flex:none;margin-top:3px}.source-tests{display:flex;gap:8px;flex-wrap:wrap;margin:14px 0}.source-message{color:var(--tp-accent);font-size:12px;overflow-wrap:anywhere}.source-pending,.welcome{padding:14px;background:var(--tp-card);border-radius:8px;margin:14px 0;font-size:12px}.source-pending{display:grid;gap:10px}.advanced{margin:14px 0}.advanced>label,.advanced>.tp-btn,.advanced>p,.advanced>h3{margin:12px}.advanced textarea{width:100%}.session{padding:14px 0;border-bottom:1px solid var(--tp-border);font-size:12px}.session-actions,.session-filters,.pagination{display:flex;gap:8px;flex-wrap:wrap;align-items:center;margin:12px 0}.session-filters input{flex:1}.session p,.session small{color:var(--tp-muted);overflow-wrap:anywhere}.pagination{justify-content:space-between}.source-editor .settings-actions{margin-top:14px}
@media(max-width:600px){.source-editor{margin-left:0}.read-state{max-width:100px}.source-row{gap:8px}}
@media(max-width:390px){.source-row{flex-wrap:wrap}.read-state{max-width:none;width:100%;order:2}.source-editor{padding:14px}}
</style>
