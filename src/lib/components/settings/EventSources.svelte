<script lang="ts">
  import {t} from '$lib/i18n';
  import {tick} from 'svelte';
  import {eventAdapterLabel,isDefaultEventName,sourceDefinitionFor,type SourceDefinition} from '$lib/source-setup';
  import {sourceStatus} from '$lib/monitor-sources';
  import type {DashboardSettings,MonitorSource,SourceSnapshot} from '$lib/types';
  import IntegrationEditor from '../IntegrationEditor.svelte';
  import ProjectScope from './ProjectScope.svelte';
  let {settings,catalog,observations,dirty,disabled,changed,sources}:{settings:DashboardSettings;catalog:SourceDefinition[];observations:SourceSnapshot[];dirty:boolean;disabled:boolean;changed:()=>void;sources:MonitorSource[]}=$props();
  let expanded=$state(''),removing=$state('');
  function agentName(definition:SourceDefinition){
    const label=eventAdapterLabel(definition.adapter_id??'',definition.name);
    return definition.adapter_id==='standard-events'?$t(label):label;
  }
  function status(source:MonitorSource){
    const observation=observations.find(item=>item.id===source.id);
    if(source.enabled && observation?.read_state==='healthy' && !observation.data_detected){
      return '等待首条任务通知';
    }
    return sourceStatus(source,observation);
  }
  async function reveal(id:string){await tick();document.querySelector(`[data-event-source="${CSS.escape(id)}"]`)?.scrollIntoView({block:'start'});}
  function remove(id:string){if(disabled)return;settings.sources=settings.sources.filter(s=>s.id!==id);removing='';expanded='';changed();}
</script>

<details class="detail-card event-sources" data-saved-event-connections>
  <summary>{$t("已保存的事件连接")}</summary>
  {#each sources as source (source.id)}
    {@const definition=sourceDefinitionFor(catalog,source)}
    {@const label=isDefaultEventName(source.name,definition)?agentName(definition):source.name}
    <div class="agent-entry" data-event-adapter={definition.adapter_id} data-event-source={source.id}>
      <div class="agent-row">
        <div class="source-grow"><strong>{label}</strong><small>{$t(status(source))}</small></div>
        <button class="ms-btn small" disabled={disabled} aria-expanded={expanded===source.id} onclick={()=>{expanded=expanded===source.id?'':source.id;if(expanded)void reveal(expanded);}}>{$t(expanded===source.id?'收起':'配置')}</button>
        <input class="settings-switch" type="checkbox" role="switch" aria-label={$t('启用 {name}',{name:label})} checked={source.enabled} disabled={disabled} onchange={e=>{source.enabled=e.currentTarget.checked;changed();}}/>
      </div>
      {#if expanded===source.id}
        <div class="agent-config">
          <label class="ms-field">{$t('来源名称')}<input value={source.name} disabled={disabled} oninput={e=>{source.name=e.currentTarget.value;changed();}}/></label>
          <IntegrationEditor {source} {dirty} {changed}/>
          <h3>{$t('项目范围')}</h3><ProjectScope projects={source.projects} choices={[]} {disabled} onchange={projects=>{source.projects=projects;changed();}}/>
          <div class="inline-actions"><button class="ms-btn danger" disabled={disabled} onclick={()=>removing=source.id}>{$t('移除来源')}</button></div>
          {#if removing===source.id}<div class="ms-warning">{$t('移除会停止采集，不会修改 Agent 配置。')} <button class="ms-btn" onclick={()=>removing=''}>{$t('取消')}</button> <button class="ms-btn danger" disabled={disabled} onclick={()=>remove(source.id)}>{$t('确认移除')}</button></div>{/if}
        </div>
      {/if}
    </div>
  {/each}
</details>
