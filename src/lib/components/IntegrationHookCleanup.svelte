<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  let {revision}:{revision:string}=$props();
  type Installation={source_id:string;adapter_id:string;orphaned:boolean};
  type Preview={token:string;path:string;entries:unknown[];notes:string[]};
  let records=$state<Installation[]>([]),preview=$state<Preview|null>(null),confirmed=$state(false),error=$state(''),message=$state(''),busy=$state(false);
  async function load(){records=await invoke<Installation[]>('integration_installations');}
  async function prepare(sourceId:string){busy=true;error='';message='';try{preview=await invoke<Preview>('integration_preview',{sourceId,remove:true});confirmed=false;}catch(e){error=String(e);}finally{busy=false;}}
  async function apply(){if(!preview||!confirmed)return;busy=true;error='';try{await invoke('integration_apply',{token:preview.token,confirmed:true});preview=null;confirmed=false;message='已撤销登记的 TaskPulse Hook；保留其他设置与 Hook。';await load();}catch(e){error=String(e);}finally{busy=false;}}
  onMount(()=>{void load().catch(e=>error=String(e));});
  $effect(()=>{void revision;void load().catch(e=>error=String(e));});
</script>

<section class="cleanup" aria-label="已登记 Hook 清理">
  <h3>宿主 Hook 登记</h3>
  <p>移除监控来源不会自动改写宿主。已安装的 Hook 在此独立保留，可随时预览并确认撤销。</p>
  <button disabled={busy} onclick={()=>void load().catch(e=>error=String(e))}>刷新 Hook 登记</button>
  {#each records as record (record.source_id)}
    <div class="record"><span>{record.adapter_id} · {record.source_id}{record.orphaned?' · 来源已移除':''}</span><button disabled={busy} onclick={()=>void prepare(record.source_id)}>预览撤销已登记 Hook</button></div>
  {/each}
  {#if preview}
    <section class="preview" aria-label="已移除来源 Hook 撤销差异">
      <h4>仅撤销 TaskPulse 自有条目</h4><p>{preview.path}</p><pre>{JSON.stringify(preview.entries,null,2)}</pre>
      {#each preview.notes as note}<p>{note}</p>{/each}
      <label><input type="checkbox" bind:checked={confirmed}/>我已审阅位置和条目，同意撤销这次差异</label>
      <button disabled={busy||!confirmed} onclick={apply}>确认撤销这次差异</button><button onclick={()=>{preview=null;confirmed=false;}}>取消，不修改文件</button>
    </section>
  {/if}
  {#if error}<p role="alert">{error}</p>{/if}{#if message}<p role="status">{message}</p>{/if}
</section>

<style>
  .cleanup{border:1px solid #ffffff20;border-radius:8px;padding:14px;margin:16px;color:#bbc3cc;font-size:12px}p{line-height:1.6;overflow-wrap:anywhere}.record{display:flex;flex-wrap:wrap;align-items:center;gap:10px;margin:10px 0}.record span{flex:1;overflow-wrap:anywhere}button{background:#272c33;border:1px solid #ffffff24;border-radius:6px;color:#c5dbe9;padding:7px 10px;cursor:pointer}button:disabled{opacity:.4;cursor:default}.preview{border:1px solid #b99a4166;padding:12px;margin-top:12px;border-radius:6px}pre{white-space:pre-wrap;overflow-wrap:anywhere;max-height:240px;overflow:auto;background:#14171b;padding:10px}label{display:flex;gap:8px;margin:12px 0}[role=alert]{color:#eb9292}
</style>
