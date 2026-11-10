<script lang="ts">
  import {t} from '$lib/i18n';
  import {tick} from 'svelte';
  import {createEventSource,eventAdapterLabel,eventConnections,isDefaultEventName,type SourceDefinition} from '$lib/source-setup';
  import {sourceStatus} from '$lib/monitor-sources';
  import type {DashboardSettings,MonitorSource,SourceSnapshot} from '$lib/types';
  import IntegrationEditor from '../IntegrationEditor.svelte';
  import ProjectScope from './ProjectScope.svelte';
  let {settings,catalog,observations,dirty,disabled,changed,localHook=false}:{settings:DashboardSettings;catalog:SourceDefinition[];observations:SourceSnapshot[];dirty:boolean;disabled:boolean;changed:()=>void;localHook?:boolean}=$props();
  let expanded=$state(''),removing=$state(''),adapterId=$state('');
  function agentName(definition:SourceDefinition){
    const label=eventAdapterLabel(definition.adapter_id??'',definition.name);
    return definition.adapter_id==='standard-events'?$t(label):label;
  }
  const groups=$derived(eventConnections(catalog,settings.sources));
  const count=$derived(groups.reduce((total,group)=>total+group.connections.length,0));
  function status(source:MonitorSource){
    const observation=observations.find(item=>item.id===source.id);
    if(source.enabled && observation?.read_state==='healthy' && !observation.data_detected){
      return '等待首条任务通知';
    }
    return sourceStatus(source,observation);
  }
  const selected=$derived(catalog.find(definition=>definition.adapter_id===adapterId));
  async function reveal(id:string){await tick();document.querySelector(`[data-event-source="${CSS.escape(id)}"]`)?.scrollIntoView({block:'start'});}
  async function add(definition:SourceDefinition){const source=createEventSource(definition,agentName(definition));settings.sources.push(source);expanded=source.id;changed();await reveal(source.id);}
  function remove(id:string){settings.sources=settings.sources.filter(s=>s.id!==id);removing='';expanded='';changed();}
</script>

<details class="detail-card event-sources" class:local-hook={localHook}>
  <summary>{$t(localHook?'Hook 连接':'连接其他工具（可选）')}{#if count} · {count}{/if}</summary>
  <p class="ms-muted">{$t(localHook?'Hook 是这个 Agent 的可选事件来源，需要单独在宿主中配置。':'上方的本机工具不用再连接。只有能主动发送任务状态的其他工具，才需要在这里添加。')}</p>
  {#if localHook}
    {#each catalog as definition (definition.id)}<button class="ms-btn" type="button" disabled={disabled} data-event-add onclick={()=>add(definition)}>{$t('添加 Hook 连接')}</button>{/each}
  {:else if catalog.length===1 && catalog[0].adapter_id==='standard-events'}
    <button class="ms-btn" type="button" disabled={disabled} data-event-add onclick={()=>add(catalog[0])}>{$t('添加 MCP 或脚本连接')}</button>
  {:else if catalog.length}
  <div class="event-picker">
    <label class="ms-field">{$t('选择连接类型')}
      <select bind:value={adapterId} disabled={disabled} data-event-picker>
        <option value="">{$t('请选择')}</option>
        {#each catalog as definition (definition.id)}
          <option value={definition.adapter_id}>{agentName(definition)}</option>
        {/each}
      </select>
    </label>
    <button class="ms-btn" type="button" disabled={disabled||!selected} data-event-add onclick={()=>selected&&add(selected)}>{$t('添加连接')}</button>
  </div>
  {/if}
  {#each groups as {definition,connections} (definition.id)}
    {@const grouped=localHook||definition.adapter_id==='standard-events'||connections.length>1}
    {#if grouped&&!localHook}<h3 class="connection-heading">{agentName(definition)} · {connections.length}</h3>{/if}
    {#each connections as source,index (source.id)}
    {@const defaultName=isDefaultEventName(source.name,definition)||source.name===$t(definition.name)}
    {@const connectionLabel=$t('连接 {number}',{number:index+1})}
    <div class="agent-entry" data-event-adapter={definition.adapter_id} data-event-source={source.id}>
      <div class="agent-row">
        <div class="source-grow"><strong>{defaultName?(grouped?connectionLabel:agentName(definition)):source.name}</strong><small>{#if grouped&&!defaultName}{connectionLabel} · {/if}{$t(status(source))}</small></div>
        <button class="ms-btn small" disabled={disabled} aria-expanded={expanded===source.id} onclick={()=>{expanded=expanded===source.id?'':source.id;if(expanded)void reveal(expanded);}}>{$t(expanded===source.id?'收起':'配置')}</button>
        <input class="settings-switch" type="checkbox" role="switch" aria-label={$t('启用 {name}',{name:grouped?`${defaultName?agentName(definition):source.name} · ${connectionLabel}`:source.name})} checked={source.enabled} disabled={disabled} onchange={e=>{source.enabled=e.currentTarget.checked;changed();}}/>
      </div>
      {#if expanded===source.id}
        <div class="agent-config">
          <label class="ms-field">{$t('来源名称')}<input value={source.name} disabled={disabled} oninput={e=>{source.name=e.currentTarget.value;changed();}}/></label>
          <IntegrationEditor {source} {dirty} {changed} {localHook}/>
          <h3>{$t('项目范围')}</h3><ProjectScope projects={source.projects} choices={[]} {disabled} onchange={projects=>{source.projects=projects;changed();}}/>
          <div class="inline-actions"><button class="ms-btn" disabled={disabled} onclick={()=>add(definition)}>{$t('添加数据来源实例')}</button><button class="ms-btn danger" disabled={disabled} onclick={()=>removing=source.id}>{$t('移除来源')}</button></div>
          {#if removing===source.id}<div class="ms-warning">{$t('移除会停止采集，不会修改 Agent 配置。')} <button class="ms-btn" onclick={()=>removing=''}>{$t('取消')}</button> <button class="ms-btn danger" disabled={disabled} onclick={()=>remove(source.id)}>{$t('确认移除')}</button></div>{/if}
        </div>
      {/if}
    </div>
    {/each}
  {/each}
</details>

<style>
  summary{cursor:pointer;font-size:13px;font-weight:600}
  .event-picker{display:flex;align-items:end;gap:8px;margin:12px 0}
  .event-picker label{flex:1;min-width:0}
  .event-picker button{flex:none}
  .local-hook{margin:8px 0 12px 39px}
  .connection-heading{margin-top:15px}
</style>
