// The command contract with src-tauri. Types mirror src-tauri/src/types.rs; change both together.

import { Channel, invoke } from '@tauri-apps/api/core'

export interface RepoInfo { path: string; name: string; folder: string; worktreeOf: string | null }
export interface CommitSummary { sha: string; subject: string; author: string; time: number }
export interface FileChange {
  path: string
  origPath: string | null
  index: string
  worktree: string
  conflicted: boolean
}
export type InProgress = 'merge' | 'rebase' | 'cherryPick' | 'revert' | 'bisect'
export interface RepoStatus {
  branch: string | null
  head: string | null
  upstream: string | null
  ahead: number
  behind: number
  changes: FileChange[]
  lastCommit: CommitSummary | null
  inProgress: InProgress | null
}
export interface LocalBranch {
  name: string
  upstream: string | null
  upstreamGone: boolean
  ahead: number
  behind: number
  current: boolean
  lastCommit: CommitSummary
}
export interface RemoteBranch { remote: string; name: string; lastCommit: CommitSummary }
export interface Branches { local: LocalBranch[]; remote: RemoteBranch[] }
export type CheckoutTarget =
  | { kind: 'local'; name: string }
  | { kind: 'remote'; remote: string; name: string }
  | { kind: 'rev'; sha: string }
export type CheckoutResult =
  | { kind: 'switched'; warning: string | null }
  | { kind: 'carried'; warning: string | null }
  | {
      kind: 'stashKept'
      stash: string
      stashMessage: string
      conflicts: string[]
      onOriginalBranch: boolean
      error: string
    }
  | { kind: 'refused'; error: string }
export type DeleteResult = { kind: 'deleted' } | { kind: 'notMerged'; commits: CommitSummary[] }
export type PullResult =
  | { kind: 'upToDate' }
  | { kind: 'fastForwarded'; from: string; to: string }
  | { kind: 'diverged'; ahead: number; behind: number }
  | { kind: 'noUpstream' }
export type Provider = 'github' | 'githubEnterprise' | 'gitlab' | 'gitlabSelfHosted'
export interface Account { id: string; provider: Provider; baseUrl: string; username: string; label: string }
export interface AccountInput extends Omit<Account, 'id'> { id: string | null; token: string | null }
export type VerifyResult =
  | { kind: 'ok'; user: string }
  | { kind: 'failed'; status: number | null; error: string }
export interface HostAccount { host: string; remotes: string[]; accountId: string | null; ambiguous: boolean }
export type Side = { kind: 'head' } | { kind: 'index' } | { kind: 'worktree' } | { kind: 'rev'; rev: string }
export type Content =
  | { kind: 'text'; text: string }
  | { kind: 'binary' }
  | { kind: 'tooLarge'; size: number }
  | { kind: 'missing' }
  | { kind: 'submodule'; commit: string }
  | { kind: 'conflicted'; text: string }
export interface FileDiff { left: Content; right: Content }
export interface CompareBase { base: string; mergeBase: string; ahead: number; behind: number }
export interface FileEntry { path: string; change: FileChange | null }
export interface FileMatch { repo: string; path: string; score: number; indices: number[] }
export interface FindQuery {
  text: string
  caseSensitive: boolean
  wholeWord: boolean
  regex: boolean
  glob: string | null
}
export type FindEvent =
  | { kind: 'match'; queryId: number; repo: string; path: string; line: number; text: string; ranges: [number, number][] }
  | { kind: 'done'; queryId: number; truncated: boolean }
  | { kind: 'error'; queryId: number; error: string }
export interface Settings {
  folders: string[]
  excludedRepos: string[]
  repoAccounts: Record<string, string>
  /** Repo -> host -> account id. */
  forgeAccounts: Record<string, Record<string, string>>
  /** Repo -> the remote whose project has the MRs. */
  mrRemotes: Record<string, string>
  gitPath: string | null
}
export interface EnvInfo { gitPath: string; gitVersion: string; path: string }

