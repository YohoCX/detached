import { test as base, expect, type Page } from '@playwright/test'
import { execFileSync } from 'node:child_process'
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { BRIDGE_PORT, FORGE_PORT } from '../playwright.config'
import type { Seed, SeedProject, Write } from './forge'

const bridge = `http://127.0.0.1:${BRIDGE_PORT}`
export const forgeUrl = `http://127.0.0.1:${FORGE_PORT}`
export const post = (url: string, body: unknown) => fetch(url, { method: 'POST', body: JSON.stringify(body) })
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
  get forge() { return join(this.root, 'forge') }
  repo = (name: string) => join(this.ws, name)

  // HOME and global config in the fixture, so the developer's git config never applies.
  git = (cwd: string, ...args: string[]) => {
    const env: NodeJS.ProcessEnv = { ...process.env, HOME: this.home, GIT_CONFIG_GLOBAL: join(this.home, '.gitconfig'), GIT_CONFIG_NOSYSTEM: '1' }
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

/**
 * `forge/gitlab` and `forge/github`, each on `feature` tracking `origin`, which points at the fake forge:
 * MR 1 is `feature` (head) into `main` (base) and both commits are local, so nothing fetches from the fake.
 */
function makeForge(fx: Fixture): Seed {
  const g = fx.git
  const projects = (['gitlab', 'ghe'] as const).map((kind, i): SeedProject => {
    const repo = join(fx.forge, kind === 'gitlab' ? 'gitlab' : 'github')
    const path = kind === 'gitlab' ? 'group/app' : 'owner/app'
    mkdirSync(repo, { recursive: true })
    g(repo, 'init', '-q')
    fx.commit(repo, 'initial', { 'README.md': lines(30), 'src/old.ts': lines(10) })
    const baseSha = g(repo, 'rev-parse', 'HEAD')
    g(repo, 'switch', '-qc', 'feature')
    g(repo, 'mv', 'src/old.ts', 'src/new.ts')
    fx.commit(repo, 'feature work', {
      'README.md': lines(30, { 3: 'line 3 changed' }),
      'src/new.ts': lines(10, { 10: 'line 10 changed' }),
      'added.txt': 'added\n',
    })
    const headSha = g(repo, 'rev-parse', 'HEAD')
    g(repo, 'remote', 'add', 'origin', `${forgeUrl}/${kind}/${path}.git`)
    g(repo, 'update-ref', 'refs/remotes/origin/main', baseSha)
    g(repo, 'update-ref', 'refs/remotes/origin/feature', headSha)
    g(repo, 'symbolic-ref', 'refs/remotes/origin/HEAD', 'refs/remotes/origin/main')
    g(repo, 'branch', '-q', '--set-upstream-to', 'origin/feature')
    const id = (i + 1) * 100
    return {
      forge: kind,
      path,
      defaultBranch: 'main',
      mrs: [{
        number: 1, title: 'Add feature', description: 'Adds the feature. <img src=x onerror=alert(1)>', author: 'tester',
        state: 'open', draft: false, sourceBranch: 'feature', targetBranch: 'main', baseSha, headSha,
      }],
      pipelines: [{
        id, ref: 'feature', sha: headSha, status: 'success',
        jobs: [{ id: id + 1, name: 'build', stage: 'build', status: 'success', log: '\x1b[32mbuilt\x1b[0m <img src=x onerror=alert(1)>\n' }],
      }],
    }
  })
  return { user: 'tester', projects }
}

export const test = base.extend<{ fx: Fixture }, { shim: string }>({
  shim: [
    async ({}, use) => {
      await use(execFileSync('bun', ['build', 'e2e/shim.ts', '--format=iife'], { encoding: 'utf8' }))
    },
    { scope: 'worker' },
  ],
  fx: [
    async ({ page, shim }, use) => {
      // realpath: git records worktrees under /private/var, not the /var symlink.
      const root = mkdtempSync(join(realpathSync(tmpdir()), 'wb-e2e-'))
      try {
        const fx = new Fixture(root)
        expect(await (await post(`${bridge}/reset`, { root: fx.root })).text()).toContain('"ok"')
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

/** Runs a command over the bridge, outside the page; throws its error. */
export async function invoke(cmd: string, args: object = {}) {
  const r = JSON.parse((await (await post(`${bridge}/invoke`, { cmd, args })).text()).trim().split('\n').pop()!)
  if ('err' in r) throw new Error(`${cmd}: ${JSON.stringify(r.err)}`)
  return r.ok
}

export const saveAccount = (provider: string, kind: string, username: string, label: string) =>
  invoke('save_account', { input: { id: null, provider, baseUrl: `${forgeUrl}/${kind}`, username, label, token: `${kind}-token` } })

export const writes = async (): Promise<Write[]> => (await fetch(`${forgeUrl}/__writes`)).json()
export const requests = async (): Promise<{ method: string; path: string }[]> => (await fetch(`${forgeUrl}/__requests`)).json()

/**
 * Seeds the fake forge from fresh `forge/` repos, saves a GitLab and a GitHub Enterprise account at its URL
 * and adds only the `forge/` folder.
 */
export async function openForge(page: Page, fx: Fixture): Promise<Seed> {
  const seed = makeForge(fx)
  expect((await post(`${forgeUrl}/reset`, seed)).ok).toBe(true)
  await saveAccount('gitlabSelfHosted', 'gitlab', seed.user, 'Fake GitLab')
  await saveAccount('githubEnterprise', 'ghe', seed.user, 'Fake GitHub')
  await page.evaluate((pick) => (window.__E2E__.pick = pick), [fx.forge])
  await page.getByRole('button', { name: 'Add folder…' }).click()
  await expect(sidebar(page).locator('.repo')).toHaveCount(2)
  return seed
}

export const sidebar = (page: Page) => page.locator('aside.repos')
export const repoRow = (page: Page, name: string) =>
  sidebar(page).locator('.repo').filter({ has: page.locator('.name', { hasText: new RegExp(`^${name}$`) }) })
