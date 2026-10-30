/// <reference types="vitest/config" />
import { svelte } from '@sveltejs/vite-plugin-svelte'
import { readFileSync } from 'node:fs'
import { defineConfig } from 'vite'

// e2e serves the app under the bundle's CSP, plus Vite's own connections and the e2e bridge.
const bridge = process.env.E2E_BRIDGE_PORT
const csp = (): string =>
  JSON.parse(readFileSync('src-tauri/tauri.conf.json', 'utf8')).app.security.csp
    .replace('connect-src', `connect-src 'self' ws: http://127.0.0.1:${bridge}`)

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  build: { chunkSizeWarningLimit: 6000 },
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ['**/src-tauri/**', '**/.claude/**'] },
    headers: bridge ? { 'Content-Security-Policy': csp() } : {},
  },
  test: { include: ['src/**/*.test.ts'] },
})
