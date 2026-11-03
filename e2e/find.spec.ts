import type { Page } from '@playwright/test'
import { expect, open, test } from './fixture'

const find = (page: Page) => page.locator('.find')
const sorted = async (page: Page) => (await find(page).locator('.match').allInnerTexts()).sort()

test('find in files groups by file, highlights, and a new query replaces the old one', async ({ page }) => {
  await open(page, 'solo')
  await page.keyboard.press('ControlOrMeta+Shift+F')
  const input = find(page).getByPlaceholder('Text to find')
  await input.fill('alpha')
  await expect(find(page).locator('.file .name').first()).toHaveText(/^item-\d+\.txt$/)
  await expect(find(page).locator('.match .hit').first()).toHaveText('alpha')
  // Replace the query while the first one still streams.
  await expect(find(page).locator('.head')).toContainText('Searching…')
  await input.fill('beta-')
  await expect(find(page).locator('.head')).toContainText('2 matches in 2 files')
  expect((await find(page).locator('.file .name').allInnerTexts()).sort()).toEqual(['beta-a.txt', 'beta-b.txt'])
  await expect(find(page).locator('.match .hit')).toHaveText(['beta-', 'beta-'])
  await expect(find(page).locator('.match', { hasText: 'alpha' })).toHaveCount(0)
})

test('find at a ref matches a \\d regex on the same lines as the working tree', async ({ page }) => {
  await open(page, 'solo')
  await page.keyboard.press('ControlOrMeta+Shift+F')
  await find(page).getByTitle('Regex').click()
  await find(page).getByPlaceholder('Text to find').fill('beta-\\d+')
  await expect(find(page).locator('.head')).toContainText('2 matches in 2 files')
  const worktree = await sorted(page)
  await page.keyboard.press('Escape')

  await page.keyboard.press('ControlOrMeta+1')
  await page.getByPlaceholder('Working tree').fill('main')
  await page.keyboard.press('Enter')
  await page.keyboard.press('ControlOrMeta+Shift+F')
  await expect(find(page).locator('.scope')).toHaveText('solo @ main')
  await expect(find(page).locator('.head')).toContainText('2 matches in 2 files')
  expect(await sorted(page)).toEqual(worktree)
})
