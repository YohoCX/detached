<script lang="ts">
  import { actions, type Action } from '../lib/actions'
  import { api, type Branches, type FileMatch } from '../lib/api'
  import { app, checkout, nameOf, run, type PaletteMode } from '../lib/app.svelte'
  import { filter, segments } from '../lib/fuzzy'
  import { chordFor } from '../lib/keymap'
  import { nav, open, type FileRef } from '../lib/nav'
  import { refOf } from '../lib/view.svelte'
  import Icon from './Icon.svelte'

  type Entry =
    | { kind: 'file'; f: FileRef; indices: number[] }
    | { kind: 'branch'; name: string; remote: string | null; indices: number[] }
    | { kind: 'action'; a: Action; indices: number[] }

  const titles: Record<PaletteMode, string> = {
    everywhere: 'Search Everywhere',
    file: 'Go to File',
    action: 'Find Action',
    vcs: 'VCS Operations',
    recent: 'Recent Files',
  }

  let query = $state('')
  let sel = $state(0)
  let files = $state<Entry[]>([])
  let branches = $state<Branches | null>(null)
  let input: HTMLInputElement
  let listEl: HTMLDivElement
  const mode = $derived(app.palette!)
  const close = () => (app.palette = null)
  const toEntry = (ref: string | null) => (m: FileMatch): Entry =>
    ({ kind: 'file', f: { repo: m.repo, ref, path: m.path }, indices: m.indices })

  let seq = 0
  $effect(() => {
    const q = query
    const fileMode = mode === 'file' || mode === 'everywhere'
    const repos = app.paletteAll ? app.repos.map((r) => r.path) : app.current ? [app.current] : []
    const id = ++seq
    if (!fileMode || !q || !repos.length) {
      files = []
      return
    }
    // One repo browses its tree's ref; across repos, the working trees.
    const ref = app.paletteAll ? null : refOf(app.current)
    api.quickOpen(repos, ref, q, mode === 'file' ? 50 : 10).then(
      (r) => {
        if (id === seq) files = r.map(toEntry(ref))
      },
      () => {
        if (id === seq) files = []
      },
    )
  })

  $effect(() => {
    if (mode === 'everywhere' && !branches && app.current) api.listBranches(app.current).then((b) => (branches = b), () => {})
  })

  const actionEntries = (group?: string) =>
    filter([...actions.values()].filter((a) => !group || a.group === group), query, (a) => a.title)
      .map((m): Entry => ({ kind: 'action', a: m.item, indices: m.indices }))

  const recentEntries = () =>
    filter(nav.recent, query, (f) => f.path).map((m): Entry => ({ kind: 'file', f: m.item, indices: m.indices }))

  const entries = $derived.by((): Entry[] => {
    switch (mode) {
      case 'file': return query ? files : recentEntries()
      case 'recent': return recentEntries()
      case 'action': return actionEntries()
      case 'vcs': return actionEntries('vcs')
      case 'everywhere': {
        if (!query) return []
        const names = [
          ...(branches?.local.map((b) => ({ name: b.name, remote: null as string | null, label: b.name })) ?? []),
          ...(branches?.remote.map((b) => ({ name: b.name, remote: b.remote, label: `${b.remote}/${b.name}` })) ?? []),
        ]
        const bs = filter(names, query, (b) => b.label).slice(0, 10)
          .map((m): Entry => ({ kind: 'branch', name: m.item.name, remote: m.item.remote, indices: m.indices }))
        return [...files, ...bs, ...actionEntries().slice(0, 10)]
      }
    }
  })

  $effect(() => {
    void mode
    sel = 0
    input.focus()
  })

  function choose(e: Entry | undefined) {
    if (!e) return
    close()
    if (e.kind === 'file') open({ ...e.f, line: null })
    else if (e.kind === 'action') run(e.a.id)
    else if (app.current)
      checkout(app.current, e.remote ? { kind: 'remote', remote: e.remote, name: e.name } : { kind: 'local', name: e.name })
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') close()
    else if (e.key === 'Enter') choose(entries[sel])
    else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      sel = (sel + (e.key === 'ArrowDown' ? 1 : -1) + entries.length) % Math.max(1, entries.length)
      listEl.children[sel]?.scrollIntoView({ block: 'nearest' })
    } else return
    e.preventDefault()
  }

  /** File name and directory, each with its share of the match indices. */
  function split(path: string, indices: number[]) {
    const chars = Array.from(path)
    const k = chars.lastIndexOf('/') + 1
    return {
      name: segments(chars.slice(k).join(''), indices.filter((i) => i >= k).map((i) => i - k)),
      dir: segments(chars.slice(0, k).join(''), indices.filter((i) => i < k)),
    }
  }
