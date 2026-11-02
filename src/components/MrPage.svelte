<script lang="ts">
  import { openUrl } from '@tauri-apps/plugin-opener'
  import { mount, unmount, untrack } from 'svelte'
  import { api, type CommitSummary, type DiffSide, type FileChange, type Hunk, type MergeRequest, type MrAction, type Side, type Thread as T } from '../lib/api'
  import { runOp, say } from '../lib/app.svelte'
  import { baseName, dirName, letter, statusOf } from '../lib/files'
  import { when } from '../lib/log'
  import { ago, bareTitle, isGitLab, tone } from '../lib/mr'
  import type { FileDoc, Zone } from '../lib/view.svelte'
  import CommitDetails from './CommitDetails.svelte'
  import Grip from './Grip.svelte'
  import Icon from './Icon.svelte'
  import Markdown from './Markdown.svelte'
  import MergeWidget from './MergeWidget.svelte'
  import Pipelines from './Pipelines.svelte'
  import Thread from './Thread.svelte'
  import Viewer from './Viewer.svelte'

  let { repo, number }: { repo: string; number: number } = $props()

  type Refs = { base: string; head: string; moved: boolean; commits: CommitSummary[]; files: FileChange[] }
  const tabs = ['Overview', 'Commits', 'Changes', 'Pipelines'] as const
  const rev = (r: string): Side => ({ kind: 'rev', rev: r })

  let tab = $state<(typeof tabs)[number]>('Overview')
  let sideWidth = $state(420)
  let filesWidth = $state(280)
  let mr = $state<MergeRequest | null>(null)
  let threads = $state<T[]>([])
  let error = $state('')
  let failure = $state('')
  let review = $state('')
  let busy = $state(false)
  let refs = $state<Refs | { error: string } | 'loading' | null>(null)
  let commit = $state<string | null>(null)
  let file = $state<FileChange | null>(null)
  let hunks = $state<Hunk[]>([])
  let draft = $state<{ side: DiffSide; line: number; other: number | null } | null>(null)

  const gitlab = $derived(mr ? isGitLab(mr) : false)
  const ready = $derived(refs && typeof refs === 'object' && 'base' in refs ? refs : null)

  async function load() {
    const [r, n] = [repo, number]
    try {
      const [m, t] = await Promise.all([api.getMr(r, n), api.mrThreads(r, n)])
      if (r !== repo || n !== number) return
      if (m.headSha !== mr?.headSha) refs = null
      ;[mr, threads, error] = [m, t, '']
    } catch (e) {
      if (r === repo && n === number) error = String(e)
    }
  }

  $effect(() => {
    void [repo, number]
    untrack(() => {
      ;[mr, threads, error, failure, refs, commit, file, draft, tab] = [null, [], '', '', null, null, null, null, 'Overview']
      load()
    })
  })

  async function act(a: MrAction): Promise<boolean> {
    busy = true
    try {
      await api.mrAction(repo, number, a)
      failure = ''
      return true
    } catch (e) {
      failure = String(e)
      return false
    } finally {
      busy = false
      await load()
    }
  }

  // The diff's commits, fetched when missing; a head that moved past the API's turns line comments off.
  async function resolve(m: MergeRequest) {
    const r = repo
    refs = 'loading'
    try {
      let head = m.headSha
      if (!(await api.mergeBase(r, m.baseSha, head).then(() => true, () => false))) {
        const remote = (await api.forgeHosts(r)).mrRemote
        if (!remote) throw 'No MR remote; choose one in Settings'
        const ref = isGitLab(m) ? `refs/merge-requests/${m.number}/head` : `refs/pull/${m.number}/head`
        head = await runOp(r, `Fetch !${m.number}`, (id, p) => api.fetchMr(r, remote, ref, m.targetBranch, id, p))
        if (head !== m.headSha) mr = m = await api.getMr(r, m.number)
      }
      const base = isGitLab(m) ? m.baseSha : await api.mergeBase(r, m.baseSha, head)
      const [commits, files] = await Promise.all([api.rangeLog(r, base, head), api.listChanges(r, rev(base), rev(head))])
      if (r === repo) refs = { base, head, moved: head !== m.headSha, commits, files: files.sort((a, b) => a.path.localeCompare(b.path)) }
    } catch (e) {
      if (r === repo) refs = { error: String(e) }
    }
  }

  $effect(() => {
    if (mr && !refs && (tab === 'Commits' || tab === 'Changes')) untrack(() => resolve(mr!))
  })

  async function refresh() {
    await load()
    refs = null
  }

  function pick(f: FileChange) {
    if (!ready) return
    file = f
    draft = null
    hunks = []
    if (!gitlab) api.hunkRanges(repo, ready.base, ready.head, f.path, f.origPath).then((h) => file === f && (hunks = h), (e) => say(String(e), true))
  }

  const doc = $derived<FileDoc | undefined>(
    ready && file ? { kind: 'diff', repo, path: file.path, origPath: file.origPath, left: rev(ready.base), right: rev(ready.head) } : undefined,
  )
  const within = (start: number, count: number, line: number) => line >= start && line < start + count
  // GitHub takes line comments only inside the PR diff's hunks.
  const commentable = (side: DiffSide, line: number) =>
    gitlab || hunks.some((h) => (side === 'new' ? within(h.newStart, h.newLines, line) : within(h.oldStart, h.oldLines, line)))

  const onFile = (t: T, f: FileChange) => t.position && !t.position.outdated && [f.path, f.origPath].includes(t.position.path)
  const fileThreads = $derived(file ? threads.filter((t) => onFile(t, file!)) : [])
  let zones = $state<Zone[]>([])
  $effect(() => {
    const m = mr
    if (!m || !file || !ready) return void (zones = [])
    const f = file
    const head = ready.head
    type Item = { side: DiffSide; line: number; thread: T | null; start?: (body: string) => MrAction; oncancel?: () => void }
    const items: Item[] = fileThreads.map((t) => ({ side: t.position!.side, line: t.position!.line, thread: t }))
    if (draft) {
      const { side, line, other } = draft
      // GitLab names both lines of an unchanged line; GitHub takes the clicked side only.
      const [oldLine, newLine] = side === 'new' ? [gitlab ? other : null, line] : [line, gitlab ? other : null]
      const start = (body: string): MrAction => ({ kind: 'lineComment', body, path: f.path, oldPath: f.origPath ?? f.path, oldLine, newLine, headSha: head })
      items.push({ side, line, thread: null, start, oncancel: () => (draft = null) })
    }
    const made = untrack(() =>
      items.map(({ side, line, ...props }) => {
        const node = document.createElement('div')
        return { zone: { side, afterLine: line, node }, c: mount(Thread, { target: node, props: { mr: m, act, zone: true, ...props } }) }
      }),
    )
    zones = made.map((x) => x.zone)
    return () => made.forEach((x) => unmount(x.c))
  })

  const open = (url: string) => openUrl(url).catch((e) => say(String(e), true))
