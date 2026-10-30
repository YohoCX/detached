<script module lang="ts">
  import { register } from '../lib/actions'
  import type { LogRefs } from '../lib/api'
  import { app, say } from '../lib/app.svelte'
  import { view } from '../lib/view.svelte'

  // Kept between openings; the branch and path reset when the repo changes.
  const filters = $state({ repo: null as string | null, refs: { kind: 'all' } as LogRefs, text: '', author: '', path: '' })

  register('vcs.fileHistory', () => {
    const s = view.shown
    if (s?.kind !== 'file' && s?.kind !== 'diff') return say('Open a file to show its history')
    app.current = s.repo
    Object.assign(filters, { repo: s.repo, path: s.path })
    app.bottom = 'log'
  })
</script>

<script lang="ts">
  import { openUrl } from '@tauri-apps/plugin-opener'
  import { onDestroy } from 'svelte'
  import { api, newId, type LogQuery, type LogRow } from '../lib/api'
  import { checkout, notify, statusOf } from '../lib/app.svelte'
  import { chip, lanes, pick, when } from '../lib/log'
  import { popup } from '../lib/menu'
  import CommitDetails from './CommitDetails.svelte'
  import Graph, { LANE } from './Graph.svelte'
  import Grip from './Grip.svelte'
  import Icon from './Icon.svelte'
  import VirtualList from './VirtualList.svelte'

  let { repo }: { repo: string | null } = $props()

  let rows = $state.raw<LogRow[]>([])
  let width = $state(0)
  let graph = $state(true)
  let loading = $state(false)
  let truncated = $state(false)
  let error = $state<string | null>(null)
  let sel = $state.raw(new Set<string>())
  let anchor = 0
  let worktree = $state(false)
  let sideWidth = $state(420)
  let branches = $state<string[]>([])
  let newBranch = $state<{ sha: string; name: string; checkout: boolean } | null>(null)
  let reload = $state(0)
  let list = $state<{ reveal(i: number): void }>()
  let current = 0

  const st = $derived(statusOf(repo))
  // Reloads when HEAD moves or a fetch brings new commits, not on every status refresh.
  const head = $derived(`${st?.head} ${st?.branch} ${st?.behind}`)
  const shas = $derived(sel.size ? rows.filter((r) => sel.has(r.sha)).map((r) => r.sha) : [])
  const refsValue = $derived(filters.refs.kind === 'branch' ? `b:${filters.refs.name}` : filters.refs.kind)

  $effect.pre(() => {
    if (repo === filters.repo) return
    Object.assign(filters, { repo, refs: { kind: 'all' }, path: '' })
    sel = new Set()
  })

  $effect(() => {
    void reload
    const r = repo
    branches = []
    if (r) api.listBranches(r).then((b) => r === repo && (branches = [...b.local.map((l) => l.name), ...b.remote.map((x) => `${x.remote}/${x.name}`)]), () => {})
  })

  $effect(() => {
    void head
    void reload
    if (repo)
      start(repo, {
        refs: { ...filters.refs },
        path: filters.path.trim() || null,
        text: filters.text.trim() || null,
        author: filters.author.trim() || null,
      })
  })

  /** Old rows stay until the first batch, so a reload neither flickers nor loses the scroll position. */
  function start(r: string, q: LogQuery) {
    if (current) api.cancelLog(current)
    const id = (current = newId())
    const g = !q.text && !q.author
    let fresh = true
    const take = (more: LogRow[]) => {
      rows = fresh ? more : rows.concat(more)
      width = Math.max(fresh ? 0 : width, ...more.map(lanes))
      graph = g
      fresh = false
    }
    const end = (err: string | null, cut = false) => {
      if (fresh) take([])
      loading = false
      error = err
      truncated = cut
    }
    loading = true
    api.log(r, q, id, (e) => {
      if (e.queryId !== current) return
      if (e.kind === 'rows') take(e.rows)
      else if (e.kind === 'done') end(null, e.truncated)
      else end(e.error)
    }).catch((e) => id === current && end(String(e)))
  }

  const timers: Record<string, ReturnType<typeof setTimeout>> = {}
  function later(k: 'text' | 'author' | 'path', v: string) {
    clearTimeout(timers[k])
    timers[k] = setTimeout(() => (filters[k] = v), 300)
  }

  function setRefs(v: string) {
    filters.refs = v.startsWith('b:') ? { kind: 'branch', name: v.slice(2) } : { kind: v as 'all' | 'head' }
  }

  function choose(i: number, how: 'one' | 'toggle' | 'range' = 'one', vsWorktree = false) {
    sel = pick(rows, sel, anchor, i, how)
    if (how !== 'range') anchor = i
    worktree = vsWorktree
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return
    e.preventDefault()
    const i = sel.size ? Math.max(0, Math.min(rows.length - 1, anchor + (e.key === 'ArrowDown' ? 1 : -1))) : 0
    if (!rows[i]) return
    choose(i)
    list?.reveal(i)
  }

  async function browse(r: string, sha: string) {
    try {
      const url = await api.commitUrl(r, sha)
      if (url) await openUrl(url)
      else say('No web page is known for this repository', true)
    } catch (e) {
      say(String(e), true)
    }
  }

  function menu(i: number) {
    const [r, sha] = [repo!, rows[i].sha]
    if (!sel.has(sha)) choose(i)
    popup([
      { text: 'Copy Revision Number', action: () => navigator.clipboard.writeText(sha).then(() => say(`Copied ${sha}`)) },
      { text: 'New Branch…', action: () => (newBranch = { sha, name: '', checkout: true }) },
      { text: 'Checkout Revision', action: () => checkout(r, { kind: 'rev', sha }) },
      { text: 'Compare with Local', action: () => choose(i, 'one', true) },
      'separator',
      { text: 'Open in Browser', action: () => browse(r, sha) },
    ])
  }

  async function createBranch() {
    const [b, r, name] = [newBranch, repo, newBranch?.name.trim()]
    if (!b || !r || !name) return
    newBranch = null
    try {
      await api.createBranch(r, name, b.sha, false)
      say(`Created branch ${name}`)
      if (b.checkout) checkout(r, { kind: 'local', name })
      else reload++
    } catch (e) {
      notify({ level: 'error', title: `New branch ${name} failed`, body: String(e) })
    }
  }

  onDestroy(() => {
    if (current) api.cancelLog(current)
    current = 0
    Object.values(timers).forEach(clearTimeout)
  })
