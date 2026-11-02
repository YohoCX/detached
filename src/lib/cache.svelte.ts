import { SvelteMap } from 'svelte/reactivity'

/** A view's last result, kept for the session; `error` is the latest refresh's failure, `at` when it ended. */
export interface Cached<T> { value?: T; error?: string; at: number }

const store = new SvelteMap<string, Cached<unknown>>()

export const cached = <T>(key: string) => store.get(key) as Cached<T> | undefined

/** Keeps the last value when the refresh fails; a focus refresh within 30 s of the last one is skipped. */
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
