<script lang="ts">
  import type {Snippet} from 'svelte';
  let {label,description='',children}:{label:string;description?:string;children:Snippet}=$props();
</script>
<div class="setting-row"><div class="setting-copy"><div class="setting-label">{label}</div>{#if description}<p class="setting-description">{description}</p>{/if}</div>{@render children()}</div>
