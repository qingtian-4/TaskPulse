<script lang="ts">
  import type {Snippet} from 'svelte';
  let {label,description='',descriptionId,children}:{label:string;description?:string;descriptionId?:string;children:Snippet}=$props();
</script>
<div class="setting-row"><div class="setting-copy"><div class="setting-label">{label}</div>{#if description}<p class="setting-description" id={descriptionId}>{description}</p>{/if}</div>{@render children()}</div>
