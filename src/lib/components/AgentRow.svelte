<script lang="ts">
  import type { TaskSnapshot } from '$lib/types';
  import {
    CONFIDENCE_LABEL,
    SOURCE_COLOR,
    SOURCE_LABEL,
    STATUS_COLOR,
    STATUS_LABEL,
    formatAgo,
    formatElapsed,
  } from '$lib/types';
  import StatusDot from './StatusDot.svelte';
  import { SquareTerminal } from '@lucide/svelte';

  let { task, onFocus }: { task: TaskSnapshot; onFocus: () => void } = $props();
  let hovered = $state(false);

  let sourceLabel = $derived(SOURCE_LABEL[task.source] || '');
  let sourceColor = $derived(SOURCE_COLOR[task.source] || '#888');
  let metaLine = $derived(task.remote ? [task.host, task.project].filter(Boolean).join(' · ') : '');

  function statusLine(): string {
    const parts: string[] = [];
    if (task.status === 'succeeded' && task.exit_code === 0) parts.push('退出码 0');
    else if (task.status === 'failed') parts.push(`退出码 ${task.exit_code ?? '?'}`);
    else parts.push(STATUS_LABEL[task.status]);

    if (task.status === 'running' || task.status === 'waiting_input') {
      const elapsed = formatElapsed(task.started_at ?? task.updated_at);
      if (elapsed) parts.push(elapsed);
    } else if (task.finished_at) {
      parts.push(`完成于 ${formatAgo(task.finished_at)}`);
    }
    if (task.remote) parts.push(CONFIDENCE_LABEL[task.confidence]);
    else if (task.cpu != null && task.cpu > 0) parts.push(`CPU ${task.cpu.toFixed(0)}%`);
    if (task.stale) parts.push('信息可能已过期');
    return parts.join(' · ');
  }
</script>

<li
  class="flex items-center gap-[9px] px-3.5 py-[7px] cursor-default transition-all duration-150 rounded-lg mx-1 list-none"
  style="background: {hovered ? 'rgba(255,255,255,0.08)' : 'transparent'}; {hovered ? 'box-shadow: inset 0 0.5px 0 rgba(255,255,255,0.06);' : ''}"
  onmouseenter={() => hovered = true}
  onmouseleave={() => hovered = false}
>
  <StatusDot status={task.status} />

  <div class="flex-1 min-w-0">
    <div class="flex items-center gap-1.5">
      <p class="text-[12px] font-medium text-[#e8e6e1] truncate">{task.name}</p>
      {#if sourceLabel}
        <span
          class="text-[9px] font-semibold px-[5px] py-[1px] rounded-[3px] flex-shrink-0 tracking-wide"
          style="color: {sourceColor}; background: {sourceColor}18; border: 0.5px solid {sourceColor}30;"
        >{sourceLabel}</span>
      {/if}
      {#if task.quota_5h_used_pct != null || task.quota_1w_used_pct != null}
        <span class="text-[9px] text-[#c9c161] flex-shrink-0 tabular-nums" title="Codex 用量：5 小时 / 1 周">
          {task.quota_5h_used_pct != null ? `5h ${Math.round(task.quota_5h_used_pct)}%` : ''}
          {task.quota_5h_used_pct != null && task.quota_1w_used_pct != null ? ' · ' : ''}
          {task.quota_1w_used_pct != null ? `1周 ${Math.round(task.quota_1w_used_pct)}%` : ''}
        </span>
      {/if}
    </div>

    <p class="text-[11px] text-[#aaa89f] truncate mt-[1px]">
      {#if task.remote && metaLine}
        <span class="inline-flex items-center gap-[3px]">
          <SquareTerminal size={11} strokeWidth={1.8} class="inline-block flex-shrink-0" />
          <span>{metaLine}</span>
        </span>
        {#if task.message}<span class="text-[#8f8d86]"> · </span>{/if}
      {/if}
      {task.message || STATUS_LABEL[task.status]}
    </p>

    <p class="text-[10px] truncate mt-[1px] tabular-nums" style="color: {task.stale ? '#b0895a' : '#8f8d86'};">
      {statusLine()}
    </p>
  </div>

  <div class="flex flex-col items-end gap-[3px] flex-shrink-0">
    {#if task.status === 'waiting_input'}
      <span class="text-[10px] font-semibold" style="color: {STATUS_COLOR.waiting_input};">需要处理</span>
    {/if}
    {#if task.can_focus}
      <button
        type="button"
        aria-label="定位到 {task.name} 的终端"
        class="text-[10px] font-medium px-[7px] py-[2px] rounded-md border transition-all duration-150 text-[#5aa8dc] border-[rgba(90,168,220,0.25)] bg-[rgba(90,168,220,0.08)] hover:bg-[rgba(90,168,220,0.18)] hover:border-[rgba(90,168,220,0.45)]"
        class:opacity-0={!hovered}
        class:opacity-100={hovered}
        onclick={onFocus}
      >
        定位 →
      </button>
    {/if}
  </div>
</li>
