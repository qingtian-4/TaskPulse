<script lang="ts">
  import {t} from '$lib/i18n';
  import BrandMark from './BrandMark.svelte';
  import {getCurrentWindow} from '@tauri-apps/api/window';
  import {isTauri} from '@tauri-apps/api/core';
  import {SOURCE_LABEL,formatAgo,type MessageCenterState,type MessageEntry,type TaskSnapshot} from '$lib/types';
  import {MESSAGE_LABELS,messageStatus,unreadMessageIds} from '$lib/messages';
  let {center,prompts,now,onRead,onClose,onSelect,onReopen,onReopenPrompt,onPrompt}:
    {center:MessageCenterState;prompts:TaskSnapshot[];now:number;onRead:(ids:string[])=>Promise<void>;
    onClose:()=>void;onSelect:(id:string)=>void;onReopen:(entry:MessageEntry)=>void;onReopenPrompt:(task:TaskSnapshot)=>void;onPrompt:(task:TaskSnapshot)=>void}=$props();
  function modal(node:HTMLDialogElement) {
    const previous=document.activeElement as HTMLElement|null;
    node.showModal();
    function keydown(event:KeyboardEvent) {
      if(event.key!=='Tab')return;
      const controls=[...node.querySelectorAll<HTMLElement>('button:not(:disabled),a[href],input:not(:disabled),summary,[tabindex="0"]')].filter(el=>el.getClientRects().length);
      const first=controls[0],last=controls.at(-1);
      if(!first){event.preventDefault();return;}
      if(event.shiftKey && document.activeElement===first){event.preventDefault();last?.focus();}
      else if(!event.shiftKey && document.activeElement===last){event.preventDefault();first.focus();}
    }
    node.addEventListener('keydown',keydown);
    return {destroy(){node.removeEventListener('keydown',keydown);node.close();if(previous?.isConnected)previous.focus();}};
  }
  // Read requires viewport intersection and a visible native popup.
  function readWhenVisible(node:HTMLElement,entry:MessageEntry) {
    let destroyed=false,reading=false;
    const check=async()=>{
      if(destroyed||reading||entry.seen||document.visibilityState==='hidden')return;
      const root=node.closest('.message-scroll')!.getBoundingClientRect(),rect=node.getBoundingClientRect();
      const overlap=Math.min(root.bottom,rect.bottom)-Math.max(root.top,rect.top);
      if(overlap<Math.min(rect.height*.5,40))return;
      if(!isTauri()||!await getCurrentWindow().isVisible()||destroyed)return;
      reading=true;
      try{await onRead([entry.id]);}finally{reading=false;}
    };
    const observer=new IntersectionObserver(()=>void check(),{root:node.closest('.message-scroll'),threshold:[0,.5,1]});
    observer.observe(node);
    document.addEventListener('visibilitychange',check);
    window.addEventListener('focus',check);
    return {update(next:MessageEntry){entry=next;void check();},destroy(){destroyed=true;observer.disconnect();document.removeEventListener('visibilitychange',check);window.removeEventListener('focus',check);}};
  }
</script>

