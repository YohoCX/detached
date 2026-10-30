<script lang="ts">
  import { openUrl } from '@tauri-apps/plugin-opener'
  import { tick } from 'svelte'
  import { api, type Job, type Project } from '../lib/api'
  import { say } from '../lib/app.svelte'
  import { LogParser, visible, type Line } from '../lib/ansi'
  import { color, glyph, jobKey, seen } from '../lib/pipelines'
  import Icon from './Icon.svelte'
  import VirtualList from './VirtualList.svelte'

  // Never {@html}: text nodes, with spans only from parsed ANSI codes.
  let { repo, project, id, name }: { repo: string; project: Project; id: number; name: string } = $props()

  const MB = 1 << 20
  let parser = new LogParser()
  let rows = $state.raw<Line[]>([])
  let job = $state.raw<Job | null>(null)
  let following = $state(true)
  let cut = $state(false)
  let error = $state<string | null>(null)
  let list = $state<{ reveal(i: number): void }>()
  let stick = true

  const browse = (url: string) => openUrl(url).catch((e) => say(String(e), true))

  async function show() {
    rows = visible(parser.lines)
    await tick()
    if (stick) list?.reveal(rows.length - 1)
  }

  $effect(() => {
    const [r, p, n] = [repo, project, id]
    const known = seen.get(jobKey(p, n))
    let stop = false
    let timer: ReturnType<typeof setTimeout>
    let offset = 0
    let raw = ''
    parser = new LogParser()
    rows = []
    job = known?.job ?? null
    following = true
    cut = false
    error = null
    stick = true

    // Status and steps come from the pipeline's job list; a GitHub job's log waits until it ends.
    const refresh = async () => {
      if (!known) return
      const stages = await api.pipelineJobs(r, p, known.pipeline).catch(() => [])
      const j = stages.flatMap((s) => s.jobs).find((x) => x.id === n)
      if (!stop && j) job = j
    }

    async function next() {
      try {
        const c = await api.jobLog(r, p, n, offset)
        if (stop) return
        offset = c.next
        raw += c.text
        if (offset > 20 * MB) {
          parser = new LogParser()
          parser.push(raw.slice(raw.indexOf('\n', raw.length - MB) + 1), true)
          raw = ''
          cut = true
          following = false
          return show()
        }
        parser.push(c.text, c.complete)
        if (c.text) show()
        if (c.complete) following = false
        if (!job?.liveLog || c.complete) await refresh()
        if (!stop && !c.complete) timer = setTimeout(next, job && !job.liveLog ? 5000 : offset < 2 * MB ? 3000 : 10000)
      } catch (e) {
        if (stop) return
        error = String(e)
        following = false
      }
    }
    next()
    return () => {
      stop = true
      clearTimeout(timer)
    }
  })

  function click(e: MouseEvent, l: Line) {
    const a = (e.target as HTMLElement).closest('a')
    if (a) {
      e.preventDefault()
      browse(a.href)
    } else if (l.head) {
      l.head.open = !l.head.open
      stick = false
      rows = visible(parser.lines)
    }
  }

  function onscroll(e: Event) {
    const el = e.target as HTMLElement
    stick = el.scrollTop + el.clientHeight >= el.scrollHeight - 4
  }
</script>

<div class="log">
  <header>
    {#if job}<span style:color={color(job.status)} title={job.status}>{glyph(job.status)}</span>{/if}
    <span class="strong">{name}</span><span class="muted">{project.path} #{id}</span>
    <span class="fill"></span>
    {#if following}<span class="muted">Following…</span>{/if}
    {#if cut}<span class="warn">Over 20 MB: showing the end only</span>{/if}
    {#if job?.webUrl}<button class="btn" onclick={() => browse(job!.webUrl)}>Open in Browser</button>{/if}
  </header>
  {#if error}
    <div class="empty error">{error}</div>
  {:else if following && job && !job.liveLog}
    <div class="steps">
      <div class="muted">GitHub serves the log once the job has finished.</div>
      {#each job.steps as s, i (i)}
        <div class="step"><span style:color={color(s.status)} title={s.status}>{glyph(s.status)}</span>{s.name}</div>
      {/each}
    </div>
  {:else if !rows.length}
    <div class="empty muted">{following ? 'Waiting for output…' : 'Empty log'}</div>
  {:else}
    <div class="lines" onscrollcapture={onscroll}>
      <VirtualList bind:this={list} items={rows} height={18}>
        {#snippet row(l)}
          <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
          <div class="line" class:head={l.head} style:padding-left="{12 + l.depth * 16}px" onclick={(e) => click(e, l)}>
            {#if l.head}<span class="fold"><Icon name={l.head.open ? 'down' : 'chevron'} size={11} /></span>{/if}
            {#each l.spans as s, k (k)}
              <svelte:element this={s.href ? 'a' : 'span'} href={s.href} class:b={s.bold} class:d={s.dim} class:i={s.italic}
                class:u={s.underline} style:color={s.fg} style:background={s.bg}>{s.text}</svelte:element>
            {/each}
          </div>
        {/snippet}
      </VirtualList>
    </div>
  {/if}
</div>

<style>
  .log {
    display: flex; flex-direction: column; height: 100%;
    --ansi-0: #1e293b; --ansi-1: #dc2626; --ansi-2: #059669; --ansi-3: #b45309; --ansi-4: #2563eb; --ansi-5: #c026d3; --ansi-6: #0891b2; --ansi-7: #64748b;
    --ansi-8: #475569; --ansi-9: #ef4444; --ansi-10: #10b981; --ansi-11: #d97706; --ansi-12: #3b82f6; --ansi-13: #d946ef; --ansi-14: #06b6d4; --ansi-15: #334155;
  }
  @media (prefers-color-scheme: dark) {
    .log {
      --ansi-0: #64748b; --ansi-1: #f87171; --ansi-2: #34d399; --ansi-3: #fbbf24; --ansi-4: #60a5fa; --ansi-5: #e879f9; --ansi-6: #22d3ee; --ansi-7: #cbd5e1;
      --ansi-8: #94a3b8; --ansi-9: #fca5a5; --ansi-10: #6ee7b7; --ansi-11: #fde68a; --ansi-12: #93c5fd; --ansi-13: #f0abfc; --ansi-14: #67e8f9; --ansi-15: #f8fafc;
    }
  }
  header { display: flex; align-items: center; gap: 8px; height: 44px; padding: 0 16px; border-bottom: 1px solid var(--line); flex: none; }
  .strong { font-weight: 700; }
  .fill { flex: 1; }
  .warn { color: var(--warning); }
  .empty { padding: 12px 16px; }
  .error { color: var(--error); white-space: pre-wrap; }
  .steps { display: flex; flex-direction: column; gap: 6px; padding: 12px 16px; }
  .step { display: flex; gap: 8px; }
  .lines { flex: 1; min-height: 0; background: var(--code); }
  .line { height: 18px; font: 12px/18px var(--mono); white-space: pre; -webkit-user-select: text; user-select: text; }
  .line.head { cursor: pointer; }
  .line.head:hover { background: var(--hover); }
  .fold { display: inline-flex; width: 14px; margin-left: -14px; color: var(--muted); vertical-align: -1px; }
  .b { font-weight: 700; }
  .d { opacity: 0.6; }
  .i { font-style: italic; }
  .u, a { text-decoration: underline; }
  a { color: var(--accent); }
</style>
