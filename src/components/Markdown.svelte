<script lang="ts" module>
  let diagrams = 0
</script>

<script lang="ts">
  import { openUrl } from '@tauri-apps/plugin-opener'
  import { api, type MergeRequest } from '../lib/api'
  import { say } from '../lib/app.svelte'
  import { languageFor } from '../lib/files'
  import { hasScheme, renderFile, repoPath, slug } from '../lib/markdown'
  import { monaco } from '../lib/monaco'
  import { links, render } from '../lib/mr'
  import { open, type FileRef } from '../lib/nav'

  // An MR's text, or a repo file's, whose relative links open in the app and images load from the repo.
  let { text, mr, file }: { text: string; mr?: MergeRequest; file?: FileRef } = $props()

  let root: HTMLDivElement
  const scheme = matchMedia('(prefers-color-scheme: dark)')
  let dark = $state(scheme.matches)
  const html = $derived(file ? renderFile(text, file.path) : render(text, links(mr!)))

  function langOf(tag: string) {
    const t = tag.toLowerCase()
    const langs = monaco.languages.getLanguages()
    return langs.find((l) => l.id === t || l.aliases?.some((a) => a.toLowerCase() === t))?.id ?? languageFor(`x.${t}`, langs)
  }

  async function mermaid(blocks: HTMLElement[]) {
    const m = (await import('mermaid')).default
    const css = getComputedStyle(document.documentElement)
    m.initialize({ startOnLoad: false, securityLevel: 'strict', theme: dark ? 'dark' : 'default', fontFamily: css.getPropertyValue('--font') })
    // The source stays in its hidden <pre>, so a theme change can draw it again.
    for (const pre of blocks) {
      const id = `mermaid-${++diagrams}`
      const out = document.createElement('div')
      try {
        out.innerHTML = (await m.render(id, pre.textContent!)).svg
        out.className = 'mermaid'
      } catch (e) {
        document.getElementById(`d${id}`)?.remove()
        out.className = 'mermaid-error'
        out.textContent = e instanceof Error ? e.message : String(e)
      }
      if (pre.nextElementSibling?.matches('.mermaid, .mermaid-error')) pre.nextElementSibling.remove()
      pre.hidden = out.className === 'mermaid'
      pre.after(out)
    }
  }

  $effect(() => {
    const f = () => (dark = scheme.matches)
    scheme.addEventListener('change', f)
    return () => scheme.removeEventListener('change', f)
  })

  $effect(() => {
    void html
    void dark
    for (const code of root.querySelectorAll<HTMLElement>('code[data-lang]')) {
      const lang = langOf(code.dataset.lang!)
      if (lang !== 'plaintext') monaco.editor.colorize(code.textContent!, lang, {}).then((h) => (code.innerHTML = h))
    }
    const blocks = [...root.querySelectorAll<HTMLElement>('pre[data-mermaid]')]
    if (blocks.length) mermaid(blocks).catch((e) => say(String(e), true))
    if (file) {
      const { repo, ref } = file
      for (const img of root.querySelectorAll<HTMLImageElement>('img[data-path]'))
        api.readImage(repo, ref, img.dataset.path!).then((url) => url && (img.src = url))
    }
  })

  function onclick(e: MouseEvent) {
    const a = (e.target as Element).closest('a')
    if (!a) return
    e.preventDefault()
    const href = a.getAttribute('href') ?? ''
    if (/^https?:\/\//i.test(href)) openUrl(href).catch((err) => say(String(err), true))
    else if (href.startsWith('#')) {
      const id = decodeURIComponent(href.slice(1))
      ;[...root.querySelectorAll('h1, h2, h3, h4, h5, h6')].find((h) => slug(h.textContent!) === id)?.scrollIntoView()
    } else if (file && href && !hasScheme(href)) open({ repo: file.repo, ref: file.ref, path: repoPath(href, file.path), line: null })
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="markdown" class:doc={file} bind:this={root} {onclick} onauxclick={onclick}>{@html html}</div>

<style>
  .markdown { overflow-wrap: anywhere; line-height: 1.5; -webkit-user-select: text; user-select: text; cursor: auto; }
  .markdown :global(:is(p, ul, ol, pre, blockquote, table, .mermaid)) { margin: 0 0 8px; }
  .markdown :global(:last-child) { margin-bottom: 0; }
  .markdown :global(:is(h1, h2, h3, h4, h5, h6)) { margin: 12px 0 6px; font-size: 14px; font-weight: 800; }
  .markdown :global(:is(ul, ol)) { padding-left: 20px; }
  .markdown :global(li:has(> input)) { list-style: none; }
  .markdown :global(li > input) { margin: 0 6px 0 -18px; }
  .markdown :global(a) { color: var(--accent); text-decoration: none; cursor: pointer; }
  .markdown :global(a:hover) { text-decoration: underline; }
  .markdown :global(code) { font-family: var(--mono); font-size: 12px; background: var(--hover); border-radius: 5px; padding: 1px 4px; }
  .markdown :global(pre) { background: var(--hover); border-radius: 9px; padding: 8px 10px; overflow-x: auto; }
  .markdown :global(pre code) { background: none; padding: 0; }
  .markdown :global(pre[data-mermaid]) { font-family: var(--mono); font-size: 12px; }
  .markdown :global(.mermaid) { text-align: center; }
  .markdown :global(.mermaid svg) { max-width: 100%; height: auto; }
  .markdown :global(.mermaid-error) { margin: -4px 0 8px; color: var(--error); font-size: 12px; white-space: pre-wrap; font-family: var(--mono); }
  .markdown :global(blockquote) { padding-left: 10px; border-left: 3px solid var(--line); color: var(--muted); }
  .markdown :global(img) { max-width: 100%; }
  .markdown :global(table) { border-collapse: collapse; }
  .markdown :global(:is(th, td)) { border: 1px solid var(--line); padding: 3px 8px; }

  .markdown.doc { max-width: 880px; margin: 0 auto; padding: 24px 32px 48px; font-size: 14px; line-height: 1.6; }
  .markdown.doc :global(:is(p, ul, ol, pre, blockquote, table, .mermaid)) { margin-bottom: 14px; }
  .markdown.doc :global(:is(h1, h2)) { padding-bottom: 6px; border-bottom: 1px solid var(--line); }
  .markdown.doc :global(h1) { margin: 24px 0 14px; font-size: 26px; }
  .markdown.doc :global(h2) { margin: 22px 0 12px; font-size: 20px; }
  .markdown.doc :global(h3) { margin: 18px 0 10px; font-size: 16px; }
  .markdown.doc :global(:is(h1, h2, h3, h4, h5, h6):first-child) { margin-top: 0; }
  .markdown.doc :global(code) { font-size: 12.5px; }
</style>
