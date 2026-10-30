<script lang="ts">
  import { app } from '../lib/app.svelte'
  import Grip from './Grip.svelte'
  import Log from './Log.svelte'
  import Pipelines from './Pipelines.svelte'

  const tabs = [['log', 'Log'], ['pipelines', 'Pipelines']] as const
  let height = $state(280)
</script>

<section class="bottom panel" style:height="{height}px">
  <Grip key="bottom" bind:size={height} edge="top" min={120} max={innerHeight - 240} />
  <header>
    {#each tabs as [tab, label] (tab)}
      <button class="tab" class:on={app.bottom === tab} onclick={() => (app.bottom = tab)}>{label}</button>
    {/each}
  </header>
  <div class="content">
    {#if app.bottom === 'log'}<Log repo={app.current} />{:else}<Pipelines repo={app.current} />{/if}
  </div>
</section>

<style>
  .bottom { position: relative; display: flex; flex-direction: column; flex: none; min-height: 0; overflow: hidden; }
  header { display: flex; gap: 4px; padding: 6px 8px; border-bottom: 1px solid var(--line); flex: none; }
  .tab { border: 0; border-radius: 8px; background: none; padding: 3px 10px; }
  .tab.on { background: var(--selected); }
  .content { flex: 1; min-height: 0; }
</style>
