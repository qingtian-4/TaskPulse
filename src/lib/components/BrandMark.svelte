<script lang="ts">
  import brand from '$lib/assets/brand.svg';
  let { size = 24 }: { size?: number } = $props();
</script>

<img src={brand} alt="" aria-hidden="true" width={size} height={size} draggable="false" />

<style>
  img { display: inline-block; flex: none; vertical-align: middle; }
</style>
