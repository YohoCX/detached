<script lang="ts">
  import type { Project } from '../lib/api'

  // Never {@html}: text nodes, with spans only from parsed ANSI codes.
  let { repo, project, id, name }: { repo: string; project: Project; id: number; name: string } = $props()
</script>

<div class="log">
  <header><span class="strong">{name}</span><span class="muted">{project.path} #{id}</span></header>
  <div class="placeholder muted">Job log</div>
</div>

<style>
  .log { display: flex; flex-direction: column; height: 100%; }
  header { display: flex; align-items: center; gap: 8px; height: 44px; padding: 0 16px; border-bottom: 1px solid var(--line); flex: none; }
  .strong { font-weight: 700; }
  .placeholder { display: flex; flex: 1; align-items: center; justify-content: center; }
</style>
