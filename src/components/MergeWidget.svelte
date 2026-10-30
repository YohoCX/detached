<script lang="ts">
  import type { MergeRequest } from '../lib/api'

  let { repo, mr, onchange }: { repo: string; mr: MergeRequest; onchange: () => void } = $props()
</script>

<div class="muted">{mr.blocking ?? 'Ready to merge'}</div>
