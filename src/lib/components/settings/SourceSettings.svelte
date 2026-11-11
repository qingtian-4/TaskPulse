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
  import {servers,createSource,pauseServer,removeServer,projectChoices,sourceStatus,localSourceStatus,sshAgents,readLabel,canRead,type AgentKind,type NewServer,type SshAgent,type SshProbe,type ServerGroup} from '$lib/monitor-sources';
  import type {DashboardSettings,MonitorSource,SourceDiscovery,DashboardSnapshot,SessionPage,SourceSnapshot} from '$lib/types';
  import type {ProjectChoice} from '$lib/project-options';
  import {configuredSource,localDiscoveryCandidates,localHookKind,sourceDefinitionFor,type SourceDefinition} from '$lib/source-setup';
  import SshSourceWizard from './SshSourceWizard.svelte';
  import ProjectScope from './ProjectScope.svelte';
  import SourceSessions from './SourceSessions.svelte';
  import EventSources from './EventSources.svelte';
  import './monitor-sources.css';
  let {reviewRequest,onDirtyChange,onAttention}:{reviewRequest:number;onDirtyChange:(value:boolean)=>void;onAttention:()=>void}=$props();
  let settings=$state<DashboardSettings|null>(null),applied=$state<DashboardSettings|null>(null);
  let snapshot=$state<DashboardSnapshot|null>(null),discovery=$state<SourceDiscovery[]>([]);
  let page=$state<'home'|'local'|'server'>('local'),tab=$state<'local'|'ssh'>('local'),selectedHost=$state(''),query=$state('');
  let dirty=$state(false),saving=$state(false),conflict=$state(false),updating=$state(false),detectionSettled=$state(false);
  let error=$state(''),notice=$state(''),platform=$state('Windows'),wizard=$state(false),closePrompt=$state(false),closing=false;
  let removePrompt=$state(false),eventPrompt=$state<{id:string;enabled:boolean}|null>(null),reading=$state('');
  let reports=$state<Record<string,SshProbe>>({}),probing=$state<Record<string,boolean>>({}),projects=$state<Record<string,string[]>>({});
  let projectAliases=$state<Record<string,ProjectChoice[]>>({});
  const projectRequests=new Map<string,number>();
  let activeRemote=$state<SshAgent>('codex');
  let activeLocal=$state<AgentKind>('codex'),activeLocalSource=$state('');
  let expandedLocal=$state<AgentKind|null>(null);
  let localAgents=$state<SourceDefinition[]>([]);
  let eventAgents=$state<SourceDefinition[]>([]);
  const discoveredLocal=$derived(localDiscoveryCandidates(localAgents,discovery,applied));
  const knownLocal=$derived(localAgents.filter(agent=>agent.kind!=='scanner'&&!discoveredLocal.some(row=>row.definition.kind===agent.kind)));
  const featuredLocal=$derived(knownLocal.filter(agent=>discovery.some(row=>row.data_location_detected===true
    && (row.candidate?.kind===agent.kind||settings?.sources.some(s=>s.kind===agent.kind&&s.id===row.id)))
    || settings?.sources.some(s=>s.kind===agent.kind&&configuredSource(s,agent,settings?.auto_discovery_seen))));
  const optionalLocal=$derived(knownLocal.filter(agent=>!featuredLocal.includes(agent)));
  let supportedOpen=$state(false);
  let catalog=$state<SourceDefinition[]>([]);
  const wslSources=$derived(settings?.sources.filter(s=>s.kind==='codex_wsl'||s.kind==='deepseek_wsl')??[]);
  const fallbackSources=$derived(settings?.sources.filter(s=>
    !wslSources.includes(s)&&s.kind!=='remote_codex'&&s.kind!=='remote_claude_code'&&s.kind!=='tmux'&&s.kind!=='generic_event'&&s.kind!=='doubao'
    &&(s.kind==='scanner'||!localAgents.some(agent=>agent.kind===s.kind))
    &&configuredSource(s,sourceDefinitionFor(catalog,s),settings?.auto_discovery_seen))??[]);
  const fallbackEvents=$derived(settings?.sources.filter(s=>{
    if(s.kind!=='generic_event')return false;
    const local=localHookKind(s.integration?.adapter_id);
    return !local||!localAgents.some(agent=>agent.kind===local)||!hookCatalog(local).some(d=>d.adapter_id===s.integration?.adapter_id);
  })??[]);
  let sourceScroll:HTMLDivElement;
  let handledReview=0, detectionRequest=0, catalogRequest=0;
  async function refreshCatalog(currentPlatform:string){
    const request=++catalogRequest;
    const entries=await invoke<SourceDefinition[]>('get_source_catalog');
    if(!active||request!==catalogRequest)return;
    catalog=entries;
    localAgents=entries.filter(agent=>agent.category==='local'&&(!agent.platforms.length||agent.platforms.includes(currentPlatform)));
    eventAgents=entries.filter(agent=>agent.kind==='generic_event'&&(!agent.platforms.length||agent.platforms.includes(currentPlatform)||settings?.sources.some(s=>s.integration?.adapter_id===agent.adapter_id)));
  }
  function patchSource(source:MonitorSource,patch:Partial<MonitorSource>){
    if(saving||conflict)return;
    Object.assign(source,patch);touch();
  }
  let legacyRemove=$state<string|null>(null);
  function removeLegacy(source:MonitorSource){
    if(!settings||saving||conflict)return;
    settings.sources=settings.sources.filter(item=>item.id!==source.id);
    settings.ssh_paused_sources=(settings.ssh_paused_sources??[]).filter(id=>id!==source.id);
    legacyRemove=null;touch();
  }

  $effect(()=>{
    if(reviewRequest===handledReview)return;
    handledReview=reviewRequest;
    page='local';tab='local';
    void tick().then(()=>sourceScroll?.scrollTo({top:0}));
    if(detectionSettled)void detect();
  });

  function hookCatalog(kind:AgentKind){return eventAgents.filter(agent=>localHookKind(agent.adapter_id)===kind);}
  function hookCount(kind:AgentKind){return settings?.sources.filter(s=>s.kind==='generic_event'&&localHookKind(s.integration?.adapter_id)===kind).length??0;}
  let active=true,revision=0;
  let sessionSource=$state<MonitorSource|null>(null);
  const groups=$derived(settings?servers(settings):[]);
  const selected=$derived(groups.find(s=>s.host===selectedHost));
  $effect(()=>{
    if(settings&&page==='server'&&!selected){
      page='home';tab='ssh';
    }
  });
  const shown=$derived(groups.filter(s=>`${s.host} ${s.name}`.toLowerCase().includes(query.toLowerCase())));
  const localRows=$derived(settings?.sources.filter(s=>localAgents.some(agent=>agent.kind===s.kind))??[]);
  const localEditor=$derived(localRows.find(s=>s.kind===activeLocal&&s.id===activeLocalSource)??localRows.find(s=>s.kind===activeLocal));
  const editor=$derived(page==='server'?selected?.agents[activeRemote]:localEditor);
  const changed=$derived(dirty||saving||conflict||wizard);
  $effect(()=>onDirtyChange(changed));
  function observation(s?:MonitorSource):SourceSnapshot|undefined{return snapshot?.view.sources.find(v=>v.id===s?.id);}
  function taskCount(s?:MonitorSource){return s?.enabled?(observation(s)?.current_tasks??0):0;}
  function serverTaskCount(group:ServerGroup){return group.sources.reduce((total,source)=>total+taskCount(source),0);}
  function localDetection(kind:AgentKind,source?:MonitorSource){
    return discovery.find(d=>source ? d.id===source.id : d.candidate?.kind===kind)?.data_location_detected;
  }
  function connection(group:ServerGroup):string {
    const report=reports[group.host];
    const claude=snapshot?.providers.find(p=>p.provider===group.agents.claude_code?.id&&group.agents.claude_code?.enabled);
    if(report&&Date.now()-report.checked_at<60000&&report.checked_at>=(claude?.last_attempt??0))return report.connection;
    if(claude?.last_attempt&&Date.now()-claude.last_attempt<30000){
      if(claude.message==='远端 Claude Code 的 SSH 连接失败')return 'disconnected';
      if(claude.status==='connected'||claude.message==='未找到远端 Claude Code 会话目录'||claude.message==='远端 Claude Code 记录未读完整，已保留上次状态')return 'connected';
    }
    const host=snapshot?.hosts.find(h=>h.host===group.host);
    if(host?.status==='disconnected')return 'disconnected';
    if(host?.last_seen&&Date.now()-host.last_seen<30000)return host.status;
    return report&&Date.now()-report.checked_at<60000?report.connection:'unknown';
  }
  function sourceHasIssue(s:MonitorSource){
    return s.enabled&&['failed','partial','stale'].includes(observation(s)?.read_state??'');
  }
  const connectionIssues=$derived(groups.filter(group=>group.sources.some(s=>s.enabled)&&connection(group)==='disconnected').length);
  const dataIssues=$derived(groups.filter(group=>connection(group)!=='disconnected').reduce((n,group)=>n+group.sources.filter(sourceHasIssue).length,0));
  function sshLabel(group:ServerGroup){return ({connected:'SSH 已连接',disconnected:'SSH 未连接',unknown:'SSH 尚未检测'})[connection(group)]??'SSH 尚未检测';}
  function rowStatus(kind:AgentKind,source?:MonitorSource){
    const saved=applied?.sources.find(s=>s.id===source?.id);
    if(source&&(!saved||source.enabled!==saved.enabled||source.enabled&&JSON.stringify(source.projects)!==JSON.stringify(saved.projects)))return '待保存';
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
  async function reload(){if(saving)return;saving=true;try{settings=await invoke<DashboardSettings>('get_monitoring_settings');applied=structuredClone($state.snapshot(settings));dirty=false;conflict=false;error='';notice='';}catch(e){fail(e);}finally{saving=false;}}
  function ensureLocal(kind:AgentKind){if(!settings||saving)return;let s=localRows.find(s=>s.kind===kind&&s.id===activeLocalSource)??settings.sources.find(s=>s.kind===kind);if(!s){const candidate=discoveredLocal.find(row=>row.definition.kind===kind)?.candidate;s=candidate?{...structuredClone($state.snapshot(candidate)),enabled:false}:createSource(kind,'',localAgents.find(agent=>agent.kind===kind)?.name);settings.sources.push(s);}return s;}
  function ensureRemote(group:ServerGroup,key=activeRemote){
    if(!settings||saving)return;
    let source=group.agents[key];
    if(!source){
      const agent=sshAgents.find(agent=>agent.key===key)!;
      source=createSource(agent.kind,group.host,group.name);
      settings.sources.push(source);
      return settings.sources[settings.sources.length-1];
    }
    return source;
  }
  function remoteStatus(group:ServerGroup,key:SshAgent){
    const source=group.agents[key],probe=reports[group.host]?.[key];
    const observed=observation(source);
    if(source?.enabled && observed && observed.read_state!=='waiting')return sourceStatus(source,observed);
    return probe && probe.state!=='not_checked' ? readLabel(probe.state) : sourceStatus(source,observed);
  }
  function editable(){if(dirty){error='请先应用或取消当前编辑。';return false;}return !saving&&!conflict;}
  async function toggleLocal(kind:AgentKind,enabled:boolean){
    if(!settings||saving||conflict)return;const source=ensureLocal(kind);if(!source)return;
    if(!applied?.onboarding_complete||!applied.sources.some(s=>s.id===source.id)){source.enabled=enabled;touch();return;}
    const previous=source.enabled;saving=true;error='';
    try{const expected=structuredClone($state.snapshot(applied)),proposal=structuredClone(expected);proposal.sources.find(s=>s.id===source.id)!.enabled=enabled;
      const next=await invoke<DashboardSettings>('save_monitoring_settings',{settings:proposal,expected});applied=structuredClone(next);source.enabled=enabled;if(!dirty)settings=structuredClone(next);notice='';
    }catch(e){source.enabled=previous;fail(e);}finally{saving=false;}
  }
  function toggleRemote(group:ServerGroup,key:SshAgent,enabled:boolean){
    if(!settings||group.paused||saving||conflict)return;const source=ensureRemote(group,key);if(!source)return;
    activeRemote=key;void loadProjects(source);source.enabled=enabled;settings.ssh_paused_sources=(settings.ssh_paused_sources??[]).filter(id=>id!==source.id);touch();
  }
  async function pause(group:ServerGroup){
    if(!settings||!editable())return;
    await commit(pauseServer($state.snapshot(settings),group.host,!group.paused));
  }
  async function remove(){if(!settings||!selected)return;const next=removeServer($state.snapshot(settings),selected.host);if(await commit(next)){removePrompt=false;page='home';}}
  function list(value:string){return value.split('\n').map(s=>s.trim()).filter(Boolean);}
  function changeProjects(value:string[]){if(!settings||saving||conflict)return;const source=editor??(page==='local'?ensureLocal(activeLocal):selected?ensureRemote(selected):undefined);if(source){source.projects=value;touch();}}
  async function detect(){const request=++detectionRequest;try{const found=await invoke<SourceDiscovery[]>('discover_sources');if(!active||request!==detectionRequest)return;discovery=found;notice='';if(expandedLocal)void loadProjects(localEditor);}catch(e){if(active&&request===detectionRequest)fail(e);}}
  async function probeHost(group:ServerGroup){
    if(probing[group.host])return;probing[group.host]=true;error='';
    try{const result=await invoke<SshProbe>('probe_ssh_source',{host:group.host});if(active&&result.host===group.host)reports[group.host]=result;}
    catch(e){if(active)fail(e);}finally{if(active)probing[group.host]=false;}
  }
  async function readLocal(source:MonitorSource){
    if(reading||dirty)return;reading=source.id;error='';
    try{const since=await invoke<number>('request_refresh');for(let i=0;i<50;i++){await new Promise(r=>setTimeout(r,300));if(!active)return;const states=await invoke<DashboardSnapshot>('get_dashboard_snapshot');updateSnapshot(states);const result=states.providers.find(p=>p.provider===source.id);if((result?.last_attempt??0)>=since){await loadProjects(source);notice=result?.status==='connected'?'读取完成，请查看来源状态':'读取失败，已保留上次状态';return;}}notice='读取未完成，已保留上次状态';}
    catch(e){fail(e);}finally{if(active)reading='';}
  }
  async function loadProjects(source?:MonitorSource){
    if(!source)return;
    const id=source.id,request=(projectRequests.get(id)??0)+1;
    projectRequests.set(id,request);
    try{const choices=await invoke<ProjectChoice[]>('get_project_choices',{sourceId:id});if(active&&projectRequests.get(id)===request){projectAliases[id]=choices;projects[id]=choices.map(row=>row.path);}}
    catch(e){if(active&&projectRequests.get(id)===request)fail(e);}
  }
  function openServer(group:ServerGroup){
    page='server';selectedHost=group.host;
    selectRemote(group,group.agents.claude_code?.enabled&&!group.agents.codex?.enabled?'claude_code':group.agents.codex?'codex':'claude_code');
  }
  function selectRemote(group:ServerGroup,key:SshAgent){activeRemote=key;void loadProjects(group.agents[key]);}
  function openLocal(kind:AgentKind=activeLocal){page='local';activeLocal=kind;if(expandedLocal!==kind)expandedLocal=null;void loadProjects(localEditor);}
  async function configureLocal(kind:AgentKind,id:string){activeLocal=kind;activeLocalSource=id;const existing=localEditor;const source=ensureLocal(kind);if(!source)return;activeLocalSource=source.id;if(!existing)touch();expandedLocal=kind;void loadProjects(source);await tick();document.querySelector(`[data-agent-kind="${kind}"]`)?.scrollIntoView({block:'start'});}
  function addWizard(){if(!editable())return;wizard=true;error='';}
  async function addServer(value:NewServer){
    if(!settings||groups.some(s=>s.host===value.host))return false;
    const next=structuredClone($state.snapshot(settings));
    for(const agent of sshAgents){
      const entry=value.agents[agent.key],enabled=entry.enabled&&canRead(value.report[agent.key]);
      if(agent.key!=='codex'&&!enabled)continue;
      const source=createSource(agent.kind,value.host,value.name);
      source.enabled=enabled;source.projects=entry.projects;next.sources.push(source);
    }
    next.onboarding_complete=true;
    if(await commit(next)){
      reports[value.host]=value.report;wizard=false;
      const group=groups.find(group=>group.host===value.host);
      if(group)openServer(group);
      return true;
    }
    return false;
  }
  function updateSnapshot(value:DashboardSnapshot){
    if(value.revision<revision)return;
    const previous=snapshot;
    revision=value.revision;snapshot=value;
    const source=page==='local'&&expandedLocal?localEditor:page==='server'?editor:undefined;
    if(!source?.enabled)return;
    const success=value.providers.find(p=>p.provider===source.id)?.last_success;
    if(!success)return;
    const before=previous?.view.sources.find(row=>row.id===source.id);
    const after=value.view.sources.find(row=>row.id===source.id);
    if(!previous?.providers.find(p=>p.provider===source.id)?.last_success||
        before?.data_detected!==after?.data_detected||
        before?.discovered_sessions!==after?.discovered_sessions)void loadProjects(source);
  }
  function trap(dialog:HTMLDialogElement){const previous=document.activeElement as HTMLElement|null;dialog.querySelector<HTMLElement>('button')?.focus();const key=(e:KeyboardEvent)=>{if(e.key==='Escape'){e.preventDefault();closePrompt=false;removePrompt=false;eventPrompt=null;}if(e.key==='Tab'){const items=[...dialog.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')];if(e.shiftKey&&document.activeElement===items[0]){e.preventDefault();items.at(-1)?.focus();}else if(!e.shiftKey&&document.activeElement===items.at(-1)){e.preventDefault();items[0]?.focus();}}};dialog.addEventListener('keydown',key);return{destroy(){dialog.removeEventListener('keydown',key);if(previous?.isConnected)previous.focus();}};}
  async function saveClose(){await save();if(!dirty&&!error){closing=true;await getCurrentWindow().close();}}
  async function discardClose(){if(saving)return;closing=true;await getCurrentWindow().close();}
  async function confirmHook(){if(saving)return;const id=eventPrompt;eventPrompt=null;if(!id)return;saving=true;try{notice=await invoke<string>('configure_claude_events',{sourceId:id.id,enabled:id.enabled,confirmed:true});}catch(e){fail(e);}finally{saving=false;}}
  onMount(()=>{
    if(!isTauri()){error='请在 TaskPulse 桌面应用中打开监控来源。';return;}
    active=true;const cleanup:Array<()=>void>=[];
    void(async()=>{
      try{
        const close=await getCurrentWindow().onCloseRequested(e=>{if(saving||get(autostart).pending||get(preferencePending)){e.preventDefault();return;}if((dirty||wizard)&&!closing){e.preventDefault();onAttention();closePrompt=true;}});if(active)cleanup.push(close);else close();
        settings=await invoke<DashboardSettings>('get_monitoring_settings');applied=structuredClone($state.snapshot(settings));error=await invoke<string|null>('get_monitoring_error')??'';
        const environment=await invoke<{platform:string}>('integration_environment');platform=environment.platform==='windows'?'Windows':environment.platform==='macos'?'macOS':'Linux';
        const catalogChanged=await listen('integration-catalog-updated',()=>{void refreshCatalog(environment.platform).catch(fail);});
        if(active)cleanup.push(catalogChanged);else catalogChanged();
        await refreshCatalog(environment.platform);
        const stop=await listen<DashboardSnapshot>('dashboard-snapshot-updated',e=>updateSnapshot(e.payload));if(active)cleanup.push(stop);else stop();
        updateSnapshot(await invoke<DashboardSnapshot>('get_dashboard_snapshot'));
        try{discovery=await invoke<SourceDiscovery[]>('discover_sources');if(settings&&!settings.onboarding_complete&&!dirty){for(const s of settings.sources)if(localAgents.some(agent=>agent.kind===s.kind))s.enabled=localAgents.some(agent=>agent.kind===s.kind&&agent.auto_enable_on_discovery)&&discovery.some(d=>d.id===s.id&&d.data_location_detected===true);dirty=true;}}catch(e){fail(e);}finally{detectionSettled=true;}
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
  <div class="ms-scroll glass-scroll" bind:this={sourceScroll}>
    <fieldset class="source-edit-lock" disabled={saving} aria-busy={saving}>
      {#snippet savedSourceRow(source:MonitorSource)}
        <details class="ms-advanced" data-saved-source={source.id} ontoggle={e=>{if(e.currentTarget.open)void loadProjects(source);}}>
          <summary>{source.name}{#if source.location} · {source.location}{/if} · {$t(sourceStatus(source,observation(source)))}</summary>
          <div class="advanced-inner">
            <label class="ms-field">{$t('来源名称')}<input value={source.name} oninput={e=>patchSource(source,{name:e.currentTarget.value})}/></label>
            <label class="ms-field">{$t('启用 {name}',{name:source.name})}<input type="checkbox" role="switch" class="settings-switch" checked={source.enabled} disabled={settings?.ssh_paused_sources?.includes(source.id)} onchange={e=>patchSource(source,{enabled:e.currentTarget.checked})}/></label>
            <ProjectScope projects={source.projects} choices={projectChoices(source,snapshot?.tasks??[],projects[source.id]??[])} aliases={projectAliases[source.id]??[]} disabled={saving||conflict} onchange={projects=>patchSource(source,{projects})}/>
            <label class="ms-field">{$t('排除规则，每行一项')}<textarea rows="2" value={source.excluded_projects.join('\n')} oninput={e=>patchSource(source,{excluded_projects:list(e.currentTarget.value)})}></textarea></label>
            {#if source.kind==='tmux'}
              <label class="ms-field">tmux {$t('会话')}<textarea rows="2" value={source.tmux_sessions.join('\n')} oninput={e=>patchSource(source,{tmux_sessions:list(e.currentTarget.value)})}></textarea></label>
              <label class="ms-field">tmux {$t('窗格')}<textarea rows="2" value={source.tmux_panes.join('\n')} oninput={e=>patchSource(source,{tmux_panes:list(e.currentTarget.value)})}></textarea></label>
            {/if}
            <div class="inline-actions">
              <button class="ms-btn" onclick={()=>sessionSource=source}>{$t('已发现会话')}</button>
              {#if legacyRemove===source.id}<button class="ms-btn danger" onclick={()=>removeLegacy(source)}>{$t('确认移除')}</button><button class="ms-btn" onclick={()=>legacyRemove=null}>{$t('取消')}</button>
              {:else}<button class="ms-btn" onclick={()=>legacyRemove=source.id}>{$t('移除来源')}</button>{/if}
            </div>
          </div>
        </details>
      {/snippet}
    {#if error}<div class="ms-error" role="alert">{$t(error)}{#if conflict}<button class="ms-btn small" onclick={reload}>{$t('加载最新配置')}</button>{:else}<button class="ms-btn icon-only ghost" aria-label={$t('关闭错误')} onclick={()=>error=''}><X size={13}/></button>{/if}</div>{/if}
    {#if notice}<p class="ms-success" role="status">{$t(notice)}</p>{/if}
    {#if !settings}<p class="ms-muted">{$t('正在读取设置…')}</p>
    {:else if page==='home'}
      {#if connectionIssues+dataIssues>0}
        <p class="ms-warning" data-ssh-issues>
          {[
            connectionIssues ? $t('SSH 连接异常：{count}', {count:connectionIssues}) : '',
            dataIssues ? $t('数据读取异常：{count}', {count:dataIssues}) : '',
          ].filter(Boolean).join(' · ')}
        </p>
      {/if}
      {#if groups.length}
        <div class="tool-row">
          <label class="server-search">
            <Search size={15}/>
            <input type="text" bind:value={query} aria-label={$t('搜索服务器')} placeholder={$t('搜索服务器')}/>
          </label>
          <button class="ms-btn small" disabled={saving} onclick={()=>void invoke('request_refresh')}><RefreshCw size={13}/>{$t('刷新状态')}</button>
        </div>
      {/if}
      {#if shown.length}
        <div class="server-list">
          {#each shown as group (group.host)}
            {@const agents=sshAgents.filter(agent=>group.agents[agent.key]?.enabled)}
            {@const tasks=serverTaskCount(group)}
            {@const linkState=connection(group)}
            <article class="server-card" data-host={group.host}>
              <Server class="server-icon" size={18} aria-hidden="true"/>
              <div class="source-grow">
                <strong class="server-name">{group.name}</strong>
                {#if group.name!==group.host || agents.length || group.paused || !group.sources.some(source=>source.enabled)}
                  <div class="server-meta">
                    {#if group.name!==group.host}<span>{group.host}</span>{/if}
                    {#if group.paused}
                      <span>{$t('已暂停')}</span>
                    {:else if agents.length}
                      <span>{agents.map(agent=>agent.name).join(' · ')}</span>
                    {:else if !group.sources.some(source=>source.enabled)}
                      <span>{$t('未启用')}</span>
                    {/if}
                  </div>
                {/if}
              </div>
              <div class="server-controls">
                {#if tasks>0}<span class="metric">{tasks} {$t('任务')}</span>{/if}
                <span class="ms-pill" class:warning={linkState==='disconnected'} class:neutral={linkState==='unknown'}><i></i>{$t(sshLabel(group))}</span>
                <button class="ms-btn small" onclick={()=>openServer(group)}>{$t('管理')}<ChevronRight size={13}/></button>
              </div>
            </article>
          {/each}
        </div>
      {:else}
        <p class="ms-empty">{$t(groups.length?'没有符合条件的服务器':'还没有添加 SSH 服务器')}</p>
      {/if}
    {:else if page==='local'}
      <div class="section-top"><h2>{$t('本机 Agent')}</h2><button class="ms-btn small" onclick={detect}><RefreshCw size={13}/>{$t('重新检测')}</button></div>
      {#snippet localAgentRow(agent:SourceDefinition, newlyFound=false)}
          {@const source=localRows.find(s=>s.kind===agent.kind&&s.id===activeLocalSource)??localRows.find(s=>s.kind===agent.kind)}
          <div class="agent-entry" data-agent-kind={agent.kind}>
<div class="agent-row"><span class="agent-icon"><Terminal size={18}/></span><div class="source-grow"><strong>{$t(agent.name)}</strong><small>{$t(newlyFound&&!source?(agent.kind==='trae'?'已检测到 · 仅支持会话目录':'已检测到 · 未启用'):rowStatus(agent.kind,source))}{#if source?.enabled&&source.projects.length&&agent.kind!=='trae'} · {taskCount(source)} {$t('任务')}{/if}{#if hookCount(agent.kind)} · {$t('{count} 个 Hook 连接',{count:hookCount(agent.kind)})}{/if}</small></div><button class="ms-btn small" class:chosen={expandedLocal===agent.kind} aria-expanded={expandedLocal===agent.kind} onclick={()=>{if(expandedLocal===agent.kind)expandedLocal=null;else void configureLocal(agent.kind,source?.id??'');}}>{$t(expandedLocal===agent.kind?'收起':newlyFound&&!source?'选择添加':source?.enabled&&!source.projects.length?'设置项目范围':'配置')}</button><input class="settings-switch" type="checkbox" role="switch" aria-label={$t(agent.kind==='trae'?'读取 Trae 会话目录':'启用 {name}',{name:$t(agent.name)})} checked={source?.enabled??false} disabled={saving||conflict} onchange={async e=>{const control=e.currentTarget;await toggleLocal(agent.kind,control.checked);control.checked=(localRows.find(s=>s.kind===agent.kind&&s.id===activeLocalSource)??localRows.find(s=>s.kind===agent.kind))?.enabled??false;}}/></div>
            {#if expandedLocal===agent.kind}
                    <section class="agent-config" data-source-id={localEditor?.id??''}><h3>{$t('项目范围')}</h3>{#if localRows.filter(s=>s.kind===activeLocal).length>1}<label class="ms-field">{$t('数据来源实例')}<select value={localEditor?.id} onchange={e=>{activeLocalSource=e.currentTarget.value;void loadProjects(localRows.find(s=>s.id===activeLocalSource));}}>{#each localRows.filter(s=>s.kind===activeLocal) as s}<option value={s.id}>{s.name} · {s.location||$t('默认位置')}</option>{/each}</select></label>{/if}{#key localEditor?.id??activeLocal}<ProjectScope projects={localEditor?.projects??['*']} choices={localEditor?projectChoices(localEditor,snapshot?.tasks??[],projects[localEditor.id]??[]):[]} aliases={localEditor?projectAliases[localEditor.id]??[]:[]} disabled={saving||conflict} onchange={changeProjects}/>{/key}
        <details class="ms-advanced"><summary><Settings2 size={13}/>{$t('数据位置、排除规则与排查')}</summary><div class="advanced-inner">{#if localEditor}<label class="ms-field">{$t('数据路径')}<input value={localEditor.kind==='codex'?'CODEX_HOME / ~/.codex':localEditor.location} placeholder={localEditor.kind==='claude_code'?'CLAUDE_CONFIG_DIR / ~/.claude':$t('默认位置')} readonly={localAgents.find(agent=>agent.kind===activeLocal)?.location_kind==='none'} oninput={e=>{if(!saving&&localEditor){localEditor.location=e.currentTarget.value;touch();}}}/></label><label class="ms-field">{$t('排除规则，每行一项')}<textarea rows="3" value={localEditor.excluded_projects.join('\n')} oninput={e=>{if(!saving&&localEditor){localEditor.excluded_projects=list(e.currentTarget.value);touch();}}}></textarea></label><button class="ms-btn" onclick={()=>sessionSource=localEditor??null}>{$t('已发现会话')}</button><button class="ms-btn" disabled={!!reading||dirty||!localEditor.enabled} onclick={()=>localEditor&&readLocal(localEditor)}><RefreshCw size={13}/>{$t(reading?'读取中…':'检测数据读取')}</button>{#if activeLocal==='claude_code'}<p class="ms-warning">{$t('本机会话采用只读观测；明确审批需单独确认官方 Hook。原生提问尚未支持。')}</p><button class="ms-btn" disabled={dirty||saving} onclick={()=>eventPrompt=localEditor?{id:localEditor.id,enabled:true}:null}>{$t('启用更准确的提醒')}</button><button class="ms-btn" disabled={dirty||saving} onclick={()=>eventPrompt=localEditor?{id:localEditor.id,enabled:false}:null}>{$t('移除 Claude 精确事件')}</button>{/if}<p class="ms-diagnostic">{$t(discovery.find(d=>d.id===localEditor?.id)?.detail??'尚未检测')}</p>{#each (snapshot?.providers??[]).filter(p=>p.provider===localEditor?.id&&p.status==='degraded'&&p.message) as provider (provider.provider)}<p class="ms-diagnostic" data-provider-diagnostic>{$t(provider.message??'尚无采集诊断')}</p>{/each}{:else}<p class="ms-muted">{$t('启用后可配置数据读取。')}</p>{/if}</div></details>
      </section>
              {#if settings && hookCatalog(agent.kind).length}<EventSources {settings} catalog={hookCatalog(agent.kind)} observations={snapshot?.view.sources??[]} {dirty} disabled={saving||conflict} changed={touch} localHook/>{/if}
            {/if}
          </div>
      {/snippet}
      <section class="detail-card local-agents">
        {#if discoveredLocal.length}
          <section class="discovery-review" data-discovery-review aria-label={$t('新发现的本机来源')}>
            <h3>{$t('新发现的本机来源')}</h3>
            {#each discoveredLocal as {definition} (definition.id)}
              {@render localAgentRow(definition,true)}
            {/each}
          </section>
        {/if}
        {#each featuredLocal as agent (agent.id)}
          {@render localAgentRow(agent)}
        {/each}
        {#if !featuredLocal.length&&!discoveredLocal.length}<p class="ms-muted">{$t('还没有检测到或配置本机来源。')}</p>{/if}
        {#if optionalLocal.length}
          <details class="ms-advanced" data-supported-sources bind:open={supportedOpen}>
            <summary>{$t('其他支持的 Agent')} · {optionalLocal.length}</summary>
            {#each optionalLocal as agent (agent.id)}{@render localAgentRow(agent)}{/each}
          </details>
        {/if}
<div class="auto-discover"><span>{$t('自动发现本机 Agent')}</span><input class="settings-switch" type="checkbox" role="switch" checked={$appSettings?.auto_discover_local??false} disabled={$preferencePending} aria-label={$t('自动发现本机 Agent')} onchange={e=>void savePreference({auto_discover_local:e.currentTarget.checked},e.currentTarget)}/></div>
      </section>
      {#if wslSources.length}
        <details class="detail-card" data-wsl-sources><summary>WSL · {$t('已有来源')}</summary>
          {#each wslSources as source (source.id)}{@render savedSourceRow(source)}{/each}
        </details>
      {/if}
      {#each fallbackSources as source (source.id)}{@render savedSourceRow(source)}{/each}
      {#if fallbackEvents.length}
        <EventSources {settings} catalog={eventAgents} sources={fallbackEvents} observations={snapshot?.view.sources??[]} {dirty} disabled={saving||conflict} changed={touch}/>
      {/if}
    {:else if selected}
      {@const source=selected.agents[activeRemote]}
      {@const report=reports[selected.host]}
      {@const linkState=connection(selected)}
      {@const diagnostic=snapshot?.providers.find(p=>p.provider===source?.id)?.message}
      <div class="detail-heading">
        <div>
          <h2>{selected.name}</h2>
          {#if selected.name!==selected.host}<p>{selected.host}</p>{/if}
        </div>
        <span class="ms-pill" class:neutral={linkState==='unknown'} class:warning={linkState==='disconnected'}><i></i>{$t(sshLabel(selected))}</span>
      </div>
      <div class="inline-actions server-actions">
        <button class="ms-btn" disabled={probing[selected.host]} onclick={()=>selected&&probeHost(selected)}><RefreshCw size={13}/>{$t(probing[selected.host]?'检测中…':'检测 SSH 与数据')}</button>
        {#if selected.paused||selected.sources.some(s=>s.enabled)}
          <button class="ms-btn" disabled={saving||dirty||conflict} onclick={()=>selected&&pause(selected)}>{#if selected.paused}<Play size={13}/>{:else}<Pause size={13}/>{/if}{$t(selected.paused?'恢复采集':'暂停采集')}</button>
        {/if}
      </div>
      {#if report?.connection==='disconnected'&&linkState==='disconnected'}<p class="ms-error" role="alert">{$t(report.message)}</p>{/if}
      <div class="detail-content">
        <section class="detail-card">
          <h3>Agent</h3>
          {#each sshAgents as agent (agent.key)}
            {@const entry=selected.agents[agent.key]}
            <div class="agent-row" data-remote-agent={agent.key}>
              <span class="agent-icon"><Terminal size={18}/></span>
              <div class="source-grow"><strong>{agent.name}</strong><small>{$t(remoteStatus(selected,agent.key))}</small></div>
              <input class="settings-switch" type="checkbox" role="switch" aria-label={$t(agent.toggle)} checked={entry?.enabled??false} disabled={selected.paused||saving||conflict} onchange={e=>selected&&toggleRemote(selected,agent.key,e.currentTarget.checked)}/>
            </div>
          {/each}
        </section>
        <section class="detail-card">
          <div class="section-top project-header">
            <h3>{$t('项目范围')}</h3>
            <select aria-label={$t('选择 Agent')} value={activeRemote} onchange={e=>selected&&selectRemote(selected,e.currentTarget.value as SshAgent)}>{#each sshAgents as agent (agent.key)}<option value={agent.key}>{agent.name}</option>{/each}</select>
          </div>
          {#key selected.host+':'+activeRemote}
            <ProjectScope projects={source?.projects??['*']} choices={source?projectChoices(source,snapshot?.tasks??[],[...(projects[source.id]??[]),...(report?.[activeRemote].projects??[])]):report?.[activeRemote].projects??[]} aliases={source?projectAliases[source.id]??[]:[]} disabled={saving||conflict} onchange={changeProjects}/>
          {/key}
          {#if source && !source.projects.includes('*') && report?.[activeRemote].state==='partial'}
            <p class="ms-warning">{$t('项目列表不完整，未列出的项目可以手动添加路径。')}</p>
          {/if}
          <details class="ms-advanced">
            <summary><Settings2 size={13}/>{$t('高级设置：轮询、路径与诊断')}</summary>
            <div class="advanced-inner">
              <label class="ms-field">{$t('服务器名称')}<input value={selected.name} oninput={e=>{if(!saving&&selected){const entries=Object.values(selected.agents).filter(entry=>entry!==undefined);if(!entries.length){const entry=ensureRemote(selected);if(entry)entries.push(entry);}for(const entry of entries)entry.name=e.currentTarget.value;touch();}}}/></label>
              <label class="ms-field">{$t('轮询间隔（毫秒）')}<input type="number" min="500" max="60000" value={source?.poll_interval_ms??3000} oninput={e=>{if(selected){const s=ensureRemote(selected);if(s){s.poll_interval_ms=Number(e.currentTarget.value);touch();}}}}/></label>
              <label class="ms-field">{$t('远端数据路径')}<input value={activeRemote==='codex'?'~/.codex':'CLAUDE_CONFIG_DIR / ~/.claude'} readonly/></label>
              <label class="ms-field">{$t('排除规则，每行一项')}<textarea rows="3" value={source?.excluded_projects.join('\n')??''} oninput={e=>{if(selected){const s=ensureRemote(selected);if(s){s.excluded_projects=list(e.currentTarget.value);touch();}}}}></textarea></label>
              {#if source}<button class="ms-btn" onclick={()=>sessionSource=source}>{$t('已发现会话')}</button>{/if}
              {#if diagnostic}<p class="ms-diagnostic">{$t(diagnostic)}</p>{/if}
            </div>
          </details>
        </section>
        {#each selected.sources.filter(item=>item.kind==='tmux') as source (source.id)}
          {@render savedSourceRow(source)}
        {/each}
      </div>
    {/if}
    </fieldset>
  </div>
  <footer class="ms-foot" class:idle={!dirty&&page!=='server'}><span>{#if dirty}<ShieldCheck size={14}/>{$t('有未保存的配置更改')}{/if}</span><div>{#if dirty}<button class="ms-btn ghost" disabled={saving} onclick={reload}>{$t('取消更改')}</button><button class="ms-btn primary" disabled={saving||conflict||updating||!detectionSettled} onclick={save}><Save size={13}/>{$t(saving?'正在保存…':settings&&!settings.onboarding_complete?'确认并开始监控':'应用更改')}</button>{:else if page==='server'}<button class="ms-btn danger" disabled={saving} onclick={()=>removePrompt=true}><Trash2 size={13}/>{$t('移除服务器')}</button>{/if}</div></footer>
  {#if sessionSource}<SourceSessions source={sessionSource} ondismiss={()=>sessionSource=null}/>{/if}
  {#if wizard}<SshSourceWizard existing={groups.map(s=>s.host)} onadd={addServer} oncancel={()=>wizard=false}/>{/if}
  {#if closePrompt||removePrompt||eventPrompt}<div class="ms-overlay"><dialog open aria-modal="true" class="ms-modal confirm-modal" use:trap aria-labelledby="source-confirm-title"><header class="modal-top"><h2 id="source-confirm-title">{$t(closePrompt?'还有未保存的配置':removePrompt?'移除服务器':'配置 Claude 精确事件')}</h2></header><div class="modal-body"><p>{$t(closePrompt?'关闭窗口前，请处理未保存的编辑。':removePrompt?'移除会停止此服务器的采集，并按现有范围规则移出其任务；不会修改远端文件。':'这会修改所选 Claude 配置。已有 Hook、代理和模型设置会保留，首次修改会备份。')}</p></div><footer class="modal-footer">{#if closePrompt}<button class="ms-btn" onclick={()=>closePrompt=false}>{$t('继续编辑')}</button><button class="ms-btn" onclick={discardClose}>{$t('放弃未保存编辑')}</button><button class="ms-btn primary" disabled={saving||conflict||wizard} onclick={saveClose}>{$t('保存并关闭')}</button>{:else if removePrompt}<button class="ms-btn" onclick={()=>removePrompt=false}>{$t('取消')}</button><button class="ms-btn danger" disabled={saving} onclick={remove}>{$t('确认移除')}</button>{:else}<button class="ms-btn" onclick={()=>eventPrompt=null}>{$t('取消')}</button><button class="ms-btn primary" onclick={confirmHook}>{$t('确认')}</button>{/if}</footer></dialog></div>{/if}
</div>
