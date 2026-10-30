<script lang="ts">
  // One commit's details and changes, or for several the changes from the oldest's first parent to the newest.
  let { repo, shas }: { repo: string; shas: string[] } = $props()
</script>

<div class="muted">{shas.length} selected</div>