</script>

<div class="page">
  <header>
    <span class="strong">{gitlab ? '!' : '#'}{number}</span>
    {#if mr}
      <span class="title ellipsis" title={mr.title}>{bareTitle(mr)}</span>
      {#if mr.draft}<span class="pill">Draft</span>{/if}
      <span class="pill {mr.state}">{mr.state}</span>
    {/if}
    <span class="spacer"></span>
    <div class="seg">
      {#each tabs as t (t)}<button class:on={tab === t} onclick={() => (tab = t)}>{t}</button>{/each}
    </div>
    <button class="icon-btn" title="Refresh" onclick={refresh}><Icon name="refresh" /></button>
  </header>
  {#if failure}<div class="bar error"><span class="ellipsis" title={failure}>{failure}</span><button class="icon-btn" title="Dismiss" onclick={() => (failure = '')}><Icon name="close" size={12} /></button></div>{/if}
  {#if ready?.moved && tab !== 'Overview'}
    <div class="bar">The source branch moved; line comments are off until you refresh. <button class="btn" onclick={refresh}>Refresh</button></div>
  {/if}
  <div class="content">
    {#if error}
      <div class="empty error">{error}</div>
    {:else if !mr}
      <div class="empty muted">Loading…</div>
    {:else if tab === 'Overview'}
      {@const m = mr}
      <div class="overview">
        <div class="meta">
          <span><span class="strong">{m.author}</span> wants to merge</span>
          <span class="mono branch">{m.sourceProject.path === m.targetProject.path ? '' : `${m.sourceProject.path}:`}{m.sourceBranch}</span>
          <span class="muted">into</span>
          <span class="mono branch">{m.targetBranch}</span>
          <span class="muted">· updated {ago(m.updated)} ago</span>
          {#if m.pipeline}<span class="dot" style:color="var(--{tone(m.pipeline)})" title="Pipeline {m.pipeline}"></span>{/if}
          <button class="link" onclick={() => open(m.webUrl)}>Open in browser</button>
        </div>
        {#if m.labels.length || m.reviewers.length || m.approvedBy.length}
          <dl>
            {#if m.labels.length}<dt>Labels</dt><dd>{#each m.labels as l (l)}<span class="label">{l}</span>{/each}</dd>{/if}
            {#if m.reviewers.length}<dt>Reviewers</dt><dd>{m.reviewers.join(', ')}</dd>{/if}
            {#if m.approvedBy.length}<dt>Approved by</dt><dd>{m.approvedBy.join(', ')}</dd>{/if}
          </dl>
        {/if}
        <div class="description">
          {#if m.description.trim()}<Markdown text={m.description} mr={m} />{:else}<span class="muted">No description.</span>{/if}
        </div>
        {#if m.state === 'open'}
          <MergeWidget mr={m} {act} />
          <div class="actions">
            {#if gitlab}
              <button class="btn" disabled={busy} onclick={() => act({ kind: m.approved ? 'unapprove' : 'approve' })}>{m.approved ? 'Unapprove' : 'Approve'}</button>
            {/if}
            <button class="btn" disabled={busy} onclick={() => act({ kind: 'setDraft', draft: !m.draft })}>{m.draft ? 'Mark as ready' : 'Mark as draft'}</button>
            <button class="btn danger" disabled={busy} onclick={() => act({ kind: 'close' })}>Close</button>
          </div>
          {#if !gitlab}
            <div class="review">
              <textarea rows="2" bind:value={review} placeholder="Review comment, required to request changes"></textarea>
              <div class="actions">
                {#if !m.approved}<button class="btn" disabled={busy} onclick={() => act({ kind: 'approve' })}>Approve</button>{/if}
                <button class="btn" disabled={busy || !review.trim()}
                  onclick={async () => (await act({ kind: 'requestChanges', body: review.trim() })) && (review = '')}>Request changes</button>
              </div>
            </div>
          {/if}
        {:else if m.state === 'closed'}
          <div class="actions"><button class="btn" disabled={busy} onclick={() => act({ kind: 'reopen' })}>Reopen</button></div>
        {/if}
        <h3>Activity</h3>
        {#each threads as t (t.id)}<Thread mr={m} thread={t} {act} />{:else}<p class="muted">No comments yet.</p>{/each}
        <Thread mr={m} thread={null} {act} start={(body) => ({ kind: 'comment', body })} />
      </div>
    {:else if tab === 'Pipelines'}
      <Pipelines {repo} mr={number} />
    {:else if !ready}
      {@const failed = typeof refs === 'object' && refs && 'error' in refs ? refs.error : ''}
      <div class="empty" class:error={failed}>{failed || 'Loading…'}</div>
    {:else if tab === 'Commits'}
      <div class="split">
        <div class="list">
          {#each ready.commits as c (c.sha)}
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <div class="row" class:selected={commit === c.sha} role="option" aria-selected={commit === c.sha} tabindex="-1" onclick={() => (commit = c.sha)}>
              <span class="mono muted">{c.sha.slice(0, 8)}</span>
              <span class="ellipsis grow">{c.subject}</span>
              <span class="muted ellipsis author">{c.author}</span>
              <span class="muted date" title={when(c.time)}>{ago(c.time)}</span>
            </div>
          {:else}<div class="empty muted">No commits</div>{/each}
        </div>
        <div class="side" style:width="{sideWidth}px">
          <Grip key="mr.side" bind:size={sideWidth} edge="left" min={260} max={1000} />
          {#if commit}<CommitDetails {repo} shas={[commit]} />{:else}<div class="empty muted">Select a commit</div>{/if}
        </div>
      </div>
    {:else}
      <div class="split">
        <div class="files" style:width="{filesWidth}px">
          <Grip key="mr.files" bind:size={filesWidth} edge="right" min={180} max={700} />
          <div class="files-head muted">{ready.files.length} {ready.files.length === 1 ? 'file' : 'files'}</div>
          {#each ready.files as f (f.path)}
            {@const n = threads.filter((t) => onFile(t, f)).length}
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <div class="row item {statusOf(f)}" class:selected={file?.path === f.path} role="option" aria-selected={file?.path === f.path} tabindex="-1"
              title={f.origPath ? `${f.origPath} → ${f.path}` : f.path} onclick={() => pick(f)}>
              <span class="st">{letter[statusOf(f)]}</span>
              <span class="name">{baseName(f.path)}</span>
              <span class="muted ellipsis grow">{dirName(f.path)}</span>
              {#if n}<span class="count" title="{n} comment threads">{n}</span>{/if}
            </div>
          {/each}
        </div>
        <div class="diff">
          {#if doc}
            <Viewer {doc} sideBySide {zones} {commentable}
              ongutter={ready.moved || mr.state !== 'open' ? undefined : (side, line, other) => (draft = { side, line, other })} />
          {:else}<div class="empty muted">Select a file</div>{/if}
        </div>
      </div>
    {/if}
  </div>
</div>

<style>
  .page { display: flex; flex-direction: column; height: 100%; }
  header { display: flex; align-items: center; gap: 8px; height: 44px; padding: 0 8px 0 16px; border-bottom: 1px solid var(--line); flex: none; min-width: 0; }
  .strong { font-weight: 800; }
  header .strong { font-size: 14px; }
  .title { font-size: 14px; font-weight: 700; min-width: 0; }
  .spacer { flex: 1; }
  .pill { flex: none; font-size: 11px; font-weight: 700; text-transform: capitalize; color: var(--muted); background: var(--hover); border-radius: 999px; padding: 1px 9px; }
  .pill.open { color: var(--success); }
  .pill.merged { color: var(--accent); }
  .bar { display: flex; align-items: center; gap: 8px; padding: 6px 16px; border-bottom: 1px solid var(--line); background: color-mix(in srgb, var(--warning) 12%, transparent); flex: none; }
  .bar.error { color: var(--error); background: color-mix(in srgb, var(--error) 10%, transparent); justify-content: space-between; }
  .content { flex: 1; min-height: 0; overflow: hidden; }
  .empty { padding: 12px 16px; }
  .error { color: var(--error); white-space: pre-wrap; }
  .overview { height: 100%; overflow-y: auto; padding: 16px 20px 24px; display: grid; gap: 14px; align-content: start; max-width: 960px; }
  .meta { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
  .branch { padding: 1px 6px; border-radius: 6px; background: var(--selected); }
  .link { padding: 0; border: 0; background: none; color: var(--accent); font-weight: 600; margin-left: auto; }
  dl { display: grid; grid-template-columns: auto 1fr; gap: 4px 12px; margin: 0; font-size: 12px; }
  dt { color: var(--muted); }
  dd { margin: 0; display: flex; flex-wrap: wrap; gap: 4px; }
  .label { padding: 0 7px; border-radius: 999px; font-size: 11px; font-weight: 700; background: var(--hover); }
  .description { padding: 12px 14px; border: 1px solid var(--line); border-radius: 12px; }
  .actions { display: flex; flex-wrap: wrap; gap: 8px; justify-content: flex-end; }
  .review { display: grid; gap: 8px; }
  h3 { font-size: 11px; font-weight: 700; margin: 6px 0 0; color: var(--muted); text-transform: uppercase; letter-spacing: 0.06em; }
  p { margin: 0; }
  .split { display: flex; height: 100%; }
  .list { flex: 1; min-width: 0; overflow-y: auto; padding: 6px 0; }
  .side { position: relative; flex: none; border-left: 1px solid var(--line); }
  .files { position: relative; flex: none; overflow-y: auto; padding-bottom: 6px; border-right: 1px solid var(--line); }
  .files-head { padding: 8px 16px 6px; font-size: 11px; font-weight: 700; letter-spacing: 0.06em; text-transform: uppercase; }
  .diff { flex: 1; min-width: 0; }
  .row { display: flex; align-items: center; gap: 8px; height: 28px; margin: 0 6px; padding: 0 8px; border-radius: 9px; white-space: nowrap; }
  .row:hover { background: var(--hover); }
  .row.selected { background: var(--selected); box-shadow: inset 0 0 0 1px var(--selected-ring); }
  .grow { flex: 1; min-width: 0; }
  .author { max-width: 100px; flex: none; }
  .date { flex: none; font-size: 12px; }
  .name { font-weight: 600; }
  .count { flex: none; min-width: 18px; padding: 0 5px; border-radius: 999px; font-size: 11px; font-weight: 700; text-align: center; background: var(--selected); color: var(--accent); }
</style>
