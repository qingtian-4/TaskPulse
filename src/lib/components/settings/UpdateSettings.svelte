<script lang="ts">
  import {appSettings,savePreference,preferencePending} from '$lib/preferences';import {t} from '$lib/i18n';import SettingGroup from './SettingGroup.svelte';import SettingRow from './SettingRow.svelte';import AppUpdates from '../AppUpdates.svelte';
  let {blocked=false}:{blocked?:boolean}=$props();
</script>
<AppUpdates blocked={blocked||$preferencePending}/>
<SettingGroup title={$t('更新偏好')}><SettingRow label={$t('自动检查更新')} description={$t('每天检查一次。有新版本时提示，由你点击安装。')}><input class="settings-switch" type="checkbox" role="switch" aria-label={$t('自动检查更新')} checked={$appSettings?.auto_check_updates??true} disabled={$preferencePending} onchange={e=>{void savePreference({auto_check_updates:e.currentTarget.checked}).catch(()=>{});}}/></SettingRow></SettingGroup>
<p class="settings-note">{$t('更新安装前会验证签名。任务数据与个人设置会保留。')}</p>
