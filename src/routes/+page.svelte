<script lang="ts">
  import { onMount } from 'svelte';
  import { emit, listen } from '@tauri-apps/api/event';
  import { invoke, isTauri } from '@tauri-apps/api/core';
  import { LogicalSize } from '@tauri-apps/api/dpi';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import type { HostHealth, Source, TaskSnapshot } from '$lib/types';
  import { SOURCE_LABEL, STATUS_LABEL, formatAgo } from '$lib/types';
  import { aggregate, localTasks, recentTasks, remoteByHost } from '$lib/utils';
  import AgentRow from '$lib/components/AgentRow.svelte';
  import AggregatePill from '$lib/components/AggregatePill.svelte';
  import ProjectTree from '$lib/components/ProjectTree.svelte';
  import StatusDot from '$lib/components/StatusDot.svelte';
  import Clock from '@lucide/svelte/icons/clock';
  import Pin from '@lucide/svelte/icons/pin';
  import PinOff from '@lucide/svelte/icons/pin-off';
  import Server from '@lucide/svelte/icons/server';

  const WIN_W = 400;
  const MAX_WIN_H = screen.availHeight - 16;
  const SOURCE_ORDER: Source[] = [
    'hermes',
    'deepseek_harness',
    'codex',
    'remote_tmux',
    'claude_code',
    'gemini',
    'local_file',
  ];
  const DEMO_TASKS: TaskSnapshot[] = [
    { id: 'demo-hermes', name: '实现监控状态协议', source: 'hermes', status: 'waiting_input', message: '等待确认状态字段', updated_at: Date.now() - 45_000, confidence: 'exact', remote: false },
    { id: 'demo-deepseek', name: '修复登录回归', source: 'deepseek_harness', status: 'running', project: 'dashboard', message: '正在运行测试', started_at: Date.now() - 520_000, updated_at: Date.now() - 8_000, confidence: 'inferred', remote: false },
    { id: 'demo-codex-1', name: '整理任务列表', source: 'codex', status: 'running', project: 'agenttray', message: '正在更新前端', started_at: Date.now() - 300_000, updated_at: Date.now() - 3_000, confidence: 'exact', remote: false, quota_5h_used_pct: 28, quota_1w_used_pct: 41 },
    { id: 'demo-codex-2', name: '补充 Rust 测试', source: 'codex', status: 'idle', project: 'agenttray', message: '等待下一步', updated_at: Date.now() - 80_000, confidence: 'exact', remote: false },
    { id: 'demo-remote-codex', name: '训练任务日志检查', source: 'codex', status: 'running', host: 'Server1', project: 'model-run', message: '正在分析输出', started_at: Date.now() - 1_800_000, updated_at: Date.now() - 12_000, confidence: 'exact', remote: true },
    { id: 'demo-tmux', name: 'tmux:3.0', source: 'remote_tmux', status: 'waiting_input', host: 'Server3', project: 'benchmark', message: '终端等待输入', started_at: Date.now() - 740_000, updated_at: Date.now() - 20_000, confidence: 'inferred', remote: true, stale: true },
    { id: 'demo-recent', name: '更新安装说明', source: 'codex', status: 'succeeded', project: 'agenttray', message: '任务已完成', updated_at: Date.now() - 180_000, finished_at: Date.now() - 180_000, confidence: 'exact', remote: false, recent: true, exit_code: 0 },
  ];
  const DEMO_HOSTS: HostHealth[] = [
    { host: 'Server1', status: 'connected', last_seen: Date.now() - 12_000 },
    { host: 'Server3', status: 'disconnected', last_seen: Date.now() - 240_000 },
  ];

  let tasks = $state<TaskSnapshot[]>([]);
  let hosts = $state<HostHealth[]>([]);
  let pinned = $state(false);
  let overPopup = $state(false);
  let statusDir = $state('~/.agent-monitor');
  let ipcError = $state('');
  let focusError = $state('');
  let focusing = $state(false);
  let selectedId = $state<string | null>(null);
  let panelEl = $state<HTMLDivElement | null>(null);

  let locals = $derived(localTasks(tasks));
  let remotes = $derived(remoteByHost(tasks));
  let recents = $derived(recentTasks(tasks));
  let aggregateState = $derived(aggregate(tasks, hosts));
  let selectedTask = $derived(tasks.find((task) => task.id === selectedId) ?? null);
  let remoteHosts = $derived.by(() => {
    const names = [...hosts.map((host) => host.host), ...remotes.keys()];
    return [...new Set(names)].map((host) => ({
      host,
      health: hosts.find((health) => health.host === host) ?? null,
      tasks: remotes.get(host) ?? [],
    }));
  });

  function bySource(sourceTasks: TaskSnapshot[]) {
    const grouped = new Map<Source, TaskSnapshot[]>();
    for (const task of sourceTasks) {
      const current = grouped.get(task.source) ?? [];
      current.push(task);
      grouped.set(task.source, current);
    }
    return SOURCE_ORDER
      .filter((source) => grouped.has(source))
      .map((source) => ({ source, tasks: grouped.get(source) ?? [] }));
  }

  async function refreshSnapshots() {
    const [nextTasks, nextHosts] = await Promise.all([
      invoke<TaskSnapshot[]>('get_agents'),
      invoke<HostHealth[]>('get_hosts'),
    ]);
    tasks = nextTasks;
    hosts = nextHosts;
    ipcError = '';
  }

  $effect(() => {
    if (!panelEl || !isTauri()) return;
    let previousHeight = 0;
    const observer = new ResizeObserver((entries) => {
      const entry = entries[0];
      if (!entry) return;
      const borderBoxHeight = entry.borderBoxSize[0]?.blockSize ?? entry.contentRect.height + 2;
      const height = Math.min(Math.round(borderBoxHeight), MAX_WIN_H);
      if (height === previousHeight) return;
      previousHeight = height;
      getCurrentWindow().setSize(new LogicalSize(WIN_W, height));
    });
    observer.observe(panelEl);
    return () => observer.disconnect();
  });

  onMount(() => {
    if (!isTauri()) {
      if (import.meta.env.DEV) {
        tasks = DEMO_TASKS;
        hosts = DEMO_HOSTS;
      }
      return;
    }

    const cleanups: Array<() => void> = [];
    let mounted = true;

    async function addListener(unlisten: Promise<() => void>) {
      const cleanup = await unlisten;
      if (mounted) cleanups.push(cleanup);
      else cleanup();
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
        await refreshSnapshots();
      } catch (error) {
        ipcError = `无法读取监控状态：${error}`;
      }

      if (tasks.length === 0) {
        await new Promise((resolve) => setTimeout(resolve, 600));
        try {
          await refreshSnapshots();
        } catch (error) {
          console.warn('[TaskPulse] retry refresh failed:', error);
        }
      }

      try {
        statusDir = await invoke<string>('get_status_dir') || statusDir;
      } catch (error) {
        console.warn('[TaskPulse] get_status_dir failed:', error);
      }

      const win = getCurrentWindow();
      let showTime = Date.now();
      await addListener(listen('tauri://focus', async () => {
        showTime = Date.now();
        try {
          await refreshSnapshots();
        } catch (error) {
          console.warn('[TaskPulse] refresh on focus failed:', error);
        }
      }));
      await addListener(win.onFocusChanged(({ payload: focused }) => {
        if (!focused && !pinned && !overPopup && Date.now() - showTime > 300) {
          invoke('close_popup');
        }
      }));
      await addListener(listen<boolean>('pinned-changed', (event) => {
        pinned = event.payload;
      }));
    })();

    return () => {
      mounted = false;
      cleanups.forEach((cleanup) => cleanup());
    };
  });

  async function focusAgent(task: TaskSnapshot) {
    if (focusing || !task.can_focus || !task.terminal) return;
    focusing = true;
    focusError = '';
    try {
      await invoke('focus_terminal', {
        req: {
          kind: task.terminal.kind,
          focus_id: task.terminal.focus_id,
          outer_id: task.terminal.outer_id,
        },
      });
      if (!pinned) invoke('close_popup');
    } catch (error) {
      focusError = String(error);
      setTimeout(() => (focusError = ''), 4000);
    } finally {
      focusing = false;
    }
  }
