<script lang="ts">
  import {onMount,tick} from 'svelte';
  import {SlidersHorizontal,PanelsTopLeft,RadioTower,Bell,RefreshCw,Info} from '@lucide/svelte';
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
  let busy=$state(false),loadError=$state(''),sourceReviewRequest=$state(0);
  const positions:Partial<Record<Section,number>>={};
  const sections=[['general','通用',SlidersHorizontal],['panel','任务面板',PanelsTopLeft],['sources','监控来源',RadioTower],['notifications','通知提醒',Bell],['updates','应用更新',RefreshCw],['about','关于',Info]] as const;
  const current=$derived(sections.find(item=>item[0]===section)!);
  async function navigate(next:Section,review=false){if(scroll)positions[section]=scroll.scrollTop;section=next;if(review)sourceReviewRequest++;await tick();if(scroll)scroll.scrollTop=review?0:positions[next]??0;}
  async function reload(){try{appSettings.set(await invoke<DashboardSettings>('get_monitoring_settings'));preferenceError.set('');loadError='';}catch(e){loadError=String(e);}}
  onMount(()=>{
    const url=new URLSearchParams(location.search),requested=url.get('section');if(sections.some(s=>s[0]===requested))section=requested as Section;if(section==='sources'&&url.get('reviewSources')==='1')sourceReviewRequest++;
    if(!isTauri())return;
    let active=true;const cleanups:Array<()=>void>=[];
    void(async()=>{
      const stopSection=await listen<string>('management-section',e=>{if(sections.some(s=>s[0]===e.payload))void navigate(e.payload as Section);});if(active)cleanups.push(stopSection);else stopSection();
      const stopReview=await listen('management-source-review',()=>void navigate('sources',true));if(active)cleanups.push(stopReview);else stopReview();
      const stopUpdate=await listen<{phase:string}>('application-update',e=>busy=['downloading','verifying','installing'].includes(e.payload.phase));if(active)cleanups.push(stopUpdate);else stopUpdate();
      const status=await invoke<{phase:string}>('get_update_status');busy=['downloading','verifying','installing'].includes(status.phase);

    })().catch(e=>loadError=String(e));
    return()=>{active=false;cleanups.forEach(fn=>fn());};
  });
</script>
<div class="settings-shell tp-theme" class:monitoring-shell={section==='sources'}>
  <aside class="settings-side"><div class="settings-brand"><BrandMark size={34}/><div><strong>TaskPulse</strong></div></div>
    <nav class="settings-nav" aria-label={$t('设置分类')}>{#each sections as [id,label,Icon]}<a href={'#'+id} aria-current={section===id?'page':undefined} onclick={e=>{e.preventDefault();void navigate(id);}}><Icon size={17} aria-hidden="true"/><span>{$t(label)}</span></a>{/each}</nav>
  </aside>
  <main class="settings-main" class:source-page={section==='sources'}>{#if section!=='sources'}<header class="settings-heading"><h1>{$t(current[1])}</h1></header>{/if}
    <div class="settings-content glass-scroll" class:source-layout={section==='sources'} bind:this={scroll}>
      {#if $preferenceError||loadError}<div class="settings-inline-error" role="alert">{$t($preferenceError||loadError)} <button class="tp-btn" onclick={reload}>{$t('重新加载')}</button></div>{/if}
      {#if !$appSettings}<p role="status">{$t(isTauri()?'正在读取设置…':'请在 TaskPulse 应用中打开设置。')}</p>{:else}
        <fieldset disabled={busy} class:source-pane={section==='sources'}>
          {#if section==='general'}<GeneralSettings/>{:else if section==='panel'}<PanelSettings/>{:else if section==='notifications'}<NotificationSettings/>{:else if section==='updates'}<UpdateSettings blocked={sourceDirty}/>{:else if section==='about'}<AboutSettings/>{/if}
          <div class:source-host={section==='sources'} hidden={section!=='sources'}><SourceSettings reviewRequest={sourceReviewRequest} onDirtyChange={value=>sourceDirty=value} onAttention={()=>void navigate('sources')}/></div>
        </fieldset>
      {/if}
    </div>
    {#if section!=='sources'}<div class="settings-status" role="status" aria-live="polite">{$t(busy?'正在更新，请稍候…':$autostart.pending?'正在核对系统启动设置…':$preferencePending?'正在保存设置…':'')}</div>{/if}
  </main>
</div>
