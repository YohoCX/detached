import { defineConfig, devices } from '@playwright/test'

// E2E_PORT moves all three ports, so two checkouts can run e2e at once.
const base = Number(process.env.E2E_PORT ?? 1520)
export const APP_PORT = base
export const BRIDGE_PORT = base + 1
export const FORGE_PORT = base + 2

export default defineConfig({
  testDir: 'e2e',
  // One bridge holds one fixture's state at a time.
  workers: 1,
  fullyParallel: false,
  forbidOnly: true,
  reporter: 'list',
  use: { ...devices['Desktop Safari'], baseURL: `http://localhost:${APP_PORT}`, viewport: { width: 1400, height: 900 } },
  projects: [{ name: 'webkit', use: { browserName: 'webkit' } }],
  webServer: [
    {
      command: `bun build.ts serve ${APP_PORT} ${BRIDGE_PORT}`,
      url: `http://localhost:${APP_PORT}`,
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
