<script lang="ts">
  import { onMount } from 'svelte'
  import BottomPanel from './components/BottomPanel.svelte'
  import Changes from './components/Changes.svelte'
  import FileTree from './components/FileTree.svelte'
  import FindInFiles from './components/FindInFiles.svelte'
  import Grip from './components/Grip.svelte'
  import Icon from './components/Icon.svelte'
  import JobLog from './components/JobLog.svelte'
  import MrList from './components/MrList.svelte'
  import MrPage from './components/MrPage.svelte'
  import Notices from './components/Notices.svelte'
  import Palette from './components/Palette.svelte'
  import Settings from './components/Settings.svelte'
  import Sidebar from './components/Sidebar.svelte'
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

  let reposWidth = $state(280)
  let toolWidth = $state(300)
  const tool = (t: 'changes' | 'files' | 'mrs') => (app.tool = app.tool === t ? null : t)
  const shown = $derived(view.shown)
</script>

<svelte:window onfocus={refreshAll} />

<div class="app">
  <TopBar />
  <div class="body">
    <nav class="strip panel">
      <button class="icon-btn" class:on={app.sidebar} title="Repositories" onclick={() => (app.sidebar = !app.sidebar)}><Icon name="repo" /></button>
      <button class="icon-btn" class:on={app.tool === 'files'} title="Project {chordFor('panel.project')}" onclick={() => tool('files')}><Icon name="tree" /></button>
      <button class="icon-btn" class:on={app.tool === 'changes'} title="Changes" onclick={() => tool('changes')}><Icon name="changes" /></button>
      <button class="icon-btn" class:on={app.tool === 'mrs'} title="Merge Requests" onclick={() => tool('mrs')}><Icon name="mr" /></button>
      <button class="icon-btn settings" title="Settings {chordFor('app.settings')}" onclick={() => (app.settings = true)}><Icon name="gear" /></button>
    </nav>
    {#if app.sidebar}
      <aside class="repos panel" style:width="{reposWidth}px"><Sidebar /><Grip key="repos" bind:size={reposWidth} edge="right" min={200} max={600} /></aside>
    {/if}
    <div class="work">
      <div class="top">
        {#if app.tool}
          <aside class="tool panel" style:width="{toolWidth}px">
            <Grip key="tool" bind:size={toolWidth} edge="right" min={220} max={800} />
            {#if app.tool === 'files'}<FileTree repo={app.current} />
            {:else if app.tool === 'mrs'}<MrList repo={app.current} />
            {:else}<Changes repo={app.current} />{/if}
          </aside>
        {/if}
        <main>
          <!-- Viewer stays mounted under an MR page or a job log, keeping its editors. -->
          <div class="doc" class:hidden={shown?.kind === 'mr' || shown?.kind === 'job'}><Viewer /></div>
          {#if shown?.kind === 'mr'}<MrPage repo={shown.repo} number={shown.number} />
          {:else if shown?.kind === 'job'}<JobLog repo={shown.repo} project={shown.project} id={shown.id} name={shown.name} />{/if}
        </main>
      </div>
      {#if app.bottom}<BottomPanel />{/if}
    </div>
  </div>
</div>
<Notices />
{#if app.palette}<Palette />{/if}
{#if app.settings}<Settings />{/if}
{#if view.find}<FindInFiles />{/if}

<style>
  .app { display: flex; flex-direction: column; gap: 10px; height: 100%; padding: 10px; }
  .body { display: flex; gap: 10px; flex: 1; min-height: 0; }
  .strip { display: flex; flex-direction: column; align-items: center; gap: 6px; width: 48px; padding: 8px 0; flex: none; }
  .strip .icon-btn { width: 34px; height: 34px; justify-content: center; border-radius: 10px; }
  .strip .settings { margin-top: auto; }
  .strip .on { background: var(--selected); color: var(--fg); box-shadow: 0 0 18px var(--selected); }
  .work, .top { display: flex; gap: 10px; flex: 1; min-width: 0; min-height: 0; }
  .work { flex-direction: column; }
  aside { position: relative; flex: none; min-width: 0; overflow: hidden; }
  .doc { height: 100%; }
  .hidden { display: none; }
  main { flex: 1; min-width: 0; overflow: hidden; border-radius: 14px; background: var(--code); border: 1px solid var(--line); box-shadow: var(--panel-shadow); }
</style>
