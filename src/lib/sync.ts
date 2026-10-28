import type { PullResult } from './api'

export interface Outcome { repo: string; ok: boolean; text: string }

/** Runs `fn` over `items`, at most `limit` at once; a rejection doesn't stop the rest. */
export async function pool<T, R>(items: T[], limit: number, fn: (item: T) => Promise<R>): Promise<PromiseSettledResult<R>[]> {
  const out: PromiseSettledResult<R>[] = new Array(items.length)
  let next = 0
  const worker = async () => {
    while (next < items.length) {
      const i = next++
      try {
        out[i] = { status: 'fulfilled', value: await fn(items[i]) }
      } catch (reason) {
        out[i] = { status: 'rejected', reason }
      }
    }
  }
  await Promise.all(Array.from({ length: Math.min(limit, items.length) }, worker))
  return out
}

const short = (sha: string) => sha.slice(0, 7)

export function describe(r: PullResult | void): string {
  if (!r) return 'Fetched'
  switch (r.kind) {
    case 'upToDate': return 'Up to date'
    case 'fastForwarded': return `Fast-forwarded ${short(r.from)}..${short(r.to)}`
    case 'diverged': return `Diverged (↑${r.ahead} ↓${r.behind}), not updated`
    case 'noUpstream': return 'No upstream'
  }
}

/** Fetch or pull every repo, four at a time, one outcome per repo in input order. */
export async function syncAll(
  repos: string[],
  run: (repo: string) => Promise<PullResult | void>,
  onDone: (o: Outcome) => void = () => {},
): Promise<Outcome[]> {
  const settled = await pool(repos, 4, async (repo) => {
    let o: Outcome
    try {
      const r = await run(repo)
      o = { repo, ok: r?.kind !== 'diverged', text: describe(r) }
    } catch (e) {
      o = { repo, ok: false, text: String(e) }
    }
    onDone(o)
    return o
  })
  return settled.map((s) => (s as PromiseFulfilledResult<Outcome>).value)
}
