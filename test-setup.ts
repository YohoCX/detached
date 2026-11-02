import { plugin } from 'bun'
import { SveltePlugin } from 'bun-plugin-svelte'
import { JSDOM } from 'jsdom'

plugin(SveltePlugin())
const { window } = new JSDOM('')
Object.assign(globalThis, { window, document: window.document, DOMParser: window.DOMParser, Element: window.Element, Node: window.Node })
