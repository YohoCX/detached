<script lang="ts">
  import { app, cancelOp, nameOf } from '../lib/app.svelte'
  import Icon from './Icon.svelte'
</script>

<footer>
  {#if app.message}
    <button class="message ellipsis" class:err={app.message.error} title={app.message.text} onclick={() => (app.message = null)}>
      {app.message.text}
    </button>
  {/if}
  <span class="fill"></span>
  {#each app.ops as op (op.id)}
    <span class="op" title={op.line}>
      <span class="spinner"></span>
      <span class="strong">{op.label}</span> {nameOf(op.repo)}
      {#if op.line}<span class="muted ellipsis line">{op.line}</span>{/if}
      <button class="icon-btn cancel" title="Cancel" onclick={() => cancelOp(op.id)}><Icon name="close" size={10} /></button>
    </span>
  {/each}
</footer>

<style>
  footer { display: flex; align-items: center; gap: 12px; height: 24px; padding: 0 10px; font-size: 12px; background: var(--surface); border-top: 1px solid var(--line); min-width: 0; }
  .message { border: 0; background: none; padding: 0; text-align: left; min-width: 0; max-width: 50%; }
  .err { color: var(--error); }
  .fill { flex: 1; }
  .op { display: flex; align-items: center; gap: 4px; min-width: 0; max-width: 320px; }
  .strong { font-weight: 600; }
  .line { min-width: 0; }
  .cancel { height: 16px; min-width: 16px; padding: 0 3px; }
  .spinner { width: 10px; height: 10px; border: 1.5px solid var(--accent); border-right-color: transparent; border-radius: 50%; animation: spin 0.8s linear infinite; flex: none; }
  @keyframes spin { to { transform: rotate(360deg); } }
</style>
