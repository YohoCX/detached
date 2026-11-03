<script lang="ts">
  import { SvelteSet } from 'svelte/reactivity'
  import type { RepoInfo } from '../lib/api'
  import { activeWorkspaces, addFolders, app, excludeRepo, outcomes, refreshAll, removeFolder, statuses, toneOf } from '../lib/app.svelte'
  import { folderName, inFolder } from '../lib/files'
  import { folderMenu, repoMenu, workspaceMenu } from '../lib/menu'
  import { load, save } from '../lib/storage'
  import Icon from './Icon.svelte'
  import VirtualList from './VirtualList.svelte'

  type Row = { workspace: string } & (
    | { head: true; folder?: undefined; repo?: undefined }
    | { folder: string; head?: undefined; repo?: undefined }
    | { repo: RepoInfo; solo: boolean; worktree: boolean; worktrees: number; head?: undefined; folder?: undefined }
  )

  // Collapsed workspaces (`workspace:` and the name), folders and repos, by path.
  const collapsed = new SvelteSet<string>(JSON.parse(load('sidebar.collapsed') ?? '[]'))
  function toggle(path: string) {
    if (!collapsed.delete(path)) collapsed.add(path)
    save('sidebar.collapsed', JSON.stringify([...collapsed]))
  }

  const rows = $derived.by(() => {
    const out: Row[] = []
    const listed = new Set(app.repos.map((r) => r.path))
    const active = activeWorkspaces()
    for (const { name: workspace, folders } of active) {
      // Headers only tell several workspaces apart.
      if (active.length > 1) {
        out.push({ workspace, head: true })
        if (collapsed.has(`workspace:${workspace}`)) continue
      }
      for (const folder of new Set(folders.map((f) => f.replace(/\/+$/, '')))) {
        // Worktrees sit under their main repo when it is listed, wherever they were found.
        const repos = app.repos.filter((r) => inFolder(r.path, folder) && !(r.worktreeOf && listed.has(r.worktreeOf)))
        // A folder that is itself the only repo needs no header; removing the repo removes the folder.
        const solo = repos.length === 1 && repos[0].path === folder
        if (!solo) out.push({ workspace, folder })
        if (!solo && collapsed.has(folder)) continue
        for (const repo of repos) {
          const wts = app.repos.filter((wt) => wt.worktreeOf === repo.path)
          out.push({ workspace, repo, solo, worktree: false, worktrees: wts.length })
          if (!collapsed.has(repo.path))
            for (const wt of wts) out.push({ workspace, repo: wt, solo: false, worktree: true, worktrees: 0 })
        }
      }
    }
    return out
  })

  const badge = { merge: 'MERGING', rebase: 'REBASING', cherryPick: 'CHERRY-PICKING', revert: 'REVERTING', bisect: 'BISECTING' }
</script>

