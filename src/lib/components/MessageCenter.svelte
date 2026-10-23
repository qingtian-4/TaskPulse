<script lang="ts">
  import {getCurrentWindow} from '@tauri-apps/api/window';
  import {isTauri} from '@tauri-apps/api/core';
  import {SOURCE_LABEL,formatAgo,type MessageCenterState,type MessageEntry,type TaskSnapshot} from '$lib/types';
  import {MESSAGE_LABELS,messageStatus,unreadMessageIds} from '$lib/messages';
  let {center,prompts,now,onRead,onClose,onSelect,onReopen,onPrompt}:
    {center:MessageCenterState;prompts:TaskSnapshot[];now:number;onRead:(ids:string[])=>Promise<void>;
    onClose:()=>void;onSelect:(id:string)=>void;onReopen:(entry:MessageEntry)=>void;onPrompt:(task:TaskSnapshot)=>void}=$props();
  // A message counts as seen only after its row intersects the scroll viewport
  // and the actual native popup is visible. Mounting a hidden tray is not read.
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

<section class="message-center glass-panel" aria-label="消息中心">
  <header><h2>消息中心 <span>{center.unread} 条未读</span></h2><button class="link" disabled={!center.unread} onclick={()=>onRead(unreadMessageIds(center))}>全部标为已读</button><button class="tp-btn tp-icon" aria-label="关闭消息中心" onclick={onClose}>×</button></header>
  <p class="explanation">显示的消息标为已读；不会查看任务结果，也不会批准或解除请求。</p>
  {#if center.error}<p class="message-error" role="alert">{center.error}</p>{/if}
  <div class="message-scroll glass-scroll" aria-label="最近消息">
    {#each center.entries as entry (entry.id)}
      <article class:unseen={!entry.seen} data-message-id={entry.id} data-seen={entry.seen} use:readWhenVisible={entry}>
        <div class="event-line"><strong>{MESSAGE_LABELS[entry.kind]}</strong><span>{entry.seen?'已读':'未读'} · {formatAgo(entry.at,now)}</span></div>
        <button class="task-link" onclick={()=>onSelect(entry.task_id)}>{entry.task_name}</button>
        <p class="context">{entry.agent_name??SOURCE_LABEL[entry.source]}{entry.project?' · '+entry.project:''}</p>
        <div class="state-line"><span>{messageStatus(entry)}</span>{#if entry.can_reopen}<button class="link" onclick={()=>onReopen(entry)}>重新打开提醒</button>{/if}</div>
      </article>
    {:else}<p class="empty">暂无新消息。历史待核对请求不会补算为新通知。</p>{/each}
    {#if prompts.length}<details class="pending-history tp-fold"><summary>当前与待核对请求 {prompts.length}</summary><p>这里记录原 Agent 的请求，与消息已读无关。</p>{#each prompts as task (task.id)}<div class="pending-item"><button class="task-link" onclick={()=>onPrompt(task)}>{task.name}</button><span>{task.stale?'来源待确认，请在原 Agent 核对':'请在原 Agent 核对是否仍待处理'}{task.input_request_ids?.length ? ' · '+task.input_request_ids.length+' 个请求':''}</span></div>{/each}</details>{/if}
  </div>
  <footer>最多保留最近 200 条；旧消息不会因打开提醒而变成新消息。</footer>
</section>

<style>
  .message-center{position:absolute;z-index:4;top:49px;right:8px;left:8px;max-height:min(480px,calc(100% - 60px));display:flex;flex-direction:column;border:1px solid var(--tp-border-strong);border-radius:var(--tp-radius-lg);box-shadow:var(--tp-shadow-window);overflow:hidden}
  header{display:flex;align-items:center;gap:8px;padding:8px 10px;flex:none;border-bottom:1px solid var(--tp-border)}h2{margin:0;flex:1;font-size:12px;color:var(--tp-text)}h2 span{font-size:9px;font-weight:400;color:var(--tp-muted);white-space:nowrap}
  button{font-size:10px}.link{color:var(--tp-accent);white-space:nowrap}.link:disabled{opacity:.45}.explanation{padding:7px 12px;margin:0;font-size:10px;line-height:1.6;color:var(--tp-muted);flex:none}
  .message-scroll{overflow:auto;min-height:50px;flex:1}.message-error{font-size:10px;color:var(--tp-danger);margin:0;padding:7px 12px}article{padding:10px 12px;border-top:1px solid var(--tp-border)}article.unseen{background:var(--tp-accent-soft)}
  .event-line,.state-line{display:flex;gap:6px;align-items:flex-start;justify-content:space-between}.event-line strong{font-size:11px;color:var(--tp-text)}.event-line>span{font-size:9px;color:var(--tp-quiet);white-space:nowrap}.task-link{display:block;text-align:left;white-space:normal;overflow-wrap:anywhere;font-size:11px;color:var(--tp-text);line-height:1.6;padding:5px 0}
  .context,.state-line{font-size:10px;color:var(--tp-muted);line-height:1.6;margin:0;overflow-wrap:anywhere}.state-line{margin-top:5px}.state-line .link{flex:none}.empty{font-size:11px;color:var(--tp-muted);padding:10px 12px;line-height:1.7}
  .pending-history{margin:8px;font-size:10px;color:var(--tp-muted)}.pending-history summary{padding:8px}.pending-history p{margin:7px}.pending-item{padding:6px 8px;border-top:1px solid var(--tp-border)}.pending-item span{display:block;font-size:10px}
  footer{flex:none;padding:7px 12px;border-top:1px solid var(--tp-border);color:var(--tp-quiet);font-size:9px;line-height:1.6}
  @media(max-width:350px){header{gap:5px;padding:6px}h2 span{display:block}.state-line{flex-wrap:wrap}.explanation{padding:6px 9px}}
</style>
