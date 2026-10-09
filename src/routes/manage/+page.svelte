<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke, isTauri } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { taskContext, taskLabel } from '$lib/dashboard';
  import { formatAgo, type DashboardSettings, type MonitorSource, type SourceDiscovery, type SourceKind, type SessionPage, type ProviderHealth } from '$lib/types';
  let settings=$state<DashboardSettings | null>(null);
  let discovery=$state<SourceDiscovery[]>([]);
  let health=$state<ProviderHealth[]>([]);
  let tab=$state<'sources'|'sessions'|'notifications'>('sources');
  let selected=$state('');
  let query=$state('');
  let sessionSource=$state('');
  let offset=$state(0);
  let sessions=$state<SessionPage>({items:[],total:0});
  let error=$state('');
  let notice=$state('');
  let saving=$state(false);
  let loadingSessions=$state(false);
  let notificationError=$state('');
  let addKind=$state<SourceKind>('remote_codex');
  let dirty=$state(false);
  let editor=$derived(settings?.sources.find(s=>s.id===selected) ?? null);
  const kinds:Array<{kind:SourceKind;name:string}>=[{kind:'codex',name:'Codex · 本机'},{kind:'hermes',name:'Hermes · 本机'},{kind:'deepseek_desktop',name:'DeepSeek · Windows'},{kind:'deepseek_wsl',name:'DeepSeek · WSL'},{kind:'remote_codex',name:'Codex · 远程服务器'},{kind:'tmux',name:'tmux · 远程服务器'},{kind:'hooks',name:'通用 Hooks'},{kind:'scanner',name:'进程扫描'}];
  function list(value:string):string[]{return value.split(/[\n,]/).map(s=>s.trim()).filter(Boolean);}
  function changed(){dirty=true;}
  function add(){
    if(!settings)return;
    const builtin:Partial<Record<SourceKind,string>>={codex:'codex',hermes:'hermes',deepseek_desktop:'deepseek_harness_desktop',deepseek_wsl:'deepseek_harness',hooks:'local_file',scanner:'scanner'};
    const candidate=builtin[addKind];
    if(['codex','hooks','scanner'].includes(addKind) && settings.sources.some(s=>s.kind===addKind)){selected=settings.sources.find(s=>s.kind===addKind)!.id;notice='本机来源已存在，请在左侧启用或编辑。';return;}
    const id=candidate && !settings.sources.some(s=>s.id===candidate) ? candidate : addKind+':'+crypto.randomUUID();
    const source:MonitorSource={id,name:kinds.find(k=>k.kind===addKind)?.name ?? addKind,kind:addKind,enabled:true,location:addKind==='deepseek_wsl'?'Ubuntu':'',projects:['*'],excluded_projects:[],tmux_sessions:[],tmux_panes:[],poll_interval_ms:3000};
    settings.sources.push(source);selected=id;dirty=true;
  }
  function remove(id:string){if(!settings)return;settings.sources=settings.sources.filter(s=>s.id!==id);selected=settings.sources[0]?.id ?? '';dirty=true;}
  async function save(){
    if(!settings || !isTauri())return;
    saving=true;error='';notice='';
    try { settings.onboarding_complete=true;settings=await invoke<DashboardSettings>('save_monitoring_settings',{settings:$state.snapshot(settings)});discovery=await invoke<SourceDiscovery[]>('discover_sources');dirty=false;notice='已保存并即时生效。受影响来源正在重新采集。'; }
    catch(e){error=String(e);}finally{saving=false;}
  }
  let request=0;
  async function loadSessions(){
    if(!isTauri())return;
    const current=++request;loadingSessions=true;
    try {const result=await invoke<SessionPage>('list_discovered_sessions',{source:sessionSource || null,query,offset,limit:30});if(current===request)sessions=result;}
    catch(e){error=String(e);}finally{if(current===request)loadingSessions=false;}
  }
  async function sessionAction(id:string,action:string){
    try {await invoke('update_task_attention',{id,action});notice=action==='follow'?'已加入关注，不补发历史通知':'会话设置已更新';await loadSessions();}
    catch(e){error=String(e);}
  }
  async function testNotification(){try{await invoke('test_notification');notice='已发送测试通知；点击后会定位 TaskPulse。';}catch(e){error=String(e);}}
  onMount(()=>{
    if(!isTauri()){error='管理配置需要在 TaskPulse 托盘应用中打开。浏览器页面不会写入真实配置。';return;}
    const cleanups:Array<()=>void>=[];let mounted=true;
    void(async()=>{
      try {
        settings=await invoke<DashboardSettings>('get_monitoring_settings');
        error=await invoke<string | null>('get_monitoring_error') ?? '';
        discovery=await invoke<SourceDiscovery[]>('discover_sources');
        health=await invoke<ProviderHealth[]>('get_providers');
        notificationError=await invoke<string | null>('get_notification_error') ?? '';
        if(!settings.onboarding_complete){for(const source of settings.sources){source.enabled=discovery.find(d=>d.id===source.id)?.available ?? false;}dirty=true;}
        selected=settings.sources[0]?.id ?? '';
        const cleanup=await listen<ProviderHealth[]>('providers-updated',e=>health=e.payload);if(mounted)cleanups.push(cleanup);else cleanup();
        const notificationCleanup=await listen<string>('notification-error',e=>notificationError=e.payload);if(mounted)cleanups.push(notificationCleanup);else notificationCleanup();
      } catch(e){error=String(e);}
    })();
    const interval=setInterval(()=>{if(tab==='sessions')void loadSessions();},3000);
    return()=>{mounted=false;clearInterval(interval);cleanups.forEach(fn=>fn());};
  });
