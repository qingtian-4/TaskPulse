<script lang="ts">
  import {onMount,tick} from 'svelte';
  import {SlidersHorizontal,PanelsTopLeft,RadioTower,Bell,RefreshCw,Info,ShieldCheck} from '@lucide/svelte';
  import {listen} from '@tauri-apps/api/event';
  import {isTauri,invoke} from '@tauri-apps/api/core';
  import BrandMark from '$lib/components/BrandMark.svelte';
  import GeneralSettings from '$lib/components/settings/GeneralSettings.svelte';
  import PanelSettings from '$lib/components/settings/PanelSettings.svelte';
  import SourceSettings from '$lib/components/settings/SourceSettings.svelte';
  import NotificationSettings from '$lib/components/settings/NotificationSettings.svelte';
  import UpdateSettings from '$lib/components/settings/UpdateSettings.svelte';
  import AboutSettings from '$lib/components/settings/AboutSettings.svelte';
  import '$lib/components/settings/settings.css';
  import {t} from '$lib/i18n';
  import {appSettings,preferencePending,preferenceError} from '$lib/preferences';
  import {autostart} from '$lib/autostart';
  import type {DashboardSettings} from '$lib/types';
  type Section='general'|'panel'|'sources'|'notifications'|'updates'|'about';
  let section=$state<Section>('general'),sourceDirty=$state(false),scroll:HTMLDivElement;
  let busy=$state(false),loadError=$state('');
  const positions:Partial<Record<Section,number>>={};
  const sections=[['general','通用','让 TaskPulse 按你的习惯工作。',SlidersHorizontal],['panel','任务面板','只保留此刻值得关注的任务信息。',PanelsTopLeft],['sources','监控来源','选择要关注的 Agent，以及它们的项目范围。',RadioTower],['notifications','通知提醒','在任务需要你时提醒，而不是打断你。',Bell],['updates','应用更新','自动发现新版本，由你决定何时安装。',RefreshCw],['about','关于','TaskPulse，你的 Agent 任务关注中心。',Info]] as const;
  const current=$derived(sections.find(item=>item[0]===section)!);
  async function navigate(next:Section){if(scroll)positions[section]=scroll.scrollTop;section=next;await tick();if(scroll)scroll.scrollTop=positions[next]??0;}
  async function reload(){try{appSettings.set(await invoke<DashboardSettings>('get_monitoring_settings'));preferenceError.set('');loadError='';}catch(e){loadError=String(e);}}
  onMount(()=>{
    const requested=new URLSearchParams(location.search).get('section');if(sections.some(s=>s[0]===requested))section=requested as Section;
    if(!isTauri())return;
    let active=true;const cleanups:Array<()=>void>=[];
    void(async()=>{
      const stopSection=await listen<string>('management-section',e=>{if(sections.some(s=>s[0]===e.payload))void navigate(e.payload as Section);});if(active)cleanups.push(stopSection);else stopSection();
      const stopUpdate=await listen<{phase:string}>('application-update',e=>busy=['downloading','verifying','installing'].includes(e.payload.phase));if(active)cleanups.push(stopUpdate);else stopUpdate();
      const status=await invoke<{phase:string}>('get_update_status');busy=['downloading','verifying','installing'].includes(status.phase);

    })().catch(e=>loadError=String(e));
    return()=>{active=false;cleanups.forEach(fn=>fn());};
  });
</script>
<div class="settings-shell tp-theme">
  <aside class="settings-side"><div class="settings-brand"><BrandMark size={34}/><div><strong>TaskPulse</strong><small>{$t('偏好设置')}</small></div></div>
    <nav class="settings-nav" aria-label={$t('设置分类')}>{#each sections as [id,label,,Icon]}<a href={'#'+id} aria-current={section===id?'page':undefined} onclick={e=>{e.preventDefault();void navigate(id);}}><Icon size={17} aria-hidden="true"/><span>{$t(label)}</span></a>{/each}</nav>
    <div class="settings-footnote"><span><ShieldCheck size={14} aria-hidden="true"/> {$t('任务数据保存在本机')}</span><span>TaskPulse</span></div>
  </aside>
  <main class="settings-main"><header class="settings-heading"><div><h1>{$t(current[1])}</h1><p>{$t(current[2])}</p></div></header>
    <div class="settings-content glass-scroll" bind:this={scroll}>
      {#if $preferenceError||loadError}<div class="settings-inline-error" role="alert">{$t($preferenceError||loadError)} <button class="tp-btn" onclick={reload}>{$t('重新加载')}</button></div>{/if}
      {#if !$appSettings}<p role="status">{$t(isTauri()?'正在读取设置…':'请在 TaskPulse 应用中打开设置。')}</p>{:else}
        <fieldset disabled={busy}>
          {#if section==='general'}<GeneralSettings/>{:else if section==='panel'}<PanelSettings/>{:else if section==='notifications'}<NotificationSettings/>{:else if section==='updates'}<UpdateSettings blocked={sourceDirty}/>{:else if section==='about'}<AboutSettings/>{/if}
          <div hidden={section!=='sources'}><SourceSettings onDirtyChange={value=>sourceDirty=value} onAttention={()=>void navigate('sources')}/></div>
        </fieldset>
      {/if}
    </div>
    <div class="settings-status" role="status" aria-live="polite">{$t(busy?'正在更新，请稍候…':$autostart.pending?'正在核对系统启动设置…':$preferencePending?'正在保存设置…':'')}</div>
  </main>
</div>
