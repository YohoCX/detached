<script lang="ts">
  import { api } from '../lib/api'
  import { app, settingsCall } from '../lib/app.svelte'
  import Icon from './Icon.svelte'

  let { onclose }: { onclose: () => void } = $props()
  let created = $state('')
  let editing = $state<string | null>(null)
  let renamed = $state('')

  const toggle = (name: string, active: boolean) => settingsCall(() => api.setWorkspaceActive(name, !active))
  const create = () => created.trim() && settingsCall(() => api.saveWorkspace(created, null)).then(() => (created = ''))
  const remove = (name: string) => settingsCall(() => api.removeWorkspace(name))

  function rename(from: string) {
    editing = null
    if (renamed.trim() && renamed.trim() !== from) settingsCall(() => api.saveWorkspace(renamed, from))
  }

  function edit(name: string) {
    editing = name
    renamed = name
  }
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && !editing && onclose()} />

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="backdrop" onclick={onclose}></div>
<div class="popup glass" role="dialog" aria-label="Workspaces">
  {#each app.workspaces as w (w.name)}
    {#if editing === w.name}
      <!-- svelte-ignore a11y_autofocus -->
      <input class="name-input" aria-label="Workspace name" autofocus bind:value={renamed} onblur={() => rename(w.name)}
        onkeydown={(e) => {
          if (e.key === 'Enter') rename(w.name)
          else if (e.key === 'Escape') editing = null
        }} />
    {:else}
      <div class="item" role="checkbox" aria-checked={w.active} tabindex="0" title={w.folders.join('\n')}
        onclick={() => toggle(w.name, w.active)}
        onkeydown={(e) => (e.key === ' ' || e.key === 'Enter') && toggle(w.name, w.active)}>
        <span class="check">{#if w.active}<Icon name="check" size={14} />{/if}</span>
        <span class="name ellipsis">{w.name}</span>
        <span class="muted count">{w.folders.length} folder{w.folders.length === 1 ? '' : 's'}</span>
        <button class="icon-btn hover" title="Rename {w.name}" onclick={(e) => {
          e.stopPropagation()
          edit(w.name)
        }}><Icon name="edit" size={12} /></button>
        <button class="icon-btn hover" title="Delete {w.name}" onclick={(e) => {
          e.stopPropagation()
          remove(w.name)
        }}><Icon name="close" size={12} /></button>
      </div>
    {/if}
  {/each}
  <input class="name-input" placeholder="New workspace" bind:value={created} onkeydown={(e) => e.key === 'Enter' && create()} />
</div>

<style>
  .backdrop { position: fixed; inset: 0; z-index: 10; }
  .popup { position: absolute; top: 100%; left: 0; z-index: 11; width: 320px; margin-top: 8px; padding: 8px 0; }
  .item { display: flex; align-items: center; gap: 8px; height: 32px; margin: 0 8px; padding: 0 6px; border-radius: 9px; }
  .item:hover { background: var(--hover); }
  .check { display: inline-flex; flex: none; width: 14px; color: var(--accent); }
  .name { flex: 1; min-width: 0; font-weight: 600; }
  .count { font-size: 12px; }
  .hover { visibility: hidden; height: 20px; min-width: 20px; }
  .item:hover .hover { visibility: visible; }
  .name-input { width: calc(100% - 16px); height: 32px; margin: 4px 8px 0; }
</style>
