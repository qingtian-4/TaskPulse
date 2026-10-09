<script lang="ts">
  import type { TaskSnapshot, DashboardSettings, ProviderHealth } from '$lib/types';
  import { formatElapsed } from '$lib/types';
  import { taskContext, taskLabel, sourceFreshness, isEndedResult } from '$lib/dashboard';
  let { tasks, settings, selectedId, onSelect, onViewed, providers=[], now=Date.now() }: { tasks: TaskSnapshot[]; settings: DashboardSettings | null; selectedId: string | null; onSelect: (task: TaskSnapshot) => void; onViewed?: (task: TaskSnapshot) => void; providers?:ProviderHealth[]; now?:number } = $props();
</script>
<ul class="m-0 p-0 list-none">
  {#each tasks as task (task.id)}
    <li class="task-item">
      <button type="button" class:selected={selectedId === task.id} class="task-row" onclick={() => onSelect(task)} aria-pressed={selectedId === task.id}>
        <span class="dot" class:running={task.status === 'running' && !task.stale} class:failed={task.status === 'failed'} class:waiting={task.status==='waiting_input'} class:stale={task.stale}></span>
        <span class="row-copy">
          <span class="row-title" title={task.name}>{task.name}</span>
          <span class="context" title={task.project_path ?? taskContext(task,settings)}>{taskContext(task,settings)}</span>
          {#if task.status === 'running' && !task.stale}<span class="activity" title={task.activity ?? ''}>{task.activity ?? '运行中 · 暂无指令信息'}{task.started_at ? ` · ${formatElapsed(task.started_at,now)}` : ''}</span>{/if}
          <span class="freshness" title={sourceFreshness(task,settings,providers,now)}>{sourceFreshness(task,settings,providers,now).split(' · ').slice(0,2).join(' · ')}</span>
        </span>
        <span class="row-status">
          <span>{taskLabel(task)}</span>
          <span class="markers">{#if task.pinned}<span>固定</span>{/if}{#if task.unread}<span class="unread" title="尚未查看" aria-label="尚未查看"></span>{/if}</span>
        </span>
      </button>
      {#if onViewed && isEndedResult(task)}<button class="viewed" onclick={()=>onViewed?.(task)} title="标记本轮已查看" aria-label={'标记已查看：'+task.name}>✓</button>{/if}
    </li>
  {/each}
</ul>
<style>
  .task-item{position:relative}.task-item:has(.viewed) .task-row{padding-right:42px}.viewed{position:absolute;right:12px;bottom:10px;background:transparent;border:0;border-radius:4px;color:#94b6bf;font-size:15px;cursor:pointer;padding:4px}.viewed:hover{background:#ffffff12}.freshness{font-size:10px;display:block;color:#798889;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;margin-top:4px}.dot.stale{background:#92999e}
  .task-row{width:100%;display:flex;align-items:flex-start;gap:10px;text-align:left;padding:12px 16px;border:0;background:transparent;color:#e5e4df;cursor:pointer;border-radius:8px}
  .task-row:hover{background:#ffffff08}.task-row.selected{background:#70b7eb12;box-shadow:inset 2px 0 #80b5d8}.dot{width:7px;height:7px;border-radius:50%;background:#8da078;flex:none;margin-top:6px}.dot.running{background:#7ab5e4;box-shadow:0 0 7px #7ab5e430}.dot.failed{background:#e29494}.dot.waiting{background:#d6be6d}.row-copy{min-width:0;flex:1}.row-title{font-size:13px;display:block;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.context{display:block;font-size:11px;color:#a6a6a1;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;margin-top:4px}.activity{display:block;font-size:11px;color:#8a9cab;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;margin-top:5px}.row-status{flex:none;font-size:10px;color:#aaa9a1;display:flex;flex-direction:column;align-items:flex-end;gap:7px;padding-top:2px}.markers{display:flex;gap:7px;align-items:center;color:#93b6d0;font-size:9px}.unread{width:5px;height:5px;border-radius:50%;background:#d6be6d}
</style>
