<script lang="ts">
  import {t} from '$lib/i18n';
  import { onMount } from 'svelte';
  import { Activity, CircleAlert, CircleCheck, ArrowUpRight, X } from '@lucide/svelte';
  import { invoke, isTauri } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import type { NoticeState } from '$lib/types';
  import { noticeContent, noticeTimeout, noticeRotation } from '$lib/notice';
  let noticeState=$state<NoticeState>({current:null,pending:0});
  let remaining=$state(15000);
  let rotationRemaining=15000;
  let hovered=$state(false);
  let busy=$state(false);
  let error=$state('');
  let currentId:number | null=null;
  let acknowledgedId:number | null=null;
  let acknowledging=false;
  const content=$derived(noticeState.current ? noticeContent(noticeState.current) : null);
  function apply(next:NoticeState){
    if(next.current?.id!==currentId){currentId=next.current?.id ?? null;remaining=next.current ? noticeTimeout(next.current) ?? 15000 : 15000;rotationRemaining=15000;error='';busy=false;}
    noticeState=next;
  }
  async function dismiss(reason='user'){
    if(!noticeState.current || busy)return;
    busy=true;
    try{apply(await invoke<NoticeState>('dismiss_task_notice',{id:noticeState.current.id,reason}));}
    catch(e){error=String(e);}finally{busy=false;}
  }
  async function next(){
    if(!noticeState.current || busy)return;busy=true;
    try{apply(await invoke<NoticeState>('next_task_notice',{id:noticeState.current.id}));}catch(e){error=String(e);}finally{busy=false;}
  }
  async function acknowledge(){
    const id=noticeState.current?.id;
    if(!id || id===acknowledgedId || acknowledging)return;
    acknowledging=true;
    try{if(await invoke<boolean>('ack_task_notice',{id}))acknowledgedId=id;}catch(e){error='展示记录未确认：'+String(e);}finally{acknowledging=false;}
  }
  async function open(){
    if(!noticeState.current || busy)return;
    busy=true;
    try{await invoke('open_task_notice',{id:noticeState.current.id});apply(await invoke<NoticeState>('get_task_notice'));}
    catch(e){error=String(e);}finally{busy=false;}
  }
  onMount(()=>{
    if(!isTauri()){error='任务提醒仅在托盘应用中显示。';return;}
    let mounted=true;let cleanup:(()=>void) | undefined;
    void(async()=>{
      try{
        const unsubscribe=await listen<NoticeState>('task-notice-updated',e=>apply(e.payload));
        if(!mounted){unsubscribe();return;}cleanup=unsubscribe;
        const state=await invoke<NoticeState>('get_task_notice');if(mounted)apply(state);
      }catch(e){if(mounted)error=String(e);}
    })();
    let last=performance.now();
    const timer=setInterval(()=>{
      const now=performance.now();const elapsed=now-last;last=now;
      void acknowledge();
      if(noticeState.current && noticeTimeout(noticeState.current)!==null && !hovered && !busy && !error){remaining=Math.max(0,remaining-elapsed);if(remaining===0)void dismiss('timeout');}
      if(noticeState.current && noticeRotation(noticeState.current,noticeState.pending)!==null && !hovered && !busy && !error){rotationRemaining=Math.max(0,rotationRemaining-elapsed);if(rotationRemaining===0)void next();}
    },200);
    return()=>{mounted=false;cleanup?.();clearInterval(timer);};
  });
</script>

