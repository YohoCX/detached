import * as monaco from 'monaco-editor'
import EditorWorker from 'monaco-editor/editor/editor.worker?worker'

// Read-only viewer: the editor worker is enough; language workers would only add diagnostics.
self.MonacoEnvironment = { getWorker: () => new EditorWorker() }

export { monaco }
