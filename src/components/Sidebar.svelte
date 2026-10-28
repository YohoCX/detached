<script lang="ts">
  import { SvelteSet } from 'svelte/reactivity'
  import type { RepoInfo } from '../lib/api'
  import { addFolders, app, excludeRepo, outcomes, refreshAll, removeFolder, statuses } from '../lib/app.svelte'
  import { folderMenu, repoMenu } from '../lib/menu'
  import Icon from './Icon.svelte'
  import VirtualList from './VirtualList.svelte'

  type Row =
    | { folder: string; repo?: undefined }
    | { repo: RepoInfo; solo: boolean; worktree: boolean; worktrees: number; folder?: undefined }

  // Collapsed folders and repos (by path), remembered across launches.
  const KEY = 'sidebar.collapsed'
  const collapsed = new SvelteSet<string>(load())
  function load(): string[] {
    try {
      return JSON.parse(localStorage.getItem(KEY) ?? '[]')
    } catch {
      return []
    }
  }
  function toggle(path: string) {
    if (!collapsed.delete(path)) collapsed.add(path)
    try {
      localStorage.setItem(KEY, JSON.stringify([...collapsed]))
    } catch {}
  }

  const rows = $derived.by(() => {
    const out: Row[] = []
    const listed = new Set(app.repos.map((r) => r.path))
    for (const folder of new Set([...app.folders, ...app.repos.map((r) => r.folder)])) {
      // Worktrees sit under their main repo when it is listed, wherever they were found.
      const repos = app.repos.filter((r) => r.folder === folder && !(r.worktreeOf && listed.has(r.worktreeOf)))
      // A folder that is itself the only repo needs no header; removing the repo removes the folder.
      const solo = repos.length === 1 && repos[0].path === folder
      if (!solo) out.push({ folder })
      if (!solo && collapsed.has(folder)) continue
      for (const repo of repos) {
        const wts = app.repos.filter((wt) => wt.worktreeOf === repo.path)
        out.push({ repo, solo, worktree: false, worktrees: wts.length })
        if (!collapsed.has(repo.path)) for (const wt of wts) out.push({ repo: wt, solo: false, worktree: true, worktrees: 0 })
      }
    }
    return out
  })

  const badge = { merge: 'MERGING', rebase: 'REBASING', cherryPick: 'CHERRY-PICKING', revert: 'REVERTING', bisect: 'BISECTING' }
  const base = (p: string) => p.split('/').filter(Boolean).pop() ?? p
</script>

<section>
  <header>
    <span class="title">Repositories</span>
    <button class="icon-btn" title="Add folder" onclick={addFolders}><Icon name="plus" /></button>
    <button class="icon-btn" title="Refresh" onclick={refreshAll}><Icon name="refresh" /></button>
  </header>
  {#if rows.length}
    <VirtualList items={rows} height={40}>
      {#snippet row(r)}
        {#if r.folder !== undefined}
          <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
          <div class="folder" title={r.folder} onclick={() => toggle(r.folder)}
            oncontextmenu={() => folderMenu(r.folder, collapsed.has(r.folder), () => toggle(r.folder))}>
            <span class="chevron" class:open={!collapsed.has(r.folder)}><Icon name="chevron" size={12} /></span>
            <Icon name="folder" /><span class="label ellipsis">{base(r.folder)}</span>
            <button class="icon-btn hover" title="Remove folder" onclick={(e) => {
              e.stopPropagation()
              removeFolder(r.folder)
            }}><Icon name="minus" /></button>
          </div>
        {:else}
          {@const s = statuses.get(r.repo.path)}
          {@const st = typeof s === 'object' ? s : null}
          {@const o = outcomes.get(r.repo.path)}
          <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
          <div
            class="repo"
            class:worktree={r.worktree}
            class:selected={app.current === r.repo.path}
            title={st?.lastCommit ? `${r.repo.path}\n${st.lastCommit.subject}` : r.repo.path}
            onclick={() => (app.current = r.repo.path)}
            oncontextmenu={() => {
              app.current = r.repo.path
              repoMenu(r.repo.path, r.solo)
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
              <span class="name ellipsis">{r.worktree ? base(r.repo.path) : r.repo.name}</span>
              {#if st?.inProgress}<span class="badge">{badge[st.inProgress]}</span>{/if}
              {#if st?.changes.length}<span class="dirty" title="Changed files">{st.changes.length}</span>{/if}
              <button class="icon-btn hover" title="Remove from list" onclick={(e) => {
                e.stopPropagation()
                if (r.solo) removeFolder(r.repo.path)
                else excludeRepo(r.repo.path)
              }}>
                <Icon name="close" size={12} />
              </button>
            </div>
            <div class="line muted">
              {#if typeof s === 'string'}
                <span class="err ellipsis">{s}</span>
              {:else if st}
                <Icon name="branch" size={12} />
                <span class="ellipsis">{st.branch ?? `${st.head?.slice(0, 8) ?? '?'} (detached)`}</span>
                {#if st.ahead}<span class="ab">↑{st.ahead}</span>{/if}
                {#if st.behind}<span class="ab">↓{st.behind}</span>{/if}
                {#if o}<span class="ellipsis outcome" class:err={!o.ok} title={o.text}>{o.text}</span>{/if}
              {/if}
            </div>
          </div>
        {/if}
      {/snippet}
    </VirtualList>
  {:else}
    <div class="empty muted">
      <p>No repositories yet.</p>
      <button class="btn primary" onclick={addFolders}>Add folder…</button>
    </div>
  {/if}
</section>

<style>
  section { display: flex; flex-direction: column; height: 100%; min-width: 0; }
  header { display: flex; align-items: center; gap: 2px; height: 34px; padding: 0 6px 0 12px; flex: none; }
  .title { flex: 1; font-weight: 600; }
  .folder { display: flex; align-items: flex-end; gap: 6px; height: 100%; padding: 0 6px 4px 4px; color: var(--muted); font-size: 12px; }
  .chevron { display: inline-flex; flex: none; width: 16px; height: 16px; align-items: center; justify-content: center; color: var(--muted); transition: transform 0.1s; }
  .chevron.open { transform: rotate(90deg); }
  .toggle { position: absolute; left: 10px; border: 0; background: none; padding: 2px; border-radius: 3px; }
  .toggle:hover { background: var(--hover); }
  .folder .label { flex: 1; }
  .repo { position: relative; height: 100%; padding: 3px 6px 3px 28px; border-radius: 4px; margin: 0 4px; }
  .repo:hover { background: var(--hover); }
  .repo.selected { background: var(--selected); }
  .repo.worktree { padding-left: 44px; }
  .line { display: flex; align-items: center; gap: 5px; height: 17px; font-size: 12px; }
  .line:first-child { font-size: 13px; }
  .name { flex: 1; }
  .badge { font-size: 10px; padding: 0 4px; border-radius: 3px; background: var(--warning); color: var(--accent-fg); }
  .dirty { color: var(--modified); font-size: 11px; }
  .ab { color: var(--fg); }
  .outcome { margin-left: auto; }
  .err { color: var(--error); }
  .hover { visibility: hidden; height: 18px; min-width: 18px; padding: 0 2px; }
  .repo:hover .hover, .folder:hover .hover { visibility: visible; }
  .empty { padding: 24px 12px; text-align: center; }
</style>