</script>

<div class="manage">
  <header><div><span class="eyebrow">TASKPULSE</span><h1>管理你的关注范围</h1><p>选择任务来源，交给 TaskPulse 跟踪新执行。</p></div><button class="primary" disabled={saving || !dirty || !settings} onclick={save}>{saving?'保存中…':dirty?'保存并生效':'配置已保存'}</button></header>
  <nav aria-label="管理页面"><button class:active={tab==='sources'} onclick={()=>tab='sources'}>监控来源</button><button class:active={tab==='sessions'} onclick={()=>{tab='sessions';void loadSessions();}}>已发现会话</button><button class:active={tab==='notifications'} onclick={()=>tab='notifications'}>通知与显示</button></nav>
  {#if error}<div class="message error" role="alert">{error}<button onclick={()=>error=''} aria-label="关闭错误">×</button></div>{/if}
  {#if notice}<div class="message" role="status">{notice}<button onclick={()=>notice=''} aria-label="关闭提示">×</button></div>{/if}
  {#if settings && !settings.onboarding_complete}<div class="welcome">首次设置：已预选发现的本机来源。确认需要的来源后，点击“保存并生效”。远程服务器由你主动添加。</div>{/if}
  <main class="glass-scroll">
    {#if settings && tab==='sources'}
      <div class="source-layout"><aside><div class="add"><select bind:value={addKind} aria-label="新增来源类型">{#each kinds as kind}<option value={kind.kind}>{kind.name}</option>{/each}</select><button onclick={add}>＋ 添加</button></div>
      {#each settings.sources as source (source.id)}<div class="source-item" class:chosen={selected===source.id}><input type="checkbox" bind:checked={source.enabled} onchange={changed} aria-label={'启用 '+source.name}/><button onclick={()=>selected=source.id}><strong>{source.name}</strong><span>{source.enabled ? health.find(h=>h.provider===source.id)?.status==='connected' ? '采集正常' : '等待采集 / 检查连接' : '未启用'}{#if health.find(h=>h.provider===source.id)?.last_success} · {formatAgo(health.find(h=>h.provider===source.id)!.last_success!)}{/if}</span></button></div>{/each}
      </aside><section class="editor">
        {#if editor}
          <h2>{editor.name}</h2>
          <label>显示名称<input bind:value={editor.name} oninput={changed}/></label>
          {#if editor.kind==='remote_codex' || editor.kind==='tmux'}<label>SSH 别名<input bind:value={editor.location} oninput={changed} placeholder="例如 Server1_wanghy"/></label><p class="help">使用系统 SSH 配置和已有登录方式，不在 TaskPulse 保存密码。</p><label>轮询周期（毫秒）<input type="number" min="500" max="60000" bind:value={editor.poll_interval_ms} oninput={changed}/></label>
          {:else if editor.kind==='deepseek_desktop'}<label>桌面版数据目录<input bind:value={editor.location} oninput={changed} placeholder="留空：使用 DSH_HOME 或本机 .dsh"/></label>
          {:else if editor.kind==='deepseek_wsl'}<label>WSL 发行版<input bind:value={editor.location} oninput={changed}/></label>
          {:else if editor.kind==='hermes'}<label>状态数据库<input bind:value={editor.location} oninput={changed} placeholder="留空：自动发现 Hermes state.db"/></label>{/if}
          {#if discovery.find(d=>d.id===editor!.id)}<p class="path">{discovery.find(d=>d.id===editor!.id)?.detail}</p>{/if}
          {#if health.find(h=>h.provider===editor!.id)?.message}<p class="message error">{health.find(h=>h.provider===editor!.id)?.message}</p>{/if}
          <h3>项目范围</h3>
          <label class="radio"><input type="radio" name="scope" checked={editor.projects.includes('*')} onchange={()=>{editor!.projects=['*'];changed();}}/>自动关注所有项目的新任务</label>
          <label class="radio"><input type="radio" name="scope" checked={!editor.projects.includes('*')} onchange={()=>{editor!.projects=[];changed();}}/>只关注指定项目</label>
          {#if !editor.projects.includes('*')}<label>项目名称或完整路径（每行一个）<textarea value={editor.projects.join('\n')} oninput={e=>{editor!.projects=list(e.currentTarget.value);changed();}} placeholder="agenttray&#10;D:\Project\my-project&#10;@unassociated"></textarea></label>{/if}
          <label>排除项目（可选，每行一个）<textarea value={editor.excluded_projects.join('\n')} oninput={e=>{editor!.excluded_projects=list(e.currentTarget.value);changed();}}></textarea></label>
          <p class="help">启用后只自动加入新执行，不导入空闲历史。排除规则优先。选择“未关联项目”时使用 @unassociated。</p>
          {#if editor.kind==='tmux'}<label>tmux session（留空表示全部）<input value={editor.tmux_sessions.join(', ')} oninput={e=>{editor!.tmux_sessions=list(e.currentTarget.value);changed();}}/></label><label>tmux pane（可选）<input value={editor.tmux_panes.join(', ')} oninput={e=>{editor!.tmux_panes=list(e.currentTarget.value);changed();}}/></label>{/if}
          <button class="remove" onclick={()=>remove(editor!.id)}>移除此监控来源</button><p class="help">移除只影响 TaskPulse，关注记录保留；恢复同一来源后继续使用。</p>
        {:else}<p class="help">选择一个来源，或添加新的监控来源。</p>{/if}
      </section></div>
    {:else if settings && tab==='sessions'}
      <div class="session-filters"><select bind:value={sessionSource} onchange={()=>{offset=0;void loadSessions();}} aria-label="会话来源"><option value="">全部来源</option>{#each settings.sources as source}<option value={source.id}>{source.name}</option>{/each}</select><input bind:value={query} placeholder="搜索任务标题、项目或路径" aria-label="搜索会话" oninput={()=>{offset=0;void loadSessions();}}/><button onclick={()=>loadSessions()} disabled={loadingSessions}>刷新目录</button></div>
      <p class="help">显示采集器已发现的会话，不是原生历史的完整镜像。尚未启用的来源不会持续采集。目录不等于关注列表。</p>
      {#each sessions.items as item (item.task.id)}<article class="session"><div><strong>{item.task.name}</strong><p>{taskContext(item.task,settings)}</p><small>{taskLabel(item.task)} · {item.reason} · 最后活动 {formatAgo(item.task.updated_at)}</small></div><div class="session-actions">{#if item.reason==='已设为不再监控'}<button onclick={()=>sessionAction(item.task.id,'unignore')}>恢复监控</button>{:else if item.reason==='正在关注'}<span>已关注</span>{:else}<button disabled={item.reason==='来源未启用' || item.reason==='来源已移除' || item.reason==='项目不在监控范围' || item.reason==='tmux 不在监控范围'} onclick={()=>sessionAction(item.task.id,'follow')}>加入关注</button>{/if}{#if item.reason!=='已设为不再监控'}<button onclick={()=>sessionAction(item.task.id,'ignore')}>不再监控</button>{/if}</div></article>{/each}
      {#if !sessions.items.length}<div class="empty">{loadingSessions?'正在读取…':'暂无已发现会话。启用来源后开始一个任务，或检查来源连接。'}</div>{/if}
      <div class="pagination"><span>共 {sessions.total} 项 · 当前 {sessions.total?offset+1:0}–{Math.min(offset+30,sessions.total)}</span><button disabled={offset===0} onclick={()=>{offset=Math.max(0,offset-30);void loadSessions();}}>上一页</button><button disabled={offset+30>=sessions.total} onclick={()=>{offset+=30;void loadSessions();}}>下一页</button></div>
    {:else if settings && tab==='notifications'}
      <section class="preferences"><h2>任务提醒</h2>{#if notificationError}<p class="message error" role="alert">最近通知发送失败：{notificationError}</p>{/if}<label class="radio"><input type="checkbox" bind:checked={settings.notifications} onchange={changed}/>任务结束、失败或需要确认时发送系统通知</label><label class="radio"><input type="checkbox" bind:checked={settings.sound} onchange={changed}/>播放通知声音</label><p class="help">每个任务分别通知；同一事件只提醒一次。通知消失后，未查看结果仍保留在面板。系统勿扰设置可能隐藏横幅。</p><button class="secondary" onclick={testNotification}>发送测试通知</button><h2>默认视图</h2><label class="radio"><input type="radio" name="view" checked={settings.default_view==='tasks'} onchange={()=>{settings!.default_view='tasks';changed();}}/>任务优先：需处理、运行中、待查看</label><label class="radio"><input type="radio" name="view" checked={settings.default_view==='projects'} onchange={()=>{settings!.default_view='projects';changed();}}/>按项目分组</label></section>
    {/if}
  </main>
</div>
<style>
  .manage{height:100vh;display:flex;flex-direction:column;background:#1b1e22;color:#d9dedb;font-family:inherit}header{padding:24px 28px 18px;display:flex;justify-content:space-between;align-items:center;border-bottom:1px solid #ffffff09}.eyebrow{color:#89a9b4;letter-spacing:.2em;font-size:10px}h1{font-size:21px;font-weight:550;margin:7px 0}header p{color:#929f9d;font-size:12px;margin:0}button{cursor:pointer;background:transparent;color:#b0c5cc;font-size:12px;border:0;border-radius:6px;padding:7px 9px}button:hover{background:#ffffff08}button:disabled{opacity:.4;cursor:default}.primary{background:#7698a1;color:#131e24;font-weight:600;padding:10px 15px}.primary:hover{background:#8dabb3}nav{display:flex;padding:10px 28px 0;gap:24px;border-bottom:1px solid #ffffff0b}nav button{border-radius:0;padding:10px 0 13px;color:#8f9b9e}nav button.active{color:#afccd7;border-bottom:2px solid #88aebf}main{flex:1;min-height:0;overflow:auto;padding:22px 28px}.source-layout{display:grid;grid-template-columns:255px minmax(0,1fr);gap:26px}.add{display:flex;gap:5px;margin-bottom:15px}.add select{min-width:0;flex:1}.source-item{display:flex;align-items:center;gap:9px;padding:7px 9px;border-radius:8px;margin:3px 0}.source-item.chosen{background:#97b9c70d;box-shadow:inset 2px 0 #85a7b6}.source-item button{text-align:left;flex:1;min-width:0;padding:6px 0}.source-item strong{display:block;font-size:12px;font-weight:500;overflow-wrap:anywhere}.source-item span{display:block;font-size:10px;color:#8a999c;margin-top:5px}input,select,textarea{background:#ffffff04;border:1px solid #ffffff13;border-radius:6px;color:#ced7d6;font-size:12px;padding:8px 10px;outline:none;max-width:100%}input:focus,textarea:focus,select:focus{border-color:#789caa}option{background:#24292d}input[type=checkbox],input[type=radio]{accent-color:#91aeb7}label{display:flex;flex-direction:column;gap:7px;font-size:12px;color:#a2afaf;margin:14px 0}.radio{display:flex;flex-direction:row;align-items:center;gap:9px;font-size:12px;margin:12px 0}h2{font-size:16px;font-weight:500;margin:0 0 17px}h3{font-size:13px;color:#b5c5c5;margin:24px 0 12px}textarea{resize:vertical;min-height:60px;font-family:inherit}.help{font-size:11px;line-height:1.8;color:#809195;margin:9px 0}.path{font-size:10px;overflow-wrap:anywhere;color:#81989e}.remove{color:#d5a6a0;margin-top:18px;border:1px solid #d5a6a025}.message,.welcome{padding:10px 28px;font-size:12px;line-height:1.7;color:#a8c7d0;background:#87b5c909;display:flex;align-items:center;gap:10px}.message button{margin-left:auto}.message.error{color:#dda69f;background:#dda69f08}.editor .message{padding:8px 10px}.session-filters{display:flex;gap:10px}.session-filters input{flex:1}.session-filters select{max-width:240px}.session{padding:16px 0;border-bottom:1px solid #ffffff0b;display:flex;gap:15px;justify-content:space-between}.session strong{font-size:13px;font-weight:500}.session p{font-size:11px;color:#96a6a6;margin:6px 0}.session small{font-size:10px;color:#7f9297}.session-actions{display:flex;align-items:center;gap:4px;flex-shrink:0}.session-actions span{font-size:11px;color:#8dad99}.pagination{display:flex;align-items:center;gap:10px;padding-top:17px;font-size:11px;color:#8b9a9d}.pagination span{margin-right:auto}.empty{text-align:center;padding:40px;font-size:12px;color:#8b9ca0}.preferences{max-width:640px}.preferences h2:not(:first-child){margin-top:32px}.secondary{border:1px solid #a3c0cc28;margin-top:10px}@media(max-width:720px){.source-layout{grid-template-columns:200px minmax(0,1fr);gap:15px}header,main{padding-left:18px;padding-right:18px}.session{flex-direction:column}.session-filters{flex-wrap:wrap}}
</style>
