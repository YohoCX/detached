import { Menu, PredefinedMenuItem } from '@tauri-apps/api/menu'
import { api } from './api'
import { addFolders, app, excludeRepo, removeFolder, say, settingsCall, syncOne, syncRepos } from './app.svelte'
import { inFolder } from './files'
import { fileManager } from './platform'
import { compare } from './view.svelte'

type Item = { text: string; action: () => unknown; enabled?: boolean } | 'separator'

export async function popup(items: Item[]) {
  const built = await Promise.all(
    items.map((i) =>
      i === 'separator'
        ? PredefinedMenuItem.new({ item: 'Separator' })
        : { text: i.text, enabled: i.enabled ?? true, action: () => void i.action() },
    ),
  )
  await (await Menu.new({ items: built })).popup()
}

const copy = (path: string) => navigator.clipboard.writeText(path).then(() => say(`Copied ${path}`))

export function repoMenu(workspace: string, repo: string, solo: boolean) {
  const branches = (view: 'checkout' | 'new') => {
    app.current = repo
    app.branches = view
  }
  return popup([
    { text: 'Fetch', action: () => syncOne('fetch', repo) },
    { text: 'Pull', action: () => syncOne('pull', repo) },
    'separator',
    { text: 'Compare with Base Branch', action: () => {
      app.current = repo
      app.tool = 'changes'
      compare.set(repo, compare.get(repo) ?? '')
    } },
    { text: 'Branches…', action: () => branches('checkout') },
    { text: 'New Branch…', action: () => branches('new') },
    'separator',
    { text: `Open in ${fileManager}`, action: () => api.openFolder(repo) },
    { text: 'Open in Zed', action: () => api.openInZed(repo, '', null) },
    { text: 'Copy Path', action: () => copy(repo) },
    'separator',
    { text: 'Remove from List', action: () => (solo ? removeFolder(workspace, repo) : excludeRepo(workspace, repo)) },
  ])
}

export function folderMenu(workspace: string, folder: string, collapsed: boolean, toggle: () => void) {
  const repos = app.repos.filter((r) => inFolder(r.path, folder)).map((r) => r.path)
  return popup([
    { text: 'Fetch All in Folder', action: () => syncRepos('fetch', repos), enabled: !app.sync },
    { text: 'Pull All in Folder', action: () => syncRepos('pull', repos), enabled: !app.sync },
    'separator',
    { text: collapsed ? 'Expand' : 'Collapse', action: toggle },
    { text: `Open in ${fileManager}`, action: () => api.openFolder(folder) },
    { text: 'Copy Path', action: () => copy(folder) },
    'separator',
    { text: 'Remove Folder', action: () => removeFolder(workspace, folder) },
  ])
}

export const workspaceMenu = (workspace: string) =>
  popup([
    { text: 'Add Folder…', action: () => addFolders(workspace) },
    { text: 'Turn Off', action: () => settingsCall(() => api.setWorkspaceActive(workspace, false)) },
  ])
