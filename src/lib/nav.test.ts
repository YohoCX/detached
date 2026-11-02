import { describe, expect, it, mock } from 'bun:test'
import { back, forward, Nav, nav, open, setOpener, type Location } from './nav'

const loc = (path: string, line: number | null = null, repo = '/r'): Location => ({ repo, ref: null, path, line })

describe('Nav', () => {
  it('keeps the last 50 recent files, most recent first, deduped', () => {
    const n = new Nav()
    for (let i = 0; i < 60; i++) n.visit(loc(`f${i}`))
    n.visit(loc('f55', 3))
    expect(n.recent).toHaveLength(50)
    expect(n.recent[0]).toEqual({ repo: '/r', ref: null, path: 'f55' })
    expect(n.recent.filter((r) => r.path === 'f55')).toHaveLength(1)
    expect(n.recent[1].path).toBe('f59')
    n.visit({ ...loc('f59'), ref: 'main' })
    expect(n.recent.slice(0, 2).map((r) => r.ref)).toEqual(['main', null])
  })

  it('caps history at 100', () => {
    const n = new Nav()
    for (let i = 0; i < 120; i++) n.visit(loc(`f${i}`))
    expect(n.history).toHaveLength(100)
    expect(n.history[0].path).toBe('f20')
    expect(n.index).toBe(99)
  })

  it('walks back and forward, and a new visit drops the forward entries', () => {
    const n = new Nav()
    n.visit(loc('a'))
    n.visit(loc('b'))
    n.visit(loc('c'))
    expect(n.back()?.path).toBe('b')
    expect(n.back()?.path).toBe('a')
    expect(n.back()).toBeNull()
    expect(n.forward()?.path).toBe('b')
    n.visit(loc('d'))
    expect(n.forward()).toBeNull()
    expect(n.history.map((l) => l.path)).toEqual(['a', 'b', 'd'])
  })

  it('does not push the current location twice, but a new line counts', () => {
    const n = new Nav()
    n.visit(loc('a', 1))
    n.visit(loc('a', 1))
    n.visit(loc('a', 9))
    expect(n.history.map((l) => l.line)).toEqual([1, 9])
  })
})

describe('open hook', () => {
  it('records on open and replays on back/forward', () => {
    const opened: Location[] = []
    setOpener((l) => opened.push(l))
    open(loc('x'))
    open(loc('y', 4))
    back()
    forward()
    forward()
    expect(opened.map((l) => l.path)).toEqual(['x', 'y', 'x', 'y'])
    expect(nav.recent[0].path).toBe('y')
    setOpener(mock())
  })
})
