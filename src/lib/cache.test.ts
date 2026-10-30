import { describe, expect, it } from 'vitest'
import { cached, refresh } from './cache.svelte'

describe('cache', () => {
  it('keeps the last value on failure and skips a focus refresh within 30 s', async () => {
    let calls = 0
    const ok = async () => ++calls
    expect(await refresh('k', ok)).toBe(1)
    expect(await refresh('k', ok, true)).toBeUndefined()
    expect(await refresh('k', () => Promise.reject('down'))).toBeUndefined()
    expect(cached('k')).toMatchObject({ value: 1, error: 'down' })
    expect(await refresh('k', ok)).toBe(2)
    expect(cached('k')?.error).toBeUndefined()
  })
})
