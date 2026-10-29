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
    <nav class="strip panel">
      <button class="icon-btn" class:on={app.sidebar} title="Repositories" onclick={() => (app.sidebar = !app.sidebar)}><Icon name="repo" /></button>
      <button class="icon-btn" class:on={app.tool === 'files'} title="Project {chordFor('panel.project')}" onclick={() => tool('files')}><Icon name="tree" /></button>
      <button class="icon-btn" class:on={app.tool === 'changes'} title="Changes" onclick={() => tool('changes')}><Icon name="changes" /></button>
      <button class="icon-btn settings" title="Settings {chordFor('app.settings')}" onclick={() => (app.settings = true)}><Icon name="gear" /></button>
    </nav>
    {#if app.sidebar}<aside class="repos panel"><Sidebar /></aside>{/if}
    {#if app.tool}
      <aside class="tool panel">
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
  .app { display: flex; flex-direction: column; gap: 10px; height: 100%; padding: 10px 10px 6px; }
  .body { display: flex; gap: 10px; flex: 1; min-height: 0; }
  .strip { display: flex; flex-direction: column; align-items: center; gap: 6px; width: 48px; padding: 8px 0; flex: none; }
  .strip .icon-btn { width: 34px; height: 34px; justify-content: center; border-radius: 10px; }
  .strip .settings { margin-top: auto; }
  .strip .on { background: var(--selected); color: var(--fg); box-shadow: 0 0 18px var(--selected); }
  aside { flex: none; min-width: 0; overflow: hidden; }
  .repos { width: 280px; }
  .tool { width: 300px; }
  main { flex: 1; min-width: 0; overflow: hidden; border-radius: 14px; background: var(--code); border: 1px solid var(--line); box-shadow: var(--panel-shadow); }
</style>
