<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { MonitorSource } from '$lib/types';
  type Capability={support:string;channel:string;validation:string;tested_versions:string[];limitations:string};
  type Manifest={adapter_id:string;agent_id:string;display_name:string;product_form:string;official_sources:string[];supported_versions:string[];capabilities:Record<string,Capability>};
  type Preview={token:string;source_id:string;path:string;action:string;entries:unknown[];decoded_command:string;notes:string[]};
  let {source,dirty,changed}:{source:MonitorSource;dirty:boolean;changed:()=>void}=$props();
  let catalog=$state<Manifest[]>([]), text=$state(''), message=$state(''), error=$state(''), busy=$state(false);
  let preview=$state<Preview|null>(null), confirmed=$state(false), diagnostic=$state(''), exported=$state('');
  const adapter=$derived(catalog.find(m=>m.adapter_id===source.integration?.adapter_id));
  let advanced=$state(false);
  const installable=$derived(['kimi-code-cli','trae-cn-ide','zcode-desktop'].includes(source.integration?.adapter_id??''));
  const labels:Record<string,string>={session_discovery:'会话发现',title:'原生标题',project:'项目',turn_start:'执行开始',phase:'运行阶段',final_end:'最终本轮结束',failure:'本轮失败',approval_request:'批准请求',approval_resolution:'批准解除',question:'提问',focus:'原生跳转'};
  const supports:Record<string,string>={explicit:'明确事件',inferred:'受限关联',unsupported:'不支持'};
  const grades:Record<string,string>={documented:'文档核验', 'fixture-tested':'合成测试', 'native-tested':'实机验证'};
  async function load(){catalog=await invoke<Manifest[]>('integration_catalog');}
  function select(id:string){
    const m=catalog.find(m=>m.adapter_id===id);if(!m)return;
    source.integration={agent_id:m.agent_id,adapter_id:m.adapter_id,transport:'local',accept_since_ms:0};
    source.name=m.display_name;preview=null;confirmed=false;changed();
  }
  async function action(fn:()=>Promise<void>){busy=true;error='';message='';try{await fn();}catch(e){error=String(e);}finally{busy=false;}}
  async function importManifest(){await action(async()=>{const m=await invoke<Manifest>('integration_import',{text});await load();message='描述文件已导入，但没有安装任何 Hook，也不会自动启用来源。';if(!source.integration?.accept_since_ms)select(m.adapter_id);});}
  async function makePreview(remove:boolean){await action(async()=>{preview=await invoke<Preview>('integration_preview',{sourceId:source.id,remove});confirmed=false;});}
  async function apply(){if(!preview||!confirmed)return;await action(async()=>{message=await invoke<string>('integration_apply',{token:preview!.token,confirmed:true});preview=null;confirmed=false;});}
  onMount(()=>{void load().catch(e=>error=String(e));});
</script>

