<script lang="ts">
  import { onMount, tick, untrack } from 'svelte'
  import { register } from '../lib/actions'
  import { api, type Content, type DiffSide } from '../lib/api'
  import { nameOf, say, statuses, statusOf } from '../lib/app.svelte'
  import { baseName, clashesWithMonaco, describe, dirName, languageFor } from '../lib/files'
  import { chordFor } from '../lib/keymap'
  import { isMarkdown } from '../lib/markdown'
  import { monaco } from '../lib/monaco'
  import { otherLine } from '../lib/mr'
  import { nav } from '../lib/nav'
  import { load, save } from '../lib/storage'
  import { showDiff, sideName, view, type FileDoc, type Zone } from '../lib/view.svelte'
  import Icon from './Icon.svelte'
  import Markdown from './Markdown.svelte'

  type Editor = monaco.editor.IStandaloneCodeEditor
  type Model = monaco.editor.ITextModel
  // Without `doc` this shows `view.shown`. With `ongutter`, a `+` in the hovered line's gutter, where
  // `commentable` allows, calls it with the other side's line when the line is unchanged.
  let { doc, sideBySide: forceSideBySide = false, zones = [], ongutter, commentable }: {
    doc?: FileDoc
    sideBySide?: boolean
    zones?: Zone[]
    ongutter?: (side: DiffSide, line: number, other: number | null) => void
    commentable?: (side: DiffSide, line: number) => boolean
  } = $props()

  let mode = $state<'empty' | 'editor' | 'diff' | 'placeholder'>('empty')
  let message = $state('')
  let conflicted = $state(false)
  let sideBySide = $state(load('viewer.sideBySide') !== 'false')
  let goto = $state<string | null>(null)
  let markdown = $state('')
  let preview = $state(false)
  let editorHost: HTMLDivElement
  let diffHost: HTMLDivElement
  let gotoInput = $state<HTMLInputElement>()
  let editor: Editor
  let diff: monaco.editor.IStandaloneDiffEditor
  let model: Model | null = null
  let pair: { original: Model; modified: Model } | null = null
  let key = ''
  let seq = 0
  let loaded = $state(0)
  let focusPending = false
  const shown = $derived(doc ?? (view.shown?.kind === 'file' || view.shown?.kind === 'diff' ? view.shown : null))
  const split = $derived(forceSideBySide || sideBySide)
  const md = $derived(shown?.kind === 'file' && isMarkdown(shown.path) && mode === 'editor' && !conflicted ? shown : null)

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
    loaded++
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

  async function show(s: FileDoc, reveal: boolean) {
    const id = ++seq
    try {
      if (s.kind === 'file') {
        const c = await api.readFile(s.repo, s.ref, s.path)
        if (id !== seq) return
        if (c.kind !== 'text' && c.kind !== 'conflicted') return placeholder(describe(c))
        setFile(`${s.repo}\0${s.ref}\0${s.path}`, c.text, s.path)
        conflicted = c.kind === 'conflicted'
        markdown = c.text
        if (reveal) {
          // A file opened at a line shows it in the source.
          preview = !s.line && load('viewer.mdPreview') !== 'false'
          revealLine(editor, s.line ?? 1)
        }
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
    preview = false
    goto = ''
    tick().then(() => gotoInput?.focus())
  }

  function gotoLine() {
    const [line, column] = (goto ?? '').split(':').map((n) => parseInt(n, 10))
    goto = null
    if (!(line > 0)) return active().focus()
    revealLine(active(), line, column > 0 ? column : 1)
    const s = shown
    if (s?.kind === 'file') nav.visit({ repo: s.repo, ref: s.ref, path: s.path, line })
  }

  function zed() {
    const s = shown
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

  function togglePreview(on: boolean) {
    preview = on
    save('viewer.mdPreview', String(on))
  }

  function toggleSideBySide(on: boolean) {
    sideBySide = on
    save('viewer.sideBySide', String(on))
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
      theme: dark.matches ? 'wb-dark' : 'wb-light',
      glyphMargin: !!untrack(() => ongutter),
    }
    editor = monaco.editor.create(editorHost, common)
    diff = monaco.editor.createDiffEditor(diffHost, {
      ...common,
      originalEditable: false,
      renderSideBySide: split,
      useInlineViewWhenSpaceIsLimited: false,
      ignoreTrimWhitespace: false,
    })
    const onTheme = () => monaco.editor.setTheme(dark.matches ? 'wb-dark' : 'wb-light')
    dark.addEventListener('change', onTheme)
    const sides = [['old', diff.getOriginalEditor()], ['new', diff.getModifiedEditor()]] as const
    const T = monaco.editor.MouseTargetType
    const gutter = [T.GUTTER_GLYPH_MARGIN, T.GUTTER_LINE_NUMBERS]
    const can = (side: DiffSide, line: number | undefined): line is number =>
      !!ongutter && !!line && (!commentable || commentable(side, line))
    const offs = [
      register('diff.next', () => mode === 'diff' && diff.goToDiff('next')),
      register('diff.prev', () => mode === 'diff' && diff.goToDiff('previous')),
      register('editor.gotoLine', openGoto),
      register('markdown.preview', () => md && togglePreview(!preview)),
      ...(untrack(() => doc)
        ? []
        : [register('diff.show', diffAction), register('file.openInZed', zed), register('find.inFiles', () => (view.find = true))]),
      ...sides.map(([side, ed]) => {
        const plus = ed.createDecorationsCollection()
        const hover = (line?: number) =>
          plus.set(can(side, line) ? [{ range: new monaco.Range(line, 1, line, 1), options: { glyphMarginClassName: 'mr-plus' } }] : [])
        const subs = [
          ed.onMouseMove((e) => hover(e.target.type === T.CONTENT_VIEW_ZONE ? undefined : e.target.position?.lineNumber)),
          ed.onMouseLeave(() => hover()),
          ed.onMouseDown((e) => {
            const line = e.target.position?.lineNumber
            if (can(side, line) && gutter.includes(e.target.type)) ongutter!(side, line, otherLine(diff.getLineChanges() ?? [], side, line))
          }),
        ]
        return () => subs.forEach((s) => s.dispose())
      }),
    ]
    return () => {
      offs.forEach((off) => off())
      dark.removeEventListener('change', onTheme)
      dropModels()
      editor.dispose()
      diff.dispose()
    }
  })

  $effect(() => {
    if (view.focusDiff) focusPending = true
  })

  $effect(() => diff.updateOptions({ renderSideBySide: split }))

  // Re-added whenever the diff reloads; each zone is as wide as the visible text and as tall as its node.
  $effect(() => {
    if (mode !== 'diff' || !zones.length) return
    void loaded
    const removers = zones.map((z) => {
      const ed = z.side === 'old' ? diff.getOriginalEditor() : diff.getModifiedEditor()
      const domNode = document.createElement('div')
      // Above the text layer, which otherwise takes the zone's clicks; Monaco would take focus on them.
      domNode.style.zIndex = '1'
      domNode.append(z.node)
      for (const t of ['mousedown', 'pointerdown', 'keydown', 'contextmenu']) domNode.addEventListener(t, (e) => e.stopPropagation())
      const zone = { afterLineNumber: z.afterLine, heightInPx: 80, domNode }
      let id = ''
      ed.changeViewZones((a) => (id = a.addZone(zone)))
      const fit = () => {
        const l = ed.getLayoutInfo()
        z.node.style.width = `${l.contentWidth - l.verticalScrollbarWidth}px`
      }
      // Keeps the content's margins inside the measured height.
      z.node.style.display = 'flow-root'
      fit()
      const layout = ed.onDidLayoutChange(fit)
      // Focus inside the zone can scroll Monaco's overflow-hidden layers natively, shifting the text.
      const unscroll = (e: Event) => {
        const t = e.target as Element
        if (t.scrollLeft && t !== domNode && t.contains(domNode)) t.scrollLeft = 0
      }
      ed.getDomNode()?.addEventListener('scroll', unscroll, true)
      const ro = new ResizeObserver(() => {
        if (!z.node.offsetHeight || z.node.offsetHeight === zone.heightInPx) return
        zone.heightInPx = z.node.offsetHeight
        ed.changeViewZones((a) => a.layoutZone(id))
      })
      ro.observe(z.node)
      return () => {
        ro.disconnect()
        layout.dispose()
        ed.getDomNode()?.removeEventListener('scroll', unscroll, true)
        ed.changeViewZones((a) => a.removeZone(id))
      }
    })
    return () => removers.forEach((remove) => remove())
  })

  let last: FileDoc | null = null
  $effect(() => {
    const s = shown
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
      </span>
      {#if shown.kind === 'diff'}<span class="pill ellipsis">{sideName(shown.left)} ↔ {sideName(shown.right)}</span>{/if}
      <span class="spacer"></span>
      {#if mode === 'diff'}
        <button class="icon-btn" title="Previous change {chordFor('diff.prev')}" onclick={() => diff.goToDiff('previous')}><Icon name="up" /></button>
        <button class="icon-btn" title="Next change {chordFor('diff.next')}" onclick={() => diff.goToDiff('next')}><Icon name="next" /></button>
        {#if !forceSideBySide}
          <div class="seg">
            <button class:on={sideBySide} onclick={() => toggleSideBySide(true)}>Side-by-side</button>
            <button class:on={!sideBySide} onclick={() => toggleSideBySide(false)}>Unified</button>
          </div>
        {/if}
      {/if}
      {#if md}
        <div class="seg">
          <button class:on={!preview} onclick={() => togglePreview(false)}>Source</button>
          <button class:on={preview} onclick={() => togglePreview(true)}>Preview</button>
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
    <div class="host" class:hidden={mode !== 'editor' || (md && preview)} bind:this={editorHost}></div>
    {#if md && preview}
      <div class="preview"><Markdown text={markdown} file={md} /></div>
    {/if}
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
  header { display: flex; align-items: center; gap: 8px; height: 44px; padding: 0 8px 0 16px; border-bottom: 1px solid var(--line); flex: none; }
  .name { font-size: 14px; font-weight: 800; white-space: nowrap; }
  .dir { min-width: 0; }
  .pill { flex: none; max-width: 40%; font-size: 12px; color: var(--muted); background: var(--hover); border-radius: 999px; padding: 2px 10px; }
  .spacer { flex: 1; }
  .bar { display: flex; align-items: center; gap: 8px; padding: 6px 16px; background: color-mix(in srgb, var(--conflict) 14%, transparent); border-bottom: 1px solid var(--line); }
  .editors { position: relative; flex: 1; min-height: 0; }
  .host { position: absolute; inset: 0; }
  .hidden { display: none; }
  .preview { position: absolute; inset: 0; overflow: auto; background: var(--code); }
  .placeholder { position: absolute; inset: 0; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 8px; }
  .host :global(.mr-plus) { cursor: pointer; }
  .host :global(.mr-plus)::before { content: '+'; display: grid; place-items: center; width: 16px; height: 16px; margin: 1px 0 0 2px; border-radius: 5px; background: var(--accent); color: var(--accent-fg); font: 700 13px/1 var(--font); }
  .goto { position: absolute; top: 10px; left: 50%; transform: translateX(-50%); z-index: 5; width: 220px; box-shadow: var(--shadow); }
</style>
