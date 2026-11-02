import { describe, expect, it } from 'bun:test'
import type { ForgeHosts } from './api'
import { otherLine, render, resolveUrl, setupProblem } from './mr'

const gl = { projectUrl: 'https://git.example.com/gitlab/group/app', baseUrl: 'https://git.example.com/gitlab', gitlab: true }
const gh = { projectUrl: 'https://github.com/owner/app', baseUrl: 'https://github.com', gitlab: false }

describe('markdown', () => {
  it('strips script, handlers, style, forms and frames', () => {
    const html = render(
      [
        '<script>alert(1)</script><img src="https://x/a.png" onerror="alert(1)">',
        '<p style="color:red" onclick="alert(1)">hi</p><style>body{}</style>',
        '<form action="https://x"><input name="a"><button>go</button></form><iframe src="https://x"></iframe>',
        '<map><area href="https://x"></map><a href="javascript:alert(1)">js</a>',
      ].join('\n'),
      gh,
    )
    expect(html).not.toMatch(/<script|onerror|onclick|style|<form|<input|<button|<iframe|<area|<map|javascript:/i)
    expect(html).toContain('<img src="https://x/a.png">')
    expect(html).toContain('hi')
  })

  it('resolves links against the project and the base URL', () => {
    expect(resolveUrl('/uploads/abc/x.png', gl)).toBe('https://git.example.com/gitlab/group/app/uploads/abc/x.png')
    expect(resolveUrl('/group/other', gl)).toBe('https://git.example.com/gitlab/group/other')
    expect(resolveUrl('docs/a.md', gl)).toBe('https://git.example.com/gitlab/group/app/docs/a.md')
    expect(resolveUrl('../b', gh)).toBe('https://github.com/owner/b')
    expect(resolveUrl('/uploads/x', gh)).toBe('https://github.com/uploads/x')
    expect(resolveUrl('https://a.b/c', gl)).toBe('https://a.b/c')
    expect(resolveUrl('#top', gl)).toBe('#top')
  })

  it('shows GitLab uploads and non-https images as links', () => {
    const html = render('![shot](/uploads/abc/x.png) ![plain](http://x/y.png) ![ok](https://x/z.png)', gl)
    expect(html).toContain('<a href="https://git.example.com/gitlab/group/app/uploads/abc/x.png">shot</a>')
    expect(html).toContain('<a href="http://x/y.png">plain</a>')
    expect(html).toContain('<img src="https://x/z.png" alt="ok">')
    expect(render('[rel](src/a.ts)', gl)).toContain('href="https://git.example.com/gitlab/group/app/src/a.ts"')
  })
})

describe('mr', () => {
  it('pairs unchanged lines across the diff', () => {
    // Old 3 changed into new 3; old 6–7 removed; new 9–10 added after old 10.
    const changes = [
      { originalStartLineNumber: 3, originalEndLineNumber: 3, modifiedStartLineNumber: 3, modifiedEndLineNumber: 3 },
      { originalStartLineNumber: 6, originalEndLineNumber: 7, modifiedStartLineNumber: 5, modifiedEndLineNumber: 0 },
      { originalStartLineNumber: 10, originalEndLineNumber: 0, modifiedStartLineNumber: 9, modifiedEndLineNumber: 10 },
    ]
    expect([1, 3, 5, 6, 8, 9, 11].map((l) => otherLine(changes, 'new', l))).toEqual([1, null, 5, 8, 10, null, 11])
    expect([3, 5, 6, 8, 10, 11].map((l) => otherLine(changes, 'old', l))).toEqual([null, 5, null, 6, 8, 11])
  })

  it('names what is missing to load MRs', () => {
    const h = (accountId: string | null, ambiguous = false, mrRemote: string | null = 'origin'): ForgeHosts =>
      ({ mrRemote, hosts: [{ host: 'h', remotes: ['origin'], accountId, ambiguous }] })
    expect(setupProblem(h('a'))).toBeNull()
    expect(setupProblem(h(null))).toMatch(/No account for h/)
    expect(setupProblem(h(null, true))).toMatch(/Several accounts fit h/)
    expect(setupProblem(h('a', false, null))).toMatch(/Choose the remote/)
  })
})
