// Page init script: Tauri's IPC goes to the e2e bridge, which runs the real command code.

import { mockIPC } from '@tauri-apps/api/mocks'

declare global {
  interface Window {
    __E2E__: { bridge: string; pick: string[] }
    __TAURI_INTERNALS__: { runCallback(id: number, data: unknown): void }
  }
}

type Channel = { id: number; toJSON(): string }
const isChannel = (v: unknown): v is Channel =>
  typeof (v as Channel)?.toJSON === 'function' && String((v as Channel).toJSON()).startsWith('__CHANNEL__:')

mockIPC(async (cmd, args) => {
  if (cmd === 'plugin:dialog|open') return window.__E2E__.pick
  if (cmd.startsWith('plugin:menu|')) return cmd === 'plugin:menu|new' ? [0, ''] : null
  const channel = Object.values(args ?? {}).find(isChannel)
  // Channel messages carry an index, like Tauri's, so the Channel class keeps them in order.
  let index = 0
  const deliver = (data: object) => channel && window.__TAURI_INTERNALS__.runCallback(channel.id, { index: index++, ...data })
  const res = await fetch(`${window.__E2E__.bridge}/invoke`, { method: 'POST', body: JSON.stringify({ cmd, args }) })
  const reader = res.body!.pipeThrough(new TextDecoderStream()).getReader()
  let buf = ''
  for (;;) {
    const { value, done } = await reader.read()
    if (done) throw `bridge closed during ${cmd}`
    buf += value
    for (let i; (i = buf.indexOf('\n')) >= 0; buf = buf.slice(i + 1)) {
      const msg = JSON.parse(buf.slice(0, i))
      if ('event' in msg) {
        deliver({ message: msg.event })
        continue
      }
      deliver({ end: true })
      if ('err' in msg) throw msg.err
      return msg.ok
    }
  }
})
