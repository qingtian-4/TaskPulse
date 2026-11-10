<script lang="ts">
  import {Plus,Folder,Check} from '@lucide/svelte';
  import {t} from '$lib/i18n';
  import {scopeOptions,type ProjectChoice} from '$lib/project-options';
  let {projects,choices=[],aliases=[],onchange,disabled=false}:{projects:string[];choices?:string[];aliases?:ProjectChoice[];onchange:(value:string[])=>void;disabled?:boolean}=$props();
  const group=$props.id();
  let path=$state(''),selectedBeforeAll=$state<string[]>([]);
  const all=$derived(projects.includes('*'));
  const options=$derived(scopeOptions(projects,choices,aliases));
  function mode(value:boolean){if(value){selectedBeforeAll=[...projects];onchange(['*']);}else onchange(selectedBeforeAll.filter(p=>p!=='*'));}
  function toggle(option:ProjectChoice,checked:boolean){onchange(checked?[...new Set([...projects.filter(p=>p!=='*'),...option.aliases])]:projects.filter(p=>!option.aliases.includes(p)));}
  function add(){const value=path.trim();if(!value||/[\r\n\0]/.test(value))return;onchange([...new Set([...projects.filter(p=>p!=='*'),value])]);path='';}
</script>
<div class="project-scope">
  <div class="scope-modes">
    <label class:selected={all}><input type="radio" name={group} checked={all} disabled={disabled} onchange={()=>mode(true)}/>{$t('自动关注全部项目')}</label>
    <label class:selected={!all}><input type="radio" name={group} checked={!all} disabled={disabled} onchange={()=>mode(false)}/>{$t('仅关注指定项目')}</label>
  </div>
  {#if all}<p class="scope-notice"><Check size={15}/>{$t('自动关注此来源后续发现的新项目。')}</p>
  {:else}
    {#if !projects.length}<p class="scope-notice">{$t('未选择项目，此来源暂不会纳入任务。')}</p>{/if}
    <div class="project-options">
      {#each options as option (option.id)}
        {@const unassociated=option.path==='@unassociated'}
        <label><input type="checkbox" checked={option.aliases.some(value=>projects.includes(value))} disabled={disabled} onchange={e=>toggle(option,e.currentTarget.checked)}/><Folder size={14}/><span title={unassociated?$t('此来源中未记录项目路径的会话'):option.path}><strong>{unassociated?$t('未关联项目'):option.path.split(/[\\/]/).filter(Boolean).at(-1)||option.path}</strong><small>{unassociated?$t('此来源中未记录项目路径的会话'):option.path}</small></span></label>
      {/each}
    </div>
    {#if !options.length}<p class="scope-empty">{$t('暂无已发现项目，可以手动添加路径。')}</p>{/if}
    <form class="add-path" onsubmit={e=>{e.preventDefault();add();}}><input bind:value={path} disabled={disabled} aria-label={$t('添加项目路径')} placeholder={$t('输入项目路径，例如 /home/user/project')}/><button class="ms-btn" type="submit" disabled={disabled||!path.trim()}><Plus size={13}/>{$t('添加')}</button></form>
  {/if}
</div>
<style>
  .scope-modes{display:flex;gap:9px}.scope-modes>label{flex:1;display:flex;align-items:center;gap:7px;padding:11px;border:1px solid var(--ms-border);border-radius:8px;font-size:13px;cursor:pointer}.scope-modes>label.selected{background:var(--ms-soft);border-color:var(--ms-line);color:var(--ms-accent);font-weight:600}.project-scope input[type=radio],.project-scope input[type=checkbox]{accent-color:var(--ms-accent);width:14px;height:14px;flex:none}.scope-notice{display:flex;align-items:center;gap:8px;background:var(--ms-soft);padding:10px 12px;border-radius:8px;color:var(--ms-accent);font-size:13px}.project-options{display:grid;grid-template-columns:1fr 1fr;gap:7px;margin-top:12px;max-height:180px;overflow:auto}.project-options>label{display:flex;gap:8px;align-items:center;padding:9px;border:1px solid var(--ms-border);border-radius:8px;min-width:0}.project-options span{min-width:0}.project-options strong,.project-options small{display:block;font-size:13px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.project-options small{font-size:12px;color:var(--ms-muted);margin-top:3px}.add-path{display:flex;gap:8px;margin-top:10px}.add-path input{flex:1;min-width:0}.scope-empty{font-size:13px;line-height:1.6;color:var(--ms-muted);margin:10px 0}
</style>
