// Client-side matching for short lists (actions, branches, recent files); files are ranked in Rust.

export interface Match { score: number; indices: number[] }

/** Case-insensitive subsequence match; tighter and earlier matches score higher. */
export function fuzzy(query: string, text: string): Match | null {
  const q = Array.from(query.toLowerCase().replace(/\s+/g, ''))
  const t = Array.from(text.toLowerCase())
  if (!q.length) return { score: 0, indices: [] }
  const indices: number[] = []
  for (let i = 0; i < t.length && indices.length < q.length; i++) if (t[i] === q[indices.length]) indices.push(i)
  if (indices.length < q.length) return null
  return { score: -(indices[indices.length - 1] - indices[0]) - indices[0] / 100, indices }
}

export function filter<T>(items: T[], query: string, text: (t: T) => string): { item: T; indices: number[] }[] {
  const out: { item: T; indices: number[]; score: number }[] = []
  for (const item of items) {
    const m = fuzzy(query, text(item))
    if (m) out.push({ item, ...m })
  }
  return query ? out.sort((a, b) => b.score - a.score) : out
}

/** Splits `text` into runs for highlighting the chars at `indices`. */
export function segments(text: string, indices: number[]): { text: string; hit: boolean }[] {
  const hits = new Set(indices)
  const out: { text: string; hit: boolean }[] = []
  Array.from(text).forEach((ch, i) => {
    const hit = hits.has(i)
    const last = out[out.length - 1]
    if (last?.hit === hit) last.text += ch
    else out.push({ text: ch, hit })
  })
  return out
}