/** `list_changes` from this for a root commit. */
export const EMPTY_TREE = '4b825dc642cb6eb9a060e54bf8d69288fbee4904'
export type LogRefs = { kind: 'all' } | { kind: 'head' } | { kind: 'branch'; name: string }
export interface LogQuery { refs: LogRefs; path: string | null; text: string | null; author: string | null }
/** A line in one half of a graph row, from column `from` to column `to`. */
export interface Edge { from: number; to: number; color: number }
export interface LogRow {
  sha: string
  parents: string[]
  author: string
  email: string
  time: number
  /** `HEAD -> main`, `origin/main`, `tag: v1`. */
  refs: string[]
  subject: string
  column: number
  color: number
  /** Top edge to the dot's middle, and middle to bottom edge; empty under text and author filters. */
  top: Edge[]
  bottom: Edge[]
}
export type LogEvent =
  | { kind: 'rows'; queryId: number; rows: LogRow[] }
  | { kind: 'done'; queryId: number; truncated: boolean }
  | { kind: 'error'; queryId: number; error: string }
export interface CommitDetails {
  sha: string
  parents: string[]
  author: string
  authorEmail: string
  authorTime: number
  committer: string
  committerEmail: string
  committerTime: number
  message: string
}
export interface Hunk { oldStart: number; oldLines: number; newStart: number; newLines: number }

export interface Project { host: string; path: string }
export interface ForgeHosts { hosts: HostAccount[]; mrRemote: string | null }
export type MrFilter = 'open' | 'mine' | 'reviewRequested' | 'assigned' | 'all'
export type MrState = 'open' | 'closed' | 'merged'
export type MergeMethod = 'merge' | 'squash' | 'rebase'
export type PipelineStatus = 'pending' | 'running' | 'success' | 'failed' | 'canceled' | 'skipped' | 'manual'
export interface MergeRequest {
  provider: Provider
  number: number
  title: string
  description: string
  state: MrState
  draft: boolean
  author: string
  sourceProject: Project
  sourceBranch: string
  targetProject: Project
  targetBranch: string
  baseSha: string
  headSha: string
  labels: string[]
  reviewers: string[]
  approvedBy: string[]
  approved: boolean
  blocking: string | null
  mergeMethods: MergeMethod[]
  canDeleteSourceBranch: boolean
  autoMerge: boolean
  pipeline: PipelineStatus | null
  webUrl: string
  projectUrl: string
  updated: number
}
export type DiffSide = 'old' | 'new'
export interface Position { path: string; side: DiffSide; line: number; outdated: boolean }
export interface Note { id: string; author: string; body: string; time: number }
export interface Thread {
  id: string
  resolvable: boolean
  resolved: boolean
  replyable: boolean
  replyTo: string
  position: Position | null
  notes: Note[]
}
export type MrAction =
  | { kind: 'comment'; body: string }
  | { kind: 'reply'; replyTo: string; body: string }
  | {
      kind: 'lineComment'
      body: string
      path: string
      oldPath: string
      oldLine: number | null
      newLine: number | null
      headSha: string
    }
  | { kind: 'resolve'; threadId: string; resolved: boolean }
  | { kind: 'approve' }
  | { kind: 'unapprove' }
  | { kind: 'requestChanges'; body: string }
  | { kind: 'merge'; method: MergeMethod; deleteSourceBranch: boolean; autoMerge: boolean; headSha: string }
  | { kind: 'setDraft'; draft: boolean }
  | { kind: 'close' }
  | { kind: 'reopen' }
export interface NewMr {
  sourceProject: Project
  sourceBranch: string
  targetProject: Project
  targetBranch: string
  title: string
}
export interface CreateMr { title: string; description: string; targetBranch: string; draft: boolean }
export interface Pipeline {
  id: number
  project: Project
  status: PipelineStatus
  ref: string
  sha: string
  source: string
  created: number
  started: number | null
  finished: number | null
  webUrl: string
  cancellable: boolean
  retryable: boolean
}
export interface Step { name: string; status: PipelineStatus }
export interface Job {
  id: number
  name: string
  status: PipelineStatus
  duration: number | null
  allowFailure: boolean
  cancellable: boolean
  playable: boolean
  retryable: boolean
  liveLog: boolean
  webUrl: string
  steps: Step[]
}
export interface Stage { name: string; status: PipelineStatus; jobs: Job[] }
export type PipelineAction = 'retry' | 'retryFailed' | 'cancel'
export type JobAction = 'retry' | 'cancel' | 'play'
export interface Workflow { id: string; name: string }
export interface RunPipeline { project: Project; branch: string; workflow: string | null; variables: [string, string][] }
export interface JobLogChunk { text: string; next: number; complete: boolean }