</script>

{#snippet hl(segs: { text: string; hit: boolean }[])}
  {#each segs as s, i (i)}<span class:hit={s.hit}>{s.text}</span>{/each}
{/snippet}

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="backdrop" onclick={close}></div>
<div class="palette" role="dialog">
  <div class="head">
    <span class="mode">{titles[mode]}</span>
    {#if mode === 'file' || mode === 'everywhere'}
      <label class="muted"><input type="checkbox" bind:checked={app.paletteAll} /> All repositories</label>
    {/if}
  </div>
  <input class="query" bind:this={input} bind:value={query} {onkeydown} oninput={() => (sel = 0)}
    placeholder={mode === 'file' && !app.paletteAll ? `Files in ${app.current ? nameOf(app.current) : 'no repository'}` : 'Type to search'} />
  <div class="list" bind:this={listEl}>
    {#each entries as e, i (i)}
      <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
      <div class="item" class:selected={i === sel} onmousemove={(e) => (e.movementX || e.movementY) && (sel = i)} onclick={() => choose(e)}>
        {#if e.kind === 'file'}
          {@const p = split(e.f.path, e.indices)}
          <Icon name="tree" size={14} />
          <span class="name">{@render hl(p.name)}</span>
          <span class="muted ellipsis dir">{@render hl(p.dir)}</span>
          <span class="muted detail">{nameOf(e.f.repo)}{e.f.ref ? ` @ ${e.f.ref}` : ''}</span>
        {:else if e.kind === 'branch'}
          <Icon name="branch" size={14} />
          <span class="name ellipsis">{@render hl(segments(e.remote ? `${e.remote}/${e.name}` : e.name, e.indices))}</span>
          <span class="muted detail">Checkout</span>
        {:else}
          <span class="spacer"></span>
          <span class="name ellipsis" class:muted={!e.a.run}>{@render hl(segments(e.a.title, e.indices))}</span>
          <span class="muted detail mono">{chordFor(e.a.id)}</span>
        {/if}
      </div>
    {:else}
      <div class="empty muted">{query || mode !== 'everywhere' ? 'Nothing found' : 'Search files, branches and actions'}</div>
    {/each}
  </div>
</div>

<style>
  .backdrop { position: fixed; inset: 0; z-index: 20; background: rgb(0 0 0 / 0.08); }
  .palette {
    position: fixed; top: 12%; left: 50%; transform: translateX(-50%); z-index: 21; width: min(680px, 90vw);
    background: var(--popup); border: 1px solid var(--line); border-radius: 10px; box-shadow: var(--shadow);
    display: flex; flex-direction: column; overflow: hidden;
  }
  .head { display: flex; align-items: center; gap: 12px; padding: 8px 12px 4px; }
  .mode { font-weight: 600; flex: 1; }
  .head label { display: flex; align-items: center; gap: 4px; font-size: 12px; }
  .query { margin: 4px 10px 6px; padding: 6px 8px; font-size: 14px; }
  .list { max-height: 420px; overflow-y: auto; padding: 0 6px 6px; }
  .item { display: flex; align-items: center; gap: 6px; height: 26px; padding: 0 8px; border-radius: 4px; }
  .item.selected { background: var(--selected); }
  .name { white-space: nowrap; }
  .dir { flex: 1; min-width: 0; font-size: 12px; }
  .item > .name.ellipsis { flex: 1; min-width: 0; }
  .detail { flex: none; font-size: 12px; }
  .spacer { width: 14px; flex: none; }
  .empty { padding: 12px 8px; }
</style>
