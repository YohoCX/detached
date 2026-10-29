<script lang="ts">
  import { untrack } from 'svelte'
  import { api, type Branches, type CommitSummary, type LocalBranch, type RemoteBranch } from '../lib/api'
  import { app, checkout, nameOf, notify, recentOf, refresh, runOp, say, statusOf } from '../lib/app.svelte'
  import { filter, segments } from '../lib/fuzzy'
  import Icon from './Icon.svelte'
  import VirtualList from './VirtualList.svelte'

  type Item =
    | { kind: 'header'; label: string }
    | { kind: 'new' }
    | { kind: 'prune' }
    | { kind: 'local'; b: LocalBranch; indices: number[] }
    | { kind: 'remote'; b: RemoteBranch; indices: number[] }
  type Branch = Extract<Item, { kind: 'local' | 'remote' }>
  type Sub =
    | { kind: 'list' }
    | { kind: 'menu'; item: Branch }
    | { kind: 'new'; start: string; label: string }
    | { kind: 'track'; branch: string }
    | { kind: 'notMerged'; name: string; commits: CommitSummary[] }
    | { kind: 'deleteRemote'; b: RemoteBranch }

  const repo = app.current
  const intent = app.branches
  const head = statusOf(repo)?.branch ?? 'HEAD'
  let data = $state<Branches | null>(null)
  let error = $state('')
  let query = $state('')
  let sel = $state(0)
  let menuSel = $state(0)
  let busy = $state(false)
  let sub = $state<Sub>(intent === 'new' ? { kind: 'new', start: 'HEAD', label: head } : { kind: 'list' })
  let text = $state('')
  let andCheckout = $state(true)
  let popup: HTMLDivElement
  let focus = $state<HTMLElement>()
  let list = $state<VirtualList<Item>>()

  const rname = (b: RemoteBranch) => `${b.remote}/${b.name}`
  const close = () => (app.branches = null)

  async function load() {
    if (!repo) return
    try {
      data = await api.listBranches(repo)
    } catch (e) {
      error = String(e)
    }
  }
  load()

  const items = $derived.by(() => {
    if (!data) return []
    const out: Item[] = []
    const section = (label: string, list: Item[]) => {
      if (list.length) out.push({ kind: 'header', label }, ...list)
    }
    if (!query && intent !== 'delete') out.push({ kind: 'new' }, { kind: 'prune' })
    const locals = filter(data.local, query, (b) => b.name).map((m) => ({ kind: 'local' as const, b: m.item, indices: m.indices }))
    if (!query) section('Recent', recentOf(repo!).flatMap((n) => locals.filter((l) => l.b.name === n && !l.b.current)))
    section('Local', locals)
    section('Remote', filter(data.remote, query, rname).map((m) => ({ kind: 'remote' as const, b: m.item, indices: m.indices })))
    return out
  })

  const tracking = $derived(data ? filter(data.remote, text, rname).slice(0, 200) : [])

  $effect(() => {
    void sub.kind
    const el = focus?.isConnected ? focus : popup
    el.focus()
  })

  function move(d: number) {
    for (let i = sel + d; i >= 0 && i < items.length; i += d)
      if (items[i].kind !== 'header') {
        sel = i
        return list?.reveal(i)
      }
  }
  const firstSel = () => Math.max(0, items.findIndex((it) => it.kind !== 'header'))
  // After the list changes (query typed or pasted, branches loaded), never before it.
  // An Enter pressed before load applies once loaded, unless the query changed since.
  let enterPending = false
  $effect(() => {
    void items
    sel = untrack(firstSel)
    if (enterPending && data) {
      enterPending = false
      untrack(() => items[sel] && choose(items[sel]))
    }
  })
  const go = (s: Sub) => {
    menuSel = 0
    text = ''
    sub = s
  }

  /** Runs a local branch write; afterwards closes the popup or returns to the reloaded list. */
  async function act(f: () => Promise<Sub | void>, after: 'close' | 'list' = 'list') {
    busy = true
    try {
      const next = await f()
      if (next) return go(next)
      if (after === 'close') return close()
      go({ kind: 'list' })
      await load()
    } catch (e) {
      notify({ level: 'error', title: 'Branch operation failed', body: String(e) })
    } finally {
      busy = false
      refresh(repo!)
    }
  }

  function choose(it: Item) {
    if (it.kind === 'header') return
    if (it.kind === 'new') return go({ kind: 'new', start: 'HEAD', label: head })
    if (it.kind === 'prune') return prune()
    if (intent === 'delete') return it.kind === 'local' ? del(it.b.name) : go({ kind: 'deleteRemote', b: it.b })
    if (it.kind === 'local' && it.b.current) return go({ kind: 'menu', item: it })
    doCheckout(it)
  }

  function doCheckout(it: Branch) {
    close()
    checkout(repo!, it.kind === 'local' ? { kind: 'local', name: it.b.name } : { kind: 'remote', remote: it.b.remote, name: it.b.name })
  }

  const create = (start: string) =>
    text.trim() &&
    act(async () => {
      const name = text.trim()
      await api.createBranch(repo!, name, start, false)
      say(`Created branch ${name}`)
      if (andCheckout) checkout(repo!, { kind: 'local', name })
    }, 'close')

  const del = (branch: string, force = false) =>
    act(async () => {
      const r = await api.deleteBranch(repo!, branch, force)
      if (r.kind === 'notMerged') return { kind: 'notMerged', name: branch, commits: r.commits }
      say(`Deleted branch ${branch}`)
    })

  const setTracking = (branch: string, upstream: string | null) => act(() => api.setUpstream(repo!, branch, upstream))

  function prune() {
    close()
    runOp(repo!, 'Prune', (id, p) => api.prune(repo!, id, p)).then(
      () => say(`${nameOf(repo!)}: pruned stale remote branches`),
      (e) => say(String(e), true),
    )
  }

  function deleteRemote(b: RemoteBranch) {
    close()
    runOp(repo!, `Delete ${rname(b)}`, (id, p) => api.deleteRemoteBranch(repo!, b.remote, b.name, id, p)).then(
      () => say(`Deleted ${rname(b)} on the remote`),
      (e) => notify({ level: 'error', title: `Delete of ${rname(b)} failed`, body: String(e) }),
    )
  }

  function menu(it: Branch): [string, () => void][] {
    const full = it.kind === 'local' ? it.b.name : rname(it.b)
    const out: [string, () => void][] = []
    if (!(it.kind === 'local' && it.b.current)) out.push(['Checkout', () => doCheckout(it)])
    out.push([`New Branch from '${full}'…`, () => go({ kind: 'new', start: full, label: full })])
    if (it.kind === 'remote') {
      const b = it.b
      out.push(['Delete on Remote…', () => go({ kind: 'deleteRemote', b })])
      return out
    }
    const b = it.b
    out.push(['Set Tracking Branch…', () => go({ kind: 'track', branch: b.name })])
    if (b.upstream) out.push(['Unset Tracking Branch', () => setTracking(b.name, null)])
    if (!b.current) out.push(['Delete', () => del(b.name)])
    return out
  }

  const step = (n: number, len: number, e: KeyboardEvent) =>
    Math.max(0, Math.min(len - 1, n + (e.key === 'ArrowDown' ? 1 : -1)))

  function onkeydown(e: KeyboardEvent) {
    const vertical = e.key === 'ArrowDown' || e.key === 'ArrowUp'
    if (e.key === 'Escape') {
      if (sub.kind === 'list' || intent === 'new') close()
      else go({ kind: 'list' })
    } else if (sub.kind === 'list') {
      const it = items[sel]
      if (vertical) move(e.key === 'ArrowDown' ? 1 : -1)
      else if (e.key === 'Enter' && !data) enterPending = true
      else if (e.key === 'Enter' && it) choose(it)
      else if (e.key === 'ArrowRight' && (it?.kind === 'local' || it?.kind === 'remote')) go({ kind: 'menu', item: it })
      else return
    } else if (sub.kind === 'menu') {
      const m = menu(sub.item)
      if (vertical) menuSel = step(menuSel, m.length, e)
      else if (e.key === 'Enter') m[menuSel]?.[1]()
      else if (e.key === 'ArrowLeft') go({ kind: 'list' })
      else return
    } else if (sub.kind === 'track') {
      if (vertical) menuSel = step(menuSel, tracking.length, e)
      else if (e.key === 'Enter' && tracking[menuSel]) setTracking(sub.branch, rname(tracking[menuSel].item))
      else return
    } else if (sub.kind === 'new' && e.key === 'Enter') create(sub.start)
    else return
    e.preventDefault()
  }