<div class="integration-editor" data-integration-editor>
  <h3>选择 Agent 与接入能力</h3>
  <p>只关注确认后的新执行。Agent 身份与所用模型分开；这里没有通用进程猜测或任意插件代码。</p>
  <label>Agent / 适配描述<select aria-label="Agent 适配描述" value={source.integration?.adapter_id??'standard-events'} disabled={!!source.integration?.accept_since_ms} onchange={e=>select(e.currentTarget.value)}>{#each catalog as m}<option value={m.adapter_id}>{m.display_name}</option>{/each}</select></label>
  {#if source.integration}
    <label>传输环境<select aria-label="通用事件传输" value="local"><option value="local">Windows 本机</option><option disabled>WSL（尚未实现）</option><option disabled>SSH（尚未实现）</option></select></label>
  {/if}
  <p class="warning">这些能力按项标注。文档或合成测试通过 ≠ 当前宿主版本实机通过；Stop 不会自动产生完成提醒。</p>
  {#if adapter}
    <div class="capability-summary"><strong>支持边界</strong><p>明确事件 {Object.values(adapter.capabilities).filter(c=>c.support==='explicit').length} 项 · 受限关联 {Object.values(adapter.capabilities).filter(c=>c.support==='inferred').length} 项 · 不支持 {Object.values(adapter.capabilities).filter(c=>c.support==='unsupported').length} 项</p><p>实机验证版本：{[...new Set(Object.values(adapter.capabilities).flatMap(c=>c.tested_versions))].join('、')||'无记录 · 仍需真实宿主验收'}</p></div>
    <details class="capability-details"><summary>逐项查看生命周期、审批与跳转能力</summary><div class="capabilities">{#each Object.entries(adapter.capabilities) as [key,c]}<div class="capability"><strong>{labels[key]??key}</strong><span>{supports[c.support]??c.support} · {grades[c.validation]??c.validation}</span><small>证据渠道：{c.channel}。{c.limitations} 实测版本：{c.tested_versions.join(', ')||'无'}</small></div>{/each}</div></details>
  {/if}
  <div class="actions">
    <button disabled={busy||dirty} onclick={()=>action(async()=>{await invoke('integration_test',{sourceId:source.id});message='测试事件接收成功（不创建任务）。尚未验证真实 Agent 的开始、结束或审批事件。';})}>测试事件接收（不创建任务）</button>
  </div>
  {#if dirty}<p>请先保存来源配置，再测试或生成配置差异。修改既有 Agent 身份请新建来源。</p>{/if}
  <details class="integration-advanced" bind:open={advanced}><summary>高级接入 · Adapter、Hook 差异与诊断</summary>
  {#if source.integration}<p class="identity">Agent ID：{source.integration.agent_id} · 来源 ID：{source.id}</p>
    <label>宿主版本（用户记录，不等于验收通过）<input aria-label="宿主版本" value={source.integration.host_version??''} placeholder="未知 / 例如 3.11.2" oninput={e=>{source.integration!.host_version=e.currentTarget.value;changed();}}/></label>{/if}
  <div class="actions">
    <button disabled={busy||dirty} onclick={()=>action(async()=>{diagnostic=JSON.stringify(await invoke('integration_diagnose',{sourceId:source.id}),null,2);})}>接入诊断</button>
    <button disabled={busy||dirty} onclick={()=>action(async()=>{exported=JSON.stringify(await invoke('integration_export',{sourceId:source.id}),null,2);})}>导出标准接入说明</button>
    {#if installable}<button disabled={busy||dirty} onclick={()=>makePreview(false)}>预览新增 Hook</button><button disabled={busy||dirty} onclick={()=>makePreview(true)}>预览撤销 Hook</button>{:else}<p>此描述无已核验安装 recipe，仅提供标准事件接入 / 导出。WorkBuddy 桌面 Hook 载荷待核验。</p>{/if}
  </div>
  {#if dirty}<p>请先保存来源配置，再测试或生成配置差异。修改既有 Agent 身份请新建来源。</p>{/if}
  {#if preview}
    <section class="preview" aria-label="Hook 配置差异">
      <h4>{preview.action==='remove'?'移除':'新增'} TaskPulse 自有 Hook</h4><p>{preview.path}</p>
      <pre>{JSON.stringify(preview.entries,null,2)}</pre><details><summary>查看命令的可读内容</summary><pre>{preview.decoded_command}</pre></details>
      {#each preview.notes as note}<p>{note}</p>{/each}
      <label class="confirmation"><input type="checkbox" bind:checked={confirmed}/>我已审阅上述文件位置与条目，同意写入这次差异</label>
      <button disabled={busy||!confirmed} onclick={apply}>确认写入这次差异</button><button onclick={()=>{preview=null;confirmed=false;}}>取消，不修改文件</button>
    </section>
  {/if}
  {#if diagnostic}<pre aria-label="接入诊断结果">{diagnostic}</pre>{/if}
  {#if exported}<pre aria-label="标准接入说明">{exported}</pre>{/if}
  <details><summary>导入社区 / 自定义 Agent 描述文件</summary><p>仅支持受限 JSON 字段映射，不执行其中的程序或写入任意宿主配置。以 integrations/community-template.json 为起点。</p><textarea aria-label="适配描述 JSON" bind:value={text} placeholder="粘贴 manifest JSON"></textarea><button disabled={busy||!text.trim()} onclick={importManifest}>验证并导入描述</button></details>
  </details>
  {#if message}<p role="status">{message}</p>{/if}{#if error}<p class="error" role="alert">{error}</p>{/if}
</div>

<style>
  .integration-editor{border-top:1px solid #ffffff14;margin-top:18px;padding-top:12px;font-size:12px;color:#b8bec5}p{line-height:1.6;overflow-wrap:anywhere}label{display:grid;gap:6px;margin:10px 0}select,input,textarea{background:#191c21;color:#e6e9ed;border:1px solid #ffffff25;border-radius:6px;padding:8px;width:100%;box-sizing:border-box}textarea{height:150px}.warning{color:#dfbd65}.capability{display:grid;grid-template-columns:110px 1fr;gap:5px;border-bottom:1px solid #ffffff0b;padding:8px 0}.capability small{grid-column:1/-1;color:#979fa8;line-height:1.5}.actions{display:flex;flex-wrap:wrap;gap:7px;margin:14px 0}button{background:#272c33;border:1px solid #ffffff24;border-radius:6px;color:#c5dbe9;padding:7px 10px;cursor:pointer}button:disabled{opacity:.4;cursor:default}pre{white-space:pre-wrap;overflow-wrap:anywhere;max-height:280px;overflow:auto;background:#14171b;padding:10px;border-radius:6px;font-size:11px}.preview{border:1px solid #b99a4166;border-radius:8px;padding:12px}.confirmation{display:flex;align-items:flex-start}.confirmation input{width:auto}.error{color:#eb9292}details{margin-top:12px}summary{cursor:pointer}.identity{color:#8eabc0}

  .capability-summary{border:1px solid var(--tp-border);background:#a3cbe605;border-radius:8px;padding:12px;margin:14px 0;color:#b9cbd4}.capability-summary strong{font-size:12px;font-weight:550}.capability-summary p{font-size:11px;margin:7px 0 0}.capability-details,.integration-advanced{border:1px solid var(--tp-border);border-radius:8px;padding:12px}.capability-details summary,.integration-advanced summary{color:var(--tp-accent);line-height:1.6}.capability small{font-size:11px}.integration-editor{margin-top:16px;padding-bottom:8px;border-bottom:1px solid var(--tp-border)}h3{font-size:13px;color:var(--tp-text);font-weight:550}.warning{font-size:11px}.actions button:hover:not(:disabled){background:#343e49}
</style>
