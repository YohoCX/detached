import { mount } from 'svelte'
import './app.css'
import App from './App.svelte'

// Text fields here hold paths, refs and queries, not prose: no macOS autocorrect bubble or capitalization.
document.addEventListener('focusin', (e) => {
  const t = e.target
  if (t instanceof HTMLInputElement || t instanceof HTMLTextAreaElement) {
    for (const [k, v] of [['autocorrect', 'off'], ['autocapitalize', 'off'], ['autocomplete', 'off'], ['spellcheck', 'false']])
      t.setAttribute(k, v)
  }
})

// The web view's own menu (Reload, Inspect Element) is not part of the app; text fields and Monaco keep theirs.
document.addEventListener('contextmenu', (e) => {
  if (!(e.target as Element).closest('input, textarea, .monaco-editor')) e.preventDefault()
})

export default mount(App, { target: document.getElementById('app')! })
