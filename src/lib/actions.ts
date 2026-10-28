// Every M1 action id from docs/KEYMAP.md. Components register `run` for the actions they own.

export type Group = 'navigate' | 'search' | 'vcs' | 'diff' | 'view'

export interface Action {
  id: string
  title: string
  group: Group
  run?: () => void
}

const defs: Action[] = [
  { id: 'search.everywhere', title: 'Search Everywhere', group: 'search' },
  { id: 'file.goto', title: 'Go to File', group: 'search' },
  { id: 'find.inFiles', title: 'Find in Files', group: 'search' },
  { id: 'action.find', title: 'Find Action', group: 'search' },
  { id: 'file.recent', title: 'Recent Files', group: 'navigate' },
  { id: 'editor.gotoLine', title: 'Go to Line', group: 'navigate' },
  { id: 'nav.back', title: 'Back', group: 'navigate' },
  { id: 'nav.forward', title: 'Forward', group: 'navigate' },
  { id: 'panel.project', title: 'Project Tree', group: 'view' },
  { id: 'vcs.update', title: 'Update Project (Pull)', group: 'vcs' },
  { id: 'vcs.popup', title: 'VCS Operations', group: 'vcs' },
  { id: 'vcs.fetch', title: 'Fetch', group: 'vcs' },
  { id: 'vcs.fetchAll', title: 'Fetch All Repositories', group: 'vcs' },
  { id: 'vcs.pullAll', title: 'Pull All Repositories', group: 'vcs' },
  { id: 'vcs.checkout', title: 'Checkout Branch…', group: 'vcs' },
  { id: 'vcs.newBranch', title: 'New Branch…', group: 'vcs' },
  { id: 'vcs.deleteBranch', title: 'Delete Branch…', group: 'vcs' },
  { id: 'vcs.prune', title: 'Prune Remote Branches', group: 'vcs' },
  { id: 'diff.show', title: 'Show Diff', group: 'diff' },
  { id: 'diff.next', title: 'Next Change', group: 'diff' },
  { id: 'diff.prev', title: 'Previous Change', group: 'diff' },
  { id: 'file.openInZed', title: 'Open in Zed', group: 'navigate' },
]

export const actions = new Map(defs.map((a) => [a.id, a]))

export function register(id: string, run: () => void): () => void {
  const a = actions.get(id)
  if (!a) throw new Error(`unknown action ${id}`)
  a.run = run
  return () => {
    if (a.run === run) a.run = undefined
  }
}

export function runAction(id: string): boolean {
  const run = actions.get(id)?.run
  run?.()
  return !!run
}
