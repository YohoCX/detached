import type { Page } from '@playwright/test'
import { expect, forgeUrl, invoke, openForge, post, repoRow, test, writes } from './fixture'

const append = (body: object) => post(`${forgeUrl}/__append`, body)
const wrote = (method: string, path: string, body?: Record<string, unknown>) =>
  expect.poll(writes).toContainEqual(expect.objectContaining({ method, path, ...(body && { body: expect.objectContaining(body) }) }))

const panel = (page: Page) => page.locator('.pipes')
const job = (page: Page, id: number) => panel(page).locator(`[data-job="${id}"]`)
const log = (page: Page) => page.locator('.log')

/** Opens the forge repo's Pipelines tab from the badge, which shows the branch's status, and expands pipeline `id`. */
async function pipelines(page: Page, repo: string, id: number) {
  await repoRow(page, repo).click()
  const badge = page.locator('header .badge')
  await expect(badge).toHaveText('✓success')
  await badge.click()
  const row = panel(page).locator(`[data-pipeline="${id}"]`)
  await expect(row).toContainText(`#${id}`)
  await expect(row).toContainText('feature')
  await row.locator('.main').click()
  await expect(job(page, id + 1)).toContainText('build')
  return row
}

/** The seeded log: colour as a span, `<img onerror>` as literal text, no element. */
async function seededLog(page: Page) {
  await expect(log(page).locator('.lines')).toContainText('built <img src=x onerror=alert(1)>')
  await expect(log(page).locator('.lines span', { hasText: /^built$/ })).toHaveAttribute('style', /var\(--ansi-2\)/)
  await expect(page.locator('.log img')).toHaveCount(0)
}

test('GitLab: pipelines and jobs, a job log, retry, a live log, run with variables, cancel and retry it', async ({ page, fx }) => {
  await openForge(page, fx)
  const row = await pipelines(page, 'gitlab', 100)
  await expect(panel(page).locator('.stage')).toHaveText('✓build')
  await expect(row.locator('.act')).toHaveCount(0)

  await job(page, 101).locator('.main').click()
  await expect(log(page).locator('header')).toContainText('group/app #101')
  await seededLog(page)

  await job(page, 101).getByRole('button', { name: 'Retry' }).click()
  await wrote('POST', '/gitlab/api/v4/projects/group/app/jobs/101/retry')
  await expect(job(page, 101).getByRole('button', { name: 'Cancel' })).toBeVisible()

  await job(page, 101).locator('.main').click()
  await expect(log(page)).toContainText('Waiting for output…')
  await append({ forge: 'gitlab', path: 'group/app', job: 101, text: '\x1b[33mstep one\x1b[0m\n', status: 'running' })
  await expect(log(page).locator('.lines')).toContainText('step one')
  await expect(log(page).locator('header')).toContainText('Following…')
  await append({ forge: 'gitlab', path: 'group/app', job: 101, text: 'step two\n', status: 'success' })
  await expect(log(page).locator('.lines')).toContainText('step two')
  await expect(log(page).locator('header')).not.toContainText('Following…')

  await panel(page).getByRole('button', { name: 'Run Pipeline…' }).click()
  const form = panel(page).locator('.run')
  await expect(form.locator('input').first()).toHaveValue('feature')
  await form.getByRole('button', { name: 'Variable' }).click()
  await form.getByPlaceholder('Key').fill('DEPLOY')
  await form.getByPlaceholder('Value').fill('yes')
  await form.getByRole('button', { name: 'Run', exact: true }).click()
  await wrote('POST', '/gitlab/api/v4/projects/group/app/pipeline', { ref: 'feature', variables: [{ key: 'DEPLOY', value: 'yes' }] })
  await expect(form).toHaveCount(0)

  const fresh = panel(page).locator('[data-pipeline]').first()
  await expect(fresh).toContainText('api')
  const id = await fresh.getAttribute('data-pipeline')
  await fresh.getByRole('button', { name: 'Cancel' }).click()
  await wrote('POST', `/gitlab/api/v4/projects/group/app/pipelines/${id}/cancel`)
  await fresh.getByRole('button', { name: 'Retry' }).click()
  await wrote('POST', `/gitlab/api/v4/projects/group/app/pipelines/${id}/retry`)
})

test('GitHub: a run’s jobs, a finished job’s log, retry, steps while running, dispatch a workflow, cancel and re-run it', async ({ page, fx }) => {
  await openForge(page, fx)
  const row = await pipelines(page, 'github', 200)
  await expect(panel(page).locator('.stage')).toHaveText('✓CI')
  await expect(row.getByRole('button', { name: 'Retry Failed' })).toHaveCount(0)
  await expect(job(page, 201).getByRole('button', { name: 'Cancel' })).toHaveCount(0)

  await job(page, 201).locator('.main').click()
  await seededLog(page)

  await job(page, 201).getByRole('button', { name: 'Retry' }).click()
  await wrote('POST', '/ghe/api/v3/repos/owner/app/actions/jobs/201/rerun')
  await expect(job(page, 201)).toContainText('○')

  await job(page, 201).locator('.main').click()
  await expect(log(page)).toContainText('GitHub serves the log once the job has finished.')
  await expect(log(page).locator('.step')).toHaveText('○Run')
  await expect(log(page).getByRole('button', { name: 'Open in Browser' })).toBeVisible()
  await append({ forge: 'ghe', path: 'owner/app', job: 201, text: 'rerun done\n', status: 'success' })
  await expect(log(page).locator('.lines')).toContainText('rerun done')

  await panel(page).getByRole('button', { name: 'Run Pipeline…' }).click()
  const form = panel(page).locator('.run')
  await expect(form.locator('select')).toHaveValue('1')
  await form.getByRole('button', { name: 'Input' }).click()
  await form.getByPlaceholder('Key').fill('level')
  await form.getByPlaceholder('Value').fill('2')
  await form.getByRole('button', { name: 'Run', exact: true }).click()
  await wrote('POST', '/ghe/api/v3/repos/owner/app/actions/workflows/1/dispatches', { ref: 'feature', inputs: { level: '2' } })
  await expect(form).toHaveCount(0)

  const fresh = panel(page).locator('[data-pipeline]').first()
  await expect(fresh).toContainText('workflow_dispatch')
  const id = await fresh.getAttribute('data-pipeline')
  await fresh.getByRole('button', { name: 'Cancel' }).click()
  await wrote('POST', `/ghe/api/v3/repos/owner/app/actions/runs/${id}/cancel`)
  await fresh.getByRole('button', { name: 'Retry', exact: true }).click()
  await wrote('POST', `/ghe/api/v3/repos/owner/app/actions/runs/${id}/rerun`)
})

test('without an account the tab says so and opens Settings, and the badge stays hidden', async ({ page, fx }) => {
  await openForge(page, fx)
  const accounts: { id: string; label: string }[] = await invoke('list_accounts')
  await invoke('remove_account', { id: accounts.find((a) => a.label === 'Fake GitLab')!.id })

  await repoRow(page, 'gitlab').click()
  await page.keyboard.press('ControlOrMeta+9')
  await page.locator('.bottom').getByRole('button', { name: 'Pipelines' }).click()
  await expect(panel(page)).toContainText(/No account for .+; add one in Settings/)
  await expect(page.locator('header .badge')).toHaveCount(0)
  await panel(page).getByRole('button', { name: 'Open Settings' }).click()
  await expect(page.getByRole('dialog', { name: 'Settings' })).toBeVisible()
})
