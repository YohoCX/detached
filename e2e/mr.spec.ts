import type { Locator, Page } from '@playwright/test'
import { join } from 'node:path'
import { expect, forgeUrl, openForge, post, repoRow, requests, saveAccount, sidebar, test, writes } from './fixture'

const last = async (method: string, path: string) => (await writes()).filter((w) => w.method === method && w.path === path).pop()

const mrPage = (page: Page) => page.locator('main .page')
const tab = (page: Page, name: string) => mrPage(page).locator('header .seg').getByRole('button', { name })
const side = (page: Page, which: 'original' | 'modified') => mrPage(page).locator(`.editor.${which}`)
const lineNumber = (ed: Locator, n: number) => ed.locator('.line-numbers').getByText(String(n), { exact: true })
const mrTool = (page: Page) => page.locator('nav.strip').getByTitle('Merge Requests')

async function openMr(page: Page, repo: string) {
  await repoRow(page, repo).click()
  await mrTool(page).click()
  const row = page.locator('aside.tool .mr')
  await expect(row).toHaveCount(1)
  await expect(row).toContainText('Add feature')
  await expect(row).toContainText('feature → main')
  await row.click()
  await expect(mrPage(page).locator('header')).toContainText('Add feature')
}

async function openFile(page: Page, name: string) {
  await mrPage(page).locator('.files .row').filter({ hasText: name }).click()
  await expect(lineNumber(side(page, 'modified'), 1)).toBeVisible()
}

/** Hovers a line's number and checks whether a `+` shows there. */
async function plusAt(ed: Locator, n: number, shown: boolean) {
  await lineNumber(ed, n).hover()
  if (shown) return expect(ed.locator('.mr-plus')).toBeVisible()
  await ed.page().waitForTimeout(100)
  await expect(ed.locator('.mr-plus')).toBeHidden()
}

async function lineComment(page: Page, ed: Locator, n: number, text: string) {
  await plusAt(ed, n, true)
  await ed.locator('.mr-plus').click()
  const box = mrPage(page).locator('.view-zones textarea')
  await box.fill(text)
  // Monaco hides view zones from the accessibility tree, so no getByRole here.
  await mrPage(page).locator('.view-zones button', { hasText: 'Comment' }).click()
  await expect(mrPage(page).locator('.view-zones .note')).toContainText([text])
}

test('GitLab: list, page, sanitized description, line comments, links, approve, merge', async ({ page, fx }) => {
  const seed = await openForge(page, fx)
  const { baseSha, headSha } = seed.projects[0].mrs[0]
  const api = '/gitlab/api/v4/projects/group/app/merge_requests/1'
  await openMr(page, 'gitlab')

  const description = mrPage(page).locator('.description .markdown')
  await expect(description).toContainText('Adds the feature.')
  await expect(page.locator('img[onerror]')).toHaveCount(0)

  await tab(page, 'Changes').click()
  await expect(mrPage(page).locator('.files .row .name')).toHaveText(['added.txt', 'README.md', 'new.ts'])
  await openFile(page, 'README.md')
  const position = { position_type: 'text', base_sha: baseSha, start_sha: baseSha, head_sha: headSha, old_path: 'README.md', new_path: 'README.md' }
  await lineComment(page, side(page, 'modified'), 3, 'Changed line')
  expect((await last('POST', `${api}/discussions`))?.body).toEqual({ body: 'Changed line', position: { ...position, new_line: 3 } })
  await lineComment(page, side(page, 'original'), 5, 'Unchanged line')
  expect((await last('POST', `${api}/discussions`))?.body).toEqual({ body: 'Unchanged line', position: { ...position, old_line: 5, new_line: 5 } })

  await tab(page, 'Overview').click()
  const activity = mrPage(page).locator('.overview > .thread')
  await expect(activity.locator('.where')).toHaveText(['README.md:3', 'README.md:5'])
  const box = activity.last().locator('textarea')
  await box.fill('See [the docs](docs/a.md) and [this](javascript:alert(1))')
  await activity.last().getByRole('button', { name: 'Comment' }).click()
  await activity.getByRole('link', { name: 'the docs' }).click()
  expect((await last('POST', `${api}/notes`))?.body).toEqual({ body: 'See [the docs](docs/a.md) and [this](javascript:alert(1))' })
  await activity.getByText('this', { exact: true }).click()
  expect(await page.evaluate(() => window.__E2E__.opened)).toEqual([`${forgeUrl}/gitlab/group/app/docs/a.md`])

  await mrPage(page).getByRole('button', { name: 'Approve' }).click()
  await expect(mrPage(page).getByRole('button', { name: 'Unapprove' })).toBeVisible()
  expect(await last('POST', `${api}/approve`)).toBeTruthy()
  await expect(mrPage(page).locator('dl')).toContainText(/Approved by\s*tester/)

  await mrPage(page).locator('.merge').getByRole('button', { name: 'Squash' }).click()
  await mrPage(page).locator('.merge').getByText('Delete source branch').click()
  await mrPage(page).locator('.merge').getByRole('button', { name: 'Merge', exact: true }).click()
  await expect(mrPage(page).locator('header .pill')).toHaveText('merged')
  expect((await last('PUT', `${api}/merge`))?.body).toEqual({ squash: true, should_remove_source_branch: true, sha: headSha })
})

