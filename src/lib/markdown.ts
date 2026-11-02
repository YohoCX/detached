import DOMPurify from 'dompurify'
import { Marked } from 'marked'

const ALLOWED_TAGS = [
  'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'p', 'br', 'hr', 'ul', 'ol', 'li', 'blockquote', 'pre', 'code', 'kbd',
  'table', 'thead', 'tbody', 'tr', 'th', 'td', 'a', 'img', 'em', 'strong', 'b', 'i', 'del', 's', 'sup', 'sub',
  'span', 'div', 'details', 'summary', 'input',
]
const ALLOWED_ATTR = ['href', 'src', 'alt', 'title', 'align', 'start', 'colspan', 'rowspan', 'type', 'checked']

const escape = (s: string) => s.replace(/[&<>"]/g, (c) => `&${{ '&': 'amp', '<': 'lt', '>': 'gt', '"': 'quot' }[c]};`)

// Code blocks carry their language for highlighting; mermaid blocks are drawn later.
const marked = new Marked({
  renderer: {
    code({ text, lang }) {
      const l = lang?.match(/^\S*/)?.[0] ?? ''
      return l === 'mermaid'
        ? `<pre data-mermaid="">${escape(text)}</pre>`
        : `<pre><code${l ? ` data-lang="${escape(l)}"` : ''}>${escape(text)}</code></pre>`
    },
  },
})

/** Foreign markdown as sanitized DOM; checkboxes only from task lists, and disabled. */
export function toDom(md: string, breaks: boolean): HTMLDivElement {
  const div = document.createElement('div')
  div.append(DOMPurify.sanitize(marked.parse(md, { async: false, breaks }), { ALLOWED_TAGS, ALLOWED_ATTR, RETURN_DOM_FRAGMENT: true }))
  for (const i of div.querySelectorAll('input')) {
    if (i.type === 'checkbox') i.setAttribute('disabled', '')
    else i.remove()
  }
  return div
}

/** GitHub's anchor for a heading. */
export const slug = (text: string) => text.trim().toLowerCase().replace(/[^\p{L}\p{N}\s_-]/gu, '').replace(/\s/g, '-')

export const hasScheme = (url: string) => /^([a-z][a-z0-9+.-]*:|\/\/)/i.test(url)

/** A relative or root-relative link in the file at `from` as a repo path, without query and anchor. */
export const repoPath = (url: string, from: string) => decodeURIComponent(new URL(url, `file:///${from}`).pathname.slice(1))

export const isMarkdown = (path: string) => /\.(md|markdown)$/i.test(path)

/** A repo file's markdown: relative images get `data-path` to load from the repo, insecure ones become links. */
export function renderFile(md: string, path: string): string {
  const div = toDom(md, false)
  for (const img of div.querySelectorAll('img')) {
    const src = img.getAttribute('src')
    if (!src) img.remove()
    if (!src || /^(https:|data:)/i.test(src)) continue
    if (!hasScheme(src)) {
      img.removeAttribute('src')
      img.dataset.path = repoPath(src, path)
      continue
    }
    const a = document.createElement('a')
    a.setAttribute('href', src)
    a.textContent = img.getAttribute('alt') || src
    img.replaceWith(a)
  }
  return div.innerHTML
}
