<script lang="ts">
  import type { LogRow } from '../lib/api'

  // One row's lanes: `row.top` and `row.bottom` drawn around the dot at `row.column`.
  let { row }: { row: LogRow } = $props()
</script>

<svg width={(row.column + 1) * 14} height="24" aria-hidden="true"></svg>
