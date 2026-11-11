<script lang="ts">
  import {appSettings,preferences,savePreference,preferencePending,DEFAULT_TASK_FONT_SIZE} from '$lib/preferences';
  import {t} from '$lib/i18n';
  import SettingGroup from './SettingGroup.svelte';import SettingRow from './SettingRow.svelte';
  function save(patch:Parameters<typeof savePreference>[0],control:HTMLInputElement|HTMLSelectElement){void savePreference(patch,control).catch(()=>{});}
</script>
<SettingGroup title={$t('信息与布局')}>
  <SettingRow label={$t('默认分组')}><select aria-label={$t('默认分组')} value={$appSettings?.default_view??'tasks'} disabled={$preferencePending} onchange={e=>save({default_view:e.currentTarget.value as 'tasks'|'projects'},e.currentTarget)}><option value="tasks">{$t('按任务')}</option><option value="projects">{$t('按项目')}</option></select></SettingRow>
  <SettingRow label={$t('显示任务模型')}><input class="settings-switch" type="checkbox" role="switch" aria-label={$t('显示任务模型')} checked={$preferences.show_model} disabled={$preferencePending} onchange={e=>save({preferences:{show_model:e.currentTarget.checked}},e.currentTarget)}/></SettingRow>
  <SettingRow label={$t('显示运行时间')}><input class="settings-switch" type="checkbox" role="switch" aria-label={$t('显示运行时间')} checked={$preferences.show_elapsed} disabled={$preferencePending} onchange={e=>save({preferences:{show_elapsed:e.currentTarget.checked}},e.currentTarget)}/></SettingRow>
  <SettingRow label={$t('任务字号')}><select aria-label={$t('任务字号')} value={$preferences.task_font_size??DEFAULT_TASK_FONT_SIZE} disabled={$preferencePending} onchange={e=>save({preferences:{task_font_size:Number(e.currentTarget.value)}},e.currentTarget)}>{#each [12,13,14,15,16,17,18,19,20] as size}<option value={size}>{size} px</option>{/each}</select></SettingRow>
  <SettingRow label={$t('面板密度')}><select aria-label={$t('面板密度')} value={$preferences.panel_density} disabled={$preferencePending} onchange={e=>save({preferences:{panel_density:e.currentTarget.value as 'comfortable'|'compact'}},e.currentTarget)}><option value="comfortable">{$t('舒适')}</option><option value="compact">{$t('紧凑')}</option></select></SettingRow>
</SettingGroup>
<SettingGroup title={$t('关注范围')}><SettingRow label={$t('最近活动')} description={$appSettings?.recent_days===0?undefined:$t('较早的任务移入归档。')}><select aria-label={$t('最近活动')} value={$appSettings?.recent_days??7} disabled={$preferencePending} onchange={e=>save({recent_days:Number(e.currentTarget.value)},e.currentTarget)}>{#each [3,7,14,30,0] as days}<option value={days}>{days?`${days} ${$t('天')}`:$t('全部关注')}</option>{/each}</select></SettingRow></SettingGroup>
