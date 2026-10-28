<script lang="ts">
  import { api, type CompareBase, type FileChange } from '../lib/api'
  import { refresh, statuses } from '../lib/app.svelte'
  import { baseName, dirName, groupChanges, statusOf } from '../lib/files'
  import { open } from '../lib/nav'
  import { compare, showDiff, view } from '../lib/view.svelte'
  import Icon from './Icon.svelte'
  import VirtualList from './VirtualList.svelte'

  let { repo }: { repo: string | null } = $props()

  type Row = { title: string; count: number; c?: undefined } | { c: FileChange }

  let collapsed = $state<Record<string, boolean>>({})
  let list = $state<{ reveal(i: number): void }>()
  const status = $derived(repo ? statuses.get(repo) : undefined)
  const base = $derived(repo ? compare.get(repo) : undefined)
  let cmp = $state<{ repo: string; info: CompareBase; changes: FileChange[] } | { repo: string; error: string } | null>(null)
  let branches = $state<string[]>([])

  // Reloads when the repo's status refreshes (after operations and on focus) or the base changes.
  $effect(() => {
    if (!repo || base === undefined) return
    void status
    const r = repo
    api.compareBase(r, base || null).then(
      async (info) => (cmp = { repo: r, info, changes: await api.listChanges(r, { kind: 'rev', rev: info.mergeBase }, { kind: 'worktree' }) }),
      (e) => (cmp = { repo: r, error: String(e) }),
    )
    api.listBranches(r).then((b) => (branches = [...b.local.map((l) => l.name), ...b.remote.map((x) => `${x.remote}/${x.name}`)]), () => {})
  })
  const cur = $derived(cmp?.repo === repo ? cmp : null)
  const info = $derived(cur && 'info' in cur ? cur.info : null)
  const groups = $derived(
    base === undefined
      ? groupChanges(typeof status === 'object' ? status.changes : [])
      : info && cur && 'changes' in cur
        ? [{ title: `Changes since ${info.base}`, items: [...cur.changes].sort((a, b) => a.path.localeCompare(b.path)) }]
        : [],
  )
  const rows = $derived(
    groups.flatMap((g): Row[] => [
      { title: g.title, count: g.items.length },
      ...(collapsed[g.title] ? [] : g.items.map((c) => ({ c }))),
    ]),
  )
  const selected = $derived(view.change?.repo === repo ? view.change?.change.path : null)

  function select(c: FileChange, diff = true) {
    if (!repo) return
    view.change = { repo, change: c }
    if (diff) showDiff(repo, c.path, c.origPath, info && base !== undefined ? { kind: 'rev', rev: info.mergeBase } : { kind: 'head' })
  }

  const openFile = (c: FileChange) => repo && open({ repo, ref: null, path: c.path, line: null })

  function onkeydown(e: KeyboardEvent) {
    const files = rows.flatMap((r) => (r.c ? [r.c] : []))
    const i = files.findIndex((c) => c.path === selected)
    if (e.key === 'ArrowDown' && e.metaKey) {
      if (files[i]) openFile(files[i])
    } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      const next = files[Math.min(files.length - 1, Math.max(0, i + (e.key === 'ArrowDown' ? 1 : -1)))]
      if (next) {
        select(next)
        list?.reveal(rows.findIndex((r) => r.c === next))
      }
    } else if (e.key === 'Enter' && files[i]) select(files[i])
    else return
    e.preventDefault()
  }
</script>

<section data-changes>
  <header>
    <span class="title">Changes</span>
    {#if repo}
      <div class="seg">
        <button class:on={base === undefined} onclick={() => compare.delete(repo)}>Local</button>
        <button class:on={base !== undefined} title="Changes since this branch left a base branch, like a merge request"
          onclick={() => compare.set(repo, compare.get(repo) ?? '')}>Compare</button>
      </div>
    {/if}
    <button class="icon-btn" title="Refresh" onclick={() => repo && refresh(repo)}><Icon name="refresh" /></button>
  </header>
  {#if repo && base !== undefined}
    <div class="base">
      <span class="muted">with</span>
      <input list="compare-bases" placeholder={info?.base ?? 'default branch'} value={base}
        onchange={(e) => compare.set(repo, e.currentTarget.value.trim())} />
      <datalist id="compare-bases">{#each branches as b (b)}<option value={b}></option>{/each}</datalist>
      {#if info}<span class="muted ab" title="Commits on this branch / on {info.base} since they split">↑{info.ahead} ↓{info.behind}</span>{/if}
    </div>
  {/if}
  {#if !repo}
    <div class="empty muted">No repository</div>
  {:else if typeof status === 'string'}
    <div class="empty error">{status}</div>
  {:else if base !== undefined && cur && 'error' in cur}
    <div class="empty error">{cur.error}</div>
  {:else if !rows.length}
    <div class="empty muted">{base === undefined ? 'No local changes' : info ? `No changes since ${info.base}` : 'Comparing…'}</div>
  {:else}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <div class="rows" tabindex="0" role="listbox" {onkeydown}>
      <VirtualList bind:this={list} items={rows} height={24}>
        {#snippet row(r)}
          {#if r.c}
            {@const c = r.c}
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <div class="item {statusOf(c)}" class:selected={c.path === selected} role="option" aria-selected={c.path === selected}
              tabindex="-1" onclick={() => select(c)} ondblclick={() => openFile(c)}
              title={c.origPath ? `${c.origPath} → ${c.path}` : c.path}>
              <span class="name">{baseName(c.path)}</span>
              <span class="muted ellipsis">{dirName(c.path)}</span>
            </div>
          {:else}
            <button class="group" onclick={() => (collapsed[r.title] = !collapsed[r.title])}>
              <span class="chev" class:open={!collapsed[r.title]}><Icon name="chevron" size={12} /></span>
              {r.title} <span class="muted">{r.count} {r.count === 1 ? 'file' : 'files'}</span>
            </button>
          {/if}
        {/snippet}
      </VirtualList>
    </div>
  {/if}
</section>

<style>
  section { display: flex; flex-direction: column; height: 100%; }
  header { display: flex; align-items: center; height: 32px; padding: 0 6px 0 10px; flex: none; }
  .title { flex: 1; font-weight: 600; }
  .seg { display: flex; border: 1px solid var(--line); border-radius: 4px; overflow: hidden; margin-right: 4px; }
  .seg button { border: 0; background: none; padding: 1px 8px; font-size: 12px; color: var(--muted); }
  .seg button.on { background: var(--selected); color: var(--fg); }
  .base { display: flex; align-items: center; gap: 6px; padding: 0 10px 6px; flex: none; font-size: 12px; }
  .base input { flex: 1; min-width: 0; }
  .ab { white-space: nowrap; }
  .rows { flex: 1; min-height: 0; outline: none; }
  .empty { padding: 12px; }
  .error { color: var(--error); white-space: pre-wrap; }
  .group { display: flex; align-items: center; gap: 4px; width: 100%; height: 24px; padding: 0 8px; border: 0; background: none; font-weight: 600; text-align: left; }
  .chev { display: inline-flex; transition: transform 0.1s; }
  .chev.open { transform: rotate(90deg); }
  .item { display: flex; align-items: center; gap: 6px; height: 24px; padding: 0 8px 0 28px; white-space: nowrap; }
  .item.selected { background: var(--selected-inactive); }
  .rows:focus-within .item.selected { background: var(--selected); }
  .modified, .renamed { color: var(--modified); }
  .added { color: var(--added); }
  .deleted { color: var(--deleted); }
  .untracked { color: var(--untracked); }
  .conflict { color: var(--conflict); }
  .item .muted { min-width: 0; }
</style>
