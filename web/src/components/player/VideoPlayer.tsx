import Hls from 'hls.js'
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent as ReactKeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
} from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { api, streamUrl, type Variant } from '../../lib/api'
import { getLang, t as i18nT, useI18n, type Key, type T } from '../../i18n'
import { formatBitrate, formatClock, formatHeight, langCode, trackLabel } from '../../lib/format'
import { variantIndex } from '../../lib/pick'
import { playUrl, subtitleUrl, torrentApi, type AudioInfo, type SubtitleInfo } from '../../lib/torrents'
import { getVolume, setVolume, type VolumePref } from '../../lib/storage'
import { PLAYER_KEYS } from '../../lib/keys'
import { getPrefs, matchesLang, usePrefs } from '../../lib/prefs'
import Icon, { type IconName } from '../Icon'
import { Flicker } from '../States'

type Props = {
  streamId: string
  /** "mp4" = single file, played natively without hls.js and without prefetch. */
  kind?: 'hls' | 'mp4'
  /** Variants as the server lists them (best first); needed to address the prefetch. */
  variants: Variant[]
  title: string
  subtitle?: string
  /** Second to start from, e.g. after switching the source mid-film. */
  startAt: number
  onTime: (seconds: number) => void
  /** The upstream stopped serving this stream (HTTP 410). */
  onExpired: () => void
  next?: { label: string; name?: string; still?: string | null; go: () => void }
  /** Called with the position to remember: every 10 s while playing, on pause, seek, end and when leaving. */
  onSave?: (seconds: number, duration: number) => void
  /** Saved position this playback resumed from; shows "Fortgesetzt bei …" with a restart button. */
  resumeHint?: number
  onHintDone?: () => void
  /** One-click way out when this source fails, e.g. "Andere Quelle". */
  fallback?: { label: string; run: () => void }
  /** Plays a torrent file instead of a provider stream (`streamId` is then only the component's identity). */
  torrent?: TorrentSource
}

/**
 * A torrent file as /playinfo describes it. "direct": the file itself, seekable natively. "transcode": ffmpeg fMP4,
 * not seekable; the player reloads `play?t=` on a seek and adds that offset to the element's clock.
 */
type TorrentSource = {
  id: string
  file: number
  mode: 'direct' | 'transcode'
  duration: number | null
  copyVideo: boolean
  audio: AudioInfo[]
  /** Track to start with (saved audio language, else the file's default). */
  defaultAudio: number
  /** Text tracks are served as WebVTT (`/subtitle`); image subtitles are left out. */
  subtitles: SubtitleInfo[]
  /** Calm state line for the bar, e.g. "Lädt von 34 Quellen · 12 MB/s". */
  status?: string
}

type Option = { id: number; label: string; hint?: string }
/** A playlist track; its label is built at render time so it follows the UI language. */
type TrackRef = { id: number; lang: string | null; name: string | null }
type MenuId = 'quality' | 'audio' | 'subs'

/** iOS Safari ignores video.volume (only muting works); everything Chromium-based honours it. */
const CAN_SET_VOLUME = (() => {
  try {
    const probe = document.createElement('video')
    probe.volume = 0.5
    return probe.volume === 0.5
  } catch {
    return true
  }
})()
const NARROW = '(max-width: 760px)'
/** Only a real hover keeps the bar up; on touch, :hover latches after a tap. */
const HOVER_MQ = window.matchMedia('(hover: hover)')

/** True while the viewport is phone-sized: the speaker then opens the slider instead of muting. */
function useNarrow(): boolean {
  const [narrow, setNarrow] = useState(() => window.matchMedia(NARROW).matches)
  useEffect(() => {
    const mq = window.matchMedia(NARROW)
    const on = () => setNarrow(mq.matches)
    mq.addEventListener('change', on)
    return () => mq.removeEventListener('change', on)
  }, [])
  return narrow
}

/** Slim slider: a 3px line in the bar, with a hit area as tall as the buttons. */
function VolumeRange({ value, onChange, t }: { value: number; onChange: (v: number) => void; t: T }) {
  return (
    <input
      type="range"
      className="vp-range"
      min={0}
      max={1}
      step={0.05}
      value={value}
      onChange={(e) => onChange(Number(e.target.value))}
      aria-label={t('vp.volume')}
      aria-valuetext={t('vp.percent', { n: Math.round(value * 100) })}
      style={{ '--fill': `${value * 100}%` } as CSSProperties}
    />
  )
}

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v))
const IDLE_MS = 2800
const SAVE_EVERY_MS = 10_000
/** Credits shortcut: offer the next episode this many seconds before the end. */
const UP_NEXT_S = 30
const HINT_MS = 8000
/** A second click within this time is a double click (fullscreen), so the first one must not toggle playback. */
const DBL_MS = 240

/** Shortcut overview (shared with the help page). */
const KEYS = PLAYER_KEYS
type Fatal = 'vp.streamLoad' | 'api.offline' | 'vp.noData' | 'vp.cantPlay' | 'vp.browserCant'

