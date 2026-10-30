import type { LogRow } from './api'

export interface Chip { kind: 'head' | 'tag' | 'branch'; label: string }

/** `HEAD -> main` and a detached `HEAD` mark the current commit; `tag: v1` is a tag. */
export function chip(ref: string): Chip {
  if (ref.startsWith('HEAD -> ')) return { kind: 'head', label: ref.slice(8) }
  if (ref === 'HEAD') return { kind: 'head', label: 'HEAD' }
  if (ref.startsWith('tag: ')) return { kind: 'tag', label: ref.slice(5) }
  return { kind: 'branch', label: ref }
}

/** Lanes a row needs: its dot and every edge crossing it. */
export const lanes = (r: LogRow) => Math.max(r.column, ...[...r.top, ...r.bottom].flatMap((e) => [e.from, e.to])) + 1

/** Click selects one row, Cmd toggles one, Shift selects the range from the anchor. */
export function pick(rows: { sha: string }[], sel: Set<string>, anchor: number, i: number, how: 'one' | 'toggle' | 'range') {
  if (how === 'range') return new Set(rows.slice(Math.min(anchor, i), Math.max(anchor, i) + 1).map((r) => r.sha))
  const sha = rows[i].sha
  if (how === 'one') return new Set([sha])
  const next = new Set(sel)
  if (!next.delete(sha)) next.add(sha)
  return next
}

const fmt = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' })
export const when = (t: number) => fmt.format(t * 1000)