let nextId = 1
/** Id for a cancellable operation or a find query. */
export const newId = () => nextId++

function channel<T>(on: (msg: T) => void): Channel<T> {
  const c = new Channel<T>()
  c.onmessage = on
  return c
}

type Progress = (line: string) => void
const noop: Progress = () => {}

export const api = {
  getSettings: () => invoke<Settings>('get_settings'),
  listRepos: () => invoke<RepoInfo[]>('list_repos'),
  addFolder: (path: string) => invoke<RepoInfo[]>('add_folder', { path }),
  removeFolder: (path: string) => invoke<RepoInfo[]>('remove_folder', { path }),
  excludeRepo: (path: string) => invoke<RepoInfo[]>('exclude_repo', { path }),
  includeRepo: (path: string) => invoke<RepoInfo[]>('include_repo', { path }),
  setRepoAccount: (repo: string, accountId: string | null) =>
    invoke<Settings>('set_repo_account', { repo, accountId }),
  setForgeAccount: (repo: string, host: string, accountId: string | null) =>
    invoke<Settings>('set_forge_account', { repo, host, accountId }),
  setMrRemote: (repo: string, remote: string | null) => invoke<Settings>('set_mr_remote', { repo, remote }),
  setGitPath: (path: string | null) => invoke<EnvInfo>('set_git_path', { path }),
  envInfo: () => invoke<EnvInfo>('env_info'),
  keymapOverrides: () => invoke<Record<string, string>>('keymap_overrides'),
  openInFinder: (path: string) => invoke<void>('open_in_finder', { path }),

  repoStatus: (repo: string) => invoke<RepoStatus>('repo_status', { repo }),
  listBranches: (repo: string) => invoke<Branches>('list_branches', { repo }),
  checkout: (repo: string, target: CheckoutTarget) => invoke<CheckoutResult>('checkout', { repo, target }),
  createBranch: (repo: string, name: string, start: string, checkout: boolean) =>
    invoke<void>('create_branch', { repo, name, start, checkout }),
  setUpstream: (repo: string, branch: string, upstream: string | null) =>
    invoke<void>('set_upstream', { repo, branch, upstream }),
  deleteBranch: (repo: string, name: string, force: boolean) =>
    invoke<DeleteResult>('delete_branch', { repo, name, force }),
  deleteRemoteBranch: (repo: string, remote: string, name: string, opId: number, onProgress = noop) =>
    invoke<void>('delete_remote_branch', { repo, remote, name, opId, progress: channel(onProgress) }),
  prune: (repo: string, opId: number, onProgress = noop) =>
    invoke<void>('prune', { repo, opId, progress: channel(onProgress) }),
  fetch: (repo: string, opId: number, onProgress = noop) =>
    invoke<void>('fetch', { repo, opId, progress: channel(onProgress) }),
  pull: (repo: string, opId: number, onProgress = noop) =>
    invoke<PullResult>('pull', { repo, opId, progress: channel(onProgress) }),
  cancelOp: (opId: number) => invoke<void>('cancel_op', { opId }),

  listChanges: (repo: string, from: Side, to: Side) => invoke<FileChange[]>('list_changes', { repo, from, to }),
  compareBase: (repo: string, base: string | null) => invoke<CompareBase>('compare_base', { repo, base }),
  fileDiff: (repo: string, path: string, origPath: string | null, left: Side, right: Side) =>
    invoke<FileDiff>('file_diff', { repo, path, origPath, left, right }),
  listFiles: (repo: string, rev: string | null) => invoke<FileEntry[]>('list_files', { repo, rev }),
  readFile: (repo: string, rev: string | null, path: string) => invoke<Content>('read_file', { repo, rev, path }),
  openInZed: (repo: string, path: string, line: number | null) => invoke<void>('open_in_zed', { repo, path, line }),

  log: (repo: string, query: LogQuery, queryId: number, on: (e: LogEvent) => void) =>
    invoke<void>('log', { repo, query, queryId, events: channel(on) }),
  cancelLog: (queryId: number) => invoke<void>('cancel_log', { queryId }),
  commitDetails: (repo: string, sha: string) => invoke<CommitDetails>('commit_details', { repo, sha }),
  branchesContaining: (repo: string, sha: string) => invoke<string[]>('branches_containing', { repo, sha }),
  mergeBase: (repo: string, a: string, b: string) => invoke<string>('merge_base', { repo, a, b }),
  rangeLog: (repo: string, base: string, head: string) => invoke<CommitSummary[]>('range_log', { repo, base, head }),
  hunkRanges: (repo: string, base: string, head: string, path: string, origPath: string | null) =>
    invoke<Hunk[]>('hunk_ranges', { repo, base, head, path, origPath }),
  /** Fetches an MR's head ref and target branch without creating refs; the fetched head's SHA. */
  fetchMr: (repo: string, remote: string, headRef: string, targetBranch: string, opId: number, onProgress = noop) =>
    invoke<string>('fetch_mr', { repo, remote, headRef, targetBranch, opId, progress: channel(onProgress) }),

  listAccounts: () => invoke<Account[]>('list_accounts'),
  saveAccount: (input: AccountInput) => invoke<Account>('save_account', { input }),
  removeAccount: (id: string) => invoke<void>('remove_account', { id }),
  verifyAccount: (id: string) => invoke<VerifyResult>('verify_account', { id }),
  repoAccounts: (repo: string) => invoke<HostAccount[]>('repo_accounts', { repo }),

  quickOpen: (repos: string[], rev: string | null, query: string, limit: number) =>
    invoke<FileMatch[]>('quick_open', { repos, rev, query, limit }),
  findInFiles: (repos: string[], rev: string | null, query: FindQuery, queryId: number, on: (e: FindEvent) => void) =>
    invoke<void>('find_in_files', { repos, rev, query, queryId, events: channel(on) }),
  cancelFind: (queryId: number) => invoke<void>('cancel_find', { queryId }),

  forgeHosts: (repo: string) => invoke<ForgeHosts>('forge_hosts', { repo }),
  commitUrl: (repo: string, sha: string) => invoke<string | null>('commit_url', { repo, sha }),
  listMrs: (repo: string, filter: MrFilter) => invoke<MergeRequest[]>('list_mrs', { repo, filter }),
  /** The current branch's open MR, matched on source project and branch. */
  currentMr: (repo: string) => invoke<MergeRequest | null>('current_mr', { repo }),
  getMr: (repo: string, number: number) => invoke<MergeRequest>('get_mr', { repo, number }),
  mrThreads: (repo: string, number: number) => invoke<Thread[]>('mr_threads', { repo, number }),
  mrAction: (repo: string, number: number, action: MrAction) => invoke<void>('mr_action', { repo, number, action }),
  newMr: (repo: string) => invoke<NewMr>('new_mr', { repo }),
  createMr: (repo: string, input: CreateMr) => invoke<MergeRequest>('create_mr', { repo, input }),
  /** The current branch's pipelines from its source project, or every branch's when `all`. */
  listPipelines: (repo: string, all: boolean) => invoke<Pipeline[]>('list_pipelines', { repo, all }),
  mrPipelines: (repo: string, number: number) => invoke<Pipeline[]>('mr_pipelines', { repo, number }),
  pipelineJobs: (repo: string, project: Project, id: number) => invoke<Stage[]>('pipeline_jobs', { repo, project, id }),
  pipelineAction: (repo: string, project: Project, id: number, action: PipelineAction) =>
    invoke<void>('pipeline_action', { repo, project, id, action }),
  jobAction: (repo: string, project: Project, id: number, action: JobAction) =>
    invoke<void>('job_action', { repo, project, id, action }),
  jobLog: (repo: string, project: Project, id: number, offset: number) =>
    invoke<JobLogChunk>('job_log', { repo, project, id, offset }),
  workflows: (repo: string, project: Project) => invoke<Workflow[]>('workflows', { repo, project }),
  runPipeline: (repo: string, input: RunPipeline) => invoke<Pipeline>('run_pipeline', { repo, input }),
}