export default function VideoPlayer({
  streamId,
  kind,
  variants,
  title,
  subtitle,
  startAt,
  onTime,
  onExpired,
  next,
  onSave,
  resumeHint,
  onHintDone,
  fallback,
  torrent,
}: Props) {
  const wrap = useRef<HTMLDivElement>(null)
  const video = useRef<HTMLVideoElement>(null)
  const bar = useRef<HTMLDivElement>(null)
  const hlsRef = useRef<Hls | null>(null)
  const pokeRef = useRef<() => void>(() => undefined)
  const qc = useQueryClient()

  const [playing, setPlaying] = useState(false)
  const [waiting, setWaiting] = useState(true)
  const [started, setStarted] = useState(false)
  const [ended, setEnded] = useState(false)
  const [fatal, setFatal] = useState<Fatal | null>(null)
  const [time, setTime] = useState(startAt)
  const [duration, setDuration] = useState(0)
  const [buffered, setBuffered] = useState<[number, number][]>([])
  const [vol, setVol] = useState<VolumePref>(getVolume)
  const [fullscreen, setFullscreen] = useState(false)
  const [levels, setLevels] = useState<Option[]>([])
  const [level, setLevel] = useState(-1)
  const [activeLevel, setActiveLevel] = useState(-1)
  const [audios, setAudios] = useState<TrackRef[]>([])
  const [audio, setAudio] = useState(-1)
  const [subs, setSubs] = useState<TrackRef[]>([])
  const [sub, setSub] = useState(-1)
  const [menu, setMenu] = useState<MenuId | null>(null)
  /** Phones: the slider is folded into the speaker until a tap opens it. */
  const [volOpen, setVolOpen] = useState(false)
  const volBox = useRef<HTMLDivElement>(null)
  const { t } = useI18n()
  const [idle, setIdle] = useState(false)
  const [hover, setHover] = useState<number | null>(null)
  const [scrub, setScrub] = useState<number | null>(null)
  const [prefetching, setPrefetching] = useState(false)
  const [help, setHelp] = useState(false)
  const narrow = useNarrow()

  // Latest callbacks/props for the long-lived listeners below.
  const live = useRef({ onTime, onExpired, onSave, variants, startAt })
  live.current = { onTime, onExpired, onSave, variants, startAt: live.current.startAt }

  // ---- torrent playback: absolute time = start of the current ffmpeg stream + the element's own clock ----
  const tor = useRef(torrent)
  tor.current = torrent
  const offset = useRef(0)
  const tState = useRef<{ mode: 'direct' | 'transcode'; audio: number | null; h264: boolean }>({
    mode: torrent?.mode ?? 'direct',
    audio: null,
    h264: false,
  })
  const pendingSeek = useRef<number | null>(null)
  const seekTimer = useRef<number | undefined>(undefined)
  /** Position in the whole video, also while a transcoded seek is still loading. */
  const nowAt = useCallback(() => pendingSeek.current ?? offset.current + (video.current?.currentTime ?? 0), [])
  const isTranscode = () => !!tor.current && tState.current.mode === 'transcode'
  /** File time of the current ffmpeg stream's first frame (subtitle cues are moved by it). */
  const [tStart, setTStart] = useState(0)
  const loadSeq = useRef(0)
  /**
   * (Re)loads the torrent stream at `at` seconds; keeps playing if it was playing. A transcoded stream asks
   * `/seek` first: a copied video starts at the keyframe before `at`, and that real start becomes the offset.
   */
  const loadTorrent = useCallback((at: number, autoplay?: boolean) => {
    const el = video.current
    const src = tor.current
    if (!el || !src) return
    const s = tState.current
    const direct = s.mode === 'direct'
    const play = autoplay ?? (!el.paused && !el.ended)
    const seq = ++loadSeq.current
    const open = (t: number, start: number) => {
      if (seq !== loadSeq.current) return
      offset.current = direct ? 0 : Math.max(0, start)
      setTStart(offset.current)
      el.src = playUrl(src.id, src.file, direct ? {} : { t, audio: s.audio, h264: s.h264 })
      if (direct && at > 0) el.addEventListener('loadedmetadata', () => (el.currentTime = at), { once: true })
      if (play) void el.play().catch(() => undefined)
    }
    setTime(at)
    if (direct || at <= 0) return open(at, at)
    pendingSeek.current = at
    const done = () => {
      if (seq === loadSeq.current) pendingSeek.current = null
    }
    torrentApi
      .seek(src.id, src.file, at, s.h264)
      .then((r) => (done(), open(r.t, r.start)))
      .catch(() => (done(), open(at, at)))
  }, [])

  // ---- progress saving: only once playback really ran, from the last known position ----
  const pos = useRef({ time: startAt, duration: 0, played: false })
  const save = useCallback(() => {
    const p = pos.current
    if (p.played && p.duration > 0) live.current.onSave?.(p.time, p.duration)
  }, [])
  useEffect(() => {
    const onHide = () => {
      if (document.visibilityState === 'hidden') save()
    }
    window.addEventListener('pagehide', save)
    // Sent by the desktop app right before it closes the window (WebView2 does not fire pagehide then).
    window.addEventListener('flimmer:beforeclose', save)
    document.addEventListener('visibilitychange', onHide)
    return () => {
      window.removeEventListener('pagehide', save)
      window.removeEventListener('flimmer:beforeclose', save)
      document.removeEventListener('visibilitychange', onHide)
      save()
    }
  }, [save])
  useEffect(() => {
    if (!playing) return
    const t = window.setInterval(save, SAVE_EVERY_MS)
    return () => window.clearInterval(t)
  }, [playing, save])

  // ---- prefetch: ask the server to load ahead from the current position (debounced) ----
  const prefetchTimer = useRef<number | undefined>(undefined)
  const requestPrefetch = useCallback(() => {
    window.clearTimeout(prefetchTimer.current)
    prefetchTimer.current = window.setTimeout(() => {
      const el = video.current
      if (!el || kind === 'mp4') return
      const hls = hlsRef.current
      const lv = hls ? hls.levels[hls.currentLevel >= 0 ? hls.currentLevel : hls.loadLevel] : undefined
      const index = lv ? variantIndex(live.current.variants, lv.height, lv.bitrate) : 0
      api
        .prefetch(streamId, index, el.currentTime)
        .then(() => {
          setPrefetching(true)
          void qc.invalidateQueries({ queryKey: ['buffer', streamId] })
        })
        .catch(() => setPrefetching(false))
    }, 600)
  }, [streamId, kind, qc])
  const prefetchRef = useRef(requestPrefetch)
  prefetchRef.current = requestPrefetch

  const buffer = useQuery({
    queryKey: ['buffer', streamId],
    queryFn: () => api.buffer(streamId),
    enabled: prefetching,
    refetchInterval: (q) => (q.state.data?.done ? false : 2000),
    staleTime: 0,
    gcTime: 0,
    retry: false,
  })

  // ---- media pipeline ----
  useEffect(() => {
    const el = video.current
    if (!el) return
    const from = live.current.startAt
    const src = tor.current
    if (src) {
      tState.current = { mode: src.mode, audio: null, h264: false }
      // A direct file plays its first track; another saved language needs ffmpeg from the start.
      const first = src.audio[0]?.index ?? -1
      if (src.mode === 'direct' && src.audio.length > 1 && src.defaultAudio !== first) {
        tState.current = { mode: 'transcode', audio: src.defaultAudio, h264: false }
      }
      setAudios(src.audio.map((a) => ({ id: a.index, lang: a.lang, name: a.title })))
      setAudio(src.audio.some((a) => a.index === src.defaultAudio) ? src.defaultAudio : first)
      const texts = src.subtitles.filter((x) => x.text)
      setSubs(texts.map((x) => ({ id: x.index, lang: x.lang, name: x.title })))
      const want = getPrefs().subLang
      const forced = texts.find((x) => x.forced && matchesLang(x.lang, want === 'off' ? 'ui' : want))
      const pick = want !== 'off' ? texts.find((x) => matchesLang(x.lang, want)) : forced
      setSub(pick?.index ?? -1)
      const onTorrentError = () => {
        const s = tState.current
        const code = el.error?.code
        // Video the browser cannot decode (e.g. HEVC without the system extension): let ffmpeg make H.264 of it.
        if (!s.h264 && src.copyVideo && (code === MediaError.MEDIA_ERR_DECODE || code === MediaError.MEDIA_ERR_SRC_NOT_SUPPORTED)) {
          s.h264 = true
          s.mode = 'transcode'
          loadTorrent(nowAt(), true)
          return
        }
        setFatal('vp.cantPlay')
      }
      el.addEventListener('error', onTorrentError)
      loadTorrent(from, true)
      return () => {
        el.removeEventListener('error', onTorrentError)
        window.clearTimeout(seekTimer.current)
        el.removeAttribute('src')
        el.load()
      }
    }
    const url = streamUrl(streamId)
    let hls: Hls | null = null
    let mediaRecoveries = 0
    let networkRetries = 0
    let disposed = false

    const playNative = () => {
      el.src = url
      if (from > 0) el.addEventListener('loadedmetadata', () => (el.currentTime = from), { once: true })
      void el.play().catch(() => undefined)
    }
    const onNativeError = () => {
      if (hlsRef.current) return
      // A plain <video> cannot tell why loading failed, so ask the server once.
      fetch(url, { method: 'GET', headers: { Range: 'bytes=0-0' } })
        .then((res) => {
          if (disposed) return
          if (res.status === 410) live.current.onExpired()
          else setFatal('vp.streamLoad')
        })
        .catch(() => !disposed && setFatal('api.offline'))
    }

    if (kind === 'mp4') {
      playNative()
    } else if (Hls.isSupported()) {
      const h = new Hls({ startPosition: from > 0 ? from : -1, maxBufferLength: 30, backBufferLength: 60 })
      hls = h
      hlsRef.current = h
      h.on(Hls.Events.MANIFEST_PARSED, () => {
        setLevels(
          h.levels
            .map((l, id) => ({ id, height: l.height, bitrate: l.bitrate }))
            .sort((a, z) => z.height - a.height || z.bitrate - a.bitrate)
            .map((l) => ({
              id: l.id,
              label: formatHeight(l.height) ?? formatBitrate(l.bitrate) ?? i18nT('vp.variant', { n: l.id + 1 }),
              hint: l.height ? (formatBitrate(l.bitrate) ?? undefined) : undefined,
            })),
        )
        void el.play().catch(() => undefined)
      })
      h.on(Hls.Events.LEVEL_SWITCHED, (_e, data) => {
        setActiveLevel(data.level)
        if (el.currentTime > 0 && !el.paused) prefetchRef.current()
      })
      const syncAudio = () => {
        setAudios(h.audioTracks.map((x, id) => ({ id, lang: x.lang ?? null, name: x.name ?? null })))
        setAudio(h.audioTrack)
      }
      const syncSubs = () => {
        setSubs(h.subtitleTracks.map((x, id) => ({ id, lang: x.lang ?? null, name: x.name ?? null })))
        setSub(h.subtitleTrack)
      }
      // State mirrors hls.js itself (no optimistic update), so the menu shows what really plays.
      // Preferred languages from the settings, once per stream; the user's own pick afterwards wins.
      let audioChosen = false
      let subsChosen = false
      h.on(Hls.Events.AUDIO_TRACKS_UPDATED, () => {
        const want = getPrefs().audioLang
        if (!audioChosen && want !== 'auto') {
          const i = h.audioTracks.findIndex((x) => matchesLang(x.lang ?? null, want))
          if (i >= 0 && i !== h.audioTrack) h.audioTrack = i
        }
        audioChosen = true
        syncAudio()
      })
      h.on(Hls.Events.SUBTITLE_TRACKS_UPDATED, () => {
        const want = getPrefs().subLang
        if (!subsChosen && want !== 'off' && h.subtitleTracks.length > 0) {
          const i = h.subtitleTracks.findIndex((x) => matchesLang(x.lang ?? null, want))
          if (i >= 0) {
            h.subtitleDisplay = true
            h.subtitleTrack = i
          }
          subsChosen = true
        }
        syncSubs()
      })
      h.on(Hls.Events.AUDIO_TRACK_SWITCHING, syncAudio)
      h.on(Hls.Events.AUDIO_TRACK_SWITCHED, syncAudio)
      h.on(Hls.Events.SUBTITLE_TRACK_SWITCH, syncSubs)
      h.on(Hls.Events.ERROR, (_e, data) => {
        if (data.response?.code === 410) {
          live.current.onExpired()
          return
        }
        if (!data.fatal) return
        if (data.details === Hls.ErrorDetails.MANIFEST_PARSING_ERROR) {
          // Not a playlist: the server points plain files at the same address.
          h.destroy()
          hlsRef.current = null
          hls = null
          playNative()
        } else if (data.type === Hls.ErrorTypes.MEDIA_ERROR && mediaRecoveries < 2) {
          mediaRecoveries += 1
          h.recoverMediaError()
        } else if (data.type === Hls.ErrorTypes.NETWORK_ERROR && networkRetries < 2) {
          networkRetries += 1
          h.startLoad()
        } else {
          setFatal(data.type === Hls.ErrorTypes.NETWORK_ERROR ? 'vp.noData' : 'vp.cantPlay')
        }
      })
      h.loadSource(url)
      h.attachMedia(el)
    } else if (el.canPlayType('application/vnd.apple.mpegurl')) {
      playNative()
    } else {
      setFatal('vp.browserCant')
    }
    el.addEventListener('error', onNativeError)

    return () => {
      disposed = true
      el.removeEventListener('error', onNativeError)
      hls?.destroy()
      hlsRef.current = null
      el.removeAttribute('src')
      el.load()
      window.clearTimeout(prefetchTimer.current)
      void api.streamRelease(streamId).catch(() => undefined)
    }
  }, [streamId])

  // ---- <video> state mirror ----
  useEffect(() => {
    const el = video.current
    if (!el) return
    const ranges = () => {
      const out: [number, number][] = []
      const o = offset.current
      for (let i = 0; i < el.buffered.length; i += 1) out.push([o + el.buffered.start(i), o + el.buffered.end(i)])
      setBuffered(out)
    }
    const on: [keyof HTMLMediaElementEventMap, () => void][] = [
      // Resuming shows the controls for the usual moment instead of hiding them at once.
      ['play', () => (setPlaying(true), setEnded(false), pokeRef.current())],
      ['pause', () => (setPlaying(false), save())],
      ['waiting', () => setWaiting(true)],
      ['canplay', () => setWaiting(false)],
      [
        'playing',
        () => {
          pos.current.played = true
          setWaiting(false)
          setStarted((was) => {
            if (!was) prefetchRef.current()
            return true
          })
        },
      ],
      ['seeked', () => (prefetchRef.current(), (pos.current.time = offset.current + el.currentTime), save())],
      [
        'timeupdate',
        () => {
          if (pendingSeek.current !== null) return
          const now = offset.current + el.currentTime
          setTime(now)
          live.current.onTime(now)
          if (pos.current.played) pos.current.time = now
        },
      ],
      [
        'durationchange',
        () => {
          // A transcoded torrent is a live-like fMP4 without a length: the timeline comes from ffprobe.
          const known = tor.current?.duration ?? 0
          const d = tor.current && tState.current.mode === 'transcode' ? known : Number.isFinite(el.duration) ? el.duration : known
          setDuration(d)
          pos.current.duration = d
        },
      ],
      ['progress', ranges],
      ['ended', () => (setEnded(true), setPlaying(false), (pos.current.time = pos.current.duration), save())],
    ]
    on.forEach(([name, fn]) => el.addEventListener(name, fn))
    return () => on.forEach(([name, fn]) => el.removeEventListener(name, fn))
  }, [save])

  useEffect(() => {
    const el = video.current
    if (!el) return
    el.volume = vol.volume
    el.muted = vol.muted
    setVolume(vol)
  }, [vol])

  // ---- controls visibility ----
  // Shows the controls and hides them IDLE_MS later; while a mouse rests on the control bar they stay.
  const idleTimer = useRef<number | undefined>(undefined)
  const poke = useCallback(() => {
    setIdle(false)
    window.clearTimeout(idleTimer.current)
    const arm = () => {
      idleTimer.current = window.setTimeout(() => {
        const overBar = HOVER_MQ.matches && !!wrap.current?.querySelector('.vp-controls:hover')
        if (overBar) arm()
        else setIdle(true)
      }, IDLE_MS)
    }
    arm()
  }, [])
  pokeRef.current = poke
  useEffect(() => () => window.clearTimeout(idleTimer.current), [])

  useEffect(() => {
    // The whole page counts too: "Vollbild beim Abspielen" puts the page in fullscreen on the click that opened it.
    const onChange = () => {
      setFullscreen(!!document.fullscreenElement && [wrap.current, document.documentElement].includes(document.fullscreenElement as HTMLDivElement))
      poke()
    }
    onChange()
    document.addEventListener('fullscreenchange', onChange)
    return () => document.removeEventListener('fullscreenchange', onChange)
  }, [poke])

  // Pointer and key activity is watched on the document (capture phase), not only on the player box:
  // in WebView2 fullscreen the box's own pointer events do not reliably arrive, so the bar never came back.
  useEffect(() => {
    let last = 0
    let at = ''
    const onMove = (e: Event) => {
      const inside = !!document.fullscreenElement || (e.target instanceof Node && !!wrap.current?.contains(e.target))
      if (!inside) return
      // Chromium sends synthetic moves at the same spot when the content under a resting cursor changes
      // (e.g. the bar fading in); counting those would keep the bar up forever.
      if (e instanceof MouseEvent && e.type !== 'pointerdown') {
        const here = `${e.screenX},${e.screenY}`
        if (here === at) return
        at = here
      }
      const now = performance.now()
      if (now - last < 120) return
      last = now
      poke()
    }
    const onKey = () => {
      if (document.fullscreenElement || wrap.current?.contains(document.activeElement)) poke()
    }
    const opts = { capture: true, passive: true }
    document.addEventListener('pointermove', onMove, opts)
    document.addEventListener('mousemove', onMove, opts)
    document.addEventListener('pointerdown', onMove, opts)
    document.addEventListener('keydown', onKey, opts)
    return () => {
      document.removeEventListener('pointermove', onMove, opts)
      document.removeEventListener('mousemove', onMove, opts)
      document.removeEventListener('pointerdown', onMove, opts)
      document.removeEventListener('keydown', onKey, opts)
    }
  }, [poke])
  const hidden = idle && playing && menu === null && scrub === null && !help && !volOpen

  // ---- actions ----
  const toggle = useCallback(() => {
    const el = video.current
    if (!el) return
    if (el.paused || el.ended) void el.play().catch(() => undefined)
    else el.pause()
  }, [])
  const seekTo = useCallback(
    (seconds: number) => {
      const el = video.current
      if (!el) return
      if (isTranscode()) {
        // Not seekable in place: restart ffmpeg at the new spot (debounced, so held arrow keys load once).
        const d = tor.current?.duration ?? 0
        if (d <= 0) return
        const at = clamp(seconds, 0, Math.max(0, d - 1))
        pendingSeek.current = at
        pos.current.time = at
        setTime(at)
        window.clearTimeout(seekTimer.current)
        seekTimer.current = window.setTimeout(() => {
          pendingSeek.current = null
          loadTorrent(at)
        }, 250)
        return
      }
      if (!Number.isFinite(el.duration)) return
      el.currentTime = clamp(seconds, 0, Math.max(0, el.duration - 0.5))
      setTime(el.currentTime)
    },
    [loadTorrent],
  )
  const toggleMute = useCallback(() => setVol((v) => ({ ...v, muted: !v.muted && v.volume > 0 })), [])
  const nudgeVolume = useCallback(
    (delta: number) => setVol((v) => ({ volume: clamp(Math.round((v.volume + delta) * 20) / 20, 0, 1), muted: false })),
    [],
  )
  const toggleFullscreen = useCallback(() => {
    if (document.fullscreenElement) void document.exitFullscreen().catch(() => undefined)
    else void wrap.current?.requestFullscreen?.().catch(() => undefined)
  }, [])

  const pickSub = useCallback((id: number) => {
    const hls = hlsRef.current
    if (hls) {
      hls.subtitleDisplay = id >= 0
      hls.subtitleTrack = id
    } else if (tor.current) {
      // Torrent: the chosen text track is a <track> fed by /subtitle.
      setSub(id)
    }
    setMenu(null)
  }, [])
  // A freshly mounted <track> is not shown by itself in every browser.
  useEffect(() => {
    const el = video.current
    if (!el || !tor.current) return
    for (const t of Array.from(el.textTracks)) t.mode = sub >= 0 ? 'showing' : 'disabled'
  }, [sub, tStart])
  // "c": subtitles on (the last chosen track, else the first in the UI language, else the first) or off.
  const lastSub = useRef(-1)
  if (sub >= 0) lastSub.current = sub
  const subsRef = useRef(subs)
  subsRef.current = subs
  const subRef = useRef(sub)
  subRef.current = sub
  const toggleSubs = useCallback(() => {
    const list = subsRef.current
    if (list.length === 0) return
    if (subRef.current >= 0) return pickSub(-1)
    const want = getLang().slice(0, 2).toUpperCase()
    const mine = list.find((s) => langCode(s.lang) === want)
    pickSub(lastSub.current >= 0 ? lastSub.current : (mine?.id ?? list[0]?.id ?? 0))
  }, [pickSub])
  const nextRef = useRef(next)
  nextRef.current = next

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey || e.metaKey || e.altKey) return
      const t = e.target instanceof HTMLElement ? e.target : null
      // Only text fields swallow the shortcuts; the volume slider keeps them (its own arrow keys stay native).
      const range = t instanceof HTMLInputElement && t.type === 'range'
      if (t && (t.isContentEditable || ['TEXTAREA', 'SELECT'].includes(t.tagName) || (t.tagName === 'INPUT' && !range))) return
      if (range && e.key.startsWith('Arrow')) return
      const inside = !!t && !!wrap.current?.contains(t)
      const onControl = !!t?.closest('button, a, [role="menuitemradio"]')
      const inMenu = !!t?.closest('.vp-pop')
      const el = video.current
      if (e.key === '?') {
        e.preventDefault()
        setMenu(null)
        setHelp((h) => !h)
        return
      }
      if (inMenu && ['ArrowUp', 'ArrowDown', ' ', 'Enter'].includes(e.key)) return
      switch (e.key.length === 1 ? e.key.toLowerCase() : e.key) {
        case ' ':
          if (onControl) return
          e.preventDefault()
          toggle()
          break
        case 'k':
          toggle()
          break
        case 'ArrowLeft':
          e.preventDefault()
          if (el) seekTo(nowAt() - 10)
          break
        case 'ArrowRight':
          e.preventDefault()
          if (el) seekTo(nowAt() + 10)
          break
        case 'ArrowUp':
          if (!inside && !document.fullscreenElement) return
          e.preventDefault()
          nudgeVolume(0.05)
          break
        case 'ArrowDown':
          if (!inside && !document.fullscreenElement) return
          e.preventDefault()
          nudgeVolume(-0.05)
          break
        case 'f':
          toggleFullscreen()
          break
        case 'm':
          toggleMute()
          break
        case 'c':
          toggleSubs()
          break
        case 'n':
          if (!nextRef.current) return
          nextRef.current.go()
          break
        case 'Escape':
          setMenu(null)
          setHelp(false)
          return
        default:
          return
      }
      poke()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [toggle, seekTo, nowAt, nudgeVolume, toggleFullscreen, toggleMute, toggleSubs, poke])

  // Single click pauses, double click goes fullscreen; wait a moment so a double click does not also pause.
  const clickTimer = useRef<number | undefined>(undefined)
  useEffect(() => () => window.clearTimeout(clickTimer.current), [])
  const onVideoClick = (e: ReactMouseEvent<HTMLVideoElement>) => {
    if ((e.nativeEvent as PointerEvent).pointerType === 'touch') {
      // Touch: the first tap only brings the controls back.
      if (hidden) poke()
      else toggle()
      return
    }
    if (e.detail > 1) return
    window.clearTimeout(clickTimer.current)
    clickTimer.current = window.setTimeout(toggle, DBL_MS)
  }
  const onVideoDoubleClick = () => {
    window.clearTimeout(clickTimer.current)
    toggleFullscreen()
  }

  // Close an open menu when the pointer goes down anywhere else.
  useEffect(() => {
    if (menu === null) return
    const onDown = (e: PointerEvent) => {
      if (!(e.target as HTMLElement | null)?.closest('.vp-menu')) setMenu(null)
    }
    window.addEventListener('pointerdown', onDown)
    return () => window.removeEventListener('pointerdown', onDown)
  }, [menu])

  // Phones: the opened slider folds back on a tap elsewhere, or 3.5 s after the last change.
  useEffect(() => {
    if (!volOpen) return
    const onDown = (e: PointerEvent) => {
      if (!volBox.current?.contains(e.target as Node)) setVolOpen(false)
    }
    window.addEventListener('pointerdown', onDown)
    const timer = window.setTimeout(() => setVolOpen(false), 3500)
    return () => {
      window.removeEventListener('pointerdown', onDown)
      window.clearTimeout(timer)
    }
  }, [volOpen, vol])
  useEffect(() => {
    if (!narrow) setVolOpen(false)
  }, [narrow])
  // Phone tap on the speaker: the first one opens the slider, the next ones mute. Desktop: mute.
  const onSpeaker = () => {
    if (narrow && CAN_SET_VOLUME && !volOpen) setVolOpen(true)
    else toggleMute()
  }
  const speakerOpens = narrow && CAN_SET_VOLUME && !volOpen

  const pickLevel = (id: number) => {
    const hls = hlsRef.current
    if (hls) hls.currentLevel = id
    setLevel(id)
    setMenu(null)
  }
  const pickAudio = (id: number) => {
    setMenu(null)
    const src = tor.current
    if (src) {
      // Another track means ffmpeg; the file's first track plays directly again where that is possible.
      const s = tState.current
      const at = nowAt()
      s.audio = id
      if (src.mode === 'direct') s.mode = id === 0 && !s.h264 ? 'direct' : 'transcode'
      setAudio(id)
      loadTorrent(at)
      return
    }
    const hls = hlsRef.current
    if (hls) hls.audioTrack = id
  }

  // ---- seek bar ----
  const ratioAt = (clientX: number) => {
    const r = bar.current?.getBoundingClientRect()
    return r && r.width > 0 ? clamp((clientX - r.left) / r.width, 0, 1) : 0
  }
  const onBarDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (duration <= 0) return
    e.currentTarget.setPointerCapture(e.pointerId)
    setScrub(ratioAt(e.clientX) * duration)
  }
  const onBarMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    const r = ratioAt(e.clientX)
    setHover(r)
    if (scrub !== null) setScrub(r * duration)
  }
  const onBarUp = () => {
    if (scrub === null) return
    seekTo(scrub)
    setScrub(null)
  }

  // ---- up next: countdown card in the last 30 s and at the end ----
  const { autoNext, autoNextSeconds: countdown } = usePrefs()
  const [upNextOff, setUpNextOff] = useState(false)
  const [count, setCount] = useState(countdown)
  const nearEnd = !!next && started && !fatal && duration > 60 && (ended || duration - time <= UP_NEXT_S)
  const upNext = nearEnd && !upNextOff
  // "Nächste Folge automatisch" off: the card still offers the next episode, it just does not count down.
  const ticking = upNext && autoNext && (playing || ended) && scrub === null
  useEffect(() => {
    if (!nearEnd) setUpNextOff(false)
  }, [nearEnd])
  useEffect(() => {
    if (!upNext) setCount(countdown)
  }, [upNext, countdown])
  useEffect(() => {
    if (!ticking) return
    const t = window.setInterval(() => setCount((c) => Math.max(0, c - 1)), 1000)
    return () => window.clearInterval(t)
  }, [ticking])
  useEffect(() => {
    if (upNext && count === 0) nextRef.current?.go()
  }, [upNext, count])

  // ---- "Fortgesetzt bei …" ----
  const [hintGone, setHintGone] = useState(false)
  const showHint = !!resumeHint && started && !hintGone && !fatal && !upNext
  const hintDone = useRef(onHintDone)
  hintDone.current = onHintDone
  useEffect(() => {
    if (!showHint) return
    const t = window.setTimeout(() => {
      setHintGone(true)
      hintDone.current?.()
    }, HINT_MS)
    return () => window.clearTimeout(t)
  }, [showHint])
  const restart = () => {
    seekTo(0)
    setHintGone(true)
    hintDone.current?.()
    const el = video.current
    if (el?.paused) void el.play().catch(() => undefined)
  }

  const shown = scrub ?? time
  const pct = (seconds: number) => (duration > 0 ? clamp((seconds / duration) * 100, 0, 100) : 0)
  const ahead = buffer.data && prefetching && buffer.data.ahead_seconds > 0 ? buffer.data : null
  const activeLabel = levels.find((l) => l.id === activeLevel)?.label
  const qualityLabel =
    level === -1
      ? activeLabel
        ? t('vp.autoWith', { level: activeLabel })
        : t('vp.auto')
      : (levels.find((l) => l.id === level)?.label ?? t('vp.auto'))
  const volumeShown = vol.muted ? 0 : vol.volume
  const subOptions: Option[] = subs.map((s) => ({ id: s.id, label: trackLabel(s, s.id, t('scan.subs')) }))
  const audioOptions: Option[] = audios.map((a) => ({ id: a.id, label: trackLabel(a, a.id, t('scan.audio')) }))
  const withKey = (label: string, key: string) => t('common.withKey', { label, key })
  const keyName = (keys: string) => (keys.startsWith('vp.') ? t(keys as Key) : keys)

  return (
    <div
      ref={wrap}
      className={`vp${hidden ? ' is-idle' : ''}${fullscreen ? ' is-fullscreen' : ''}`}
      data-audio-track={audio}
      data-subtitle-track={sub}
      onPointerLeave={() => playing && !document.fullscreenElement && setIdle(true)}
    >
      <video
        ref={video}
        className="vp-video"
        playsInline
        preload="auto"
        aria-label={subtitle ? `${title}, ${subtitle}` : title}
        onClick={onVideoClick}
        onDoubleClick={onVideoDoubleClick}
      >
        {torrent && sub >= 0 && (
          <track
            key={`${sub}-${tStart}`}
            kind="subtitles"
            src={subtitleUrl(torrent.id, torrent.file, sub, tStart)}
            srcLang={subs.find((x) => x.id === sub)?.lang?.slice(0, 2) ?? undefined}
            label={subOptions.find((x) => x.id === sub)?.label ?? ''}
            default
          />
        )}
      </video>

      {(waiting || !started) && !fatal && !ended && (
        <div className="vp-center" aria-hidden={started}>
          <Flicker size={52} label={started ? undefined : t('vp.loading')} />
          {torrent?.status && <p className="vp-center-status">{torrent.status}</p>}
        </div>
      )}

      {started && !playing && !waiting && !ended && !fatal && (
        <button type="button" className="vp-bigplay" onClick={toggle} aria-label={t('common.play')}>
          <Icon name="play" size={34} />
        </button>
      )}

      {fatal && (
        <div className="vp-overlay" role="alert">
          <Icon name="warn" size={28} />
          <p className="vp-overlay-title">{t(fatal)}</p>
          <p>{t('vp.fatalHint')}</p>
          {fallback && (
            <button type="button" className="btn btn-primary btn-lg" onClick={fallback.run} autoFocus>
              <Icon name="refresh" size={18} />
              {fallback.label}
            </button>
          )}
        </div>
      )}

      {help && (
        <div className="vp-help" role="dialog" aria-modal="false" aria-labelledby="vp-help-title">
          <div className="vp-help-card">
            <div className="vp-help-head">
              <p className="vp-help-title" id="vp-help-title">
                {t('vp.shortcuts')}
              </p>
              <button type="button" className="vp-btn" onClick={() => setHelp(false)} aria-label={t('vp.closeHelp')} autoFocus>
                <Icon name="close" size={20} />
              </button>
            </div>
            <dl className="vp-help-list">
              {KEYS.filter((k) => (k.id !== 'next' || !!next) && (k.id !== 'subs' || subs.length > 0)).map((k) => (
                <div key={k.id}>
                  <dt>
                    {keyName(k.keys)
                      .split(' / ')
                      .map((part, i) => (
                        <span key={part}>
                          {i > 0 && ' '}
                          <kbd>{part}</kbd>
                        </span>
                      ))}
                  </dt>
                  <dd>{t(k.what)}</dd>
                </div>
              ))}
            </dl>
          </div>
        </div>
      )}

      {showHint && resumeHint !== undefined && (
        <div className="vp-resume" role="status">
          <span>{t('vp.resumed', { t: formatClock(resumeHint) })}</span>
          <button type="button" className="vp-resume-btn" onClick={restart}>
            <Icon name="refresh" size={15} />
            {t('vp.restart')}
          </button>
        </div>
      )}

      {upNext && next && (
        <section className="vp-upnext" aria-label={t('player.nextEpisode')}>
          {next.still && <img className="vp-upnext-still" src={next.still} alt="" />}
          <div className="vp-upnext-body">
            <p className="vp-upnext-kicker">{autoNext ? t('vp.nextIn', { n: count }) : t('vp.upNext')}</p>
            <p className="vp-upnext-title">{next.label}</p>
            {next.name && <p className="vp-upnext-name">{next.name}</p>}
            <div className="vp-upnext-actions">
              <button type="button" className="btn btn-primary vp-upnext-go" onClick={next.go}>
                {autoNext && (
                  <span
                    className="vp-upnext-fill"
                    aria-hidden="true"
                    style={{ transform: `scaleX(${(countdown - count) / countdown})` }}
                  />
                )}
                <Icon name="play" size={16} />
                {t('vp.playNow')}
              </button>
              <button
                type="button"
                className="btn btn-glass vp-upnext-cancel"
                onClick={() => setUpNextOff(true)}
                aria-label={t('common.cancel')}
              >
                <Icon name="close" size={16} />
                <span className="vp-upnext-cancel-text">{t('common.cancel')}</span>
              </button>
            </div>
          </div>
        </section>
      )}

      {ended && !fatal && !upNext && (
        <div className="vp-overlay">
          {next ? (
            <>
              <p className="vp-overlay-kicker">{t('vp.upNext')}</p>
              <p className="vp-overlay-title">{next.label}</p>
              <div className="actions">
                <button type="button" className="btn btn-primary btn-lg" onClick={next.go} autoFocus>
                  <Icon name="next" size={18} />
                  {t('player.nextEpisode')}
                </button>
                <button type="button" className="btn btn-glass btn-lg" onClick={() => (seekTo(0), toggle())}>
                  <Icon name="refresh" size={18} />
                  {t('vp.again')}
                </button>
              </div>
            </>
          ) : (
            <>
              <p className="vp-overlay-title">{t('vp.theEnd')}</p>
              <button type="button" className="btn btn-glass btn-lg" onClick={() => (seekTo(0), toggle())}>
                <Icon name="refresh" size={18} />
                {t('vp.watchAgain')}
              </button>
            </>
          )}
        </div>
      )}

      <div className="vp-top">
        <p className="vp-title">{title}</p>
        {subtitle && <p className="vp-subtitle">{subtitle}</p>}
      </div>

      <div className="vp-controls" onPointerEnter={poke}>
        <div
          ref={bar}
          className={`vp-seek${scrub !== null ? ' is-scrubbing' : ''}`}
          role="slider"
          tabIndex={0}
          aria-label={t('vp.position')}
          aria-valuemin={0}
          aria-valuemax={Math.round(duration)}
          aria-valuenow={Math.round(shown)}
          aria-valuetext={t('vp.positionText', { a: formatClock(shown), b: formatClock(duration) })}
          onPointerDown={onBarDown}
          onPointerMove={onBarMove}
          onPointerUp={onBarUp}
          onPointerCancel={() => setScrub(null)}
          onPointerLeave={() => setHover(null)}
        >
          <div className="vp-seek-track">
            {ahead && (
              <span
                className="vp-seek-ahead"
                style={{ left: `${pct(ahead.from_seconds)}%`, width: `${pct(ahead.ahead_seconds)}%` }}
              />
            )}
            {buffered.map(([a, z]) => (
              <span key={a} className="vp-seek-buffered" style={{ left: `${pct(a)}%`, width: `${pct(z - a)}%` }} />
            ))}
            <span className="vp-seek-played" style={{ width: `${pct(shown)}%` }} />
          </div>
          <span className="vp-seek-knob" style={{ left: `${pct(shown)}%` }} />
          {hover !== null && duration > 0 && (
            <span className="vp-seek-tip" style={{ left: `${clamp(hover * 100, 3, 97)}%` }}>
              {formatClock(hover * duration)}
            </span>
          )}
        </div>

        <div className="vp-row">
          <button
            type="button"
            className="vp-btn"
            onClick={toggle}
            aria-label={playing ? t('vp.pause') : t('common.play')}
            aria-keyshortcuts="k"
            title={withKey(playing ? t('vp.pause') : t('common.play'), t('vp.space'))}
          >
            <Icon name={playing ? 'pause' : 'play'} size={22} />
          </button>
          {next && (
            <button
              type="button"
              className="vp-btn"
              onClick={next.go}
              aria-label={t('vp.nextLabel', { label: next.label })}
              aria-keyshortcuts="n"
              title={withKey(t('player.nextEpisode'), 'N')}
            >
              <Icon name="next" size={22} />
            </button>
          )}
          {/* Speaker plus a slim slider that slides out of it (hover / focus on desktop, a tap on phones). */}
          <div ref={volBox} className={`vp-volume${volOpen ? ' is-open' : ''}`}>
            <button
              type="button"
              className="vp-btn"
              onClick={onSpeaker}
              aria-label={speakerOpens ? t('vp.volume') : vol.muted ? t('vp.unmute') : t('vp.mute')}
              aria-expanded={narrow && CAN_SET_VOLUME ? volOpen : undefined}
              aria-pressed={speakerOpens ? undefined : vol.muted}
              aria-keyshortcuts="m"
              title={speakerOpens ? t('vp.volume') : withKey(vol.muted ? t('vp.unmute') : t('vp.mute'), 'M')}
            >
              <Icon name={volumeShown === 0 ? 'mute' : 'volume'} size={22} />
            </button>
            {CAN_SET_VOLUME && <VolumeRange t={t} value={volumeShown} onChange={(v) => setVol({ volume: v, muted: false })} />}
          </div>
          <p className="vp-time">
            <span>{formatClock(shown)}</span>
            <span className="vp-time-total">{formatClock(duration)}</span>
          </p>
          {ahead && (
            <p className="vp-ahead" title={t('vp.aheadTitle')}>
              <span className="vp-ahead-dot" />
              {t(ahead.done ? 'vp.prefetched' : 'vp.prefetching', { n: Math.max(1, Math.round(ahead.ahead_seconds / 60)) })}
            </p>
          )}
          {torrent?.status && (
            <p className="vp-ahead vp-peers" role="status">
              <span className="vp-ahead-dot" />
              {torrent.status}
            </p>
          )}

          <span className="vp-spacer" />

          {subs.length > 0 && (
            <PlayerMenu
              id="subs"
              icon="subtitles"
              title={t('scan.subs')}
              hint="C"
              open={menu === 'subs'}
              onToggle={setMenu}
              value={sub}
              options={[{ id: -1, label: t('vp.off') }, ...subOptions]}
              onPick={pickSub}
            />
          )}
          {audios.length > 1 && (
            <PlayerMenu
              id="audio"
              icon="audio"
              title={t('scan.audio')}
              open={menu === 'audio'}
              onToggle={setMenu}
              value={audio}
              options={audioOptions}
              onPick={pickAudio}
            />
          )}
          {levels.length > 0 && (
            <PlayerMenu
              id="quality"
              icon="layers"
              title={t('vp.quality')}
              text={qualityLabel}
              open={menu === 'quality'}
              onToggle={setMenu}
              value={level}
              options={[{ id: -1, label: t('vp.auto') }, ...levels]}
              onPick={pickLevel}
            />
          )}
          <button
            type="button"
            className="vp-btn vp-btn-help"
            onClick={() => (setMenu(null), setHelp((h) => !h))}
            aria-label={t('vp.shortcuts')}
            aria-expanded={help}
            aria-keyshortcuts="?"
            title={withKey(t('vp.shortcuts'), '?')}
          >
            <span className="vp-help-q" aria-hidden="true">
              ?
            </span>
          </button>
          <button
            type="button"
            className="vp-btn"
            onClick={toggleFullscreen}
            aria-label={fullscreen ? t('vp.exitFullscreen') : t('vp.fullscreen')}
            aria-keyshortcuts="f"
            title={withKey(fullscreen ? t('vp.exitFullscreen') : t('vp.fullscreen'), 'F')}
          >
            <Icon name={fullscreen ? 'shrink' : 'expand'} size={22} />
          </button>
        </div>
      </div>
    </div>
  )
}

