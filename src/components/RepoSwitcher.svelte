<script lang="ts">
  import { app, statuses, toneOf } from '../lib/app.svelte'
  import { filter, segments } from '../lib/fuzzy'
  import Icon from './Icon.svelte'

  let { onclose }: { onclose: () => void } = $props()
  let query = $state('')
  let sel = $state(Math.max(0, app.repos.findIndex((r) => r.path === app.current)))
  let input: HTMLInputElement
  let list: HTMLDivElement

  const base = (p: string) => p.split('/').filter(Boolean).pop() ?? p
  const items = $derived(filter(app.repos, query, (r) => r.name))
  $effect(() => input.focus())

  function choose(path: string) {
    app.current = path
    onclose()
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      sel = Math.max(0, Math.min(items.length - 1, sel + (e.key === 'ArrowDown' ? 1 : -1)))
      list.children[sel]?.scrollIntoView({ block: 'nearest' })
    } else if (e.key === 'Enter' && items[sel]) choose(items[sel].item.path)
    else if (e.key === 'Escape') onclose()
    else return
    e.preventDefault()
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="backdrop" onclick={onclose}></div>
<div class="popup glass" role="dialog" aria-label="Switch repository">
  <input class="search" placeholder="Switch repository" bind:this={input} bind:value={query} {onkeydown} oninput={() => (sel = 0)} />
  <div class="list" role="listbox" bind:this={list}>
    {#each items as { item: r, indices }, i (r.path)}
      {@const s = statuses.get(r.path)}
      {@const tone = toneOf(s)}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div class="item" class:selected={i === sel} role="option" aria-selected={i === sel} tabindex="-1" title={r.path}
        onmousemove={(e) => (e.movementX || e.movementY) && (sel = i)} onclick={() => choose(r.path)}>
        <span class="check">{#if r.path === app.current}<Icon name="check" size={14} />{/if}</span>
        <span class="name ellipsis">{#each segments(r.name, indices) as seg, k (k)}<span class:hit={seg.hit}>{seg.text}</span>{/each}
          {#if r.folder !== r.path}<span class="muted folder">{base(r.folder)}</span>{/if}</span>
        <span class="muted ellipsis branch">
          {#if typeof s === 'string'}{s}{:else if s}{s.branch ?? 'detached'}{s.ahead ? ` ↑${s.ahead}` : ''}{s.behind ? ` ↓${s.behind}` : ''}{/if}
        </span>
        {#if tone}<span class="dot" style:color="var(--{tone})"></span>{/if}
      </div>
    {:else}
      <p class="empty muted">No repositories match.</p>
    {/each}
  </div>
</div>

<style>
  .backdrop { position: fixed; inset: 0; z-index: 10; }
  .popup { position: absolute; top: 100%; left: 0; z-index: 11; width: 420px; margin-top: 8px; padding: 8px 0; }
  .search { width: calc(100% - 16px); height: 36px; margin: 0 8px 6px; font-size: 14px; }
  .list { max-height: 420px; overflow-y: auto; }
  .item { display: flex; align-items: center; gap: 8px; height: 32px; margin: 0 8px; padding: 0 10px 0 6px; border-radius: 9px; }
  .item.selected { background: var(--selected); box-shadow: inset 0 0 0 1px var(--selected-ring); }
  .check { display: inline-flex; flex: none; width: 14px; color: var(--accent); }
  .name { flex: 1; min-width: 0; font-weight: 600; }
  .folder { margin-left: 4px; font-size: 12px; font-weight: 500; }
  .branch { max-width: 45%; font-size: 12px; }
  .empty { margin: 0; padding: 8px 18px; }
</style>
