import { expect, forgeUrl, openForge, repoRow, test } from './fixture'

test('the forge repos track the seeded fake; Cmd+9 opens the bottom panel', async ({ page, fx }) => {
  const seed = await openForge(page, fx)
  expect(await (await fetch(`${forgeUrl}/health`)).json()).toEqual({ projects: 2 })
  for (const [name, p] of [['gitlab', seed.projects[0]], ['github', seed.projects[1]]] as const) {
    await expect(repoRow(page, name)).toBeVisible()
    expect(fx.git(`${fx.forge}/${name}`, 'rev-parse', '@{u}')).toBe(p.mrs[0].headSha)
  }

  await page.keyboard.press('ControlOrMeta+9')
  const panel = page.locator('.bottom')
  await expect(panel.getByRole('button', { name: 'Log' })).toBeVisible()
  await panel.getByRole('button', { name: 'Pipelines' }).click()
  await expect(panel).toContainText('Pipelines')
  await page.keyboard.press('ControlOrMeta+9')
  await expect(panel.getByRole('button', { name: 'Log' })).toHaveClass(/on/)
  await page.keyboard.press('ControlOrMeta+9')
  await expect(panel).toBeHidden()
})
