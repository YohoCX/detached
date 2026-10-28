// The command contract with src-tauri. Types mirror src-tauri/src/types.rs; change both together.

import { Channel, invoke } from '@tauri-apps/api/core'

export interface RepoInfo { path: string; name: string; folder: string }
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
  gitPath: string | null
}
export interface EnvInfo { gitPath: string; gitVersion: string; path: string }

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
  setGitPath: (path: string | null) => invoke<EnvInfo>('set_git_path', { path }),
  envInfo: () => invoke<EnvInfo>('env_info'),
  keymapOverrides: () => invoke<Record<string, string>>('keymap_overrides'),

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
  fileDiff: (repo: string, path: string, origPath: string | null, left: Side, right: Side) =>
    invoke<FileDiff>('file_diff', { repo, path, origPath, left, right }),
  listFiles: (repo: string, rev: string | null) => invoke<FileEntry[]>('list_files', { repo, rev }),
  readFile: (repo: string, rev: string | null, path: string) => invoke<Content>('read_file', { repo, rev, path }),
  openInZed: (repo: string, path: string, line: number | null) => invoke<void>('open_in_zed', { repo, path, line }),

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
}
