<script lang="ts">
  import {onMount} from 'svelte';
  import {invoke} from '@tauri-apps/api/core';
  import {Check,RefreshCw,X,ChevronRight,ArrowLeft} from '@lucide/svelte';
  import {t} from '$lib/i18n';
  import {canRead,readLabel,sshAgents,type NewServer,type SshProbe,type SshSelections} from '$lib/monitor-sources';
  import ProjectScope from './ProjectScope.svelte';
  let {existing,onadd,oncancel}:{existing:string[];onadd:(value:NewServer)=>Promise<boolean>;oncancel:()=>void}=$props();
  let step=$state(1),host=$state(''),name=$state(''),aliases=$state<string[]>([]),hostError=$state('');
  let report=$state<SshProbe|null>(null),testing=$state(false),saving=$state(false);
  const defaults=():SshSelections=>({codex:{enabled:false,projects:['*']},claude_code:{enabled:false,projects:['*']}});
  let agents=$state(defaults());
  let alive=true,ticket=0;
  onMount(()=>{void invoke<string[]>('discover_ssh_hosts').then(v=>{if(alive)aliases=v;}).catch(()=>{if(alive)hostError='无法读取 SSH 配置，可手动输入 Host';});return()=>{alive=false;ticket++;};});
  const duplicate=$derived(existing.includes(host.trim()));
  const valid=$derived(!!host.trim()&&!/\s|[\x00-\x1f]/.test(host.trim())&&!host.trim().startsWith('-')&&!duplicate);
  function editHost(value:string){host=value;report=null;ticket++;testing=false;agents=defaults();hostError='';}
  async function test(){
    const target=host.trim(),request=++ticket;testing=true;hostError='';report=null;
    try{const value=await invoke<SshProbe>('probe_ssh_source',{host:target});if(alive&&request===ticket&&host.trim()===target){report=value;agents.codex.enabled=canRead(value.codex);
      agents.claude_code.enabled=agents.claude_code.enabled&&canRead(value.claude_code);}}
    catch(e){if(alive&&request===ticket)hostError=String(e);}finally{if(alive&&request===ticket)testing=false;}
  }
  async function finish(){if(!report||saving)return;saving=true;try{const saved=await onadd({host:host.trim(),name:name.trim()||host.trim(),agents:$state.snapshot(agents),report});if(!saved)hostError='配置未保存，请检查冲突后重试。';}catch(e){hostError=String(e);}finally{saving=false;}}
  function focus(dialog:HTMLElement){
    (dialog.querySelector<HTMLElement>('input:not(:disabled)')??dialog.querySelector<HTMLElement>('button'))?.focus();
    const handler=(e:KeyboardEvent)=>{if(e.key==='Escape'&&!saving){e.preventDefault();oncancel();}if(e.key==='Tab'){const nodes=[...dialog.querySelectorAll<HTMLElement>('button:not(:disabled),input:not(:disabled),select:not(:disabled)')].filter(n=>n.getClientRects().length);if(e.shiftKey&&document.activeElement===nodes[0]){e.preventDefault();nodes.at(-1)?.focus();}else if(!e.shiftKey&&document.activeElement===nodes.at(-1)){e.preventDefault();nodes[0]?.focus();}}};
    dialog.addEventListener('keydown',handler);return{destroy(){dialog.removeEventListener('keydown',handler);}};
  }
