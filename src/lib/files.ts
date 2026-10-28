import type { Content, FileChange, FileEntry } from './api'
import { matches, parseChord, type Key, type KeyChord } from './keymap'

export type Status = 'conflict' | 'untracked' | 'added' | 'deleted' | 'renamed' | 'modified'

export function statusOf(c: FileChange): Status {
  if (c.conflicted) return 'conflict'
  if (c.index === '?') return 'untracked'
  if (c.index === 'A' || c.worktree === 'A') return 'added'
  if (c.index === 'D' || c.worktree === 'D') return 'deleted'
  if (c.index === 'R' || c.index === 'C') return 'renamed'
  return 'modified'
}

export function groupChanges(cs: FileChange[]) {
  const sorted = [...cs].sort((a, b) => a.path.localeCompare(b.path))
  return [
    { title: 'Conflicts', items: sorted.filter((c) => c.conflicted) },
    { title: 'Changes', items: sorted.filter((c) => !c.conflicted && c.index !== '?') },
    { title: 'Unversioned Files', items: sorted.filter((c) => c.index === '?') },
  ].filter((g) => g.items.length)
}

export const baseName = (p: string) => p.slice(p.lastIndexOf('/') + 1)
export const dirName = (p: string) => p.slice(0, Math.max(0, p.lastIndexOf('/')))

export interface Dir { name: string; path: string; dirs: Map<string, Dir>; files: FileEntry[]; dirty: boolean }
export interface Row { path: string; name: string; depth: number; dir: boolean; dirty: boolean; change: FileChange | null }

export function buildTree(entries: FileEntry[]): Dir {
  const root: Dir = { name: '', path: '', dirs: new Map(), files: [], dirty: false }
  for (const e of entries) {
    let d = root
    const parts = e.path.split('/')
    for (const name of parts.slice(0, -1)) {
      if (e.change) d.dirty = true
      let next = d.dirs.get(name)
      if (!next) d.dirs.set(name, (next = { name, path: d.path ? `${d.path}/${name}` : name, dirs: new Map(), files: [], dirty: false }))
      d = next
    }
    if (e.change) d.dirty = true
    d.files.push(e)
  }
  return root
}

const byName = (a: string, b: string) => a.localeCompare(b, undefined, { sensitivity: 'base', numeric: true })

/** Visible rows: directories first, children of expanded directories only. */
export function flatten(root: Dir, expanded: { has(path: string): boolean }): Row[] {
  const out: Row[] = []
  const walk = (d: Dir, depth: number) => {
    for (const sub of [...d.dirs.values()].sort((a, b) => byName(a.name, b.name))) {
      out.push({ path: sub.path, name: sub.name, depth, dir: true, dirty: sub.dirty, change: null })
      if (expanded.has(sub.path)) walk(sub, depth + 1)
    }
    for (const f of [...d.files].sort((a, b) => byName(a.path, b.path)))
      out.push({ path: f.path, name: baseName(f.path), depth, dir: false, dirty: !!f.change, change: f.change })
  }
  walk(root, 0)
  return out
}

/** Text runs for UTF-8 byte `ranges` into `text`, leading indentation dropped. */
export function highlight(text: string, ranges: [number, number][]): { text: string; hit: boolean }[] {
  const at: number[] = []
  let b = 0
  for (let i = 0; i < text.length; ) {
    const cp = text.codePointAt(i)!
    const n = cp < 0x80 ? 1 : cp < 0x800 ? 2 : cp < 0x10000 ? 3 : 4
    for (let k = 0; k < n; k++) at[b + k] = i
    b += n
    i += cp > 0xffff ? 2 : 1
  }
  const idx = (byte: number) => at[byte] ?? text.length
  const out: { text: string; hit: boolean }[] = []
  let pos = text.length - text.trimStart().length
  for (const [s, e] of ranges) {
    const [from, to] = [Math.max(idx(s), pos), idx(e)]
    if (to <= from) continue
    if (from > pos) out.push({ text: text.slice(pos, from), hit: false })
    out.push({ text: text.slice(from, to), hit: true })
    pos = to
  }
  if (pos < text.length) out.push({ text: text.slice(pos), hit: false })
  return out
}

export interface Lang { id: string; extensions?: string[]; filenames?: string[] }
const extra: Record<string, string> = { '.svelte': 'html', '.vue': 'html', '.astro': 'html', '.mjs': 'javascript', '.cjs': 'javascript' }

export function languageFor(path: string, langs: Lang[]): string {
  const name = baseName(path).toLowerCase()
  const dot = name.lastIndexOf('.')
  const ext = dot > 0 ? name.slice(dot) : ''
  return (
    langs.find((l) => l.filenames?.some((f) => f.toLowerCase() === name))?.id ??
    (ext ? (extra[ext] ?? langs.find((l) => l.extensions?.includes(ext))?.id) : undefined) ??
    'plaintext'
  )
}

export function describe(c: Content): string {
  switch (c.kind) {
    case 'binary': return 'Binary file'
    case 'tooLarge': return `File too large to show (${(c.size / 1048576).toFixed(1)} MB)`
    case 'missing': return 'No file here'
    case 'submodule': return `Submodule at ${c.commit.slice(0, 10)}`
    default: return 'Text'
  }
}

/** Monaco's own bindings on chords docs/KEYMAP.md gives to the app; Cmd+K starts its chords. */
const monacoClashes = ['Cmd+K', 'Cmd+D', 'Cmd+L', 'Cmd+E', 'Cmd+[', 'Cmd+]', 'F7', 'Shift+F7', 'Cmd+Shift+O', 'Cmd+Shift+F', 'Ctrl+V']
  .map((s) => parseChord(s) as KeyChord)

export const clashesWithMonaco = (e: Key) => monacoClashes.some((c) => matches(c, e))
