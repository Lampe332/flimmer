import { useRef, useState, type ReactNode } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Link, useNavigate } from 'react-router'
import { PrefRow, Segmented, Toggle } from '../components/Controls'
import Icon, { type IconName } from '../components/Icon'
import { OpenPage, TokenField, VpnImport } from '../components/SetupParts'
import { startTour } from '../components/Tour'
import { VpnBadge } from '../components/Torrents'
import { LANGUAGES, useI18n, type Key } from '../i18n'
import { ApiError, api, type AudioLang, type MinHeight, type Settings as ScanSettings, type SortKey } from '../lib/api'
import { ACCENTS, COUNTDOWNS, START_PAGES, loadPrefs, resetLocalPrefs, setPrefs, usePrefs, type Accent } from '../lib/prefs'
import { readTextFile, setupApi, useSetup, type Page } from '../lib/setup'
import { formatBytes, refreshTorrents, torrentApi, useTorrents, type TorrentSettings } from '../lib/torrents'

const SECTIONS: { id: string; label: Key; icon: IconName }[] = [
  { id: 'allgemein', label: 'pref.general', icon: 'sliders' },
  { id: 'player', label: 'pref.player', icon: 'play' },
  { id: 'quellen', label: 'pref.sources', icon: 'layers' },
  { id: 'torrents', label: 'pref.torrents', icon: 'download' },
  { id: 'tmdb', label: 'pref.tmdb', icon: 'film' },
  { id: 'daten', label: 'pref.data', icon: 'folder' },
  { id: 'info', label: 'pref.about', icon: 'info' },
]

const ACCENT_SWATCH: Record<Accent, string> = { amber: '#ffb443', rose: '#ff7f9c', mint: '#5fd8b0', sky: '#7db4ff' }

function Section({ id, title, icon, children }: { id: string; title: string; icon: IconName; children: ReactNode }) {
  return (
    <section className="set-card pref-card" id={id} aria-labelledby={`${id}-h`}>
      <h2 className="set-title" id={`${id}-h`}>
        <Icon name={icon} size={18} />
        {title}
      </h2>
      {children}
    </section>
  )
}

export default function Settings() {
  const { t } = useI18n()
  return (
    <div className="page prefs">
      <div className="page-head">
        <div>
          <h1 className="page-title">{t('settings.title')}</h1>
          <p className="page-sub">{t('pref.sub')}</p>
        </div>
      </div>
      <div className="prefs-layout">
        <nav className="prefs-nav" aria-label={t('pref.sections')}>
          {SECTIONS.map((s) => (
            <button
              key={s.id}
              type="button"
              className="prefs-nav-link"
              onClick={() => document.getElementById(s.id)?.scrollIntoView({ block: 'start' })}
            >
              <Icon name={s.icon} size={16} />
              {t(s.label)}
            </button>
          ))}
          <Link to="/hilfe" className="prefs-nav-link">
            <Icon name="info" size={16} />
            {t('help.title')}
          </Link>
        </nav>
        <div className="prefs-main">
          <General />
          <Player />
          <Sources />
          <Torrents />
          <Tmdb />
          <Data />
          <About />
        </div>
      </div>
    </div>
  )
}

