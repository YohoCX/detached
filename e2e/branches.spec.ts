import type { Page } from '@playwright/test'
import { expect, open, test } from './fixture'

const popup = (page: Page) => page.locator('.popup')

/** Types into the branches popup's search and waits until `branch` is the selected row. */
async function search(page: Page, placeholder: string, branch: string) {
  await popup(page).getByPlaceholder(placeholder).pressSequentially(branch)
  await expect(popup(page).locator('.item.selected')).toHaveText(branch)
}

test('checkout over conflicting local changes keeps the stash and names it', async ({ page, fx }) => {
  await open(page, 'app')
  await page.getByTitle('Branches').click()
  await search(page, 'Search branches', 'other')
  await page.keyboard.press('Enter')
  const notice = page.locator('.notice.error')
  await expect(notice.locator('.title span')).toHaveText('Checked out other, your changes did not apply cleanly')
  await expect(notice.locator('pre')).toContainText('Stash kept: workbench: main -> other')
  await expect(notice.locator('li .mono')).toHaveText(['README.md'])
  expect(fx.git(fx.repo('app'), 'stash', 'list')).toContain('workbench: main -> other')
})

test('deleting an unmerged branch lists its commits and asks first', async ({ page, fx }) => {
  const app = fx.repo('app')
  await open(page, 'app')
  await page.keyboard.press('Control+V')
  await page.keyboard.type('Delete Branch')
  await page.keyboard.press('Enter')
  await search(page, 'Delete branch…', 'spike')
  await page.keyboard.press('Enter')
  await expect(popup(page).locator('.title')).toHaveText('spike is not fully merged')
  await expect(popup(page).locator('.commit')).toContainText(['spike work'])
  expect(fx.git(app, 'branch', '--list', 'spike')).toContain('spike')
  await popup(page).getByRole('button', { name: 'Delete anyway' }).click()
  await expect.poll(() => fx.git(app, 'branch', '--list', 'spike')).toBe('')
})

test('a new branch named -D is refused and its start branch survives', async ({ page, fx }) => {
  await open(page, 'app')
  await page.getByTitle('Branches').click()
  await search(page, 'Search branches', 'other')
  await page.keyboard.press('ArrowRight')
  await popup(page).getByText("New Branch from 'other'…").click()
  await popup(page).getByPlaceholder('Branch name').fill('-D')
  await popup(page).getByRole('button', { name: 'Create' }).click()
  await expect(page.locator('.notice.error')).toContainText('invalid name: "-D"')
  expect(fx.git(fx.repo('app'), 'branch', '--list', 'other')).toContain('other')
})

test('Enter right after pasting a query checks out the first match, even before branches load', async ({ page, fx }) => {
  await open(page, 'lib')
  // Hold list_branches back so Enter surely arrives before the branches do.
  await page.route('**/invoke', async (route) => {
    if (route.request().postDataJSON()?.cmd === 'list_branches') await new Promise((r) => setTimeout(r, 800))
    await route.continue()
  })
  await page.getByTitle('Branches').click()
  await page.keyboard.insertText('main')
  await page.keyboard.press('Enter')
  await expect.poll(() => fx.git(fx.repo('lib'), 'branch', '--show-current')).toBe('main')
})
