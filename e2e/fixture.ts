import { test as base, expect, type Page } from '@playwright/test'
import { execFileSync } from 'node:child_process'
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { build } from 'vite'
import { BRIDGE_PORT } from '../playwright.config'

const bridge = `http://127.0.0.1:${BRIDGE_PORT}`
const lines = (n: number, edit: Record<number, string> = {}) =>
  Array.from({ length: n }, (_, i) => edit[i + 1] ?? `line ${i + 1}`).join('\n') + '\n'

/**
 * A temp dir of real repos. `ws/` holds app (+ worktree app-wt), group/api (behind), group/web (diverged)
 * and lib (on feature); `solo` is a repo added as its own folder. All clone the bare `origin.git`.
 */
export class Fixture {
  get home() { return join(this.root, 'home') }
  get ws() { return join(this.root, 'ws') }
  get solo() { return join(this.root, 'solo') }
  repo = (name: string) => join(this.ws, name)

  // HOME and global config in the fixture, so the developer's git config never applies.
  git = (cwd: string, ...args: string[]) => {
    const env = { ...process.env, HOME: this.home, GIT_CONFIG_GLOBAL: join(this.home, '.gitconfig'), GIT_CONFIG_NOSYSTEM: '1' }
    for (const k of ['GIT_DIR', 'GIT_WORK_TREE', 'GIT_INDEX_FILE']) delete env[k]
    return execFileSync('git', args, { cwd, env, encoding: 'utf8', stdio: 'pipe' }).trim()
  }

  write(repo: string, files: Record<string, string>) {
    for (const [path, text] of Object.entries(files)) {
      mkdirSync(dirname(join(repo, path)), { recursive: true })
      writeFileSync(join(repo, path), text)
    }
  }

  commit(repo: string, msg: string, files: Record<string, string>) {
    this.write(repo, files)
    this.git(repo, 'add', '-A')
    this.git(repo, 'commit', '-qm', msg)
  }

  constructor(public root: string) {
    const g = this.git
    mkdirSync(this.home)
    writeFileSync(join(this.home, '.gitconfig'), '[user]\n\tname = Test\n\temail = test@example.com\n[init]\n\tdefaultBranch = main\n')
    mkdirSync(join(this.ws, 'group'), { recursive: true })
    g(root, 'init', '-q', '--bare', 'origin.git')
    g(root, 'clone', '-q', 'origin.git', 'seed')
    const seed = join(root, 'seed')
    const comps = Object.fromEntries(Array.from({ length: 40 }, (_, i) => [`src/components/comp-${String(i + 1).padStart(2, '0')}.ts`, `export const c${i} = ${i}\n`]))
    this.commit(seed, 'initial', { 'README.md': lines(60), 'src/old.ts': 'export const old = 1\n', 'src/index.ts': 'export {}\n', ...comps })
    g(seed, 'push', '-q', 'origin', 'main')
    const clone = (to: string) => g(root, 'clone', '-q', 'origin.git', to)
    clone(this.repo('group/api'))
    clone(this.repo('group/web'))
    this.commit(seed, 'upstream change', { 'CHANGELOG.md': 'upstream\n' })
    g(seed, 'push', '-q', 'origin', 'main')
    this.commit(this.repo('group/web'), 'web local', { 'web.txt': 'web\n' })
    for (const r of ['group/api', 'group/web']) g(this.repo(r), 'fetch', '-q')

    const app = this.repo('app')
    clone(app)
    g(app, 'switch', '-qc', 'other')
    this.commit(app, 'other readme', { 'README.md': lines(60, { 5: 'line 5 on other' }) })
    g(app, 'switch', '-qc', 'spike', 'main')
    this.commit(app, 'spike work', { 'spike.txt': 'spike\n' })
    g(app, 'switch', '-q', 'main')
    g(app, 'worktree', 'add', '-q', '../app-wt', '-b', 'wt')
    this.write(app, { 'README.md': lines(60, { 5: 'line 5 changed', 50: 'line 50 changed' }), 'notes.txt': 'notes\n' })
    g(app, 'mv', 'src/old.ts', 'src/new.ts')

    const lib = this.repo('lib')
    clone(lib)
    g(lib, 'switch', '-qc', 'feature', 'HEAD~1')
    this.commit(lib, 'add feature', { 'feature.txt': 'feature\n' })
    this.commit(lib, 'touch index', { 'src/index.ts': 'export const touched = true\n' })

    // Long lines dense with "alpha" keep a find for it streaming past the next query's start.
    clone(this.solo)
    const data = Object.fromEntries(Array.from({ length: 1000 }, (_, i) => [`data/item-${i}.txt`, `${i} ${'alpha '.repeat(160)}\n`.repeat(6)]))
    this.commit(this.solo, 'data', { ...data, 'data/beta-a.txt': 'beta-42 first\n', 'data/beta-b.txt': 'x\nbeta-7 second\n' })
  }
}

export const test = base.extend<{ fx: Fixture }, { shim: string }>({
  shim: [
    async ({}, use) => {
      const out = await build({
        configFile: false,
        logLevel: 'warn',
        build: { write: false, minify: false, lib: { entry: 'e2e/shim.ts', formats: ['iife'], name: 'shim' } },
      })
      await use((out as { output: { code: string }[] }[])[0].output[0].code)
    },
    { scope: 'worker' },
  ],
  fx: [
    async ({ page, shim }, use) => {
      // realpath: git records worktrees under /private/var, not the /var symlink.
      const root = mkdtempSync(join(realpathSync(tmpdir()), 'wb-e2e-'))
      try {
        const fx = new Fixture(root)
        const r = await fetch(`${bridge}/reset`, { method: 'POST', body: JSON.stringify({ root: fx.root }) })
        expect(await r.text()).toContain('"ok"')
        await page.addInitScript((cfg) => Object.assign(window, { __E2E__: cfg }), { bridge, pick: [fx.ws, fx.solo] })
        await page.addInitScript({ content: shim })
        page.on('pageerror', (e) => console.log('page error:', e))
        await page.goto('/')
        await use(fx)
      } finally {
        rmSync(root, { recursive: true, force: true })
      }
    },
    { auto: true },
  ],
})

export { expect }

/** Adds the fixture's folders through the (mocked) folder picker and selects `repo`. */
export async function open(page: Page, repo?: string) {
  await page.getByRole('button', { name: 'Add folder…' }).click()
  await expect(sidebar(page).locator('.repo')).toHaveCount(6)
  if (repo) await repoRow(page, repo).click()
}

export const sidebar = (page: Page) => page.locator('aside.repos')
export const repoRow = (page: Page, name: string) =>
  sidebar(page).locator('.repo').filter({ has: page.locator('.name', { hasText: new RegExp(`^${name}$`) }) })
