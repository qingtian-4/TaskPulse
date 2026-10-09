<script lang="ts">
  import type { Status, TaskSnapshot } from '$lib/types';
  import { STATUS_LABEL, STATUS_PRIORITY, formatElapsed } from '$lib/types';
  import StatusDot from './StatusDot.svelte';

  let {
    tasks,
    selectedId = null,
    title,
    onSelect,
  }: {
    tasks: TaskSnapshot[];
    selectedId?: string | null;
    title: string;
    onSelect?: (id: string) => void;
  } = $props();

  const groups = $derived.by(() => {
    const projects = new Map<string, TaskSnapshot[]>();
    const flat: TaskSnapshot[] = [];
    for (const task of tasks) {
      const project = (task.project ?? '').trim();
      if (!project) {
        flat.push(task);
        continue;
      }
      const projectTasks = projects.get(project) ?? [];
      projectTasks.push(task);
      projects.set(project, projectTasks);
    }
    return {
      projects: [...projects.entries()].map(([project, projectTasks]) => ({
        project,
        tasks: projectTasks,
      })),
      flat,
    };
  });

  let open = $state<Record<string, boolean>>({});

  function groupStatus(groupTasks: TaskSnapshot[]): Status {
    return groupTasks.reduce<Status>(
      (best, task) => STATUS_PRIORITY[task.status] < STATUS_PRIORITY[best] ? task.status : best,
      'unknown',
    );
  }

  function shouldOpen(groupTasks: TaskSnapshot[]): boolean {
    // The backend has already reduced history to an attention list, so every
    // remaining task deserves to be visible without another click.
    return groupTasks.length > 0;
  }

  function activityLabel(task: TaskSnapshot): string {
    if (task.stale) return '采集暂不可用，显示上次状态';
    if (task.status === 'running') return task.activity ?? task.message ?? '正在分析 / 执行';
    if (task.status === 'waiting_input') return '等待你的输入或确认';
    if (task.status === 'failed') return '执行失败，需要处理';
    if (task.status === 'succeeded') return task.unread ? '执行完成，尚未查看' : '执行完成';
    return STATUS_LABEL[task.status];
  }
</script>

{#snippet taskSummary(task: TaskSnapshot)}
  <span class="flex-1 min-w-0">
    <span class="block truncate text-[12px] {task.id === selectedId ? 'text-[#f1eee8]' : 'text-[#ccc9c1]'}" title={task.name}>{task.name}</span>
    <span class="block truncate mt-0.5 text-[10px] text-[#8f8d86]" title={activityLabel(task)}>{activityLabel(task)}</span>
  </span>
  {#if task.status === 'running' && task.started_at}
    <span class="text-[9px] text-[#77756f] tabular-nums" title="本轮已运行时间">{formatElapsed(task.started_at)}</span>
  {/if}
{/snippet}

<section class="py-1" aria-label={title}>
  <div class="flex items-center gap-2 px-4 pt-1.5 pb-1">
    <p class="text-[10px] font-semibold tracking-[0.12em] text-[#aaa89f]">{title}</p>
    <span class="text-[9px] text-[#74726c] tabular-nums">{tasks.length}</span>
  </div>

  <ul class="m-0 p-0">
    {#each groups.projects as group (group.project)}
      {@const status = groupStatus(group.tasks)}
      {@const isOpen = open[group.project] ?? shouldOpen(group.tasks)}
      <li>
        <button
          type="button"
          aria-expanded={isOpen}
          class="w-full flex items-center gap-2 px-3.5 py-1.5 text-left hover:bg-white/[0.04] transition-colors"
          onclick={() => (open[group.project] = !isOpen)}
        >
          <span aria-hidden="true" class="text-[10px] text-[#77756f] transition-transform {isOpen ? 'rotate-90' : ''}">›</span>
          <StatusDot status={status} />
          <span class="min-w-0 flex-1 truncate text-[12px] font-medium text-[#ddd9d2]">{group.project}</span>
          <span class="text-[9px] text-[#8f8d86]">{STATUS_LABEL[status]}</span>
          <span class="text-[9px] text-[#686762] tabular-nums">{group.tasks.length}</span>
        </button>

        {#if isOpen}
          <ul class="m-0 p-0">
            {#each group.tasks as task (task.id)}
              <li>
                <button
                  type="button"
                  aria-pressed={task.id === selectedId}
                  class="w-full flex items-center gap-2 px-3.5 py-1.5 pl-8 text-left hover:bg-white/[0.04] transition-colors {task.id === selectedId ? 'bg-[#4a9eea]/[0.12]' : ''}"
                  onclick={() => onSelect?.(task.id)}
                >
                  <StatusDot status={task.status} />
                  {@render taskSummary(task)}
                  {#if task.pinned}<span class="text-[9px] text-[#8fb6d8]">固定</span>{/if}
                  {#if task.unread}<span class="w-[5px] h-[5px] rounded-full bg-[#e8c840]" title="尚未查看"></span>{/if}
                  <span class="text-[9px] text-[#77756f] flex-shrink-0">{task.stale ? '待更新' : STATUS_LABEL[task.status]}</span>
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </li>
    {/each}

    {#each groups.flat as task (task.id)}
      <li>
        <button
          type="button"
          aria-pressed={task.id === selectedId}
          class="w-full flex items-center gap-2 px-3.5 py-1.5 text-left hover:bg-white/[0.04] transition-colors {task.id === selectedId ? 'bg-[#4a9eea]/[0.12]' : ''}"
          onclick={() => onSelect?.(task.id)}
        >
          <StatusDot status={task.status} />
          {@render taskSummary(task)}
          {#if task.pinned}<span class="text-[9px] text-[#8fb6d8]">固定</span>{/if}
          {#if task.unread}<span class="w-[5px] h-[5px] rounded-full bg-[#e8c840]" title="尚未查看"></span>{/if}
          <span class="text-[9px] text-[#77756f] flex-shrink-0">{task.stale ? '待更新' : STATUS_LABEL[task.status]}</span>
        </button>
      </li>
    {/each}
  </ul>
</section>
