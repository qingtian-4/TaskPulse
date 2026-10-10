<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke, isTauri } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import type { NoticeState } from '$lib/types';
  let noticeState=$state<NoticeState>({current:null,pending:0});
  let remaining=$state(15000);
  let hovered=$state(false);
  let busy=$state(false);
  let error=$state('');
  let currentId:number | null=null;
  function apply(next:NoticeState){
    if(next.current?.id!==currentId){currentId=next.current?.id ?? null;remaining=15000;error='';busy=false;}
    noticeState=next;
  }
  async function dismiss(){
    if(!noticeState.current || busy)return;
    busy=true;
    try{apply(await invoke<NoticeState>('dismiss_task_notice',{id:noticeState.current.id}));}
    catch(e){error=String(e);}finally{busy=false;}
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
        apply(await invoke<NoticeState>('get_task_notice'));
      }catch(e){error=String(e);}
    })();
    let last=performance.now();
    const timer=setInterval(()=>{
      const now=performance.now();const elapsed=now-last;last=now;
      if(noticeState.current && !hovered && !busy && !error){remaining=Math.max(0,remaining-elapsed);if(remaining===0)void dismiss();}
    },200);
    return()=>{mounted=false;cleanup?.();clearInterval(timer);};
  });
</script>

<div class="notice-window" role="region" aria-label="TaskPulse 任务提醒" onpointerenter={()=>hovered=true} onpointerleave={()=>hovered=false}>
  <header><span>TASKPULSE · {noticeState.current?.test?'测试提醒':'任务提醒'}</span><button aria-label="关闭提醒（保持未查看）" title="关闭提醒，任务仍未查看" disabled={busy} onclick={dismiss}>×</button></header>
  {#if noticeState.current}
    <div class="copy" role="status"><h1>{noticeState.current.title.replace('TaskPulse · ','')}</h1><p title={noticeState.current.body}>{noticeState.current.body}</p></div>
    <div class="actions"><button class="primary" disabled={busy} onclick={open}>查看详情</button><button disabled={busy} onclick={dismiss}>稍后处理</button>{#if noticeState.pending}<span>还有 {noticeState.pending} 条提醒</span>{/if}</div>
    <footer>{hovered?'鼠标移开后自动收起':Math.ceil(remaining/1000)+' 秒后收起'} · 不会标记已查看</footer>
    <div class="progress" style:width={(remaining/15000*100)+'%'}></div>
  {/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}
</div>

<style>
  .notice-window{box-sizing:border-box;height:245px;width:100%;position:relative;display:flex;flex-direction:column;background:#1c2227;border:1px solid #749caf42;border-radius:10px;color:#dce4e6;overflow:hidden;padding:16px 20px;font-family:inherit}header{display:flex;align-items:center;justify-content:space-between;gap:16px;color:#90a5af;font-size:10px;letter-spacing:.12em}button{background:transparent;border:0;border-radius:6px;color:#b3c6cf;cursor:pointer;font-size:12px;padding:7px 10px}button:hover{background:#ffffff0d}button:disabled{opacity:.5;cursor:default}header button{font-size:21px;padding:0 4px;line-height:1}.copy{flex:1;min-height:0;margin:17px 0 13px}.copy h1{font-size:16px;line-height:1.5;font-weight:550;margin:0 0 10px}.copy p{font-size:12px;line-height:1.7;overflow:hidden;overflow-wrap:anywhere;display:-webkit-box;-webkit-line-clamp:3;line-clamp:3;-webkit-box-orient:vertical;margin:0;color:#afbdc4}.actions{display:flex;align-items:center;gap:9px}.primary{background:#8cb5c9;color:#10242f;font-weight:600}.primary:hover{background:#a6cbdc}.actions span{font-size:10px;margin-left:auto;color:#9daeb7}footer{margin-top:12px;font-size:10px;color:#8599a3}.progress{position:absolute;height:2px;bottom:0;left:0;background:#8cb5c9;transition:width .2s linear}.error{font-size:11px;color:#e8ada1;overflow-wrap:anywhere}
</style>
