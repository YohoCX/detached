<script lang="ts">
  import { untrack } from 'svelte'
  import { api, type MergeRequest, type MrFilter } from '../lib/api'
  import { app } from '../lib/app.svelte'
  import { cached, refresh } from '../lib/cache.svelte'
  import { filter as fuzzy } from '../lib/fuzzy'
  import { ago, bareTitle, isGitLab, setupProblem, tone } from '../lib/mr'
  import { showMr, view } from '../lib/view.svelte'
  import Icon from './Icon.svelte'

  let { repo }: { repo: string | null } = $props()

  const filters: [MrFilter, string][] = [['open', 'Open'], ['mine', 'Mine'], ['reviewRequested', 'Review requested'], ['assigned', 'Assigned to me'], ['all', 'All states']]
  let filter = $state<MrFilter>('open')
  let query = $state('')

  const key = $derived(`mrs\0${repo}\0${filter}`)
  const cur = $derived(cached<{ mrs?: MergeRequest[]; setup?: string }>(key))
  const rows = $derived(fuzzy(cur?.value?.mrs ?? [], query, (m) => `${m.number} ${m.title} ${m.author} ${m.sourceBranch}`).map((r) => r.item))
  const shown = $derived(view.shown?.kind === 'mr' && view.shown.repo === repo ? view.shown.number : null)

  function load(focus = false) {
    const [r, f] = [repo, filter]
    if (!r) return
    refresh(key, async () => {
      const setup = setupProblem(await api.forgeHosts(r))
      return setup ? { setup } : { mrs: await api.listMrs(r, f) }
    }, focus)
  }

  $effect(() => {
    void key
    untrack(() => load())
    const timer = setInterval(() => !document.hidden && load(), 60_000)
    return () => clearInterval(timer)
  })
</script>

<svelte:window onfocus={() => load(true)} />

<section>
  <header>
    <span class="title">Merge Requests</span>
    <button class="icon-btn" title="Refresh" onclick={() => load()}><Icon name="refresh" /></button>
  </header>
  {#if repo}
    <div class="bar">
      <input placeholder="Search" bind:value={query} />
      <select bind:value={filter}>{#each filters as [f, label] (f)}<option value={f}>{label}</option>{/each}</select>
    </div>
  {/if}
  {#if cur?.value && cur.error}<div class="stale error ellipsis" title={cur.error}>{cur.error}</div>{/if}
  {#if !repo}
    <div class="empty muted">No repository</div>
  {:else if !cur}
    <div class="empty muted">Loading…</div>
  {:else if cur.value?.setup}
    <div class="empty"><p class="muted">{cur.value.setup}</p><button class="btn" onclick={() => (app.settings = true)}>Open Settings</button></div>
  {:else if !cur.value}
    <div class="empty error">{cur.error}</div>
  {:else if !rows.length}
    <div class="empty muted">{query ? 'No merge requests match' : 'No merge requests'}</div>
  {:else}
    <div class="rows">
      {#each rows as m (m.number)}
        <button class="mr" class:selected={shown === m.number} title={m.title} onclick={() => showMr(repo, m.number)}>
          <div class="line">
            <span class="num">{isGitLab(m) ? '!' : '#'}{m.number}</span>
            <span class="ellipsis name">{bareTitle(m)}</span>
            {#if m.draft}<span class="tag">Draft</span>{/if}
            {#if m.state !== 'open'}<span class="tag">{m.state}</span>{/if}
            {#if m.pipeline}<span class="dot" style:color="var(--{tone(m.pipeline)})" title="Pipeline {m.pipeline}"></span>{/if}
          </div>
          <div class="line muted">
            <span class="ellipsis branch">{m.sourceBranch} → {m.targetBranch}</span>
            <span class="ellipsis">{m.author}</span>
            {#if m.approvedBy.length}<span class="ok" title="Approved by {m.approvedBy.join(', ')}">✓{m.approvedBy.length}</span>{/if}
            <span class="when">{ago(m.updated)}</span>
          </div>
        </button>
      {/each}
    </div>
  {/if}
</section>

<style>
  section { display: flex; flex-direction: column; height: 100%; padding-bottom: 6px; }
  header { display: flex; align-items: center; gap: 4px; height: 44px; padding: 0 8px 0 16px; flex: none; }
  .title { flex: 1; font-size: 14px; font-weight: 800; }
  .bar { display: flex; gap: 6px; padding: 0 10px 8px; flex: none; font-size: 12px; }
  .bar input { flex: 1; min-width: 0; }
  .bar select { width: 110px; }
  .empty { padding: 12px 16px; display: grid; gap: 10px; justify-items: start; }
  .empty p { margin: 0; }
  .error { color: var(--error); white-space: pre-wrap; }
  .stale { flex: none; padding: 0 16px 6px; font-size: 11px; white-space: nowrap; }
  .rows { flex: 1; min-height: 0; overflow-y: auto; }
  .mr { display: block; width: calc(100% - 12px); margin: 0 6px 2px; padding: 5px 10px; border: 0; border-radius: 10px; background: none; text-align: left; }
  .mr:hover { background: var(--hover); }
  .mr.selected { background: var(--selected); box-shadow: inset 0 0 0 1px var(--selected-ring); }
  .line { display: flex; align-items: center; gap: 6px; height: 19px; font-size: 12px; min-width: 0; }
  .line:first-child { font-size: 13px; font-weight: 600; }
  .num { flex: none; color: var(--muted); font-weight: 700; }
  .name { flex: 1; }
  .tag { flex: none; font-size: 10px; font-weight: 700; padding: 0 5px; border-radius: 4px; background: var(--hover); color: var(--muted); text-transform: capitalize; }
  .branch { flex: 0 1 auto; max-width: 60%; }
  .ok { flex: none; color: var(--success); font-weight: 700; }
  .when { flex: none; margin-left: auto; }
</style>
