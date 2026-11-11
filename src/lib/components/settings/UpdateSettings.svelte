<script lang="ts">
  import {Clock3} from '@lucide/svelte';
  import {appSettings,savePreference,preferencePending} from '$lib/preferences';
  import {t} from '$lib/i18n';
  import AppUpdates from '../AppUpdates.svelte';
  let {blocked=false}:{blocked?:boolean}=$props();
</script>

<div class="update-stack">
  <AppUpdates blocked={blocked||$preferencePending}/>
  <div class="settings-group">
    <label class="setting-row preference-row">
      <Clock3 size={18} class="update-icon" aria-hidden="true"/>
      <span class="setting-label setting-copy">{$t('自动检查更新')}</span>
      <input class="settings-switch" type="checkbox" role="switch" checked={$appSettings?.auto_check_updates??true} disabled={$preferencePending} onchange={e=>{void savePreference({auto_check_updates:e.currentTarget.checked},e.currentTarget).catch(()=>{});}}/>
    </label>
  </div>
</div>

<style>
  .update-stack{display:grid;grid-template-columns:minmax(0,1fr);gap:6px}
  .update-stack > :global(.settings-group){border:1px solid var(--tp-border)}
  .preference-row{gap:12px;min-height:66px}
  .preference-row :global(.update-icon){flex:none;color:var(--tp-muted)}
</style>
