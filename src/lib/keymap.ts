// Defaults are docs/KEYMAP.md. Chords match on `event.code`, so they work under any keyboard layout.

export const defaults: Record<string, string> = {
  'search.everywhere': 'Double Shift',
  'file.goto': 'Cmd+Shift+O',
  'find.inFiles': 'Cmd+Shift+F',
  'action.find': 'Cmd+Shift+A',
  'file.recent': 'Cmd+E',
  'editor.gotoLine': 'Cmd+L',
  'nav.back': 'Cmd+[',
  'nav.forward': 'Cmd+]',
  'panel.project': 'Cmd+1',
  'vcs.update': 'Cmd+T',
  'vcs.popup': 'Ctrl+V',
  'diff.show': 'Cmd+D',
  'diff.next': 'F7',
  'diff.prev': 'Shift+F7',
  'app.settings': 'Cmd+,',
  'panel.log': 'Cmd+9',
}

/** Chords kept free for actions that do not exist yet. */
export const reserved: Record<string, string> = {
  'panel.commit': 'Cmd+0',
  'vcs.commit': 'Cmd+K',
  'vcs.push': 'Cmd+Shift+K',
  'vcs.rollback': 'Cmd+Alt+Z',
}

export interface KeyChord { meta: boolean; ctrl: boolean; alt: boolean; shift: boolean; code: string; key: string }
export type Chord = KeyChord | 'doubleShift'
export type Key = Pick<KeyboardEvent, 'key' | 'code' | 'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey' | 'repeat'>

const mods: Record<string, 'meta' | 'ctrl' | 'alt' | 'shift'> = {
  cmd: 'meta', meta: 'meta', command: 'meta', ctrl: 'ctrl', control: 'ctrl',
  alt: 'alt', opt: 'alt', option: 'alt', shift: 'shift',
}
const codes: Record<string, string> = {
  '[': 'BracketLeft', ']': 'BracketRight', ',': 'Comma', '.': 'Period', '/': 'Slash', ';': 'Semicolon',
  "'": 'Quote', '-': 'Minus', '=': 'Equal', '`': 'Backquote', '\\': 'Backslash',
  enter: 'Enter', esc: 'Escape', escape: 'Escape', tab: 'Tab', space: 'Space', backspace: 'Backspace',
  delete: 'Delete', up: 'ArrowUp', down: 'ArrowDown', left: 'ArrowLeft', right: 'ArrowRight',
  home: 'Home', end: 'End', pageup: 'PageUp', pagedown: 'PageDown',
}

function keyCode(k: string): string | null {
  if (/^[a-z]$/i.test(k)) return 'Key' + k.toUpperCase()
  if (/^[0-9]$/.test(k)) return 'Digit' + k
  if (/^f([1-9]|1[0-9]|2[0-4])$/i.test(k)) return k.toUpperCase()
  return codes[k.toLowerCase()] ?? null
}

/** "Cmd+Shift+O", "Ctrl+V", "F7", "Cmd+[", "Double Shift"; null when invalid or empty. */
export function parseChord(s: string): Chord | null {
  s = s.trim()
  if (/^double\s+shift$/i.test(s)) return 'doubleShift'
  const parts = s.split('+').map((p) => p.trim())
  const key = parts.pop()
  const code = key ? keyCode(key) : null
  if (!key || !code) return null
  const c: KeyChord = { meta: false, ctrl: false, alt: false, shift: false, code, key: key.length === 1 ? key.toUpperCase() : key }
  for (const p of parts) {
    const m = mods[p.toLowerCase()]
    if (!m) return null
    c[m] = true
  }
  return c
}

export const matches = (c: KeyChord, e: Key) =>
  e.code === c.code && e.metaKey === c.meta && e.ctrlKey === c.ctrl && e.altKey === c.alt && e.shiftKey === c.shift

export function formatChord(c: Chord | null): string {
  if (!c) return ''
  if (c === 'doubleShift') return '⇧⇧'
  return (c.ctrl ? '⌃' : '') + (c.alt ? '⌥' : '') + (c.shift ? '⇧' : '') + (c.meta ? '⌘' : '') + c.key
}

/** Defaults with `keymap.json` overrides applied; an override that does not parse unbinds the action. */
export function bindings(overrides: Record<string, string> = {}): Map<string, Chord> {
  const out = new Map<string, Chord>()
  for (const [id, s] of Object.entries({ ...defaults, ...overrides })) {
    const c = parseChord(s)
    if (c) out.set(id, c)
  }
  return out
}

/** Two Shift keydowns within 300 ms with no other key between. */
export class DoubleShift {
  private last: number | null = null
  down(e: Key, now: number): boolean {
    const lone = e.key === 'Shift' && !e.metaKey && !e.ctrlKey && !e.altKey
    if (!lone) {
      this.last = null
      return false
    }
    if (e.repeat) return false
    if (this.last !== null && now - this.last <= 300) {
      this.last = null
      return true
    }
    this.last = now
    return false
  }
}

let bound = bindings()

export const chordFor = (id: string) => formatChord(bound.get(id) ?? null)

export function setOverrides(o: Record<string, string>) {
  bound = bindings(o)
}

/** One capture-phase listener on window, so chords win over Monaco and inputs. */
export function install(dispatch: (id: string) => void): () => void {
  const double = new DoubleShift()
  const onKey = (e: KeyboardEvent) => {
    const twice = double.down(e, e.timeStamp)
    for (const [id, c] of bound) {
      if (c === 'doubleShift' ? twice : matches(c, e)) {
        e.preventDefault()
        e.stopPropagation()
        dispatch(id)
        return
      }
    }
  }
  window.addEventListener('keydown', onKey, true)
  return () => window.removeEventListener('keydown', onKey, true)
}
