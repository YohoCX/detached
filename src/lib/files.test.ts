import { describe, expect, it } from 'bun:test'
import type { FileChange } from './api'
import { buildTree, clashesWithMonaco, flatten, groupChanges, highlight, languageFor, statusOf } from './files'

const change = (path: string, index: string, worktree: string, conflicted = false): FileChange =>
  ({ path, origPath: null, index, worktree, conflicted })

describe('changes', () => {
  it('groups conflicts, changes and unversioned files', () => {
    const g = groupChanges([change('b', '.', 'M'), change('u', '?', '?'), change('c', 'U', 'U', true), change('a', 'A', '.')])
    expect(g.map((x) => [x.title, x.items.map((c) => c.path)])).toEqual([
      ['Conflicts', ['c']],
      ['Changes', ['a', 'b']],
      ['Unversioned Files', ['u']],
    ])
  })

  it('names the status', () => {
    expect([change('a', 'A', 'M'), change('d', '.', 'D'), change('r', 'R', '.'), change('m', 'M', 'M')].map(statusOf))
      .toEqual(['added', 'deleted', 'renamed', 'modified'])
  })
})

describe('tree', () => {
  const entries = [
    { path: 'README.md', change: null },
    { path: 'src/b.ts', change: change('src/b.ts', '.', 'M') },
    { path: 'src/lib/a.ts', change: null },
    { path: 'Cargo.toml', change: null },
  ]

  it('lists directories first and only expanded children', () => {
    const tree = buildTree(entries)
    expect(flatten(tree, new Set()).map((r) => r.path)).toEqual(['src', 'Cargo.toml', 'README.md'])
    const rows = flatten(tree, new Set(['src', 'src/lib']))
    expect(rows.map((r) => [r.path, r.depth])).toEqual([
      ['src', 0], ['src/lib', 1], ['src/lib/a.ts', 2], ['src/b.ts', 1], ['Cargo.toml', 0], ['README.md', 0],
    ])
    expect(rows.filter((r) => r.dirty).map((r) => r.path)).toEqual(['src', 'src/b.ts'])
  })
})

describe('highlight', () => {
  it('maps byte ranges to characters and drops indentation', () => {
    expect(highlight('  let x = y', [[6, 7]])).toEqual([{ text: 'let ', hit: false }, { text: 'x', hit: true }, { text: ' = y', hit: false }])
    // é is 2 bytes, 😀 is 4 bytes and 2 UTF-16 units.
    expect(highlight('é😀ab', [[6, 7]])).toEqual([{ text: 'é😀', hit: false }, { text: 'a', hit: true }, { text: 'b', hit: false }])
    expect(highlight('ab', [])).toEqual([{ text: 'ab', hit: false }])
  })
})

describe('languageFor', () => {
  const langs = [
    { id: 'typescript', extensions: ['.ts', '.tsx'] },
    { id: 'dockerfile', extensions: ['.dockerfile'], filenames: ['Dockerfile'] },
  ]
  it('uses file names, then extensions, then plaintext', () => {
    expect(languageFor('src/App.ts', langs)).toBe('typescript')
    expect(languageFor('docker/Dockerfile', langs)).toBe('dockerfile')
    expect(languageFor('App.svelte', langs)).toBe('html')
    expect(languageFor('.env', langs)).toBe('plaintext')
  })
})

describe('clashesWithMonaco', () => {
  const key = (code: string, mods: Partial<Record<'metaKey' | 'shiftKey' | 'ctrlKey' | 'altKey', boolean>> = {}) =>
    ({ key: '', code, metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, repeat: false, ...mods })
  it('catches the reserved chords only', () => {
    expect(clashesWithMonaco(key('KeyK', { metaKey: true }))).toBe(true)
    expect(clashesWithMonaco(key('F7', { shiftKey: true }))).toBe(true)
    expect(clashesWithMonaco(key('KeyF', { metaKey: true }))).toBe(false)
    expect(clashesWithMonaco(key('KeyC', { metaKey: true }))).toBe(false)
  })
})
