// Fake GitLab (under /gitlab) and GitHub Enterprise (under /ghe) APIs for e2e, seeded per test.
// Runs as plain node: type annotations only, no imports beyond node's.

import { createServer } from 'node:http'

export type Status = 'pending' | 'running' | 'success' | 'failed' | 'canceled' | 'skipped' | 'manual'
export interface SeedJob { id: number; name: string; stage: string; status: Status; log: string }
export interface SeedPipeline { id: number; ref: string; sha: string; status: Status; jobs: SeedJob[] }
export interface SeedMr {
  number: number
  title: string
  description: string
  author: string
  state: 'open' | 'closed' | 'merged'
  draft: boolean
  sourceBranch: string
  targetBranch: string
  baseSha: string
  headSha: string
}
export interface SeedProject {
  forge: 'gitlab' | 'ghe'
  /** Below the base URL: `group/app`, `owner/app`. */
  path: string
  defaultBranch: string
  mrs: SeedMr[]
  pipelines: SeedPipeline[]
}
/** `user` is the account every token belongs to. */
export interface Seed { user: string; projects: SeedProject[] }

let seed: Seed = { user: '', projects: [] }

createServer(async (req, res) => {
  let body = ''
  for await (const chunk of req) body += chunk
  const send = (status: number, v: unknown) => {
    res.writeHead(status, { 'Content-Type': 'application/json' })
    res.end(JSON.stringify(v))
  }
  const { pathname } = new URL(req.url!, 'http://fake')
  if (pathname === '/health') return send(200, { projects: seed.projects.length })
  if (req.method === 'POST' && pathname === '/reset') {
    seed = JSON.parse(body)
    return send(200, null)
  }
  send(404, { message: '404 Not Found' })
}).listen(Number(process.argv[2]), '127.0.0.1')
