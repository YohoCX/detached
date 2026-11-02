import { describe, expect, it } from 'bun:test'
import { took } from './pipelines'

describe('pipelines', () => {
  it('formats durations', () => {
    expect([null, 0, 59, 65, 3600 + 125].map(took)).toEqual(['', '0s', '59s', '1m 05s', '1h 02m'])
  })
})
