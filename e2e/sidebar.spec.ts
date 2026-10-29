import { rmSync } from 'node:fs'
import { expect, open, repoRow, sidebar, test } from './fixture'

test('adding a folder lists nested repos, worktrees under their repo and branch state', async ({ page }) => {
  await open(page)
  const side = sidebar(page)
  await expect(side.locator('.repo .name')).toHaveText(['app', 'app-wt', 'group/api', 'group/web', 'lib', 'solo'])
  // `solo` is a folder that is itself a repo: no header for it.
  await expect(side.locator('.folder')).toHaveText(['ws'])
  const wt = repoRow(page, 'app-wt')
  await expect(wt).toHaveClass(/worktree/)
  const x = async (name: string) => (await repoRow(page, name).locator('.name').boundingBox())!.x
  expect(await x('app-wt')).toBeGreaterThan(await x('app'))
  await expect(repoRow(page, 'group/api').locator('.line').nth(1)).toHaveText('main ↓1')
  await expect(repoRow(page, 'group/web').locator('.line').nth(1)).toHaveText('main ↑1 ↓1')
  await expect(repoRow(page, 'lib').locator('.line').nth(1)).toHaveText('feature')

  await repoRow(page, 'app').getByTitle('Hide 1 worktree').click()
  await expect(side.locator('.repo .name')).toHaveText(['app', 'group/api', 'group/web', 'lib', 'solo'])
  await side.locator('.folder').click()
  await expect(side.locator('.repo .name')).toHaveText(['solo'])
})

test('a worktree deleted on disk drops out when the window gets focus', async ({ page, fx }) => {
  await open(page)
  rmSync(fx.repo('app-wt'), { recursive: true })
  await page.evaluate(() => window.dispatchEvent(new Event('focus')))
  await expect(sidebar(page).locator('.repo .name')).toHaveText(['app', 'group/api', 'group/web', 'lib', 'solo'])
  await expect(sidebar(page).locator('.err')).toHaveCount(0)
})

test('double-clicking empty sidebar space selects no text', async ({ page }) => {
  await open(page)
  const box = (await sidebar(page).boundingBox())!
  await page.mouse.dblclick(box.x + box.width / 2, box.y + box.height - 20)
  expect(await page.evaluate(() => window.getSelection()!.toString())).toBe('')
})

test('right-clicking the sidebar suppresses the web view menu', async ({ page }) => {
  await open(page)
  const prevented = page.evaluate(
    () => new Promise((done) => window.addEventListener('contextmenu', (e) => done(e.defaultPrevented), { once: true })),
  )
  const box = (await sidebar(page).boundingBox())!
  await page.mouse.click(box.x + box.width / 2, box.y + box.height - 20, { button: 'right' })
  expect(await prevented).toBe(true)
})

test('the repository button opens a switcher: type, Enter selects', async ({ page }) => {
  await open(page, 'app')
  await page.getByTitle('Switch repository').click()
  await expect(page.getByRole('option', { selected: true })).toHaveAttribute('title', /\/app$/)
  await page.keyboard.type('lib')
  await page.keyboard.press('Enter')
  await expect(page.getByRole('dialog', { name: 'Switch repository' })).toHaveCount(0)
  await expect(page.getByTitle('Switch repository')).toHaveText('lib')
  await expect(repoRow(page, 'lib')).toHaveClass(/selected/)
})
