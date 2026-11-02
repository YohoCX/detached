import { describe, expect, it } from 'bun:test'
import { renderFile, repoPath, slug, toDom } from './markdown'

describe('markdown', () => {
  it('marks code languages and mermaid blocks, escaped', () => {
    const html = toDom('```ts title\nconst a = "<b>"\n```\n\n```mermaid\ngraph TD; A-->B\n```\n\n```\nplain\n```', false).innerHTML
    expect(html).toContain('<pre><code data-lang="ts">const a = "&lt;b&gt;"</code></pre>')
    expect(html).toContain('<pre data-mermaid="">graph TD; A--&gt;B</pre>')
    expect(html).toContain('<pre><code>plain</code></pre>')
  })

  it('keeps task list checkboxes, disabled, and drops other inputs', () => {
    const html = toDom('- [x] done\n- [ ] todo\n\n<input type="text" value="x">', false).innerHTML
    expect(html.match(/<input [^>]*>/g)).toEqual(['<input checked="" type="checkbox" disabled="">', '<input type="checkbox" disabled="">'])
  })

  it('loads relative images from the repo and links insecure ones', () => {
    const html = renderFile('![a](img/a.png) ![b](/b.svg) ![c](https://x/c.png) ![d](http://x/d.png) ![e](../e.gif)', 'docs/README.md')
    expect(html).toContain('<img alt="a" data-path="docs/img/a.png">')
    expect(html).toContain('<img alt="b" data-path="b.svg">')
    expect(html).toContain('<img src="https://x/c.png" alt="c">')
    expect(html).toContain('<a href="http://x/d.png">d</a>')
    expect(html).toContain('data-path="e.gif"')
  })

  it('resolves repo links and heading anchors', () => {
    expect(repoPath('b.md#part', 'docs/a.md')).toBe('docs/b.md')
    expect(repoPath('../src/x%20y.ts?plain=1', 'docs/a.md')).toBe('src/x y.ts')
    expect(repoPath('/LICENSE', 'docs/deep/a.md')).toBe('LICENSE')
    expect(slug(' Install & Run: v2_beta ')).toBe('install--run-v2_beta')
    expect(slug('Привет, мир')).toBe('привет-мир')
  })
})
