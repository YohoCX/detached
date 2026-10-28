<script lang="ts">
  import { onMount } from 'svelte'
  import { api, type RepoInfo } from './lib/api'
  import { monaco } from './lib/monaco'

  let repos = $state<RepoInfo[]>([])
  let host: HTMLDivElement

  onMount(() => {
    api.listRepos().then((r) => (repos = r))
    const editor = monaco.editor.create(host, { value: '// Workbench', readOnly: true, automaticLayout: true })
    return () => editor.dispose()
  })
</script>

<main>
  <aside>
    {#each repos as r (r.path)}<div>{r.name}</div>{/each}
  </aside>
  <div class="editor" bind:this={host}></div>
</main>

<style>
  main { display: grid; grid-template-columns: 260px 1fr; height: 100%; }
  aside { border-right: 1px solid var(--line); padding: 8px; overflow: auto; }
  .editor { min-width: 0; }
</style>
