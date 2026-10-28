<script lang="ts">
  import { onMount, tick, untrack } from 'svelte'
  import { register } from '../lib/actions'
  import { api, type Content } from '../lib/api'
  import { nameOf, say, statuses, statusOf } from '../lib/app.svelte'
  import { baseName, clashesWithMonaco, describe, dirName, languageFor } from '../lib/files'
  import { chordFor } from '../lib/keymap'
  import { monaco } from '../lib/monaco'
  import { nav } from '../lib/nav'
  import { showDiff, sideName, view, type Shown } from '../lib/view.svelte'
  import Icon from './Icon.svelte'

  type Editor = monaco.editor.IStandaloneCodeEditor
  type Model = monaco.editor.ITextModel

  let mode = $state<'empty' | 'editor' | 'diff' | 'placeholder'>('empty')
  let message = $state('')
  let conflicted = $state(false)
  let sideBySide = $state(load('viewer.sideBySide') !== 'false')
  let goto = $state<string | null>(null)
  let editorHost: HTMLDivElement
  let diffHost: HTMLDivElement
  let gotoInput = $state<HTMLInputElement>()
  let editor: Editor
  let diff: monaco.editor.IStandaloneDiffEditor
  let model: Model | null = null
  let pair: { original: Model; modified: Model } | null = null
  let key = ''
  let seq = 0
  const shown = $derived(view.shown)

  function load(k: string) {
    try {
      return localStorage.getItem(k)
    } catch {
      return null
    }
  }

  const active = (): Editor => (mode === 'diff' ? diff.getModifiedEditor() : editor)
  const lang = (path: string) => languageFor(path, monaco.languages.getLanguages())
  const textOf = (c: Content) => (c.kind === 'text' || c.kind === 'conflicted' ? c.text : c.kind === 'missing' ? '' : null)

  function dropModels() {
    editor.setModel(null)
    diff.setModel(null)
    model?.dispose()
    pair?.original.dispose()
    pair?.modified.dispose()
    model = pair = null
    key = ''
  }

  const update = (m: Model, text: string) => m.getValue() !== text && m.setValue(text)

  function setFile(k: string, text: string, path: string) {
    if (k === key && model) update(model, text)
    else {
      dropModels()
      model = monaco.editor.createModel(text, lang(path))
      editor.setModel(model)
      key = k
    }
    mode = 'editor'
  }

  function setDiff(k: string, a: string, b: string, path: string, reveal: boolean) {
    if (k === key && pair) {
      update(pair.original, a)
      update(pair.modified, b)
    } else {
      dropModels()
      pair = { original: monaco.editor.createModel(a, lang(path)), modified: monaco.editor.createModel(b, lang(path)) }
      diff.setModel(pair)
      key = k
    }
    mode = 'diff'
    if (focusPending) {
      focusPending = false
      diff.getModifiedEditor().focus()
    }
    if (reveal) {
      const sub = diff.onDidUpdateDiff(() => {
        sub.dispose()
        diff.revealFirstDiff()
      })
    }
  }

  function placeholder(text: string) {
    dropModels()
    message = text
    mode = 'placeholder'
    conflicted = false
  }

  function revealLine(ed: Editor, line: number, column = 1) {
    ed.setPosition({ lineNumber: line, column })
    ed.revealLineInCenter(line)
    ed.focus()
  }

  async function show(s: Shown, reveal: boolean) {
    const id = ++seq
    try {
      if (s.kind === 'file') {
        const c = await api.readFile(s.repo, s.ref, s.path)
        if (id !== seq) return
        if (c.kind !== 'text' && c.kind !== 'conflicted') return placeholder(describe(c))
        setFile(`${s.repo}\0${s.ref}\0${s.path}`, c.text, s.path)
        conflicted = c.kind === 'conflicted'
        if (reveal) revealLine(editor, s.line ?? 1)
        return
      }
      const d = await api.fileDiff(s.repo, s.path, s.origPath, s.left, s.right)
      if (id !== seq) return
      const k = `${s.repo}\0${s.path}\0${JSON.stringify([s.left, s.right])}`
      const [a, b] = [textOf(d.left), textOf(d.right)]
      conflicted = d.right.kind === 'conflicted'
      if (d.right.kind === 'conflicted') setFile(k, d.right.text, s.path)
      else if (a === null || b === null || (d.left.kind === 'missing' && d.right.kind === 'missing'))
        placeholder(`${sideName(s.left)}: ${describe(d.left)}. ${sideName(s.right)}: ${describe(d.right)}.`)
      else setDiff(k, a, b, s.path, reveal)
    } catch (e) {
      if (id === seq) placeholder(String(e))
    }
  }

  function openGoto() {
    if (mode !== 'editor' && mode !== 'diff') return
    goto = ''
    tick().then(() => gotoInput?.focus())
  }

  function gotoLine() {
    const [line, column] = (goto ?? '').split(':').map((n) => parseInt(n, 10))
    goto = null
    if (!(line > 0)) return active().focus()
    revealLine(active(), line, column > 0 ? column : 1)
    const s = view.shown
    if (s?.kind === 'file') nav.visit({ repo: s.repo, ref: s.ref, path: s.path, line })
  }

  function zed() {
    const s = view.shown
    if (!s) return
    const line = mode === 'editor' || mode === 'diff' ? (active().getPosition()?.lineNumber ?? null) : null
    api.openInZed(s.repo, s.path, line).catch((e) => say(String(e), true))
  }

  /** The selected change when Changes has focus or no file is open, else the open file against HEAD or its ref. */
  function diffAction() {
    const s = view.shown
    const inChanges = document.activeElement?.closest('[data-changes]')
    if (view.change && (inChanges || s?.kind !== 'file'))
      return showDiff(view.change.repo, view.change.change.path, view.change.change.origPath)
    if (s?.kind !== 'file') return
    if (s.ref) return showDiff(s.repo, s.path, null, { kind: 'rev', rev: s.ref })
    showDiff(s.repo, s.path, statusOf(s.repo)?.changes.find((c) => c.path === s.path)?.origPath ?? null)
  }

  function toggleSideBySide(on: boolean) {
    sideBySide = on
    diff.updateOptions({ renderSideBySide: on })
    try {
      localStorage.setItem('viewer.sideBySide', String(on))
    } catch {}
  }

  onMount(() => {
    const dark = matchMedia('(prefers-color-scheme: dark)')
    const common = {
      readOnly: true,
      domReadOnly: true,
      automaticLayout: true,
      minimap: { enabled: false },
      scrollBeyondLastLine: false,
      fontFamily: getComputedStyle(document.documentElement).getPropertyValue('--mono'),
      fontSize: 13,
      theme: dark.matches ? 'vs-dark' : 'vs',
    }
    editor = monaco.editor.create(editorHost, common)
    diff = monaco.editor.createDiffEditor(diffHost, {
      ...common,
      originalEditable: false,
      renderSideBySide: sideBySide,
      useInlineViewWhenSpaceIsLimited: false,
      ignoreTrimWhitespace: false,
    })
    const onTheme = () => monaco.editor.setTheme(dark.matches ? 'vs-dark' : 'vs')
    dark.addEventListener('change', onTheme)
    const offs = [
      register('diff.next', () => mode === 'diff' && diff.goToDiff('next')),
      register('diff.prev', () => mode === 'diff' && diff.goToDiff('previous')),
      register('diff.show', diffAction),
      register('editor.gotoLine', openGoto),
      register('file.openInZed', zed),
      register('find.inFiles', () => (view.find = true)),
    ]
    return () => {
      offs.forEach((off) => off())
      dark.removeEventListener('change', onTheme)
      dropModels()
      editor.dispose()
      diff.dispose()
    }
  })

  let focusPending = false
  $effect(() => {
    if (view.focusDiff) focusPending = true
  })

  let last: Shown | null = null
  $effect(() => {
    const s = view.shown
    if (!s) return
    void statuses.get(s.repo)
    const fresh = s !== last
    last = s
    untrack(() => show(s, fresh))
  })
