import { expect, open, test } from './fixture'

test('go to file: filters, moves by keyboard under a resting mouse, opens, ignores case', async ({ page }) => {
  await open(page, 'app')
  await page.keyboard.press('Meta+Shift+O')
  const pal = page.locator('.palette')
  await expect(pal.locator('.mode')).toHaveText('Go to File')
  await page.keyboard.type('comp')
  const items = pal.locator('.item')
  await expect(items).toHaveCount(40)

  const list = (await pal.locator('.list').boundingBox())!
  await page.mouse.move(list.x + list.width / 2, list.y + list.height / 2)
  const selected = () => items.evaluateAll((els) => els.findIndex((e) => e.classList.contains('selected')))
  const start = await selected()
  // WebKit sends a mousemove a moment after content scrolls under a resting pointer.
  const moved = page.evaluate(() => new Promise((done) => document.addEventListener('mousemove', done, { once: true, capture: true })))
  for (let i = 0; i < 20; i++) await page.keyboard.press('ArrowDown')
  await moved
  await expect.poll(selected).toBe(start + 20)
  for (let i = 0; i < 3; i++) await page.keyboard.press('ArrowUp')
  await expect.poll(selected).toBe(start + 17)
  const name = (await items.nth(start + 17).locator('.name').textContent())!
  await page.keyboard.press('Enter')
  await expect(pal).toBeHidden()
  await expect(page.locator('.viewer header .name')).toHaveText(name)
  await expect(page.locator('.viewer .monaco-editor').first()).toContainText(`export const c${start + 17}`)

  await page.keyboard.press('Meta+Shift+O')
  await page.keyboard.type('COMP-07')
  await expect(items.first().locator('.name')).toHaveText('comp-07.ts')
})

test('keymap opens each palette and settings', async ({ page }) => {
  await open(page, 'app')
  const mode = page.locator('.palette .mode')
  const rows: [string[], string][] = [
    [['Shift', 'Shift'], 'Search Everywhere'],
    [['Meta+Shift+A'], 'Find Action'],
    [['Control+V'], 'VCS Operations'],
    [['Meta+E'], 'Recent Files'],
  ]
  for (const [keys, title] of rows) {
    for (const k of keys) await page.keyboard.press(k)
    await expect(mode).toHaveText(title)
    await page.keyboard.press('Escape')
    await expect(mode).toBeHidden()
  }
  await page.keyboard.press('Meta+Comma')
  await expect(page.getByRole('dialog', { name: 'Settings' })).toBeVisible()
})
