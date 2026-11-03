<script module lang="ts">
  import { SvelteSet } from 'svelte/reactivity'

  const expandedFor = new Map<string, SvelteSet<string>>()
</script>

<script lang="ts">
  import { api, type FileEntry } from '../lib/api'
  import { app, branchNames, statuses } from '../lib/app.svelte'
  import { buildTree, dirName, flatten, nestingDefaults, statusOf, type Row } from '../lib/files'
  import { open } from '../lib/nav'
  import { load, save } from '../lib/storage'
  import { refOf, refs } from '../lib/view.svelte'
  import Icon from './Icon.svelte'
  import VirtualList from './VirtualList.svelte'

  let { repo }: { repo: string | null } = $props()

  let entries = $state.raw<FileEntry[]>([])
  let error = $state<string | null>(null)
  let sel = $state<string | null>(null)
  let refInput = $state('')
  let branches = $state.raw<string[]>([])
  let list = $state<{ reveal(i: number): void }>()
  const ref = $derived(refOf(repo))
  const expanded = $derived.by(() => {
    const key = repo ?? ''
    if (!expandedFor.has(key)) expandedFor.set(key, new SvelteSet())
    return expandedFor.get(key)!
  })
  let nest = $state(load('tree.nesting') !== 'false')
  const rows = $derived(flatten(buildTree(entries), expanded, nest ? { ...nestingDefaults, ...app.fileNesting } : {}))

  function toggleNesting() {
    nest = !nest
    save('tree.nesting', String(nest))
  }

  function toggle(path: string) {
    if (!expanded.delete(path)) expanded.add(path)
  }

  let seq = 0
  $effect(() => {
    const [r, rev] = [repo, ref]
    if (!r) return
    // The working tree reloads with each status refresh.
    if (!rev) void statuses.get(r)
    const id = ++seq
    api.listFiles(r, rev).then(
      (e) => {
        if (id !== seq) return
        entries = e
        error = null
      },
      (e) => {
        if (id !== seq) return
        entries = []
        error = String(e)
      },
    )
  })

  $effect(() => {
    refInput = ref ?? ''
  })

  function setRef(v: string) {
    if (!repo) return
    if (v.trim()) refs.set(repo, v.trim())
    else refs.delete(repo)
  }

  const loadBranches = () => repo && branchNames(repo).then((b) => (branches = b), () => (branches = []))

  function activate(r: Row) {
    sel = r.path
    if (!r.dir) return repo && open({ repo, ref, path: r.path, line: null })
    toggle(r.path)
  }

  function onkeydown(e: KeyboardEvent) {
    const i = rows.findIndex((r) => r.path === sel)
    const r = rows[i]
    const go = (j: number) => {
      const next = rows[Math.min(rows.length - 1, Math.max(0, j))]
      if (next) sel = next.path
      list?.reveal(Math.max(0, Math.min(rows.length - 1, j)))
    }
    if (e.key === 'ArrowDown') go(i + 1)
    else if (e.key === 'ArrowUp') go(i - 1)
    else if (e.key === 'ArrowRight' && (r?.dir || r?.kids)) expanded.add(r.path)
    else if (e.key === 'ArrowLeft' && r) {
      if ((r.dir || r.kids) && expanded.has(r.path)) expanded.delete(r.path)
      else if (r.parent || dirName(r.path)) go(rows.findIndex((x) => x.path === (r.parent ?? dirName(r.path))))
    } else if (e.key === 'Enter' && r) activate(r)
    else return
    e.preventDefault()
  }
</script>

<section>
  <header>
    <span class="title">Project</span>
    <input class="ref" class:at={ref} list="tree-refs" placeholder="Working tree" title="Browse a branch, tag or commit"
      bind:value={refInput} onfocus={loadBranches} onchange={() => setRef(refInput)}
      onkeydown={(e) => e.key === 'Enter' && setRef(refInput)} />
    <datalist id="tree-refs">
      {#each branches as b (b)}<option value={b}></option>{/each}
    </datalist>
    {#if ref}<button class="icon-btn" title="Back to the working tree" onclick={() => setRef('')}><Icon name="close" size={12} /></button>{/if}
    <button class="icon-btn" class:on={nest} title="Nest related files" aria-pressed={nest} onclick={toggleNesting}><Icon name="tree" size={14} /></button>
  </header>
  {#if !repo}
    <div class="empty muted">No repository</div>
  {:else if error}
    <div class="empty error">{error}</div>
  {:else}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <div class="rows" tabindex="0" role="tree" {onkeydown}>
      <VirtualList bind:this={list} items={rows} height={24}>
        {#snippet row(r)}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <div class="item {r.change ? statusOf(r.change) : r.dirty ? 'modified' : ''}" class:selected={r.path === sel}
            role="treeitem" aria-selected={r.path === sel} tabindex="-1" style:padding-left="{8 + r.depth * 16}px"
            onclick={() => activate(r)} title={r.path}>
            {#if r.dir}
              <span class="chev" class:open={expanded.has(r.path)}><Icon name="chevron" size={12} /></span>
              <Icon name="folder" size={14} />
            {:else if r.kids}
              <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
              <span class="chev" class:open={expanded.has(r.path)} onclick={(e) => {
                e.stopPropagation()
                toggle(r.path)
              }}><Icon name="chevron" size={12} /></span>
            {:else}
              <span class="chev"></span>
            {/if}
            <span class="ellipsis">{r.name}</span>
          </div>
        {/snippet}
      </VirtualList>
    </div>
  {/if}
</section>

<style>
  section { display: flex; flex-direction: column; height: 100%; padding-bottom: 6px; }
  header { display: flex; align-items: center; gap: 6px; height: 44px; padding: 0 8px 0 16px; flex: none; }
  .title { flex: 1; font-size: 14px; font-weight: 800; }
  .ref { width: 140px; height: 26px; font-size: 12px; }
  .ref.at { border-color: var(--selected-ring); }
  .rows { flex: 1; min-height: 0; outline: none; }
  .empty { padding: 12px; }
  .error { color: var(--error); white-space: pre-wrap; }
  .item { display: flex; align-items: center; gap: 4px; height: 24px; margin: 0 6px; padding-right: 8px; border-radius: 7px; white-space: nowrap; }
  .item:hover { background: var(--hover); }
  .item.selected { background: var(--selected-inactive); }
  .rows:focus-within .item.selected { background: var(--selected); box-shadow: inset 0 0 0 1px var(--selected-ring); }
  .chev { display: inline-flex; width: 12px; flex: none; color: var(--muted); transition: transform 0.1s; }
  .chev.open { transform: rotate(90deg); }
  .on { color: var(--accent); }
  .modified, .renamed { color: var(--modified); }
  .added { color: var(--added); }
  .deleted { color: var(--deleted); }
  .untracked { color: var(--untracked); }
  .conflict { color: var(--conflict); }
</style>
