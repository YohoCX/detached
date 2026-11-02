import { describe, expect, it } from 'bun:test'
import { filter, fuzzy, segments } from './fuzzy'

describe('fuzzy', () => {
  it('matches subsequences case-insensitively', () => {
    expect(fuzzy('gtf', 'Go to File')?.indices).toEqual([0, 3, 6])
    expect(fuzzy('xyz', 'Go to File')).toBeNull()
    expect(fuzzy('', 'anything')).toEqual({ score: 0, indices: [] })
  })

  it('ranks tighter matches first and keeps order for an empty query', () => {
    const items = ['feature/login', 'fix-logging', 'main']
    expect(filter(items, 'log', (s) => s).map((m) => m.item)).toEqual(['fix-logging', 'feature/login'])
    expect(filter(items, '', (s) => s).map((m) => m.item)).toEqual(items)
    expect(filter(['a-x-b-c', 'abc'], 'abc', (s) => s)[0].item).toBe('abc')
  })

  it('splits text into highlight runs', () => {
    expect(segments('main', [0, 1])).toEqual([{ text: 'ma', hit: true }, { text: 'in', hit: false }])
  })
})
