import { describe, expect, it } from 'bun:test'
import type { LogRow } from './api'
import { chip, lanes, pick } from './log'

const row = (column: number, top: [number, number][] = [], bottom: [number, number][] = []) =>
  ({ column, top: top.map(([from, to]) => ({ from, to, color: 0 })), bottom: bottom.map(([from, to]) => ({ from, to, color: 0 })) }) as LogRow

describe('log', () => {
  it('reads ref labels', () => {
    expect(['HEAD -> main', 'HEAD', 'tag: v1', 'origin/main'].map(chip)).toEqual([
      { kind: 'head', label: 'main' },
      { kind: 'head', label: 'HEAD' },
      { kind: 'tag', label: 'v1' },
      { kind: 'branch', label: 'origin/main' },
    ])
  })

  it('counts the lanes of the dot and the edges crossing a row', () => {
    expect(lanes(row(0))).toBe(1)
    expect(lanes(row(1, [[0, 0]]))).toBe(2)
    expect(lanes(row(0, [[2, 0]], [[0, 0], [0, 3]]))).toBe(4)
  })

  it('selects one, toggles with Cmd and ranges with Shift', () => {
    const rows = ['a', 'b', 'c', 'd'].map((sha) => ({ sha }))
    const one = pick(rows, new Set(), 0, 1, 'one')
    expect([...one]).toEqual(['b'])
    const two = pick(rows, one, 1, 3, 'toggle')
    expect([...two]).toEqual(['b', 'd'])
    expect([...pick(rows, two, 1, 3, 'toggle')]).toEqual(['b'])
    expect([...pick(rows, two, 3, 1, 'range')]).toEqual(['b', 'c', 'd'])
  })
})
