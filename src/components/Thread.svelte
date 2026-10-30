<script lang="ts">
  import type { MergeRequest, MrAction, Thread } from '../lib/api'
  import { ago } from '../lib/mr'
  import Markdown from './Markdown.svelte'

  // A thread with reply and resolve, or with `thread` null a box whose text `start` turns into the action.
  // In a diff `zone` it leaves out its position.
  let { mr, thread, act, start, oncancel, zone = false }: {
    mr: MergeRequest
    thread: Thread | null
    act: (a: MrAction) => Promise<boolean>
    start?: (body: string) => MrAction
    oncancel?: () => void
    zone?: boolean
  } = $props()

  let text = $state('')
  let replying = $state(false)
  let busy = $state(false)

  async function send() {
    const body = text.trim()
    if (!body || busy) return
    busy = true
    if (await act(thread ? { kind: 'reply', replyTo: thread.replyTo, body } : start!(body))) {
      text = ''
      replying = false
      oncancel?.()
    }
    busy = false
  }

  function cancel() {
    text = ''
    replying = false
    oncancel?.()
  }
</script>

<div class="thread" class:zone class:resolved={thread?.resolved}>
  {#if thread}
    {@const pos = thread.position}
    {#if pos && !zone}
      <div class="where">
        <span class="mono ellipsis">{pos.path}:{pos.line}</span>
        {#if pos.outdated}<span class="pill">outdated</span>{/if}
      </div>
    {/if}
    {#each thread.notes as n (n.id)}
      <div class="note">
        <div class="head"><span class="author">{n.author}</span><span class="muted">{ago(n.time)}</span></div>
        <Markdown text={n.body} {mr} />
      </div>
    {/each}
  {/if}
  {#if !thread || replying}
    <textarea bind:value={text} rows="3" disabled={busy} placeholder={thread ? 'Reply' : 'Write a comment'}
      {@attach (el) => void ((thread || oncancel) && el.focus({ preventScroll: true }))}
      onkeydown={(e) => (e.key === 'Enter' && e.metaKey ? send() : e.key === 'Escape' && (thread || oncancel) && cancel())}></textarea>
    <div class="buttons">
      {#if thread || oncancel}<button class="btn" onclick={cancel}>Cancel</button>{/if}
      <button class="btn primary" disabled={busy || !text.trim()} onclick={send}>{thread ? 'Reply' : 'Comment'}</button>
    </div>
  {:else if thread.replyable || thread.resolvable}
    <div class="buttons">
      {#if thread.resolved}<span class="muted done">Resolved</span>{/if}
      {#if thread.replyable}<button class="btn" onclick={() => (replying = true)}>Reply</button>{/if}
      {#if thread.resolvable}
        <button class="btn" disabled={busy} onclick={async () => {
          busy = true
          await act({ kind: 'resolve', threadId: thread.id, resolved: !thread.resolved })
          busy = false
        }}>{thread.resolved ? 'Unresolve' : 'Resolve'}</button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .thread { display: grid; gap: 8px; padding: 10px 12px; border: 1px solid var(--line); border-radius: 12px; background: var(--panel); }
  .thread.zone { margin: 4px 12px 6px 4px; font-family: var(--font); font-size: 13px; box-shadow: var(--panel-shadow); }
  .resolved { opacity: 0.75; }
  .where { display: flex; align-items: center; gap: 6px; min-width: 0; color: var(--muted); }
  .pill { flex: none; font-size: 10px; font-weight: 700; padding: 0 6px; border-radius: 999px; border: 1px solid var(--warning); color: var(--warning); }
  .note { display: grid; gap: 2px; }
  .note + .note { padding-top: 8px; border-top: 1px solid var(--line); }
  .head { display: flex; gap: 8px; font-size: 12px; }
  .author { font-weight: 700; }
  .buttons { display: flex; flex-wrap: wrap; justify-content: flex-end; align-items: center; gap: 6px; }
  .done { margin-right: auto; font-size: 12px; }
</style>
