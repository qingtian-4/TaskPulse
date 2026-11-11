<script lang="ts">
  import {t} from '$lib/i18n';
  import {preferences} from '$lib/preferences';
  import type { TaskSnapshot, DashboardSettings } from '$lib/types';
  import { taskContext, taskProjectLabel, taskLabel, taskAgent, taskSummary, taskSummaryIsAppCopy, taskSource, isEndedResult, matchesSignal, taskTone,taskElapsed } from '$lib/dashboard';
  import Pin from '@lucide/svelte/icons/pin';
  let { tasks, settings, selectedId, onSelect, now=Date.now(), fitFour=false }: { tasks: TaskSnapshot[]; settings: DashboardSettings | null; selectedId: string | null; onSelect: (task: TaskSnapshot) => void; now?:number;fitFour?:boolean } = $props();
  function navigate(event:KeyboardEvent) {
    if(!['ArrowDown','ArrowUp','Home','End'].includes(event.key))return;
    const row=event.currentTarget as HTMLButtonElement;
    const rows=Array.from(row.closest('ul')?.querySelectorAll<HTMLButtonElement>('.task-row')??[]);
    const index=rows.indexOf(row);
    const next=event.key==='Home'?0:event.key==='End'?rows.length-1:Math.max(0,Math.min(rows.length-1,index+(event.key==='ArrowDown'?1:-1)));
    event.preventDefault();rows[next]?.focus();rows[next]?.scrollIntoView({block:'nearest'});
  }
