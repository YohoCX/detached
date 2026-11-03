const ua = navigator.userAgent

export const mac = ua.includes('Mac')

export const fileManager = mac ? 'Finder' : ua.includes('Windows') ? 'Explorer' : 'File Manager'

/** The primary modifier: Cmd on macOS, Ctrl elsewhere. */
export const primary = (e: MouseEvent | KeyboardEvent) => (mac ? e.metaKey : e.ctrlKey)
