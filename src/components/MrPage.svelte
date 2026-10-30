<script lang="ts">
  import Pipelines from './Pipelines.svelte'

  let { repo, number }: { repo: string; number: number } = $props()

  const tabs = ['Overview', 'Commits', 'Changes', 'Pipelines'] as const
  let tab = $state<(typeof tabs)[number]>('Overview')
</script>

<div class="page">
  <header>
    <span class="strong">!{number}</span>
    {#each tabs as t (t)}<button class="tab" class:on={tab === t} onclick={() => (tab = t)}>{t}</button>{/each}
  </header>
  <div class="content">
    {#if tab === 'Pipelines'}<Pipelines {repo} mr={number} />{:else}<div class="muted">{tab}</div>{/if}
  </div>
</div>

<style>
  .page { display: flex; flex-direction: column; height: 100%; }
  header { display: flex; align-items: center; gap: 4px; height: 44px; padding: 0 8px 0 16px; border-bottom: 1px solid var(--line); flex: none; }
  .strong { font-weight: 700; margin-right: 8px; }
  .tab { border: 0; border-radius: 8px; background: none; padding: 3px 10px; }
  .tab.on { background: var(--selected); }
  .content { flex: 1; min-height: 0; }
</style>
