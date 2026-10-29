<script lang="ts">
  import { api } from '../lib/api'
  import { app, dismiss, say } from '../lib/app.svelte'
  import Icon from './Icon.svelte'

  const zed = (repo: string, path: string) => api.openInZed(repo, path, null).catch((e) => say(String(e), true))
</script>

<div class="stack">
  {#each app.notices as n (n.id)}
    <div class="notice glass {n.level}">
      <div class="title">
        <span>{n.title}</span>
        <button class="icon-btn" title="Close" onclick={() => dismiss(n.id)}><Icon name="close" size={12} /></button>
      </div>
      {#if n.body}<pre>{n.body}</pre>{/if}
      {#if n.repo && n.files?.length}
        {@const repo = n.repo}
        <ul>
          {#each n.files as f (f)}
            <li><span class="mono ellipsis">{f}</span><button class="btn" onclick={() => zed(repo, f)}>Open in Zed</button></li>
          {/each}
        </ul>
      {/if}
    </div>
  {/each}
</div>

<style>
  .stack { position: fixed; right: 16px; bottom: 34px; z-index: 30; display: flex; flex-direction: column; gap: 8px; width: 380px; }
  .notice { --c: var(--success); padding: 10px 12px 10px 14px; }
  .notice.warn { --c: var(--warning); }
  .notice.error { --c: var(--error); }
  .title { display: flex; align-items: center; gap: 8px; font-weight: 700; }
  .title::before { content: ''; flex: none; width: 8px; height: 8px; border-radius: 50%; background: var(--c); box-shadow: 0 0 10px var(--c); }
  .title span { flex: 1; }
  .title .icon-btn { height: 18px; min-width: 18px; }
  pre { margin: 4px 0 0; font: 12px/1.4 var(--font); white-space: pre-wrap; word-break: break-word; max-height: 200px; overflow: auto; -webkit-user-select: text; user-select: text; }
  ul { list-style: none; margin: 6px 0 0; padding: 0; max-height: 200px; overflow-y: auto; }
  li { display: flex; align-items: center; gap: 8px; padding: 2px 0; }
  li span { flex: 1; min-width: 0; }
  li .btn { height: 24px; padding: 0 10px; font-size: 12px; }
</style>