<dialog use:modal class="message-center glass-panel" aria-label={$t("消息中心")} oncancel={event=>{event.preventDefault();onClose();}}>
  <header><h2><BrandMark size={18}/> {$t("消息中心")} <span>{center.unread} {$t("条未读")}</span></h2><button class="link" disabled={!center.unread} onclick={()=>onRead(unreadMessageIds(center))}>{$t("全部标为已读")}</button><button class="tp-btn tp-icon" aria-label={$t("关闭消息中心")} onclick={onClose}>×</button></header>
  <p class="explanation">{$t("显示的消息标为已读；不会查看任务结果，也不会批准或解除请求。")}</p>
  {#if center.error}<p class="message-error" role="alert">{center.error}</p>{/if}
  <div class="message-scroll glass-scroll" aria-label={$t("最近消息")}>
    {#each center.entries as entry (entry.id)}
      <article class:unseen={!entry.seen} data-message-id={entry.id} data-seen={entry.seen} use:readWhenVisible={entry}>
        <div class="event-line"><strong>{$t(MESSAGE_LABELS[entry.kind])}</strong><span>{entry.seen?$t('已读'):$t('未读')} · {$t(formatAgo(entry.at,now))}</span></div>
        <button class="task-link" onclick={()=>onSelect(entry.task_id)}>{entry.task_name}</button>
        <p class="context">{entry.agent_name??SOURCE_LABEL[entry.source]}{entry.project?' · '+entry.project:''}</p>
        <div class="state-line"><span>{$t(messageStatus(entry))}</span>{#if entry.can_reopen}<button class="link" onclick={()=>onReopen(entry)}>{$t("重新打开提醒")}</button>{/if}</div>
      </article>
    {:else}<p class="empty">{$t("暂无新消息。历史待核对请求不会补算为新通知。")}</p>{/each}
    {#if prompts.length}<details class="pending-history tp-fold"><summary>{$t("当前与待核对请求")} {prompts.length}</summary><p>{$t("这里记录原 Agent 的请求，与消息已读无关。")}</p>{#each prompts as task (task.id)}<div class="pending-item" data-prompt-id={task.id}><button class="task-link" onclick={()=>onPrompt(task)}>{task.name}</button><span>{task.stale?$t('来源待确认，请在原 Agent 核对'):$t('请在原 Agent 核对是否仍待处理')}{task.input_request_ids?.length ? ' · '+task.input_request_ids.length+' 个请求':''}</span>{#if !task.stale && task.confidence!=='inferred'}<button class="link" onclick={()=>onReopenPrompt(task)}>{$t("重新打开提醒")}</button>{/if}</div>{/each}</details>{/if}
  </div>
  <footer>{$t("最多保留最近 200 条；旧消息不会因打开提醒而变成新消息。")}</footer>
</dialog>

<style>
  .message-center{position:absolute;margin:0;width:auto;padding:0;z-index:4;top:56px;right:8px;left:8px;max-height:calc(100% - 64px);display:flex;flex-direction:column;border:1px solid var(--tp-border-strong);border-radius:var(--tp-radius-lg);box-shadow:var(--tp-shadow-window);overflow:hidden;color:var(--tp-text)}
  .message-center::backdrop{background:rgb(0 0 0 / 25%)}
  header{display:flex;align-items:center;gap:8px;padding:12px;flex:none;border-bottom:1px solid var(--tp-border)}
  h2{margin:0;flex:1;font-size:14px}h2 span{font-size:11px;font-weight:400;color:var(--tp-muted);white-space:nowrap}
  button{font-size:11px}.link{color:var(--tp-accent);white-space:nowrap}.link:disabled{opacity:.45}
  .explanation{padding:8px 12px;margin:0;font-size:11px;line-height:1.6;color:var(--tp-muted);flex:none}
  .message-scroll{overflow:auto;min-height:50px;flex:1}.message-error{font-size:11px;color:var(--tp-danger);margin:0;padding:8px 12px}
  article{padding:12px;border-top:1px solid var(--tp-border)}article.unseen{background:var(--tp-accent-soft)}
  .event-line,.state-line{display:flex;gap:6px;align-items:flex-start;justify-content:space-between}.event-line strong{font-size:12px}.event-line>span{font-size:11px;color:var(--tp-quiet);white-space:nowrap}
  .task-link{display:block;text-align:left;white-space:normal;overflow-wrap:anywhere;font-size:13px;line-height:1.5;padding:8px 0}
  .context,.state-line{font-size:11px;color:var(--tp-muted);line-height:1.6;margin:0;overflow-wrap:anywhere}.state-line{margin-top:8px}.state-line .link{flex:none}.empty{font-size:11px;color:var(--tp-muted);padding:10px 12px;line-height:1.7}
  .pending-history{margin:8px;font-size:11px;color:var(--tp-muted)}.pending-history summary{padding:8px}.pending-history p{margin:7px}.pending-item{padding:6px 8px;border-top:1px solid var(--tp-border)}.pending-item span{display:block;font-size:11px}
  footer{flex:none;padding:8px 12px;border-top:1px solid var(--tp-border);color:var(--tp-quiet);font-size:11px;line-height:1.6}
  @media(max-width:350px){header{gap:4px;padding:8px}h2 span{display:block}.state-line{flex-wrap:wrap}.explanation{padding:8px}.event-line>span{white-space:normal;text-align:right}}
</style>
