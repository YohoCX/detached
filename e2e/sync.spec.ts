import { expect, open, repoRow, test } from './fixture'

test('fetch all and pull all report per repo; the diverged repo is left as it was', async ({ page, fx }) => {
  await open(page)
  const [api, web] = [fx.repo('group/api'), fx.repo('group/web')]
  const webHead = fx.git(web, 'rev-parse', 'HEAD')
  const outcome = (name: string) => repoRow(page, name).locator('.outcome')

  await page.getByRole('button', { name: 'Fetch All' }).click()
  for (const name of ['app', 'group/api', 'group/web', 'solo']) await expect(outcome(name)).toHaveText('Fetched')

  await page.getByRole('button', { name: 'Pull All' }).click()
  await expect(outcome('group/api')).toHaveText(/^Fast-forwarded \w{7}\.\.\w{7}$/)
  await expect(outcome('group/web')).toHaveText('Diverged (↑1 ↓1), not updated')
  await expect(outcome('app')).toHaveText('Up to date')
  await expect(outcome('lib')).toHaveText('No upstream')
  await expect(page.locator('.notice')).toContainText('Pull all: 1 not updated')
  await expect(repoRow(page, 'group/api').locator('.line').nth(1)).toHaveText(/^\s*main\s*Fast-forwarded/)

  await expect(outcome('group/api')).toHaveCount(0, { timeout: 10_000 })
  await expect(outcome('group/web')).toHaveText('Diverged (↑1 ↓1), not updated')

  expect(fx.git(api, 'rev-parse', 'HEAD')).toBe(fx.git(api, 'rev-parse', 'origin/main'))
  expect(fx.git(web, 'rev-parse', 'HEAD')).toBe(webHead)
})
