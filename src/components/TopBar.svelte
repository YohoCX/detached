<script lang="ts">
  import { app, nameOf, run, statusOf, syncRepos } from '../lib/app.svelte'
  import { chordFor } from '../lib/keymap'
  import { mac } from '../lib/platform'
  import Branches from './Branches.svelte'
  import Icon from './Icon.svelte'
  import MrChip from './MrChip.svelte'
  import PipelineBadge from './PipelineBadge.svelte'
  import RepoSwitcher from './RepoSwitcher.svelte'
  import StatusBar from './StatusBar.svelte'

  const st = $derived(statusOf(app.current))
  let switcher = $state(false)
</script>

<!-- Only elements carrying the attribute drag the window, so empty space gets it too. -->
<header class="panel" class:mac data-tauri-drag-region>
  <div class="anchor">
    <button class="widget repo" title="Switch repository" disabled={!app.repos.length} onclick={() => (switcher = !switcher)}>
      <span class="strong ellipsis">{app.current ? nameOf(app.current) : 'No repository'}</span>
      <Icon name="down" size={12} />
    </button>
    {#if switcher}<RepoSwitcher onclose={() => (switcher = false)} />{/if}
  </div>
  <div class="anchor">
    <button class="widget" title="Branches" disabled={!app.current} onclick={() => (app.branches = app.branches ? null : 'checkout')}>
      <span class="accent"><Icon name="branch" size={14} /></span>
      <span>{st ? (st.branch ?? `${st.head?.slice(0, 8) ?? '?'} (detached)`) : '—'}</span>
      {#if st?.ahead}<span class="muted">↑{st.ahead}</span>{/if}
      {#if st?.behind}<span class="muted">↓{st.behind}</span>{/if}
      <Icon name="down" size={12} />
    </button>
    {#key app.branches}{#if app.branches}<Branches />{/if}{/key}
  </div>
  <MrChip repo={app.current} />
  <PipelineBadge repo={app.current} />
  <StatusBar />
  <button class="search" onclick={() => run('search.everywhere')}>
    <Icon name="search" size={14} /><span class="fill">Search everywhere</span><kbd>{chordFor('search.everywhere')}</kbd>
  </button>
  {#each [['fetch', 'Fetch All'], ['pull', 'Pull All']] as const as [kind, label] (kind)}
    <button class="btn sync" class:primary={kind === 'pull'} title="{label} Repositories" disabled={!!app.sync || !app.repos.length}
      onclick={() => syncRepos(kind)}>
      <Icon name={kind} size={14} />
      {#if app.sync?.kind === kind}
        <span class="progress">{app.sync.done}/{app.sync.total}</span>
      {:else}
        <span>{label}</span>
      {/if}
    </button>
  {/each}
</header>

<style>
  header { display: flex; align-items: center; gap: 6px; height: 46px; padding: 0 8px; flex: none; }
  .mac { padding-left: 90px; }
  .widget { display: flex; align-items: center; gap: 7px; height: 32px; padding: 0 10px; border: 0; border-radius: 10px; background: none; min-width: 0; }
  .widget:hover { background: var(--hover); }
  .repo { background: var(--input); border: 1px solid var(--line); max-width: 320px; }
  .strong { font-weight: 700; }
  .accent { display: inline-flex; color: var(--accent); }
  .anchor { position: relative; min-width: 0; }
  .fill { flex: 1; text-align: left; }
  .search { display: flex; align-items: center; gap: 8px; width: 240px; height: 30px; padding: 0 10px 0 12px; border: 0; border-radius: 999px; background: var(--hover); color: var(--muted); }
  kbd { font: inherit; font-size: 11px; border: 1px solid var(--line); border-radius: 6px; padding: 0 5px; }
  .sync { display: inline-flex; align-items: center; gap: 6px; height: 30px; }
  .progress { font-variant-numeric: tabular-nums; }
</style>
