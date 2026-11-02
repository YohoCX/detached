<script lang="ts">
  import { untrack } from 'svelte'
  import { api, EMPTY_TREE, type CommitDetails, type FileChange, type Side } from '../lib/api'
  import { baseName, dirName, letter, statusOf } from '../lib/files'
  import { when } from '../lib/log'
  import { view } from '../lib/view.svelte'
  import VirtualList from './VirtualList.svelte'

  // Several `shas` (newest first) show the changes from the oldest's first parent to the newest;
  // `worktree` compares the one commit with the working tree.
  let { repo, shas, worktree = false }: { repo: string; shas: string[]; worktree?: boolean } = $props()

  const rev = (r: string): Side => ({ kind: 'rev', rev: r })
  const short = (sha: string) => sha.slice(0, 10)
  let data = $state<{ d: CommitDetails; left: Side; right: Side; changes: FileChange[] } | null>(null)
  let error = $state('')
  let containing = $state<string[] | string | null>(null)
  let selected = $state<string | null>(null)
  let list = $state<{ reveal(i: number): void }>()

  const key = $derived(`${repo}\0${shas.join()}\0${worktree}`)
  $effect(() => {
    void key
    const [r, picked, wt] = untrack(() => [repo, shas, worktree] as const)
    let stale = false
    data = null
    error = ''
    containing = null
    selected = null
    ;(async () => {
      const d = await api.commitDetails(r, picked[picked.length - 1])
      const left = rev(wt ? d.sha : (d.parents[0] ?? EMPTY_TREE))
      const right: Side = wt ? { kind: 'worktree' } : rev(picked[0])
      const changes = (await api.listChanges(r, left, right)).sort((a, b) => a.path.localeCompare(b.path))
      if (!stale) data = { d, left, right, changes }
    })().catch((e) => !stale && (error = String(e)))
    return () => (stale = true)
  })

  function loadContaining() {
    const k = key
    containing = 'Searching…'
    api.branchesContaining(repo, shas[0]).then(
      (b) => k === key && (containing = b),
      (e) => k === key && (containing = String(e)),
    )
  }

  function select(c: FileChange, focus = false) {
    if (!data) return
    selected = c.path
    view.shown = { kind: 'diff', repo, path: c.path, origPath: c.origPath, left: data.left, right: data.right }
    if (focus) view.focusDiff++
  }

  function onkeydown(e: KeyboardEvent) {
    const files = data?.changes ?? []
    const i = files.findIndex((c) => c.path === selected)
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      const j = Math.min(files.length - 1, Math.max(0, i + (e.key === 'ArrowDown' ? 1 : -1)))
      if (files[j]) select(files[j])
      list?.reveal(j)
    } else if (e.key === 'Enter' && files[i]) select(files[i], true)
    else return
    e.preventDefault()
  }

  const person = (name: string, email: string, time: number) => `${name} <${email}>, ${when(time)}`
</script>

<div class="details">
  {#if error}
    <div class="pad error">{error}</div>
  {:else if !data}
    <div class="pad muted">Loading…</div>
  {:else}
    {@const d = data.d}
    <div class="info">
      {#if shas.length > 1}
        <div class="subject">{shas.length} commits</div>
        <div class="mono muted">{short(d.parents[0] ?? EMPTY_TREE)} → {short(shas[0])}</div>
      {:else}
        {@const [subject, ...body] = d.message.trim().split('\n')}
        <div class="subject">{subject}</div>
        {#if body.length}<div class="body">{body.join('\n').trim()}</div>{/if}
        <dl>
          <dt>Commit</dt><dd class="mono">{d.sha}</dd>
          <dt>Author</dt><dd>{person(d.author, d.authorEmail, d.authorTime)}</dd>
          {#if d.committer !== d.author || d.committerEmail !== d.authorEmail || d.committerTime !== d.authorTime}
            <dt>Committer</dt><dd>{person(d.committer, d.committerEmail, d.committerTime)}</dd>
          {/if}
          {#if d.parents.length}<dt>{d.parents.length > 1 ? 'Parents' : 'Parent'}</dt><dd class="mono">{d.parents.map(short).join(' ')}</dd>{/if}
          <dt>Branches</dt>
          <dd>
            {#if containing === null}<button class="link" onclick={loadContaining}>Show branches containing this commit</button>
            {:else if typeof containing === 'string'}<span class="muted">{containing}</span>
            {:else}{containing.join(', ') || 'None'}{/if}
          </dd>
        </dl>
      {/if}
    </div>
    <div class="files-head muted">
      {data.changes.length} {data.changes.length === 1 ? 'file' : 'files'}{worktree ? ' changed since, in the working tree' : ''}
    </div>
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <div class="files" tabindex="0" role="listbox" {onkeydown}>
      <VirtualList bind:this={list} items={data.changes} height={28}>
        {#snippet row(c)}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <div class="item {statusOf(c)}" class:selected={c.path === selected} role="option" aria-selected={c.path === selected}
            tabindex="-1" onclick={() => select(c)} ondblclick={() => select(c, true)}
            title={c.origPath ? `${c.origPath} → ${c.path}` : c.path}>
            <span class="st">{letter[statusOf(c)]}</span>
            <span class="name">{baseName(c.path)}</span>
            <span class="muted ellipsis">{dirName(c.path)}</span>
          </div>
        {/snippet}
      </VirtualList>
    </div>
  {/if}
</div>

<style>
  .details { display: flex; flex-direction: column; height: 100%; min-width: 0; }
  .pad { padding: 12px 16px; }
  .error { color: var(--error); white-space: pre-wrap; }
  .info { flex: none; max-height: 50%; overflow-y: auto; padding: 10px 16px; border-bottom: 1px solid var(--line); -webkit-user-select: text; user-select: text; cursor: auto; }
  .subject { font-weight: 800; font-size: 14px; overflow-wrap: anywhere; }
  .body { margin-top: 6px; white-space: pre-wrap; overflow-wrap: anywhere; }
  dl { display: grid; grid-template-columns: auto 1fr; gap: 3px 12px; margin: 10px 0 0; font-size: 12px; }
  dt { color: var(--muted); }
  dd { margin: 0; min-width: 0; overflow-wrap: anywhere; }
  .link { padding: 0; border: 0; background: none; color: var(--accent); font-weight: 600; }
  .files-head { flex: none; padding: 6px 16px; font-size: 11px; font-weight: 700; letter-spacing: 0.06em; text-transform: uppercase; }
  .files { flex: 1; min-height: 0; outline: none; padding-bottom: 6px; }
  .item { display: flex; align-items: center; gap: 8px; height: 28px; margin: 0 6px; padding: 0 8px; border-radius: 9px; white-space: nowrap; }
  .item:hover { background: var(--hover); }
  .item.selected { background: var(--selected-inactive); }
  .files:focus-within .item.selected { background: var(--selected); box-shadow: inset 0 0 0 1px var(--selected-ring); }
  .item .muted { min-width: 0; }
  .name { font-weight: 600; }
  .deleted .name { text-decoration: line-through; color: var(--muted); }
</style>
