import { useQuery } from '@tanstack/react-query'
import { request } from './api'

/**
 * Who this page is to the server: the desktop window ("pc"), a paired phone, tablet or browser ("device"),
 * or nobody yet ("none": the pairing page shows).
 */
export type Role = 'pc' | 'device' | 'none'

export interface Session {
  role: Role
  /** "lan" = the page was opened over the home network. */
  via: 'local' | 'lan'
  /** Name of this device in the PC's list. */
  device: string | null
}

export interface LanDevice {
  id: string
  name: string
  created_at: string
  last_seen: string
  last_ip: string
}

export interface LanPairing {
  pin: string
  /** The address in the QR code; null while the network address is not open yet. */
  link: string | null
  /** The QR code as SVG markup. */
  qr: string | null
  expires_in: number
}

export interface LanState {
  enabled: boolean
  /** The address picked by hand, null = automatic. */
  ip: string | null
  active_ip: string | null
  url: string | null
  port: number
  candidates: { ip: string; name: string }[]
  error: string | null
  pairing: LanPairing | null
  devices: LanDevice[]
}

export type PairError = 'wrong_pin' | 'locked' | 'closed' | 'full' | 'offline' | 'other'

export class PairFailed extends Error {
  readonly code: PairError
  /** Seconds to wait when `code` is "locked". */
  readonly wait: number
  constructor(code: PairError, wait = 0) {
    super(code)
    this.code = code
    this.wait = wait
  }
}

const json = (method: string, body: unknown): RequestInit => ({
  method,
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify(body),
})

async function pair(body: { pin: string } | { token: string }): Promise<void> {
  let res: Response
  try {
    res = await fetch('/api/pair', json('POST', body))
  } catch {
    throw new PairFailed('offline')
  }
  if (res.ok) return
  let code: string | undefined
  let wait = 0
  try {
    const data = (await res.json()) as { code?: string; retry_after?: number }
    code = data.code
    wait = data.retry_after ?? 0
  } catch {
    /* no JSON body */
  }
  const known: PairError[] = ['wrong_pin', 'locked', 'closed', 'full']
  throw new PairFailed(known.includes(code as PairError) ? (code as PairError) : 'other', wait)
}

export const sessionApi = {
  get: () => request<Session>('/session'),
  pairPin: (pin: string) => pair({ pin }),
  pairToken: (token: string) => pair({ token }),
  logout: () => request<void>('/pair/logout', { method: 'POST' }),
}

export const lanApi = {
  get: () => request<LanState>('/lan'),
  put: (enabled: boolean, ip: string | null) => request<void>('/lan', json('PUT', { enabled, ip })),
  pairingStart: () => request<LanState>('/lan/pairing', { method: 'POST' }),
  pairingStop: () => request<void>('/lan/pairing', { method: 'DELETE' }),
  deviceRemove: (id: string) => request<void>(`/lan/devices/${encodeURIComponent(id)}`, { method: 'DELETE' }),
}

export function useSession() {
  return useQuery({ queryKey: ['session'], queryFn: sessionApi.get, staleTime: Infinity, retry: 2 })
}

/** "pc" until the answer is there, so nothing flickers in the desktop window. */
export function useRole(): Role {
  return useSession().data?.role ?? 'pc'
}

/**
 * The app shell works offline for an installed phone app. Only where the browser allows a service worker
 * (https or localhost) and never in the desktop window, which always has the PC next to it.
 */
export function registerShellWorker(): void {
  if (!('serviceWorker' in navigator) || !window.isSecureContext) return
  navigator.serviceWorker.register('/sw.js').catch(() => {
    /* the app works without it */
  })
}
