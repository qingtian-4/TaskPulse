<script lang="ts">
  import type {Snippet} from 'svelte';
  let {title,children}:{title:string;children:Snippet}=$props();
</script>
<section class="settings-section"><h2>{title}</h2><div class="settings-group">{@render children()}</div></section>
