<script lang="ts">
  import { untrack } from 'svelte'
  import { api, type Pipeline } from '../lib/api'
  import { app, statusOf } from '../lib/app.svelte'
  import { cached, refresh } from '../lib/cache.svelte'
  import { active, color, glyph } from '../lib/pipelines'

  // The current branch's pipeline status; nothing when it has none.
  let { repo }: { repo: string | null } = $props()

  const st = $derived(statusOf(repo))
  // Without an upstream the backend lists every branch's pipelines, none of them this branch's.
  const key = $derived(st?.upstream ? `badge ${repo} ${st.head} ${st.branch}` : null)
  const p = $derived(key ? cached<Pipeline | null>(key)?.value : null)

  function load(focus = false) {
    const r = repo
    if (r && key) refresh(key, async () => (await api.listPipelines(r, false))[0] ?? null, focus)
  }

  $effect(() => {
    void key
    untrack(() => load())
  })

  $effect(() => {
    if (!p || !active(p.status)) return
    const t = setInterval(() => document.hidden || load(), 5000)
    return () => clearInterval(t)
  })
</script>

<svelte:window onfocus={() => load(true)} />

{#if p}
  <button class="badge" title="Pipeline #{p.id}: {p.status}" onclick={() => (app.bottom = 'pipelines')}>
    <span style:color={color(p.status)}>{glyph(p.status)}</span><span>{p.status}</span>
  </button>
{/if}

<style>
  .badge { display: flex; align-items: center; gap: 6px; height: 32px; padding: 0 10px; border: 0; border-radius: 10px; background: none; white-space: nowrap; }
  .badge:hover { background: var(--hover); }
</style>
