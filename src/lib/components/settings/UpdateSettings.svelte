<script lang="ts">
  import {appSettings,savePreference,preferencePending} from '$lib/preferences';import {t} from '$lib/i18n';import SettingGroup from './SettingGroup.svelte';import SettingRow from './SettingRow.svelte';import AppUpdates from '../AppUpdates.svelte';
  let {blocked=false}:{blocked?:boolean}=$props();
</script>
<AppUpdates blocked={blocked||$preferencePending}/>
<SettingGroup title={$t('更新偏好')}><SettingRow label={$t('自动检查更新')}><input class="settings-switch" type="checkbox" role="switch" aria-label={$t('自动检查更新')} checked={$appSettings?.auto_check_updates??true} disabled={$preferencePending} onchange={e=>{void savePreference({auto_check_updates:e.currentTarget.checked},e.currentTarget).catch(()=>{});}}/></SettingRow></SettingGroup>
