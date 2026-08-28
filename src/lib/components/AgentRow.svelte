<script lang="ts">
  import type { TaskSnapshot } from '$lib/types';
  import {
    STATUS_LABEL,
    STATUS_COLOR,
    SOURCE_LABEL,
    SOURCE_COLOR,
    CONFIDENCE_LABEL,
    formatElapsed,
    formatAgo,
  } from '$lib/types';
  import StatusDot from './StatusDot.svelte';
  import { SquareTerminal } from '@lucide/svelte';

  let {
    task,
    onFocus,
  }: { task: TaskSnapshot; onFocus: () => void } = $props();

  let hovered = $state(false);

  let sourceLabel = $derived(SOURCE_LABEL[task.source] || '');
  let sourceColor = $derived(SOURCE_COLOR[task.source] || '#888');

  // Line 2: host · project (remote) or terminal chip (local)
  let metaLine = $derived(
    task.remote
      ? [task.host, task.project].filter(Boolean).join(' · ')
      : ''
  );

  // Line 3: status · elapsed · confidence
  function statusLine(): string {
    const parts: string[] = [];
    if (task.status === 'waiting_input') parts.push('waiting input');
    else if (task.status === 'succeeded' && task.exit_code === 0) parts.push('exit 0');
    else if (task.status === 'failed') parts.push(`exit ${task.exit_code ?? '?'}`);
    else parts.push(STATUS_LABEL[task.status]);

    if (task.remote) {
      if (task.status === 'running' || task.status === 'waiting_input') {
        const el = formatElapsed(task.started_at ?? task.updated_at);
        if (el) parts.push(el);
      } else if (task.finished_at) {
        parts.push(`finished ${formatAgo(task.finished_at)}`);
      }
      parts.push(CONFIDENCE_LABEL[task.confidence]);
    } else if (task.cpu != null && task.cpu > 0) {
      parts.push(`${task.cpu.toFixed(0)}% cpu`);
    }
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
          class="text-[9px] font-semibold px-[5px] py-[1px] rounded-[3px] flex-shrink-0 uppercase tracking-wide"
          style="color: {sourceColor}; background: {sourceColor}18; border: 0.5px solid {sourceColor}30;"
        >{sourceLabel}</span>
      {/if}
      {#if task.status === 'succeeded'}
        <span class="text-[10px] text-[#78b644] flex-shrink-0">✓</span>
      {:else if task.status === 'failed'}
        <span class="text-[10px] text-[#dd4f4f] flex-shrink-0">✗</span>
      {/if}
      {#if task.quota_5h_used_pct != null || task.quota_1w_used_pct != null}
        <span class="text-[9px] text-[#c9c161] flex-shrink-0 tabular-nums" title="Codex 用量：5 小时 / 1 周">
          {task.quota_5h_used_pct != null ? `5h ${Math.round(task.quota_5h_used_pct)}%` : ''}
          {task.quota_5h_used_pct != null && task.quota_1w_used_pct != null ? ' · ' : ''}
          {task.quota_1w_used_pct != null ? `1周 ${Math.round(task.quota_1w_used_pct)}%` : ''}
        </span>
      {/if}
    </div>

    {#if task.remote}
      <p class="text-[11px] text-[#9a9890] truncate mt-[1px]">
        {#if metaLine}
          <span class="inline-flex items-center gap-[3px]">
            <SquareTerminal size={11} strokeWidth={1.8} class="inline-block flex-shrink-0" />
            <span>{metaLine}</span>
          </span>
        {/if}
        {#if task.message}
          {#if metaLine}<span class="text-[#8f8d86]"> · </span>{/if}
          <span>{task.message}</span>
        {/if}
      </p>
    {:else}
      <p class="text-[11px] text-[#aaa89f] truncate mt-[1px]">
        {task.message || STATUS_LABEL[task.status]}
      </p>
    {/if}

    {#if statusLine()}
      <p class="text-[10px] text-[#8f8d86] truncate mt-[1px] tabular-nums" style="color: {task.stale ? '#b0895a' : '#8f8d86'};">
        {statusLine()}
        {#if task.stale}<span class="text-[#b0895a]"> · stale</span>{/if}
      </p>
    {/if}
  </div>

  <div class="flex flex-col items-end gap-[3px] flex-shrink-0">
    {#if task.status === 'waiting_input'}
      <span class="text-[10px] font-mono font-semibold tabular-nums" style="color: {STATUS_COLOR.waiting_input};">needs you</span>
    {/if}
    {#if task.can_focus}
      <button
        type="button"
        aria-label="Focus terminal for {task.name}"
        class="text-[10px] font-medium px-[7px] py-[2px] rounded-md border transition-all duration-150
               text-[#5aa8dc] border-[rgba(90,168,220,0.25)] bg-[rgba(90,168,220,0.08)]
               hover:bg-[rgba(90,168,220,0.18)] hover:border-[rgba(90,168,220,0.45)]
               disabled:opacity-30"
        style="backdrop-filter: blur(8px); -webkit-backdrop-filter: blur(8px);"
        class:opacity-0={!hovered}
        class:opacity-100={hovered}
        onclick={onFocus}
      >
        focus ↗
      </button>
    {/if}
  </div>
</li>
