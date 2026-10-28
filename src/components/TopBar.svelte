<script lang="ts">
  import { app, nameOf, statusOf, syncRepos } from '../lib/app.svelte'
  import Branches from './Branches.svelte'
  import Icon from './Icon.svelte'

  const st = $derived(statusOf(app.current))
</script>

<header>
  <button class="widget" title="Repositories" onclick={() => (app.sidebar = !app.sidebar)}>
    <Icon name="repo" size={14} />
    <span class="strong">{app.current ? nameOf(app.current) : 'No repository'}</span>
    <Icon name="down" size={12} />
  </button>
  <div class="anchor">
    <button class="widget" title="Branches" disabled={!app.current} onclick={() => (app.branches = app.branches ? null : 'checkout')}>
      <Icon name="branch" size={14} />
      <span>{st ? (st.branch ?? `${st.head?.slice(0, 8) ?? '?'} (detached)`) : '—'}</span>
      {#if st?.ahead}<span class="muted">↑{st.ahead}</span>{/if}
      {#if st?.behind}<span class="muted">↓{st.behind}</span>{/if}
      <Icon name="down" size={12} />
    </button>
    {#key app.branches}{#if app.branches}<Branches />{/if}{/key}
  </div>
  <span class="fill"></span>
  {#each [['fetch', 'Fetch All'], ['pull', 'Pull All']] as const as [kind, label] (kind)}
    <button class="icon-btn" title="{label} Repositories" disabled={!!app.sync || !app.repos.length} onclick={() => syncRepos(kind)}>
      <Icon name={kind} />
      {#if app.sync?.kind === kind}
        <span class="progress">{app.sync.done}/{app.sync.total}</span>
      {:else}
        <span>{label}</span>
      {/if}
    </button>
  {/each}
</header>

<style>
  header { display: flex; align-items: center; gap: 4px; height: 40px; padding: 0 8px; background: var(--surface); border-bottom: 1px solid var(--line); }
  .widget { display: flex; align-items: center; gap: 6px; height: 28px; padding: 0 8px; border: 0; border-radius: 6px; background: none; }
  .widget:hover { background: var(--hover); }
  .strong { font-weight: 600; }
  .anchor { position: relative; }
  .fill { flex: 1; }
  .progress { font-variant-numeric: tabular-nums; color: var(--accent); }
</style>
