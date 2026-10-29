import type { Page } from '@playwright/test'
import { expect, open, test } from './fixture'

const changes = (page: Page) => page.locator('[data-changes]')
const item = (page: Page, name: string) => changes(page).locator('.item').filter({ has: page.locator('.name', { hasText: name }) })

test('changes: list, diff on click and double-click, file on Cmd+Down, F7 to the next change', async ({ page }) => {
  await open(page, 'app')
  await expect(changes(page).locator('.item .name')).toHaveText(['README.md', 'new.ts', 'notes.txt'])
  await expect(item(page, 'README.md')).toHaveClass(/modified/)
  await expect(item(page, 'new.ts')).toHaveClass(/renamed/)
  await expect(item(page, 'notes.txt')).toHaveClass(/untracked/)

  const header = page.locator('.viewer header')
  await item(page, 'README.md').click()
  await expect(header).toContainText('HEAD ↔ Working tree')
  await item(page, 'README.md').press('Meta+ArrowDown')
  await expect(header.locator('.name')).toHaveText('README.md')
  await expect(header).not.toContainText('↔')

  await item(page, 'README.md').dblclick()
  await expect(header).toContainText('HEAD ↔ Working tree')
  const cursor = page.locator('.monaco-diff-editor .editor.modified .active-line-number')
  await expect(cursor).toHaveText('5')
  await page.keyboard.press('F7')
  await expect(cursor).toHaveText('50')
})

test('compare lists only the branch’s changes since the merge base', async ({ page }) => {
  await open(page, 'lib')
  await changes(page).getByRole('button', { name: 'Compare' }).click()
  await expect(changes(page).locator('.ab')).toHaveText('↑2 ↓1')
  await expect(changes(page).locator('.group')).toContainText('Changes since origin/main')
  await expect(changes(page).locator('.item .name')).toHaveText(['feature.txt', 'index.ts'])
})
