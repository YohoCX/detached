<script lang="ts">
  // The repo's pipelines, or the MR's when `mr` is given.
  let { repo, mr }: { repo: string | null; mr?: number } = $props()
</script>

<div class="placeholder muted">{repo ? (mr ? `Pipelines of !${mr}` : 'Pipelines') : 'No repository'}</div>

<style>
  .placeholder { display: flex; align-items: center; justify-content: center; height: 100%; }
</style>
