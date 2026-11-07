<script lang="ts">
  import {t} from '$lib/i18n';
  import {tick} from 'svelte';
  import {createEventSource,type SourceDefinition} from '$lib/source-setup';
  import {sourceStatus} from '$lib/monitor-sources';
  import type {DashboardSettings,MonitorSource,SourceSnapshot} from '$lib/types';
  import IntegrationEditor from '../IntegrationEditor.svelte';
  import ProjectScope from './ProjectScope.svelte';
  let {settings,catalog,observations,dirty,disabled,changed}:{settings:DashboardSettings;catalog:SourceDefinition[];observations:SourceSnapshot[];dirty:boolean;disabled:boolean;changed:()=>void}=$props();
  let expanded=$state(''),removing=$state(''),qoder=$state(false);
  const rows=$derived(catalog.flatMap<{definition:SourceDefinition;source?:MonitorSource}>(definition=>{
    const saved=settings.sources.filter(s=>s.kind==='generic_event'&&s.integration?.adapter_id===definition.adapter_id);
    return saved.length?saved.map(source=>({definition,source})):[{definition,source:undefined}];
  }));
  async function reveal(id:string){await tick();document.querySelector(`[data-event-source="${id}"]`)?.scrollIntoView({block:'start'});}
  async function add(definition:SourceDefinition){const source=createEventSource(definition);settings.sources.push(source);expanded=source.id;changed();await reveal(source.id);}
  function remove(id:string){settings.sources=settings.sources.filter(s=>s.id!==id);removing='';expanded='';changed();}
</script>

<section class="detail-card event-sources" aria-label={$t('事件接入')}>
  <h3>{$t('事件接入')}<small>{$t('需单独配置')}</small></h3>
  {#each rows as {definition,source} (source?.id??definition.id)}
    <div class="agent-entry" data-event-adapter={definition.adapter_id} data-event-source={source?.id}>
      <div class="agent-row">
        <div class="source-grow"><strong>{source?source.name:$t(definition.name)}</strong><small>{$t(source?sourceStatus(source,observations.find(s=>s.id===source.id)):'尚未接入')}</small></div>
        <span class="cap-tag planned">{$t('事件接入')}</span>
        <button class="ms-btn small" disabled={disabled} aria-expanded={!!source&&expanded===source.id} onclick={()=>{if(!source)void add(definition);else {expanded=expanded===source.id?'':source.id;if(expanded)void reveal(expanded);}}}>{$t(source&&expanded===source.id?'收起':'配置')}</button>
        {#if source}<input class="settings-switch" type="checkbox" role="switch" aria-label={$t('启用 {name}',{name:source.name})} checked={source.enabled} disabled={disabled} onchange={e=>{source.enabled=e.currentTarget.checked;changed();}}/>{/if}
      </div>
      {#if source&&expanded===source.id}
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
  <div class="agent-entry"><div class="agent-row"><div class="source-grow"><strong>Qoder</strong><small>{$t('原生读取与匹配解除尚未适配')}</small></div><span class="cap-tag planned">{$t('待适配')}</span><button class="ms-btn small" aria-expanded={qoder} onclick={()=>qoder=!qoder}>{$t(qoder?'收起':'支持边界')}</button></div>{#if qoder}<p class="ms-muted">{$t('Qoder IDE 与 CLI 的 Hook 不同；当前不提供自动采集。可使用“其他 Agent（标准事件）”手动接入，任务与请求关联由生产者负责。')}</p>{/if}</div>
</section>
