<script lang="ts">
  import {t} from '$lib/i18n';
  import {eventAdapterLabel,localHookKind} from '$lib/source-setup';
  import {emit} from '@tauri-apps/api/event';
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { MonitorSource } from '$lib/types';
  type Capability={support:string;channel:string;validation:string;tested_versions:string[];limitations:string};
  type Manifest={adapter_id:string;agent_id:string;display_name:string;product_form:string;systems:string[];official_sources:string[];supported_versions:string[];capabilities:Record<string,Capability>};
  type Preview={token:string;source_id:string;path:string;action:string;entries:unknown[];decoded_command:string;notes:string[]};
  let {source,dirty,changed,localHook=false}:{source:MonitorSource;dirty:boolean;changed:()=>void;localHook?:boolean}=$props();
  let catalog=$state<Manifest[]>([]), text=$state(''), message=$state(''), error=$state(''), busy=$state(false);
  let preview=$state<Preview|null>(null), confirmed=$state(false), diagnostic=$state(''), exported=$state('');
  const adapter=$derived(catalog.find(m=>m.adapter_id===source.integration?.adapter_id));
  let platform=$state(''), mcpConfig=$state('');
  const standard=$derived(source.integration?.adapter_id==='standard-events');
  const installable=$derived(platform==='windows' && ['kimi-code-cli','trae-cn-ide','zcode-desktop'].includes(source.integration?.adapter_id??''));
  const platformName=$derived(({windows:'Windows',macos:'macOS',linux:'Linux'} as Record<string,string>)[platform]??'当前系统');
  const labels:Record<string,string>={session_discovery:'会话发现',title:'原生标题',project:'项目',turn_start:'执行开始',phase:'运行阶段',final_end:'最终本轮结束',failure:'本轮失败',approval_request:'批准请求',approval_resolution:'批准解除',question:'提问',focus:'原生跳转'};
  const supports:Record<string,string>={explicit:'明确事件',inferred:'受限关联',unsupported:'不支持'};
  const grades:Record<string,string>={documented:'文档核验', 'fixture-tested':'合成测试', 'native-tested':'实机验证'};
  function adapterName(m:Manifest){
    const label=eventAdapterLabel(m.adapter_id,m.display_name);
    return m.adapter_id==='standard-events'?$t(label):label;
  }
  async function load(){catalog=await invoke<Manifest[]>('integration_catalog');}
  function select(id:string){
    const m=catalog.find(m=>m.adapter_id===id);if(!m)return;
    source.integration={agent_id:m.agent_id,adapter_id:m.adapter_id,transport:'local',accept_since_ms:0};
    source.name=adapterName(m);preview=null;confirmed=false;mcpConfig='';changed();
  }
  async function action(fn:()=>Promise<void>){busy=true;error='';message='';try{await fn();}catch(e){error=String(e);}finally{busy=false;}}
  async function importManifest(){await action(async()=>{const m=await invoke<Manifest>('integration_import',{text});await load();await emit('integration-catalog-updated');message='描述文件已导入，但没有安装任何 Hook，也不会自动启用来源。';if(!source.integration?.accept_since_ms)select(m.adapter_id);});}
  async function makePreview(remove:boolean){await action(async()=>{preview=await invoke<Preview>('integration_preview',{sourceId:source.id,remove});confirmed=false;});}
  async function apply(){if(!preview||!confirmed)return;await action(async()=>{message=await invoke<string>('integration_apply',{token:preview!.token,confirmed:true});preview=null;confirmed=false;});}
  async function exportMcp(){await action(async()=>{const result=await invoke<{mcp_config:unknown}>('integration_export',{sourceId:source.id});if(!result.mcp_config)throw Error('请新建标准事件来源');mcpConfig=JSON.stringify(result.mcp_config,null,2);});}
  $effect(()=>{if(dirty){mcpConfig='';exported='';}});
  onMount(()=>{void Promise.all([load(),invoke<{platform:string}>('integration_environment').then(e=>platform=e.platform)]).catch(e=>error=String(e));});
</script>

