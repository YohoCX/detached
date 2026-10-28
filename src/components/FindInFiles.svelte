<script module lang="ts">
  // Kept between openings, like JetBrains' Find in Files.
  const opts = $state({ text: '', caseSensitive: false, wholeWord: false, regex: false, glob: '', all: false })
</script>

<script lang="ts">
  import { onDestroy, onMount } from 'svelte'
  import { api, newId, type FindEvent, type FindQuery } from '../lib/api'
  import { app, nameOf } from '../lib/app.svelte'
  import { baseName, dirName, highlight } from '../lib/files'
  import { open } from '../lib/nav'
  import { refOf, view } from '../lib/view.svelte'
  import VirtualList from './VirtualList.svelte'

  type Match = Extract<FindEvent, { kind: 'match' }>
  type Row = { file: { repo: string; path: string; count: number }; m?: undefined } | { m: Match; file?: undefined }

  let rows = $state.raw<Row[]>([])
  let sel = $state(-1)
  let running = $state(false)
  let truncated = $state(false)
  let error = $state<string | null>(null)
  let count = $state(0)
  let files = $state(0)
  let input: HTMLInputElement
  let list = $state<{ reveal(i: number): void }>()
  let current = 0
  let searchedRev: string | null = null
  let pending: Match[] = []
  let frame = 0
  const groups = new Map<string, Match[]>()

  const repos = $derived(opts.all ? app.repos.map((r) => r.path) : app.current ? [app.current] : [])
  const rev = $derived(opts.all ? null : refOf(app.current))
  const scope = $derived(opts.all ? 'All repositories' : app.current ? nameOf(app.current) + (rev ? ` @ ${rev}` : '') : 'No repository')

  $effect(() => {
    const q: FindQuery = {
      text: opts.text,
      caseSensitive: opts.caseSensitive,
      wholeWord: opts.wholeWord,
      regex: opts.regex,
      glob: opts.glob.trim() || null,
    }
    const [rs, rv] = [repos, rev]
    const t = setTimeout(() => start(q, rs, rv), 150)
    return () => clearTimeout(t)
  })

  function start(q: FindQuery, rs: string[], rv: string | null) {
    if (current) api.cancelFind(current)
    current = 0
    groups.clear()
    pending = []
    rows = []
    sel = -1
    truncated = false
    error = null
    count = files = 0
    running = !!q.text && !!rs.length
    if (!running) return
    const id = (current = newId())
    searchedRev = rv
    api.findInFiles(rs, rv, q, id, (e) => {
      if (e.queryId !== current) return
      if (e.kind === 'match') {
        pending.push(e)
        frame ||= requestAnimationFrame(flush)
        return
      }
      flush()
      running = false
      if (e.kind === 'done') truncated = e.truncated
      else error = e.error
    }).catch((e) => {
      if (id !== current) return
      running = false
      error = String(e)
    })
  }

  function flush() {
    cancelAnimationFrame(frame)
    frame = 0
    if (!pending.length) return
    for (const m of pending) {
      const k = `${m.repo}\0${m.path}`
      if (!groups.has(k)) groups.set(k, [])
      groups.get(k)!.push(m)
    }
    count += pending.length
    files = groups.size
    pending = []
    rows = [...groups.values()].flatMap((ms): Row[] => [
      { file: { repo: ms[0].repo, path: ms[0].path, count: ms.length } },
      ...ms.map((m) => ({ m })),
    ])
    if (sel < 0) sel = 1
  }

  const close = () => (view.find = false)

  function choose(i: number) {
    const m = rows[i]?.m
    if (!m) return
    close()
    open({ repo: m.repo, ref: searchedRev, path: m.path, line: m.line })
  }

  function move(d: number) {
    let i = sel
    do i += d
    while (rows[i] && !rows[i].m)
    if (!rows[i]) return
    sel = i
    list?.reveal(i)
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') close()
    else if (e.key === 'Enter') choose(sel)
    else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') move(e.key === 'ArrowDown' ? 1 : -1)
    else return
    e.preventDefault()
  }

  const toggle = (k: 'caseSensitive' | 'wholeWord' | 'regex') => (opts[k] = !opts[k])

  onMount(() => input.select())
  onDestroy(() => {
    if (current) api.cancelFind(current)
    current = 0
    cancelAnimationFrame(frame)
  })
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="backdrop" onclick={close}></div>
<div class="find" role="dialog">
  <div class="head">
    <span class="title">Find in Files</span>
    <span class="muted">
      {#if running}Searching… {count} {:else if opts.text}{count} {count === 1 ? 'match' : 'matches'} in {files}
        {files === 1 ? 'file' : 'files'}{/if}
    </span>
    <span class="spacer"></span>
    <label class="muted"><input type="checkbox" bind:checked={opts.all} /> All repositories</label>
  </div>
  <div class="line">
    <input class="text" bind:this={input} bind:value={opts.text} {onkeydown} placeholder="Text to find" />
    {#each [['caseSensitive', 'Cc', 'Match case'], ['wholeWord', 'W', 'Words'], ['regex', '.*', 'Regex']] as const as [k, label, title] (k)}
      <button class="toggle mono" class:on={opts[k]} {title} onmousedown={(e) => e.preventDefault()} onclick={() => toggle(k)}>{label}</button>
    {/each}
  </div>
  <div class="line">
    <input class="glob" bind:value={opts.glob} {onkeydown} placeholder="File mask: *.ts, !*.test.ts" />
    <span class="muted ellipsis scope">{scope}</span>
  </div>
  <div class="list">
    <VirtualList bind:this={list} items={rows} height={24}>
      {#snippet row(r, i)}
        {#if r.file}
          <div class="file" title={r.file.path}>
            <span class="name">{baseName(r.file.path)}</span>
            <span class="muted ellipsis">{dirName(r.file.path)}</span>
            <span class="muted">{opts.all ? nameOf(r.file.repo) : ''} {r.file.count}</span>
          </div>
        {:else}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <div class="match mono" class:selected={i === sel} role="option" aria-selected={i === sel} tabindex="-1"
            onmousemove={() => (sel = i)} onclick={() => choose(i)}>
            <span class="no muted">{r.m.line}</span>
            <span class="ellipsis">{#each highlight(r.m.text, r.m.ranges) as s, k (k)}<span class:hit={s.hit}>{s.text}</span>{/each}</span>
          </div>
        {/if}
      {/snippet}
    </VirtualList>
  </div>
  {#if truncated}<div class="note">Results truncated at 10,000 matches. Narrow the search.</div>{/if}
  {#if error}<div class="note error">{error}</div>{/if}
</div>

<style>
  .backdrop { position: fixed; inset: 0; z-index: 20; background: rgb(0 0 0 / 0.08); }
  .find {
    position: fixed; top: 10%; left: 50%; transform: translateX(-50%); z-index: 21; width: min(820px, 92vw); height: min(560px, 80vh);
    background: var(--popup); border: 1px solid var(--line); border-radius: 10px; box-shadow: var(--shadow);
    display: flex; flex-direction: column; overflow: hidden;
  }
  .head { display: flex; align-items: center; gap: 12px; padding: 8px 12px 4px; }
  .title { font-weight: 600; }
  .spacer { flex: 1; }
  .head label { display: flex; align-items: center; gap: 4px; font-size: 12px; }
  .line { display: flex; align-items: center; gap: 4px; padding: 3px 10px; }
  .text { flex: 1; font-size: 14px; padding: 5px 8px; }
  .glob { width: 260px; font-size: 12px; }
  .scope { flex: 1; text-align: right; font-size: 12px; }
  .toggle { width: 30px; height: 26px; border: 1px solid transparent; border-radius: 4px; background: none; }
  .toggle:hover { background: var(--hover); }
  .toggle.on { background: var(--selected); border-color: var(--accent); }
  .list { flex: 1; min-height: 0; margin-top: 4px; border-top: 1px solid var(--line); }
  .file { display: flex; align-items: center; gap: 6px; height: 24px; padding: 0 10px; white-space: nowrap; }
  .file .name { font-weight: 600; }
  .file .ellipsis { flex: 1; min-width: 0; }
  .match { display: flex; align-items: center; gap: 8px; height: 24px; padding: 0 10px 0 22px; white-space: pre; }
  .match.selected { background: var(--selected); }
  .no { width: 40px; flex: none; text-align: right; }
  .note { padding: 6px 12px; border-top: 1px solid var(--line); font-size: 12px; color: var(--warning); }
  .note.error { color: var(--error); }
</style>
