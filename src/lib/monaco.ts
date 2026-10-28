import * as monaco from 'monaco-editor'
import EditorWorker from 'monaco-editor/editor/editor.worker?worker'

// Read-only viewer: the editor worker is enough; language workers would only add diagnostics.
self.MonacoEnvironment = { getWorker: () => new EditorWorker() }

// With every feature off the language services never ask for their workers; JSON keeps its tokenizer.
const { typescript: ts, css, html, json } = monaco
for (const d of [ts.typescriptDefaults, ts.javascriptDefaults, css.cssDefaults, css.scssDefaults, css.lessDefaults,
  html.htmlDefaults, html.handlebarDefaults, html.razorDefaults])
  d.setModeConfiguration({})
json.jsonDefaults.setModeConfiguration({ tokens: true })

export { monaco }
