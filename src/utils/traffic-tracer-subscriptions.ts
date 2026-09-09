import type { UnlistenFn } from '@tauri-apps/api/event'

// Track each successful registration independently: Promise.all loses access
// to fulfilled registrations when any sibling rejects.
export function ownTrafficTracerSubscriptions(
  registrations: Array<Promise<UnlistenFn>>,
  onError: (error: unknown) => void,
) {
  let disposed = false
  const owned: UnlistenFn[] = []
  const release = (unlisten: UnlistenFn) => {
    try {
      unlisten()
    } catch (error) {
      onError(error)
    }
  }
  for (const registration of registrations) {
    void registration
      .then((unlisten) => {
        if (disposed) release(unlisten)
        else owned.push(unlisten)
      })
      .catch(onError)
  }
  return () => {
    if (disposed) return
    disposed = true
    owned.forEach(release)
    owned.length = 0
  }
}