function General() {
  const { t, lang, setLang } = useI18n()
  const prefs = usePrefs()
  const navigate = useNavigate()
  return (
    <Section id="allgemein" title={t('pref.general')} icon="sliders">
      <PrefRow label={t('settings.language')} hint={t('settings.languageHint')} htmlFor="pref-lang">
        <span className="select">
          <select id="pref-lang" value={lang} onChange={(e) => setLang(e.target.value as typeof lang)}>
            {LANGUAGES.map((l) => (
              <option key={l.id} value={l.id} lang={l.locale}>
                {l.name}
              </option>
            ))}
          </select>
          <Icon name="down" size={14} />
        </span>
      </PrefRow>
      <PrefRow label={t('pref.start')} hint={t('pref.startHint')}>
        <Segmented
          label={t('pref.start')}
          value={prefs.start}
          options={START_PAGES.map((p) => ({ id: p.id, label: t(`pref.start_${p.id}` as Key) }))}
          onChange={(start) => setPrefs({ start })}
        />
      </PrefRow>
      <PrefRow label={t('pref.accent')}>
        <Segmented
          label={t('pref.accent')}
          value={prefs.accent}
          options={ACCENTS.map((a) => ({ id: a, label: t(`pref.accent_${a}` as Key), swatch: ACCENT_SWATCH[a] }))}
          onChange={(accent) => setPrefs({ accent })}
        />
      </PrefRow>
      <Toggle label={t('pref.motion')} hint={t('pref.motionHint')} checked={prefs.reduceMotion} onChange={(reduceMotion) => setPrefs({ reduceMotion })} />
      <div className="pref-actions">
        <button
          type="button"
          className="btn btn-glass btn-s"
          onClick={() => {
            navigate('/')
            window.setTimeout(startTour, 300)
          }}
        >
          <Icon name="play" size={15} />
          {t('pref.tour')}
        </button>
        <Link to="/einrichtung" className="btn btn-glass btn-s">
          <Icon name="refresh" size={15} />
          {t('pref.setup')}
        </Link>
        <Link to="/hilfe" className="btn btn-glass btn-s">
          <Icon name="info" size={15} />
          {t('help.title')}
        </Link>
      </div>
    </Section>
  )
}

/** Source and prefetch settings live on the server (shared with the player). */
function useScanSettings() {
  const qc = useQueryClient()
  const q = useQuery({ queryKey: ['settings'], queryFn: api.settings, staleTime: Infinity, retry: false })
  const m = useMutation({
    mutationFn: api.settingsPut,
    onMutate: (next) => qc.setQueryData(['settings'], next),
    onError: () => void qc.invalidateQueries({ queryKey: ['settings'] }),
  })
  const set = (patch: Partial<ScanSettings>) => q.data && m.mutate({ ...q.data, ...patch })
  return { data: q.data, set, error: m.error }
}

function trackOptions(t: (k: Key) => string, base: 'off' | 'auto') {
  return [
    { id: base, label: t(base === 'off' ? 'pref.subOff' : 'pref.audioAuto') },
    { id: 'ui', label: t('pref.sameAsApp') },
    ...LANGUAGES.map((l) => ({ id: l.id as string, label: l.name })),
  ]
}

function Player() {
  const { t } = useI18n()
  const prefs = usePrefs()
  const scan = useScanSettings()
  const minutes = [5, 10, 15, 20, 30, 60]
  return (
    <Section id="player" title={t('pref.player')} icon="play">
      <Toggle label={t('pref.autoNext')} hint={t('pref.autoNextHint')} checked={prefs.autoNext} onChange={(autoNext) => setPrefs({ autoNext })} />
      <PrefRow label={t('pref.countdown')} htmlFor="pref-countdown">
        <span className="select">
          <select
            id="pref-countdown"
            value={prefs.autoNextSeconds}
            disabled={!prefs.autoNext}
            onChange={(e) => setPrefs({ autoNextSeconds: Number(e.target.value) })}
          >
            {COUNTDOWNS.map((n) => (
              <option key={n} value={n}>
                {t('pref.seconds', { n })}
              </option>
            ))}
          </select>
          <Icon name="down" size={14} />
        </span>
      </PrefRow>
      <PrefRow label={t('pref.subLang')} hint={t('pref.subLangHint')} htmlFor="pref-sub">
        <span className="select">
          <select id="pref-sub" value={prefs.subLang} onChange={(e) => setPrefs({ subLang: e.target.value })}>
            {trackOptions(t, 'off').map((o) => (
              <option key={o.id} value={o.id}>
                {o.label}
              </option>
            ))}
          </select>
          <Icon name="down" size={14} />
        </span>
      </PrefRow>
      <PrefRow label={t('pref.audioLang')} hint={t('pref.audioLangHint')} htmlFor="pref-audio">
        <span className="select">
          <select id="pref-audio" value={prefs.audioLang} onChange={(e) => setPrefs({ audioLang: e.target.value })}>
            {trackOptions(t, 'auto').map((o) => (
              <option key={o.id} value={o.id}>
                {o.label}
              </option>
            ))}
          </select>
          <Icon name="down" size={14} />
        </span>
      </PrefRow>
      <Toggle
        label={t('pref.rememberVolume')}
        hint={t('pref.rememberVolumeHint')}
        checked={prefs.rememberVolume}
        onChange={(rememberVolume) => setPrefs({ rememberVolume })}
      />
      <PrefRow label={t('pref.volume')} htmlFor="pref-volume">
        <span className="pref-range">
          <input
            id="pref-volume"
            type="range"
            min={0}
            max={100}
            step={5}
            value={Math.round(prefs.volume * 100)}
            onChange={(e) => setPrefs({ volume: Number(e.target.value) / 100 })}
          />
          <output htmlFor="pref-volume">{Math.round(prefs.volume * 100)} %</output>
        </span>
      </PrefRow>
      <PrefRow label={t('scan.prefetch')} hint={t('pref.prefetchHint')} htmlFor="pref-prefetch">
        <span className="select">
          <select
            id="pref-prefetch"
            value={scan.data?.prefetch_minutes ?? 10}
            disabled={!scan.data}
            onChange={(e) => scan.set({ prefetch_minutes: Number(e.target.value) })}
          >
            {[...new Set([...minutes, scan.data?.prefetch_minutes ?? 10])]
              .sort((a, z) => a - z)
              .map((n) => (
                <option key={n} value={n}>
                  {t('pref.minutes', { n })}
                </option>
              ))}
          </select>
          <Icon name="down" size={14} />
        </span>
      </PrefRow>
      <Toggle
        label={t('pref.fullscreen')}
        hint={t('pref.fullscreenHint')}
        checked={prefs.autoFullscreen}
        onChange={(autoFullscreen) => setPrefs({ autoFullscreen })}
      />
    </Section>
  )
}

