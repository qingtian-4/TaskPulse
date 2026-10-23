<script lang="ts">
  import type { TaskSnapshot, DashboardSettings, ProviderHealth } from '$lib/types';
  import { formatElapsed } from '$lib/types';
  import { taskContext, taskLabel, taskAgent, taskSummary, sourceFreshness, isEndedResult, matchesSignal, taskTone } from '$lib/dashboard';
  import Check from '@lucide/svelte/icons/check';
  import Pin from '@lucide/svelte/icons/pin';
  let { tasks, settings, selectedId, onSelect, onViewed, providers=[], now=Date.now() }: { tasks: TaskSnapshot[]; settings: DashboardSettings | null; selectedId: string | null; onSelect: (task: TaskSnapshot) => void; onViewed?: (task: TaskSnapshot) => void; providers?:ProviderHealth[]; now?:number } = $props();
  function navigate(event:KeyboardEvent) {
    if(!['ArrowDown','ArrowUp','Home','End'].includes(event.key))return;
    const row=event.currentTarget as HTMLButtonElement;
    const rows=Array.from(row.closest('ul')?.querySelectorAll<HTMLButtonElement>('.task-row')??[]);
    const index=rows.indexOf(row);
    const next=event.key==='Home'?0:event.key==='End'?rows.length-1:Math.max(0,Math.min(rows.length-1,index+(event.key==='ArrowDown'?1:-1)));
    event.preventDefault();rows[next]?.focus();rows[next]?.scrollIntoView({block:'nearest'});
  }
</script>
<ul class="m-0 p-0 list-none">
  {#each tasks as task (task.id)}
    <li class="task-item tp-tone" data-tone={taskTone(task)}>
      <button type="button" data-task-id={task.id} class:selected={selectedId===task.id} class:action-needed={matchesSignal(task,'action')} class="task-row" onclick={()=>onSelect(task)} onkeydown={navigate} aria-pressed={selectedId===task.id}>
        <span class="dot" class:running={task.status==='running' && !task.stale} class:failed={task.status==='failed'} class:waiting={task.status==='waiting_input'} class:stale={task.stale}></span>
        <span class="row-copy">
          <span class="row-title" title={task.name}>{task.name}</span>
          <span class="context" title={taskContext(task,settings)+(task.project_path?' · '+task.project_path:'')}><span class="agent">{taskAgent(task)}</span><span class="project">{task.project ?? '未关联项目'}</span><span>{task.host ?? '本机'}</span></span>
          <span class="activity" title={taskSummary(task)}>{taskSummary(task)}</span>
          {#if task.stale || sourceFreshness(task,settings,providers,now).startsWith('来源数据待更新')}<span class="freshness" title={sourceFreshness(task,settings,providers,now)}>来源数据待更新</span>{/if}
        </span>
        <span class="row-status"><span class="status-label tp-chip">{taskLabel(task)}</span><span class="markers">{#if task.pinned}<Pin size={11} aria-label="固定"/>{/if}{#if task.unread}<span class="unread" title="尚未查看" aria-label="尚未查看"></span>{/if}</span>{#if task.status==='running' && !task.stale && task.started_at}<span class="elapsed">{formatElapsed(task.started_at,now)}</span>{/if}</span>
      </button>
      {#if onViewed && isEndedResult(task)}<button class="viewed tp-btn tp-icon" onclick={()=>onViewed?.(task)} title="标记本轮已查看" aria-label={'标记已查看：'+task.name}><Check size={15}/></button>{/if}
    </li>
  {/each}
</ul>
<style>
  .task-item{position:relative;margin:6px 0}
  .task-row{position:relative;width:100%;display:flex;align-items:flex-start;gap:9px;text-align:left;padding:11px 10px;border:1px solid var(--tp-border);border-radius:var(--tp-radius-md);background:var(--tp-card);color:var(--tp-text);cursor:pointer;box-shadow:var(--tp-shadow-card);transition:background .12s,border-color .12s}
  .task-row:hover{background:var(--tp-raised);border-color:var(--tp-border-strong)}
  .task-row.selected{background:linear-gradient(110deg,var(--tp-accent-soft),transparent 90%),var(--tp-card);border-color:var(--tp-accent-line);box-shadow:inset 2px 0 var(--tp-accent),var(--tp-glow)}
  .task-item[data-tone=action] .task-row,.task-item[data-tone=danger] .task-row{border-left:2px solid var(--tone);background:linear-gradient(110deg,var(--tone-soft),transparent 90%),var(--tp-card)}
  .task-item[data-tone=action] .task-row.selected,.task-item[data-tone=danger] .task-row.selected{border-color:var(--tone-line);box-shadow:inset 1px 0 var(--tone),var(--tp-shadow-card)}
  .dot{width:6px;height:6px;border-radius:50%;background:var(--tone);flex:none;margin-top:7px}
  .dot.running{box-shadow:0 0 6px #89c5f430}.dot.stale{box-shadow:none}
  .row-copy{min-width:0;flex:1}.row-title{font-size:13px;line-height:1.55;font-weight:600;display:-webkit-box;-webkit-line-clamp:2;line-clamp:2;-webkit-box-orient:vertical;overflow:hidden;overflow-wrap:anywhere}
  .context{display:flex;gap:6px;align-items:center;min-width:0;font-size:10px;color:var(--tp-quiet);margin-top:5px}
  .context>span{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
  .context .agent{color:var(--tp-muted);flex-shrink:1;max-width:45%;min-width:0}
  .project{max-width:48%;border-left:1px solid var(--tp-border-strong);padding-left:6px}
  .activity,.freshness{display:block;font-size:11px;line-height:1.6;color:var(--tp-muted);overflow:hidden;text-overflow:ellipsis;white-space:nowrap;margin-top:6px}
  .task-item[data-tone=action] .activity{color:var(--tp-action)}.freshness{color:var(--tp-stale);font-size:10px}
  .row-status{flex:none;max-width:98px;font-size:10px;color:var(--tp-muted);display:flex;flex-direction:column;align-items:flex-end;gap:8px;text-align:right;line-height:1.5}
  .status-label{justify-content:center;text-align:center;font-size:10px;padding:2px 5px}
  .markers{display:flex;gap:7px;align-items:center;color:var(--tp-quiet);height:10px}
  .unread{width:5px;height:5px;border-radius:50%;background:var(--tp-unread);box-shadow:0 0 0 2px #76d6c40b}
  .elapsed{font-size:10px;color:var(--tp-quiet);font-variant-numeric:tabular-nums}
  .task-item:has(.viewed) .row-status{padding-bottom:26px}
  .viewed{position:absolute;right:8px;bottom:7px;width:28px;height:26px;min-height:26px;color:var(--tp-accent);background:var(--tp-accent-soft);border-color:var(--tp-accent-line)}
  @media(max-width:400px){.task-row{gap:7px;padding:9px 8px}.row-status{max-width:88px;gap:6px}.context{gap:5px;margin-top:4px}.activity,.freshness{margin-top:4px}}
  @media(max-width:340px){.task-row{gap:6px;padding:10px 8px}.row-status{max-width:84px}.status-label{padding:2px 4px}.row-title{font-size:12px}}
</style>
