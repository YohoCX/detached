<script lang="ts">
  import { untrack } from 'svelte'
  import { load, save } from '../lib/storage'

  // On one edge of a relatively positioned pane; the size is remembered per key.
  let { key, size = $bindable(), edge, min = 160, max = 1200 }: {
    key: string
    size: number
    edge: 'left' | 'right' | 'top'
    min?: number
    max?: number
  } = $props()

  untrack(() => (size = Number(load(`size.${key}`)) || size))

  const pos = (e: PointerEvent) => (edge === 'top' ? e.clientY : e.clientX)

  function down(e: PointerEvent) {
    const grip = e.currentTarget as HTMLElement
    const [start, from, sign] = [pos(e), size, edge === 'right' ? 1 : -1]
    grip.setPointerCapture(e.pointerId)
    grip.onpointermove = (m) => (size = Math.round(Math.max(min, Math.min(max, from + sign * (pos(m) - start)))))
    grip.onlostpointercapture = () => {
      grip.onpointermove = null
      save(`size.${key}`, String(size))
    }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="grip {edge}" onpointerdown={down}></div>

<style>
  .grip { position: absolute; z-index: 3; }
  .grip:hover { background: var(--selected-ring); }
  .top { top: 0; left: 14px; right: 14px; height: 4px; cursor: row-resize; border-radius: 0 0 4px 4px; }
  .left, .right { top: 14px; bottom: 14px; width: 4px; cursor: col-resize; }
  .left { left: 0; border-radius: 0 4px 4px 0; }
  .right { right: 0; border-radius: 4px 0 0 4px; }
</style>
