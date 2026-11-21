import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from 'react-router'
import { useI18n } from '../i18n'
import { titlePath, type Item } from '../lib/api'
import { codeText } from '../lib/errors'
import { formatDate } from '../lib/format'
import { formatBytes, torrentApi, type AutoRule, type AutoRuleBody, type Decision } from '../lib/torrents'
import { useWatchlist } from '../lib/watchlist'
import { Segmented, Toggle } from './Controls'
import Icon from './Icon'
import { EmptyState, ErrorState, PageLoader } from './States'

const HEIGHTS = [0, 720, 1080, 2160] as const
/** Today as YYYY-MM-DD in the user's time zone (toISOString would give the UTC day). */
const localToday = () => new Date(Date.now() - new Date().getTimezoneOffset() * 60_000).toISOString().slice(0, 10)
const SOURCES = ['Remux', 'BluRay', 'WEB-DL', 'WEBRip', 'HDTV']

type Target = { type: Item['type']; id: number; title: string; year: number | null; poster: string | null }

function draftOf(rule: AutoRule | undefined, x: Target): AutoRuleBody {
  if (rule) {
    const { type: _t, id: _i, created_at: _c, on_watchlist: _w, ...body } = rule
    return body
  }
  return { title: x.title, year: x.year, enabled: true, min_height: 1080, max_height: 0, lang: 'any', sources: [], max_size_gb: 0, min_seeders: 3 }
}

/**
 * Auto-download per title of the watchlist: what quality, language, source and size, on or off. "Vorschau" asks
 * the server what the rule would take right now (stores and queues nothing); "Speichern" makes it real.
 */
export default function AutoRules() {
  const { t } = useI18n()
  const watch = useWatchlist()
  const rules = useQuery({ queryKey: ['autodl-rules'], queryFn: torrentApi.rules, retry: false })
  if (watch.error || rules.error) return <ErrorState error={watch.error ?? rules.error} onRetry={() => (void watch.refetch(), void rules.refetch())} />
  if (!watch.data || !rules.data) return <PageLoader />
  const byKey = new Map(rules.data.rules.map((r) => [`${r.type}:${r.id}`, r]))
  const targets: Target[] = watch.data
    .filter((x) => x.type === 'tv')
    .map((x) => ({ type: x.type, id: x.id, title: x.title, year: x.year, poster: x.poster }))
  // Rules whose title left the watchlist (or movies) stay visible: they do not run, but they can be removed.
  for (const r of rules.data.rules) {
    if (!targets.some((x) => x.type === r.type && x.id === r.id)) targets.push({ type: r.type, id: r.id, title: r.title, year: r.year, poster: null })
  }
  return (
    <div className="ar">
      <p className="field-hint ar-intro">
        {t('ar.intro', { n: rules.data.interval_minutes })}
        {rules.data.last_run && ` ${t('ar.lastRun', { when: formatDate(rules.data.last_run) ?? '' })}`}
      </p>
      {targets.length === 0 ? (
        <EmptyState icon="clock" title={t('ar.emptyTitle')} text={t('ar.emptyText')}>
          <Link to="/serien" className="btn btn-primary">
            <Icon name="tv" size={17} />
            {t('nav.tv')}
          </Link>
        </EmptyState>
      ) : (
        <ul className="ar-list">
          {targets.map((x) => (
            <RuleCard key={`${x.type}:${x.id}`} target={x} rule={byKey.get(`${x.type}:${x.id}`)} />
          ))}
        </ul>
      )}
      <History />
    </div>
  )
}

