<script module lang="ts">
  import { register } from '../lib/actions'
  import { app } from '../lib/app.svelte'

  const ui = $state({ all: false, run: false })

  register('pipeline.run', () => {
    app.bottom = 'pipelines'
    ui.run = true
  })
</script>

<script lang="ts">
  import { untrack } from 'svelte'
  import { SvelteMap } from 'svelte/reactivity'
  import { api, type Job, type JobAction, type Pipeline, type PipelineAction, type Project, type Stage, type Workflow } from '../lib/api'
  import { statusOf } from '../lib/app.svelte'
  import { cached, refresh } from '../lib/cache.svelte'
  import { when } from '../lib/log'
  import { active, color, glyph, jobKey, seen, took } from '../lib/pipelines'
  import { showJob } from '../lib/view.svelte'
  import Icon from './Icon.svelte'

  let { repo, mr }: { repo: string | null; mr?: number } = $props()

  interface RunForm { project: Project; branch: string; workflow: string; workflows: Workflow[]; vars: [string, string][] }

  let note = $state<string | null>(null)
  let busy = $state(false)
  let github = $state.raw(new Set<string>())
  let run = $state<RunForm | null>(null)
  /** Expanded pipelines: their stages, null while loading, or the error. */
  const stages = new SvelteMap<number, Stage[] | string | null>()

  const st = $derived(statusOf(repo))
  const head = $derived(`${st?.head} ${st?.branch}`)
  const key = $derived(`pipes ${repo} ${mr} ${ui.all}`)
  const cur = $derived(cached<Pipeline[]>(key))
  const list = $derived(cur?.value)
  const error = $derived(list ? null : cur?.error)
  const gh = (p: Project) => github.has(p.host)
  const running = $derived(
    !!list?.some((p) => active(p.status)) ||
      [...stages.values()].some((s) => Array.isArray(s) && s.some((g) => g.jobs.some((j) => active(j.status)))),
  )

  let expanded = ''
  $effect(() => {
    const k = key
    void head
    untrack(() => {
      if (k !== expanded) {
        expanded = k
        stages.clear()
      }
      load()
    })
  })

  // Hosts whose account is GitHub, which alone retries failed jobs and runs workflows.
  $effect(() => {
    const r = repo
    if (r)
      Promise.all([api.forgeHosts(r), api.listAccounts()]).then(([h, accounts]) => {
        const ids = new Set(accounts.filter((a) => a.provider.startsWith('github')).map((a) => a.id))
        if (r === repo) github = new Set(h.hosts.filter((x) => x.accountId && ids.has(x.accountId)).map((x) => x.host))
      }, () => {})
  })

  $effect(() => {
    if (!running) return
    const t = setInterval(() => document.hidden || load(), 5000)
    return () => clearInterval(t)
  })

  $effect(() => {
    void repo
    run = null
  })

  $effect(() => {
    if (!ui.run || mr) return
    ui.run = false
    untrack(openRun)
  })

  async function load(focus = false) {
    const [r, n, all, k] = [repo, mr, ui.all, key]
    if (!r) return
    const ps = await refresh(k, () => (n ? api.mrPipelines(r, n) : api.listPipelines(r, all)), focus)
    if (!ps || k !== key) return
    for (const id of stages.keys()) {
      const p = ps.find((x) => x.id === id)
      if (p) jobs(p)
      else stages.delete(id)
    }
  }

  async function jobs(p: Pipeline) {
    const k = key
    try {
      const s = await api.pipelineJobs(repo!, p.project, p.id)
      for (const j of s.flatMap((x) => x.jobs)) seen.set(jobKey(p.project, j.id), { job: j, pipeline: p.id })
      if (k === key && stages.has(p.id)) stages.set(p.id, s)
    } catch (e) {
      if (k === key && stages.has(p.id)) stages.set(p.id, String(e))
    }
  }

  function toggle(p: Pipeline) {
    if (stages.has(p.id)) return void stages.delete(p.id)
    stages.set(p.id, null)
    jobs(p)
  }

  async function act(f: () => Promise<unknown>) {
    busy = true
    note = null
    let ok = true
    try {
      await f()
    } catch (e) {
      note = String(e)
      ok = false
    }
    busy = false
    await load()
    return ok
  }
  const pipe = (p: Pipeline, a: PipelineAction) => act(() => api.pipelineAction(repo!, p.project, p.id, a))
  const job = (p: Pipeline, j: Job, a: JobAction) => act(() => api.jobAction(repo!, p.project, j.id, a))

  async function openRun() {
    const r = repo
    if (!r) return
    try {
      const project = list?.[0]?.project ?? (await api.newMr(r)).sourceProject
      const workflows = gh(project) ? await api.workflows(r, project) : []
      if (r === repo) run = { project, branch: st?.branch ?? '', workflow: workflows[0]?.id ?? '', workflows, vars: [] }
    } catch (e) {
      note = String(e)
    }
  }

  async function start(f: RunForm) {
    const input = {
      project: f.project,
      branch: f.branch.trim(),
      workflow: gh(f.project) ? f.workflow : null,
      variables: f.vars.filter(([k]) => k.trim()),
    }
    if (await act(() => api.runPipeline(repo!, input))) run = null
  }
