<script lang="ts">
  import type { TaskSnapshot } from '$lib/types';
  import StatusDot from './StatusDot.svelte';

  let {
    tasks,
    selectedId = null,
    title = '项目',
    onSelect,
  }: {
    tasks: TaskSnapshot[];
    selectedId?: string | null;
    title?: string;
    onSelect?: (id: string) => void;
  } = $props();

  // Group by project; tasks without a project go to "flat".
  const groups = $derived.by(() => {
    const m = new Map<string, TaskSnapshot[]>();
    const flat: TaskSnapshot[] = [];
    for (const t of tasks) {
      const p = (t.project ?? '').trim();
      if (p) {
        const arr = m.get(p);
        if (arr) arr.push(t);
        else m.set(p, [t]);
      } else flat.push(t);
    }
    return {
      projects: [...m.entries()].map(([project, ts]) => ({ project, tasks: ts })),
      flat,
    };
  });

  // Which projects are expanded.
  let open = $state<Record<string, boolean>>({});

  function colorFor(status: TaskSnapshot['status']) {
    const c: Record<TaskSnapshot['status'], string> = {
      running: '#4a9eea', waiting_input: '#c9c161', succeeded: '#78b644',
      failed: '#dd4f4f', idle: '#8f8d86', disconnected: '#dd4f4f', unknown: '#8f8d86',
    };
    return c[status] ?? '#8f8d86';
  }
</script>

<p class="text-[9px] font-semibold tracking-widest uppercase text-[#8f8d86] px-4 pt-2 pb-0.5">{title}</p>

<ul class="m-0 p-0">
  {#each groups.projects as g (g.project)}
    {@const isOpen = open[g.project] ?? false}
    <li>
      <button
        class="w-full flex items-center gap-1.5 px-3.5 py-1 text-left hover:bg-white/[0.03]"
        onclick={() => (open[g.project] = !isOpen)}
      >
        <span class="text-[9px] text-[#8f8d86] transition-transform {isOpen ? 'rotate-90' : ''}">›</span>
        <span class="text-[11px] font-semibold text-[#d9d7d2]">{g.project}</span>
        {#if g.tasks.some((t) => t.host)}
          <span class="text-[9px] text-[#8a7fd9] flex items-center gap-1">
            {g.tasks.find((t) => t.host)?.host}
            <StatusDot status="waiting_input" />
          </span>
        {/if}
        <span class="ml-auto text-[9px] text-[#8f8d86] tabular-nums">{g.tasks.length} 任务</span>
      </button>
      {#if isOpen}
        <ul class="m-0 p-0">
          {#each g.tasks as task (task.id)}
            <li>
              <button
                class="w-full flex items-center gap-2 px-3.5 py-1 pl-7 text-left hover:bg-white/[0.03] {task.id === selectedId ? 'bg-[#4a9eea]/[0.12]' : ''}"
                onclick={() => onSelect?.(task.id)}
              >
                <StatusDot status={task.status} />
                <span class="flex-1 min-w-0 truncate text-[11.5px] {task.id === selectedId ? 'text-[#e4e2dd]' : 'text-[#aaa89f]'}">{task.name}</span>
                {#if task.quota_1w_used_pct != null}
                  <span class="text-[8px] text-[#c9c161] tabular-nums flex-shrink-0">
                    {task.quota_5h_used_pct != null ? `5h ${Math.round(task.quota_5h_used_pct)}%` : ''}{task.quota_1w_used_pct != null ? ` · 1周 ${Math.round(task.quota_1w_used_pct)}%` : ''}
                  </span>
                {/if}
                {#if task.id === selectedId}
                  <span class="text-[8px] text-[#4a9eea] flex-shrink-0">● 监控中</span>
                {/if}
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </li>
  {/each}

  {#each groups.flat as task (task.id)}
    <li>
      <button class="w-full flex items-center gap-2 px-3.5 py-1 text-left hover:bg-white/[0.03] {task.id === selectedId ? 'bg-[#4a9eea]/[0.12]' : ''}" onclick={() => onSelect?.(task.id)}>
        <StatusDot status={task.status} />
        <span class="flex-1 min-w-0 truncate text-[11.5px] {task.id === selectedId ? 'text-[#e4e2dd]' : 'text-[#aaa89f]'}">{task.name}</span>
        {#if task.id === selectedId}<span class="text-[8px] text-[#4a9eea]">● 监控中</span>{/if}
      </button>
    </li>
  {/each}
</ul>