function Sources() {
  const { t } = useI18n()
  const prefs = usePrefs()
  const scan = useScanSettings()
  const f = scan.data?.filter
  return (
    <Section id="quellen" title={t('pref.sources')} icon="layers">
      <p className="field-hint">{t('pref.sourcesIntro')}</p>
      {f && scan.data && (
        <>
          <PrefRow label={t('scan.minQuality')}>
            <Segmented<MinHeight>
              label={t('scan.minQuality')}
              value={f.min_height}
              options={([0, 480, 720, 1080, 2160] as MinHeight[]).map((h) => ({
                id: h,
                label: h === 0 ? t('scan.any') : h === 2160 ? t('scan.only4k') : t('scan.from', { h: `${h}p` }),
              }))}
              onChange={(min_height) => scan.set({ filter: { ...f, min_height } })}
            />
          </PrefRow>
          <PrefRow label={t('scan.audio')}>
            <Segmented<AudioLang>
              label={t('scan.audio')}
              value={f.audio_lang}
              options={[
                { id: 'any', label: t('scan.any') },
                { id: 'de', label: t('lang.de') },
                { id: 'en', label: t('lang.en') },
              ]}
              onChange={(audio_lang) => scan.set({ filter: { ...f, audio_lang } })}
            />
          </PrefRow>
          <PrefRow label={t('scan.tie')}>
            <Segmented<SortKey>
              label={t('scan.tie')}
              value={f.sort}
              options={[
                { id: 'quality', label: t('scan.sortQuality') },
                { id: 'bitrate', label: t('scan.sortBitrate') },
                { id: 'speed', label: t('scan.sortSpeed') },
              ]}
              onChange={(sort) => scan.set({ filter: { ...f, sort } })}
            />
          </PrefRow>
          <PrefRow label={t('pref.parallel')} hint={t('pref.parallelHint')} htmlFor="pref-parallel">
            <span className="pref-range">
              <input
                id="pref-parallel"
                type="range"
                min={4}
                max={12}
                step={1}
                value={scan.data.parallel}
                onChange={(e) => scan.set({ parallel: Number(e.target.value) })}
              />
              <output htmlFor="pref-parallel">{scan.data.parallel}</output>
            </span>
          </PrefRow>
        </>
      )}
      <Toggle label={t('pref.hideFailed')} hint={t('pref.hideFailedHint')} checked={prefs.hideFailed} onChange={(hideFailed) => setPrefs({ hideFailed })} />
      {scan.error && <p className="field-error">{t('set.saveFailed')}</p>}
    </Section>
  )
}

