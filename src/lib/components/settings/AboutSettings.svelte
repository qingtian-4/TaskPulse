<script lang="ts">
  import {onMount} from 'svelte';import {invoke} from '@tauri-apps/api/core';import {t} from '$lib/i18n';import BrandMark from '../BrandMark.svelte';import SettingGroup from './SettingGroup.svelte';import SettingRow from './SettingRow.svelte';
  let info=$state<{version:string;commit:string;modified:boolean;profile:string;executable:string}|null>(null),error=$state('');
  async function load(){try{info=await invoke('get_build_info');}catch(e){error=String(e);}}
  onMount(()=>{void load();});
</script>
<div class="about-brand"><BrandMark size={52}/><div><h2>TaskPulse</h2><p>{$t('把注意力留给真正需要你的任务。')}</p></div></div>
<SettingGroup title={$t('应用信息')}><SettingRow label={$t('当前版本')}><span>{info?.version??'…'}</span></SettingRow><SettingRow label={$t('开源许可')}><span>MIT</span></SettingRow></SettingGroup>
<SettingGroup title={$t('数据与隐私')}><SettingRow label={$t('本机来源只读识别')} description={$t('读取任务状态，不修改 Agent 的原始会话。')}><span aria-hidden="true">✓</span></SettingRow><SettingRow label={$t('由你控制连接')} description={$t('检查更新连接 GitHub；远程来源仅在你主动添加后启用。')}><span aria-hidden="true">✓</span></SettingRow></SettingGroup>
<div class="settings-actions"><a class="tp-btn" href="https://github.com/qingtian-4/TaskPulse" target="_blank" rel="noopener noreferrer">{$t('GitHub 项目')}</a><a class="tp-btn" href="https://github.com/qingtian-4/TaskPulse/blob/main/LICENSE" target="_blank" rel="noopener noreferrer">{$t('查看许可')}</a></div>
{#if info}<details class="diagnostic tp-fold"><summary>{$t('诊断信息')}</summary><p>{info.commit.slice(0,12)} · {info.profile}</p><button class="tp-btn" onclick={async()=>{try{await navigator.clipboard.writeText(JSON.stringify(info,null,2));}catch(e){error=String(e);}}}>{$t('复制构建信息')}</button></details>{/if}
{#if error}<p class="error" role="alert">{error}<button class="tp-btn" onclick={load}>{$t('重试')}</button></p>{/if}
<style>.about-brand{display:flex;align-items:center;gap:15px;margin:4px 0 24px}.about-brand h2{margin:0;font-size:23px}.about-brand p{font-size:12px;color:var(--tp-muted);margin:4px 0}.diagnostic{margin-top:24px;font-size:12px}.diagnostic p,.diagnostic button{margin:12px}</style>
