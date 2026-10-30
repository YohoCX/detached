<script lang="ts">
  import type { MergeMethod, MergeRequest, MrAction } from '../lib/api'
  import { isGitLab } from '../lib/mr'

  let { mr, act }: { mr: MergeRequest; act: (a: MrAction) => Promise<boolean> } = $props()

  const names: Record<MergeMethod, string> = { merge: 'Merge commit', squash: 'Squash', rebase: 'Rebase' }
  let picked = $state<MergeMethod | null>(null)
  let del = $state(false)
  let auto = $state(false)
  let busy = $state(false)
  const method = $derived(picked && mr.mergeMethods.includes(picked) ? picked : (mr.mergeMethods[0] ?? 'merge'))
  const running = $derived(mr.pipeline === 'pending' || mr.pipeline === 'running')
  // GitHub deletes the branch itself after a merge, which it cannot do after an auto-merge.
  const canDelete = $derived(mr.canDeleteSourceBranch && (isGitLab(mr) || !(auto && running)))

  async function merge() {
    busy = true
    await act({ kind: 'merge', method, deleteSourceBranch: canDelete && del, autoMerge: auto && running, headSha: mr.headSha })
    busy = false
  }
</script>

<div class="merge">
  <div class="status">
    <span class="dot" style:color="var(--{mr.autoMerge ? 'accent' : mr.blocking ? 'warning' : 'success'})"></span>
    {#if mr.autoMerge}Set to merge when the pipeline succeeds
    {:else if mr.blocking}Cannot merge: {mr.blocking}
    {:else}Ready to merge{/if}
  </div>
  {#if !mr.autoMerge}
    <div class="controls">
      {#if mr.mergeMethods.length > 1}
        <div class="seg">
          {#each mr.mergeMethods as m (m)}<button class:on={m === method} onclick={() => (picked = m)}>{names[m]}</button>{/each}
        </div>
      {/if}
      {#if canDelete}<label><input type="checkbox" bind:checked={del} /> Delete source branch</label>{/if}
      {#if running}<label><input type="checkbox" bind:checked={auto} /> When the pipeline succeeds</label>{/if}
      <button class="btn primary" disabled={busy || (!!mr.blocking && !(auto && running))} onclick={merge}>
        {auto && running ? 'Set auto-merge' : 'Merge'}
      </button>
    </div>
  {/if}
</div>

<style>
  .merge { display: grid; gap: 10px; padding: 12px 14px; border: 1px solid var(--line); border-radius: 12px; background: var(--panel); }
  .status { display: flex; align-items: center; gap: 8px; font-weight: 700; }
  .controls { display: flex; flex-wrap: wrap; align-items: center; gap: 12px; }
  .controls .btn { margin-left: auto; }
  label { display: flex; align-items: center; gap: 5px; }
</style>
