// localStorage can throw (storage disabled or full); UI preferences are optional.
export function load(key: string): string | null {
  try {
    return localStorage.getItem(key)
  } catch {
    return null
  }
}

export function save(key: string, value: string) {
  try {
    localStorage.setItem(key, value)
  } catch {}
}
