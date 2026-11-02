import { afterEach, describe, expect, it } from 'bun:test'
import { actions } from './actions'
import { bindings, defaults, DoubleShift, formatChord, install, matches, parseChord, reserved, setOverrides, type Key, type KeyChord } from './keymap'

const key = (code: string, k: string, mods: Partial<Key> = {}): Key => ({
  code, key: k, metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, repeat: false, ...mods,
})
const shift = (mods: Partial<Key> = {}) => key('ShiftLeft', 'Shift', { shiftKey: true, ...mods })
const chord = (s: string) => parseChord(s) as KeyChord

describe('parseChord', () => {
  it('parses modifiers and keys to codes', () => {
    expect(parseChord('Cmd+Shift+O')).toEqual({ meta: true, ctrl: false, alt: false, shift: true, code: 'KeyO', key: 'O' })
    expect(chord('Ctrl+V')).toMatchObject({ ctrl: true, meta: false, code: 'KeyV' })
    expect(chord('F7')).toMatchObject({ shift: false, code: 'F7' })
    expect(chord('Shift+F7')).toMatchObject({ shift: true, code: 'F7' })
    expect(chord('Cmd+[').code).toBe('BracketLeft')
    expect(chord('Cmd+]').code).toBe('BracketRight')
    expect(chord('cmd+alt+z')).toMatchObject({ meta: true, alt: true, code: 'KeyZ' })
    expect(chord('Cmd+1').code).toBe('Digit1')
    expect(parseChord('Double Shift')).toBe('doubleShift')
  })

  it('rejects invalid chords', () => {
    for (const s of ['', 'Cmd+', 'Hyper+X', 'Cmd+Shift', 'F25', 'Cmd+Foo']) expect(parseChord(s)).toBeNull()
  })

  it('formats with macOS symbols', () => {
    expect(formatChord(parseChord('Cmd+Shift+O'))).toBe('⇧⌘O')
    expect(formatChord(parseChord('Ctrl+V'))).toBe('⌃V')
    expect(formatChord(parseChord('Cmd+['))).toBe('⌘[')
    expect(formatChord(parseChord('Double Shift'))).toBe('⇧⇧')
  })
})

describe('matches', () => {
  it('uses event.code, so other layouts match', () => {
    expect(matches(chord('Cmd+['), key('BracketLeft', '[', { metaKey: true }))).toBe(true)
    expect(matches(chord('Cmd+['), key('BracketLeft', 'х', { metaKey: true }))).toBe(true)
    expect(matches(chord('Cmd+Shift+O'), key('KeyO', 'Щ', { metaKey: true, shiftKey: true }))).toBe(true)
    expect(matches(chord('Cmd+E'), key('KeyR', 'e', { metaKey: true }))).toBe(false)
  })

  it('requires exact modifiers', () => {
    expect(matches(chord('F7'), key('F7', 'F7'))).toBe(true)
    expect(matches(chord('F7'), key('F7', 'F7', { shiftKey: true }))).toBe(false)
    expect(matches(chord('Shift+F7'), key('F7', 'F7', { shiftKey: true }))).toBe(true)
    expect(matches(chord('Cmd+Shift+O'), key('KeyO', 'o', { metaKey: true }))).toBe(false)
    expect(matches(chord('Ctrl+V'), key('KeyV', 'v', { metaKey: true }))).toBe(false)
  })
})

describe('bindings', () => {
  it('binds every default to a registry action and keeps M2 chords free', () => {
    const b = bindings()
    for (const id of Object.keys(defaults)) {
      expect(actions.has(id), id).toBe(true)
      expect(b.has(id), id).toBe(true)
    }
    const used = new Set(Object.values(defaults))
    for (const [id, s] of Object.entries(reserved)) {
      expect(actions.has(id), id).toBe(false)
      expect(used.has(s), s).toBe(false)
    }
  })

  it('applies overrides; an empty or invalid override unbinds', () => {
    const b = bindings({ 'file.goto': 'Cmd+P', 'nav.back': '', 'diff.next': 'nonsense' })
    expect(matches(b.get('file.goto') as KeyChord, key('KeyP', 'p', { metaKey: true }))).toBe(true)
    expect(b.has('nav.back')).toBe(false)
    expect(b.has('diff.next')).toBe(false)
    expect(b.get('nav.forward')).toEqual(parseChord('Cmd+]')!)
  })
})

describe('DoubleShift', () => {
  it('fires on two Shift keydowns within 300 ms', () => {
    const d = new DoubleShift()
    expect(d.down(shift(), 1000)).toBe(false)
    expect(d.down(shift(), 1250)).toBe(true)
  })

  it('does not fire when too slow', () => {
    const d = new DoubleShift()
    d.down(shift(), 1000)
    expect(d.down(shift(), 1301)).toBe(false)
    expect(d.down(shift(), 1500)).toBe(true)
  })

  it('is interrupted by another key or a modifier chord', () => {
    const d = new DoubleShift()
    d.down(shift(), 1000)
    d.down(key('KeyA', 'A', { shiftKey: true }), 1050)
    expect(d.down(shift(), 1100)).toBe(false)
    expect(d.down(shift({ metaKey: true }), 1150)).toBe(false)
    expect(d.down(shift(), 1200)).toBe(false)
  })

  it('ignores auto-repeat and needs a fresh pair after firing', () => {
    const d = new DoubleShift()
    d.down(shift(), 1000)
    expect(d.down(shift({ repeat: true }), 1100)).toBe(false)
    expect(d.down(shift(), 1150)).toBe(true)
    expect(d.down(shift(), 1200)).toBe(false)
    expect(d.down(shift(), 1250)).toBe(true)
  })
})

describe('install', () => {
  const realWindow = globalThis.window
  afterEach(() => {
    Object.assign(globalThis, { window: realWindow })
    setOverrides({})
  })

  function setup() {
    let listener: (e: KeyboardEvent) => void = () => {}
    Object.assign(globalThis, { window: {
      addEventListener: (_: string, f: typeof listener, capture: boolean) => {
        expect(capture).toBe(true)
        listener = f
      },
      removeEventListener: () => {},
    } })
    const ran: string[] = []
    install((id) => ran.push(id))
    const press = (k: Key, timeStamp = 0) => {
      const e = { ...k, timeStamp, prevented: false, preventDefault() { e.prevented = true }, stopPropagation() {} }
      listener(e as unknown as KeyboardEvent)
      return e.prevented
    }
    return { ran, press }
  }

  it('dispatches chords, including actions another component owns', () => {
    const { ran, press } = setup()
    expect(press(key('F7', 'F7', { shiftKey: true }))).toBe(true)
    expect(press(key('KeyL', 'l', { metaKey: true }))).toBe(true)
    expect(press(key('KeyX', 'x'))).toBe(false)
    press(shift(), 10)
    press(shift(), 200)
    expect(ran).toEqual(['diff.prev', 'editor.gotoLine', 'search.everywhere'])
  })

  it('uses overrides', () => {
    const { ran, press } = setup()
    setOverrides({ 'file.goto': 'Cmd+P' })
    press(key('KeyO', 'o', { metaKey: true, shiftKey: true }))
    press(key('KeyP', 'p', { metaKey: true }))
    expect(ran).toEqual(['file.goto'])
  })
})
