import { describe, expect, it } from 'vitest'
import type { PullResult } from './api'
import { pool, syncAll, type Outcome } from './sync'

const tick = () => new Promise((r) => setTimeout(r, 1))

describe('pool', () => {
  it('runs at most `limit` at once and keeps input order', async () => {
    let active = 0
    let peak = 0
    const out = await pool([...Array(10).keys()], 4, async (i) => {
      peak = Math.max(peak, ++active)
      await tick()
      active--
      if (i === 3) throw new Error('boom')
      return i * 2
    })
    expect(peak).toBe(4)
    expect(out[3]).toEqual({ status: 'rejected', reason: new Error('boom') })
    expect(out.filter((s) => s.status === 'fulfilled').map((s) => s.value)).toEqual([0, 2, 4, 8, 10, 12, 14, 16, 18])
  })

  it('handles an empty list', async () => {
    expect(await pool([], 4, async () => 1)).toEqual([])
  })
})

describe('syncAll', () => {
  it('reports every repo; one failure does not stop the rest', async () => {
    const results: Record<string, PullResult | Error> = {
      a: { kind: 'upToDate' },
      b: { kind: 'fastForwarded', from: '1111111aaaa', to: '2222222bbbb' },
      c: new Error('fatal: could not read from remote'),
      d: { kind: 'diverged', ahead: 1, behind: 2 },
      e: { kind: 'noUpstream' },
      f: { kind: 'upToDate' },
    }
    let active = 0
    let peak = 0
    const done: Outcome[] = []
    const out = await syncAll(
      Object.keys(results),
      async (repo) => {
        peak = Math.max(peak, ++active)
        await tick()
        active--
        const r = results[repo]
        if (r instanceof Error) throw r.message
        return r
      },
      (o) => done.push(o),
    )
    expect(peak).toBeLessThanOrEqual(4)
    expect(out).toEqual([
      { repo: 'a', ok: true, text: 'Up to date' },
      { repo: 'b', ok: true, text: 'Fast-forwarded 1111111..2222222' },
      { repo: 'c', ok: false, text: 'fatal: could not read from remote' },
      { repo: 'd', ok: false, text: 'Diverged (↑1 ↓2), not updated' },
      { repo: 'e', ok: true, text: 'No upstream' },
      { repo: 'f', ok: true, text: 'Up to date' },
    ])
    expect(done).toHaveLength(6)
  })

  it('reports a fetch as fetched', async () => {
    expect(await syncAll(['a'], async () => {})).toEqual([{ repo: 'a', ok: true, text: 'Fetched' }])
  })
})
