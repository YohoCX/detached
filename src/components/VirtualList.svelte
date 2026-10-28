<script lang="ts" generics="T">
  import type { Snippet } from 'svelte'

  let { items, height = 24, row }: { items: T[]; height?: number; row: Snippet<[T, number]> } = $props()

  let el: HTMLDivElement
  let top = $state(0)
  let view = $state(400)
  const first = $derived(Math.max(0, Math.floor(top / height) - 5))
  const last = $derived(Math.min(items.length, Math.ceil((top + view) / height) + 5))

  export function reveal(i: number) {
    if (i * height < el.scrollTop) el.scrollTop = i * height
    else if ((i + 1) * height > el.scrollTop + el.clientHeight) el.scrollTop = (i + 1) * height - el.clientHeight
  }
</script>

<div class="list" bind:this={el} bind:clientHeight={view} onscroll={() => (top = el.scrollTop)}>
  <div style:height="{items.length * height}px" style:position="relative">
    {#each items.slice(first, last) as item, k (first + k)}
      <div class="row" style:top="{(first + k) * height}px" style:height="{height}px">
        {@render row(item, first + k)}
      </div>
    {/each}
  </div>
</div>

<style>
  .list { overflow-y: auto; height: 100%; contain: strict; }
  .row { position: absolute; left: 0; right: 0; }
</style>
