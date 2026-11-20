const VOLUME_KEY = 'flimmer.volume'

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

export type VolumePref = { volume: number; muted: boolean }

export function getVolume(): VolumePref {
  const raw = read(VOLUME_KEY)
  if (raw) {
    try {
      const v = JSON.parse(raw) as Partial<VolumePref>
      if (typeof v.volume === 'number' && v.volume >= 0 && v.volume <= 1) {
        return { volume: v.volume, muted: v.muted === true }
      }
    } catch {
      /* fall through to the default */
    }
  }
  return { volume: 1, muted: false }
}

export function setVolume(pref: VolumePref): void {
  write(VOLUME_KEY, JSON.stringify(pref))
}
