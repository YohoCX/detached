<script lang="ts">
  import { onMount } from 'svelte'
  import { monaco } from '../lib/monaco'

  // Agent D: viewer and diff. This read-only editor only shows Monaco loads and chords win over it.
  let host: HTMLDivElement

  onMount(() => {
    const editor = monaco.editor.create(host, {
      value: '// Workbench',
      readOnly: true,
      automaticLayout: true,
      minimap: { enabled: false },
      theme: matchMedia('(prefers-color-scheme: dark)').matches ? 'vs-dark' : 'vs',
    })
    return () => editor.dispose()
  })
</script>

<div class="editor" bind:this={host}></div>

<style>
  .editor { height: 100%; min-width: 0; }
</style>