function RuleCard({ target, rule }: { target: Target; rule: AutoRule | undefined }) {
  const { t } = useI18n()
  const qc = useQueryClient()
  const [open, setOpen] = useState(false)
  const [draft, setDraft] = useState<AutoRuleBody>(() => draftOf(rule, target))
  const set = (patch: Partial<AutoRuleBody>) => {
    preview.reset()
    setDraft((d) => ({ ...d, ...patch }))
  }
  const refresh = () => void qc.invalidateQueries({ queryKey: ['autodl-rules'] })
  const preview = useMutation({ mutationFn: () => torrentApi.rulePreview(target.type, target.id, draft) })
  const save = useMutation({ mutationFn: () => torrentApi.rulePut(target.type, target.id, draft), onSuccess: refresh })
  const remove = useMutation({
    mutationFn: () => torrentApi.ruleDelete(target.type, target.id),
    onSuccess: () => (setDraft(draftOf(undefined, target)), refresh()),
  })
  const offWatchlist = rule && !rule.on_watchlist
  const status = !rule ? t('ar.none') : !rule.enabled ? t('ar.off') : offWatchlist ? t('ar.notWatched') : t('ar.on')
  const id = `ar-${target.type}-${target.id}`
  return (
    <li className={`ar-card${rule?.enabled && !offWatchlist ? ' is-on' : ''}`}>
      <div className="ar-head">
        {target.poster ? <img className="ar-poster" src={target.poster} alt="" loading="lazy" /> : <span className="ar-poster" aria-hidden="true" />}
        <div className="ar-name">
          <Link to={titlePath(target)} className="ar-title">
            {target.title}
            {target.year && <span className="ar-year"> ({target.year})</span>}
          </Link>
          <span className={`tag${rule?.enabled && !offWatchlist ? '' : ' tag-dim'}`}>{status}</span>
        </div>
        <button type="button" className="btn btn-glass btn-s" aria-expanded={open} aria-controls={id} onClick={() => setOpen((o) => !o)}>
          <Icon name="sliders" size={15} />
          {rule ? t('ar.edit') : t('ar.make')}
        </button>
      </div>
      {open && (
        <div className="ar-body" id={id}>
          <Toggle label={t('ar.enabled')} hint={t('ar.enabledHint')} checked={draft.enabled} onChange={(enabled) => set({ enabled })} />
          <div className="ar-field">
            <span className="field-label">{t('tf.quality')}</span>
            <Segmented<number>
              label={t('tf.quality')}
              value={draft.min_height}
              options={HEIGHTS.map((h) => ({ id: h, label: h === 0 ? t('scan.any') : h === 2160 ? '4K' : t('scan.from', { h: `${h}p` }) }))}
              onChange={(h) => set({ min_height: h as AutoRuleBody['min_height'] })}
            />
          </div>
          <div className="ar-field">
            <span className="field-label">{t('tf.language')}</span>
            <Segmented<string>
              label={t('tf.language')}
              value={draft.lang}
              options={[
                { id: 'any', label: t('scan.any') },
                { id: 'de', label: t('lang.de') },
                { id: 'en', label: t('lang.en') },
              ]}
              onChange={(lang) => set({ lang })}
            />
          </div>
          <div className="ar-field">
            <span className="field-label">{t('tf.source')}</span>
            <div className="tf-chips">
              {SOURCES.map((s) => (
                <button
                  key={s}
                  type="button"
                  className="tf-chip"
                  aria-pressed={draft.sources.includes(s)}
                  onClick={() => set({ sources: draft.sources.includes(s) ? draft.sources.filter((x) => x !== s) : [...draft.sources, s] })}
                >
                  {s}
                </button>
              ))}
            </div>
            <span className="field-hint">{draft.sources.length === 0 ? t('ar.anySource') : t('ar.onlySources')}</span>
          </div>
          {target.type === 'tv' && (
            <label className="ar-field">
              <span className="field-label">{t('ar.since')}</span>
              <input
                className="input ar-date"
                type="date"
                value={draft.since ?? localToday()}
                max={localToday()}
                onChange={(e) => set({ since: e.target.value || undefined })}
              />
              <span className="field-hint">{t('ar.sinceHint')}</span>
            </label>
          )}
          <label className="ar-field">
            <span className="field-label">{t('ar.maxSize')}</span>
            <span className="set-unit">
              <input
                className="input"
                type="number"
                min={0}
                max={200}
                step={0.5}
                value={draft.max_size_gb}
                onChange={(e) => set({ max_size_gb: Math.max(0, Math.min(200, Number(e.target.value) || 0)) })}
              />
              <span>GB</span>
            </span>
            <span className="field-hint">{t('ar.maxSizeHint')}</span>
          </label>
          <div className="ar-actions">
            <button type="button" className="btn btn-glass" onClick={() => preview.mutate()} disabled={preview.isPending}>
              {preview.isPending ? <span className="bars" aria-hidden="true" /> : <Icon name="search" size={16} />}
              {t('ar.preview')}
            </button>
            <button type="button" className="btn btn-primary" onClick={() => save.mutate()} disabled={save.isPending}>
              <Icon name="check" size={16} />
              {t('ar.save')}
            </button>
            {rule && (
              <button type="button" className="btn btn-glass" onClick={() => remove.mutate()} disabled={remove.isPending}>
                <Icon name="trash" size={16} />
                {t('ar.delete')}
              </button>
            )}
            {save.isSuccess && !save.isPending && (
              <span className="set-saved" role="status">
                <Icon name="check" size={15} stroke={2.2} />
                {t('set.saved')}
              </span>
            )}
          </div>
          {(save.isError || remove.isError || preview.isError) && <p className="field-error">{t('ar.failed')}</p>}
          {preview.data && <Preview decisions={preview.data.decisions} />}
        </div>
      )}
    </li>
  )
}