<div class="notice-window tp-theme tp-tone" data-tone={content?.tone==='attention'?'action':content?.tone==='info'?'brand':content?.tone ?? 'neutral'} role="region" aria-label={$t("TaskPulse 任务提醒")} onpointerenter={()=>hovered=true} onpointerleave={()=>hovered=false}>
  <button class="close tp-icon" aria-label={$t("关闭提醒（保持未查看）")} title={$t("关闭提醒，任务仍未查看")} disabled={busy || !noticeState.current} onclick={()=>void dismiss()}><X size={16} strokeWidth={1.7}/></button>
  {#if noticeState.current && content}
    {#key noticeState.current.id}
      <button class="message" disabled={busy} aria-label={'查看任务详情：'+content.heading} onclick={open}>
        <span class="avatar" aria-hidden="true">{#if content.tone==='success'}<CircleCheck size={17} strokeWidth={1.7}/>{:else if content.tone==='attention' || content.tone==='danger'}<CircleAlert size={17} strokeWidth={1.7}/>{:else}<Activity size={17} strokeWidth={1.7}/>{/if}<span class={'status-dot '+content.tone}></span></span>
        <span class="copy">
          <span class="sender-line">
            <span class="sender" title={content.sender}>{content.sender}</span>
            {#if noticeState.current.test}<span class="test-badge tp-chip">{$t("测试提醒")}</span>{/if}
          </span>
          <span class="heading" role="heading" aria-level={1} title={noticeState.current.context ? content.heading : $t(content.heading)}>{noticeState.current.context ? content.heading : $t(content.heading)}</span>
          <span class={'summary '+content.tone} role="status" title={$t(content.summary)}>{$t(content.summary)}</span>
        </span>
      </button>
    {/key}
    <footer>
      <div class="hint" title={$t("自动收起或稍后处理都不会标记任务已查看")}>
        {#if error}<span class="error" role="alert">{error}</span>
        {:else if noticeState.pending}<button class="pending" disabled={busy} onclick={next} title={$t("切换下一条，当前提醒仍保留")}>{$t("下一条 · 还有")} {noticeState.pending} {$t("条")}</button>
        {:else if noticeTimeout(noticeState.current)===null}<span>{$t("等待处理，不会自动收起")}</span>
        {:else}<span>{hovered?$t('鼠标移开后自动收起'):$t('{seconds} 秒后收起',{seconds:Math.ceil(remaining/1000)})}</span>{/if}
      </div>
      <div class="actions"><button class="later tp-btn" data-variant="ghost" title={$t("收起提醒，保持未查看")} disabled={busy} onclick={()=>void dismiss()}>{$t("稍后处理")}</button><button class="primary tp-btn" data-variant="ghost" disabled={busy} onclick={open}>{$t("查看详情")}<ArrowUpRight size={12} strokeWidth={1.8}/></button></div>
    </footer>
    {#if noticeTimeout(noticeState.current)!==null}<div class="progress-track" aria-hidden="true"><div class="progress" style:width={(remaining/(noticeTimeout(noticeState.current) ?? 15000)*100)+'%'}></div></div>{/if}
  {:else if error}
    <p class="empty-error" role="alert">{error}</p>
  {/if}
</div>

<style>
  .notice-window{box-sizing:border-box;height:100dvh;width:100%;position:relative;display:flex;flex-direction:column;overflow:hidden;background:var(--tp-panel);border:1px solid var(--tp-border-strong);border-radius:var(--tp-radius-lg);color:var(--tp-text);box-shadow:var(--tp-shadow-card)}
  .notice-window::before{content:'';position:absolute;left:0;top:11px;bottom:11px;width:2px;border-radius:2px;background:var(--tone);opacity:.8;pointer-events:none}
  button{cursor:pointer;font:inherit;border:0;background:transparent}.close{position:absolute;top:5px;right:5px;z-index:1;width:24px;height:24px;color:var(--tp-quiet)}.close:hover{background:var(--tp-hover);color:var(--tp-text)}
  .message{min-height:0;flex:1;display:flex;align-items:flex-start;gap:10px;padding:11px 12px 9px 13px;text-align:left;color:inherit}.message:hover{background:var(--tp-hover)}
  .avatar{position:relative;flex-shrink:0;width:28px;height:28px;display:grid;place-items:center;background:var(--tone-soft);border:1px solid var(--tone-line);border-radius:var(--tp-radius-md);color:var(--tone);box-shadow:inset 0 1px 0 #ffffff06}
  .status-dot{position:absolute;width:8px;height:8px;bottom:-2px;right:-2px;border:2px solid var(--tp-panel);border-radius:50%;background:var(--tone)}
  .copy{flex:1;min-width:0}.sender-line{display:flex;align-items:center;gap:6px;padding-right:25px;line-height:15px}.sender{font-size:10px;color:var(--tp-muted);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
  .test-badge{flex-shrink:0;padding:0 4px;border-radius:3px;font-size:9px;line-height:14px;white-space:nowrap}
  .heading{margin-top:4px;font-size:12.5px;font-weight:600;line-height:18px;overflow-wrap:anywhere;display:-webkit-box;-webkit-line-clamp:2;line-clamp:2;-webkit-box-orient:vertical;overflow:hidden;letter-spacing:.005em}
  .summary{display:block;margin-top:4px;font-size:11px;line-height:16px;color:var(--tone);overflow:hidden;white-space:nowrap;text-overflow:ellipsis}
  footer{display:flex;align-items:center;justify-content:space-between;gap:6px;padding:4px 8px 5px 12px;border-top:1px solid var(--tp-border);background:var(--tp-card);color:var(--tp-text);flex:none}
  .hint{min-width:0;color:var(--tp-muted);font-size:9px;line-height:16px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
  .pending{color:var(--tp-muted);padding:2px 0;font-size:9px;line-height:16px}
  .actions{display:flex;align-items:center;gap:2px;flex-shrink:0}.actions .tp-btn{min-height:24px;gap:3px;padding:3px 5px;border-radius:var(--tp-radius-sm);font-size:10px;line-height:16px}.actions .later{color:var(--tp-text)}.actions .primary{color:var(--tp-accent);font-weight:550}.actions .primary:hover{background:var(--tp-accent-soft)}
  .progress-track{position:absolute;bottom:0;left:0;right:0;height:1px;background:var(--tp-border)}.progress{height:100%;background:var(--tone);opacity:.6;transition:width .2s linear}
  .error{color:var(--tp-danger)}.empty-error{padding:28px 14px;font-size:11px;color:var(--tp-danger);overflow-wrap:anywhere}
  @media(max-height:124px){.message{padding-top:8px;padding-bottom:6px;gap:8px}.heading,.summary{margin-top:2px}}
</style>