</script>

{#snippet hl(text: string, indices: number[])}
  {#each segments(text, indices) as s, i (i)}<span class:hit={s.hit}>{s.text}</span>{/each}
{/snippet}

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="backdrop" onclick={close}></div>
<div class="popup glass" role="dialog" tabindex="-1" bind:this={popup} {onkeydown}>
  {#if !repo}
    <p class="pad muted">Select a repository first.</p>
  {:else if sub.kind === 'list'}
    <input class="search" placeholder={intent === 'delete' ? 'Delete branch…' : 'Search branches'} bind:this={focus} bind:value={query}
 oninput={() => (enterPending = false)} />
    {#if error}
      <p class="pad err">{error}</p>
    {:else if !data}
      <p class="pad muted">Loading…</p>
    {:else}
      <div class="list" style:height="{Math.min(items.length * 28, 448)}px">
        <VirtualList bind:this={list} {items} height={28}>
          {#snippet row(it, i)}
            {#if it.kind === 'header'}
              <div class="header">{it.label}</div>
            {:else}
              <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
              <div class="item" class:selected={i === sel} onmousemove={(e) => (e.movementX || e.movementY) && (sel = i)} onclick={() => choose(it)}>
                {#if it.kind === 'new'}
                  <Icon name="plus" size={14} /><span class="label">New Branch…</span>
                {:else if it.kind === 'prune'}
                  <Icon name="minus" size={14} /><span class="label">Prune Remote Branches</span>
                {:else if it.kind === 'local'}
                  <span class:current={it.b.current}><Icon name="branch" size={14} /></span>
                  <span class="label ellipsis" class:current={it.b.current}>{@render hl(it.b.name, it.indices)}</span>
                  {#if it.b.upstreamGone}<span class="gone" title="Upstream branch is gone">gone</span>{/if}
                  {#if it.b.ahead}<span>↑{it.b.ahead}</span>{/if}
                  {#if it.b.behind}<span>↓{it.b.behind}</span>{/if}
                  {#if it.b.upstream}<span class="muted ellipsis up">{it.b.upstream}</span>{/if}
                {:else}
                  <Icon name="branch" size={14} />
                  <span class="label ellipsis">{@render hl(rname(it.b), it.indices)}</span>
                {/if}
                {#if it.kind === 'local' || it.kind === 'remote'}
                  <button class="icon-btn more" title="Actions" onclick={(e) => {
                    e.stopPropagation()
                    go({ kind: 'menu', item: it })
                  }}><Icon name="chevron" size={12} /></button>
                {/if}
              </div>
            {/if}
          {/snippet}
        </VirtualList>
      </div>
      {#if !items.length}<p class="pad muted">No branches match.</p>{/if}
    {/if}
  {:else if sub.kind === 'menu'}
    {@const it = sub.item}
    <div class="title"><button class="icon-btn" onclick={() => go({ kind: 'list' })}>‹</button>
      {it.kind === 'local' ? it.b.name : rname(it.b)}
      <span class="muted mono">{it.b.lastCommit.sha.slice(0, 8)} {it.b.lastCommit.subject}</span>
    </div>
    {#each menu(it) as [label, run], i (label)}
      <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
      <div class="item" class:selected={i === menuSel} onmousemove={(e) => (e.movementX || e.movementY) && (menuSel = i)} onclick={run}><span class="label ellipsis" title={label}>{label}</span></div>
    {/each}
  {:else if sub.kind === 'new'}
    {@const start = sub.start}
    <div class="title">New branch from {sub.label}</div>
    <div class="pad form">
      <input placeholder="Branch name" bind:this={focus} bind:value={text} disabled={busy} />
      <label><input type="checkbox" bind:checked={andCheckout} /> Checkout branch</label>
      <button class="btn primary" disabled={busy || !text.trim()} onclick={() => create(start)}>Create</button>
    </div>
  {:else if sub.kind === 'track'}
    {@const branch = sub.branch}
    <div class="title">Tracking branch for {branch}</div>
    <input class="search" placeholder="Remote branch" bind:this={focus} bind:value={text} oninput={() => (menuSel = 0)} />
    <div class="scroll">
      {#each tracking as m, i (rname(m.item))}
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
        <div class="item" class:selected={i === menuSel} onmousemove={(e) => (e.movementX || e.movementY) && (menuSel = i)} onclick={() => setTracking(branch, rname(m.item))}>
          <Icon name="branch" size={14} /><span class="label ellipsis">{@render hl(rname(m.item), m.indices)}</span>
        </div>
      {/each}
    </div>
  {:else if sub.kind === 'notMerged'}
    {@const s = sub}
    <div class="title">{s.name} is not fully merged</div>
    <div class="pad">
      <p class="muted">These commits are only on {s.name}; deleting it loses them:</p>
      <div class="scroll commits">
        {#each s.commits as c (c.sha)}
          <div class="commit"><span class="mono muted">{c.sha.slice(0, 8)}</span> <span class="ellipsis">{c.subject}</span> <span class="muted">{c.author}</span></div>
        {/each}
      </div>
      <div class="buttons">
        <button class="btn" onclick={() => go({ kind: 'list' })}>Cancel</button>
        <button class="btn danger" disabled={busy} bind:this={focus} onclick={() => del(s.name, true)}>Delete anyway</button>
      </div>
    </div>
  {:else if sub.kind === 'deleteRemote'}
    {@const b = sub.b}
    <div class="title">Delete {b.name} on {b.remote}?</div>
    <div class="pad buttons">
      <button class="btn" onclick={() => go({ kind: 'list' })}>Cancel</button>
      <button class="btn danger" bind:this={focus} onclick={() => deleteRemote(b)}>Delete on remote</button>
    </div>
  {/if}
</div>

<style>
  .backdrop { position: fixed; inset: 0; z-index: 10; }
  .popup { position: absolute; top: 100%; left: 0; z-index: 11; width: 440px; margin-top: 8px; padding: 8px 0; outline: none; }
  .search { width: calc(100% - 16px); height: 36px; margin: 0 8px 6px; font-size: 14px; }
  .list { max-height: 448px; }
  .scroll { max-height: 300px; overflow-y: auto; }
  .header { display: flex; align-items: flex-end; height: 100%; padding: 0 18px 5px; font-size: 11px; font-weight: 700; letter-spacing: 0.06em; text-transform: uppercase; color: var(--muted); }
  .item { display: flex; align-items: center; gap: 8px; height: 28px; margin: 0 8px; padding: 0 8px 0 10px; border-radius: 9px; }
  .item.selected { background: var(--selected); box-shadow: inset 0 0 0 1px var(--selected-ring); }
  .label { flex: 1; min-width: 0; }
  .current { color: var(--accent); font-weight: 800; }
  .gone { font-size: 10px; font-weight: 700; padding: 0 5px; border-radius: 4px; border: 1px solid var(--warning); color: var(--warning); }
  .up { max-width: 140px; font-size: 12px; }
  .more { height: 20px; min-width: 20px; visibility: hidden; }
  .item.selected .more { visibility: visible; }
  .title { display: flex; align-items: center; gap: 6px; padding: 2px 16px 8px; font-weight: 800; min-width: 0; }
  .title .muted { font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pad { padding: 6px 16px; margin: 0; }
  .form { display: flex; flex-direction: column; gap: 10px; align-items: flex-start; }
  .form input:not([type]) { width: 100%; height: 34px; }
  .buttons { display: flex; justify-content: flex-end; gap: 8px; margin-top: 10px; }
  .commits { border: 1px solid var(--line); border-radius: 10px; padding: 6px 8px; }
  .commit { display: flex; gap: 6px; font-size: 12px; }
  .err { color: var(--error); }
</style>
