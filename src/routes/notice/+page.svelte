<script lang="ts">
  import { onMount } from 'svelte';
  import { Activity, ArrowUpRight, X } from '@lucide/svelte';
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

<div class="notice-window" role="region" aria-label="TaskPulse 任务提醒" onpointerenter={()=>hovered=true} onpointerleave={()=>hovered=false}>
  <button class="close" aria-label="关闭提醒（保持未查看）" title="关闭提醒，任务仍未查看" disabled={busy || !noticeState.current} onclick={()=>void dismiss()}><X size={16} strokeWidth={1.7}/></button>
  {#if noticeState.current && content}
    {#key noticeState.current.id}
      <button class="message" disabled={busy} aria-label={'查看任务详情：'+content.heading} onclick={open}>
        <span class="avatar" aria-hidden="true"><Activity size={18} strokeWidth={1.8}/><span class={'status-dot '+content.tone}></span></span>
        <span class="copy">
          <span class="sender-line">
            <span class="sender" title={content.sender}>{content.sender}</span>
            {#if noticeState.current.test}<span class="test-badge">测试提醒</span>{/if}
          </span>
          <span class="heading" role="heading" aria-level={1} title={content.heading}>{content.heading}</span>
          <span class={'summary '+content.tone} role="status" title={content.summary}>{content.summary}</span>
        </span>
      </button>
    {/key}
    <footer>
      <div class="hint" title="自动收起或稍后处理都不会标记任务已查看">
        {#if error}<span class="error" role="alert">{error}</span>
        {:else if noticeState.pending}<button class="pending" disabled={busy} onclick={next} title="切换下一条，当前提醒仍保留">下一条 · 还有 {noticeState.pending} 条</button>
        {:else if noticeTimeout(noticeState.current)===null}<span>等待处理，不会自动收起</span>
        {:else}<span>{hovered?'鼠标移开后自动收起':Math.ceil(remaining/1000)+' 秒后收起'}</span>{/if}
      </div>
      <div class="actions"><button class="later" title="收起提醒，保持未查看" disabled={busy} onclick={()=>void dismiss()}>稍后处理</button><button class="primary" disabled={busy} onclick={open}>查看详情<ArrowUpRight size={12} strokeWidth={1.8}/></button></div>
    </footer>
    {#if noticeTimeout(noticeState.current)!==null}<div class="progress-track" aria-hidden="true"><div class="progress" style:width={(remaining/(noticeTimeout(noticeState.current) ?? 15000)*100)+'%'}></div></div>{/if}
  {:else if error}
    <p class="empty-error" role="alert">{error}</p>
  {/if}
</div>

<style>
  .notice-window {
    box-sizing:border-box; height:132px; width:100%; position:relative;
    display:flex; flex-direction:column; overflow:hidden; background:#1a1b1e;
    border:1px solid #ffffff14; border-radius:10px; color:#e0e5e9;
    font-family:'Segoe UI','Microsoft YaHei UI',-apple-system,sans-serif;
  }
  button { cursor:pointer; font:inherit; border:0; background:transparent; }
  button:disabled { cursor:default; opacity:.55; }
  button:focus-visible { outline:2px solid #8cb5c9; outline-offset:-3px; }
  .close { position:absolute; top:5px; right:5px; z-index:1; display:grid; place-items:center; width:24px; height:24px; border-radius:5px; color:#909aa6; }
  .close:hover { background:#ffffff0d; color:#dce4e6; }
  .message { min-height:0; flex:1; display:flex; align-items:flex-start; gap:10px; padding:11px 12px 9px; text-align:left; color:inherit; animation:arrive .18s ease-out; }
  .message:hover { background:#ffffff05; }
  .avatar { position:relative; flex-shrink:0; width:28px; height:28px; display:grid; place-items:center; background:#8cb5c91a; border:1px solid #8cb5c91a; border-radius:8px; color:#8cb5c9; }
  .status-dot { position:absolute; width:9px; height:9px; bottom:-2px; right:-2px; border:2px solid #1a1b1e; border-radius:50%; background:#8cb5c9; }
  .status-dot.success { background:#35a475; }
  .status-dot.attention { background:#dc8b24; }
  .status-dot.danger { background:#e45b5b; }
  .copy { flex:1; min-width:0; }
  .sender-line { display:flex; align-items:center; gap:6px; padding-right:28px; line-height:15px; }
  .sender { font-size:10.5px; color:#909aa6; white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }
  .test-badge { flex-shrink:0; color:#90afca; background:#8cb5c91a; padding:0 4px; border-radius:3px; font-size:9px; white-space:nowrap; }
  .heading { margin-top:4px; font-size:12.5px; font-weight:550; line-height:18px; overflow-wrap:anywhere; display:-webkit-box; -webkit-line-clamp:2; line-clamp:2; -webkit-box-orient:vertical; overflow:hidden; }
  .summary { display:block; margin-top:4px; font-size:11px; line-height:16px; color:#a7b0bb; overflow:hidden; white-space:nowrap; text-overflow:ellipsis; }
  .summary.danger { color:#e5a1a1; }
  .summary.attention { color:#d9b975; }
  footer { display:flex; align-items:center; justify-content:space-between; gap:6px; padding:4px 8px 5px 12px; border-top:1px solid #ffffff0a; }
  .hint { min-width:0; color:#84909d; font-size:9.5px; white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }
  .pending { color:#a7b0bb; }
  .actions { display:flex; align-items:center; gap:2px; flex-shrink:0; }
  .actions button { display:inline-flex; align-items:center; gap:3px; padding:3px 5px; border-radius:4px; font-size:11px; line-height:16px; }
  .later { color:#99a2ac; }
  .later:hover { background:#ffffff0a; }
  .primary { color:#91b5cc; font-weight:500; }
  .primary:hover { background:#8cb5c91a; }
  .progress-track { position:absolute; bottom:0; left:0; right:0; height:1px; background:#ffffff08; }
  .progress { height:100%; background:#8cb5c966; transition:width .2s linear; }
  .error { color:#e5a1a1; }
  .empty-error { padding:28px 14px; font-size:11px; color:#e5a1a1; overflow-wrap:anywhere; }
  @keyframes arrive { from { opacity:0; transform:translateY(4px); } to { opacity:1; transform:translateY(0); } }
</style>
