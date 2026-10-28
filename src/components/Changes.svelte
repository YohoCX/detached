<script lang="ts">
  let { repo }: { repo: string | null } = $props()
</script>

<!-- Agent D: change list for `repo`. -->
<div class="slot muted">Changes{repo ? '' : ': no repository'}</div>

<style>
  .slot { padding: 12px; }
</style>
