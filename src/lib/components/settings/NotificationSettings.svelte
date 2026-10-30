<script lang="ts">
  import {invoke} from '@tauri-apps/api/core';import {appSettings,savePreference,preferencePending} from '$lib/preferences';import {t} from '$lib/i18n';
  import SettingGroup from './SettingGroup.svelte';import SettingRow from './SettingRow.svelte';
  let testResult=$state(''),busy=$state(false);
  function save(patch:Parameters<typeof savePreference>[0],control:HTMLInputElement|HTMLSelectElement){void savePreference(patch,control).catch(()=>{});}
  async function test(){busy=true;try{await invoke('test_notification');testResult='测试提醒已发送。';}catch(e){testResult=String(e);}finally{busy=false;}}
</script>
<SettingGroup title={$t('提醒方式')}>
  <SettingRow label={$t('启用任务提醒')} description={$t('任务完成、失败或明确需要你输入时提醒。')}><input class="settings-switch" type="checkbox" role="switch" aria-label={$t('启用任务提醒')} checked={$appSettings?.notifications??true} disabled={$preferencePending} onchange={e=>save({notifications:e.currentTarget.checked},e.currentTarget)}/></SettingRow>
  <SettingRow label={$t('提醒通道')} description={$t('TaskPulse 提醒会保留需要处理的请求。')}><select aria-label={$t('提醒通道')} value={$appSettings?.notification_mode??'popup'} disabled={$preferencePending||!$appSettings?.notifications} onchange={e=>save({notification_mode:e.currentTarget.value as 'popup'|'system'|'both'},e.currentTarget)}><option value="popup">{$t('TaskPulse 弹窗')}</option><option value="system">{$t('系统通知')}</option><option value="both">{$t('两者同时')}</option></select></SettingRow>
  <SettingRow label={$t('播放提醒声音')} description={$t('系统通知的显示受系统勿扰设置影响。')}><input class="settings-switch" type="checkbox" role="switch" aria-label={$t('播放提醒声音')} checked={$appSettings?.sound??false} disabled={$preferencePending||!$appSettings?.notifications} onchange={e=>save({sound:e.currentTarget.checked},e.currentTarget)}/></SettingRow>
</SettingGroup>
<SettingGroup title={$t('提醒预览')}><SettingRow label={$t('确认提醒的样式')} description={$t('测试提醒不会创建监控任务。')}><button class="tp-btn" disabled={busy||$preferencePending||!$appSettings?.notifications} onclick={test}>{$t('发送测试提醒')}</button></SettingRow></SettingGroup>
<p class="settings-note">{$t('普通结果在 15 秒后收起；需要处理的请求持续保留。收起提醒不会把任务标记为已读。')}</p>
{#if testResult}<p role="status">{$t(testResult)}</p>{/if}
