import { describe, expect, it } from 'vitest'
import { actions, register, runAction } from './actions'

describe('actions', () => {
  it('runs a registered action and unregisters it', () => {
    let ran = 0
    const off = register('file.goto', () => ran++)
    expect(runAction('file.goto')).toBe(true)
    off()
    expect(runAction('file.goto')).toBe(false)
    expect(ran).toBe(1)
  })

  it('rejects unknown ids', () => {
    expect(() => register('nope', () => {})).toThrow()
    expect(actions.has('search.everywhere')).toBe(true)
  })
})
