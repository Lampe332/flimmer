import { useQuery } from '@tanstack/react-query'
import { request } from './api'

export interface SetupState {
  tmdb: 'set' | 'missing'
  onboarded: boolean
  vpn_file: boolean
  version: string
  defaults: { movies_dir: string; series_dir: string }
}

export interface Maintenance {
  stream_cache_bytes: number
  scan_profile_bytes: number
  scan_profile_pending: boolean
  data_dir: string
}

/** Fixed help and license pages the server opens in the system browser. */
export type Page =
  | 'tmdb_signup'
  | 'tmdb_api'
  | 'proton_wireguard'
  | 'proton_help'
  | 'tmdb'
  | 'ffmpeg'
  | 'ffmpeg_source'
  | 'ubol'
  | 'librqbit'
  | 'boringtun'
  | 'hlsjs'
  | 'tork'

/** Why a token was not taken: the server's `code`. */
export type TokenError = 'format' | 'api_key' | 'rejected' | 'unreachable' | 'other'

const json = (method: string, body: unknown): RequestInit => ({
  method,
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify(body),
})

/** Like `request`, but keeps the server's machine-readable `code`. */
async function coded(path: string, init: RequestInit): Promise<void> {
  let res: Response
  try {
    res = await fetch(`/api${path}`, init)
  } catch {
    throw new SetupError('unreachable')
  }
  if (res.ok) return
  let code: string | undefined
  try {
    code = ((await res.json()) as { code?: string }).code
  } catch {
    /* no JSON body */
  }
  const known: TokenError[] = ['format', 'api_key', 'rejected', 'unreachable']
  throw new SetupError(known.includes(code as TokenError) ? (code as TokenError) : 'other')
}

export class SetupError extends Error {
  readonly code: TokenError
  constructor(code: TokenError) {
    super(code)
    this.code = code
  }
}

export const setupApi = {
  state: () => request<SetupState>('/setup'),
  token: (token: string) => coded('/setup/tmdb', json('PUT', { token })),
  done: () => request<void>('/setup/done', { method: 'POST' }),
  vpnImport: (text: string) => request<void>('/setup/vpn', json('PUT', { text })),
  vpnRemove: () => request<void>('/setup/vpn', { method: 'DELETE' }),
  open: (page: Page) => request<void>('/setup/open', json('POST', { page })),
  reset: () => request<void>('/setup/reset', json('POST', { confirm: 'reset' })),
  exportFile: () => request<{ path: string }>('/export', { method: 'POST' }),
  importFile: (text: string) =>
    request<{ watchlist: number }>('/import', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: text }),
  maintenance: () => request<Maintenance>('/maintenance'),
  clearCache: () => request<{ stream_cache_bytes: number; scan_profile_pending: boolean }>('/maintenance/cache', { method: 'DELETE' }),
}

export function useSetup() {
  return useQuery({ queryKey: ['setup'], queryFn: setupApi.state, staleTime: Infinity, retry: 2 })
}

/** Reads a small text file the user picked (WireGuard .conf, export .json). */
export function readTextFile(file: File, max: number): Promise<string> {
  if (file.size > max) return Promise.reject(new Error('too large'))
  return file.text()
}
