import Hls from 'hls.js'
import { useCallback, useEffect, useRef, useState, type CSSProperties, type PointerEvent as ReactPointerEvent } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { api, streamUrl, type Variant } from '../../lib/api'
import { formatBitrate, formatClock, formatHeight, trackLabel } from '../../lib/format'
import { variantIndex } from '../../lib/pick'
import { getVolume, setVolume, type VolumePref } from '../../lib/storage'
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
}

type Option = { id: number; label: string; hint?: string }
type MenuId = 'quality' | 'audio' | 'subs'

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v))
const IDLE_MS = 2800
const SAVE_EVERY_MS = 10_000
/** Credits shortcut: offer the next episode this many seconds before the end. */
const UP_NEXT_S = 30
const UP_NEXT_COUNTDOWN = 8
const HINT_MS = 8000

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
}: Props) {
  const wrap = useRef<HTMLDivElement>(null)
  const video = useRef<HTMLVideoElement>(null)
  const bar = useRef<HTMLDivElement>(null)
  const hlsRef = useRef<Hls | null>(null)
  const qc = useQueryClient()

  const [playing, setPlaying] = useState(false)
  const [waiting, setWaiting] = useState(true)
  const [started, setStarted] = useState(false)
  const [ended, setEnded] = useState(false)
  const [fatal, setFatal] = useState<string | null>(null)
  const [time, setTime] = useState(startAt)
  const [duration, setDuration] = useState(0)
  const [buffered, setBuffered] = useState<[number, number][]>([])
  const [vol, setVol] = useState<VolumePref>(getVolume)
  const [fullscreen, setFullscreen] = useState(false)
  const [levels, setLevels] = useState<Option[]>([])
  const [level, setLevel] = useState(-1)
  const [activeLevel, setActiveLevel] = useState(-1)
  const [audios, setAudios] = useState<Option[]>([])
  const [audio, setAudio] = useState(-1)
  const [subs, setSubs] = useState<Option[]>([])
  const [sub, setSub] = useState(-1)
  const [menu, setMenu] = useState<MenuId | null>(null)
  const [idle, setIdle] = useState(false)
  const [hover, setHover] = useState<number | null>(null)
  const [scrub, setScrub] = useState<number | null>(null)
  const [prefetching, setPrefetching] = useState(false)

  // Latest callbacks/props for the long-lived listeners below.
  const live = useRef({ onTime, onExpired, onSave, variants, startAt })
  live.current = { onTime, onExpired, onSave, variants, startAt: live.current.startAt }

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
    document.addEventListener('visibilitychange', onHide)
    return () => {
      window.removeEventListener('pagehide', save)
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
    const url = streamUrl(streamId)
    const from = live.current.startAt
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
          else setFatal('Der Stream lässt sich nicht laden.')
        })
        .catch(() => !disposed && setFatal('Der Flimmer-Server ist nicht erreichbar.'))
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
              label: formatHeight(l.height) ?? formatBitrate(l.bitrate) ?? `Variante ${l.id + 1}`,
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
        setAudios(h.audioTracks.map((t, id) => ({ id, label: trackLabel(t, id, 'Tonspur') })))
        setAudio(h.audioTrack)
      }
      const syncSubs = () => {
        setSubs(h.subtitleTracks.map((t, id) => ({ id, label: trackLabel(t, id, 'Untertitel') })))
        setSub(h.subtitleTrack)
      }
      h.on(Hls.Events.AUDIO_TRACKS_UPDATED, syncAudio)
      h.on(Hls.Events.AUDIO_TRACK_SWITCHED, syncAudio)
      h.on(Hls.Events.SUBTITLE_TRACKS_UPDATED, syncSubs)
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
          setFatal(
            data.type === Hls.ErrorTypes.NETWORK_ERROR
              ? 'Die Quelle liefert keine Daten mehr.'
              : 'Dieses Video lässt sich hier nicht abspielen.',
          )
        }
      })
      h.loadSource(url)
      h.attachMedia(el)
    } else if (el.canPlayType('application/vnd.apple.mpegurl')) {
      playNative()
    } else {
      setFatal('Dieser Browser kann den Stream nicht abspielen.')
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
      for (let i = 0; i < el.buffered.length; i += 1) out.push([el.buffered.start(i), el.buffered.end(i)])
      setBuffered(out)
    }
    const on: [keyof HTMLMediaElementEventMap, () => void][] = [
      ['play', () => (setPlaying(true), setEnded(false))],
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
      ['seeked', () => (prefetchRef.current(), (pos.current.time = el.currentTime), save())],
      [
        'timeupdate',
        () => {
          setTime(el.currentTime)
          live.current.onTime(el.currentTime)
          if (pos.current.played) pos.current.time = el.currentTime
        },
      ],
      [
        'durationchange',
        () => {
          const d = Number.isFinite(el.duration) ? el.duration : 0
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

  useEffect(() => {
    const onChange = () => setFullscreen(document.fullscreenElement === wrap.current)
    document.addEventListener('fullscreenchange', onChange)
    return () => document.removeEventListener('fullscreenchange', onChange)
  }, [])

  // ---- controls visibility ----
  const idleTimer = useRef<number | undefined>(undefined)
  const poke = useCallback(() => {
    setIdle(false)
    window.clearTimeout(idleTimer.current)
    idleTimer.current = window.setTimeout(() => setIdle(true), IDLE_MS)
  }, [])
  useEffect(() => () => window.clearTimeout(idleTimer.current), [])
  const hidden = idle && playing && menu === null && scrub === null

  // ---- actions ----
  const toggle = useCallback(() => {
    const el = video.current
    if (!el) return
    if (el.paused || el.ended) void el.play().catch(() => undefined)
    else el.pause()
  }, [])
  const seekTo = useCallback((seconds: number) => {
    const el = video.current
    if (!el || !Number.isFinite(el.duration)) return
    el.currentTime = clamp(seconds, 0, Math.max(0, el.duration - 0.5))
    setTime(el.currentTime)
  }, [])
  const toggleMute = useCallback(() => setVol((v) => ({ ...v, muted: !v.muted && v.volume > 0 })), [])
  const nudgeVolume = useCallback(
    (delta: number) => setVol((v) => ({ volume: clamp(Math.round((v.volume + delta) * 20) / 20, 0, 1), muted: false })),
    [],
  )
  const toggleFullscreen = useCallback(() => {
    if (document.fullscreenElement) void document.exitFullscreen().catch(() => undefined)
    else void wrap.current?.requestFullscreen?.().catch(() => undefined)
  }, [])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey || e.metaKey || e.altKey) return
      const t = e.target as HTMLElement | null
      if (t && (t.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(t.tagName))) return
      const inside = !!t && !!wrap.current?.contains(t)
      const onControl = !!t?.closest('button, a, [role="menuitemradio"]')
      const el = video.current
      switch (e.key) {
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
          if (el) seekTo(el.currentTime - 10)
          break
        case 'ArrowRight':
          e.preventDefault()
          if (el) seekTo(el.currentTime + 10)
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
        case 'Escape':
          setMenu(null)
          return
        default:
          return
      }
      poke()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [toggle, seekTo, nudgeVolume, toggleFullscreen, toggleMute, poke])

  // Close an open menu when the pointer goes down anywhere else.
  useEffect(() => {
    if (menu === null) return
    const onDown = (e: PointerEvent) => {
      if (!(e.target as HTMLElement | null)?.closest('.vp-menu')) setMenu(null)
    }
    window.addEventListener('pointerdown', onDown)
    return () => window.removeEventListener('pointerdown', onDown)
  }, [menu])

  const pickLevel = (id: number) => {
    const hls = hlsRef.current
    if (hls) hls.currentLevel = id
    setLevel(id)
    setMenu(null)
  }
  const pickAudio = (id: number) => {
    const hls = hlsRef.current
    if (hls) hls.audioTrack = id
    setAudio(id)
    setMenu(null)
  }
  const pickSub = (id: number) => {
    const hls = hlsRef.current
    if (hls) {
      hls.subtitleDisplay = id >= 0
      hls.subtitleTrack = id
    }
    setSub(id)
    setMenu(null)
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
  const nextRef = useRef(next)
  nextRef.current = next
  const [upNextOff, setUpNextOff] = useState(false)
  const [count, setCount] = useState(UP_NEXT_COUNTDOWN)
  const nearEnd = !!next && started && !fatal && duration > 60 && (ended || duration - time <= UP_NEXT_S)
  const upNext = nearEnd && !upNextOff
  const ticking = upNext && (playing || ended) && scrub === null
  useEffect(() => {
    if (!nearEnd) setUpNextOff(false)
  }, [nearEnd])
  useEffect(() => {
    if (!upNext) setCount(UP_NEXT_COUNTDOWN)
  }, [upNext])
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
  const qualityLabel = level === -1 ? (activeLabel ? `Auto, ${activeLabel}` : 'Auto') : (levels.find((l) => l.id === level)?.label ?? 'Auto')
  const volumeShown = vol.muted ? 0 : vol.volume

  return (
    <div
      ref={wrap}
      className={`vp${hidden ? ' is-idle' : ''}${fullscreen ? ' is-fullscreen' : ''}`}
      onPointerMove={poke}
      onPointerLeave={() => playing && setIdle(true)}
    >
      <video
        ref={video}
        className="vp-video"
        playsInline
        preload="auto"
        aria-label={subtitle ? `${title}, ${subtitle}` : title}
        onClick={(e) => {
          if ((e.nativeEvent as PointerEvent).pointerType === 'touch' && hidden) poke()
          else toggle()
        }}
        onDoubleClick={toggleFullscreen}
      />

      {(waiting || !started) && !fatal && !ended && (
        <div className="vp-center" aria-hidden={started}>
          <Flicker size={52} label={started ? undefined : 'Video wird geladen'} />
        </div>
      )}

      {started && !playing && !waiting && !ended && !fatal && (
        <button type="button" className="vp-bigplay" onClick={toggle} aria-label="Abspielen">
          <Icon name="play" size={34} />
        </button>
      )}

      {fatal && (
        <div className="vp-overlay" role="alert">
          <Icon name="warn" size={28} />
          <p className="vp-overlay-title">{fatal}</p>
          <p>Wähle rechts eine andere Quelle oder prüfe die Quellen neu.</p>
        </div>
      )}

      {showHint && resumeHint !== undefined && (
        <div className="vp-resume" role="status">
          <span>Fortgesetzt bei {formatClock(resumeHint)}</span>
          <button type="button" className="vp-resume-btn" onClick={restart}>
            <Icon name="refresh" size={15} />
            Von vorn
          </button>
        </div>
      )}

      {upNext && next && (
        <section className="vp-upnext" aria-label="Nächste Folge">
          {next.still && <img className="vp-upnext-still" src={next.still} alt="" />}
          <div className="vp-upnext-body">
            <p className="vp-upnext-kicker">Nächste Folge in {count} s</p>
            <p className="vp-upnext-title">{next.label}</p>
            {next.name && <p className="vp-upnext-name">{next.name}</p>}
            <div className="vp-upnext-actions">
              <button type="button" className="btn btn-primary vp-upnext-go" onClick={next.go}>
                <span
                  className="vp-upnext-fill"
                  aria-hidden="true"
                  style={{ transform: `scaleX(${(UP_NEXT_COUNTDOWN - count) / UP_NEXT_COUNTDOWN})` }}
                />
                <Icon name="play" size={16} />
                Jetzt abspielen
              </button>
              <button type="button" className="btn btn-glass" onClick={() => setUpNextOff(true)}>
                Abbrechen
              </button>
            </div>
          </div>
        </section>
      )}

      {ended && !fatal && !upNext && (
        <div className="vp-overlay">
          {next ? (
            <>
              <p className="vp-overlay-kicker">Als Nächstes</p>
              <p className="vp-overlay-title">{next.label}</p>
              <div className="actions">
                <button type="button" className="btn btn-primary btn-lg" onClick={next.go} autoFocus>
                  <Icon name="next" size={18} />
                  Nächste Folge
                </button>
                <button type="button" className="btn btn-glass btn-lg" onClick={() => (seekTo(0), toggle())}>
                  <Icon name="refresh" size={18} />
                  Noch einmal
                </button>
              </div>
            </>
          ) : (
            <>
              <p className="vp-overlay-title">Zu Ende</p>
              <button type="button" className="btn btn-glass btn-lg" onClick={() => (seekTo(0), toggle())}>
                <Icon name="refresh" size={18} />
                Noch einmal ansehen
              </button>
            </>
          )}
        </div>
      )}

      <div className="vp-top">
        <p className="vp-title">{title}</p>
        {subtitle && <p className="vp-subtitle">{subtitle}</p>}
      </div>

      <div className="vp-controls" onPointerMove={(e) => e.stopPropagation()} onPointerEnter={() => setIdle(false)}>
        <div
          ref={bar}
          className={`vp-seek${scrub !== null ? ' is-scrubbing' : ''}`}
          role="slider"
          tabIndex={0}
          aria-label="Position im Video"
          aria-valuemin={0}
          aria-valuemax={Math.round(duration)}
          aria-valuenow={Math.round(shown)}
          aria-valuetext={`${formatClock(shown)} von ${formatClock(duration)}`}
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
          <button type="button" className="vp-btn" onClick={toggle} aria-label={playing ? 'Pause' : 'Abspielen'}>
            <Icon name={playing ? 'pause' : 'play'} size={22} />
          </button>
          {next && (
            <button type="button" className="vp-btn" onClick={next.go} aria-label={`Nächste Folge: ${next.label}`} title="Nächste Folge">
              <Icon name="next" size={22} />
            </button>
          )}
          <div className="vp-volume">
            <button type="button" className="vp-btn" onClick={toggleMute} aria-label={vol.muted ? 'Ton an' : 'Ton aus'} aria-pressed={vol.muted}>
              <Icon name={volumeShown === 0 ? 'mute' : 'volume'} size={22} />
            </button>
            <input
              type="range"
              className="vp-range"
              min={0}
              max={1}
              step={0.05}
              value={volumeShown}
              onChange={(e) => setVol({ volume: Number(e.target.value), muted: false })}
              aria-label="Lautstärke"
              style={{ '--fill': `${volumeShown * 100}%` } as CSSProperties}
            />
          </div>
          <p className="vp-time">
            <span>{formatClock(shown)}</span>
            <span className="vp-time-total">{formatClock(duration)}</span>
          </p>
          {ahead && (
            <p className="vp-ahead" title="So weit hat Flimmer den Film schon auf die Platte geladen">
              <span className="vp-ahead-dot" />
              {ahead.done ? 'Vorgeladen' : 'Lädt vor'}: {Math.max(1, Math.round(ahead.ahead_seconds / 60))} Min.
            </p>
          )}

          <span className="vp-spacer" />

          {subs.length > 0 && (
            <PlayerMenu
              id="subs"
              icon="subtitles"
              title="Untertitel"
              open={menu === 'subs'}
              onToggle={setMenu}
              value={sub}
              options={[{ id: -1, label: 'Aus' }, ...subs]}
              onPick={pickSub}
            />
          )}
          {audios.length > 1 && (
            <PlayerMenu id="audio" icon="audio" title="Tonspur" open={menu === 'audio'} onToggle={setMenu} value={audio} options={audios} onPick={pickAudio} />
          )}
          {levels.length > 0 && (
            <PlayerMenu
              id="quality"
              icon="layers"
              title="Qualität"
              text={qualityLabel}
              open={menu === 'quality'}
              onToggle={setMenu}
              value={level}
              options={[{ id: -1, label: 'Auto' }, ...levels]}
              onPick={pickLevel}
            />
          )}
          <button type="button" className="vp-btn" onClick={toggleFullscreen} aria-label={fullscreen ? 'Vollbild verlassen' : 'Vollbild'}>
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
  text?: string
  open: boolean
  onToggle: (id: MenuId | null) => void
  value: number
  options: Option[]
  onPick: (id: number) => void
}

function PlayerMenu({ id, icon, title, text, open, onToggle, value, options, onPick }: MenuProps) {
  return (
    <div className="vp-menu">
      <button
        type="button"
        className={`vp-btn${text ? ' has-text' : ''}`}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={text ? `${title}: ${text}` : title}
        title={title}
        onClick={() => onToggle(open ? null : id)}
      >
        <Icon name={icon} size={22} />
        {text && <span className="vp-btn-text">{text}</span>}
      </button>
      {open && (
        <div className="vp-pop" role="menu" aria-label={title}>
          <p className="vp-pop-title">{title}</p>
          {options.map((o) => (
            <button
              key={o.id}
              type="button"
              role="menuitemradio"
              aria-checked={o.id === value}
              className="vp-pop-item"
              onClick={() => onPick(o.id)}
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