</script>

<svelte:window onfocus={() => load(true)} />

{#if !repo}
  <div class="empty muted">No repository</div>
{:else}
  <div class="pipes">
    <div class="bar">
      {#if !mr}
        <div class="seg">
          <button class:on={!ui.all} onclick={() => (ui.all = false)}>Current branch</button>
          <button class:on={ui.all} onclick={() => (ui.all = true)}>All branches</button>
        </div>
      {/if}
      <span class="note error ellipsis" title={note ?? cur?.error}>{note ?? (list && cur?.error) ?? ''}</span>
      {#if !mr}<button class="btn" disabled={busy} onclick={openRun}>Run Pipeline…</button>{/if}
      <button class="icon-btn" title="Refresh" onclick={() => load()}><Icon name="refresh" /></button>
    </div>
    {#if run}
      {@const f = run}
      <div class="run">
        <label>Branch <input bind:value={f.branch} /></label>
        {#if gh(f.project)}
          <label>Workflow
            <select bind:value={f.workflow}>{#each f.workflows as w (w.id)}<option value={w.id}>{w.name}</option>{/each}</select>
          </label>
        {/if}
        {#each f.vars as v, i (i)}
          <span class="var">
            <input placeholder="Key" bind:value={v[0]} /><input placeholder="Value" bind:value={v[1]} />
            <button class="icon-btn" title="Remove" onclick={() => f.vars.splice(i, 1)}><Icon name="close" size={12} /></button>
          </span>
        {/each}
        <button class="icon-btn" onclick={() => f.vars.push(['', ''])}><Icon name="plus" size={12} />{gh(f.project) ? 'Input' : 'Variable'}</button>
        <span class="fill"></span>
        <button class="btn primary" disabled={busy || !f.branch.trim() || (gh(f.project) && !f.workflow)} onclick={() => start(f)}>Run</button>
        <button class="btn" onclick={() => (run = null)}>Cancel</button>
      </div>
    {/if}
    <div class="body">
      {#if error}
        <div class="empty error">
          {error}
          {#if /in Settings$/.test(error)}<button class="btn" onclick={() => (app.settings = true)}>Open Settings</button>{/if}
        </div>
      {:else if !list}
        <div class="empty muted">Loading…</div>
      {:else if !list.length}
        <div class="empty muted">No pipelines</div>
      {:else}
        {#each list as p (p.id)}
          {@const s = stages.get(p.id)}
          <div class="row" data-pipeline={p.id}>
            <button class="main" onclick={() => toggle(p)}>
              <Icon name={stages.has(p.id) ? 'down' : 'chevron'} size={12} />
              <span style:color={color(p.status)} title={p.status}>{glyph(p.status)}</span>
              <span class="strong">#{p.id}</span>
              <span class="ellipsis">{p.ref}</span>
              <span class="mono muted">{p.sha.slice(0, 8)}</span>
              <span class="muted ellipsis">
                {p.source} · {when(p.created)}{p.started ? ` · ${took((p.finished ?? Math.floor(Date.now() / 1000)) - p.started)}` : ''}
              </span>
            </button>
            {#if gh(p.project) && p.retryable && p.status === 'failed'}
              <button class="act" disabled={busy} onclick={() => pipe(p, 'retryFailed')}>Retry Failed</button>
            {/if}
            {#if p.retryable}<button class="act" disabled={busy} onclick={() => pipe(p, 'retry')}>Retry</button>{/if}
            {#if p.cancellable}<button class="act" disabled={busy} onclick={() => pipe(p, 'cancel')}>Cancel</button>{/if}
          </div>
          {#if s === null}
            <div class="sub muted">Loading…</div>
          {:else if typeof s === 'string'}
            <div class="sub error">{s}</div>
          {:else if s}
            {#each s as g (g.name)}
              <div class="sub stage"><span style:color={color(g.status)}>{glyph(g.status)}</span>{g.name}</div>
              {#each g.jobs as j (j.id)}
                <div class="row job" data-job={j.id}>
                  <button class="main" onclick={() => showJob(repo!, p.project, j.id, j.name)}>
                    <span style:color={color(j.status)} title={j.status}>{glyph(j.status)}</span>
                    <span class="ellipsis">{j.name}</span>
                    {#if j.allowFailure}<span class="tag">allowed to fail</span>{/if}
                    <span class="muted">{took(j.duration)}</span>
                  </button>
                  {#if j.playable}<button class="act" disabled={busy} onclick={() => job(p, j, 'play')}>Play</button>{/if}
                  {#if j.retryable}<button class="act" disabled={busy} onclick={() => job(p, j, 'retry')}>Retry</button>{/if}
                  {#if j.cancellable}<button class="act" disabled={busy} onclick={() => job(p, j, 'cancel')}>Cancel</button>{/if}
                </div>
              {/each}
            {/each}
          {/if}
        {/each}
      {/if}
    </div>
  </div>
{/if}

<style>
  .pipes { display: flex; flex-direction: column; height: 100%; }
  .bar, .run { display: flex; align-items: center; gap: 6px; padding: 6px 10px; flex: none; border-bottom: 1px solid var(--line); font-size: 12px; }
  .run { flex-wrap: wrap; }
  .run label, .var { display: flex; align-items: center; gap: 4px; }
  .var input { width: 120px; }
  .note { flex: 1; min-width: 0; }
  .fill { flex: 1; }
  .body { flex: 1; min-height: 0; overflow-y: auto; padding: 4px 0; }
  .empty { padding: 12px 16px; }
  .error { color: var(--error); white-space: pre-wrap; }
  .row { display: flex; align-items: center; gap: 2px; height: 28px; padding: 0 8px; }
  .row:hover { background: var(--hover); }
  .main { display: flex; align-items: center; gap: 8px; flex: 1; min-width: 0; height: 100%; border: 0; background: none; padding: 0 4px; text-align: left; }
  .job { padding-left: 48px; }
  .sub { display: flex; align-items: center; gap: 8px; height: 24px; padding-left: 34px; font-size: 12px; }
  .stage { color: var(--muted); font-weight: 700; }
  .strong { font-weight: 700; }
  .tag { font-size: 11px; color: var(--warning); border: 1px solid var(--line); border-radius: 6px; padding: 0 5px; white-space: nowrap; }
  .act { height: 22px; padding: 0 8px; border: 1px solid var(--line); border-radius: 999px; background: var(--input); font-size: 11px; font-weight: 700; }
  .act:disabled { opacity: 0.5; }
</style>
