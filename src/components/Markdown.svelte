<script lang="ts">
  import { openUrl } from '@tauri-apps/plugin-opener'
  import type { MergeRequest } from '../lib/api'
  import { say } from '../lib/app.svelte'
  import { links, render } from '../lib/mr'

  // Foreign markdown, sanitized; a link opens in the browser when it is http(s), else does nothing.
  let { text, mr }: { text: string; mr: MergeRequest } = $props()

  const html = $derived(render(text, links(mr)))

  function onclick(e: MouseEvent) {
    const a = (e.target as Element).closest('a')
    if (!a) return
    e.preventDefault()
    const href = a.getAttribute('href') ?? ''
    if (/^https?:\/\//i.test(href)) openUrl(href).catch((err) => say(String(err), true))
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="markdown" {onclick} onauxclick={onclick}>{@html html}</div>

<style>
  .markdown { overflow-wrap: anywhere; line-height: 1.5; -webkit-user-select: text; user-select: text; cursor: auto; }
  .markdown :global(:is(p, ul, ol, pre, blockquote, table)) { margin: 0 0 8px; }
  .markdown :global(:last-child) { margin-bottom: 0; }
  .markdown :global(:is(h1, h2, h3, h4, h5, h6)) { margin: 12px 0 6px; font-size: 14px; font-weight: 800; }
  .markdown :global(:is(ul, ol)) { padding-left: 20px; }
  .markdown :global(a) { color: var(--accent); text-decoration: none; cursor: pointer; }
  .markdown :global(a:hover) { text-decoration: underline; }
  .markdown :global(code) { font-family: var(--mono); font-size: 12px; background: var(--hover); border-radius: 5px; padding: 1px 4px; }
  .markdown :global(pre) { background: var(--hover); border-radius: 9px; padding: 8px 10px; overflow-x: auto; }
  .markdown :global(pre code) { background: none; padding: 0; }
  .markdown :global(blockquote) { padding-left: 10px; border-left: 3px solid var(--line); color: var(--muted); }
  .markdown :global(img) { max-width: 100%; }
  .markdown :global(table) { border-collapse: collapse; }
  .markdown :global(:is(th, td)) { border: 1px solid var(--line); padding: 3px 8px; }
</style>
