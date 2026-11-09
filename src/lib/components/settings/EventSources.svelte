<script lang="ts">
  import {t} from '$lib/i18n';
  import {tick} from 'svelte';
  import {createEventSource,eventAdapterLabel,type SourceDefinition} from '$lib/source-setup';
  import {sourceStatus} from '$lib/monitor-sources';
  import type {DashboardSettings,MonitorSource,SourceSnapshot} from '$lib/types';
  import IntegrationEditor from '../IntegrationEditor.svelte';
  import ProjectScope from './ProjectScope.svelte';
  let {settings,catalog,observations,dirty,disabled,changed}:{settings:DashboardSettings;catalog:SourceDefinition[];observations:SourceSnapshot[];dirty:boolean;disabled:boolean;changed:()=>void}=$props();
  let expanded=$state(''),removing=$state(''),adapterId=$state('');
  function agentName(definition:SourceDefinition){
    const label=eventAdapterLabel(definition.adapter_id??'',definition.name);
    return definition.adapter_id==='standard-events'?$t(label):label;
  }
  const rows=$derived(catalog.flatMap(definition=>settings.sources
    .filter(s=>s.kind==='generic_event'&&s.integration?.adapter_id===definition.adapter_id)
    .map(source=>({definition,source}))));
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

<details class="detail-card event-sources">
  <summary>{$t('高级接入：手动连接')}{#if rows.length} · {rows.length}{/if}</summary>
  <p class="ms-muted">{$t('用于让 Agent 或脚本主动发送任务开始、结束或请求等通知。需要在发送端单独配置。')}</p>
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
  {#each rows as {definition,source} (source.id)}
    <div class="agent-entry" data-event-adapter={definition.adapter_id} data-event-source={source.id}>
      <div class="agent-row">
        <div class="source-grow"><strong>{source.name===definition.name||source.name===$t(definition.name)?agentName(definition):source.name}</strong><small>{$t(status(source))}</small></div>
        <button class="ms-btn small" disabled={disabled} aria-expanded={expanded===source.id} onclick={()=>{expanded=expanded===source.id?'':source.id;if(expanded)void reveal(expanded);}}>{$t(expanded===source.id?'收起':'配置')}</button>
        <input class="settings-switch" type="checkbox" role="switch" aria-label={$t('启用 {name}',{name:source.name})} checked={source.enabled} disabled={disabled} onchange={e=>{source.enabled=e.currentTarget.checked;changed();}}/>
      </div>
      {#if expanded===source.id}
        <div class="agent-config">
          <label class="ms-field">{$t('来源名称')}<input value={source.name} disabled={disabled} oninput={e=>{source.name=e.currentTarget.value;changed();}}/></label>
          <IntegrationEditor {source} {dirty} {changed}/>
          <h3>{$t('项目范围')}</h3><ProjectScope projects={source.projects} choices={[]} {disabled} onchange={projects=>{source.projects=projects;changed();}}/>
          <div class="inline-actions"><button class="ms-btn" disabled={disabled} onclick={()=>add(definition)}>{$t('添加数据来源实例')}</button><button class="ms-btn danger" disabled={disabled} onclick={()=>removing=source.id}>{$t('移除来源')}</button></div>
          {#if removing===source.id}<div class="ms-warning">{$t('移除会停止采集，不会修改 Agent 配置。')} <button class="ms-btn" onclick={()=>removing=''}>{$t('取消')}</button> <button class="ms-btn danger" disabled={disabled} onclick={()=>remove(source.id)}>{$t('确认移除')}</button></div>{/if}
        </div>
      {/if}
    </div>
  {/each}
</details>

<style>
  summary{cursor:pointer;font-size:13px;font-weight:600}
  .event-picker{display:flex;align-items:end;gap:8px;margin:12px 0}
  .event-picker label{flex:1;min-width:0}
  .event-picker button{flex:none}
</style>
