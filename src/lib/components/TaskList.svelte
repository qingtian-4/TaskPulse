<script lang="ts">
  import type { TaskSnapshot, DashboardSettings, ProviderHealth } from '$lib/types';
  import { formatElapsed } from '$lib/types';
  import { taskContext, taskLabel, taskAgent, taskSummary, sourceFreshness, isEndedResult, matchesSignal } from '$lib/dashboard';
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
    <li class="task-item">
      <button type="button" data-task-id={task.id} class:selected={selectedId===task.id} class:action-needed={matchesSignal(task,'action')} class="task-row" onclick={()=>onSelect(task)} onkeydown={navigate} aria-pressed={selectedId===task.id}>
        <span class="dot" class:running={task.status==='running' && !task.stale} class:failed={task.status==='failed'} class:waiting={task.status==='waiting_input'} class:stale={task.stale}></span>
        <span class="row-copy">
          <span class="row-title" title={task.name}>{task.name}</span>
          <span class="context" title={taskContext(task,settings)+(task.project_path?' · '+task.project_path:'')}><span class="agent">{taskAgent(task)}</span><span class="project">{task.project ?? '未关联项目'}</span><span>{task.host ?? '本机'}</span></span>
          <span class="activity" title={taskSummary(task)}>{taskSummary(task)}</span>
          {#if task.stale || sourceFreshness(task,settings,providers,now).startsWith('来源数据待更新')}<span class="freshness" title={sourceFreshness(task,settings,providers,now)}>来源数据待更新</span>{/if}
        </span>
        <span class="row-status"><span class="status-label">{taskLabel(task)}</span><span class="markers">{#if task.pinned}<Pin size={11} aria-label="固定"/>{/if}{#if task.unread}<span class="unread" title="尚未查看" aria-label="尚未查看"></span>{/if}</span>{#if task.status==='running' && !task.stale && task.started_at}<span class="elapsed">{formatElapsed(task.started_at,now)}</span>{/if}</span>
      </button>
      {#if onViewed && isEndedResult(task)}<button class="viewed" onclick={()=>onViewed?.(task)} title="标记本轮已查看" aria-label={'标记已查看：'+task.name}><Check size={15}/></button>{/if}
    </li>
  {/each}
</ul>
<style>
  .task-item{position:relative;margin:4px 0}.task-item:has(.viewed) .row-status{padding-bottom:27px}.viewed{position:absolute;right:8px;bottom:7px;display:grid;place-items:center;width:30px;height:28px;background:#ffffff06;border:1px solid var(--tp-border);border-radius:6px;color:var(--tp-accent);cursor:pointer}.viewed:hover{background:var(--tp-hover)}
  .task-row{width:100%;display:flex;align-items:flex-start;gap:10px;text-align:left;padding:12px 10px;border:1px solid transparent;background:#ffffff03;color:var(--tp-text);cursor:pointer;border-radius:9px;transition:background .12s,border-color .12s}.task-row:hover{background:var(--tp-hover)}.task-row.selected{background:#70b7eb12;border-color:#80b5d82e;box-shadow:inset 2px 0 var(--tp-accent)}.task-row.action-needed .status-label{color:var(--tp-warning)}
  .dot{width:7px;height:7px;border-radius:50%;background:#8da078;flex:none;margin-top:7px}.dot.running{background:var(--tp-accent);box-shadow:0 0 7px #7ab5e430}.dot.failed{background:var(--tp-danger)}.dot.waiting{background:var(--tp-warning)}.dot.stale{background:#92999e}
  .row-copy{min-width:0;flex:1}.row-title{font-size:13px;line-height:1.55;font-weight:550;display:-webkit-box;-webkit-line-clamp:2;line-clamp:2;-webkit-box-orient:vertical;overflow:hidden;overflow-wrap:anywhere}.context{display:flex;gap:6px;align-items:center;min-width:0;font-size:11px;color:var(--tp-muted);margin-top:6px}.context>span{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.context .agent{color:#b8c5cc;flex-shrink:0}.project{max-width:48%;border-left:1px solid var(--tp-border);padding-left:6px}.activity,.freshness{display:block;font-size:11px;line-height:1.6;color:#94a9b4;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;margin-top:6px}.freshness{color:var(--tp-muted);font-size:10px}
  .row-status{flex:none;max-width:91px;font-size:10px;color:#b2b8b6;display:flex;flex-direction:column;align-items:flex-end;gap:9px;padding-top:3px;text-align:right;line-height:1.5}.markers{display:flex;gap:7px;align-items:center;color:var(--tp-accent);height:11px}.unread{width:5px;height:5px;border-radius:50%;background:var(--tp-warning)}.elapsed{font-size:10px;color:var(--tp-muted)}
</style>