/** What the rule would take now; nothing of it is queued. */
function Preview({ decisions }: { decisions: Decision[] }) {
  const { t } = useI18n()
  const grabs = decisions.filter((d) => d.action === 'grab').length
  return (
    <section className="ar-preview" aria-label={t('ar.preview')}>
      <p className="ar-preview-head">
        <Icon name="info" size={15} />
        {decisions.length === 0 ? t('ar.previewNone') : t('ar.previewSum', { n: grabs })}
      </p>
      <ul>
        {decisions.map((d, i) => (
          <li key={`${d.season}-${d.episode}-${i}`} className={`ar-dec is-${d.action}`}>
            <span className="ar-dec-what">
              {d.season !== null && d.episode !== null ? `S${String(d.season).padStart(2, '0')}E${String(d.episode).padStart(2, '0')}` : d.title}
              {d.air_date && <span className="ar-dec-date">{formatDate(d.air_date)}</span>}
            </span>
            {d.candidate ? (
              <span className="ar-dec-pick">
                <span className="ar-dec-name" title={d.candidate.name}>
                  {d.candidate.name}
                </span>
                <span className="ar-dec-facts">
                  {formatBytes(d.candidate.size)} · {t('tor.sourcesShort', { n: d.candidate.seeders })} · {t('ar.found', { n: d.found })}
                </span>
              </span>
            ) : (
              <span className="ar-dec-why">{d.reason_code && d.reason_code !== 'nothing_fits' ? codeText(d.reason_code) : t('ar.nothingFits', { n: d.found })}</span>
            )}
            <span className="tag tag-dim">{d.action === 'grab' ? t('ar.wouldGrab') : d.action === 'skip' ? t('ar.skip') : t('ar.noneFits')}</span>
          </li>
        ))}
      </ul>
    </section>
  )
}

function History() {
  const { t } = useI18n()
  const qc = useQueryClient()
  const grabs = useQuery({ queryKey: ['autodl-history'], queryFn: torrentApi.grabs, retry: false })
  const remove = useMutation({ mutationFn: torrentApi.grabDelete, onSettled: () => void qc.invalidateQueries({ queryKey: ['autodl-history'] }) })
  const list = grabs.data ?? []
  return (
    <section className="ar-history" aria-labelledby="ar-history-h">
      <h2 className="dl-section" id="ar-history-h">
        {t('ar.history')}
      </h2>
      {list.length === 0 ? (
        <p className="field-hint">{t('ar.historyEmpty')}</p>
      ) : (
        <ul className="ar-grabs">
          {list.map((g) => (
            <li key={g.key}>
              <span className="ar-grab-title">
                {g.title}
                {g.season !== null && g.episode !== null && ` · S${String(g.season).padStart(2, '0')}E${String(g.episode).padStart(2, '0')}`}
              </span>
              <span className="ar-grab-name" title={g.name}>
                {g.name}
              </span>
              <span className="ar-grab-facts">
                {formatBytes(g.size)} · {formatDate(g.at)}
              </span>
              <button type="button" className="icon-btn" onClick={() => remove.mutate(g.key)} aria-label={t('ar.forget', { title: g.title })} title={t('ar.forgetShort')}>
                <Icon name="trash" size={15} />
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}
