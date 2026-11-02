import type { Project } from './api'

/** What the main area shows instead of a file: an MR page or a job log. */
export type Doc = { kind: 'mr'; number: number } | { kind: 'job'; project: Project; id: number; name: string }
/** `ref` null is the working tree. */
export interface FileRef { repo: string; ref: string | null; path: string }
/** With `doc` set, `ref`, `path` and `line` are unused. */
export interface Location extends FileRef { line: number | null; doc?: Doc }

const same = (a: FileRef, b: FileRef) => a.repo === b.repo && a.ref === b.ref && a.path === b.path
const sameDoc = (a: Location, b: Location) => JSON.stringify(a.doc) === JSON.stringify(b.doc)

export class Nav {
  recent: FileRef[] = []
  history: Location[] = []
  index = -1

  /** Recent files (50, deduped) and history (100, forward entries dropped). */
  visit(loc: Location) {
    const { repo, ref, path } = loc
    if (!loc.doc) this.recent = [{ repo, ref, path }, ...this.recent.filter((r) => !same(r, loc))].slice(0, 50)
    const cur = this.history[this.index]
    if (cur && same(cur, loc) && cur.line === loc.line && sameDoc(cur, loc)) return
    this.history = [...this.history.slice(0, this.index + 1), loc].slice(-100)
    this.index = this.history.length - 1
  }

  back(): Location | null {
    return this.index > 0 ? this.history[--this.index] : null
  }

  forward(): Location | null {
    return this.index < this.history.length - 1 ? this.history[++this.index] : null
  }
}

export const nav = new Nav()

let opener: (loc: Location) => void = () => {}

/** The opener shows `loc` without recording it again. */
export const setOpener = (f: (loc: Location) => void) => (opener = f)

export function open(loc: Location) {
  nav.visit(loc)
  opener(loc)
}

export function back() {
  const loc = nav.back()
  if (loc) opener(loc)
}

export function forward() {
  const loc = nav.forward()
  if (loc) opener(loc)
}
