<script lang="ts">
  import {t} from '$lib/i18n';
  import {CAPABILITY_NAMES, CAPABILITY_EVIDENCE, type SourceDefinition} from '$lib/source-setup';
  let {definition}:{definition:SourceDefinition}=$props();
</script>

{#if definition.capabilities}
  <dl class="capabilities" aria-label={$t("来源能力")}>
    {#each Object.entries(CAPABILITY_NAMES) as [key,label]}<div><dt>{$t(label)}</dt><dd>{$t(CAPABILITY_EVIDENCE[definition.capabilities[key as keyof typeof CAPABILITY_NAMES]])}</dd></div>{/each}
    <div><dt>{$t("返回任务")}</dt><dd>{$t(definition.capabilities.return_target==='host_window'?'有目标时聚焦宿主窗口':'查看详情或复制信息')}</dd></div>
    <div><dt>{$t("支持平台")}</dt><dd>{definition.platforms.join(' / ')}</dd></div>
    <div><dt>{$t("实机验证平台")}</dt><dd>{definition.verified_platforms?.join(' / ')||$t("未记录实机验证")}</dd></div>
  </dl>
{/if}

<style>
.capabilities{display:grid;gap:5px;margin:12px}.capabilities>div{display:grid;grid-template-columns:minmax(90px,1fr) minmax(0,2fr);gap:10px}.capabilities dd{margin:0;color:var(--tp-muted);overflow-wrap:anywhere}
</style>