<section>
  <header>
    <span class="title">Repositories</span>
    {#if !app.workspaces.length || activeWorkspaces().length}
      <button class="icon-btn" title="Add folder" onclick={() => addFolders()}><Icon name="plus" /></button>
    {/if}
    <button class="icon-btn" title="Refresh" onclick={refreshAll}><Icon name="refresh" /></button>
  </header>
  {#if rows.length}
    <VirtualList items={rows} height={40}>
      {#snippet row(r)}
        {#if r.head}
          {@const key = `workspace:${r.workspace}`}
          <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
          <div class="folder workspace" onclick={() => toggle(key)} oncontextmenu={() => workspaceMenu(r.workspace)}>
            <span class="chevron" class:open={!collapsed.has(key)}><Icon name="chevron" size={12} /></span>
            <span class="label ellipsis">{r.workspace}</span>
            <button class="icon-btn hover" title="Add folder to {r.workspace}" onclick={(e) => {
              e.stopPropagation()
              addFolders(r.workspace)
            }}><Icon name="plus" /></button>
          </div>
        {:else if r.folder !== undefined}
          <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
          <div class="folder" title={r.folder} onclick={() => toggle(r.folder)}
            oncontextmenu={() => folderMenu(r.workspace, r.folder, collapsed.has(r.folder), () => toggle(r.folder))}>
            <span class="chevron" class:open={!collapsed.has(r.folder)}><Icon name="chevron" size={12} /></span>
            <Icon name="folder" /><span class="label ellipsis">{folderName(r.folder)}</span>
            <button class="icon-btn hover" title="Remove folder" onclick={(e) => {
              e.stopPropagation()
              removeFolder(r.workspace, r.folder)
            }}><Icon name="minus" /></button>
          </div>
        {:else}
          {@const s = statuses.get(r.repo.path)}
          {@const st = typeof s === 'object' ? s : null}
          {@const o = outcomes.get(r.repo.path)}
          {@const tone = toneOf(s)}
          <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
          <div
            class="repo"
            class:worktree={r.worktree}
            class:selected={app.current === r.repo.path}
            title={st?.lastCommit ? `${r.repo.path}\n${st.lastCommit.subject}` : r.repo.path}
            onclick={() => (app.current = r.repo.path)}
            oncontextmenu={() => {
              app.current = r.repo.path
              repoMenu(r.workspace, r.repo.path, r.solo)
            }}
          >
            <div class="line">
              {#if r.worktrees}
                <button class="chevron toggle" class:open={!collapsed.has(r.repo.path)}
                  title="{collapsed.has(r.repo.path) ? 'Show' : 'Hide'} {r.worktrees} worktree{r.worktrees > 1 ? 's' : ''}"
                  onclick={(e) => {
                    e.stopPropagation()
                    toggle(r.repo.path)
                  }}><Icon name="chevron" size={12} /></button>
              {/if}
              <span class="name ellipsis">{r.worktree ? folderName(r.repo.path) : r.repo.name}</span>
              {#if st?.inProgress}<span class="badge">{badge[st.inProgress]}</span>{/if}
              {#if st?.changes.length}<span class="dirty" title="Changed files">{st.changes.length}</span>{/if}
              <button class="icon-btn hover" title="Remove from list" onclick={(e) => {
                e.stopPropagation()
                if (r.solo) removeFolder(r.workspace, r.repo.path)
                else excludeRepo(r.workspace, r.repo.path)
              }}>
                <Icon name="close" size={12} />
              </button>
              {#if tone}<span class="dot" style:color="var(--{tone})"></span>{/if}
            </div>
            <div class="line muted">
              {#if typeof s === 'string'}
                <span class="err ellipsis">{s}</span>
              {:else if st}
                <Icon name="branch" size={12} />
                <span class="ellipsis branch">{st.branch ?? `${st.head?.slice(0, 8) ?? '?'} (detached)`}</span>
                {#if st.ahead}<span class="ab">↑{st.ahead}</span>{/if}
                {#if st.behind}<span class="ab">↓{st.behind}</span>{/if}
                {#if o}<span class="ellipsis outcome" class:err={!o.ok} title={o.text}>{o.text}</span>{/if}
              {/if}
            </div>
          </div>
        {/if}
      {/snippet}
    </VirtualList>
  {:else if app.workspaces.length && !activeWorkspaces().length}
    <div class="empty muted"><p>No workspace is on. Turn one on in the top bar.</p></div>
  {:else}
    <div class="empty muted">
      <p>No repositories yet.</p>
      <button class="btn primary" onclick={() => addFolders()}>Add folder…</button>
    </div>
  {/if}
</section>

<style>
  section { display: flex; flex-direction: column; height: 100%; min-width: 0; padding-bottom: 6px; }
  header { display: flex; align-items: center; gap: 2px; height: 44px; padding: 0 8px 0 16px; flex: none; }
  .title { flex: 1; font-size: 14px; font-weight: 800; }
  .folder { display: flex; align-items: flex-end; gap: 6px; height: 100%; padding: 0 8px 6px 8px; color: var(--muted); font-size: 11px; font-weight: 700; letter-spacing: 0.06em; text-transform: uppercase; }
  .chevron { display: inline-flex; flex: none; width: 16px; height: 16px; align-items: center; justify-content: center; color: var(--muted); transition: transform 0.1s; }
  .chevron.open { transform: rotate(90deg); }
  .toggle { position: absolute; left: 8px; border: 0; background: none; padding: 2px; border-radius: 4px; }
  .toggle:hover { background: var(--hover); }
  .folder .label { flex: 1; }
  .workspace { color: var(--fg); font-size: 12px; }
  .repo { position: relative; height: 100%; padding: 3px 10px 3px 28px; border-radius: 10px; margin: 0 6px; }
  .repo:hover { background: var(--hover); }
  .repo.selected { background: var(--selected); box-shadow: inset 0 0 0 1px var(--selected-ring); }
  .repo.worktree { padding-left: 44px; }
  .line { display: flex; align-items: center; gap: 5px; height: 17px; font-size: 12px; }
  .line:first-child { font-size: 13px; font-weight: 600; }
  .name { flex: 1; }
  .badge { font-size: 10px; font-weight: 700; padding: 0 5px; border-radius: 4px; background: var(--warning); color: var(--accent-fg); }
  .dirty { color: var(--warning); font-size: 11px; font-weight: 700; }
  .ab { color: var(--fg); }
  .branch { flex: none; max-width: 75%; }
  .outcome { margin-left: auto; min-width: 0; }
  .err { color: var(--error); }
  .hover { visibility: hidden; height: 18px; min-width: 18px; padding: 0 2px; }
  .repo:hover .hover, .folder:hover .hover { visibility: visible; }
  .empty { padding: 24px 12px; text-align: center; }
</style>
