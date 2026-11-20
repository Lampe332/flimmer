import type { Key } from '../i18n'

/** Player shortcuts (player overlay and help page): keys (a text key or the literal keys) and what they do. */
export const PLAYER_KEYS: { id: string; keys: Key | string; what: Key }[] = [
  { id: 'play', keys: 'vp.keyPlayKeys', what: 'vp.keyPlay' },
  { id: 'seek', keys: '← / →', what: 'vp.keySeek' },
  { id: 'volume', keys: '↑ / ↓', what: 'vp.keyVolume' },
  { id: 'mute', keys: 'M', what: 'vp.keyMute' },
  { id: 'full', keys: 'vp.keyFullscreenKeys', what: 'vp.fullscreen' },
  { id: 'subs', keys: 'C', what: 'vp.keySubs' },
  { id: 'next', keys: 'N', what: 'player.nextEpisode' },
  { id: 'help', keys: '?', what: 'vp.keyHelp' },
]
