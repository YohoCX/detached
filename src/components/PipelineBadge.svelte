<script lang="ts">
  import { api, type Pipeline } from '../lib/api'
  import { app, statusOf } from '../lib/app.svelte'
  import { active, color, glyph } from '../lib/pipelines'

  // The current branch's pipeline status; nothing when it has none.
  let { repo }: { repo: string | null } = $props()

  let p = $state.raw<Pipeline | null>(null)
  const st = $derived(statusOf(repo))
  // Without an upstream the backend lists every branch's pipelines, none of them this branch's.
  const key = $derived(st?.upstream ? `${repo} ${st.head} ${st.branch}` : null)

  async function load() {
    const k = key
    if (!k) return
    try {
      const ps = await api.listPipelines(repo!, false)
      if (k === key) p = ps[0] ?? null
    } catch {
      if (k === key) p = null
    }
  }

  $effect(() => {
    void key
    p = null
    load()
  })

  $effect(() => {
    if (!p || !active(p.status)) return
    const t = setInterval(() => document.hidden || load(), 5000)
    return () => clearInterval(t)
  })
</script>

<svelte:window onfocus={load} />

{#if p}
  <button class="badge" title="Pipeline #{p.id}: {p.status}" onclick={() => (app.bottom = 'pipelines')}>
    <span style:color={color(p.status)}>{glyph(p.status)}</span><span>{p.status}</span>
  </button>
{/if}

<style>
  .badge { display: flex; align-items: center; gap: 6px; height: 32px; padding: 0 10px; border: 0; border-radius: 10px; background: none; white-space: nowrap; }
  .badge:hover { background: var(--hover); }
</style>
