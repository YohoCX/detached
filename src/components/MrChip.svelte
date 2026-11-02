<script lang="ts">
  import { onMount, untrack } from 'svelte'
  import { register } from '../lib/actions'
  import { api, type MergeRequest, type NewMr } from '../lib/api'
  import { say, statusOf } from '../lib/app.svelte'
  import { cached, refresh } from '../lib/cache.svelte'
  import { bareTitle, isGitLab } from '../lib/mr'
  import { showMr } from '../lib/view.svelte'
  import Icon from './Icon.svelte'

  let { repo }: { repo: string | null } = $props()

  let form = $state<(NewMr & { description: string; draft: boolean; error: string; busy: boolean }) | null>(null)
  const st = $derived(statusOf(repo))
  const key = $derived(repo && st?.branch && st.upstream ? `chip\0${repo}\0${st.branch}\0${st.upstream}` : '')
  /** undefined while unknown or when the repo has no working forge setup. */
  const mr = $derived(key ? cached<MergeRequest | null>(key)?.value : undefined)

  function load(focus = false) {
    const r = repo
    if (r && key) refresh(key, () => api.currentMr(r), focus)
  }

  $effect(() => {
    void key
    form = null
    untrack(() => load())
  })

  async function openForm() {
    const [r, k] = [repo, key]
    if (!r || !k) return say('Create Merge Request needs a branch with an upstream', true)
    if (mr) return showMr(r, mr.number)
    try {
      const m = await api.newMr(r)
      if (k === key) form = { ...m, description: '', draft: false, error: '', busy: false }
    } catch (e) {
      say(String(e), true)
    }
  }

  async function create() {
    const [f, r] = [form, repo]
    if (!f || !r || !f.title.trim()) return
    f.busy = true
    try {
      const m = await api.createMr(r, { title: f.title.trim(), description: f.description, targetBranch: f.targetBranch.trim(), draft: f.draft })
      form = null
      load()
      showMr(r, m.number)
    } catch (e) {
      f.error = String(e)
      f.busy = false
    }
  }

  onMount(() => register('mr.create', openForm))
</script>

<svelte:window onfocus={() => load(true)} />

{#if repo && mr !== undefined}
  <div class="anchor">
    {#if mr}
      <button class="chip" title={mr.title} onclick={() => showMr(repo, mr!.number)}>
        <Icon name="mr" size={14} /><span class="num">{isGitLab(mr) ? '!' : '#'}{mr.number}</span><span class="ellipsis">{bareTitle(mr)}</span>
      </button>
    {:else}
      <button class="icon-btn" title="Create Merge Request" onclick={openForm}><Icon name="mr" size={14} /><Icon name="plus" size={12} /></button>
    {/if}
    {#if form}
      {@const f = form}
      <div class="backdrop" onclick={() => (form = null)} role="presentation"></div>
      <div class="popup glass" role="dialog" aria-label="Create Merge Request">
        <div class="title">New merge request</div>
        <div class="muted from">
          <span class="mono">{f.sourceProject.path}:{f.sourceBranch}</span> into <span class="mono">{f.targetProject.path}</span>
        </div>
        <label>Title<input bind:value={f.title} {@attach (el) => el.focus()} /></label>
        <label>Target branch<input bind:value={f.targetBranch} /></label>
        <label>Description<textarea rows="5" bind:value={f.description}></textarea></label>
        <label class="check"><input type="checkbox" bind:checked={f.draft} /> Draft</label>
        {#if f.error}<p class="error">{f.error}</p>{/if}
        <div class="buttons">
          <button class="btn" onclick={() => (form = null)}>Cancel</button>
          <button class="btn primary" disabled={f.busy || !f.title.trim() || !f.targetBranch.trim()} onclick={create}>Create</button>
        </div>
      </div>
    {/if}
  </div>
{/if}

<svelte:document onkeydown={(e) => form && e.key === 'Escape' && (form = null)} />

<style>
  .anchor { position: relative; min-width: 0; }
  .chip { display: flex; align-items: center; gap: 6px; height: 28px; max-width: 260px; padding: 0 10px; border: 1px solid var(--selected-ring); border-radius: 999px; background: var(--selected); }
  .chip:hover { box-shadow: 0 0 12px var(--selected); }
  .num { font-weight: 800; }
  .backdrop { position: fixed; inset: 0; z-index: 10; }
  .popup { position: absolute; top: 100%; left: 0; z-index: 11; width: 440px; margin-top: 8px; padding: 14px 16px; display: grid; gap: 10px; }
  .title { font-weight: 800; }
  .from { font-size: 12px; }
  label { display: grid; gap: 4px; font-size: 12px; font-weight: 600; color: var(--muted); }
  label input:not([type]), textarea { color: var(--fg); font-weight: 500; }
  .check { display: flex; align-items: center; gap: 6px; }
  .error { margin: 0; color: var(--error); white-space: pre-wrap; }
  .buttons { display: flex; justify-content: flex-end; gap: 8px; }
</style>