</script>
<div class="ms-overlay">
  <dialog open aria-modal="true" class="ms-modal ssh-wizard" aria-labelledby="ssh-wizard-title" use:focus>
    <header class="modal-top">
      <div class="modal-topline">
        <h2 id="ssh-wizard-title">{$t('添加 SSH 服务器')}</h2>
        <button class="ms-btn icon-only ghost" aria-label={$t('关闭向导')} disabled={saving} onclick={oncancel}><X size={17}/></button>
      </div>
      <ol class="modal-stepper" aria-label={$t('添加服务器步骤')}>
        {#each ['选择 Host','检测连接','Agent 与项目'] as label,i}
          <li class:active={step===i+1} aria-current={step===i+1?'step':undefined}>{$t(label)}</li>
        {/each}
      </ol>
    </header>

    <div class="modal-body">
      {#if hostError}<div class="ms-error" role="alert">{$t(hostError)}</div>{/if}
      {#if step===1}
        <div class="two-cols">
          <label class="ms-field">{$t('SSH Host')}<input value={host} oninput={e=>editHost(e.currentTarget.value)} aria-label={$t('SSH Host')} placeholder="gpu-server" autocomplete="off"/></label>
          <label class="ms-field">{$t('显示名称')}<input bind:value={name} aria-label={$t('服务器显示名称')}/></label>
        </div>
        {#if aliases.length}
          <div class="host-choices">
            <h3>{$t('从已有 SSH 配置中选择')}</h3>
            <div class="alias-grid">
              {#each aliases as alias (alias)}
                <button class:selected={host===alias} class="alias-choice" aria-pressed={host===alias} disabled={existing.includes(alias)} title={existing.includes(alias)?$t('已添加'):undefined} onclick={()=>editHost(alias)}>
                  {alias}{#if host===alias}<Check size={14}/>{/if}
                </button>
              {/each}
            </div>
          </div>
        {/if}
        {#if duplicate}<p class="ms-error" role="alert">{$t('此 SSH Host 已添加，请管理现有服务器。')}</p>{/if}
      {:else if step===2}
        <div class="probe-heading">
          <strong>{host}</strong>
          <div class="test-action">
            <button class="ms-btn" disabled={testing} onclick={test}><RefreshCw size={14}/>{$t(testing?'正在检测…':report?'重新检测':'开始检测')}</button>
          </div>
        </div>
        <dl class="test-steps" aria-live="polite">
          <div><dt>{$t('SSH 连接')}</dt><dd class:ok={report?.connection==='connected'} class:problem={report?.connection==='disconnected'}>{$t(report?report.connection==='connected'?'连接通过':'连接失败':testing?'检测中…':'待检测')}</dd></div>
          {#each sshAgents as agent (agent.key)}
            <div>
              <dt>{agent.name}</dt>
              <dd class:ok={canRead(report?.[agent.key])} class:problem={report?.connection==='connected'&&!canRead(report?.[agent.key])}>{$t(report?.[agent.key].state==='partial'?'项目列表不完整':readLabel(report?.[agent.key].state))}</dd>
            </div>
          {/each}
        </dl>
        {#if report?.connection==='disconnected'}<p class="ms-error" role="alert">{$t(report.message)}</p>{/if}
      {:else}
        <div class="wizard-agents">
          {#each sshAgents as agent (agent.key)}
            {@const available=canRead(report?.[agent.key])}
            <label class:checked={agents[agent.key].enabled}>
              <input type="checkbox" bind:checked={agents[agent.key].enabled} disabled={!available}/>
              <strong>{agent.name}</strong>
              {#if !available}<span class="agent-state">{$t(readLabel(report?.[agent.key].state))}</span>{/if}
            </label>
          {/each}
        </div>
        {#each sshAgents.filter(agent=>agents[agent.key].enabled) as agent (agent.key)}
          <section class="wizard-projects">
            <h3>{agent.name} · {$t('项目范围')}</h3>
            <ProjectScope projects={agents[agent.key].projects} choices={report?.[agent.key].projects??[]} onchange={value=>agents[agent.key].projects=value}/>
            {#if !agents[agent.key].projects.includes('*')&&report?.[agent.key].state==='partial'}
              <p class="ms-warning">{$t('项目列表不完整，未列出的项目可以手动添加路径。')}</p>
            {/if}
          </section>
        {/each}
      {/if}
    </div>

    <footer class="modal-footer">
      <button class="ms-btn ghost" disabled={saving} onclick={()=>step>1?step--:oncancel()}>
        {#if step>1}<ArrowLeft size={13}/>{/if}{$t(step>1?'上一步':'取消')}
      </button>
      {#if step<3}
        <button class="ms-btn primary" disabled={step===1?!valid:testing||report?.connection!=='connected'} onclick={()=>step++}>{$t('下一步')}<ChevronRight size={13}/></button>
      {:else}
        <button class="ms-btn primary" disabled={saving} onclick={finish}><Check size={14}/>{$t(saving?'正在保存…':sshAgents.some(agent=>agents[agent.key].enabled)?'添加并开始监控':'保存连接，暂不启用')}</button>
      {/if}
    </footer>
  </dialog>
</div>
