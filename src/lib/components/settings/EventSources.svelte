<script lang="ts">
  import {t} from '$lib/i18n';
  import {tick} from 'svelte';
  import {createEventSource,eventAdapterLabel,eventConnections,isDefaultEventName,sourceDefinitionFor,type SourceDefinition} from '$lib/source-setup';
  import {sourceStatus} from '$lib/monitor-sources';
  import type {DashboardSettings,MonitorSource,SourceSnapshot} from '$lib/types';
  import IntegrationEditor from '../IntegrationEditor.svelte';
  import ProjectScope from './ProjectScope.svelte';
  let {settings,catalog,observations,dirty,disabled,changed,localHook=false,sources}:{settings:DashboardSettings;catalog:SourceDefinition[];observations:SourceSnapshot[];dirty:boolean;disabled:boolean;changed:()=>void;localHook?:boolean;sources?:MonitorSource[]}=$props();
  let expanded=$state(''),removing=$state('');
  function agentName(definition:SourceDefinition){
    const label=eventAdapterLabel(definition.adapter_id??'',definition.name);
    return definition.adapter_id==='standard-events'?$t(label):label;
  }
  const existing=$derived(sources??settings.sources);
  const groups=$derived(sources===undefined?eventConnections(catalog,existing):[
    ...eventConnections(catalog,existing),
    ...existing.filter(source=>!catalog.some(d=>d.adapter_id===source.integration?.adapter_id))
      .map(source=>({definition:sourceDefinitionFor(catalog,source),connections:[source]})),
  ]);
  const count=$derived(groups.reduce((total,group)=>total+group.connections.length,0));
  function status(source:MonitorSource){
    const observation=observations.find(item=>item.id===source.id);
    if(source.enabled && observation?.read_state==='healthy' && !observation.data_detected){
      return '等待首条任务通知';
    }
    return sourceStatus(source,observation);
  }
  async function reveal(id:string){await tick();document.querySelector(`[data-event-source="${CSS.escape(id)}"]`)?.scrollIntoView({block:'start'});}
  async function add(definition:SourceDefinition){if(disabled||!localHook)return;const source=createEventSource(definition,agentName(definition));settings.sources.push(source);expanded=source.id;changed();await reveal(source.id);}
  function remove(id:string){if(disabled)return;settings.sources=settings.sources.filter(s=>s.id!==id);removing='';expanded='';changed();}
</script>

{#snippet connections()}
  {#each groups as {definition,connections} (definition.id)}
    {@const grouped=localHook}
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
          <div class="inline-actions">{#if localHook&&definition.adapter_id&&definition.agent_id}<button class="ms-btn" disabled={disabled} onclick={()=>add(definition)}>{$t('添加数据来源实例')}</button>{/if}<button class="ms-btn danger" disabled={disabled} onclick={()=>removing=source.id}>{$t('移除来源')}</button></div>
          {#if removing===source.id}<div class="ms-warning">{$t('移除会停止采集，不会修改 Agent 配置。')} <button class="ms-btn" onclick={()=>removing=''}>{$t('取消')}</button> <button class="ms-btn danger" disabled={disabled} onclick={()=>remove(source.id)}>{$t('确认移除')}</button></div>{/if}
        </div>
      {/if}
    </div>
    {/each}
  {/each}
{/snippet}

{#if localHook}
  <details class="detail-card event-sources local-hook">
    <summary>{$t('启用更准确的提醒')}{#if count} · {count}{/if}</summary>
    <p class="ms-muted">{$t('Hook 是这个 Agent 的可选事件来源，需要单独在宿主中配置。')}</p>
    {#each catalog as definition (definition.id)}
      <button class="ms-btn" type="button" disabled={disabled} data-event-add onclick={()=>add(definition)}>{$t('添加 Hook 连接')}</button>
    {/each}
    {@render connections()}
  </details>
{:else if count}
  <div class="detail-card event-sources" data-saved-event-connections>
    {@render connections()}
  </div>
{/if}


<style>
  summary{cursor:pointer;font-size:13px;font-weight:600}
  .local-hook{margin:8px 0 12px 39px}
</style>