</script>
<ul class="m-0 p-0 list-none" class:fit-four={fitFour}>
  {#each tasks as task (task.id)}
    {@const elapsed=taskElapsed(task,now)}
    <li class="task-item tp-tone" class:compact={$preferences.panel_density==='compact'} data-tone={taskTone(task)}>
      <button type="button" data-task-id={task.id} class:selected={selectedId===task.id} class:action-needed={matchesSignal(task,'action')} class="task-row" onclick={()=>onSelect(task)} onkeydown={navigate} aria-pressed={selectedId===task.id}>
        <span class="dot" class:running={task.status==='running' && !task.stale} class:failed={task.status==='failed'} class:waiting={task.status==='waiting_input'} class:stale={task.stale}></span>
        <span class="row-copy">
          <span class="row-title" title={task.name}>{task.name}</span>
          <span class="context" title={taskContext(task,settings)+(task.project_path?' · '+task.project_path:'')}>
            <span class="agent">{taskAgent(task)}</span>
            <span class="project">{taskProjectLabel(task)}</span>
            <span class="location-group">
              <span class="location">{task.remote ? 'SSH · '+(task.host ?? $t('远程')) : taskSource(task,settings??undefined)?.kind.endsWith('_wsl') ? 'WSL · '+taskSource(task,settings??undefined)?.location : $t('本机')}</span>
              {#if task.unread}<span class="unread" title={$t("尚未查看")} aria-label={$t("尚未查看")}></span>{/if}
              {#if task.pinned}<Pin size={11} aria-label={$t("固定")}/>{/if}
            </span>
            {#if (!task.model || !$preferences.show_model) && $preferences.show_elapsed && elapsed}
              <span class="elapsed" title={$t(isEndedResult(task)?'总时长':'运行时长')}>{elapsed}</span>
            {/if}
          </span>
          {#if task.model && $preferences.show_model}<span class="metadata"><span class="model" title={'来源最近上报的模型：'+task.model}><span>{$t("模型")}</span> {task.model}{#if task.reasoning_effort}<b class="effort" title={$t('推理强度')}>{task.reasoning_effort}</b>{/if}</span>{#if $preferences.show_elapsed && elapsed}<span class="elapsed" title={$t(isEndedResult(task)?'总时长':'运行时长')}>{elapsed}</span>{/if}</span>{/if}
          <span class="activity" title={taskSummaryIsAppCopy(task)?$t(taskSummary(task)):taskSummary(task)}>{taskSummaryIsAppCopy(task)?$t(taskSummary(task)):taskSummary(task)}</span>
        </span>
        <span class="row-status"><span class="status-label tp-chip">{$t(taskLabel(task))}{#if matchesSignal(task,'question')&&(task.input_request_ids?.length??0)>1}<span class="request-count" title={$t('待回答')}> {task.input_request_ids!.length}</span>{/if}</span></span>
      </button>
    </li>
  {/each}
</ul>
<style>
  ul{display:flex;flex-direction:column;gap:8px;padding-top:2px}
  .task-item{position:relative;margin:0;font-size:var(--tp-task-font-size,12px);line-height:1.5}
  .task-row{position:relative;width:100%;display:grid;grid-template-columns:minmax(0,1fr) auto;gap:6px 8px;font-size:inherit;text-align:left;padding:10px 11px 10px 23px;border:1px solid var(--tp-border);border-radius:8px;background:var(--tp-card);color:var(--tp-text);cursor:pointer;transition:background .12s,border-color .12s}
  .task-row:hover{background:var(--tp-raised);border-color:var(--tp-border-strong)}
  .task-row.selected{background:var(--tp-accent-soft);border-color:var(--tp-accent-line);box-shadow:inset 2px 0 var(--tp-accent)}
  .task-item[data-tone=action] .task-row,.task-item[data-tone=danger] .task-row{border-left:2px solid var(--tone)}
  .task-item[data-tone=action] .task-row.selected,.task-item[data-tone=danger] .task-row.selected{border-color:var(--tone-line)}
  .dot{position:absolute;left:10px;top:18px;width:6px;height:6px;border-radius:50%;background:var(--tone)}
  .row-copy,.row-status{display:contents}
  .row-title{grid-row:1;grid-column:1;min-width:0;font-size:inherit;line-height:1.5;font-weight:600;display:-webkit-box;-webkit-line-clamp:2;line-clamp:2;-webkit-box-orient:vertical;overflow:hidden;overflow-wrap:anywhere}
  .context{grid-row:2;grid-column:1/-1;display:flex;gap:8px;align-items:center;min-width:0;font-size:inherit;line-height:1.5;color:var(--tp-quiet);margin-top:0}
  .context>span{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.agent{max-width:34%;color:var(--tp-muted)}.project{flex:1;border-left:1px solid var(--tp-border-strong);padding-left:8px}.context .location-group{display:inline-flex;align-items:center;gap:5px;max-width:42%;overflow:visible}
  .location{min-width:0;overflow:hidden;text-overflow:ellipsis}
  .metadata{grid-row:3;grid-column:1/-1;display:flex;align-items:baseline;gap:8px;min-width:0;flex-wrap:wrap}.model{flex:1;min-width:0;font-size:inherit;line-height:1.5;color:var(--tp-muted);overflow-wrap:anywhere}.model>span{color:var(--tp-quiet);margin-right:4px}
  .effort{display:inline-flex;align-items:center;margin-left:6px;padding:0 5px;border-radius:4px;background:var(--tp-accent-soft);color:var(--tp-accent);font-size:10px;font-weight:600;white-space:nowrap}
  .activity{grid-row:4;grid-column:1/-1;font-size:inherit;line-height:1.5;color:var(--tp-muted);display:-webkit-box;-webkit-line-clamp:1;line-clamp:1;-webkit-box-orient:vertical;overflow:hidden;overflow-wrap:anywhere;margin-top:0}
  .task-row:not(:has(.metadata)) .activity{grid-row:3}
  .task-item[data-tone=action] .activity{color:var(--tp-action)}
  .status-label{grid-row:1;grid-column:2;align-self:start;max-width:112px;justify-content:center;font-size:inherit;line-height:1.5;padding:2px 6px;text-align:center}
  .location-group :global(svg){flex:none;color:var(--tp-quiet)}
  .unread{flex:none;width:5px;height:5px;border-radius:50%;background:var(--tone)}
  .elapsed{margin-left:auto;flex:none;font-size:inherit;color:var(--tp-quiet);font-variant-numeric:tabular-nums;white-space:nowrap}
  @media(max-width:380px){.task-row{padding-top:8px;padding-bottom:8px}.dot{top:16px}}
  @media(max-width:350px){.task-row{padding:8px 8px 8px 20px;gap:4px 6px}.dot{left:8px;top:16px}.status-label{max-width:90px;padding:2px 4px}.context{gap:4px}.project{padding-left:4px}}
  .fit-four .task-row{padding-top:7px;padding-bottom:7px;row-gap:4px}
  .compact .task-row{padding-top:8px;padding-bottom:8px}.compact .dot{top:16px}

  .task-item{font-size:var(--tp-task-font-size,12px);line-height:1.5}
  .task-row{font-size:inherit;row-gap:6px}
  .task-row :is(.row-title,.context,.model,.activity,.status-label,.elapsed,.effort){font-size:inherit;line-height:1.5;margin-top:0}
  .task-row .row-title{font-weight:600}
  .fit-four .task-row,.compact .task-row{row-gap:4px}
</style>
