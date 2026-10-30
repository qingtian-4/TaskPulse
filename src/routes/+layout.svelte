<script lang="ts">
  import '../app.css';
  import { onMount } from 'svelte';
  import { connectPreferences } from '$lib/preferences';
  let { children } = $props();
  let ready=$state(false), error=$state('');
  onMount(()=>{
    let active=true, stop:(()=>void)|undefined;
    void connectPreferences().then(cleanup=>{if(active){stop=cleanup;ready=true}else cleanup();}).catch(e=>{error=String(e);ready=true;});
    return()=>{active=false;stop?.();};
  });
</script>
{#if error}<p role="alert" class="boot-error">{error}</p>{/if}
{#if ready}{@render children()}{/if}
<style>.boot-error{padding:12px;color:var(--tp-danger);background:var(--tp-bg);font:13px/1.5 system-ui;margin:0}</style>
