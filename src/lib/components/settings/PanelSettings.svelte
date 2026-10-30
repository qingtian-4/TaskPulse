<script lang="ts">
  import {appSettings,preferences,savePreference,preferencePending} from '$lib/preferences';
  import {t} from '$lib/i18n';
  import SettingGroup from './SettingGroup.svelte';import SettingRow from './SettingRow.svelte';
  function save(patch:Parameters<typeof savePreference>[0]){void savePreference(patch).catch(()=>{});}
</script>
<SettingGroup title={$t('信息与布局')}>
  <SettingRow label={$t('默认分组')} description={$t('按任务查看，或把同一项目放在一起。')}><select aria-label={$t('默认分组')} value={$appSettings?.default_view??'tasks'} disabled={$preferencePending} onchange={e=>save({default_view:e.currentTarget.value as 'tasks'|'projects'})}><option value="tasks">{$t('按任务')}</option><option value="projects">{$t('按项目')}</option></select></SettingRow>
  <SettingRow label={$t('显示任务模型')} description={$t('使用任务实际报告的模型名称。')}><input class="settings-switch" type="checkbox" role="switch" aria-label={$t('显示任务模型')} checked={$preferences.show_model} disabled={$preferencePending} onchange={e=>save({preferences:{show_model:e.currentTarget.checked}})}/></SettingRow>
  <SettingRow label={$t('显示运行时间')} description={$t('在任务旁显示本轮运行的时长。')}><input class="settings-switch" type="checkbox" role="switch" aria-label={$t('显示运行时间')} checked={$preferences.show_elapsed} disabled={$preferencePending} onchange={e=>save({preferences:{show_elapsed:e.currentTarget.checked}})}/></SettingRow>
  <SettingRow label={$t('面板密度')}><select aria-label={$t('面板密度')} value={$preferences.panel_density} disabled={$preferencePending} onchange={e=>save({preferences:{panel_density:e.currentTarget.value as 'comfortable'|'compact'}})}><option value="comfortable">{$t('舒适')}</option><option value="compact">{$t('紧凑')}</option></select></SettingRow>
</SettingGroup>
<SettingGroup title={$t('关注范围')}><SettingRow label={$t('最近活动')} description={$t($appSettings?.recent_days===0?'显示全部关注，不按活动时间归档。':'较早的关注移入归档，未读与置顶记录仍会保留。')}><select aria-label={$t('最近活动')} value={$appSettings?.recent_days??7} disabled={$preferencePending} onchange={e=>save({recent_days:Number(e.currentTarget.value)})}>{#each [3,7,14,30,0] as days}<option value={days}>{days?`${days} ${$t('天')}`:$t('全部关注')}</option>{/each}</select></SettingRow></SettingGroup>
<p class="settings-note">{$t('运行中的任务和你手动置顶的任务始终保留。')}</p>
