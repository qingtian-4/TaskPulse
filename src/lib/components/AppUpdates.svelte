<script lang="ts">
  import {t} from '$lib/i18n';
  import {onMount} from 'svelte';
  import {RefreshCw} from '@lucide/svelte';
  import {invoke,isTauri} from '@tauri-apps/api/core';
  import {listen} from '@tauri-apps/api/event';
  import type {UpdateStatus} from '$lib/types';
  let {blocked=false}:{blocked?:boolean}=$props();
  let status=$state<UpdateStatus|null>(null),error=$state('');
  const busy=$derived(!!status && ['checking','downloading','verifying','installing'].includes(status.phase));
  const label=$derived(status ? ({unavailable:'开发版本',idle:'',checking:'正在检查更新…',current:'已是最新版本',available:$t('发现新版本 {version}',{version:status.version??''}),downloading:'正在下载安装包…',verifying:'正在验证签名…',installing:'正在安装…',error:'暂时无法更新'}[status.phase] ?? '') : '正在读取版本…');
  async function check(){error='';try{status=await invoke<UpdateStatus>('check_application_update');}catch(e){error=String(e);}}
  async function install(){if(blocked || !status?.version)return;error='';try{await invoke('install_application_update',{version:status.version});}catch(e){error=String(e);}}
  onMount(()=>{if(!isTauri())return;let stop:undefined|(()=>void),active=true;
    void listen<UpdateStatus>('application-update',e=>status=e.payload).then(fn=>{if(active)stop=fn;else fn();});
    void invoke<UpdateStatus>('get_update_status').then(value=>{if(active)status=value;}).catch(e=>error=String(e));
    return()=>{active=false;stop?.();};
  });
</script>
<section class="updates settings-group" aria-labelledby="update-title">
  <div class="setting-row version-row">
    <RefreshCw size={18} class="update-icon" aria-hidden="true"/>
    <div class="setting-copy">
      <h2 id="update-title" class="setting-label">{$t('当前版本')} {status?.current_version??'…'}</h2>
      {#if label}<p class="setting-description" role="status">{$t(label)}</p>{/if}
      {#if status?.checked_at && (!label || status.phase==='current')}
        <p class="setting-description">{$t('上次检查：')} {new Date(status.checked_at).toLocaleString()}</p>
      {/if}
    </div>
    <button class="tp-btn" disabled={busy||status?.phase==='unavailable'} onclick={check}>
      {status?.phase==='checking'?$t('正在检查…'):$t('检查更新')}
    </button>
  </div>
  {#if status?.phase==='available' || status?.phase==='downloading' || status?.notes || error || status?.message}
    <div class="update-details">
      {#if status?.phase==='downloading'}<progress max={status.total ?? 1} value={status.total ? status.downloaded : undefined} aria-label={$t('更新下载进度')}></progress>{/if}
      {#if status?.notes}<details class="tp-fold"><summary>{$t('版本更新内容')}</summary><p class="notes">{status.notes}</p></details>{/if}
      {#if error || status?.message}<p class="error" role="alert">{error || status?.message}</p>{/if}
      {#if status?.phase==='available'}
        {#if blocked}<p class="setting-description">{$t('请先保存或放弃未保存的设置，再安装更新。')}</p>{/if}
        {#if !status.can_install}<p class="setting-description">{$t('请在已安装的 TaskPulse 中安装更新。')}</p>{/if}
        <button class="tp-btn install-action" data-variant="primary" disabled={blocked || !status.can_install} onclick={install}>{$t('下载并安装 · 将重启')}</button>
      {/if}
    </div>
  {/if}
</section>
<style>
  .version-row{gap:12px;min-height:66px}
  .version-row :global(.update-icon){flex:none;color:var(--tp-muted)}
  h2{margin:0;font-weight:600;overflow-wrap:anywhere}
  .version-row button{flex:none}
  .update-details{display:grid;gap:8px;padding:12px 0 16px;font-size:12px;overflow-wrap:anywhere}
  .update-details p{margin:0}
  .install-action{justify-self:end}
  .notes{white-space:pre-wrap;max-height:240px;overflow:auto;padding:10px}
  progress{width:100%;accent-color:var(--tp-accent)}
  @media (max-width: 360px){.version-row{flex-wrap:wrap}.version-row button{margin-inline-start:auto}}
</style>
