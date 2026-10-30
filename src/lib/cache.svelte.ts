import { SvelteMap } from 'svelte/reactivity'

/** A view's last result; `error` is its latest refresh's failure, `at` when that ended. */
export interface Cached<T> { value?: T; error?: string; at: number }

// In memory for the session, so a view shows its last result at once while it refreshes.
const store = new SvelteMap<string, Cached<unknown>>()

export const cached = <T>(key: string) => store.get(key) as Cached<T> | undefined

/** Refreshes `key`, keeping the last value when that fails; the new value, if any. A focus refresh within 30 s of the last one is skipped. */
export async function refresh<T>(key: string, f: () => Promise<T>, focus = false): Promise<T | undefined> {
  const old = store.get(key)
  if (focus && old && Date.now() - old.at < 30_000) return
  try {
    const value = await f()
    store.set(key, { value, at: Date.now() })
    return value
  } catch (e) {
    store.set(key, { ...store.get(key), error: String(e), at: Date.now() })
  }
}
