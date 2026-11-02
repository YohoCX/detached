// Every action id from docs/KEYMAP.md; components register `run` for the ones they own.
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
  { id: 'app.settings', title: 'Settings…', group: 'view' },
  { id: 'panel.log', title: 'Git Log', group: 'view' },
  { id: 'panel.mergeRequests', title: 'Merge Requests', group: 'view' },
  { id: 'panel.pipelines', title: 'Pipelines', group: 'view' },
  { id: 'vcs.fileHistory', title: 'Show History for File', group: 'vcs' },
  { id: 'mr.create', title: 'Create Merge Request…', group: 'vcs' },
  { id: 'pipeline.run', title: 'Run Pipeline…', group: 'vcs' },
  { id: 'markdown.preview', title: 'Toggle Markdown Preview', group: 'view' },
]

export const actions = new Map(defs.map((a) => [a.id, a]))

/** A later registration wins until it is removed, then the earlier one runs again (an MR's diff viewer over the main one). */
export function register(id: string, run: () => void): () => void {
  const a = actions.get(id)
  if (!a) throw new Error(`unknown action ${id}`)
  const prev = a.run
  a.run = run
  return () => {
    if (a.run === run) a.run = prev
  }
}

export function runAction(id: string): boolean {
  const run = actions.get(id)?.run
  run?.()
  return !!run
}
