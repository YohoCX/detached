<script lang="ts">
  let { repo }: { repo: string | null } = $props()
</script>

<div class="placeholder muted">{repo ? 'Merge requests' : 'No repository'}</div>

<style>
  .placeholder { display: flex; align-items: center; justify-content: center; height: 100%; }
</style>
