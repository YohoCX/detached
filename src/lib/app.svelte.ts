import { open as pickFolders } from '@tauri-apps/plugin-dialog'
import { SvelteMap } from 'svelte/reactivity'
import { actions, register, runAction } from './actions'
import { api, newId, type CheckoutResult, type CheckoutTarget, type RepoInfo, type RepoStatus, type Workspace } from './api'
import * as nav from './nav'
import { pool, syncAll, type Outcome } from './sync'

export type PaletteMode = 'file' | 'action' | 'vcs' | 'everywhere' | 'recent'
export type BranchesView = 'checkout' | 'delete' | 'new'
export interface Op { id: number; repo: string; label: string; line: string }
export interface Notice {
  id: number
  level: 'info' | 'warn' | 'error'
  title: string
  body?: string
  repo?: string
  files?: string[]
}

export const app = $state({
  repos: [] as RepoInfo[],
  workspaces: [] as Workspace[],
  fileNesting: {} as Record<string, string[]>,
  current: null as string | null,
  ops: [] as Op[],
  notices: [] as Notice[],
  message: null as { text: string; error: boolean } | null,
  sync: null as { kind: 'fetch' | 'pull'; done: number; total: number } | null,
  sidebar: true,
  tool: 'changes' as 'changes' | 'files' | 'mrs' | null,
  bottom: null as 'log' | 'pipelines' | null,
  palette: null as PaletteMode | null,
  paletteAll: false,
  branches: null as BranchesView | null,
  settings: false,
})

/** RepoStatus, or the error text when status failed. */
export const statuses = new SvelteMap<string, RepoStatus | string>()
export const outcomes = new SvelteMap<string, Outcome>()
const recentBranches = new Map<string, string[]>()

export const nameOf = (repo: string) => app.repos.find((r) => r.path === repo)?.name ?? repo
export const statusOf = (repo: string | null) => {
  const s = repo ? statuses.get(repo) : undefined
  return typeof s === 'object' ? s : null
}
/** The status dot's color token: error, local changes, diverged from upstream, no upstream, in sync. */
export const toneOf = (s: RepoStatus | string | undefined) =>
  typeof s === 'string' ? 'error'
  : !s ? null
  : s.changes.length || s.inProgress ? 'warning'
  : s.ahead || s.behind ? 'accent'
  : s.branch && !s.upstream ? 'deleted'
  : 'success'
export const recentOf = (repo: string) => recentBranches.get(repo) ?? []

export function run(id: string) {
  if (!runAction(id)) say(`${actions.get(id)?.title ?? id} is not available yet`)
}

export function say(text: string, error = false) {
  app.message = { text, error }
}

let noticeId = 0
export function notify(n: Omit<Notice, 'id'>) {
  const id = ++noticeId
  app.notices.push({ id, ...n })
  if (n.level === 'info') setTimeout(() => dismiss(id), 6000)
}
export function dismiss(id: number) {
  app.notices = app.notices.filter((n) => n.id !== id)
}

export async function refresh(repo: string) {
  try {
    statuses.set(repo, await api.repoStatus(repo))
  } catch (e) {
    statuses.set(repo, String(e))
  }
}

function setRepos(list: RepoInfo[]) {
  app.repos = list
  const paths = new Set(list.map((r) => r.path))
  for (const repo of [...statuses.keys()]) if (!paths.has(repo)) statuses.delete(repo)
  for (const repo of [...outcomes.keys()]) if (!paths.has(repo)) outcomes.delete(repo)
  if (!app.current || !paths.has(app.current)) app.current = list[0]?.path ?? null
}

let refreshing = false
let again = false
/** Rescans the folders first: repos and worktrees come and go on disk. */
export async function refreshAll() {
  if (refreshing) return void (again = true)
  refreshing = true
  try {
    do {
      again = false
      setRepos(await api.listRepos())
      await pool(app.repos.map((r) => r.path), 8, refresh)
    } while (again)
  } catch (e) {
    say(String(e), true)
  } finally {
    refreshing = false
  }
}

export async function loadRepos(list?: RepoInfo[]) {
  try {
    if (list) setRepos(list)
    const s = await api.getSettings()
    app.workspaces = s.workspaces
    app.fileNesting = s.fileNesting
  } catch (e) {
    return say(String(e), true)
  }
  await refreshAll()
}

export async function settingsCall(f: () => Promise<RepoInfo[]>) {
  try {
    await loadRepos(await f())
  } catch (e) {
    say(String(e), true)
  }
}

export const activeWorkspaces = () => app.workspaces.filter((w) => w.active)

/** To the first active workspace by default; the first folder ever makes a "Default" one. */
export async function addFolders(workspace = activeWorkspaces()[0]?.name ?? 'Default') {
  const picked = await pickFolders({ directory: true, multiple: true })
  for (const path of picked ?? []) await settingsCall(() => api.addFolder(workspace, path))
}
export const removeFolder = (workspace: string, path: string) => settingsCall(() => api.removeFolder(workspace, path))
export const excludeRepo = (workspace: string, path: string) => settingsCall(() => api.excludeRepo(workspace, path))

/** Runs a cancellable network operation with its progress in the status bar, then refreshes the repo. */
export async function runOp<T>(
  repo: string,
  label: string,
  f: (opId: number, onProgress: (line: string) => void) => Promise<T>,
): Promise<T> {
  const op: Op = { id: newId(), repo, label, line: '' }
  app.ops.push(op)
  const live = app.ops[app.ops.length - 1]
  try {
    return await f(op.id, (line) => (live.line = line))
  } finally {
    app.ops = app.ops.filter((o) => o.id !== op.id)
    refresh(repo)
  }
}

