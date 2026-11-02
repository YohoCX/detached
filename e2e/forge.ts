// Fake GitLab (under /gitlab) and GitHub Enterprise (under /ghe) APIs for e2e, seeded by `POST /reset`.
// Runs as plain node: type annotations only, no imports beyond node's.
// Test hooks: `GET /__writes` lists the writes (GraphQL mutations, not queries) since the reset and
// `GET /__requests` every API request, the path decoded and without its query. `POST /__append
// {forge, path, job, text, status?}` grows a job's log for a live log; `POST /__delay {ms}` slows replies.

import { createServer } from 'node:http'

type Status = 'pending' | 'running' | 'success' | 'failed' | 'canceled' | 'skipped' | 'manual'
interface SeedJob { id: number; name: string; stage: string; status: Status; log: string }
interface SeedPipeline { id: number; ref: string; sha: string; status: Status; jobs: SeedJob[]; /** `push` when unset. */ source?: string }
interface SeedMr {
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

interface FakeNote { id: number; thread: string; body: string; author: string; created: string; position?: Record<string, unknown> }
interface FakeReview { id: number; state: string; body: string; user: string }
interface FakeMr extends SeedMr { notes: FakeNote[]; resolved: string[]; approvedBy: string[]; reviews: FakeReview[]; autoMerge: boolean }
interface FakeProject extends SeedProject { id: number; mrs: FakeMr[]; branches: Record<string, string> }
export interface Write { method: string; path: string; body: unknown }
interface Reply { status?: number; body?: unknown; text?: string; headers?: Record<string, string> }

const T = '2026-10-01T10:00:00Z'
const ACTIVE = ['pending', 'running']
const glMissing = { status: 404, body: { message: '404 Not found' } }
const ghMissing = { status: 404, body: { message: 'Not Found' } }
let user = ''
let projects: FakeProject[] = []
let writes: Write[] = []
let requests: { method: string; path: string }[] = []
let delay = 0
let next = 1000

function reset(s: Seed) {
  user = s.user
  writes = []
  requests = []
  delay = 0
  projects = s.projects.map((p, i) => {
    const branches: Record<string, string> = {}
    for (const m of p.mrs) Object.assign(branches, { [m.targetBranch]: m.baseSha, [m.sourceBranch]: m.headSha })
    const mrs = p.mrs.map((m) => ({ ...m, notes: [], resolved: [], approvedBy: [], reviews: [], autoMerge: false }))
    return { ...p, id: i + 1, mrs, branches }
  })
}

const origin = () => `http://127.0.0.1:${process.argv[2]}`
const project = (forge: string, path: string) => projects.find((p) => p.forge === forge && p.path === path)
const findMr = (p: FakeProject, n: string) => p.mrs.find((m) => m.number === Number(n))
const findJob = (p: FakeProject, id: number) => p.pipelines.flatMap((pl) => pl.jobs.map((j) => ({ pl, j }))).find((x) => x.j.id === id)
const route = (method: string, path: string) => (m: string, re: RegExp) => (method === m ? path.match(re) : null)
const cancel = (pl: SeedPipeline) => {
  for (const j of pl.jobs) if (ACTIVE.includes(j.status)) j.status = 'canceled'
  pl.status = 'canceled'
}
const headPipeline = (p: FakeProject, m: FakeMr) => p.pipelines.find((pl) => pl.sha === m.headSha)
const newPipeline = (p: FakeProject, ref: string, source: string): SeedPipeline => {
  const id = ++next
  const pl: SeedPipeline = { id, ref, sha: p.branches[ref] ?? '', status: 'pending', source, jobs: [{ id: ++next, name: 'build', stage: 'build', status: 'pending', log: '' }] }
  p.pipelines.unshift(pl)
  return pl
}
const addMr = (p: FakeProject, m: Partial<SeedMr>): FakeMr => {
  const number = Math.max(0, ...p.mrs.map((x) => x.number)) + 1
  const mr: FakeMr = {
    number, title: '', description: '', author: user, state: 'open', draft: false, sourceBranch: '', targetBranch: p.defaultBranch,
    baseSha: '', headSha: '', notes: [], resolved: [], approvedBy: [], reviews: [], autoMerge: false, ...m,
  }
  mr.baseSha ||= p.branches[mr.targetBranch] ?? ''
  mr.headSha ||= p.branches[mr.sourceBranch] ?? ''
  p.mrs.push(mr)
  return mr
}
const note = (m: FakeMr, thread: string | null, body: string, position?: Record<string, unknown>) => {
  const id = ++next
  const n: FakeNote = { id, thread: thread ?? String(id), body, author: user, created: new Date().toISOString(), position }
  m.notes.push(n)
  return n
}
const threads = (m: FakeMr) => [...new Set(m.notes.map((n) => n.thread))].map((id) => ({ id, notes: m.notes.filter((n) => n.thread === id) }))

/** The page of `list` that `url` asks for, with GitLab's `X-Next-Page` or GitHub's `Link`. */
function paged(list: unknown[], url: URL, wrap?: string): Reply {
  const per = Number(url.searchParams.get('per_page') ?? 20)
  const page = Number(url.searchParams.get('page') ?? 1)
  const items = list.slice((page - 1) * per, page * per)
  const more = page * per < list.length
  const next = new URL(url)
  next.searchParams.set('page', String(page + 1))
  const headers: Record<string, string> = url.pathname.startsWith('/ghe/') ? (more ? { Link: `<${next}>; rel="next"` } : {}) : { 'X-Next-Page': more ? String(page + 1) : '' }
  return { body: wrap ? { total_count: list.length, [wrap]: items } : items, headers }
}

// GitLab v4

const glProject = (p: FakeProject) => ({
  id: p.id, path_with_namespace: p.path, default_branch: p.defaultBranch, web_url: `${origin()}/gitlab/${p.path}`,
  squash_option: 'default_off', merge_method: 'merge',
})
const glMr = (p: FakeProject, m: FakeMr) => {
  const pl = headPipeline(p, m)
  return {
    id: p.id * 1000 + m.number, iid: m.number, project_id: p.id, title: m.draft ? `Draft: ${m.title}` : m.title, description: m.description,
    state: m.state === 'open' ? 'opened' : m.state, draft: m.draft, work_in_progress: m.draft, author: { username: m.author },
    source_branch: m.sourceBranch, target_branch: m.targetBranch, source_project_id: p.id, target_project_id: p.id, sha: m.headSha,
    diff_refs: { base_sha: m.baseSha, start_sha: m.baseSha, head_sha: m.headSha }, labels: [], reviewers: [],
    detailed_merge_status: m.state !== 'open' ? 'not_open' : m.draft ? 'draft_status' : 'mergeable',
    merge_when_pipeline_succeeds: m.autoMerge, head_pipeline: pl ? { id: pl.id, status: pl.status } : null,
    web_url: `${origin()}/gitlab/${p.path}/-/merge_requests/${m.number}`, created_at: T, updated_at: T,
  }
}
const glPipeline = (p: FakeProject, pl: SeedPipeline) => ({
  id: pl.id, iid: pl.id, project_id: p.id, sha: pl.sha, ref: pl.ref, status: pl.status, source: pl.source ?? 'push', created_at: T, updated_at: T,
  started_at: pl.status === 'pending' ? null : T, finished_at: ACTIVE.includes(pl.status) ? null : T,
  web_url: `${origin()}/gitlab/${p.path}/-/pipelines/${pl.id}`,
})
const glJob = (p: FakeProject, pl: SeedPipeline, j: SeedJob) => ({
  id: j.id, name: j.name, stage: j.stage, status: j.status, duration: ACTIVE.includes(j.status) ? null : 60, allow_failure: false,
  web_url: `${origin()}/gitlab/${p.path}/-/jobs/${j.id}`, pipeline: { id: pl.id, project_id: p.id, ref: pl.ref, sha: pl.sha, status: pl.status },
})
const glDiscussion = (m: FakeMr, t: { id: string; notes: FakeNote[] }) => ({
  id: t.id, individual_note: !t.notes[0].position && t.notes.length === 1,
  notes: t.notes.map((n) => ({
    id: n.id, type: n.position ? 'DiffNote' : null, body: n.body, author: { username: n.author }, created_at: n.created, system: false,
    resolvable: !!t.notes[0].position, resolved: m.resolved.includes(t.id), position: t.notes[0].position,
  })),
})

function glGraphql({ query, variables: v }: any): Reply {
  const p = project('gitlab', v.path)
  if (!query.includes('mergeRequests(')) return { body: { errors: [{ message: 'unknown query' }] } }
  if (!p) return { body: { data: { project: null } } }
  const arg = (k: string) => query.match(new RegExp(`${k}:"?([^",)]+)`))?.[1]
  const state = arg('state')
  const list = p.mrs.filter((m) => (!state || glMr(p, m).state === state) && (!arg('authorUsername') || m.author === arg('authorUsername')) &&
    !arg('reviewerUsername') && !arg('assigneeUsername'))
  const nodes = list.reverse().map((m) => {
    const r = glMr(p, m)
    return {
      iid: String(m.number), title: r.title, state: r.state, draft: m.draft, sourceBranch: m.sourceBranch, targetBranch: m.targetBranch,
      updatedAt: T, webUrl: r.web_url, author: { username: m.author }, approvedBy: { nodes: m.approvedBy.map((username) => ({ username })) },
      headPipeline: r.head_pipeline && { status: r.head_pipeline.status.toUpperCase() }, sourceProject: { fullPath: p.path },
    }
  })
  return { body: { data: { project: { mergeRequests: { nodes } } } } }
}

function gitlab(method: string, rest: string, body: any, url: URL): Reply {
  if (rest === '/version') return { body: { version: '17.11.0', revision: 'fake' } }
  if (rest === '/user') return { body: { id: 1, username: user } }
  const pm = rest.match(/^\/projects\/([^/]+)(\/.*)?$/)
  const p = pm && projects.find((x) => x.forge === 'gitlab' && (x.path === decodeURIComponent(pm[1]) || String(x.id) === pm[1]))
  if (!pm || !p) return { status: 404, body: { message: '404 Project Not Found' } }
  const q = url.searchParams
  const r = pm[2] ?? ''
  const is = route(method, r)
  let x: RegExpMatchArray | null
  if (is('GET', /^$/)) return { body: glProject(p) }
  if (is('GET', /^\/merge_requests$/)) {
    const state = q.get('state') ?? 'all'
    const list = p.mrs.filter((m) =>
      (state === 'all' || glMr(p, m).state === state) && (!q.has('source_branch') || m.sourceBranch === q.get('source_branch')) &&
      (q.get('scope') !== 'created_by_me' || m.author === user) && q.get('scope') !== 'assigned_to_me' && !q.has('reviewer_username'))
    return paged(list.reverse().map((m) => glMr(p, m)), url)
  }
  if (is('POST', /^\/merge_requests$/)) {
    const draft = /^draft:\s*/i.test(body.title)
    return { status: 201, body: glMr(p, addMr(p, { title: body.title.replace(/^draft:\s*/i, ''), description: body.description ?? '', draft, sourceBranch: body.source_branch, targetBranch: body.target_branch })) }
  }
  if (is('GET', /^\/pipelines$/)) {
    const list = p.pipelines.filter((pl) => (!q.has('ref') || pl.ref === q.get('ref')) && (!q.has('sha') || pl.sha === q.get('sha')))
    return paged(list.map((pl) => glPipeline(p, pl)), url)
  }
  if (is('POST', /^\/pipeline$/)) return { status: 201, body: glPipeline(p, newPipeline(p, body.ref, 'api')) }
  if ((x = r.match(/^\/pipelines\/(\d+)(\/.*)?$/))) {
    const pl = p.pipelines.find((y) => y.id === Number(x![1]))
    if (!pl) return glMissing
    if (method === 'GET' && x[2] === '/jobs') return paged(pl.jobs.map((j) => glJob(p, pl, j)).reverse(), url)
    if (method === 'POST' && x[2] === '/retry') {
      for (const j of pl.jobs) if (j.status === 'failed' || j.status === 'canceled') j.status = 'pending'
      pl.status = 'running'
      return { status: 201, body: glPipeline(p, pl) }
    }
    if (method === 'POST' && x[2] === '/cancel') {
      cancel(pl)
      return { body: glPipeline(p, pl) }
    }
  }
  if ((x = r.match(/^\/jobs\/(\d+)(\/\w+)?$/))) {
    const hit = findJob(p, Number(x[1]))
    if (!hit) return glMissing
    const { pl, j } = hit
    if (method === 'GET' && !x[2]) return { body: glJob(p, pl, j) }
    if (method === 'GET' && x[2] === '/trace') return { text: j.log }
    if (method === 'POST' && x[2] === '/retry') Object.assign(j, { status: 'pending', log: '' })
    else if (method === 'POST' && x[2] === '/cancel') j.status = 'canceled'
    else if (method === 'POST' && x[2] === '/play') j.status = 'pending'
    else return glMissing
    return { status: 201, body: glJob(p, pl, j) }
  }
  const m = (x = r.match(/^\/merge_requests\/(\d+)(\/.*)?$/)) && findMr(p, x[1])
  if (!x || !m) return glMissing
  const at = route(method, x[2] ?? '')
  if (at('GET', /^$/)) return { body: glMr(p, m) }
  if (at('PUT', /^$/)) {
    if (body.title !== undefined) Object.assign(m, { draft: /^draft:\s*/i.test(body.title), title: body.title.replace(/^draft:\s*/i, '') })
    if (body.state_event === 'close') m.state = 'closed'
    if (body.state_event === 'reopen') m.state = 'open'
    return { body: glMr(p, m) }
  }
  if (at('GET', /^\/approvals$/)) {
    return { body: { approved: m.approvedBy.length > 0, user_has_approved: m.approvedBy.includes(user), approved_by: m.approvedBy.map((u) => ({ user: { username: u } })) } }
  }
  if (at('POST', /^\/approve$/)) m.approvedBy = [...new Set([...m.approvedBy, user])]
  if (at('POST', /^\/unapprove$/)) m.approvedBy = m.approvedBy.filter((u) => u !== user)
  if (at('POST', /^\/(un)?approve$/)) return { status: 201, body: {} }
  if (at('GET', /^\/pipelines$/)) return { body: p.pipelines.filter((pl) => pl.sha === m.headSha).map((pl) => glPipeline(p, pl)) }
  if (at('GET', /^\/discussions$/)) return paged(threads(m).map((t) => glDiscussion(m, t)), url)
  if (at('POST', /^\/notes$/)) return { status: 201, body: { id: note(m, null, body.body).id } }
  if (at('POST', /^\/discussions$/)) {
    const n = note(m, null, body.body, body.position)
    return { status: 201, body: glDiscussion(m, { id: n.thread, notes: [n] }) }
  }
  if ((x = at('POST', /^\/discussions\/([^/]+)\/notes$/))) return { status: 201, body: { id: note(m, decodeURIComponent(x[1]), body.body).id } }
  if ((x = at('PUT', /^\/discussions\/([^/]+)$/))) {
    const id = decodeURIComponent(x[1])
    m.resolved = body.resolved ? [...m.resolved, id] : m.resolved.filter((t) => t !== id)
    return { body: glDiscussion(m, threads(m).find((t) => t.id === id)!) }
  }
  if (at('PUT', /^\/merge$/)) {
    if (m.state !== 'open') return { status: 405, body: { message: '405 Method Not Allowed' } }
    if (body.sha && body.sha !== m.headSha) return { status: 409, body: { message: 'SHA does not match HEAD of source branch' } }
    if ((body.auto_merge || body.merge_when_pipeline_succeeds) && headPipeline(p, m)?.status !== 'success') m.autoMerge = true
    else m.state = 'merged'
    if (m.state === 'merged' && body.should_remove_source_branch) delete p.branches[m.sourceBranch]
    return { body: glMr(p, m) }
  }
  return glMissing
}

// GitHub Enterprise REST v3 and GraphQL

const ghRepo = (p: FakeProject) => ({
  id: p.id, node_id: `R_${p.id}`, name: p.path.split('/')[1], full_name: p.path, owner: { login: p.path.split('/')[0] },
  default_branch: p.defaultBranch, html_url: `${origin()}/ghe/${p.path}`, allow_merge_commit: true, allow_squash_merge: true,
  allow_rebase_merge: false, permissions: { admin: false, push: true, pull: true },
})
const ghPr = (p: FakeProject, m: FakeMr) => ({
  id: p.id * 1000 + m.number, node_id: `PR_${p.id}_${m.number}`, number: m.number, title: m.title, body: m.description,
  state: m.state === 'open' ? 'open' : 'closed', merged: m.state === 'merged', merged_at: m.state === 'merged' ? T : null,
  draft: m.draft, user: { login: m.author }, labels: [], requested_reviewers: [], assignees: [],
  head: { ref: m.sourceBranch, sha: m.headSha, repo: ghRepo(p) }, base: { ref: m.targetBranch, sha: m.baseSha, repo: ghRepo(p) },
  mergeable: m.state === 'open' ? true : null, mergeable_state: m.state !== 'open' ? 'unknown' : m.draft ? 'draft' : 'clean',
  auto_merge: m.autoMerge ? { merge_method: 'merge' } : null, html_url: `${origin()}/ghe/${p.path}/pull/${m.number}`, created_at: T, updated_at: T,
})
const ghStatus = (s: Status) =>
  s === 'pending' ? { status: 'queued', conclusion: null } : s === 'running' ? { status: 'in_progress', conclusion: null }
    : { status: 'completed', conclusion: { success: 'success', failed: 'failure', canceled: 'cancelled', skipped: 'skipped', manual: 'action_required' }[s] }
const ghRun = (p: FakeProject, pl: SeedPipeline) => ({
  id: pl.id, name: 'CI', workflow_id: 1, head_branch: pl.ref, head_sha: pl.sha, event: pl.source ?? 'push', ...ghStatus(pl.status),
  created_at: T, run_started_at: T, updated_at: T, html_url: `${origin()}/ghe/${p.path}/actions/runs/${pl.id}`,
})
const ghJob = (p: FakeProject, pl: SeedPipeline, j: SeedJob) => {
  const done = !ACTIVE.includes(j.status)
  return {
    id: j.id, run_id: pl.id, workflow_name: 'CI', head_branch: pl.ref, head_sha: pl.sha, name: j.name, ...ghStatus(j.status),
    started_at: T, completed_at: done ? '2026-10-01T10:01:00Z' : null, html_url: `${origin()}/ghe/${p.path}/actions/runs/${pl.id}/job/${j.id}`,
    steps: [{ name: 'Run', number: 1, ...ghStatus(j.status) }],
  }
}
const ghComment = (n: FakeNote) => ({ id: n.id, node_id: `IC_${n.id}`, body: n.body, user: { login: n.author }, created_at: n.created })
const ghReview = (r: FakeReview) => ({ id: r.id, node_id: `PRR_${r.id}`, user: { login: r.user }, body: r.body, state: r.state, submitted_at: T })

function graphql(body: any): Reply {
  const { query, variables: v } = body
  const byNode = (id: string) => {
    for (const p of projects) for (const m of p.mrs) if (`PR_${p.id}_${m.number}` === id) return { p, m }
    return null
  }
  const noRepo = { body: { data: { repository: null }, errors: [{ message: 'Could not resolve to a Repository' }] } }
  const noNode = { body: { errors: [{ message: `Could not resolve to a node with the global id of '${v.id}'` }] } }
  if (query.includes('pullRequests(')) {
    const p = project('ghe', `${v.owner}/${v.name}`)
    if (!p) return noRepo
    const nodes = p.mrs.filter((m) => !v.states || v.states.includes(m.state.toUpperCase())).reverse().map((m) => {
      const pl = headPipeline(p, m)
      const latest = new Map(m.reviews.filter((r) => ['APPROVED', 'CHANGES_REQUESTED'].includes(r.state)).map((r) => [r.user, r.state]))
      return {
        number: m.number, title: m.title, state: m.state.toUpperCase(), isDraft: m.draft, headRefName: m.sourceBranch, baseRefName: m.targetBranch,
        updatedAt: T, url: `${origin()}/ghe/${p.path}/pull/${m.number}`, author: { login: m.author }, headRepository: { nameWithOwner: p.path },
        assignees: { nodes: [] }, reviewRequests: { nodes: [] }, latestOpinionatedReviews: { nodes: [...latest].map(([login, state]) => ({ state, author: { login } })) },
        commits: { nodes: [{ commit: { statusCheckRollup: pl && { state: { success: 'SUCCESS', failed: 'FAILURE', canceled: 'FAILURE' }[pl.status as string] ?? 'PENDING' } } }] },
      }
    })
    return { body: { data: { repository: { pullRequests: { nodes } } } } }
  }
  const pr = byNode(v.id ?? '')
  const ok = { body: { data: { result: { clientMutationId: null } } } }
  if (query.includes('reviewThreads')) {
    const m = project('ghe', `${v.owner}/${v.name}`)?.mrs.find((x) => x.number === v.number)
    if (!m) return noRepo
    const nodes = threads(m).filter((t) => t.notes[0].position).map((t) => {
      const pos = t.notes[0].position as { path: string; line: number; side: string }
      return {
        id: t.id, isResolved: m.resolved.includes(t.id), isOutdated: false, path: pos.path, line: pos.line, originalLine: pos.line, diffSide: pos.side,
        comments: { nodes: t.notes.map((n) => ({ databaseId: n.id, body: n.body, createdAt: n.created, author: { login: n.author } })) },
      }
    })
    return { body: { data: { repository: { pullRequest: { reviewThreads: { pageInfo: { hasNextPage: false, endCursor: null }, nodes } } } } } }
  }
  for (const [name, fn] of [['unresolveReviewThread', false], ['resolveReviewThread', true]] as const) {
    if (!query.includes(`${name}(`)) continue
    for (const p of projects) for (const m of p.mrs) if (m.notes.some((n) => n.thread === v.id)) {
      m.resolved = fn ? [...m.resolved, v.id] : m.resolved.filter((t) => t !== v.id)
      return ok
    }
    return noNode
  }
  if (!pr) return noNode
  if (query.includes('convertPullRequestToDraft(')) pr.m.draft = true
  else if (query.includes('markPullRequestReadyForReview(')) pr.m.draft = false
  else if (query.includes('enablePullRequestAutoMerge(')) pr.m.autoMerge = true
  else return { body: { errors: [{ message: 'unknown query' }] } }
  return ok
}

function ghe(method: string, rest: string, body: any, url: URL): Reply {
  if (rest === '/graphql' && method === 'POST') return graphql(body)
  rest = rest.replace(/^\/v3/, '')
  if (rest === '/user') return { body: { login: user } }
  const rm = rest.match(/^\/repos\/([^/]+\/[^/]+)(\/.*)?$/)
  const p = rm && project('ghe', rm[1])
  if (!rm || !p) return ghMissing
  const q = url.searchParams
  const r = rm[2] ?? ''
  const is = route(method, r)
  let x: RegExpMatchArray | null
  if (is('GET', /^$/)) return { body: ghRepo(p) }
  if (is('GET', /^\/pulls$/)) {
    const state = q.get('state') ?? 'open'
    const list = p.mrs.filter((m) => (state === 'all' || ghPr(p, m).state === state) && (!q.has('head') || q.get('head') === `${p.path.split('/')[0]}:${m.sourceBranch}`))
    return paged(list.reverse().map((m) => ghPr(p, m)), url)
  }
  if (is('POST', /^\/pulls$/)) {
    const head = String(body.head).split(':').pop()!
    return { status: 201, body: ghPr(p, addMr(p, { title: body.title, description: body.body ?? '', draft: !!body.draft, sourceBranch: head, targetBranch: body.base })) }
  }
  if ((x = is('GET', /^\/git\/ref\/heads\/(.+)$/))) {
    const sha = p.branches[decodeURIComponent(x[1])]
    return sha ? { body: { ref: `refs/heads/${x[1]}`, object: { type: 'commit', sha } } } : ghMissing
  }
  if ((x = is('DELETE', /^\/git\/refs\/heads\/(.+)$/))) {
    const b = decodeURIComponent(x[1])
    if (!p.branches[b]) return { status: 422, body: { message: 'Reference does not exist' } }
    delete p.branches[b]
    return { status: 204 }
  }
  if (is('GET', /^\/actions\/runs$/) || is('GET', /^\/actions\/workflows\/[^/]+\/runs$/)) {
    const list = p.pipelines.filter((pl) => (!q.has('branch') || pl.ref === q.get('branch')) && (!q.has('head_sha') || pl.sha === q.get('head_sha')) &&
      (!q.has('event') || q.get('event') === (pl.source ?? 'push')))
    return paged(list.map((pl) => ghRun(p, pl)), url, 'workflow_runs')
  }
  if (is('GET', /^\/actions\/workflows$/)) return { body: { total_count: 1, workflows: [{ id: 1, name: 'CI', path: '.github/workflows/ci.yml', state: 'active' }] } }
  if (is('POST', /^\/actions\/workflows\/[^/]+\/dispatches$/)) {
    newPipeline(p, body.ref, 'workflow_dispatch')
    return { status: 204 }
  }
  if ((x = r.match(/^\/actions\/runs\/(\d+)(\/[\w-]+)?$/))) {
    const pl = p.pipelines.find((y) => y.id === Number(x![1]))
    if (!pl) return ghMissing
    if (method === 'GET' && x[2] === '/jobs') return paged(pl.jobs.map((j) => ghJob(p, pl, j)), url, 'jobs')
    if (method === 'GET' && !x[2]) return { body: ghRun(p, pl) }
    const rerun = (pick: (j: SeedJob) => boolean) => {
      for (const j of pl.jobs) if (pick(j)) Object.assign(j, { status: 'pending', log: '' })
      pl.status = 'pending'
      return { status: 201, body: {} }
    }
    if (method === 'POST' && x[2] === '/rerun') return rerun(() => true)
    if (method === 'POST' && x[2] === '/rerun-failed-jobs') return rerun((j) => j.status === 'failed')
    if (method === 'POST' && x[2] === '/cancel') {
      cancel(pl)
      return { status: 202, body: {} }
    }
  }
  if ((x = r.match(/^\/actions\/jobs\/(\d+)(\/\w+)?$/))) {
    const hit = findJob(p, Number(x[1]))
    if (!hit) return ghMissing
    const { pl, j } = hit
    if (method === 'GET' && !x[2]) return { body: ghJob(p, pl, j) }
    if (method === 'GET' && x[2] === '/logs') {
      if (ACTIVE.includes(j.status)) return ghMissing
      return { status: 302, headers: { Location: `${origin()}/__blob/${p.id}/${j.id}?sig=fake` } }
    }
    if (method === 'POST' && x[2] === '/rerun') {
      Object.assign(j, { status: 'pending', log: '' })
      return { status: 201, body: {} }
    }
  }
  if ((x = r.match(/^\/issues\/(\d+)\/comments$/))) {
    const m = findMr(p, x[1])
    if (!m) return ghMissing
    if (method === 'GET') return paged(m.notes.filter((n) => !n.position && n.thread === String(n.id)).map(ghComment), url)
    if (method === 'POST') return { status: 201, body: ghComment(note(m, null, body.body)) }
  }
  const m = (x = r.match(/^\/pulls\/(\d+)(\/.*)?$/)) && findMr(p, x[1])
  if (!x || !m) return ghMissing
  const at = route(method, x[2] ?? '')
  if (at('GET', /^$/)) return { body: ghPr(p, m) }
  if (at('PATCH', /^$/)) {
    if (body.state) m.state = body.state === 'closed' ? 'closed' : 'open'
    return { body: ghPr(p, m) }
  }
  if (at('GET', /^\/reviews$/)) return paged(m.reviews.map(ghReview), url)
  if (at('POST', /^\/reviews$/)) {
    const state = { APPROVE: 'APPROVED', REQUEST_CHANGES: 'CHANGES_REQUESTED', COMMENT: 'COMMENTED' }[body.event as string] ?? 'PENDING'
    if (state === 'CHANGES_REQUESTED' && !body.body) return { status: 422, body: { message: 'Unprocessable Entity', errors: ['Review body is required'] } }
    const review = { id: ++next, state, body: body.body ?? '', user }
    m.reviews.push(review)
    return { body: ghReview(review) }
  }
  if (at('POST', /^\/comments$/)) {
    if (body.commit_id !== m.headSha) return { status: 422, body: { message: 'Validation Failed', errors: ['commit_id is not part of the pull request'] } }
    const id = ++next
    const n = note(m, `PRRT_${id}`, body.body, { path: body.path, line: body.line, side: body.side })
    return { status: 201, body: { id: n.id, body: n.body, path: body.path, line: body.line, side: body.side, commit_id: body.commit_id } }
  }
  if ((x = at('POST', /^\/comments\/(\d+)\/replies$/))) {
    const first = m.notes.find((n) => n.id === Number(x![1]) && n.position)
    if (!first) return ghMissing
    return { status: 201, body: { id: note(m, first.thread, body.body).id } }
  }
  if (at('PUT', /^\/merge$/)) {
    if (m.state !== 'open') return { status: 405, body: { message: 'Pull Request is not mergeable' } }
    if (body.sha && body.sha !== m.headSha) return { status: 409, body: { message: 'Head branch was modified. Review and try the merge again.' } }
    m.state = 'merged'
    return { body: { sha: m.headSha, merged: true, message: 'Pull Request successfully merged' } }
  }
  return ghMissing
}

createServer(async (req, res) => {
  let raw = ''
  for await (const chunk of req) raw += chunk
  const send = (r: Reply) => {
    res.writeHead(r.status ?? 200, { 'Content-Type': r.text !== undefined ? 'text/plain' : 'application/json', ...r.headers })
    res.end(r.text ?? (r.body === undefined ? '' : JSON.stringify(r.body)))
  }
  const url = new URL(req.url!, origin())
  const { pathname } = url
  const path = decodeURIComponent(pathname)
  const method = req.method ?? 'GET'
  const body = raw ? JSON.parse(raw) : {}
  if (pathname === '/health') return send({ body: { projects: projects.length } })
  if (pathname === '/__writes') return send({ body: writes })
  if (pathname === '/__requests') return send({ body: requests })
  if (method === 'POST' && pathname === '/__delay') {
    delay = body.ms
    return send({ body: null })
  }
  if (method === 'POST' && pathname === '/reset') {
    reset(body)
    return send({ body: null })
  }
  if (method === 'POST' && pathname === '/__append') {
    const p = project(body.forge, body.path)
    const j = p && findJob(p, body.job)?.j
    if (!j) return send({ status: 404, body: { message: 'no such job' } })
    j.log += body.text ?? ''
    if (body.status) j.status = body.status
    return send({ body: null })
  }
  const blob = pathname.match(/^\/__blob\/(\d+)\/(\d+)$/)
  if (blob) {
    if (req.headers.authorization) return send({ status: 400, body: { message: 'the signed URL must not get the token' } })
    const p = projects.find((x) => x.id === Number(blob[1]))
    const j = p && findJob(p, Number(blob[2]))?.j
    return send(j ? { text: j.log } : ghMissing)
  }
  const gl = pathname.match(/^\/gitlab\/api\/v4(\/.*)$/)
  const glq = pathname === '/gitlab/api/graphql'
  const gh = pathname.match(/^\/ghe\/api(\/.*)$/)
  if (gl || glq || gh) {
    requests.push({ method, path })
    await new Promise((r) => setTimeout(r, delay))
  }
  if ((gl || glq) && req.headers['private-token'] !== 'gitlab-token') return send({ status: 401, body: { message: '401 Unauthorized' } })
  if (gh && req.headers.authorization !== 'Bearer ghe-token') return send({ status: 401, body: { message: 'Bad credentials' } })
  const read = typeof body.query === 'string' && body.query.startsWith('query')
  if (method !== 'GET' && (gl || gh) && !read) writes.push({ method, path, body: raw ? body : null })
  if (glq && method === 'POST') return send(glGraphql(body))
  if (gl) return send(gitlab(method, gl[1], body, url))
  if (gh) return send(ghe(method, gh[1], body, url))
  send({ status: 404, body: { message: '404 Not Found' } })
}).listen(Number(process.argv[2]), '127.0.0.1')