function Torrents() {
  const { t } = useI18n()
  const qc = useQueryClient()
  const vpn = useTorrents().data?.vpn
  const saved = useQuery({ queryKey: ['torrent-settings'], queryFn: torrentApi.settings, retry: false })
  const [draft, setDraft] = useState<TorrentSettings | null>(null)
  const [confirm, setConfirm] = useState(false)
  const form = draft ?? saved.data
  const save = useMutation({
    mutationFn: torrentApi.settingsPut,
    onSuccess: (_v, next) => {
      qc.setQueryData(['torrent-settings'], next)
      setDraft(null)
      refreshTorrents(qc)
    },
  })
  const remove = useMutation({
    mutationFn: setupApi.vpnRemove,
    onSuccess: () => {
      setConfirm(false)
      void qc.invalidateQueries({ queryKey: ['setup'] })
      refreshTorrents(qc)
    },
  })
  const set = (patch: Partial<TorrentSettings>) => form && (save.reset(), setDraft({ ...form, ...patch }))

  return (
    <Section id="torrents" title={t('pref.torrents')} icon="download">
      <p className="pref-note">
        <Icon name="shield" size={16} />
        {t('pref.vpnOnly')}
      </p>
      <div className="pref-row">
        <div className="pref-text">
          <span className="pref-label">{t('set.vpnFile')}</span>
          <span className="field-hint">{t('set.vpnFileHint')}</span>
        </div>
        <VpnBadge vpn={vpn} always />
      </div>
      <VpnImport has={!!vpn?.config_found} onChange={() => refreshTorrents(qc)} />
      {vpn?.config_found && (
        <div className="pref-actions">
          {!confirm ? (
            <button type="button" className="btn btn-glass btn-s" onClick={() => setConfirm(true)}>
              <Icon name="trash" size={15} />
              {t('pref.vpnRemove')}
            </button>
          ) : (
            <>
              <span className="pref-confirm">{t('pref.vpnRemoveAsk')}</span>
              <button type="button" className="btn btn-danger btn-s" onClick={() => remove.mutate()} disabled={remove.isPending}>
                {t('pref.vpnRemoveYes')}
              </button>
              <button type="button" className="btn btn-glass btn-s" onClick={() => setConfirm(false)}>
                {t('common.cancel')}
              </button>
            </>
          )}
        </div>
      )}
      {form && (
        <form
          className="pref-form"
          onSubmit={(e) => {
            e.preventDefault()
            if (draft) save.mutate(draft)
          }}
        >
          <label className="field">
            <span className="field-label">{t('set.moviesDir')}</span>
            <input className="input" value={form.movies_dir} onChange={(e) => set({ movies_dir: e.target.value })} spellCheck={false} />
          </label>
          <label className="field">
            <span className="field-label">{t('set.seriesDir')}</span>
            <input className="input" value={form.series_dir} onChange={(e) => set({ series_dir: e.target.value })} spellCheck={false} />
          </label>
          <label className="field set-upload">
            <span className="field-label">{t('set.upload')}</span>
            <span className="set-unit">
              <input
                className="input"
                type="number"
                min={1}
                max={1000}
                value={form.upload_kbps}
                onChange={(e) => set({ upload_kbps: Math.max(1, Math.min(1000, Math.round(Number(e.target.value) || 1))) })}
              />
              <span>KB/s</span>
            </span>
            <span className="field-hint">{t('set.uploadHint')}</span>
          </label>
          <div className="set-actions">
            {save.isError && (
              <p className="field-error" role="alert">
                {save.error instanceof ApiError && /absolute/.test(save.error.message) ? t('ob.folderAbsolute') : t('set.saveFailed')}
              </p>
            )}
            {save.isSuccess && !draft && (
              <p className="set-saved" role="status">
                <Icon name="check" size={15} stroke={2.2} />
                {t('set.saved')}
              </p>
            )}
            <button type="button" className="btn btn-glass" disabled={!draft} onClick={() => (setDraft(null), save.reset())}>
              {t('set.reset')}
            </button>
            <button type="submit" className="btn btn-primary" disabled={!draft || save.isPending}>
              {t('set.save')}
            </button>
          </div>
        </form>
      )}
    </Section>
  )
}

