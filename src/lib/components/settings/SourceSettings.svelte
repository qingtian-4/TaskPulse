<script lang="ts">
  import {onMount,tick} from 'svelte';
  import {get} from 'svelte/store';
  import {invoke,isTauri} from '@tauri-apps/api/core';
  import {listen} from '@tauri-apps/api/event';
  import {getCurrentWindow} from '@tauri-apps/api/window';
  import {Plus,Server,Laptop,Search,RefreshCw,ChevronRight,ArrowLeft,ShieldCheck,Link,Info,Folder,Terminal,Save,Trash2,Check,Pause,Play,X,Settings2,Activity} from '@lucide/svelte';
  import {t} from '$lib/i18n';
  import {appSettings,savePreference,preferencePending} from '$lib/preferences';
  import {autostart} from '$lib/autostart';
  import {reconcileSourceDraft} from '$lib/settings-draft';
  import {servers,createSource,pauseServer,removeServer,projectChoices,sourceStatus,localSourceStatus,scopeLabel,type AgentKind,type SshProbe,type ServerGroup} from '$lib/monitor-sources';
  import type {DashboardSettings,MonitorSource,SourceDiscovery,DashboardSnapshot,SessionPage,SourceSnapshot} from '$lib/types';
  import {formatAgo} from '$lib/types';
  import type {ProjectChoice} from '$lib/project-options';
  import {localHookKind,type SourceDefinition} from '$lib/source-setup';
  import SshSourceWizard from './SshSourceWizard.svelte';
  import ProjectScope from './ProjectScope.svelte';
  import SourceSessions from './SourceSessions.svelte';
  import EventSources from './EventSources.svelte';
  import './monitor-sources.css';
  let {onDirtyChange,onAttention}:{onDirtyChange:(value:boolean)=>void;onAttention:()=>void}=$props();
  let settings=$state<DashboardSettings|null>(null),applied=$state<DashboardSettings|null>(null);
  let snapshot=$state<DashboardSnapshot|null>(null),discovery=$state<SourceDiscovery[]>([]);
  let page=$state<'home'|'local'|'server'>('local'),tab=$state<'local'|'ssh'>('local'),selectedHost=$state(''),query=$state('');
  let dirty=$state(false),saving=$state(false),conflict=$state(false),updating=$state(false),detectionSettled=$state(false);
  let error=$state(''),notice=$state(''),platform=$state('Windows'),wizard=$state(false),closePrompt=$state(false),closing=false;
  let removePrompt=$state(false),eventPrompt=$state<{id:string;enabled:boolean}|null>(null),reading=$state('');
  let reports=$state<Record<string,SshProbe>>({}),probing=$state<Record<string,boolean>>({}),projects=$state<Record<string,string[]>>({});
  let projectAliases=$state<Record<string,ProjectChoice[]>>({});
  let activeLocal=$state<AgentKind>('codex'),activeLocalSource=$state('');
  let expandedLocal=$state<AgentKind|null>(null);
  let localAgents=$state<SourceDefinition[]>([]);
  let eventAgents=$state<SourceDefinition[]>([]);
  const processAgent=$derived(localAgents.find(agent=>agent.kind==='scanner'));
  const manualEventAgents=$derived(eventAgents.filter(agent=>!localHookKind(agent.adapter_id)));
  function hookCatalog(kind:AgentKind){return eventAgents.filter(agent=>localHookKind(agent.adapter_id)===kind);}
  function hookCount(kind:AgentKind){return settings?.sources.filter(s=>s.kind==='generic_event'&&localHookKind(s.integration?.adapter_id)===kind).length??0;}
  let active=true,revision=0;
  let sessionSource=$state<MonitorSource|null>(null);
  const groups=$derived(settings?servers(settings):[]);
  const selected=$derived(groups.find(s=>s.host===selectedHost));
  const shown=$derived(groups.filter(s=>`${s.host} ${s.name}`.toLowerCase().includes(query.toLowerCase())));
  const localRows=$derived(settings?.sources.filter(s=>localAgents.some(agent=>agent.kind===s.kind))??[]);
  const localEditor=$derived(localRows.find(s=>s.kind===activeLocal&&s.id===activeLocalSource)??localRows.find(s=>s.kind===activeLocal));
  const editor=$derived(page==='server'?selected?.codex:localEditor);
  const changed=$derived(dirty||saving||conflict||wizard);
  $effect(()=>onDirtyChange(changed));
  function observation(s?:MonitorSource):SourceSnapshot|undefined{return snapshot?.view.sources.find(v=>v.id===s?.id);}
  function taskCount(s?:MonitorSource){return s?.enabled?(observation(s)?.current_tasks??0):0;}
  function serverTaskCount(group:ServerGroup){return group.sources.reduce((total,source)=>total+taskCount(source),0);}
  function localDetection(kind:AgentKind,source?:MonitorSource){
    return discovery.find(d=>source ? d.id===source.id : d.candidate?.kind===kind)?.data_location_detected;
  }
  function connection(group:ServerGroup):string {
    const report=reports[group.host];if(report&&Date.now()-report.checked_at<60000)return report.connection;
    const host=snapshot?.hosts.find(h=>h.host===group.host);
    return host?.last_seen&&Date.now()-host.last_seen<30000?host.status:'unknown';
  }
  function sshLabel(group:ServerGroup){return ({connected:'SSH 已连接',disconnected:'SSH 未连接',unknown:'SSH 尚未检测'})[connection(group)]??'SSH 尚未检测';}
  function rowStatus(kind:AgentKind,source?:MonitorSource){
    const saved=applied?.sources.find(s=>s.id===source?.id);
    if(source&&(!saved||source.enabled!==saved.enabled))return '待保存';
    const message=snapshot?.providers.find(p=>p.provider===source?.id)?.message;
    if(kind==='trae'&&source?.enabled
      && (message?.includes('当前会话库格式不可直接读取')||message?.includes('目前只读取公开会话元数据'))){
      return '已检测到 · 仅支持会话目录';
    }
    return localSourceStatus(source,observation(source),localDetection(kind,source));
  }
  function assign(next:DashboardSettings){settings=next;dirty=true;notice='';}
  function touch(){dirty=true;notice='';}
  function fail(e:unknown){error=String(e);if(error.includes('配置已更新')||error.includes('其他窗口更新'))conflict=true;}
  async function commit(proposal:DashboardSettings):Promise<boolean>{
    if(!applied||saving||conflict)return false;saving=true;error='';
    try{const next=await invoke<DashboardSettings>('save_monitoring_settings',{settings:proposal,expected:$state.snapshot(applied)});settings=structuredClone(next);applied=structuredClone(next);dirty=false;notice='已保存';return true;}
    catch(e){fail(e);return false;}finally{saving=false;}
  }
  async function save(){if(settings)await commit({...$state.snapshot(settings),onboarding_complete:true});}
  async function reload(){try{settings=await invoke<DashboardSettings>('get_monitoring_settings');applied=structuredClone($state.snapshot(settings));dirty=false;conflict=false;error='';notice='';}catch(e){fail(e);}}
  function ensureLocal(kind:AgentKind){if(!settings)return;let s=localRows.find(s=>s.kind===kind&&s.id===activeLocalSource)??settings.sources.find(s=>s.kind===kind);if(!s){s=createSource(kind,'',localAgents.find(agent=>agent.kind===kind)?.name);settings.sources.push(s);}return s;}
  function ensureRemote(group:ServerGroup){if(!settings)return;let s=group.codex;if(!s){s=createSource('remote_codex',group.host,group.name);settings.sources.push(s);}return s;}
  function editable(){if(dirty){error='请先应用或取消当前编辑。';return false;}return !saving&&!conflict;}
  async function toggleLocal(kind:AgentKind,enabled:boolean){
    if(!settings||saving||conflict)return;const source=ensureLocal(kind);if(!source)return;
    if(!applied?.onboarding_complete||!applied.sources.some(s=>s.id===source.id)){source.enabled=enabled;touch();return;}
    const previous=source.enabled;saving=true;error='';
    try{const expected=structuredClone($state.snapshot(applied)),proposal=structuredClone(expected);proposal.sources.find(s=>s.id===source.id)!.enabled=enabled;
      const next=await invoke<DashboardSettings>('save_monitoring_settings',{settings:proposal,expected});applied=structuredClone(next);source.enabled=enabled;if(!dirty)settings=structuredClone(next);notice='';
    }catch(e){source.enabled=previous;fail(e);}finally{saving=false;}
  }
  async function toggleCodex(group:ServerGroup,enabled:boolean){
    if(!settings||group.paused||saving||conflict)return;const source=ensureRemote(group);if(!source)return;
    source.enabled=enabled;settings.ssh_paused_sources=(settings.ssh_paused_sources??[]).filter(id=>id!==source.id);touch();
  }
  async function pause(group:ServerGroup){if(!settings||!editable())return;if(group.paused||group.sources.some(s=>s.enabled)){await commit(pauseServer($state.snapshot(settings),group.host,!group.paused));}else{const source=ensureRemote(group);if(source){source.enabled=true;touch();await save();}}}
  async function remove(){if(!settings||!selected)return;const next=removeServer($state.snapshot(settings),selected.host);if(await commit(next)){removePrompt=false;page='home';}}
  function list(value:string){return value.split('\n').map(s=>s.trim()).filter(Boolean);}
  function changeProjects(value:string[]){if(!settings)return;const source=editor??(page==='local'?ensureLocal(activeLocal):selected?ensureRemote(selected):undefined);if(source){source.projects=value;touch();}}
  async function detect(){try{discovery=await invoke<SourceDiscovery[]>('discover_sources');notice='';}catch(e){fail(e);}}
  async function probeHost(group:ServerGroup){
    if(probing[group.host])return;probing[group.host]=true;error='';
    try{const result=await invoke<SshProbe>('probe_ssh_source',{host:group.host});if(active&&result.host===group.host)reports[group.host]=result;}
    catch(e){if(active)fail(e);}finally{if(active)probing[group.host]=false;}
  }
  async function readLocal(source:MonitorSource){
    if(reading||dirty)return;reading=source.id;error='';
    try{const since=await invoke<number>('request_refresh');for(let i=0;i<50;i++){await new Promise(r=>setTimeout(r,300));if(!active)return;const states=await invoke<DashboardSnapshot>('get_dashboard_snapshot');updateSnapshot(states);const result=states.providers.find(p=>p.provider===source.id);if((result?.last_attempt??0)>=since){notice=result?.status==='connected'?'读取完成，请查看来源状态':'读取失败，已保留上次状态';return;}}notice='读取未完成，已保留上次状态';}
    catch(e){fail(e);}finally{if(active)reading='';}
  }
  async function loadProjects(source?:MonitorSource){
    if(!source)return;try{const choices=await invoke<ProjectChoice[]>('get_project_choices',{sourceId:source.id});if(active){projectAliases[source.id]=choices;projects[source.id]=choices.map(row=>row.path);}}catch(e){fail(e);}
  }
  function openServer(group:ServerGroup){page='server';selectedHost=group.host;void loadProjects(group.codex);}
  function openLocal(kind:AgentKind=activeLocal){page='local';activeLocal=kind;if(expandedLocal!==kind)expandedLocal=null;void loadProjects(localEditor);}
  async function configureLocal(kind:AgentKind,id:string){activeLocal=kind;activeLocalSource=id;const existing=localEditor;const source=ensureLocal(kind);if(!source)return;activeLocalSource=source.id;if(!existing)touch();expandedLocal=kind;void loadProjects(source);await tick();document.querySelector(`[data-agent-kind="${kind}"]`)?.scrollIntoView({block:'start'});}
  function addWizard(){if(!editable())return;wizard=true;error='';}
  async function addServer(value:{host:string;name:string;enabled:boolean;projects:string[];report:SshProbe}){
    if(!settings||groups.some(s=>s.host===value.host))return false;
    const next=structuredClone($state.snapshot(settings)),source=createSource('remote_codex',value.host,value.name);source.enabled=value.enabled;source.projects=value.projects;next.sources.push(source);next.onboarding_complete=true;
    if(await commit(next)){reports[value.host]=value.report;wizard=false;selectedHost=value.host;page='server';return true;}return false;
  }
  function updateSnapshot(value:DashboardSnapshot){if(value.revision<revision)return;revision=value.revision;snapshot=value;}
  function trap(dialog:HTMLDialogElement){const previous=document.activeElement as HTMLElement|null;dialog.querySelector<HTMLElement>('button')?.focus();const key=(e:KeyboardEvent)=>{if(e.key==='Escape'){e.preventDefault();closePrompt=false;removePrompt=false;eventPrompt=null;}if(e.key==='Tab'){const items=[...dialog.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')];if(e.shiftKey&&document.activeElement===items[0]){e.preventDefault();items.at(-1)?.focus();}else if(!e.shiftKey&&document.activeElement===items.at(-1)){e.preventDefault();items[0]?.focus();}}};dialog.addEventListener('keydown',key);return{destroy(){dialog.removeEventListener('keydown',key);if(previous?.isConnected)previous.focus();}};}
  async function saveClose(){await save();if(!dirty&&!error){closing=true;await getCurrentWindow().close();}}
  async function discardClose(){closing=true;await getCurrentWindow().close();}
  async function confirmHook(){const id=eventPrompt;eventPrompt=null;if(!id)return;saving=true;try{notice=await invoke<string>('configure_claude_events',{sourceId:id.id,enabled:id.enabled,confirmed:true});}catch(e){fail(e);}finally{saving=false;}}
  onMount(()=>{
    if(!isTauri()){error='请在 TaskPulse 桌面应用中打开监控来源。';return;}
    active=true;const cleanup:Array<()=>void>=[];
    void(async()=>{
      try{
        const close=await getCurrentWindow().onCloseRequested(e=>{if(get(autostart).pending||get(preferencePending)){e.preventDefault();return;}if((dirty||saving||wizard)&&!closing){e.preventDefault();onAttention();closePrompt=true;}});if(active)cleanup.push(close);else close();
        settings=await invoke<DashboardSettings>('get_monitoring_settings');applied=structuredClone($state.snapshot(settings));error=await invoke<string|null>('get_monitoring_error')??'';
        const environment=await invoke<{platform:string}>('integration_environment');platform=environment.platform==='windows'?'Windows':environment.platform==='macos'?'macOS':'Linux';
        const catalog=await invoke<SourceDefinition[]>('get_source_catalog');
        localAgents=catalog.filter(agent=>agent.category==='local'&&(!agent.platforms.length||agent.platforms.includes(environment.platform)));
        eventAgents=catalog.filter(agent=>agent.kind==='generic_event'&&(!agent.platforms.length||agent.platforms.includes(environment.platform)||settings?.sources.some(s=>s.integration?.adapter_id===agent.adapter_id)));
        const stop=await listen<DashboardSnapshot>('dashboard-snapshot-updated',e=>updateSnapshot(e.payload));if(active)cleanup.push(stop);else stop();
        updateSnapshot(await invoke<DashboardSnapshot>('get_dashboard_snapshot'));
        try{discovery=await invoke<SourceDiscovery[]>('discover_sources');if(settings&&!settings.onboarding_complete&&!dirty){for(const s of settings.sources)if(localAgents.some(agent=>agent.kind===s.kind))s.enabled=s.kind!=='trae'&&discovery.some(d=>d.id===s.id&&d.data_location_detected===true);dirty=true;}}catch(e){fail(e);}finally{detectionSettled=true;}
        const changed=await listen<DashboardSettings>('monitoring-settings-updated',e=>{if(saving||!settings||!applied)return;if(dirty){const result=reconcileSourceDraft($state.snapshot(settings),$state.snapshot(applied),e.payload);if(result.conflict){conflict=true;error='监控配置已在其他窗口更新，请加载最新配置。';return;}settings=result.settings;applied=structuredClone(e.payload);}else{settings=structuredClone(e.payload);applied=structuredClone(e.payload);}});if(active)cleanup.push(changed);else changed();
        const update=await listen<{phase:string}>('application-update',e=>updating=['downloading','verifying','installing'].includes(e.payload.phase));if(active)cleanup.push(update);else update();
        updating=['downloading','verifying','installing'].includes((await invoke<{phase:string}>('get_update_status')).phase);
      }catch(e){fail(e);}
    })();return()=>{active=false;cleanup.forEach(fn=>fn());};
  });
</script>

<div class="monitor-sources">
  <header class="ms-head"><h1>{$t(page==='server'?'服务器设置':'监控来源')}</h1>{#if page!=='local'}<button class="ms-btn" class:primary={page==='home'} disabled={saving||updating} onclick={addWizard}><Plus size={14}/>{$t('添加 SSH 服务器')}</button>{/if}</header>
  <div class="ms-tabsbar">{#if page!=='server'}<div class="ms-tabs">{#each [['local','本机'],['ssh','SSH 服务器']] as [id,label]}<button class:active={page==='local'?id==='local':tab===id} onclick={()=>{tab=id as typeof tab;if(id==='local')openLocal();else page='home';}} aria-pressed={page==='local'?id==='local':tab===id}>{$t(label)}{#if id==='ssh'}<small>{groups.length}</small>{/if}</button>{/each}</div>{:else}<button class="ms-back" onclick={()=>{page='home';tab='ssh';}}><ArrowLeft size={13}/>{$t('返回来源列表')}</button>{/if}</div>
  <div class="ms-scroll glass-scroll">
    {#if error}<div class="ms-error" role="alert">{$t(error)}{#if conflict}<button class="ms-btn small" onclick={reload}>{$t('加载最新配置')}</button>{:else}<button class="ms-btn icon-only ghost" aria-label={$t('关闭错误')} onclick={()=>error=''}><X size={13}/></button>{/if}</div>{/if}
    {#if notice}<p class="ms-success" role="status">{$t(notice)}</p>{/if}
    {#if !settings}<p class="ms-muted">{$t('正在读取设置…')}</p>
    {:else if page==='home'}
      {#if tab==='ssh'}<div class="summary-grid"><div><small><Server size={13}/>{$t('已添加服务器')}</small><strong>{groups.length}<span>{$t('台 SSH 主机')}</span></strong></div><div><small><Link size={13}/>{$t('SSH 在线')}</small><strong>{groups.filter(g=>connection(g)==='connected').length}<span>{$t('台最近连接正常')}</span></strong></div><div class="warn"><small><Info size={13}/>{$t('需要处理')}</small><strong>{groups.filter(g=>connection(g)==='disconnected').length}<span>{$t('台连接异常')}</span></strong></div></div>{/if}
      {#if tab!=='local'}<div class="section-top"><h2><Server size={16}/>{$t('SSH 服务器')}<span class="count-badge">{groups.length}</span></h2></div><div class="tool-row"><label class="server-search"><Search size={15}/><input bind:value={query} aria-label={$t('搜索服务器')} placeholder={$t('搜索服务器名称或 SSH Host')}/></label><button class="ms-btn small" disabled={saving} onclick={()=>void invoke('request_refresh')}><RefreshCw size={13}/>{$t('刷新状态')}</button></div><div class="server-list">{#each shown as group (group.host)}<article class="server-card" data-host={group.host}><span class="source-symbol remote"><Server size={19}/></span><div class="source-grow"><div class="server-head"><strong>{group.name}</strong><small>{group.host}</small></div><p class="server-description">{$t(group.paused?'采集已暂停，原启用状态已保留':sourceStatus(group.codex,observation(group.codex)))}</p><div class="agent-tags"><span class="mini-tag">Codex · {$t(group.codex?.enabled?'已启用':'未启用')}</span><span class="mini-tag pending">Claude Code · {$t('待适配')}</span><span class="mini-tag">{$t(scopeLabel(group.codex?.projects??[]))}{#if group.codex&&!group.codex.projects.includes('*')} {group.codex.projects.length}{/if}</span></div></div><div class="server-controls"><span class="ms-pill" class:warning={connection(group)==='disconnected'} class:neutral={connection(group)==='unknown'}><i></i>{$t(sshLabel(group))}</span><span class="metric"><b>{serverTaskCount(group)}</b> {$t('任务')}</span><button class="ms-btn small" onclick={()=>openServer(group)}>{$t('管理')}<ChevronRight size={13}/></button></div></article>{/each}{#if !shown.length}<div class="ms-empty"><Server size={25}/><strong>{$t(groups.length?'没有符合条件的服务器':'还没有添加 SSH 服务器')}</strong><button class="ms-btn" onclick={addWizard}><Plus size={13}/>{$t('添加服务器')}</button></div>{/if}</div>{/if}
    {:else if page==='local'}
      <div class="section-top"><h2>{$t('本机 Agent')}</h2><button class="ms-btn small" onclick={detect}><RefreshCw size={13}/>{$t('重新检测')}</button></div>
      {#snippet localAgentRow(agent:SourceDefinition)}
          {@const source=localRows.find(s=>s.kind===agent.kind&&s.id===activeLocalSource)??localRows.find(s=>s.kind===agent.kind)}
          <div class="agent-entry" data-agent-kind={agent.kind}>
<div class="agent-row"><span class="agent-icon"><Terminal size={18}/></span><div class="source-grow"><strong>{$t(agent.name)}</strong><small>{$t(rowStatus(agent.kind,source))}{#if agent.kind!=='trae'} · {taskCount(source)} {$t('任务')}{/if}{#if hookCount(agent.kind)} · {$t('{count} 个 Hook 连接',{count:hookCount(agent.kind)})}{/if}</small></div><button class="ms-btn small" class:chosen={expandedLocal===agent.kind} aria-expanded={expandedLocal===agent.kind} onclick={()=>{if(expandedLocal===agent.kind)expandedLocal=null;else void configureLocal(agent.kind,source?.id??'');}}>{$t(expandedLocal===agent.kind?'收起':'配置')}</button><input class="settings-switch" type="checkbox" role="switch" aria-label={$t(agent.kind==='trae'?'读取 Trae 会话目录':'启用 {name}',{name:$t(agent.name)})} checked={source?.enabled??false} disabled={saving||conflict} onchange={async e=>{const control=e.currentTarget;await toggleLocal(agent.kind,control.checked);control.checked=(localRows.find(s=>s.kind===agent.kind&&s.id===activeLocalSource)??localRows.find(s=>s.kind===agent.kind))?.enabled??false;}}/></div>
            {#if expandedLocal===agent.kind}
              {#if agent.kind==='trae'}<p class="ms-muted">{$t('Trae 的任务库尚不能直接读取。可在已发现会话中查看目录，任务进度与提醒暂不可用。')}</p>{/if}
              {#if agent.kind==='claude_code' && observation(source)?.data_detected===false}<p class="ms-muted">{$t('Claude Code 有新活动后会自动更新，旧会话不会当作运行中的任务。')}</p>{/if}
                    <section class="agent-config" data-source-id={localEditor?.id??''}><h3>{$t('项目范围')}</h3>{#if localRows.filter(s=>s.kind===activeLocal).length>1}<label class="ms-field">{$t('数据来源实例')}<select value={localEditor?.id} onchange={e=>{activeLocalSource=e.currentTarget.value;void loadProjects(localRows.find(s=>s.id===activeLocalSource));}}>{#each localRows.filter(s=>s.kind===activeLocal) as s}<option value={s.id}>{s.name} · {s.location||$t('默认位置')}</option>{/each}</select></label>{/if}{#key localEditor?.id??activeLocal}<ProjectScope projects={localEditor?.projects??['*']} choices={localEditor?projectChoices(localEditor,snapshot?.tasks??[],projects[localEditor.id]??[]):[]} aliases={localEditor?projectAliases[localEditor.id]??[]:[]} onchange={changeProjects}/>{/key}
        <details class="ms-advanced"><summary><Settings2 size={13}/>{$t('高级设置：路径、排除规则与诊断')}</summary><div class="advanced-inner">{#if localEditor}<label class="ms-field">{$t('数据路径')}<input value={localEditor.kind==='codex'?'CODEX_HOME / ~/.codex':localEditor.location} placeholder={localEditor.kind==='claude_code'?'CLAUDE_CONFIG_DIR / ~/.claude':$t('默认位置')} readonly={localAgents.find(agent=>agent.kind===activeLocal)?.location_kind==='none'} oninput={e=>{if(localEditor){localEditor.location=e.currentTarget.value;touch();}}}/></label><label class="ms-field">{$t('排除规则，每行一项')}<textarea rows="3" value={localEditor.excluded_projects.join('\n')} oninput={e=>{if(localEditor){localEditor.excluded_projects=list(e.currentTarget.value);touch();}}}></textarea></label><button class="ms-btn" onclick={()=>sessionSource=localEditor??null}>{$t('已发现会话')}</button><button class="ms-btn" disabled={!!reading||dirty||!localEditor.enabled} onclick={()=>localEditor&&readLocal(localEditor)}><RefreshCw size={13}/>{$t(reading?'读取中…':'检测数据读取')}</button>{#if activeLocal==='claude_code'}<p class="ms-warning">{$t('本机会话采用只读观测；明确审批需单独确认官方 Hook。原生提问尚未支持。')}</p><button class="ms-btn" disabled={dirty||saving} onclick={()=>eventPrompt=localEditor?{id:localEditor.id,enabled:true}:null}>{$t('配置 Claude 精确事件')}</button><button class="ms-btn" disabled={dirty||saving} onclick={()=>eventPrompt=localEditor?{id:localEditor.id,enabled:false}:null}>{$t('移除 Claude 精确事件')}</button>{/if}<p class="ms-diagnostic">{$t(discovery.find(d=>d.id===localEditor?.id)?.detail??'尚未检测')}</p>{:else}<p class="ms-muted">{$t('启用后可配置数据读取。')}</p>{/if}</div></details>
      </section>
              {#if settings && hookCatalog(agent.kind).length}<EventSources {settings} catalog={hookCatalog(agent.kind)} observations={snapshot?.view.sources??[]} {dirty} disabled={saving||conflict} changed={touch} localHook/>{/if}
            {/if}
          </div>
      {/snippet}
      <section class="detail-card local-agents">
        {#each localAgents.filter(agent=>agent.kind!=='scanner') as agent (agent.id)}
          {@render localAgentRow(agent)}
        {/each}
<div class="auto-discover"><span>{$t('自动发现本机 Agent')}</span><input class="settings-switch" type="checkbox" role="switch" checked={$appSettings?.auto_discover_local??false} disabled={$preferencePending} aria-label={$t('自动发现本机 Agent')} onchange={e=>void savePreference({auto_discover_local:e.currentTarget.checked},e.currentTarget)}/></div>
      </section>
      {#if processAgent}
        <details class="detail-card process-discovery" data-process-discovery>
          <summary>{$t('高级：进程检测')}</summary>
          <p class="ms-muted">{$t('通过进程和终端窗口识别 Gemini CLI 等工具的活动。无法确认审批、提问或任务完成。')}</p>
          {@render localAgentRow(processAgent)}
        </details>
      {/if}
      <EventSources {settings} catalog={manualEventAgents} observations={snapshot?.view.sources??[]} {dirty} disabled={saving||conflict} changed={touch}/>
    {:else if selected}
      {@const source=selected.codex}
      <div class="detail-heading"><div><h2>{selected.name}</h2><p>SSH · {selected.host}</p></div><span class="ms-pill" class:neutral={connection(selected)==='unknown'} class:warning={connection(selected)==='disconnected'}>{$t(sshLabel(selected))}</span></div>
      <div class="detail-content"><section class="detail-card soft"><h3>{$t('SSH 连接')}</h3><div class="connection-facts"><div><span>{$t('SSH Host')}</span><strong>{selected.host}</strong></div><div><span>{$t('最近检测')}</span><strong>{reports[selected.host]?$t(formatAgo(reports[selected.host].checked_at)):$t('尚未检测')}</strong></div><div><span>{$t('数据读取')}</span><strong>{$t(sourceStatus(selected.codex,observation(selected.codex)))}</strong></div><div><span>{$t('当前任务')}</span><strong>{serverTaskCount(selected)}</strong></div></div><div class="inline-actions"><button class="ms-btn" disabled={probing[selected.host]} onclick={()=>selected&&probeHost(selected)}><RefreshCw size={13}/>{$t(probing[selected.host]?'检测中…':'检测 SSH 与数据')}</button><button class="ms-btn" disabled={saving||dirty||conflict} onclick={()=>selected&&pause(selected)}>{#if selected.paused||!selected.sources.some(s=>s.enabled)}<Play size={13}/>{:else}<Pause size={13}/>{/if}{$t(selected.paused?'恢复采集':selected.sources.some(s=>s.enabled)?'暂停采集':'启用采集')}</button></div>{#if reports[selected.host]}<p class="ms-muted">{$t(reports[selected.host].message)} · Codex: {$t(reports[selected.host].codex.state==='readable'?'数据可读取':reports[selected.host].codex.state==='empty'?'目录可读取，暂无会话':reports[selected.host].codex.state==='missing'?'未发现数据目录':'数据读取失败')}</p>{/if}</section>
      <section class="detail-card"><h3>{$t('Agent')}</h3><div class="agent-row"><span class="agent-icon"><Terminal size={18}/></span><div class="source-grow"><strong>Codex</strong><small>{$t(sourceStatus(selected.codex,observation(selected.codex)))}</small></div><span class="cap-tag">{$t('SSH 已支持')}</span><input class="settings-switch" type="checkbox" role="switch" aria-label={$t('启用远端 Codex')} checked={selected.codex?.enabled??false} disabled={selected.paused||saving||conflict} onchange={e=>selected&&toggleCodex(selected,e.currentTarget.checked)}/></div><div class="agent-row unsupported"><span class="agent-icon"><Terminal size={18}/></span><div class="source-grow"><strong>Claude Code</strong><small>{$t('当前尚不能读取远端 Claude 会话')}</small></div><span class="cap-tag planned">{$t('SSH 待适配')}</span><input class="settings-switch" type="checkbox" role="switch" aria-label={$t('启用远端 Claude Code')} disabled/></div></section>
      <section class="detail-card"><h3>{$t('项目范围')}<small>Codex</small></h3>{#key selected.host}<ProjectScope projects={selected.codex?.projects??['*']} choices={selected.codex?projectChoices(selected.codex,snapshot?.tasks??[],[...(projects[selected.codex.id]??[]),...(reports[selected.host]?.codex.projects??[])]):reports[selected.host]?.codex.projects??[]} aliases={source?projectAliases[source.id]??[]:[]} onchange={changeProjects}/>{/key}<details class="ms-advanced"><summary><Settings2 size={13}/>{$t('高级设置：轮询、路径与诊断')}</summary><div class="advanced-inner"><label class="ms-field">{$t('服务器名称')}<input value={source?.name??selected.name} oninput={e=>{if(selected){const s=ensureRemote(selected);if(s){s.name=e.currentTarget.value;touch();}}}}/></label><label class="ms-field">{$t('轮询间隔（毫秒）')}<input type="number" min="500" max="60000" value={source?.poll_interval_ms??3000} oninput={e=>{if(selected){const s=ensureRemote(selected);if(s){s.poll_interval_ms=Number(e.currentTarget.value);touch();}}}}/></label><label class="ms-field">{$t('远端数据路径')}<input value="~/.codex" readonly/></label><p class="ms-muted">{$t('沿用现有远端采集路径；当前不提供路径覆盖。')}</p><label class="ms-field">{$t('排除规则，每行一项')}<textarea rows="3" value={source?.excluded_projects.join('\n')??''} oninput={e=>{if(selected){const s=ensureRemote(selected);if(s){s.excluded_projects=list(e.currentTarget.value);touch();}}}}></textarea></label>{#if selected.sources.some(s=>s.kind==='tmux')}<p class="ms-muted">{$t('已有任务运行方式过滤规则已保留，暂停会同时暂停该服务器的兼容采集。')}</p>{/if}{#if source}<button class="ms-btn" onclick={()=>sessionSource=source}>{$t('已发现会话')}</button>{/if}<p class="ms-diagnostic">{snapshot?.providers.find(p=>p.provider===source?.id)?.message??$t('尚无采集诊断')} · {$t('读取异常会保留上次状态，不会当作任务结束。')}</p></div></details></section></div>
    {/if}
  </div>
  <footer class="ms-foot" class:idle={!dirty&&page!=='server'}><span>{#if dirty}<ShieldCheck size={14}/>{$t('有未保存的配置更改')}{/if}</span><div>{#if dirty}<button class="ms-btn ghost" disabled={saving} onclick={reload}>{$t('取消更改')}</button><button class="ms-btn primary" disabled={saving||conflict||updating||!detectionSettled} onclick={save}><Save size={13}/>{$t(saving?'正在保存…':settings&&!settings.onboarding_complete?'确认并开始监控':'应用更改')}</button>{:else if page==='server'}<button class="ms-btn danger" disabled={saving} onclick={()=>removePrompt=true}><Trash2 size={13}/>{$t('移除服务器')}</button>{/if}</div></footer>
  {#if sessionSource}<SourceSessions source={sessionSource} ondismiss={()=>sessionSource=null}/>{/if}
  {#if wizard}<SshSourceWizard existing={groups.map(s=>s.host)} onadd={addServer} oncancel={()=>wizard=false}/>{/if}
  {#if closePrompt||removePrompt||eventPrompt}<div class="ms-overlay"><dialog open aria-modal="true" class="ms-modal confirm-modal" use:trap aria-labelledby="source-confirm-title"><header class="modal-top"><h2 id="source-confirm-title">{$t(closePrompt?'还有未保存的配置':removePrompt?'移除服务器':'配置 Claude 精确事件')}</h2></header><div class="modal-body"><p>{$t(closePrompt?'关闭窗口前，请处理未保存的编辑。':removePrompt?'移除会停止此服务器的采集，并按现有范围规则移出其任务；不会修改远端文件。':'这会修改所选 Claude 配置。已有 Hook、代理和模型设置会保留，首次修改会备份。')}</p></div><footer class="modal-footer">{#if closePrompt}<button class="ms-btn" onclick={()=>closePrompt=false}>{$t('继续编辑')}</button><button class="ms-btn" onclick={discardClose}>{$t('放弃未保存编辑')}</button><button class="ms-btn primary" disabled={saving||conflict||wizard} onclick={saveClose}>{$t('保存并关闭')}</button>{:else if removePrompt}<button class="ms-btn" onclick={()=>removePrompt=false}>{$t('取消')}</button><button class="ms-btn danger" disabled={saving} onclick={remove}>{$t('确认移除')}</button>{:else}<button class="ms-btn" onclick={()=>eventPrompt=null}>{$t('取消')}</button><button class="ms-btn primary" onclick={confirmHook}>{$t('确认')}</button>{/if}</footer></dialog></div>{/if}
</div>
