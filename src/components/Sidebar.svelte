<script lang="ts">
  import type { RepoInfo } from '../lib/api'
  import { addFolders, app, excludeRepo, outcomes, refreshAll, removeFolder, statuses } from '../lib/app.svelte'
  import Icon from './Icon.svelte'
  import VirtualList from './VirtualList.svelte'

  type Row = { folder: string; repo?: undefined } | { repo: RepoInfo; solo: boolean; worktree: boolean; folder?: undefined }

  const rows = $derived.by(() => {
    const out: Row[] = []
    const listed = new Set(app.repos.map((r) => r.path))
    for (const folder of new Set([...app.folders, ...app.repos.map((r) => r.folder)])) {
      // Worktrees sit under their main repo when it is listed, wherever they were found.
      const repos = app.repos.filter((r) => r.folder === folder && !(r.worktreeOf && listed.has(r.worktreeOf)))
      // A folder that is itself the only repo needs no header; removing the repo removes the folder.
      const solo = repos.length === 1 && repos[0].path === folder
      if (!solo) out.push({ folder })
      for (const repo of repos) {
        out.push({ repo, solo, worktree: false })
        for (const wt of app.repos) if (wt.worktreeOf === repo.path) out.push({ repo: wt, solo: false, worktree: true })
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
          <div class="folder" title={r.folder}>
            <Icon name="folder" /><span class="ellipsis">{base(r.folder)}</span>
            <button class="icon-btn hover" title="Remove folder" onclick={() => removeFolder(r.folder)}><Icon name="minus" /></button>
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
          >
            <div class="line">
              <span class="name ellipsis">{r.repo.name}</span>
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
  .folder { display: flex; align-items: flex-end; gap: 6px; height: 100%; padding: 0 6px 4px 12px; color: var(--muted); font-size: 12px; }
  .folder span { flex: 1; }
  .repo { height: 100%; padding: 3px 6px 3px 28px; border-radius: 4px; margin: 0 4px; }
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
