<script lang="ts">
  import type { FileChange } from '../lib/api'
  import { refresh, statuses } from '../lib/app.svelte'
  import { baseName, dirName, groupChanges, statusOf } from '../lib/files'
  import { open } from '../lib/nav'
  import { showDiff, view } from '../lib/view.svelte'
  import Icon from './Icon.svelte'
  import VirtualList from './VirtualList.svelte'

  let { repo }: { repo: string | null } = $props()

  type Row = { title: string; count: number; c?: undefined } | { c: FileChange }

  let collapsed = $state<Record<string, boolean>>({})
  let list = $state<{ reveal(i: number): void }>()
  const status = $derived(repo ? statuses.get(repo) : undefined)
  const changes = $derived(typeof status === 'object' ? status.changes : [])
  const rows = $derived(
    groupChanges(changes).flatMap((g): Row[] => [
      { title: g.title, count: g.items.length },
      ...(collapsed[g.title] ? [] : g.items.map((c) => ({ c }))),
    ]),
  )
  const selected = $derived(view.change?.repo === repo ? view.change?.change.path : null)

  function select(c: FileChange, diff = true) {
    if (!repo) return
    view.change = { repo, change: c }
    if (diff) showDiff(repo, c.path, c.origPath)
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
    <button class="icon-btn" title="Refresh" onclick={() => repo && refresh(repo)}><Icon name="refresh" /></button>
  </header>
  {#if !repo}
    <div class="empty muted">No repository</div>
  {:else if typeof status === 'string'}
    <div class="empty error">{status}</div>
  {:else if !rows.length}
    <div class="empty muted">No local changes</div>
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
