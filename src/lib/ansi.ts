// Job log text to lines of styled spans: ANSI SGR, OSC 8 links, GitLab sections, GitHub groups.
// Only plain data comes out; components render it as text nodes.

export interface Style { fg?: string; bg?: string; bold?: boolean; dim?: boolean; italic?: boolean; underline?: boolean; href?: string }
export interface Span extends Style { text: string }
/** A section's first line carries `head`: `end` is the index of its last line, -1 while open. */
export interface Line { spans: Span[]; depth: number; head?: { end: number; open: boolean } }

const CUBE = [0, 95, 135, 175, 215, 255]
function c256(n: number) {
  if (n < 16) return `var(--ansi-${n})`
  if (n >= 232) return `rgb(${Array(3).fill(8 + 10 * (n - 232)).join(',')})`
  n -= 16
  return `rgb(${CUBE[Math.floor(n / 36) % 6]},${CUBE[Math.floor(n / 6) % 6]},${CUBE[n % 6]})`
}

function sgr(codes: string, s: Style): Style {
  const p = codes.split(/[;:]/).map(Number)
  s = { ...s }
  for (let i = 0; i < p.length; i++) {
    const n = p[i] || 0
    if (n === 0) s = { href: s.href }
    else if (n === 1) s.bold = true
    else if (n === 2) s.dim = true
    else if (n === 3) s.italic = true
    else if (n === 4) s.underline = true
    else if (n === 22) s.bold = s.dim = undefined
    else if (n === 23) s.italic = undefined
    else if (n === 24) s.underline = undefined
    else if (n === 39) s.fg = undefined
    else if (n === 49) s.bg = undefined
    else if ((n >= 30 && n <= 37) || (n >= 90 && n <= 97)) s.fg = c256((n % 10) + (n >= 90 ? 8 : 0))
    else if ((n >= 40 && n <= 47) || (n >= 100 && n <= 107)) s.bg = c256((n % 10) + (n >= 100 ? 8 : 0))
    else if (n === 38 || n === 48) {
      const k = n === 38 ? 'fg' : 'bg'
      if (p[i + 1] === 5) s[k] = c256(Math.min(255, p[i + 2] || 0)), (i += 2)
      else if (p[i + 1] === 2) s[k] = `rgb(${p.slice(i + 2, i + 5).map((v) => Math.min(255, v || 0)).join(',')})`, (i += 4)
    }
  }
  return s
}

// CSI, OSC 8 link, other OSC, charset, any other escape, a carriage return, other control characters.
const TOKEN = /\x1b\[([0-9;:?]*)[ -/]*([@-~])|\x1b\]8;[^;\x07\x1b]*;([^\x07\x1b]*)(?:\x07|\x1b\\)|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[()*+].|\x1b.?|(\r)|[\x00-\x08\x0b-\x1f\x7f]/g
const SECTION = /section_(start|end):\d+:([\w.-]+)(?:\[([^\]]*)\])?\r(?:\x1b\[0K)?/g
const GROUP = /^(\S+Z )?##\[(group|endgroup)\]/

export class LogParser {
  lines: Line[] = []
  private rest = ''
  private style: Style = {}
  private open: number[] = []
  /** State before the shown partial last line, which the next push parses again. */
  private saved: { n: number; style: Style; open: number[] } | null = null

  push(text: string, done = false) {
    if (this.saved) {
      this.lines.length = this.saved.n
      this.style = this.saved.style
      this.open = this.saved.open
      this.saved = null
    }
    const parts = (this.rest + text).split('\n')
    this.rest = parts.pop()!
    for (const p of parts) this.line(p)
    if (!this.rest) return
    if (done) {
      this.line(this.rest)
      this.rest = ''
      return
    }
    this.saved = { n: this.lines.length, style: this.style, open: [...this.open] }
    this.line(this.rest)
  }

  private close() {
    const h = this.open.pop()
    if (h !== undefined) this.lines[h].head!.end = this.lines.length - 1
  }

  private line(raw: string) {
    let head: { end: number; open: boolean } | undefined
    let marked = false
    raw = raw.replace(SECTION, (_, kind: string, _name, opts?: string) => {
      marked = true
      if (kind === 'end') this.close()
      else head = { end: -1, open: !/collapsed=true/.test(opts ?? '') }
      return ''
    })
    const g = raw.match(GROUP)
    if (g?.[2] === 'endgroup') return this.close()
    if (g) {
      head = { end: -1, open: false }
      raw = raw.slice(g[0].length)
    }

    raw = raw.replace(/\r+$/, '')
    let spans: Span[] = []
    let at = 0
    const text = (t: string) => t && spans.push({ ...this.style, text: t })
    for (const m of raw.matchAll(TOKEN)) {
      text(raw.slice(at, m.index))
      at = m.index + m[0].length
      if (m[2] === 'm') this.style = sgr(m[1], this.style)
      else if (m[3] !== undefined) this.style = { ...this.style, href: /^https?:\/\//i.test(m[3]) ? m[3] : undefined }
      else if (m[4]) spans = []
    }
    text(raw.slice(at))
    if (marked && !head && !spans.length) return
    this.lines.push({ spans, depth: this.open.length, head })
    if (head) this.open.push(this.lines.length - 1)
  }
}

/** The lines outside folded sections. */
export function visible(lines: Line[]): Line[] {
  const out: Line[] = []
  for (let i = 0; i < lines.length; i++) {
    out.push(lines[i])
    const h = lines[i].head
    if (h && !h.open) i = h.end < 0 ? lines.length : h.end
  }
  return out
}
