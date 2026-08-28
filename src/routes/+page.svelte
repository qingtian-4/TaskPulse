<script lang="ts">
  import { onMount } from 'svelte';
  import { listen, emit } from '@tauri-apps/api/event';
  import { invoke } from '@tauri-apps/api/core';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { LogicalSize } from '@tauri-apps/api/dpi';
  import type { TaskSnapshot, HostHealth } from '$lib/types';
  import { formatAgo } from '$lib/types';
  import AgentRow from '$lib/components/AgentRow.svelte';
  import AggregatePill from '$lib/components/AggregatePill.svelte';
  import { aggregate, localTasks, remoteByHost, recentTasks } from '$lib/utils';
  import Pin from '@lucide/svelte/icons/pin';
  import PinOff from '@lucide/svelte/icons/pin-off';
  import Server from '@lucide/svelte/icons/server';
  import Clock from '@lucide/svelte/icons/clock';

  // Window width — height is fully responsive, capped to available screen height
  const WIN_W = 400;
  const SCREEN_H = screen.availHeight;
  const MAX_WIN_H = SCREEN_H - 16;

  let tasks = $state<TaskSnapshot[]>([]);
  let hosts = $state<HostHealth[]>([]);
  let pinned = $state(false);
  let overPopup = $state(false);
  let statusDir = $state('~/.agent-monitor');
  let ipcError = $state('');

  let locals = $derived(localTasks(tasks));
  let remotes = $derived(remoteByHost(tasks));
  let recents = $derived(recentTasks(tasks));
  let aggregateState = $derived(aggregate(tasks));

  // Group a task list by project (Codex tasks share a project → one group),
  // so each project shows its task names; tasks without a project stay flat.
  function groupProject(tasks: TaskSnapshot[]) {
    const groups: { project: string; tasks: TaskSnapshot[] }[] = [];
    const flat: TaskSnapshot[] = [];
    for (const t of tasks) {
      if (t.project) {
        const g = groups.find((x) => x.project === t.project);
        if (g) g.tasks.push(t);
        else groups.push({ project: t.project, tasks: [t] });
      } else flat.push(t);
    }
    return { groups, flat };
  }

  function quotaText(t: TaskSnapshot) {
    if (t.quota_used_pct == null) return '';
    const pct = Math.round(t.quota_used_pct);
    if (t.quota_resets_at) {
      const days = Math.max(1, Math.round((t.quota_resets_at - Date.now() / 1000) / 86400));
      return `额度 ${pct}% · ${days}d`;
    }
    return `额度 ${pct}%`;
  }

  let panelEl = $state<HTMLDivElement | null>(null);

  $effect(() => {
    if (!panelEl) return;
    let prevH = 0;
    const observer = new ResizeObserver((entries) => {
      const entry = entries[0];
      if (!entry) return;
      const borderBoxH = entry.borderBoxSize[0]?.blockSize ?? entry.contentRect.height + 2;
      const totalH = Math.min(Math.round(borderBoxH), MAX_WIN_H);
      if (totalH === prevH) return;
      prevH = totalH;
      getCurrentWindow().setSize(new LogicalSize(WIN_W, totalH));
    });
    observer.observe(panelEl);
    return () => observer.disconnect();
  });

  onMount(() => {
    const cleanups: Array<() => void> = [];
    let mounted = true;

    async function addListener<T>(unlisten: Promise<() => void>) {
      const fn = await unlisten;
      if (mounted) cleanups.push(fn);
      else fn();
    }

    (async () => {
      await addListener(listen<TaskSnapshot[]>('agents-updated', (event) => {
        tasks = event.payload;
        ipcError = '';
      }));
      await addListener(listen<HostHealth[]>('hosts-updated', (event) => {
        hosts = event.payload;
      }));

      await emit('popup-ready', null);

      try {
        const result = await invoke<TaskSnapshot[]>('get_agents');
        tasks = result;
      } catch (e) {
        ipcError = `IPC error: ${e}`;
        console.error('[TaskPulse] get_agents failed:', e);
      }

      if (tasks.length === 0) {
        await new Promise(r => setTimeout(r, 600));
        try {
          const retry = await invoke<TaskSnapshot[]>('get_agents');
          if (retry.length > 0) tasks = retry;
        } catch (e) {
          console.warn('[TaskPulse] retry get_agents failed:', e);
        }
      }

      try {
        statusDir = await invoke<string>('get_status_dir') || statusDir;
      } catch (e) {
        console.warn('[TaskPulse] get_status_dir failed:', e);
      }

      const win = getCurrentWindow();
      let showTime = Date.now();

      await addListener(listen('tauri://focus', async () => {
        showTime = Date.now();
        try {
          tasks = await invoke<TaskSnapshot[]>('get_agents');
        } catch (e) {
          console.warn('[TaskPulse] get_agents on focus failed:', e);
        }
      }));

      const BLUR_DEBOUNCE_MS = 300;
      await addListener(win.onFocusChanged(({ payload: focused }) => {
        // Don't auto-hide when pinned, when the pointer is still over the
        // popup (clicking/dragging the drag-region header causes a transient
        // blur), or within the debounce window after opening.
        if (!focused && !pinned && !overPopup && Date.now() - showTime > BLUR_DEBOUNCE_MS)
          invoke('close_popup');
      }));

      await addListener(listen<boolean>('pinned-changed', (event) => {
        pinned = event.payload;
      }));
    })();

    return () => {
      mounted = false;
      cleanups.forEach(fn => fn());
    };
  });

  let focusing = $state(false);
  let focusError = $state('');
  async function focusAgent(task: TaskSnapshot) {
    if (focusing || !task.can_focus || !task.terminal) return;
    focusing = true;
    focusError = '';
    try {
      await invoke('focus_terminal', {
        req: {
          kind:     task.terminal.kind,
          focus_id: task.terminal.focus_id,
          outer_id: task.terminal.outer_id,
        }
      });
      if (!pinned) invoke('close_popup');
    } catch (e) {
      focusError = String(e);
      setTimeout(() => { focusError = ''; }, 4000);
    } finally {
      focusing = false;
    }
  }

  let remoteHosts = $derived(
    [...remotes.keys()].map((host) => ({
      host,
      health: hosts.find((h) => h.host === host) ?? null,
    }))
  );

  function hostStatusLine(host: string): string | null {
    const h = hosts.find((x) => x.host === host);
    if (!h) return null;
    if (h.status === 'disconnected') {
      return h.last_seen ? `disconnected · last seen ${formatAgo(h.last_seen)}` : 'disconnected';
    }
    return h.last_seen ? `connected · ${formatAgo(h.last_seen)}` : null;
  }
