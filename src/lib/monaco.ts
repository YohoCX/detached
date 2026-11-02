import * as monaco from 'monaco-editor'

// Read-only viewer: the editor worker is enough; language workers would only add diagnostics.
self.MonacoEnvironment = { getWorker: () => new Worker('/editor.worker.js', { type: 'module' }) }

// With every feature off the language services never ask for their workers; JSON keeps its tokenizer.
const { typescript: ts, css, html, json } = monaco
for (const d of [ts.typescriptDefaults, ts.javascriptDefaults, css.cssDefaults, css.scssDefaults, css.lessDefaults,
  html.htmlDefaults, html.handlebarDefaults, html.razorDefaults])
  d.setModeConfiguration({})
json.jsonDefaults.setModeConfiguration({ tokens: true })

const diffColors = (add: string, del: string) => ({
  'diffEditor.insertedLineBackground': `${add}1f`,
  'diffEditor.insertedTextBackground': `${add}38`,
  'diffEditor.removedLineBackground': `${del}22`,
  'diffEditor.removedTextBackground': `${del}40`,
  'diffEditorGutter.insertedLineBackground': `${add}2a`,
  'diffEditorGutter.removedLineBackground': `${del}2e`,
})
monaco.editor.defineTheme('wb-dark', {
  base: 'vs-dark',
  inherit: true,
  rules: [],
  colors: {
    'editor.background': '#0e1220',
    'editorGutter.background': '#0e1220',
    'editor.lineHighlightBackground': '#ffffff08',
    'editor.lineHighlightBorder': '#00000000',
    'editorLineNumber.foreground': '#5b6685',
    'editorLineNumber.activeForeground': '#a5b0c8',
    'editor.selectionBackground': '#818cf84d',
    'diffEditor.diagonalFill': '#ffffff0d',
    'scrollbarSlider.background': '#ffffff14',
    'scrollbarSlider.hoverBackground': '#ffffff24',
    ...diffColors('#34d399', '#fb7185'),
  },
})
monaco.editor.defineTheme('wb-light', {
  base: 'vs',
  inherit: true,
  rules: [],
  colors: {
    'editor.background': '#ffffff',
    'editorGutter.background': '#ffffff',
    'editor.lineHighlightBackground': '#1e29500a',
    'editor.lineHighlightBorder': '#00000000',
    'editorLineNumber.foreground': '#8a93a8',
    'editorLineNumber.activeForeground': '#3b4458',
    'editor.selectionBackground': '#6366f133',
    'diffEditor.diagonalFill': '#1e29500d',
    'scrollbarSlider.background': '#1e295014',
    'scrollbarSlider.hoverBackground': '#1e295024',
    ...diffColors('#10b981', '#f43f5e'),
  },
})

export { monaco }
