import { describe, expect, it } from 'bun:test'
import { LogParser, visible } from './ansi'

const parse = (...chunks: string[]) => {
  const p = new LogParser()
  chunks.forEach((c, i) => p.push(c, i === chunks.length - 1))
  return p.lines
}
const texts = (lines: { spans: { text: string }[] }[]) => lines.map((l) => l.spans.map((s) => s.text).join(''))

describe('ansi', () => {
  it('turns SGR codes into span styles and keeps everything else as text', () => {
    const [l] = parse('\x1b[1;31mred\x1b[0m <img src=x onerror=alert(1)> \x1b[38;5;196mx\x1b[48;2;1;2;300my\x1b[22;39m.\n')
    expect(l.spans).toEqual([
      { bold: true, fg: 'var(--ansi-1)', text: 'red' },
      { text: ' <img src=x onerror=alert(1)> ' },
      { fg: 'rgb(255,0,0)', text: 'x' },
      { fg: 'rgb(255,0,0)', bg: 'rgb(1,2,255)', text: 'y' },
      { bg: 'rgb(1,2,255)', text: '.' },
    ])
    expect(parse('\x1b[92mok\n')[0].spans[0].fg).toBe('var(--ansi-10)')
    expect(parse('\x1b[244mg\n')[0].spans).toEqual([{ text: 'g' }])
    expect(parse('\x1b[38;5;244mg\n')[0].spans[0].fg).toBe('rgb(128,128,128)')
  })

  it('carries styles across lines and drops other escapes and control characters', () => {
    expect(parse('\x1b[32ma\nb\x1b[0m\n')[1].spans).toEqual([{ fg: 'var(--ansi-2)', text: 'b' }])
    expect(texts(parse('\x1b[0K\x1b[2Ja\x07b\x1b]0;title\x07c\x1b(B\n'))).toEqual(['abc'])
  })

  it('shows what a carriage return overwrote last, and strips CRLF', () => {
    expect(texts(parse('10%\r50%\r100%\nend\r\n'))).toEqual(['100%', 'end'])
  })

  it('links OSC 8 hyperlinks to http(s) only', () => {
    const [l] = parse('see \x1b]8;;https://ex.com/a\x1b\\here\x1b]8;;\x1b\\ and \x1b]8;;javascript:alert(1)\x07there\x1b]8;;\x07\n')
    expect(l.spans).toEqual([
      { text: 'see ' },
      { href: 'https://ex.com/a', text: 'here' },
      { text: ' and ' },
      { text: 'there' },
    ])
  })

  it('continues a partial line and a split escape in the next chunk', () => {
    const p = new LogParser()
    p.push('one\ntw\x1b[3')
    expect(texts(p.lines)).toEqual(['one', 'tw3'])
    p.push('1mo\nthree')
    expect(texts(p.lines)).toEqual(['one', 'two', 'three'])
    expect(p.lines[2].spans[0].fg).toBe('var(--ansi-1)')
    p.push('', true)
    expect(texts(p.lines)).toEqual(['one', 'two', 'three'])
  })

  it('folds GitLab sections, nested and collapsed', () => {
    const s = (k: string, name: string, opts = '') => `\x1b[0Ksection_${k}:1700000000:${name}${opts}\r\x1b[0K`
    const lines = parse(
      `${s('start', 'prep')}Preparing\n`,
      'inside\n',
      `${s('start', 'inner', '[collapsed=true]')}Inner\n`,
      'deep\n',
      `${s('end', 'inner')}${s('end', 'prep')}${s('start', 'script')}Script\n`,
      'run\n',
      `${s('end', 'script')}\n`,
      'after\n',
    )
    expect(texts(lines)).toEqual(['Preparing', 'inside', 'Inner', 'deep', 'Script', 'run', 'after'])
    expect(lines.map((l) => l.depth)).toEqual([0, 1, 1, 2, 0, 1, 0])
    expect(lines.map((l) => l.head)).toEqual([
      { end: 3, open: true }, undefined, { end: 3, open: false }, undefined, { end: 5, open: true }, undefined, undefined,
    ])
    expect(texts(visible(lines))).toEqual(['Preparing', 'inside', 'Inner', 'Script', 'run', 'after'])
    lines[0].head!.open = false
    expect(texts(visible(lines))).toEqual(['Preparing', 'Script', 'run', 'after'])
  })

  it('folds GitHub groups, collapsed, and an unclosed one to the end', () => {
    const lines = parse(
      '2026-10-01T10:00:00.0000000Z ##[group]Run tests\n',
      '2026-10-01T10:00:01.0000000Z ok\n',
      '2026-10-01T10:00:02.0000000Z ##[endgroup]\n',
      '##[group]Post\n',
      'cleanup\n',
    )
    expect(texts(lines)).toEqual(['Run tests', '2026-10-01T10:00:01.0000000Z ok', 'Post', 'cleanup'])
    expect(lines[0].head).toEqual({ end: 1, open: false })
    expect(lines[2].head).toEqual({ end: -1, open: false })
    expect(texts(visible(lines))).toEqual(['Run tests', 'Post'])
  })
})