</script>

<div class="viewer">
  {#if shown}
    <header>
      <span class="name">{baseName(shown.path)}</span>
      <span class="muted ellipsis dir">{dirName(shown.path)}</span>
      <span class="muted ellipsis">
        {nameOf(shown.repo)}
        {#if shown.kind === 'file' && shown.ref} @ {shown.ref}{/if}
        {#if shown.kind === 'diff'} · {sideName(shown.left)} ↔ {sideName(shown.right)}{/if}
      </span>
      <span class="spacer"></span>
      {#if mode === 'diff'}
        <button class="icon-btn" title="Previous change {chordFor('diff.prev')}" onclick={() => diff.goToDiff('previous')}>↑</button>
        <button class="icon-btn" title="Next change {chordFor('diff.next')}" onclick={() => diff.goToDiff('next')}>↓</button>
        <div class="seg">
          <button class:on={sideBySide} onclick={() => toggleSideBySide(true)}>Side-by-side</button>
          <button class:on={!sideBySide} onclick={() => toggleSideBySide(false)}>Unified</button>
        </div>
      {/if}
      <button class="icon-btn" title="Open in Zed" onclick={zed}><Icon name="zed" /></button>
    </header>
  {/if}
  {#if conflicted && mode === 'editor'}
    <div class="bar">This file has merge conflicts. <button class="btn" onclick={zed}>Open in Zed</button></div>
  {/if}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="editors" onkeydowncapture={(e) => clashesWithMonaco(e) && e.stopPropagation()}>
    <div class="host" class:hidden={mode !== 'editor'} bind:this={editorHost}></div>
    <div class="host" class:hidden={mode !== 'diff'} bind:this={diffHost}></div>
    {#if mode === 'placeholder'}
      <div class="placeholder muted">{message}</div>
    {:else if mode === 'empty'}
      <div class="placeholder muted">
        <div>Go to file <span class="mono">{chordFor('file.goto')}</span></div>
        <div>Search everywhere <span class="mono">{chordFor('search.everywhere')}</span></div>
        <div>Find in files <span class="mono">{chordFor('find.inFiles')}</span></div>
      </div>
    {/if}
    {#if goto !== null}
      <input class="goto" bind:this={gotoInput} bind:value={goto} placeholder="Line[:column]" onblur={() => (goto = null)}
        onkeydown={(e) => {
          if (e.key === 'Enter') gotoLine()
          else if (e.key === 'Escape') {
            goto = null
            active().focus()
          }
        }} />
    {/if}
  </div>
</div>

<style>
  .viewer { display: flex; flex-direction: column; height: 100%; }
  header { display: flex; align-items: center; gap: 6px; height: 32px; padding: 0 8px; border-bottom: 1px solid var(--line); flex: none; }
  .name { font-weight: 600; white-space: nowrap; }
  .dir { min-width: 0; }
  .spacer { flex: 1; }
  .seg { display: flex; border: 1px solid var(--line); border-radius: 4px; overflow: hidden; }
  .seg button { border: 0; background: none; padding: 2px 8px; font-size: 12px; }
  .seg button.on { background: var(--selected); }
  .bar { display: flex; align-items: center; gap: 8px; padding: 4px 10px; background: color-mix(in srgb, var(--conflict) 12%, var(--surface)); border-bottom: 1px solid var(--line); }
  .bar .btn { height: 22px; }
  .editors { position: relative; flex: 1; min-height: 0; }
  .host { position: absolute; inset: 0; }
  .hidden { display: none; }
  .placeholder { position: absolute; inset: 0; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 6px; }
  .goto { position: absolute; top: 8px; left: 50%; transform: translateX(-50%); z-index: 5; width: 220px; box-shadow: var(--shadow); }
</style>