export const cancelOp = (id: number) => api.cancelOp(id).catch((e) => say(String(e), true))

export function prune(repo: string) {
  runOp(repo, 'Prune', (id, p) => api.prune(repo, id, p)).then(
    () => say(`${nameOf(repo)}: pruned stale remote branches`),
    (e) => say(String(e), true),
  )
}

export const branchNames = (repo: string) =>
  api.listBranches(repo).then((b) => [...b.local.map((l) => l.name), ...b.remote.map((r) => `${r.remote}/${r.name}`)])

const fetchRepo = (repo: string) => runOp(repo, 'Fetch', (id, p) => api.fetch(repo, id, p))
const pullRepo = (repo: string) => runOp(repo, 'Pull', (id, p) => api.pull(repo, id, p))

/** A sync result in the repo's row: failures stay until the next sync, successes fade after a few seconds. */
function showOutcome(o: Outcome) {
  outcomes.set(o.repo, o)
  if (o.ok) setTimeout(() => outcomes.get(o.repo) === o && outcomes.delete(o.repo), 6000)
}

export async function syncOne(kind: 'fetch' | 'pull', repo: string | null) {
  if (!repo) return
  const [o] = await syncAll([repo], kind === 'fetch' ? fetchRepo : pullRepo)
  showOutcome(o)
  say(`${nameOf(repo)}: ${o.text}`, !o.ok)
}

export async function syncRepos(kind: 'fetch' | 'pull', repos = app.repos.map((r) => r.path)) {
  if (app.sync) return
  app.sync = { kind, done: 0, total: repos.length }
  const sync = app.sync
  const results = await syncAll(repos, kind === 'fetch' ? fetchRepo : pullRepo, (o) => {
    showOutcome(o)
    sync.done++
  })
  app.sync = null
  const failed = results.filter((o) => !o.ok)
  const verb = kind === 'fetch' ? 'Fetched' : 'Pulled'
  say(`${verb} ${results.length - failed.length} of ${results.length} repositories`, failed.length > 0)
  if (failed.length)
    notify({
      level: 'warn',
      title: `${kind === 'fetch' ? 'Fetch' : 'Pull'} all: ${failed.length} not updated`,
      body: failed.map((o) => `${nameOf(o.repo)}: ${o.text}`).join('\n'),
    })
}

export async function checkout(repo: string, target: CheckoutTarget) {
  const from = statusOf(repo)?.branch
  const name = target.kind === 'rev' ? target.sha.slice(0, 10) : target.name
  let r: CheckoutResult
  say(`Checking out ${name}…`)
  try {
    r = await api.checkout(repo, target)
  } catch (e) {
    return notify({ level: 'error', title: `Checkout of ${name} failed`, body: String(e) })
  } finally {
    refresh(repo)
  }
  if ((r.kind === 'switched' || r.kind === 'carried') && target.kind !== 'rev') {
    const rest = recentOf(repo).filter((b) => b !== name && b !== from)
    recentBranches.set(repo, [name, ...(from ? [from] : []), ...rest].slice(0, 6))
  }
  const n = checkoutNotice(repo, name, r)
  notify(n)
  say(n.title, n.level === 'error')
}

function checkoutNotice(repo: string, name: string, r: CheckoutResult): Omit<Notice, 'id'> {
  switch (r.kind) {
    case 'switched':
    case 'carried':
      return {
        level: r.warning ? 'warn' : 'info',
        title: `Checked out ${name}${r.kind === 'carried' ? ', local changes carried over' : ''}`,
        body: r.warning ?? undefined,
      }
    case 'refused':
      return { level: 'error', title: `Checkout of ${name} refused`, body: r.error }
    case 'stashKept':
      return {
        level: 'error',
        title: r.onOriginalBranch
          ? `Checkout of ${name} failed, your changes are kept in a stash`
          : `Checked out ${name}, your changes did not apply cleanly`,
        body: `${r.error}\nStash kept: ${r.stashMessage} (${r.stash.slice(0, 10)})` +
          (r.conflicts.length ? '\nConflicts:' : ''),
        repo,
        files: r.conflicts,
      }
  }
}

export function openPalette(mode: PaletteMode) {
  if (app.palette === mode && mode === 'file') app.paletteAll = !app.paletteAll
  else app.palette = mode
}

export function registerActions(): () => void {
  const cur = () => app.current
  const offs = [
    register('search.everywhere', () => openPalette('everywhere')),
    register('file.goto', () => openPalette('file')),
    register('action.find', () => openPalette('action')),
    register('vcs.popup', () => openPalette('vcs')),
    register('file.recent', () => openPalette('recent')),
    register('nav.back', nav.back),
    register('nav.forward', nav.forward),
    register('panel.project', () => (app.tool = app.tool === 'files' ? null : 'files')),
    register('app.settings', () => (app.settings = true)),
    register('panel.log', () => (app.bottom = app.bottom === 'log' ? null : 'log')),
    register('panel.pipelines', () => (app.bottom = app.bottom === 'pipelines' ? null : 'pipelines')),
    register('panel.mergeRequests', () => (app.tool = app.tool === 'mrs' ? null : 'mrs')),
    register('vcs.update', () => syncOne('pull', cur())),
    register('vcs.fetch', () => syncOne('fetch', cur())),
    register('vcs.fetchAll', () => syncRepos('fetch')),
    register('vcs.pullAll', () => syncRepos('pull')),
    register('vcs.checkout', () => (app.branches = 'checkout')),
    register('vcs.newBranch', () => (app.branches = 'new')),
    register('vcs.deleteBranch', () => (app.branches = 'delete')),
    register('vcs.prune', () => app.current && prune(app.current)),
  ]
  return () => offs.forEach((off) => off())
}
