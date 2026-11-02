import { expect, open, test } from './fixture'

const guide = `# Guide

[Usage](#usage) [Other](other.md) [Web](https://example.com/x)

![logo](img/logo.svg)

- [x] done

\`\`\`ts
const a = 1
\`\`\`

\`\`\`mermaid
graph TD; A-->B
\`\`\`

\`\`\`mermaid
graph TD; A-->
\`\`\`
${'\nfiller\n'.repeat(80)}
## Usage

end
`

test('markdown preview: code, mermaid, repo images, anchors and links; Source sticks', async ({ page, fx }) => {
  fx.commit(fx.repo('lib'), 'docs', {
    'docs/guide.md': guide,
    'docs/other.md': '# Other\n',
    'docs/img/logo.svg': '<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"/>',
  })
  await open(page, 'lib')
  const goTo = async (name: string) => {
    await page.keyboard.press('Meta+Shift+O')
    await page.keyboard.type(name)
    await expect(page.locator('.palette .item .name').first()).toHaveText(name)
    await page.keyboard.press('Enter')
  }
  await goTo('guide.md')

  const doc = page.locator('.viewer .preview .markdown')
  await expect(doc.locator('h1')).toHaveText('Guide')
  await expect(doc.locator('input[type=checkbox]')).toBeDisabled()
  await expect(doc.locator('code[data-lang=ts] span').first()).toBeVisible()
  await expect(doc.locator('.mermaid svg')).toHaveCount(1)
  await expect(doc.locator('.mermaid-error')).toContainText('Parse error')
  await expect(page.locator('[id^=dmermaid]')).toHaveCount(0)
  const light = await doc.locator('.mermaid svg').getAttribute('id')
  await page.emulateMedia({ colorScheme: 'dark' })
  await expect(doc.locator('.mermaid svg')).not.toHaveAttribute('id', light!)
  await expect(doc.locator('.mermaid svg')).toHaveCount(1)
  await expect(doc.locator('.mermaid-error')).toHaveCount(1)
  await expect(doc.locator('img')).toHaveAttribute('src', /^data:image\/svg\+xml;base64,/)

  await expect(doc.locator('h2')).not.toBeInViewport()
  await doc.getByRole('link', { name: 'Usage' }).click()
  await expect(doc.locator('h2')).toBeInViewport()
  await doc.getByRole('link', { name: 'Web' }).click()
  expect(await page.evaluate(() => window.__E2E__.opened)).toEqual(['https://example.com/x'])
  await doc.getByRole('link', { name: 'Other' }).click()
  await expect(page.locator('.viewer header .name')).toHaveText('other.md')
  await expect(doc.locator('h1')).toHaveText('Other')

  await page.getByRole('button', { name: 'Source' }).click()
  await expect(doc).toBeHidden()
  await expect(page.locator('.viewer .monaco-editor').first()).toContainText('# Other')
  await goTo('guide.md')
  await expect(page.locator('.viewer .monaco-editor').first()).toContainText('# Guide')
  await expect(doc).toBeHidden()
})
