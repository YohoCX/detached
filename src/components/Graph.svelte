<script module lang="ts">
  // Mid tones that read on both the light and the dark panel.
  const palette = ['#6366f1', '#10b981', '#f59e0b', '#ec4899', '#06b6d4', '#8b5cf6', '#ef4444', '#84cc16']
  export const LANE = 14
</script>

<script lang="ts">
  import type { Edge, LogRow } from '../lib/api'
  import { lanes } from '../lib/log'

  let { row }: { row: LogRow } = $props()

  const H = 28
  const x = (c: number) => c * LANE + LANE / 2
  const color = (c: number) => palette[c % palette.length]
  // Vertical at both ends, so segments join smoothly across rows.
  const path = (e: Edge, y1: number, y2: number) =>
    e.from === e.to
      ? `M${x(e.from)} ${y1}V${y2}`
      : `M${x(e.from)} ${y1}C${x(e.from)} ${(y1 + y2) / 2} ${x(e.to)} ${(y1 + y2) / 2} ${x(e.to)} ${y2}`
</script>

<svg width={lanes(row) * LANE} height={H} aria-hidden="true">
  {#each row.top as e, i (i)}<path d={path(e, 0, H / 2)} stroke={color(e.color)} />{/each}
  {#each row.bottom as e, i (i)}<path d={path(e, H / 2, H)} stroke={color(e.color)} />{/each}
  <circle cx={x(row.column)} cy={H / 2} r="4" fill={color(row.color)} />
</svg>

<style>
  svg { display: block; flex: none; }
  path { fill: none; stroke-width: 2; stroke-linecap: round; }
</style>
