import { t } from '../i18n'
import { ApiError } from './api'

/**
 * Reason codes of the server (`CODES` in server/src/error.rs). Each one has a text `code.<name>` in every
 * dictionary: the template type below fails the build when one is missing. The server's own English
 * message is for the log and never shown.
 */
const CODES = [
  'bad_request',
  'forbidden',
  'not_found',
  'conflict',
  'bad_gateway',
  'unavailable',
  'timeout',
  'internal',
  'needs_setup',
  'pc_only',
  'needs_vpn',
  'invalid_magnet',
  'invalid_infohash',
  'folder_outside',
  'folder_missing',
  'folder_local',
  'no_peers',
  'metadata_failed',
  'no_files',
  'disk_full',
  'write_failed',
  'start_failed',
  'download_failed',
  'vpn_blocked',
  'vpn_config',
  'vpn_handshake',
  'vpn_error',
  'not_on_watchlist',
  'tmdb_down',
  'already_there',
  'next_run',
  'search_failed',
  'nothing_fits',
  'dry_run',
  'scan_skipped',
  'scan_failed',
] as const
type Code = (typeof CODES)[number]

function isCode(c: unknown): c is Code {
  return typeof c === 'string' && (CODES as readonly string[]).includes(c)
}

/** The code of an error that brings no known reason, by its HTTP status (as the server does). */
function byStatus(status: number): Code {
  if (status === 400) return 'bad_request'
  if (status === 401 || status === 403) return 'forbidden'
  if (status === 404) return 'not_found'
  if (status === 409) return 'conflict'
  if (status === 502) return 'bad_gateway'
  if (status === 503) return 'unavailable'
  if (status === 504) return 'timeout'
  return 'internal'
}

/** The UI's own words for a server reason; a missing or unknown one gets a general text. */
export function codeText(code: string | null | undefined): string {
  return isCode(code) ? t(`code.${code}`) : t('code.unknown')
}

/** Text for any failed request: by its reason, else by its status. Never the server's message. */
export function errorText(e: unknown): string {
  if (!(e instanceof ApiError)) return t('code.unknown')
  if (e.status === 0) return t('err.offline')
  return t(`code.${isCode(e.code) ? e.code : byStatus(e.status)}`)
}
