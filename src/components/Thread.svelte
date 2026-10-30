<script lang="ts">
  import type { MergeRequest, Thread } from '../lib/api'
  import Markdown from './Markdown.svelte'

  let { repo, mr, thread, onchange }: { repo: string; mr: MergeRequest; thread: Thread; onchange: () => void } = $props()
</script>

<div class="thread">
  {#each thread.notes as n (n.id)}<Markdown text={n.body} projectUrl={mr.projectUrl} baseUrl={mr.projectUrl} />{/each}
</div>