test('GitHub: + only inside hunks, a renamed file, request changes needs text, approve, merge', async ({ page, fx }) => {
  const seed = await openForge(page, fx)
  const { headSha } = seed.projects[1].mrs[0]
  const api = '/ghe/api/v3/repos/owner/app/pulls/1'
  await openMr(page, 'github')
  await expect(page.locator('img[onerror]')).toHaveCount(0)

  await tab(page, 'Changes').click()
  await openFile(page, 'README.md')
  const [left, right] = [side(page, 'original'), side(page, 'modified')]
  await plusAt(right, 20, false)
  await plusAt(right, 6, true)
  await plusAt(right, 7, false)

  await openFile(page, 'new.ts')
  await plusAt(right, 2, false)
  await plusAt(left, 2, false)
  await plusAt(left, 8, true)
  await lineComment(page, right, 9, 'In the hunk')
  expect((await last('POST', `${api}/comments`))?.body).toEqual({ body: 'In the hunk', commit_id: headSha, path: 'src/new.ts', line: 9, side: 'RIGHT' })

  await tab(page, 'Overview').click()
  const request = mrPage(page).getByRole('button', { name: 'Request changes' })
  await expect(request).toBeDisabled()
  await mrPage(page).locator('.review textarea').fill('Please split this')
  await request.click()
  await expect(mrPage(page).locator('.review textarea')).toHaveValue('')
  expect((await last('POST', `${api}/reviews`))?.body).toEqual({ event: 'REQUEST_CHANGES', body: 'Please split this' })
  await expect(mrPage(page).locator('.overview > .thread').filter({ hasText: 'Please split this' })).toBeVisible()

  await mrPage(page).locator('.review').getByRole('button', { name: 'Approve' }).click()
  await expect(mrPage(page).locator('dl')).toContainText(/Approved by\s*tester/)
  expect((await last('POST', `${api}/reviews`))?.body).toEqual({ event: 'APPROVE' })

  await mrPage(page).locator('.merge').getByRole('button', { name: 'Merge', exact: true }).click()
  await expect(mrPage(page).locator('header .pill')).toHaveText('merged')
  expect((await last('PUT', `${api}/merge`))?.body).toEqual({ merge_method: 'merge', sha: headSha })
})

test('the top bar chip opens the current branch’s MR', async ({ page, fx }) => {
  await openForge(page, fx)
  await repoRow(page, 'github').click()
  const chip = page.locator('header .chip')
  await expect(chip).toContainText('#1')
  await chip.click()
  await expect(mrPage(page).locator('header')).toContainText('#1')
  await expect(mrPage(page).locator('header')).toContainText('Add feature')
})

test('creates an MR from the current branch, showing the target project', async ({ page, fx }) => {
  await openForge(page, fx)
  const repo = join(fx.forge, 'gitlab')
  fx.git(repo, 'switch', '-qc', 'topic')
  fx.commit(repo, 'topic work', { 'topic.txt': 'topic\n' })
  fx.git(repo, 'update-ref', 'refs/remotes/origin/topic', 'HEAD')
  fx.git(repo, 'branch', '-q', '--set-upstream-to', 'origin/topic')
  await repoRow(page, 'gitlab').click()
  await sidebar(page).getByTitle('Refresh').click()
  await page.getByTitle('Create Merge Request').click()
  const form = page.getByRole('dialog', { name: 'Create Merge Request' })
  await expect(form).toContainText('group/app:topic into group/app')
  await expect(form.getByLabel('Title')).toHaveValue('topic work')
  await expect(form.getByLabel('Target branch')).toHaveValue('main')
  await form.getByLabel('Description').fill('Why')
  await form.getByRole('button', { name: 'Create' }).click()
  await expect(mrPage(page).locator('header')).toContainText('!2')
  expect((await last('POST', '/gitlab/api/v4/projects/group/app/merge_requests'))?.body)
    .toEqual({ source_branch: 'topic', target_branch: 'main', title: 'topic work', description: 'Why' })
  await expect(page.locator('header .chip')).toContainText('!2')
})

test('an ambiguous host says so and Settings picks the account', async ({ page, fx }) => {
  await openForge(page, fx)
  await saveAccount('gitlabSelfHosted', 'gitlab', 'other', 'Second GitLab')
  await repoRow(page, 'gitlab').click()
  await mrTool(page).click()
  const list = page.locator('aside.tool')
  await expect(list).toContainText('Several accounts fit 127.0.0.1')
  await list.getByRole('button', { name: 'Open Settings' }).click()
  const section = page.getByRole('dialog', { name: 'Settings' }).locator('section', { hasText: 'Merge requests for gitlab' })
  await expect(section.locator('select').first()).toHaveValue('')
  await section.locator('select').nth(1).selectOption({ label: 'Fake GitLab' })
  await page.keyboard.press('Escape')
  await list.getByTitle('Refresh').click()
  await expect(list.locator('.mr')).toContainText('Add feature')
})

test('the MR list is one request, and a repo returned to shows its cached list at once', async ({ page, fx }) => {
  await openForge(page, fx)
  await repoRow(page, 'gitlab').click()
  await expect(page.locator('header .chip')).toContainText('!1')
  const before = (await requests()).length
  await mrTool(page).click()
  const rows = page.locator('aside.tool .mr')
  await expect(rows).toContainText('Add feature')
  const lists = (await requests()).slice(before).filter((r) => /merge_requests|graphql/.test(r.path))
  expect(lists).toEqual([{ method: 'POST', path: '/gitlab/api/graphql' }])

  await repoRow(page, 'github').click()
  await expect(rows).toContainText('#1')
  await post(`${forgeUrl}/__delay`, { ms: 3000 })
  const back = (await requests()).length
  await repoRow(page, 'gitlab').click()
  await expect(rows).toContainText('!1', { timeout: 1000 })
  await expect.poll(async () => (await requests()).slice(back).some((r) => r.path === '/gitlab/api/graphql')).toBe(true)
})
