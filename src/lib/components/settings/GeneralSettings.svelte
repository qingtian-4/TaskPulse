<script lang="ts">
  import {onMount} from 'svelte';
  import {invoke} from '@tauri-apps/api/core';
  import {preferences,savePreference,preferencePending} from '$lib/preferences';
  import {t} from '$lib/i18n';
  import SettingGroup from './SettingGroup.svelte';
  import SettingRow from './SettingRow.svelte';
  let startup=$state<{enabled:boolean;available:boolean}|null>(null),startupBusy=$state(false),startupError=$state('');
  const themes=[{value:'system',label:'跟随系统'},{value:'light',label:'浅色'},{value:'dark',label:'深色'}] as const;
  function save(patch:Parameters<typeof savePreference>[0],control:HTMLInputElement|HTMLSelectElement){void savePreference(patch,control).catch(()=>{});}
  async function readStartup(){try{startup=await invoke('get_autostart');startupError='';}catch(e){startupError=String(e);}}
  async function setStartup(enabled:boolean,control:HTMLInputElement){startupBusy=true;startupError='';try{startup=await invoke('set_autostart',{enabled});}catch(e){startupError=String(e);}finally{control.checked=startup?.enabled??false;startupBusy=false;}}
  onMount(()=>{void readStartup();window.addEventListener('focus',readStartup);return()=>window.removeEventListener('focus',readStartup);});
</script>
<SettingGroup title={$t('界面')}>
  <div class="appearance-copy"><div class="setting-label">{$t('外观')}</div><p class="setting-description">{$t('跟随系统，或为 TaskPulse 选择固定主题。')}</p></div>
  <div class="theme-picker">{#each themes as theme}<label class="theme-choice" data-preview={theme.value}><span class="mini-window" aria-hidden="true"><span class="mini-side"><i></i><i></i><i></i></span><span class="mini-main"><i></i><i></i><i></i></span></span><span class="theme-caption">{$t(theme.label)}<input type="radio" name="appearance" value={theme.value} checked={$preferences.theme===theme.value} disabled={$preferencePending} onchange={e=>save({preferences:{theme:theme.value}},e.currentTarget)}/></span></label>{/each}</div>
  <SettingRow label={$t('显示语言')} description={$t('更改后立即生效，无需重启。')}><select aria-label={$t('显示语言')} value={$preferences.language} disabled={$preferencePending} onchange={e=>save({preferences:{language:e.currentTarget.value as 'system'|'zh-CN'|'en'}},e.currentTarget)}><option value="system">{$t('跟随系统')}</option><option value="zh-CN">{$t("简体中文")}</option><option value="en">English</option></select></SettingRow>
</SettingGroup>
<SettingGroup title={$t('启动')}>
  <SettingRow label={$t('登录时自动启动')} description={$t(startup?.available===false?$t('请在已安装的 TaskPulse 中设置登录启动。'):$t('登录后，在托盘或菜单栏继续关注任务。'))}><input class="settings-switch" type="checkbox" role="switch" aria-label={$t('登录时自动启动')} checked={startup?.enabled??false} disabled={!startup?.available||startupBusy} onchange={e=>setStartup(e.currentTarget.checked,e.currentTarget)}/></SettingRow>
  <SettingRow label={$t('启动时展开任务面板')} description={$t('关闭时安静地在后台运行。')}><input class="settings-switch" type="checkbox" role="switch" aria-label={$t('启动时展开任务面板')} checked={$preferences.show_on_start} disabled={$preferencePending} onchange={e=>save({preferences:{show_on_start:e.currentTarget.checked}},e.currentTarget)}/></SettingRow>
</SettingGroup>
{#if startupError}<p class="error" role="alert">{startupError}<button class="tp-btn" onclick={readStartup}>{$t('重试')}</button></p>{/if}
<p class="settings-note">{$t('个性化设置会自动保存。监控来源与通知可在左侧分别管理。')}</p>
<style>
.appearance-copy{padding-top:17px}.theme-picker{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:10px;margin:20px 0 8px}.theme-choice{display:grid;gap:9px;padding:9px;border:1px solid var(--tp-border-strong);border-radius:8px;min-width:0;cursor:pointer;background:var(--tp-bg)}.theme-choice:has(input:checked){border-color:var(--tp-accent);box-shadow:0 0 0 1px var(--tp-accent)}.mini-window{height:65px;display:grid;grid-template-columns:32% 1fr;border-radius:5px;overflow:hidden;background:#fff}.mini-side{background:#e8efed;padding:9px 5px}.mini-main{padding:12px 9px}.mini-window i{display:block;height:4px;border-radius:2px;margin-bottom:6px;background:#c2d1cd}.mini-main i{background:#dce6e3}.mini-main i:first-child{background:#72b9a9;width:65%}.theme-choice[data-preview=dark] .mini-window{background:#15242c}.theme-choice[data-preview=dark] .mini-side{background:#10191e}.theme-choice[data-preview=dark] i{background:#33494f}.theme-choice[data-preview=dark] .mini-main i:first-child{background:#76d6c4}.theme-choice[data-preview=system] .mini-window{background:linear-gradient(90deg,#fff 50%,#15242c 50%)}.theme-caption{display:flex;align-items:center;justify-content:space-between;gap:4px;font-size:12px}.theme-caption input{accent-color:var(--tp-accent);margin:0}
@media(max-width:600px){.theme-picker{gap:8px}.theme-choice{padding:7px}.mini-window{height:55px}.mini-main{padding:10px 7px}.theme-caption{font-size:11px}}
</style>
