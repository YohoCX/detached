<script lang="ts">
  let { repo }: { repo: string | null } = $props()
</script>

<!-- Agent D: file tree for `repo` at a ref. -->
<div class="slot muted">Project{repo ? '' : ': no repository'}</div>

<style>
  .slot { padding: 12px; }
</style>
