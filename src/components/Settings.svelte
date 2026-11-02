<script lang="ts">
  import { onMount } from 'svelte'
  import { api, type Account, type EnvInfo, type ForgeHosts, type HostAccount, type Settings } from '../lib/api'
  import { app, loadRepos, nameOf, say } from '../lib/app.svelte'
  import Accounts from './Accounts.svelte'
  import Icon from './Icon.svelte'

  let accounts = $state<Account[]>([])
  let hosts = $state<HostAccount[]>([])
  let forge = $state<ForgeHosts | null>(null)
  let saved = $state<Settings | null>(null)
  let excluded = $state<string[]>([])
  let env = $state<EnvInfo | null>(null)
  let gitPath = $state('')

  const repo = $derived(app.current)
  const close = () => (app.settings = false)

  async function attempt(f: () => Promise<unknown>) {
    try {
      await f()
    } catch (e) {
      say(String(e), true)
    }
  }

  async function reload() {
    const s = (saved = await api.getSettings())
    excluded = s.excludedRepos
    gitPath = s.gitPath ?? ''
    accounts = await api.listAccounts()
    hosts = repo ? await api.repoAccounts(repo) : []
    forge = repo ? await api.forgeHosts(repo) : null
  }

  onMount(() => {
    attempt(reload)
    attempt(async () => (env = await api.envInfo()))
  })

  const setAccount = (id: string) =>
    attempt(async () => {
      await api.setRepoAccount(repo!, id || null)
      await reload()
    })

  const setForgeAccount = (host: string, id: string) =>
    attempt(async () => {
      await api.setForgeAccount(repo!, host, id || null)
      await reload()
    })

  const setMrRemote = (remote: string) =>
    attempt(async () => {
      await api.setMrRemote(repo!, remote || null)
      await reload()
    })

  const restore = (path: string) =>
    attempt(async () => {
      await loadRepos(await api.includeRepo(path))
      await reload()
    })

  const saveGitPath = () => attempt(async () => (env = await api.setGitPath(gitPath.trim() || null)))
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && close()} />

<div class="backdrop scrim" onclick={close} role="presentation"></div>
<div class="dialog glass" role="dialog" aria-label="Settings">
  <header>
    <h2>Settings</h2>
    <button class="icon-btn" title="Close" onclick={close}><Icon name="close" /></button>
  </header>
  <div class="body">
    <section onfocusin={() => attempt(reload)}>
      <h3>Accounts</h3>
      <Accounts />
    </section>

    {#if repo}
      <section>
        <h3>Account for {nameOf(repo)}</h3>
        {#each hosts as h (h.host)}
          <label class="row">
            <span class="mono">{h.host}</span>
            <span class="muted">{h.remotes.join(', ')}</span>
            <select value={h.accountId ?? ''} onchange={(e) => setAccount(e.currentTarget.value)}>
              <option value="">{h.ambiguous ? 'Choose an account' : 'Automatic'}</option>
              {#each accounts.filter((a) => a.baseUrl.includes(h.host)) as a (a.id)}
                <option value={a.id}>{a.label || a.username}</option>
              {/each}
            </select>
          </label>
        {:else}
          <p class="muted">No https remotes; git uses your own credentials (ssh agent, keychain).</p>
        {/each}
      </section>
    {/if}

    {#if repo && forge}
      {@const f = forge}
      {@const chosen = saved?.forgeAccounts[repo] ?? {}}
      <section>
        <h3>Merge requests for {nameOf(repo)}</h3>
        {#if f.hosts.length}
          <label class="row">
            <span>Remote</span>
            <select value={saved?.mrRemotes[repo] ?? ''} onchange={(e) => setMrRemote(e.currentTarget.value)}>
              <option value="">{f.mrRemote ? `Automatic (${f.mrRemote})` : 'Choose a remote'}</option>
              {#each f.hosts.flatMap((h) => h.remotes) as r (r)}<option value={r}>{r}</option>{/each}
            </select>
          </label>
          {#each f.hosts as h (h.host)}
            {@const auto = accounts.find((a) => a.id === h.accountId)}
            <label class="row">
              <span class="mono">{h.host}</span>
              <span class="muted">{h.remotes.join(', ')}</span>
              <select value={chosen[h.host] ?? ''} onchange={(e) => setForgeAccount(h.host, e.currentTarget.value)}>
                <option value="">{h.ambiguous ? 'Choose an account' : auto ? `Automatic (${auto.label || auto.username})` : 'No account fits'}</option>
                {#each accounts as a (a.id)}<option value={a.id}>{a.label || a.username}</option>{/each}
              </select>
            </label>
          {/each}
        {:else}
          <p class="muted">No remote is on GitLab or GitHub.</p>
        {/if}
      </section>
    {/if}

    <section>
      <h3>Removed repositories</h3>
      {#each excluded as path (path)}
        <div class="row">
          <span class="mono ellipsis">{path}</span>
          <button class="btn" onclick={() => restore(path)}>Restore</button>
        </div>
      {:else}
        <p class="muted">None.</p>
      {/each}
    </section>

    <section>
      <h3>Git</h3>
      {#if env}<p class="muted mono">{env.gitVersion} · {env.gitPath}</p>{/if}
      <div class="row">
        <input placeholder="git binary (empty: from your shell PATH)" bind:value={gitPath} />
        <button class="btn" onclick={saveGitPath}>Apply</button>
      </div>
    </section>
  </div>
</div>

<style>
  .backdrop { z-index: 30; }
  .dialog {
    position: fixed; z-index: 31; top: 8vh; left: 50%; transform: translateX(-50%);
    width: min(720px, 92vw); max-height: 84vh; display: flex; flex-direction: column; border-radius: 20px; overflow: hidden;
    -webkit-user-select: text;
    user-select: text;
  }
  header { display: flex; align-items: center; justify-content: space-between; padding: 12px 14px 12px 20px; border-bottom: 1px solid var(--line); }
  h2 { font-size: 15px; font-weight: 800; margin: 0; }
  h3 { font-size: 11px; font-weight: 700; margin: 0 0 10px; color: var(--muted); text-transform: uppercase; letter-spacing: 0.06em; }
  .body { overflow: auto; padding: 18px 20px; display: grid; gap: 22px; }
  .row { display: flex; align-items: center; gap: 10px; margin-bottom: 6px; }
  .row input { flex: 1; }
  .row .mono { min-width: 0; }
  p { margin: 0; }
</style>
