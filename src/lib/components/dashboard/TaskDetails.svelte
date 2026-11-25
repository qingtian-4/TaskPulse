<script lang="ts">
  import X from '@lucide/svelte/icons/x';
  import { t } from '$lib/i18n';
  import { preferences } from '$lib/preferences';
  import { taskActivity, taskContext, taskElapsed, taskLabel, taskModel, taskTone, sourceFreshness } from '$lib/dashboard';
  import { CONFIDENCE_LABEL, formatAgo, type DashboardSettings, type ProviderHealth, type TaskSnapshot } from '$lib/types';

  let { task, settings, providers, now, archived, onClose, onOpen, onCopy, onAction }: {
    task: TaskSnapshot;
    settings: DashboardSettings | null;
    providers: ProviderHealth[];
    now: number;
    archived: boolean;
    onClose: () => void;
    onOpen: (task: TaskSnapshot) => void;
    onCopy: (task: TaskSnapshot) => void;
    onAction: (task: TaskSnapshot, name: string, value?: boolean) => void;
  } = $props();
  const model = $derived($preferences.show_model ? taskModel(task) : undefined);
  const effort = $derived($preferences.show_model ? task.reasoning_effort : undefined);
  const elapsed = $derived($preferences.show_elapsed ? taskElapsed(task, now) : null);
</script>

<aside class="detail glass-scroll tp-tone" data-tone={taskTone(task)} aria-label={$t('任务详情')}>
  <div class="detail-heading">
    <strong title={task.name}>{task.name}</strong>
    <span class="tp-chip">{$t(taskLabel(task))}</span>
    <button class="tp-icon" onclick={onClose} aria-label={$t('关闭详情')}><X size={16}/></button>
  </div>
  <p class="context">{taskContext(task, settings, $t)}</p>

  {#if model || effort || elapsed}
    <dl class="facts">
      {#if model}<div class="model-detail"><dt>{$t('模型')}</dt><dd>{model}</dd></div>{/if}
      {#if effort}<div><dt>{$t('推理强度')}</dt><dd>{effort}</dd></div>{/if}
      {#if elapsed}<div><dt>{$t(task.status === 'running' ? '运行时长' : '总时长')}</dt><dd>{elapsed}</dd></div>{/if}
    </dl>
  {/if}
  <div class="command glass-scroll">
    <span>{task.stale ? $t('上次观测活动（非实时）') : task.status === 'running' ? $t('当前活动') : $t('最近消息')}</span>
    <pre>{taskActivity(task, $t)}</pre>
  </div>
  <details class="evidence tp-fold">
    <summary>{$t('状态依据与采集时间')}</summary>
    <p>{$t('状态依据：')}{$t(CONFIDENCE_LABEL[task.confidence])}</p>
    <p>{sourceFreshness(task,settings,providers,now,$t)} {$t('· 最后活动')} {$t(formatAgo(task.updated_at,now))}</p>
  </details>
  <div class="detail-actions">
    {#if task.can_focus}
      <button class="tp-btn" title={$t('只聚焦宿主窗口，不标记已查看')} onclick={() => onOpen(task)}>
        {task.terminal?.kind === 'windows_native' ? $t('聚焦运行窗口') : $t('请求返回终端')}
      </button>
    {/if}
    <button class="tp-btn" data-variant="ghost" onclick={() => onCopy(task)}>{$t('复制任务信息')}</button>
    {#if task.unread}
      <button class="tp-btn" data-variant="primary" title={$t('更新本轮已查看标记，不代表批准或项目完成')} onclick={() => onAction(task,'viewed')}>{$t('已查看')}</button>
    {/if}
    {#if archived}<button class="tp-btn" onclick={() => onAction(task,'follow')}>{$t('重新加入')}</button>{/if}
    <button class="tp-btn" data-variant="ghost" onclick={() => onAction(task,'pin',archived || !task.pinned)}>
      {archived ? $t('固定到近期面板') : task.pinned ? $t('取消固定') : $t('固定')}
    </button>
    <details class="more tp-fold">
      <summary>{$t('更多关注操作')}</summary>
      {#if task.status !== 'running'}<button onclick={() => onAction(task,'hide')}>{$t('隐藏本轮（新执行会恢复）')}</button>{/if}
      <button onclick={() => onAction(task,'ignore')}>{$t('停止监控此会话（需手动恢复）')}</button>
    </details>
  </div>
</aside>

<style>
  .detail {
    flex: 0 1 auto; min-height: 96px; max-height: 32dvh; overflow: auto; overscroll-behavior: contain;
    margin: 0 12px 8px; padding: 12px; border: 1px solid var(--tone-line); border-radius: var(--tp-radius-lg);
    background: var(--tp-card); font-size: var(--tp-task-font-size, 12px); line-height: 1.5; overflow-wrap: anywhere;
  }
  .detail-heading { display: grid; grid-template-columns: minmax(0,1fr) auto auto; align-items: start; gap: 6px; }
  .detail-heading strong { font-size: max(14px,1em); font-weight: 650; }
  .detail-heading .tp-chip { max-width: clamp(100px, 6em, 128px); font-size: inherit; }
  .detail-heading button { width: 28px; height: 28px; }
  .context { margin: 2px 0 8px; color: var(--tp-muted); }
  .facts {
    display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 7em), 1fr)); gap: 8px 12px;
    padding-block: 8px; margin-block: 8px; border-block: 1px solid var(--tp-border);
  }
  .facts > div { min-width: 0; }
  .model-detail { grid-column: 1 / -1; }
  dt, .command span { color: var(--tp-muted); }
  dd { margin: 1px 0 0; font-weight: 600; font-variant-numeric: tabular-nums; }
  .command { max-height: 6.5em; overflow: auto; padding: 6px 9px; border-radius: var(--tp-radius-sm); background: var(--tp-input); }
  .command pre { font: inherit; white-space: pre-wrap; overflow-wrap: anywhere; margin: 2px 0 0; }
  .evidence { margin-block: 8px; color: var(--tp-muted); }
  .evidence p { margin: 4px 8px; }
  summary, .more button { padding: 5px 8px; }
  .detail-actions { display: flex; flex-wrap: wrap; gap: 4px 8px; }
  .detail-actions > .tp-btn { font-size: inherit; min-height: 28px; padding: 4px 8px; }
  .more { width: 100%; }
  .more button { display: block; text-align: start; }
  @media (max-width: 350px) { .detail { margin-inline: 8px; padding: 8px; } }
  @media (max-height: 500px) { .detail { max-height: 30dvh; min-height: 80px; padding: 8px; } }
</style>