function Tmdb() {
  const { t } = useI18n()
  const setup = useSetup()
  const [replace, setReplace] = useState(false)
  const has = setup.data?.tmdb === 'set'
  return (
    <Section id="tmdb" title={t('pref.tmdb')} icon="film">
      <p className="field-hint">{t('pref.tmdbIntro')}</p>
      {has && !replace ? (
        <div className="ob-saved">
          <span className="token-status is-ok">
            <Icon name="check" size={16} stroke={2.4} />
            {t('ob.tokenStored')}
          </span>
          <button type="button" className="link-btn" onClick={() => setReplace(true)}>
            {t('ob.tokenReplace')}
          </button>
        </div>
      ) : (
        <>
          <TokenField onSaved={() => setReplace(false)} />
          <OpenPage page="tmdb_api" label={t('ob.tmdbApiPage')} quiet />
        </>
      )}
    </Section>
  )
}

/** Two-step button for actions that delete something. */
function Confirmed({ label, ask, yes, icon, run, busy }: { label: string; ask: string; yes: string; icon: IconName; run: () => void; busy?: boolean }) {
  const { t } = useI18n()
  const [open, setOpen] = useState(false)
  if (!open)
    return (
      <button type="button" className="btn btn-glass btn-s" onClick={() => setOpen(true)}>
        <Icon name={icon} size={15} />
        {label}
      </button>
    )
  return (
    <span className="pref-confirm-row" role="group" aria-label={ask}>
      <span className="pref-confirm">{ask}</span>
      <button
        type="button"
        className="btn btn-danger btn-s"
        disabled={busy}
        autoFocus
        onClick={() => {
          run()
          setOpen(false)
        }}
      >
        {yes}
      </button>
      <button type="button" className="btn btn-glass btn-s" onClick={() => setOpen(false)}>
        {t('common.cancel')}
      </button>
    </span>
  )
}