<div class="integration-editor" data-integration-editor>
  {#if standard}
    <section class="universal tp-surface" aria-label={$t("连接步骤")}>
      <h4>{$t("连接步骤")}</h4>
      <ol>
        <li><strong>{$t("开启这个连接并应用更改")}</strong><br/>{$t("可以把上方的来源名称改成你要连接的工具名。")}</li>
        <li><strong>{$t("到对方工具里完成设置")}</strong><br/>{$t("支持本机 MCP（stdio）：生成并复制配置，把 taskpulse 条目加到对方已有的 MCP 设置，再重载 MCP。只支持远程 URL 的工具不能使用这份配置。")}</li>
        <li><strong>{$t("确认对方能发来状态")}</strong><br/>{$t("在对方工具里确认能找到 taskpulse_report，再用下方示例测试。连接成功后，仍需对方在任务开始或结束时主动上报。")}</li>
      </ol>
      {#if dirty || !source.enabled}<p role="status">{$t("打开此连接的开关并应用更改后，即可生成配置。")}</p>{/if}
      <div class="actions"><button class="tp-btn" disabled={busy||dirty||!source.enabled} onclick={exportMcp}>{$t("生成本机 MCP 配置")}</button></div>
      {#if mcpConfig}
        <pre aria-label={$t("MCP 接入配置")}>{mcpConfig}</pre><button class="tp-btn" onclick={()=>action(async()=>{await navigator.clipboard.writeText(mcpConfig);message='MCP 配置已复制。请合并到宿主配置；TaskPulse 没有改动任何宿主设置。';})}>{$t("复制 MCP 配置")}</button>
        <details class="tp-fold"><summary>{$t("查看 MCP 测试参数")}</summary><p>{$t("让对方调用 taskpulse_report，传入以下参数。这个测试不会创建任务。")}</p><pre>{JSON.stringify({kind:'connection.test',session_id:'taskpulse-connection-test',event_id:'test-'+source.id.slice(-8)},null,2)}</pre></details>
      {/if}
      <details class="tp-fold"><summary>{$t("使用脚本发送任务状态")}</summary><p>{$t("如果对方能用脚本发出任务状态，导出命令和示例，在真实任务事件发生时把 JSON 传给命令。")}</p><button class="tp-btn" disabled={busy||dirty||!source.enabled} onclick={()=>action(async()=>{const {argv,sample}=await invoke<{argv:string[];sample:unknown}>('integration_export',{sourceId:source.id});exported=JSON.stringify({argv,sample},null,2);})}>{$t("导出脚本命令和示例")}</button>{#if exported}<pre aria-label={$t("脚本命令和示例")}>{exported}</pre>{/if}</details>
      <p>{$t("TaskPulse 不会自动读取对方的聊天。工具没有本机 MCP，也不能用脚本上报时，可以跳过此连接。")}</p>
    </section>
  {:else if adapter && platform && !adapter.systems.includes(platform)}
    <p class="warning">{$t("该适配描述尚未支持")} {platformName}{$t("；本平台不提供新增 Hook 操作。可尝试通过 MCP 或脚本连接。")}</p>
  {/if}
  <details class="integration-advanced tp-fold"><summary>{$t("支持范围与排查")}</summary>
  {#if !localHook}<label>{$t("Agent / 适配描述")}<select aria-label={$t("Agent 适配描述")} value={source.integration?.adapter_id??'standard-events'} disabled={!!source.integration?.accept_since_ms} onchange={e=>select(e.currentTarget.value)}>{#each catalog.filter(m=>!localHookKind(m.adapter_id)) as m (m.adapter_id)}<option value={m.adapter_id}>{adapterName(m)}</option>{/each}</select></label>{/if}
  <p class="warning">{$t("这些能力按项标注。文档或合成测试通过 ≠ 当前宿主版本实机通过；Stop 不会自动产生完成提醒。")}</p>
  {#if adapter}
    <div class="capability-summary tp-surface"><div class="capability-title"><strong>{$t("支持边界")}</strong><span class="tp-chip">{$t("按项核验")}</span></div><p>{$t("明确事件")} {Object.values(adapter.capabilities).filter(c=>c.support==='explicit').length} {$t("项 · 受限关联")} {Object.values(adapter.capabilities).filter(c=>c.support==='inferred').length} {$t("项 · 不支持")} {Object.values(adapter.capabilities).filter(c=>c.support==='unsupported').length} {$t("项")}</p><p>{$t("实机验证版本：")}{[...new Set(Object.values(adapter.capabilities).flatMap(c=>c.tested_versions))].join('、')||$t('无记录 · 仍需真实宿主验收')}</p></div>
    <details class="capability-details tp-fold"><summary>{$t("逐项查看生命周期、审批与跳转能力")}</summary><div class="capabilities">{#each Object.entries(adapter.capabilities) as [key,c]}<div class="capability"><strong>{$t(labels[key]??key)}</strong><span class="capability-badges"><span class="tp-chip tp-tone" data-tone={c.support==='explicit'?'brand':'stale'}>{$t(supports[c.support]??c.support)}</span><span class="tp-chip tp-tone" data-tone="stale">{$t(grades[c.validation]??c.validation)}</span></span><small>{$t("证据渠道：")}{c.channel}。{c.limitations} {$t("实测版本：")}{c.tested_versions.join(', ')||$t('无')}</small></div>{/each}</div></details>
  {/if}
  <div class="actions">
    <button class="tp-btn" disabled={busy||dirty||!source.enabled} onclick={()=>action(async()=>{await invoke('integration_test',{sourceId:source.id});message='测试事件接收成功（不创建任务）。尚未验证真实 Agent 的开始、结束或审批事件。';})}>{$t("测试事件接收（不创建任务）")}</button>
  </div>
  {#if dirty}<p>{$t("请先保存来源配置，再测试或生成配置差异。修改既有 Agent 身份请新建来源。")}</p>{/if}
  {#if source.integration}<p class="identity">Agent ID：{source.integration.agent_id} {$t("· 来源 ID：")}{source.id}</p>
    <label>{$t("宿主版本（用户记录，不等于验收通过）")}<input aria-label={$t("宿主版本")} value={source.integration.host_version??''} placeholder={$t("未知 / 例如 3.11.2")} oninput={e=>{source.integration!.host_version=e.currentTarget.value;changed();}}/></label>{/if}
  <div class="actions">
    <button class="tp-btn" disabled={busy||dirty} onclick={()=>action(async()=>{diagnostic=JSON.stringify(await invoke('integration_diagnose',{sourceId:source.id}),null,2);})}>{$t("接入诊断")}</button>
    {#if !standard}<button class="tp-btn" disabled={busy||dirty} onclick={()=>action(async()=>{exported=JSON.stringify(await invoke('integration_export',{sourceId:source.id}),null,2);})}>{$t("导出标准接入说明")}</button>{/if}
    {#if installable}<button class="tp-btn" disabled={busy||dirty} onclick={()=>makePreview(false)}>{$t("预览新增 Hook")}</button><button class="tp-btn" disabled={busy||dirty} onclick={()=>makePreview(true)}>{$t("预览撤销 Hook")}</button>{:else}<p>{$t("当前平台没有此描述的已核验 Hook 安装方式。标准事件 / MCP 需要宿主主动接入，不会自动安装。")}</p>{/if}
  </div>
  {#if dirty}<p>{$t("请先保存来源配置，再测试或生成配置差异。修改既有 Agent 身份请新建来源。")}</p>{/if}
  {#if preview}
    <section class="preview" aria-label={$t("Hook 配置差异")}>
      <h4>{preview.action==='remove'?$t('移除'):$t('新增')} {$t("TaskPulse 自有 Hook")}</h4><p>{preview.path}</p>
      <pre>{JSON.stringify(preview.entries,null,2)}</pre><details class="tp-fold"><summary>{$t("查看命令的可读内容")}</summary><pre>{preview.decoded_command}</pre></details>
      {#each preview.notes as note}<p>{$t(note)}</p>{/each}
      <label class="confirmation"><input type="checkbox" bind:checked={confirmed}/>{$t("我已审阅上述文件位置与条目，同意写入这次差异")}</label>
      <button class="tp-btn" disabled={busy||!confirmed} onclick={apply}>{$t("确认写入这次差异")}</button><button class="tp-btn" onclick={()=>{preview=null;confirmed=false;}}>{$t("取消，不修改文件")}</button>
    </section>
  {/if}
  {#if diagnostic}<pre aria-label={$t("接入诊断结果")}>{diagnostic}</pre>{/if}
  {#if exported&&!standard}<pre aria-label={$t("标准接入说明")}>{exported}</pre>{/if}
  {#if !localHook}<details class="manifest tp-fold"><summary>{$t("导入社区 / 自定义 Agent 描述文件")}</summary><p>{$t("仅支持受限 JSON 字段映射，不执行其中的程序或写入任意宿主配置。以 integrations/community-template.json 为起点。")}</p><textarea aria-label={$t("适配描述 JSON")} bind:value={text} placeholder={$t("粘贴 manifest JSON")}></textarea><button class="tp-btn" disabled={busy||!text.trim()} onclick={importManifest}>{$t("验证并导入描述")}</button></details>{/if}
  </details>
  {#if message}<p role="status">{$t(message)}</p>{/if}{#if error}<p class="error" role="alert">{error}</p>{/if}
</div>

<style>
  .universal{padding:12px;margin:12px 0}.universal h4{margin:0;color:var(--tp-text)}.universal ol{list-style:decimal;padding-left:20px;font-size:13px;color:var(--tp-muted);line-height:1.7}.universal li{margin:8px 0}
  .integration-editor{border-top:1px solid var(--tp-border);border-bottom:1px solid var(--tp-border);margin:17px 0 12px;padding:15px 0 8px;font-size:13px;color:var(--tp-muted)}
  p{line-height:1.8;overflow-wrap:anywhere;margin:8px 0}label{display:grid;gap:6px;margin:12px 0;color:var(--tp-muted)}
  select,input,textarea{width:100%;box-sizing:border-box}textarea{height:140px;resize:vertical}
  .warning{color:var(--tp-action);font-size:12px;border-left:2px solid #edc38040;padding:4px 0 4px 10px;margin:14px 0}
  .capability-summary{padding:12px;background:linear-gradient(120deg,var(--tp-accent-soft),transparent 80%),var(--tp-card);border-color:var(--tp-accent-line);border-radius:var(--tp-radius-md);margin:12px 0}
  .capability-title{display:flex;align-items:center;gap:8px;justify-content:space-between}.capability-title strong{font-size:13px;font-weight:600;color:var(--tp-text)}.capability-title .tp-chip{font-size:12px;color:var(--tp-quiet)}
  .capability-summary p{font-size:13px;color:var(--tp-muted);margin:8px 0 0;line-height:1.8}.capability-summary p:last-child{border-top:1px solid var(--tp-border);padding-top:8px;color:var(--tp-stale)}
  .capability-details,.integration-advanced{margin:12px 0}.capability-details>summary,.integration-advanced>summary{font-size:13px}
  .capabilities{padding:0 12px}.capability{display:grid;grid-template-columns:minmax(80px,1fr) minmax(0,2fr);align-items:center;gap:6px;padding:11px 0;border-bottom:1px solid var(--tp-border)}
  .capability:last-child{border:0}.capability strong{font-weight:550;font-size:13px;color:var(--tp-text)}.capability-badges{display:flex;gap:5px;flex-wrap:wrap;justify-content:flex-end}.capability small{grid-column:1/-1;color:var(--tp-muted);font-size:12px;line-height:1.8;overflow-wrap:anywhere}
  .actions{display:flex;flex-wrap:wrap;gap:7px;margin:12px 0}.tp-btn{font-size:13px;min-height:30px}.actions p{width:100%;font-size:12px}
  .integration-advanced>.actions,.integration-advanced>p,.integration-advanced>label,.integration-advanced>.tp-fold{margin-left:12px;margin-right:12px}.identity{font-size:12px;color:var(--tp-quiet)}
  pre{white-space:pre-wrap;overflow-wrap:anywhere;max-height:260px;overflow:auto;background:var(--tp-input);border:1px solid var(--tp-border);padding:10px;border-radius:var(--tp-radius-sm);font-size:12px;color:var(--tp-muted);margin:10px 12px}
  .preview{border:1px solid #edc38045;background:#edc38005;border-radius:var(--tp-radius-md);padding:12px;margin:12px}.preview h4{font-size:13px;color:var(--tp-action);margin:0 0 10px}.preview pre{margin:10px 0}.preview .tp-btn{margin:4px 4px 0 0}
  .confirmation{display:flex;align-items:flex-start;gap:8px;color:var(--tp-text);line-height:1.8}.confirmation input{width:auto;margin-top:3px;flex:none}.error{color:var(--tp-danger)}
  .manifest p,.manifest textarea,.manifest>.tp-btn{margin:10px 12px}.manifest textarea{width:calc(100% - 24px)}
  @media(max-width:400px){.capability{grid-template-columns:1fr}.capability-badges{justify-content:flex-start}}
</style>