</script>

<div
  bind:this={panelEl}
  role="region"
  aria-label="TaskPulse Agent 任务监控"
  class="glass-panel glass-noise relative w-full rounded-[14px] overflow-hidden flex flex-col"
  style="max-height: {MAX_WIN_H}px;"
  onpointerenter={() => (overPopup = true)}
  onpointerleave={() => (overPopup = false)}
>
  <div data-tauri-drag-region class="flex items-center justify-between px-3.5 py-2.5 border-b border-white/[0.06] shrink-0" style="background: rgba(255,255,255,0.03);">
    <span data-tauri-drag-region class="text-[11px] font-semibold tracking-[0.16em] uppercase text-[#bbb8b0]">TaskPulse</span>
    <div class="flex items-center gap-1.5">
      <AggregatePill state={aggregateState} />
      <button
        type="button"
        onclick={() => invoke('toggle_pin')}
        class="p-0.5 rounded hover:bg-white/10 transition-colors {pinned ? 'text-white/80' : 'text-[#a5a39b]'}"
        aria-label={pinned ? '取消固定窗口' : '固定窗口'}
        title={pinned ? '取消固定窗口' : '固定窗口'}
      >
        {#if pinned}<Pin size={12} />{:else}<PinOff size={12} />{/if}
      </button>
    </div>
  </div>

  {#if focusError || ipcError}
    <div class="px-3 py-1.5 bg-[#3a2020] border-b border-red-500/20 text-[10px] text-red-400 truncate shrink-0">
      {focusError || ipcError}
    </div>
  {/if}

  {#if tasks.length === 0 && hosts.length === 0}
    <p class="text-[11px] text-[#aaa89f] text-center py-5 px-4 leading-relaxed">
      暂未检测到任务<br />
      <span class="text-[#77756f]">本地状态目录：{statusDir}</span>
    </p>
  {:else}
    <div class="glass-scroll flex-1 min-h-0 overflow-y-auto py-1">
      {#if locals.length > 0}
        <p class="text-[10px] font-semibold tracking-[0.16em] uppercase text-[#8f8d86] px-4 pt-2 pb-0.5">本地 Agents</p>
        {#each bySource(locals) as group (group.source)}
          <ProjectTree tasks={group.tasks} {selectedId} onSelect={(id) => (selectedId = id)} title={SOURCE_LABEL[group.source]} />
        {/each}
      {/if}

      {#if remoteHosts.length > 0}
        <p class="text-[10px] font-semibold tracking-[0.16em] uppercase text-[#8f8d86] px-4 pt-2 pb-1">远程主机</p>
        {#each remoteHosts as remote (remote.host)}
          <section class="mx-2 mb-1.5 rounded-lg border border-white/[0.06] bg-white/[0.02] overflow-hidden" aria-label={`远程主机 ${remote.host}`}>
            <div class="flex items-center gap-2 px-2.5 py-2 border-b border-white/[0.05]">
              <Server size={12} class="text-[#8a7fd9]" />
              <span class="flex-1 min-w-0 truncate text-[12px] font-semibold text-[#ddd9d2]">{remote.host}</span>
              {#if remote.health?.status === 'disconnected'}
                <StatusDot status="disconnected" />
                <span class="text-[9px] text-[#dd6969]">
                  已断开{remote.health.last_seen ? ` · ${formatAgo(remote.health.last_seen)}` : ''}
                </span>
              {:else}
                <span class="w-[6px] h-[6px] rounded-full bg-[#78b644]"></span>
                <span class="text-[9px] text-[#8eaa78]">已连接</span>
              {/if}
            </div>
            {#if remote.tasks.length > 0}
              {#each bySource(remote.tasks) as group (group.source)}
                <ProjectTree tasks={group.tasks} {selectedId} onSelect={(id) => (selectedId = id)} title={SOURCE_LABEL[group.source]} />
              {/each}
            {:else}
              <p class="px-3 py-2 text-[10px] text-[#77756f]">暂无运行中的任务</p>
            {/if}
          </section>
        {/each}
      {/if}

      {#if recents.length > 0}
        <p class="text-[10px] font-semibold tracking-[0.16em] uppercase text-[#8f8d86] px-4 pt-2 pb-0.5">最近完成</p>
        <ul class="m-0 p-0">
          {#each recents as task (task.id)}
            <AgentRow {task} onFocus={() => focusAgent(task)} />
          {/each}
        </ul>
        <p class="text-[9px] text-[#77756f] px-4 pb-1 flex items-center gap-1">
          <Clock size={9} /> 完成的任务保留 30 分钟
        </p>
      {/if}
    </div>
  {/if}

  {#if selectedTask}
    <div class="px-3.5 py-2 border-t border-white/[0.06] flex items-center gap-2" style="background:rgba(74,158,234,0.06)">
      <span class="text-[9px] tracking-widest text-[#8f8d86]">详情</span>
      <span class="flex-1 min-w-0 truncate text-[11px] text-[#e4e2dd] font-medium">
        {selectedTask.project ? `${selectedTask.project} · ` : ''}{selectedTask.name}
      </span>
      <span class="text-[9px] text-[#8f8d86] tabular-nums">{STATUS_LABEL[selectedTask.status]}</span>
      {#if selectedTask.quota_5h_used_pct != null || selectedTask.quota_1w_used_pct != null}
        <span class="text-[9px] text-[#c9c161] tabular-nums">
          {selectedTask.quota_5h_used_pct != null ? `5h ${Math.round(selectedTask.quota_5h_used_pct)}%` : ''}
          {selectedTask.quota_5h_used_pct != null && selectedTask.quota_1w_used_pct != null ? ' · ' : ''}
          {selectedTask.quota_1w_used_pct != null ? `1周 ${Math.round(selectedTask.quota_1w_used_pct)}%` : ''}
        </span>
      {/if}
    </div>
  {/if}
</div>
