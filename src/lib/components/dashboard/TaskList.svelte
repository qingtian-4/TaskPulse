<script lang="ts">
  import { t } from '#lib/i18n.ts';
  import { preferences } from '#lib/preferences.ts';
  import type { TaskSnapshot, DashboardSettings } from '#lib/types.ts';
  import {
    taskContext, taskProjectLabel, taskLabel, taskAgent, taskSummary,
    taskSource, matchesSignal, taskTone, taskElapsed, taskModel
  } from '#lib/dashboard.ts';
  import Pin from '@lucide/svelte/icons/pin';
  import Timer from '@lucide/svelte/icons/timer';

  let { tasks, settings, selectedId, onSelect, now = Date.now(), fitFour = false }: {
    tasks: TaskSnapshot[];
    settings: DashboardSettings | null;
    selectedId: string | null;
    onSelect: (task: TaskSnapshot) => void;
    now?: number;
    fitFour?: boolean;
  } = $props();

  function navigate(event: KeyboardEvent) {
    if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
    const row = event.currentTarget as HTMLButtonElement;
    const rows = Array.from(row.closest('main')?.querySelectorAll<HTMLButtonElement>('.task-row') ?? []);
    const index = rows.indexOf(row);
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? rows.length - 1
      : Math.max(0, Math.min(rows.length - 1, index + (event.key === 'ArrowDown' ? 1 : -1)));
    event.preventDefault();
    rows[next]?.focus();
    rows[next]?.scrollIntoView({ block: 'nearest' });
  }
</script>

{#snippet timing(value: string, label: string)}
  <span class="timing" title={label}>
    <Timer size={13} aria-hidden="true"/>
    <span class="sr-only">{label}</span>
    <span class="elapsed">{value}</span>
  </span>
{/snippet}

<ul class="m-0 p-0 list-none" class:fit-four={fitFour}>
  {#each tasks as task (task.id)}
    {@const elapsed = $preferences.show_elapsed ? taskElapsed(task, now) : null}
    {@const timeLabel = $t(task.status === 'running' ? '运行时长' : '总时长')}
    {@const model = taskModel(task)}
    {@const showMetadata = $preferences.show_model && (model || task.reasoning_effort)}
    {@const source = taskSource(task, settings)}
    {@const summary = taskSummary(task, $t)}
    <li class="task-item tp-tone" class:compact={$preferences.panel_density === 'compact'} data-tone={taskTone(task)}>
      <button type="button" data-task-id={task.id} class:selected={selectedId === task.id}
        class="task-row" onclick={() => onSelect(task)} onkeydown={navigate} aria-pressed={selectedId === task.id}>
        <span class="row-title" title={task.name}>{task.name}</span>
        <span class="row-status">
          <span class="status-label tp-chip">
            {$t(taskLabel(task))}{#if matchesSignal(task,'question') && (task.input_request_ids?.length ?? 0) > 1}<span class="request-count" title={$t('待回答')}>{task.input_request_ids!.length}</span>{/if}
          </span>
        </span>
        <span class="context" title={taskContext(task,settings,$t) + (task.project_path ? ' · ' + task.project_path : '')}>
          <span class="agent">{taskAgent(task,$t)}</span>
          <span class="project">{taskProjectLabel(task,$t)}</span>
          <span class="location-group">
            <span class="location">{task.remote ? 'SSH · ' + (task.host ?? $t('远程')) : source?.kind.endsWith('_wsl') ? 'WSL · ' + source.location : $t('本机')}</span>
            {#if task.unread}<span class="unread" title={$t('尚未查看')}><span class="sr-only">{$t('尚未查看')}</span></span>{/if}
            {#if task.pinned}<Pin size={12} aria-label={$t('固定')}/>{/if}
          </span>
          {#if !showMetadata && elapsed}{@render timing(elapsed, timeLabel)}{/if}
        </span>
        {#if showMetadata}
          <span class="metadata">
            {#if model}<span class="model" title={model}><span class="sr-only">{$t('模型')} </span>{model}</span>{/if}
            {#if task.reasoning_effort}<span class="sr-only">{$t('推理强度')}</span><span class="effort" title={$t('推理强度')}>{task.reasoning_effort}</span>{/if}
            {#if elapsed}{@render timing(elapsed, timeLabel)}{/if}
          </span>
        {/if}
        <span class="activity" title={summary}>{summary}</span>
      </button>
    </li>
  {/each}
</ul>

<style>
  ul { display: flex; flex-direction: column; gap: var(--tp-space-2); padding-top: 2px; }
  .task-item { font-size: var(--tp-task-font-size, 12px); line-height: 1.5; }
  .task-row {
    display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 5px 8px;
    width: 100%; padding: 10px 12px; text-align: start; font: inherit;
    border: 1px solid var(--tp-border); border-radius: var(--tp-radius-md);
    background: var(--tp-card); color: var(--tp-text);
  }
  .task-row:hover { background: var(--tp-raised); border-color: var(--tp-border-strong); }
  .task-row.selected { border-color: var(--tp-accent-line); background: var(--tp-accent-soft); }
  .task-item:is([data-tone=action], [data-tone=danger]) .task-row { box-shadow: inset 2px 0 var(--tone); }
  .row-title {
    min-width: 0; align-self: start; font-weight: 650; overflow-wrap: anywhere;
    display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden;
  }
  .row-status { display: flex; align-self: start; }
  .status-label { max-width: clamp(100px, 7em, 140px); justify-content: center; font-size: inherit; line-height: inherit; text-align: center; }
  .context, .metadata, .activity { grid-column: 1 / -1; min-width: 0; }
  .context, .metadata { display: flex; align-items: center; gap: 6px; color: var(--tp-muted); }
  .agent, .project, .location { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .agent { max-width: 34%; }
  .project { flex: 1; border-inline-start: 1px solid var(--tp-border-strong); padding-inline-start: 6px; }
  .location-group { min-width: 0; max-width: 42%; display: inline-flex; align-items: center; gap: 5px; color: var(--tp-quiet); }
  .location-group :global(svg), .unread { flex: none; }
  .unread { width: 5px; height: 5px; border-radius: 50%; background: var(--tone); }
  .metadata { flex-wrap: wrap; gap: 3px 6px; }
  .model, .effort { min-width: 0; max-width: 100%; overflow-wrap: anywhere; }
  .effort { padding: 0 5px; border-radius: 4px; background: var(--tp-accent-soft); color: var(--tp-accent); font-weight: 550; }
  .timing { display: inline-flex; align-items: center; gap: 3px; flex: none; margin-inline-start: auto; color: var(--tp-quiet); white-space: nowrap; font-variant-numeric: tabular-nums; }
  .activity { color: var(--tp-muted); overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
  .task-item[data-tone=action] .activity { color: var(--tp-action); }
  .fit-four .task-row, .compact .task-row { padding-block: 7px; row-gap: 4px; }
  @media (max-width: 350px) {
    .task-row { padding: 8px 9px; column-gap: 6px; }
    .context { gap: 4px; }
    .project { padding-inline-start: 4px; }
  }
</style>
