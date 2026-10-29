<script lang="ts">
  import { onMount } from 'svelte'
  import { api, type Account, type Provider, type VerifyResult } from '../lib/api'

  const providers: { id: Provider; name: string; base: string }[] = [
    { id: 'github', name: 'GitHub', base: 'https://github.com' },
    { id: 'githubEnterprise', name: 'GitHub Enterprise', base: '' },
    { id: 'gitlab', name: 'GitLab.com', base: 'https://gitlab.com' },
    { id: 'gitlabSelfHosted', name: 'GitLab self-hosted', base: '' },
  ]
  const providerOf = (id: Provider) => providers.find((p) => p.id === id)!

  type Form = { id: string | null; provider: Provider; baseUrl: string; username: string; label: string; token: string }

  let accounts = $state<Account[]>([])
  let form = $state<Form | null>(null)
  let checks = $state<Record<string, VerifyResult | 'checking'>>({})
  let removing = $state<string | null>(null)
  let error = $state('')

  async function attempt(f: () => Promise<unknown>) {
    error = ''
    try {
      await f()
    } catch (e) {
      error = String(e)
    }
  }

  const reload = async () => (accounts = await api.listAccounts())
  onMount(() => attempt(reload))

  function edit(a?: Account) {
    removing = null
    form = a
      ? { ...a, token: '' }
      : { id: null, provider: 'github', baseUrl: 'https://github.com', username: '', label: '', token: '' }
  }

  async function check(id: string) {
    checks[id] = 'checking'
    checks[id] = await api
      .verifyAccount(id)
      .catch((e): VerifyResult => ({ kind: 'failed', status: null, error: String(e) }))
  }

  const save = (f: Form) =>
    attempt(async () => {
      const token = f.token.trim() || null
      const saved = await api.saveAccount({ ...f, token })
      form = null
      delete checks[saved.id]
      await reload()
      if (token) check(saved.id)
    })

  const remove = (id: string) =>
    attempt(async () => {
      await api.removeAccount(id)
      removing = null
      delete checks[id]
      await reload()
    })
</script>

<section class="accounts">
  <header>
    <h2>Accounts</h2>
    <button onclick={() => edit()} disabled={!!form}>Add account</button>
  </header>

  {#if error}<p class="error">{error}</p>{/if}

  {#each accounts as a (a.id)}
    {@const c = checks[a.id]}
    <div class="row">
      <div class="info">
        <strong>{a.label || a.baseUrl}</strong>
        <span class="muted">{providerOf(a.provider).name} · {a.baseUrl} · {a.username}</span>
        {#if c === 'checking'}
          <span class="muted">Checking…</span>
        {:else if c?.kind === 'ok'}
          <span class="ok">Token ok: {c.user}</span>
        {:else if c?.kind === 'failed'}
          <span class="error">Check failed{c.status ? ` (${c.status})` : ''}: {c.error}</span>
        {/if}
      </div>
      <div class="actions">
        <button onclick={() => check(a.id)} disabled={c === 'checking'}>Check token</button>
        <button onclick={() => edit(a)} disabled={!!form}>Edit</button>
        {#if removing === a.id}
          <button class="danger" onclick={() => remove(a.id)}>Confirm remove</button>
          <button onclick={() => (removing = null)}>Cancel</button>
        {:else}
          <button onclick={() => (removing = a.id)}>Remove</button>
        {/if}
      </div>
    </div>
  {:else}
    {#if !form}<p class="muted">No accounts. Add one to fetch and pull over https with a token.</p>{/if}
  {/each}

  {#if form}
    {@const f = form}
    {@const fixed = !!providerOf(f.provider).base}
    <form
      onsubmit={(e) => {
        e.preventDefault()
        save(f)
      }}
    >
      <label>
        Provider
        <select bind:value={f.provider} onchange={() => (f.baseUrl = providerOf(f.provider).base)}>
          {#each providers as p (p.id)}<option value={p.id}>{p.name}</option>{/each}
        </select>
      </label>
      <label>
        Base URL
        <input bind:value={f.baseUrl} disabled={fixed} placeholder="https://git.example.com/gitlab" required />
      </label>
      <label>
        Username
        <input
          bind:value={f.username}
          placeholder={f.provider.startsWith('github') ? 'Your login' : 'oauth2'}
          autocomplete="off"
          required
        />
      </label>
      <label>
        Label
        <input bind:value={f.label} placeholder="Work GitLab" />
      </label>
      <label>
        Token
        <input
          type="password"
          bind:value={f.token}
          placeholder={f.id ? 'Leave empty to keep the stored token' : 'Personal access token'}
          autocomplete="off"
        />
      </label>
      <div class="actions">
        <button type="submit" class="primary">Save</button>
        <button type="button" onclick={() => (form = null)}>Cancel</button>
      </div>
    </form>
  {/if}
</section>

<style>
  .accounts { display: flex; flex-direction: column; gap: 10px; color: var(--fg); }
  header { display: flex; align-items: center; justify-content: space-between; }
  h2 { font-size: 14px; font-weight: 800; margin: 0; }
  .row, form { padding: 10px 12px; background: var(--hover); border: 1px solid var(--line); border-radius: 12px; }
  .row { display: flex; align-items: center; gap: 12px; }
  .info { display: flex; flex-direction: column; gap: 2px; flex: 1; min-width: 0; }
  .info span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .actions { display: flex; gap: 6px; }
  form { display: grid; grid-template-columns: 1fr; gap: 10px; }
  label { display: flex; flex-direction: column; gap: 4px; color: var(--muted); }
  input:disabled { color: var(--muted); }
  button { height: 28px; padding: 0 14px; border: 1px solid var(--line); border-radius: 999px; background: var(--input); font-weight: 700; }
  button:disabled { opacity: 0.5; }
  .primary { background: var(--primary); border-color: var(--primary); color: var(--primary-fg); }
  .danger { color: var(--error); }
  .muted { color: var(--muted); }
  .ok { color: var(--added); }
  .error { color: var(--error); margin: 0; }
</style>
