const SOURCE_KEY = 'flimmer.lastSource'
const SANDBOX_KEY = 'flimmer.popupGuard'

function read(key: string): string | null {
  try {
    return localStorage.getItem(key)
  } catch {
    return null
  }
}

function write(key: string, value: string): void {
  try {
    localStorage.setItem(key, value)
  } catch {
    /* storage blocked: preference just isn't remembered */
  }
}

export function getLastSource(): string | null {
  return read(SOURCE_KEY)
}

export function setLastSource(id: string): void {
  write(SOURCE_KEY, id)
}

function readGuardMap(): Record<string, boolean> {
  const raw = read(SANDBOX_KEY)
  if (!raw) return {}
  try {
    const parsed: unknown = JSON.parse(raw)
    return parsed && typeof parsed === 'object' ? (parsed as Record<string, boolean>) : {}
  } catch {
    return {}
  }
}

/** Popup guard (iframe sandbox) per source id, default on. */
export function getPopupGuard(sourceId: string): boolean {
  return readGuardMap()[sourceId] ?? true
}

export function setPopupGuard(sourceId: string, on: boolean): void {
  const map = readGuardMap()
  map[sourceId] = on
  write(SANDBOX_KEY, JSON.stringify(map))
}
