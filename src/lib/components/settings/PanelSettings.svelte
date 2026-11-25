<script lang="ts">
  import {
    appSettings, preferences, preferencePending, savePreferenceFromControl,
    DEFAULT_TASK_FONT_SIZE
  } from '#lib/preferences.ts';
  import {t} from '#lib/i18n.ts';
  import SettingGroup from './SettingGroup.svelte';
  import SettingRow from './SettingRow.svelte';

  const recentDays = $derived($appSettings?.recent_days ?? 7);
  let invalidDays = $state(false);

  function changeRecentDays(control: HTMLInputElement) {
    invalidDays = !control.validity.valid;
    if (invalidDays) {
      control.value = String(recentDays);
      return;
    }
    if (control.valueAsNumber !== recentDays) {
      savePreferenceFromControl({recent_days: control.valueAsNumber}, control);
    }
  }
</script>

<SettingGroup title={$t('信息与布局')}>
  <SettingRow label={$t('默认分组')}>
    <select aria-label={$t('默认分组')} value={$appSettings?.default_view ?? 'tasks'} disabled={$preferencePending}
      onchange={e => savePreferenceFromControl({default_view: e.currentTarget.value as 'tasks' | 'projects'}, e.currentTarget)}>
      <option value="tasks">{$t('按任务')}</option>
      <option value="projects">{$t('按项目')}</option>
    </select>
  </SettingRow>
  <SettingRow label={$t('显示任务模型')}>
    <input class="settings-switch" type="checkbox" role="switch" aria-label={$t('显示任务模型')}
      checked={$preferences.show_model} disabled={$preferencePending}
      onchange={e => savePreferenceFromControl({preferences: {show_model: e.currentTarget.checked}}, e.currentTarget)} />
  </SettingRow>
  <SettingRow label={$t('显示运行时间')}>
    <input class="settings-switch" type="checkbox" role="switch" aria-label={$t('显示运行时间')}
      checked={$preferences.show_elapsed} disabled={$preferencePending}
      onchange={e => savePreferenceFromControl({preferences: {show_elapsed: e.currentTarget.checked}}, e.currentTarget)} />
  </SettingRow>
  <SettingRow label={$t('任务字号')}>
    <select aria-label={$t('任务字号')} value={$preferences.task_font_size ?? DEFAULT_TASK_FONT_SIZE} disabled={$preferencePending}
      onchange={e => savePreferenceFromControl({preferences: {task_font_size: Number(e.currentTarget.value)}}, e.currentTarget)}>
      {#each [12,13,14,15,16,17,18,19,20] as size}
        <option value={size}>{size} px</option>
      {/each}
    </select>
  </SettingRow>
  <SettingRow label={$t('面板密度')}>
    <select aria-label={$t('面板密度')} value={$preferences.panel_density} disabled={$preferencePending}
      onchange={e => savePreferenceFromControl({preferences: {panel_density: e.currentTarget.value as 'comfortable' | 'compact'}}, e.currentTarget)}>
      <option value="comfortable">{$t('舒适')}</option>
      <option value="compact">{$t('紧凑')}</option>
    </select>
  </SettingRow>
</SettingGroup>

<SettingGroup title={$t('关注范围')}>
  <SettingRow label={$t('最近活动')} descriptionId="recent-days-hint"
    description={$t('按最近活动天数归档较早的任务。填 0 可显示全部。')}>
    <label class="days-control">
      <input type="number" required min="0" max="365" step="1" value={recentDays}
        list="recent-day-presets" aria-label={$t('最近活动')} aria-describedby="recent-days-hint"
        disabled={$preferencePending} oninput={() => invalidDays = false} onchange={e => changeRecentDays(e.currentTarget)} />
      <span>{$t('天')}</span>
    </label>
    <datalist id="recent-day-presets">
      {#each [0, 3, 7, 14, 30] as days}<option value={days}></option>{/each}
    </datalist>
  </SettingRow>
  {#if invalidDays}
    <p class="days-error" role="alert">{$t('请输入 0 到 365 之间的整数。已保留原设置。')}</p>
  {/if}
</SettingGroup>

<style>
  .days-control { display: flex; align-items: center; gap: 8px; color: var(--tp-muted); }
  .days-control input { width: 5.5rem; font-variant-numeric: tabular-nums; }
  .days-error { margin: 0; padding: 10px 0; font-size: 13px; color: var(--tp-danger); }
</style>
