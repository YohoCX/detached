import type { Job, PipelineStatus, Project } from './api'

export const active = (s: PipelineStatus) => s === 'pending' || s === 'running'

const look: Record<PipelineStatus, [string, string]> = {
  success: ['✓', 'success'],
  failed: ['✕', 'error'],
  running: ['●', 'accent'],
  pending: ['○', 'warning'],
  manual: ['▶', 'muted'],
  canceled: ['⊘', 'muted'],
  skipped: ['»', 'muted'],
}
export const glyph = (s: PipelineStatus) => look[s][0]
export const color = (s: PipelineStatus) => `var(--${look[s][1]})`

export function took(secs: number | null) {
  if (secs === null) return ''
  const [h, m, s] = [Math.floor(secs / 3600), Math.floor(secs / 60) % 60, secs % 60]
  const two = (n: number) => String(n).padStart(2, '0')
  return h ? `${h}h ${two(m)}m` : m ? `${m}m ${two(s)}s` : `${s}s`
}

/** Jobs Pipelines listed, with their pipeline: JobLog's status, steps and browser link. */
export const seen = new Map<string, { job: Job; pipeline: number }>()
export const jobKey = (p: Project, id: number) => `${p.host}/${p.path}#${id}`
