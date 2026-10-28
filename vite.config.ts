import { svelte } from '@sveltejs/vite-plugin-svelte'
import { defineConfig } from 'vite'

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  build: { chunkSizeWarningLimit: 6000 },
  server: { port: 1420, strictPort: true, watch: { ignored: ['**/src-tauri/**'] } },
})
