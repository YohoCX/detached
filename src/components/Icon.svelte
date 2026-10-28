<script lang="ts">
  const paths = {
    branch: 'M5 3v10M5 13a1.5 1.5 0 1 0 0 .01M5 3a1.5 1.5 0 1 0 0 .01M11 5a1.5 1.5 0 1 0 0 .01M11 6.5c0 3-6 2-6 5',
    fetch: 'M8 2v9M4.5 7.5 8 11l3.5-3.5M3 14h10',
    pull: 'M8 2v8M5 7l3 3 3-3M3 10v3h10v-3',
    refresh: 'M13 8a5 5 0 1 1-1.5-3.5M13 2.5v3h-3',
    plus: 'M8 3v10M3 8h10',
    minus: 'M3 8h10',
    close: 'M4 4l8 8M12 4l-8 8',
    folder: 'M2 4.5h4l1.5 1.5H14v6.5H2z',
    repo: 'M3.5 2.5h9v11h-9zM6 2.5v11',
    tree: 'M3 3h4M5 3v9h3M5 7.5h3M10 7.5h3M10 12h3',
    changes: 'M3 3h10v10H3zM6 8h4M8 6v4',
    chevron: 'M6 4l4 4-4 4',
    down: 'M4 6l4 4 4-4',
    zed: 'M4 4h8L4 12h8',
  }
  let { name, size = 16 }: { name: keyof typeof paths; size?: number } = $props()
</script>

<svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3"
  stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d={paths[name]} /></svg>
