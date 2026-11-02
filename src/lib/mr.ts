import type { DiffSide, ForgeHosts, MergeRequest, PipelineStatus } from './api'
import { hasScheme, toDom } from './markdown'

/** Why MRs cannot load for a repo, from `forge_hosts`; null when they can. */
export function setupProblem(h: ForgeHosts): string | null {
  if (!h.mrRemote) return 'Choose the remote that holds the merge requests in Settings.'
  const host = h.hosts.find((x) => x.remotes.includes(h.mrRemote!))
  if (!host) return `Remote ${h.mrRemote} is not on a forge.`
  if (host.accountId) return null
  return host.ambiguous ? `Several accounts fit ${host.host}; choose one in Settings.` : `No account for ${host.host}; add one in Settings.`
}

export const isGitLab = (mr: MergeRequest) => mr.provider.startsWith('gitlab')
export const bareTitle = (mr: MergeRequest) => (mr.draft ? mr.title.replace(/^(draft:|\[draft\]|\(draft\))\s*/i, '') : mr.title)

export const tone = (s: PipelineStatus | null) =>
  s === 'success' ? 'success' : s === 'failed' ? 'error' : s === 'running' || s === 'pending' ? 'accent' : 'deleted'

export function ago(t: number, now = Date.now() / 1000) {
  const s = now - t
  if (s < 3600) return `${Math.max(1, Math.round(s / 60))}m`
  if (s < 86400) return `${Math.round(s / 3600)}h`
  if (s < 30 * 86400) return `${Math.round(s / 86400)}d`
  return new Date(t * 1000).toLocaleDateString()
}

/** On the line changes of a diff, the other side's line for an unchanged `line`, else null. */
export function otherLine(
  changes: { originalStartLineNumber: number; originalEndLineNumber: number; modifiedStartLineNumber: number; modifiedEndLineNumber: number }[],
  side: DiffSide,
  line: number,
): number | null {
  let delta = 0
  for (const c of changes) {
    const o = [c.originalStartLineNumber, c.originalEndLineNumber] as const
    const m = [c.modifiedStartLineNumber, c.modifiedEndLineNumber] as const
    const [[s, e], [os, oe]] = side === 'new' ? [m, o] : [o, m]
    const n = e ? e - s + 1 : 0
    if (n && line >= s && line <= e) return null
    // A side without lines (end 0) has the change after its start line.
    if ((n ? e : s) < line) delta += (oe ? oe - os + 1 : 0) - n
  }
  return line + delta
}

export interface Links { projectUrl: string; baseUrl: string; gitlab: boolean }

export function links(mr: MergeRequest): Links {
  const p = mr.projectUrl.replace(/\/$/, '')
  const tail = `/${mr.targetProject.path}`
  return { projectUrl: p, baseUrl: p.endsWith(tail) ? p.slice(0, -tail.length) : new URL(p).origin, gitlab: isGitLab(mr) }
}

/** GitLab uploads under the project, other root-relative URLs under the base URL, relative ones against the project. */
export function resolveUrl(url: string, l: Links): string {
  if (hasScheme(url) || url.startsWith('#')) return url
  if (url.startsWith('/')) return (l.gitlab && url.startsWith('/uploads/') ? l.projectUrl : l.baseUrl) + url
  return new URL(url, `${l.projectUrl}/`).href
}

export function render(md: string, l: Links): string {
  const div = toDom(md, true)
  for (const a of div.querySelectorAll('a[href]')) a.setAttribute('href', resolveUrl(a.getAttribute('href')!, l))
  for (const img of div.querySelectorAll('img')) {
    const src = img.getAttribute('src')
    if (!src) {
      img.remove()
      continue
    }
    const url = resolveUrl(src, l)
    if (/^(https:|data:)/i.test(url) && !(l.gitlab && url.startsWith(`${l.projectUrl}/uploads/`))) {
      img.setAttribute('src', url)
      continue
    }
    const a = document.createElement('a')
    a.setAttribute('href', url)
    a.textContent = img.getAttribute('alt') || url
    img.replaceWith(a)
  }
  return div.innerHTML
}
