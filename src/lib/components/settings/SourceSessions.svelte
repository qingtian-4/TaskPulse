<script lang="ts">
  import {onMount} from 'svelte';
  import {invoke} from '@tauri-apps/api/core';
  import {X,RefreshCw} from '@lucide/svelte';
  import {t} from '$lib/i18n';
  import type {MonitorSource,SessionPage} from '$lib/types';
  let {source,ondismiss}:{source:MonitorSource;ondismiss:()=>void}=$props();
  let page=$state<SessionPage>({items:[],total:0}),offset=$state(0),query=$state(''),loading=$state(false),error=$state('');
  let ticket=0,alive=true;
  async function read(){const n=++ticket;loading=true;error='';try{const result=await invoke<SessionPage>('list_discovered_sessions',{source:source.id,query,offset,limit:30});if(alive&&n===ticket)page=result;}catch(e){if(alive&&n===ticket)error=String(e);}finally{if(alive&&n===ticket)loading=false;}}
  async function act(id:string,action:string){try{await invoke('update_task_attention',{id,action});await read();}catch(e){error=String(e);}}
  onMount(()=>{void read();return()=>{alive=false;ticket++;};});
  function focus(dialog:HTMLDialogElement){
    const previous=document.activeElement as HTMLElement|null;
    dialog.querySelector<HTMLElement>('button')?.focus();
    const key=(e:KeyboardEvent)=>{
      if(e.key==='Escape'){e.preventDefault();ondismiss();}
      if(e.key!=='Tab')return;
      const nodes=[...dialog.querySelectorAll<HTMLElement>('button:not(:disabled),input:not(:disabled)')];
      if(e.shiftKey&&document.activeElement===nodes[0]){e.preventDefault();nodes.at(-1)?.focus();}
      else if(!e.shiftKey&&document.activeElement===nodes.at(-1)){e.preventDefault();nodes[0]?.focus();}
    };
    dialog.addEventListener('keydown',key);
    return{destroy(){dialog.removeEventListener('keydown',key);if(previous?.isConnected)previous.focus();}};
  }
</script>
<div class="ms-overlay"><dialog open aria-modal="true" class="ms-modal" aria-labelledby="sessions-title" use:focus><header class="modal-top"><div class="modal-topline"><h2 id="sessions-title">{source.name} · {$t('已发现会话')}</h2><button class="ms-btn icon-only ghost" aria-label={$t('关闭会话目录')} onclick={ondismiss}><X size={16}/></button></div><p>{$t('目录来自已采集会话，不会导入完整历史或补发通知。')}</p></header><div class="modal-body"><form class="tool-row" onsubmit={e=>{e.preventDefault();offset=0;void read();}}><input bind:value={query} placeholder={$t('搜索任务标题、项目或路径')} aria-label={$t('搜索会话')}/><button class="ms-btn" disabled={loading}><RefreshCw size={13}/>{$t('刷新')}</button></form>{#if error}<p class="ms-error" role="alert">{$t(error)}</p>{/if}{#each page.items as item (item.task.id)}<article class="session-entry"><div><strong>{item.task.name}</strong><small>{item.task.project_path||item.task.project||''} · {$t(item.reason)}</small></div><div>{#if item.reason==='已设为不再监控'}<button class="ms-btn small" onclick={()=>act(item.task.id,'unignore')}>{$t('恢复监控')}</button>{:else if item.reason==='正在关注'}<span class="ms-muted">{$t('已关注')}</span>{:else}<button class="ms-btn small" disabled={['来源未启用','来源已移除','项目不在监控范围','tmux 不在监控范围'].includes(item.reason)} onclick={()=>act(item.task.id,'follow')}>{$t('加入关注')}</button>{/if}{#if item.reason!=='已设为不再监控'}<button class="ms-btn small ghost" onclick={()=>act(item.task.id,'ignore')}>{$t('不再监控')}</button>{/if}</div></article>{/each}{#if !page.items.length}<p class="ms-muted">{$t(loading?'正在读取…':'暂无已发现会话。启用来源后开始一个任务，或检查来源连接。')}</p>{/if}</div><footer class="modal-footer"><button class="ms-btn" disabled={offset===0||loading} onclick={()=>{offset=Math.max(0,offset-30);void read();}}>{$t('上一页')}</button><span class="ms-muted">{page.total} {$t('项')}</span><button class="ms-btn" disabled={offset+30>=page.total||loading} onclick={()=>{offset+=30;void read();}}>{$t('下一页')}</button></footer></dialog></div>
<style>.session-entry{display:flex;gap:10px;justify-content:space-between;padding:12px 0;border-bottom:1px solid var(--ms-border);font-size:12px}.session-entry small{display:block;font-size:10px;color:var(--ms-muted);overflow-wrap:anywhere;margin-top:4px}.session-entry>div:last-child{display:flex;align-items:center;gap:6px;flex:none}.tool-row input{flex:1;min-width:0}</style>