function Data() {
  const { t } = useI18n()
  const qc = useQueryClient()
  const navigate = useNavigate()
  const sizes = useQuery({ queryKey: ['maintenance'], queryFn: setupApi.maintenance })
  const [note, setNote] = useState<{ ok: boolean; text: string } | null>(null)
  const file = useRef<HTMLInputElement>(null)

  const say = (ok: boolean, text: string) => setNote({ ok, text })
  const clearHistory = useMutation({
    mutationFn: api.historyClear,
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ['history'] })
      say(true, t('pref.historyCleared'))
    },
    onError: () => say(false, t('pref.failed')),
  })
  const clearCache = useMutation({
    mutationFn: setupApi.clearCache,
    onSuccess: (r) => {
      void sizes.refetch()
      say(true, r.scan_profile_pending ? t('pref.cacheClearedRestart') : t('pref.cacheCleared'))
    },
    onError: () => say(false, t('pref.failed')),
  })
  const exportIt = useMutation({
    mutationFn: setupApi.exportFile,
    onSuccess: (r) => say(true, t('pref.exported', { path: r.path })),
    onError: () => say(false, t('pref.failed')),
  })
  const importIt = useMutation({
    mutationFn: async (f: File) => setupApi.importFile(await readTextFile(f, 2 * 1024 * 1024)),
    onSuccess: async (r) => {
      await loadPrefs()
      await qc.invalidateQueries()
      say(true, t('pref.imported', { n: r.watchlist }))
    },
    onError: () => say(false, t('pref.importBad')),
  })
  const reset = useMutation({
    mutationFn: setupApi.reset,
    onSuccess: async () => {
      resetLocalPrefs()
      try {
        localStorage.removeItem('flimmer.volume')
      } catch {
        /* storage blocked */
      }
      qc.clear()
      navigate('/', { replace: true })
      await qc.invalidateQueries()
    },
    onError: () => say(false, t('pref.failed')),
  })

  const cacheBytes = (sizes.data?.stream_cache_bytes ?? 0) + (sizes.data?.scan_profile_bytes ?? 0)
  return (
    <Section id="daten" title={t('pref.data')} icon="folder">
      <PrefRow label={t('pref.history')} hint={t('pref.historyHint')}>
        <Confirmed
          label={t('pref.historyClear')}
          ask={t('pref.historyAsk')}
          yes={t('pref.historyYes')}
          icon="trash"
          busy={clearHistory.isPending}
          run={() => clearHistory.mutate()}
        />
      </PrefRow>
      <PrefRow
        label={t('pref.cache')}
        hint={
          sizes.data
            ? t('pref.cacheSizes', { stream: formatBytes(sizes.data.stream_cache_bytes), scan: formatBytes(sizes.data.scan_profile_bytes) })
            : t('common.loading')
        }
      >
        <button type="button" className="btn btn-glass btn-s" disabled={clearCache.isPending || cacheBytes === 0} onClick={() => clearCache.mutate()}>
          <Icon name="refresh" size={15} />
          {t('pref.cacheClear', { size: formatBytes(cacheBytes) })}
        </button>
      </PrefRow>
      <PrefRow label={t('pref.backup')} hint={t('pref.backupHint')}>
        <span className="pref-actions">
          <button type="button" className="btn btn-glass btn-s" disabled={exportIt.isPending} onClick={() => exportIt.mutate()}>
            <Icon name="download" size={15} />
            {t('pref.export')}
          </button>
          <button type="button" className="btn btn-glass btn-s" disabled={importIt.isPending} onClick={() => file.current?.click()}>
            <Icon name="folder" size={15} />
            {t('pref.import')}
          </button>
          <input
            ref={file}
            type="file"
            accept=".json,application/json"
            className="sr-only"
            tabIndex={-1}
            aria-hidden="true"
            onChange={(e) => {
              const f = e.target.files?.[0]
              e.target.value = ''
              if (f) importIt.mutate(f)
            }}
          />
        </span>
      </PrefRow>
      <PrefRow label={t('pref.reset')} hint={t('pref.resetHint')}>
        <Confirmed label={t('pref.resetBtn')} ask={t('pref.resetAsk')} yes={t('pref.resetYes')} icon="warn" busy={reset.isPending} run={() => reset.mutate()} />
      </PrefRow>
      {note && (
        <p className={note.ok ? 'set-saved' : 'field-error'} role={note.ok ? 'status' : 'alert'}>
          {note.ok && <Icon name="check" size={15} stroke={2.2} />}
          {note.text}
        </p>
      )}
      {sizes.data && <p className="field-hint pref-path">{t('pref.dataDir', { path: sizes.data.data_dir })}</p>}
    </Section>
  )
}

const LICENSES: { name: string; license: string; page: Page }[] = [
  { name: 'FFmpeg (gyan.dev essentials build)', license: 'GPL-3.0', page: 'ffmpeg' },
  { name: 'uBlock Origin Lite', license: 'GPL-3.0', page: 'ubol' },
  { name: 'librqbit', license: 'Apache-2.0', page: 'librqbit' },
  { name: 'BoringTun', license: 'BSD-3-Clause', page: 'boringtun' },
  { name: 'hls.js', license: 'Apache-2.0', page: 'hlsjs' },
  { name: 'tork', license: 'MIT', page: 'tork' },
]

function About() {
  const { t } = useI18n()
  const setup = useSetup()
  return (
    <Section id="info" title={t('pref.about')} icon="info">
      <p className="pref-version">
        Flimmer <strong>{setup.data?.version ?? '…'}</strong>
      </p>
      <p className="field-hint">{t('pref.tmdbNotice')}</p>
      <OpenPage page="tmdb" label="themoviedb.org" quiet />
      <h3 className="field-label">{t('pref.licenses')}</h3>
      <ul className="pref-licenses">
        {LICENSES.map((l) => (
          <li key={l.name}>
            <span>
              {l.name} <span className="pref-license">{l.license}</span>
            </span>
            <OpenPage page={l.page} label={t('pref.source')} quiet />
          </li>
        ))}
      </ul>
      <p className="field-hint">{t('pref.noticesFile')}</p>
    </Section>
  )
}