type MenuProps = {
  id: MenuId
  icon: IconName
  title: string
  /** Keyboard shortcut shown in the tooltip. */
  hint?: string
  text?: string
  open: boolean
  onToggle: (id: MenuId | null) => void
  value: number
  options: Option[]
  onPick: (id: number) => void
}

function PlayerMenu({ id, icon, title, hint, text, open, onToggle, value, options, onPick }: MenuProps) {
  const trigger = useRef<HTMLButtonElement>(null)
  const pop = useRef<HTMLDivElement>(null)
  const wasOpen = useRef(false)
  const { t } = useI18n()
  // Opening moves focus to the current choice; closing gives it back to the button.
  useEffect(() => {
    if (open) {
      pop.current?.querySelector<HTMLElement>('[aria-checked="true"]')?.focus()
      pop.current?.querySelector<HTMLElement>('[aria-checked="true"]')?.scrollIntoView({ block: 'nearest' })
    } else if (wasOpen.current && pop.current === null && document.activeElement === document.body) {
      trigger.current?.focus()
    }
    wasOpen.current = open
  }, [open])
  const onKeyDown = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp' && e.key !== 'Home' && e.key !== 'End') return
    e.preventDefault()
    const items = [...(pop.current?.querySelectorAll<HTMLElement>('[role="menuitemradio"]') ?? [])]
    const at = items.indexOf(document.activeElement as HTMLElement)
    const to =
      e.key === 'Home' ? 0 : e.key === 'End' ? items.length - 1 : (at + (e.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length
    items[to]?.focus()
  }
  return (
    <div className="vp-menu">
      <button
        ref={trigger}
        type="button"
        className={`vp-btn${text ? ' has-text' : ''}`}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={text ? `${title}: ${text}` : title}
        title={hint ? t('common.withKey', { label: title, key: hint }) : title}
        onClick={() => onToggle(open ? null : id)}
      >
        <Icon name={icon} size={22} />
        {text && <span className="vp-btn-text">{text}</span>}
      </button>
      {open && (
        <div className="vp-pop" role="menu" aria-label={title} ref={pop} onKeyDown={onKeyDown}>
          <p className="vp-pop-title">{title}</p>
          {options.map((o) => (
            <button
              key={o.id}
              type="button"
              role="menuitemradio"
              aria-checked={o.id === value}
              className="vp-pop-item"
              onClick={() => (onPick(o.id), trigger.current?.focus())}
            >
              <span className="vp-pop-check">{o.id === value && <Icon name="check" size={16} />}</span>
              <span className="vp-pop-label">{o.label}</span>
              {o.hint && <span className="vp-pop-hint">{o.hint}</span>}
            </button>
          ))}
        </div>
      )}
    </div>
  )
}
