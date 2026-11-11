<script lang="ts">
  import {invoke} from '@tauri-apps/api/core';import {appSettings,savePreference,preferencePending} from '$lib/preferences';import {t} from '$lib/i18n';
  import SettingGroup from './SettingGroup.svelte';import SettingRow from './SettingRow.svelte';
  let testResult=$state(''),testError=$state(''),busy=$state(false);
  function save(patch:Parameters<typeof savePreference>[0],control:HTMLInputElement|HTMLSelectElement){void savePreference(patch,control).catch(()=>{});}
  async function test(){busy=true;testError='';testResult='';try{await invoke('test_notification');testResult='测试提醒已发送。';}catch(e){testError=String(e);}finally{busy=false;}}
</script>
<SettingGroup title={$t('提醒方式')}>
  <SettingRow label={$t('启用任务提醒')}><input class="settings-switch" type="checkbox" role="switch" aria-label={$t('启用任务提醒')} checked={$appSettings?.notifications??true} disabled={$preferencePending} onchange={e=>save({notifications:e.currentTarget.checked},e.currentTarget)}/></SettingRow>
  <SettingRow label={$t('提醒通道')}><select aria-label={$t('提醒通道')} value={$appSettings?.notification_mode??'popup'} disabled={$preferencePending||!$appSettings?.notifications} onchange={e=>save({notification_mode:e.currentTarget.value as 'popup'|'system'|'both'},e.currentTarget)}><option value="popup">{$t('TaskPulse 弹窗')}</option><option value="system">{$t('系统通知')}</option><option value="both">{$t('两者同时')}</option></select></SettingRow>
  <SettingRow label={$t('播放提醒声音')}><input class="settings-switch" type="checkbox" role="switch" aria-label={$t('播放提醒声音')} checked={$appSettings?.sound??false} disabled={$preferencePending||!$appSettings?.notifications} onchange={e=>save({sound:e.currentTarget.checked},e.currentTarget)}/></SettingRow>
  <SettingRow label={$t('测试提醒')}><button class="tp-btn" disabled={busy||$preferencePending||!$appSettings?.notifications} onclick={test}>{$t('发送测试提醒')}</button></SettingRow>
</SettingGroup>
{#if testError}<p class="settings-inline-error" role="alert">{$t(testError)}</p>{:else if testResult}<p class="setting-description" role="status">{$t(testResult)}</p>{/if}
