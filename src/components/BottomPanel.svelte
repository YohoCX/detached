<script lang="ts">
  import { app } from '../lib/app.svelte'
  import Log from './Log.svelte'
  import Pipelines from './Pipelines.svelte'

  const tabs = [['log', 'Log'], ['pipelines', 'Pipelines']] as const
  let height = $state(load())

  function load() {
    try {
      return Number(localStorage.getItem('bottom.height')) || 280
    } catch {
      return 280
    }
  }

  function resize(e: PointerEvent) {
    const grip = e.currentTarget as HTMLElement
    const [y, from] = [e.clientY, height]
    grip.setPointerCapture(e.pointerId)
    grip.onpointermove = (m) => (height = Math.max(120, Math.min(innerHeight - 240, from + y - m.clientY)))
    grip.onpointerup = () => {
      grip.onpointermove = null
      try {
        localStorage.setItem('bottom.height', String(height))
      } catch {}
    }
  }
</script>

<section class="bottom panel" style:height="{height}px">
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="grip" onpointerdown={resize}></div>
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
  .grip { position: absolute; top: 0; left: 0; right: 0; height: 5px; cursor: row-resize; z-index: 1; }
  header { display: flex; gap: 4px; padding: 6px 8px; border-bottom: 1px solid var(--line); flex: none; }
  .tab { border: 0; border-radius: 8px; background: none; padding: 3px 10px; }
  .tab.on { background: var(--selected); }
  .content { flex: 1; min-height: 0; }
</style>