</script>

<div bind:this={panelEl} class="glass-panel glass-noise relative w-full rounded-[14px] overflow-hidden flex flex-col"
     style="max-height: {MAX_WIN_H}px;"
     onpointerenter={() => (overPopup = true)}
     onpointerleave={() => (overPopup = false)}>
  <!-- Header (drag region) -->
  <div data-tauri-drag-region class="flex items-center justify-between px-3.5 py-2.5 border-b border-white/[0.06] shrink-0"
       style="background: rgba(255,255,255,0.03);">
    <span data-tauri-drag-region class="text-[10px] font-semibold tracking-widest uppercase text-[#aaa89f]">TaskPulse</span>
    <div class="flex items-center gap-1.5">
      <AggregatePill state={aggregateState} />
      <button
        onclick={() => invoke('toggle_pin')}
        class="p-0.5 rounded hover:bg-white/10 transition-colors {pinned ? 'text-white/80' : 'text-[#a5a39b]'}"
        title={pinned ? 'Unpin window' : 'Pin window'}
      >
        {#if pinned}
          <Pin size={12} />
        {:else}
          <PinOff size={12} />
        {/if}
      </button>
    </div>
  </div>

  <!-- Error toasts -->
  {#if focusError}
    <div class="px-3 py-1.5 bg-[#3a2020] border-b border-red-500/20 text-[10px] text-red-400 truncate shrink-0">
      {focusError}
    </div>
  {/if}
  {#if ipcError}
    <div class="px-3 py-1.5 bg-[#3a2020] border-b border-red-500/20 text-[10px] text-red-400 truncate shrink-0">
      {ipcError}
    </div>
  {/if}

  {#if tasks.length === 0}
    <p class="text-[11px] text-[#aaa89f] text-center py-5 leading-relaxed">
      No tasks detected.<br/>Local status files: {statusDir}<br/>Remote hosts: see ~/.taskpulse/config.toml
    </p>
  {:else}
    <div class="glass-scroll flex-1 min-h-0 overflow-y-auto py-1">
      {#if locals.length > 0}
        <p class="text-[9px] font-semibold tracking-widest uppercase text-[#8f8d86] px-4 pt-1.5 pb-0.5">Local Agents</p>
        <ul class="m-0 p-0">
          {#each locals as task (task.id)}
            <AgentRow {task} onFocus={() => focusAgent(task)} />
          {/each}
        </ul>
      {/if}

      {#if remotes.size > 0 || hosts.length > 0}
        <p class="text-[9px] font-semibold tracking-widest uppercase text-[#8f8d86] px-4 pt-2 pb-0.5">Remote Tmux</p>
        {#if remotes.size === 0}
          <p class="text-[10px] text-[#8f8d86] px-4 py-1">
            {#each hosts as h (h.host)}
              <span class="mr-2 inline-flex items-center gap-1">
                <Server size={10} class="inline-block" />
                {h.host}
                <span style="color: {h.status === 'disconnected' ? '#dd4f4f' : '#78b644'}">
                  {h.status === 'disconnected' ? (h.last_seen ? `disconnected · ${formatAgo(h.last_seen)}` : 'disconnected') : 'idle'}
                </span>
              </span>
            {/each}
          </p>
        {:else}
          {#each remoteHosts as rh (rh.host)}
            <div class="px-3.5 pt-1 flex items-center gap-1.5">
              <Server size={11} strokeWidth={1.8} class="flex-shrink-0 text-[#8a7fd9]" />
              <span class="text-[11px] font-medium text-[#d9d7d2]">{rh.host}</span>
              {#if hostStatusLine(rh.host)}
                <span class="text-[10px] text-[#8f8d86] tabular-nums">{hostStatusLine(rh.host)}</span>
              {/if}
            </div>
            {@const { groups, flat } = groupProject(remotes.get(rh.host) ?? [])}
            {#each groups as g (g.project)}
              <div class="px-3.5 pt-1 flex items-center gap-1.5">
                <span class="text-[9px] font-semibold tracking-wider uppercase text-[#8f8d86]">{g.project}</span>
                <span class="text-[9px] text-[#8a887f] ml-auto tabular-nums">{g.tasks.length} 任务</span>
              </div>
              <ul class="m-0 p-0">
                {#each g.tasks as task (task.id)}
                  <AgentRow {task} onFocus={() => focusAgent(task)} />
                {/each}
              </ul>
            {/each}
            {#each flat as task (task.id)}
              <AgentRow {task} onFocus={() => focusAgent(task)} />
            {/each}
          {/each}
        {/if}
      {/if}

      {#if recents.length > 0}
        <p class="text-[9px] font-semibold tracking-widest uppercase text-[#8f8d86] px-4 pt-2 pb-0.5">Recent</p>
        <ul class="m-0 p-0">
          {#each recents as task (task.id)}
            <AgentRow {task} onFocus={() => focusAgent(task)} />
          {/each}
        </ul>
        <p class="text-[9px] text-[#8a887f] px-4 pb-1 flex items-center gap-1">
          <Clock size={9} class="inline-block" /> completed tasks stay for 30 minutes
        </p>
      {/if}
    </div>
  {/if}
</div>
