import { SvelteMap } from 'svelte/reactivity'
import type { FileChange, Side } from './api'
import { setOpener, type Location } from './nav'

export type Shown =
  | ({ kind: 'file' } & Location)
  | { kind: 'diff'; repo: string; path: string; origPath: string | null; left: Side; right: Side }

export const view = $state({
  shown: null as Shown | null,
  /** The selected row in Changes, for Show Diff. */
  change: null as { repo: string; change: FileChange } | null,
  find: false,
})

/** Repos whose Changes panel compares with a base branch; '' means the default branch. */
export const compare = new SvelteMap<string, string>()

/** The ref each repo's tree browses; absent is the working tree. */
export const refs = new SvelteMap<string, string>()
export const refOf = (repo: string | null) => (repo && refs.get(repo)) || null

setOpener((loc) => {
  view.shown = { kind: 'file', ...loc }
})

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
