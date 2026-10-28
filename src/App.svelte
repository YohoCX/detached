<script lang="ts">
  import { onMount } from 'svelte'
  import Changes from './components/Changes.svelte'
  import FileTree from './components/FileTree.svelte'
  import FindInFiles from './components/FindInFiles.svelte'
  import Icon from './components/Icon.svelte'
  import Notices from './components/Notices.svelte'
  import Palette from './components/Palette.svelte'
  import Settings from './components/Settings.svelte'
  import Sidebar from './components/Sidebar.svelte'
  import StatusBar from './components/StatusBar.svelte'
  import TopBar from './components/TopBar.svelte'
  import Viewer from './components/Viewer.svelte'
  import { api } from './lib/api'
  import { app, loadRepos, refreshAll, registerActions, run } from './lib/app.svelte'
  import { chordFor, install, setOverrides } from './lib/keymap'
  import { view } from './lib/view.svelte'

  onMount(() => {
    loadRepos()
    api.keymapOverrides().then(setOverrides, () => {})
    const off = registerActions()
    const uninstall = install(run)
    return () => {
      off()
      uninstall()
    }
  })

  const tool = (t: 'changes' | 'files') => (app.tool = app.tool === t ? null : t)
</script>

<svelte:window onfocus={refreshAll} />

<div class="app">
  <TopBar />
  <div class="body">
    <nav class="strip">
      <button class="icon-btn" class:on={app.sidebar} title="Repositories" onclick={() => (app.sidebar = !app.sidebar)}><Icon name="repo" /></button>
      <button class="icon-btn" class:on={app.tool === 'files'} title="Project {chordFor('panel.project')}" onclick={() => tool('files')}><Icon name="tree" /></button>
      <button class="icon-btn" class:on={app.tool === 'changes'} title="Changes" onclick={() => tool('changes')}><Icon name="changes" /></button>
      <button class="icon-btn settings" title="Settings {chordFor('app.settings')}" onclick={() => (app.settings = true)}><Icon name="gear" /></button>
    </nav>
    {#if app.sidebar}<aside class="repos"><Sidebar /></aside>{/if}
    {#if app.tool}
      <aside class="tool">
        {#if app.tool === 'files'}<FileTree repo={app.current} />{:else}<Changes repo={app.current} />{/if}
      </aside>
    {/if}
    <main><Viewer /></main>
  </div>
  <StatusBar />
</div>
<Notices />
{#if app.palette}<Palette />{/if}
{#if app.settings}<Settings />{/if}
{#if view.find}<FindInFiles />{/if}

<style>
  .app { display: flex; flex-direction: column; height: 100%; }
  .body { display: flex; flex: 1; min-height: 0; }
  .strip { display: flex; flex-direction: column; gap: 4px; width: 40px; padding: 6px; flex: none; background: var(--surface); border-right: 1px solid var(--line); }
  .strip .icon-btn { width: 28px; height: 28px; justify-content: center; color: var(--muted); }
  .strip .settings { margin-top: auto; }
  .strip .on { background: var(--selected); color: var(--fg); }
  aside { flex: none; min-width: 0; border-right: 1px solid var(--line); background: var(--bg); }
  .repos { width: 280px; }
  .tool { width: 300px; }
  main { flex: 1; min-width: 0; background: var(--surface); }
</style>
