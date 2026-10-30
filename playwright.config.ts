import { defineConfig, devices } from '@playwright/test'

export const VITE_PORT = 1520
export const BRIDGE_PORT = 1521
export const FORGE_PORT = 1522

export default defineConfig({
  testDir: 'e2e',
  // One bridge holds one fixture's state at a time.
  workers: 1,
  fullyParallel: false,
  forbidOnly: true,
  reporter: 'list',
  use: { ...devices['Desktop Safari'], baseURL: `http://localhost:${VITE_PORT}`, viewport: { width: 1400, height: 900 } },
  projects: [{ name: 'webkit', use: { browserName: 'webkit' } }],
  webServer: [
    {
      command: `E2E_BRIDGE_PORT=${BRIDGE_PORT} pnpm exec vite --port ${VITE_PORT} --strictPort`,
      url: `http://localhost:${VITE_PORT}`,
    },
    {
      command: `cargo run --quiet --manifest-path src-tauri/Cargo.toml --features e2e --bin e2e-bridge -- ${BRIDGE_PORT}`,
      url: `http://127.0.0.1:${BRIDGE_PORT}/`,
      timeout: 600_000,
    },
    {
      command: `node e2e/forge.ts ${FORGE_PORT}`,
      url: `http://127.0.0.1:${FORGE_PORT}/health`,
    },
  ],
})