</script>

{#if !repo}
  <div class="empty muted">No repository</div>
{:else}
  <div class="log">
    <div class="bar">
      <select value={refsValue} onchange={(e) => setRefs(e.currentTarget.value)}>
        <option value="all">All branches</option>
        <option value="head">HEAD</option>
        {#each branches as b (b)}<option value="b:{b}">{b}</option>{/each}
      </select>
      <input class="text" placeholder="Text or hash" value={filters.text} oninput={(e) => later('text', e.currentTarget.value)} />
      <input placeholder="Author" value={filters.author} oninput={(e) => later('author', e.currentTarget.value)} />
      <input placeholder="Path" value={filters.path} oninput={(e) => later('path', e.currentTarget.value)} />
      <span class="muted status">{loading ? 'Loading…' : `${rows.length.toLocaleString()} commits`}</span>
      {#if truncated}<span class="warn">History truncated at 100,000 commits</span>{/if}
      <button class="icon-btn" title="Refresh" onclick={() => reload++}><Icon name="refresh" /></button>
    </div>
    {#if newBranch}
      {@const b = newBranch}
      <div class="new">
        <span class="muted">New branch at <span class="mono">{b.sha.slice(0, 10)}</span></span>
        <input placeholder="Branch name" bind:value={b.name} {@attach (el) => el.focus()}
          onkeydown={(e) => (e.key === 'Enter' ? createBranch() : e.key === 'Escape' && (newBranch = null))} />
        <label><input type="checkbox" bind:checked={b.checkout} /> Checkout</label>
        <button class="btn primary" disabled={!b.name.trim()} onclick={createBranch}>Create</button>
        <button class="btn" onclick={() => (newBranch = null)}>Cancel</button>
      </div>
    {/if}
    <div class="body">
      {#if error}
        <div class="empty error">{error}</div>
      {:else if !rows.length && !loading}
        <div class="empty muted">No commits</div>
      {:else}
        <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
        <div class="rows" tabindex="0" role="listbox" aria-multiselectable="true" {onkeydown}>
          <VirtualList bind:this={list} items={rows} height={28}>
            {#snippet row(r, i)}
              <!-- svelte-ignore a11y_click_events_have_key_events -->
              <div class="commit" class:selected={sel.has(r.sha)} role="option" aria-selected={sel.has(r.sha)} tabindex="-1"
                onclick={(e) => choose(i, e.shiftKey ? 'range' : e.metaKey ? 'toggle' : 'one')}
                oncontextmenu={(e) => (e.preventDefault(), menu(i))}>
                {#if graph}<div class="graph" style:width="{Math.min(width, 24) * LANE}px"><Graph row={r} /></div>{/if}
                <span class="subject">
                  {#each r.refs.slice(0, 2) as ref (ref)}{@const c = chip(ref)}<span class="chip ellipsis {c.kind}" title={ref}>{c.label}</span>{/each}
                  {#if r.refs.length > 2}<span class="chip more" title={r.refs.slice(2).join('\n')}>+{r.refs.length - 2}</span>{/if}
                  <span class="ellipsis">{r.subject}</span>
                </span>
                <span class="author muted ellipsis">{r.author}</span>
                <span class="date muted">{when(r.time)}</span>
              </div>
            {/snippet}
          </VirtualList>
        </div>
      {/if}
      <div class="side" style:width="{sideWidth}px">
        <Grip key="log.side" bind:size={sideWidth} edge="left" min={260} max={1000} />
        {#if shas.length}<CommitDetails {repo} {shas} {worktree} />{:else}<div class="empty muted">Select a commit</div>{/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .log { display: flex; flex-direction: column; height: 100%; }
  .bar, .new { display: flex; align-items: center; gap: 6px; padding: 6px 10px; flex: none; border-bottom: 1px solid var(--line); font-size: 12px; }
  .bar input { width: 120px; }
  .bar .text { width: 180px; }
  .bar select { max-width: 180px; }
  .status { margin-left: auto; white-space: nowrap; }
  .warn { color: var(--warning); white-space: nowrap; }
  .new input:not([type]) { width: 200px; }
  .new label { display: flex; align-items: center; gap: 4px; }
  .body { display: flex; flex: 1; min-height: 0; }
  .rows { flex: 1; min-width: 0; outline: none; }
  .side { position: relative; flex: none; border-left: 1px solid var(--line); }
  .empty { padding: 12px 16px; flex: 1; }
  .error { color: var(--error); white-space: pre-wrap; }
  .commit { display: flex; align-items: center; gap: 10px; height: 28px; margin: 0 6px; padding: 0 8px 0 4px; border-radius: 9px; white-space: nowrap; }
  .commit:hover { background: var(--hover); }
  .commit.selected { background: var(--selected-inactive); }
  .rows:focus-within .commit.selected { background: var(--selected); box-shadow: inset 0 0 0 1px var(--selected-ring); }
  .graph { flex: none; overflow: hidden; }
  .subject { display: flex; align-items: center; gap: 4px; flex: 1; min-width: 0; overflow: hidden; }
  .chip { flex: none; max-width: 160px; padding: 0 6px; border-radius: 6px; font-size: 11px; font-weight: 700; line-height: 18px; color: var(--c); background: color-mix(in srgb, var(--c) 15%, transparent); box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--c) 35%, transparent); }
  .chip.head { --c: var(--accent); }
  .chip.branch { --c: var(--success); }
  .chip.tag { --c: var(--warning); }
  .chip.more { --c: var(--muted); flex: none; }
  .author { width: 140px; flex: none; }
  .date { width: 150px; flex: none; text-align: right; font-size: 12px; }
</style>
