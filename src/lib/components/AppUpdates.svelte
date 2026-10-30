<script lang="ts">
  import {t} from '$lib/i18n';
  import {onMount} from 'svelte';
  import {invoke,isTauri} from '@tauri-apps/api/core';
  import {listen} from '@tauri-apps/api/event';
  import type {UpdateStatus} from '$lib/types';
  let {blocked=false}:{blocked?:boolean}=$props();
  let status=$state<UpdateStatus|null>(null),error=$state('');
  const busy=$derived(!!status && ['checking','downloading','verifying','installing'].includes(status.phase));
  const label=$derived(status ? ({idle:'检查 TaskPulse 的新版本',checking:'正在检查更新…',current:'已是最新版本',available:$t('发现新版本 {version}',{version:status.version??''}),downloading:'正在下载安装包…',verifying:'正在验证签名…',installing:'正在安装…',error:'暂时无法更新'}[status.phase] ?? '') : '正在读取版本…');
  async function check(){error='';try{status=await invoke<UpdateStatus>('check_application_update');}catch(e){error=String(e);}}
  async function install(){if(blocked || !status?.version)return;error='';try{await invoke('install_application_update',{version:status.version});}catch(e){error=String(e);}}
  onMount(()=>{if(!isTauri())return;let stop:undefined|(()=>void),active=true;
    void listen<UpdateStatus>('application-update',e=>status=e.payload).then(fn=>{if(active)stop=fn;else fn();});
    void invoke<UpdateStatus>('get_update_status').then(value=>{if(active)status=value;}).catch(e=>error=String(e));
    return()=>{active=false;stop?.();};
  });
</script>
<section class="updates tp-surface" aria-labelledby="update-title">
  <div class="heading"><h2 id="update-title">{$t("应用更新")}</h2>{#if status}<span>{$t("当前版本")} {status.current_version}</span>{/if}</div>
  <p role="status" aria-live="polite">{$t(label)}</p>
  <p class="help">{$t("自动检查只会提示新版本。点击安装后才会下载、校验并重启 TaskPulse；你的监控设置会保留。")}</p>
  {#if status?.phase==='downloading'}<progress max={status.total ?? 1} value={status.total ? status.downloaded : undefined} aria-label={$t("更新下载进度")}></progress>{/if}
  {#if status?.notes}<details class="tp-fold"><summary>{$t("版本更新内容")}</summary><p class="notes">{status.notes}</p></details>{/if}
  {#if error || status?.message}<p class="error" role="alert">{error || status?.message}</p>{/if}
  <div class="actions"><button class="tp-btn" disabled={busy} onclick={check}>{status?.phase==='checking'?$t('正在检查…'):$t('检查更新')}</button>
  {#if status?.phase==='available'}<button class="tp-btn" data-variant="primary" disabled={busy || blocked || !status.can_install} onclick={install}>{$t("下载并安装 · 将重启")}</button>{/if}</div>
  {#if blocked && status?.phase==='available'}<p class="help">{$t("请先保存或放弃未保存的设置，再安装更新。")}</p>{/if}
  {#if status?.phase==='available' && !status.can_install}<p class="help">{$t("请在已安装的 TaskPulse 中安装更新。")}</p>{/if}
  {#if status?.checked_at}<p class="help">{$t("上次检查：")}{new Date(status.checked_at).toLocaleString()}</p>{/if}
</section>
<style>
  .updates{max-width:680px;padding:18px;margin-top:20px;border:1px solid var(--tp-border);border-radius:var(--tp-radius-lg);background:var(--tp-panel)}
  .heading{display:flex;align-items:baseline;justify-content:space-between;gap:12px;flex-wrap:wrap}h2{font-size:15px;margin:0 0 12px;font-weight:600}.heading span,.help{color:var(--tp-muted);font-size:11px;line-height:1.9}p{font-size:12px;line-height:1.8}.actions{display:flex;flex-wrap:wrap;gap:8px}.actions button{font-size:11px}.error{color:var(--tp-danger)}.notes{white-space:pre-wrap;overflow-wrap:anywhere;max-height:240px;overflow:auto;padding:10px}progress{width:100%;accent-color:var(--tp-accent)}details{font-size:11px;margin:10px 0}
</style>
