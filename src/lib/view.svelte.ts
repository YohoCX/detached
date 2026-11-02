import { SvelteMap } from 'svelte/reactivity'
import type { DiffSide, FileChange, Project, Side } from './api'
import { open, setOpener, type Doc, type Location } from './nav'

/** What Viewer shows: a file, or a diff (an MR's has rev sides). */
export type FileDoc =
  | ({ kind: 'file' } & Location)
  | { kind: 'diff'; repo: string; path: string; origPath: string | null; left: Side; right: Side }
/** A view zone under `afterLine` on a diff's old or new side, as tall as its node. */
export interface Zone { side: DiffSide; afterLine: number; node: HTMLElement }
export type Shown = FileDoc | ({ repo: string } & Doc)

export const view = $state({
  shown: null as Shown | null,
  /** The selected row in Changes, for Show Diff. */
  change: null as { repo: string; change: FileChange } | null,
  find: false,
  /** Bumped to move keyboard focus into the diff once it has loaded. */
  focusDiff: 0,
})

/** Repos whose Changes panel compares with a base branch; '' means the default branch. */
export const compare = new SvelteMap<string, string>()

/** The ref each repo's tree browses; absent is the working tree. */
export const refs = new SvelteMap<string, string>()
export const refOf = (repo: string | null) => (repo && refs.get(repo)) || null

setOpener((loc) => {
  view.shown = loc.doc ? { repo: loc.repo, ...loc.doc } : { kind: 'file', ...loc }
})

const at = (repo: string, doc: Doc): Location => ({ repo, ref: null, path: '', line: null, doc })
export const showMr = (repo: string, number: number) => open(at(repo, { kind: 'mr', number }))
export const showJob = (repo: string, project: Project, id: number, name: string) =>
  open(at(repo, { kind: 'job', project, id, name }))

export function showDiff(repo: string, path: string, origPath: string | null = null, left: Side = { kind: 'head' }) {
  view.shown = { kind: 'diff', repo, path, origPath, left, right: { kind: 'worktree' } }
}

export function sideName(s: Side): string {
  switch (s.kind) {
    case 'head': return 'HEAD'
    case 'index': return 'Staged'
    case 'worktree': return 'Working tree'
    case 'rev': return /^[0-9a-f]{40}$/.test(s.rev) ? s.rev.slice(0, 10) : s.rev
  }
}
