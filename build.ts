import { SveltePlugin } from 'bun-plugin-svelte'
import { rmSync, watch } from 'node:fs'

// `bun build.ts` writes dist for Tauri. `bun build.ts serve PORT [BRIDGE_PORT]` rebuilds on change, serves dist
// and reloads the page; with a bridge port it serves under the bundle's CSP, like e2e needs.
const [mode, port, bridge] = process.argv.slice(2)
const dev = mode === 'serve'

async function build(): Promise<boolean> {
  rmSync('dist', { recursive: true, force: true })
  const r = await Bun.build({
    // Monaco's editor worker is its own entry: Bun does not split out `new Worker(new URL(...))`.
    entrypoints: ['index.html', Bun.resolveSync('monaco-editor/editor/editor.worker', '.')],
    outdir: 'dist',
    target: 'browser',
    splitting: true,
    minify: !dev,
    sourcemap: dev ? 'linked' : 'none',
    naming: { entry: '[name].[ext]', chunk: 'assets/[name]-[hash].[ext]', asset: 'assets/[name]-[hash].[ext]' },
    plugins: [SveltePlugin({ development: dev })],
  })
  for (const log of r.logs) console.error(log)
  return r.success
}

if (!(await build())) process.exit(1)

if (dev) {
  const clients = new Set<ReadableStreamDefaultController>()
  let pending: Promise<unknown> = Promise.resolve()
  watch('src', { recursive: true }, () => {
    pending = pending.then(build).then((ok) => ok && clients.forEach((c) => c.enqueue('data: \n\n')))
  })

  const csp = bridge
    ? (await Bun.file('src-tauri/tauri.conf.json').json()).app.security.csp
      .replace('connect-src', `connect-src 'self' http://127.0.0.1:${bridge}`)
    : null
  const headers = csp ? { 'Content-Security-Policy': csp } : undefined

  Bun.serve({
    port: Number(port),
    idleTimeout: 0,
    async fetch(req) {
      const path = new URL(req.url).pathname
      if (path === '/reload') {
        let ctl: ReadableStreamDefaultController
        const body = new ReadableStream({
          start: (c) => void clients.add((ctl = c)),
          cancel: () => void clients.delete(ctl),
        })
        return new Response(body, { headers: { 'Content-Type': 'text/event-stream' } })
      }
      if (path === '/reload.js')
        return new Response(`new EventSource('/reload').onmessage = () => location.reload()`, {
          headers: { 'Content-Type': 'text/javascript' },
        })
      await pending
      if (path === '/') {
        const html = (await Bun.file('dist/index.html').text()).replace('</head>', '<script src="/reload.js"></script></head>')
        return new Response(html, { headers: { 'Content-Type': 'text/html', ...headers } })
      }
      const file = Bun.file(`dist${path}`)
      return (await file.exists()) ? new Response(file, { headers }) : new Response('Not found', { status: 404 })
    },
  })
  console.log(`http://localhost:${port}`)
}
