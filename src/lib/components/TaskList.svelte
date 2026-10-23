<script lang="ts">
  import type { TaskSnapshot, DashboardSettings, ProviderHealth } from '$lib/types';
  import { formatElapsed } from '$lib/types';
  import { taskContext, taskLabel, taskAgent, taskSummary, taskSource, sourceFreshness, isEndedResult, matchesSignal, taskTone } from '$lib/dashboard';
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
          <span class="context" title={taskContext(task,settings)+(task.project_path?' · '+task.project_path:'')}><span class="agent">{taskAgent(task)}</span><span class="project">{task.project ?? '未关联项目'}</span><span class="location">{task.remote ? 'SSH · '+(task.host ?? '远程') : taskSource(task,settings??undefined)?.kind.endsWith('_wsl') ? 'WSL · '+taskSource(task,settings??undefined)?.location : '本机'}</span></span>
          <span class="activity" title={taskSummary(task)}>{taskSummary(task)}</span>
          {#if task.stale || sourceFreshness(task,settings,providers,now).startsWith('来源数据待更新')}<span class="freshness" title={sourceFreshness(task,settings,providers,now)}>来源数据待更新</span>{/if}
        </span>
        <span class="row-status"><span class="status-label tp-chip">{taskLabel(task)}</span><span class="markers">{#if task.pinned}<Pin size={11} aria-label="固定"/>{/if}{#if task.unread}<span class="unread" title="尚未查看" aria-label="尚未查看"></span>{/if}</span>{#if task.status==='running' && !task.stale && task.started_at}<span class="elapsed">{formatElapsed(task.started_at,now)}</span>{/if}</span>
      </button>
      {#if onViewed && isEndedResult(task)}<button class="viewed tp-btn" onclick={()=>onViewed?.(task)} title="标记本轮已查看" aria-label={'标记已查看：'+task.name}><Check size={14}/>已查看</button>{/if}
    </li>
  {/each}
</ul>
<style>
  .task-item{position:relative;margin:8px 0}
  .task-row{position:relative;width:100%;display:grid;grid-template-columns:minmax(0,1fr) auto;gap:4px 8px;text-align:left;padding:12px 12px 12px 24px;border:1px solid var(--tp-border);border-radius:8px;background:var(--tp-card);color:var(--tp-text);cursor:pointer;transition:background .12s,border-color .12s}
  .task-row:hover{background:var(--tp-raised);border-color:var(--tp-border-strong)}
  .task-row.selected{background:var(--tp-accent-soft);border-color:var(--tp-accent-line);box-shadow:inset 2px 0 var(--tp-accent)}
  .task-item[data-tone=action] .task-row,.task-item[data-tone=danger] .task-row{border-left:2px solid var(--tone)}
  .task-item[data-tone=action] .task-row.selected,.task-item[data-tone=danger] .task-row.selected{border-color:var(--tone-line)}
  .dot{position:absolute;left:10px;top:20px;width:6px;height:6px;border-radius:50%;background:var(--tone)}
  .row-copy,.row-status{display:contents}
  .row-title{grid-row:1;grid-column:1;min-width:0;font-size:14px;line-height:1.5;font-weight:600;display:-webkit-box;-webkit-line-clamp:2;line-clamp:2;-webkit-box-orient:vertical;overflow:hidden;overflow-wrap:anywhere}
  .context{grid-row:2;grid-column:1/-1;display:flex;gap:8px;align-items:center;min-width:0;font-size:11px;line-height:1.5;color:var(--tp-quiet);margin-top:4px}
  .context>span{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.agent{max-width:34%;color:var(--tp-muted)}.project{flex:1;border-left:1px solid var(--tp-border-strong);padding-left:8px}.location{max-width:34%}
  .activity{grid-row:3;grid-column:1/-1;font-size:12px;line-height:1.5;color:var(--tp-muted);display:-webkit-box;-webkit-line-clamp:2;line-clamp:2;-webkit-box-orient:vertical;overflow:hidden;overflow-wrap:anywhere;margin-top:4px}
  .task-item[data-tone=action] .activity{color:var(--tp-action)}.freshness{grid-row:4;grid-column:1/-1;color:var(--tp-stale);font-size:11px}
  .status-label{grid-row:1;grid-column:2;align-self:start;max-width:112px;justify-content:center;font-size:11px;line-height:1.5;padding:2px 6px;text-align:center}
  .markers{grid-row:5;grid-column:1;display:flex;gap:8px;align-items:center;color:var(--tp-quiet);min-height:16px}.unread{width:5px;height:5px;border-radius:50%;background:var(--tp-unread)}
  .elapsed{grid-row:5;grid-column:2;font-size:11px;color:var(--tp-quiet);font-variant-numeric:tabular-nums;text-align:right}
  .task-item:has(.viewed) .markers{min-height:28px}
  .viewed{position:absolute;right:12px;bottom:12px;min-height:28px;padding:4px 8px;gap:4px;font-size:11px;color:var(--tp-accent);background:var(--tp-accent-soft);border-color:var(--tp-accent-line)}
  @media(max-width:350px){.task-row{padding:8px 8px 8px 20px;gap:4px 6px}.dot{left:8px;top:16px}.status-label{max-width:90px;padding:2px 4px}.context{gap:4px}.project{padding-left:4px}.viewed{right:8px;bottom:8px}}
</style>
