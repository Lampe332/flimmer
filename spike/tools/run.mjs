// Tests 2-4 in one loop, through the real Flimmer UI with the iframe sandbox switched off.
//
//   node tools/run.mjs <mode: adblock|none> <title: movie|tv> [provider ...]
//
// Per provider: load /play/... in the spike window, count blocked requests / popup attempts /
// blocked top navigations, capture the first playlist, check that the provider's own player
// plays, parse the master playlist (via the local proxy with the captured headers) and finally
// play the same stream in spike/player with hls.js through the proxy.
import { spawn } from 'node:child_process'
import { join, resolve } from 'node:path'
import { launch, CDP, ROOT, PROXY, sleep, readLog, saveJson, mark, section, proxyUrl } from './lib.mjs'

const [mode = 'adblock', title = 'movie', ...onlyProviders] = process.argv.slice(2)
const PROVIDERS = ['cinezo', 'vidlink', 'videasy', 'vidfast', 'vixsrc', '111movies'].filter(p => !onlyProviders.length || onlyProviders.includes(p))
const FLIMMER = 'http://localhost:43300'
const PLAY = title === 'movie' ? `${FLIMMER}/play/movie/278` : `${FLIMMER}/play/tv/84773?s=1&e=1`
const SOURCES = title === 'movie' ? `${FLIMMER}/api/sources/movie/278` : `${FLIMMER}/api/sources/tv/84773?s=1&e=1`
const withPlayer = mode === 'adblock'
const tag = `${mode}-${title}`

// --- Flimmer backend: use a running instance, otherwise start one (and stop only that one) ---
let server = null
async function flimmerUp() { try { return (await fetch(`${FLIMMER}/api/sources/movie/278`, { signal: AbortSignal.timeout(3000) })).ok } catch { return false } }
if (!(await flimmerUp())) {
  const flimmerRoot = resolve(ROOT, '..')
  server = spawn(join(flimmerRoot, 'server', 'target', 'release', 'flimmer.exe'), ['--root', flimmerRoot], { stdio: 'ignore' })
  for (let i = 0; i < 20 && !(await flimmerUp()); i++) await sleep(500)
  if (!(await flimmerUp())) { console.log('ERROR: Flimmer server did not start'); server.kill(); process.exit(1) }
}
const sources = await (await fetch(SOURCES)).json()

const VIDEO = `(() => { const out = []; const walk = w => { try {
  for (const v of w.document.querySelectorAll('video')) out.push({ t: +v.currentTime.toFixed(2), paused: v.paused, w: v.videoWidth, h: v.videoHeight, rs: v.readyState, dur: Math.round(v.duration || 0) })
  for (let i = 0; i < w.frames.length; i++) walk(w.frames[i]) } catch {} }; walk(window)
  return { videos: out, text: (document.body?.innerText ?? '').replace(/\\s+/g, ' ').slice(0, 140), host: location.host } })()`

/** Video elements of every frame (cross-origin iframes are separate CDP targets). */
async function videoStates() {
  const out = []
  for (const t of (await CDP.targets()).filter(t => ['page', 'iframe'].includes(t.type) && t.url.startsWith('http'))) {
    const c = new CDP()
    try { await c.open(t.webSocketDebuggerUrl); const r = await c.eval(VIDEO); out.push(r); } catch { /* target gone */ }
    c.close()
  }
  return out
}
const bestVideo = states => states.flatMap(s => s.videos).sort((a, b) => b.t - a.t)[0] ?? null

const attrs = line => Object.fromEntries([...line.matchAll(/([A-Z0-9-]+)=("[^"]*"|[^,]*)/g)].map(m => [m[1], m[2].replace(/^"|"$/g, '')]))
function parsePlaylist(text) {
  const lines = text.split(/\r?\n/)
  const variants = lines.filter(l => l.startsWith('#EXT-X-STREAM-INF:')).map(l => attrs(l.slice(18)))
  const media = lines.filter(l => l.startsWith('#EXT-X-MEDIA:')).map(l => attrs(l.slice(13)))
  const px = v => { const [w, h] = (v.RESOLUTION ?? '0x0').split('x').map(Number); return w * h }
  const top = [...variants].sort((a, b) => px(b) - px(a) || (+b.BANDWIDTH || 0) - (+a.BANDWIDTH || 0))[0]
  const lang = type => [...new Set(media.filter(m => m.TYPE === type).map(m => m.LANGUAGE || m.NAME || '?'))]
  return {
    isMaster: variants.length > 0,
    isMedia: lines.some(l => l.startsWith('#EXTINF')),
    variants: variants.map(v => ({ res: v.RESOLUTION ?? null, bw: +v.BANDWIDTH || null, codecs: v.CODECS ?? null })),
    maxResolution: top?.RESOLUTION ?? null,
    maxBandwidth: variants.length ? Math.max(...variants.map(v => +v.BANDWIDTH || 0)) : null,
    codecs: [...new Set(variants.map(v => v.CODECS).filter(Boolean))],
    audio: lang('AUDIO'), subtitles: lang('SUBTITLES'),
  }
}

const xattrs = s => Object.fromEntries([...s.matchAll(/([\w:-]+)="([^"]*)"/g)].map(m => [m[1], m[2]]))
function parseMpd(text) {
  const sets = [...text.matchAll(/<AdaptationSet\b([^>]*)>([\s\S]*?)<\/AdaptationSet>/g)].map(m => {
    const a = xattrs(m[1])
    const reps = [...m[2].matchAll(/<Representation\b([^>]*)/g)].map(r => xattrs(r[1]))
    const mime = a.mimeType ?? reps[0]?.mimeType ?? ''
    const kind = a.contentType ?? (mime.startsWith('video') || reps.some(r => r.width) ? 'video' : mime.startsWith('audio') ? 'audio' : 'text')
    return { kind, lang: a.lang ?? null, codecs: a.codecs, reps }
  })
  const video = sets.filter(s => s.kind === 'video').flatMap(s => s.reps.map(r => ({ res: r.width ? `${r.width}x${r.height}` : null, bw: +r.bandwidth || null, codecs: r.codecs ?? s.codecs ?? null, px: (+r.width || 0) * (+r.height || 0) })))
  const top = [...video].sort((a, b) => b.px - a.px || b.bw - a.bw)[0]
  const langs = kind => [...new Set(sets.filter(s => s.kind === kind).map(s => s.lang || '?'))]
  return {
    format: 'dash', isMaster: video.length > 0, isMedia: false,
    variants: video.map(({ px, ...v }) => v), maxResolution: top?.res ?? null,
    maxBandwidth: video.length ? Math.max(...video.map(v => v.bw || 0)) : null,
    codecs: [...new Set(video.map(v => v.codecs).filter(Boolean))], audio: langs('audio'), subtitles: langs('text'),
  }
}
const hex = s => Buffer.from(s, 'utf8').toString('hex')

const app = await launch({ mode, name: `run-${tag}`, lockOrigin: true, url: `${FLIMMER}/` })
const results = []
try {
  const { cdp } = app
  await sleep(1500)
  const ua = await cdp.eval('navigator.userAgent')
  const guardOff = JSON.stringify(Object.fromEntries(sources.map(s => [s.id, false])))

  for (const id of PROVIDERS) {
    const src = sources.find(s => s.id === id)
    const r = { provider: id, title, mode, embed: src?.url ?? null }
    results.push(r)
    if (!src) { r.fail = 'provider not in /api/sources'; continue }
    const label = `${tag}-${id}`
    try {
      await cdp.navigate(`${FLIMMER}/`)
      await sleep(800)
      await cdp.eval(`localStorage.setItem('flimmer.popupGuard', ${JSON.stringify(guardOff)}); localStorage.setItem('flimmer.lastSource', ${JSON.stringify(id)})`)
      await mark(label)
      const t0 = Date.now()
      await cdp.navigate(PLAY)
      const events = () => section(readLog(app.log), label)
      const mediaEvents = () => events().filter(e => e.kind === 'media_resp' || e.kind === 'media_req')
      const isPlaylistEv = e => /mpegurl|dash\+xml/i.test(e.ctype ?? '') || /\.m3u8|\.mpd(\?|$)/i.test(e.url)
      const iframeCenter = async () => cdp.eval(`(() => { const f = document.querySelector('iframe'); if (!f) return null; const b = f.getBoundingClientRect(); return { x: Math.round(b.x + b.width / 2), y: Math.round(b.y + b.height / 2), sandbox: f.getAttribute('sandbox'), src: f.src } })()`)

      // wait for a playlist; click the player centre if nothing shows up within ~8 s (max 3 clicks)
      let clicks = 0
      for (let i = 0; i < 38; i++) {
        await sleep(1000)
        if (mediaEvents().some(isPlaylistEv)) break
        if (i === 7 || i === 15 || i === 23) {
          const c = await iframeCenter().catch(() => null)
          if (c) { r.iframe = { sandbox: c.sandbox, src: c.src }; await cdp.click(c.x, c.y); clicks++ }
        }
      }
      const c0 = await iframeCenter().catch(() => null)
      if (c0) r.iframe = { sandbox: c0.sandbox, src: c0.src }
      r.clicks = clicks

      // does the provider's own player play? (one more click if a video exists but is paused)
      await sleep(4000)
      let v1 = bestVideo(await videoStates())
      if ((!v1 || v1.paused) && c0 && clicks < 4) { await cdp.click(c0.x, c0.y); r.clicks = ++clicks; await sleep(3000); v1 = bestVideo(await videoStates()) }
      await sleep(5000)
      const states = await videoStates()
      const v2 = bestVideo(states)
      r.providerPlayer = { t1: v1?.t ?? null, t2: v2?.t ?? null, playing: !!(v1 && v2 && v2.t - v1.t > 2), resolution: v2 ? `${v2.w}x${v2.h}` : null, duration: v2?.dur ?? null }
      r.frameTexts = states.map(s => `${s.host}: ${s.text}`).slice(0, 4)
      r.shotProvider = await cdp.shot(`t2-${label}.png`)

      // numbers from the Rust hooks
      const ev = events()
      const reqs = ev.filter(e => e.kind === 'req')
      const tMark = readLog(app.log).find(e => e.kind === 'mark' && e.name === label)?.t ?? 0
      const blockedHosts = {}
      for (const e of reqs.filter(e => e.blocked)) { const h = new URL(e.url).host; blockedHosts[h] = (blockedHosts[h] ?? 0) + 1 }
      r.requests = reqs.length
      r.blocked = reqs.filter(e => e.blocked).length
      r.blockedHosts = Object.entries(blockedHosts).sort((a, b) => b[1] - a[1]).slice(0, 8).map(([h, n]) => `${h} (${n})`)
      r.popupsBlocked = ev.filter(e => e.kind === 'popup_blocked').length
      r.popupHosts = [...new Set(ev.filter(e => e.kind === 'popup_blocked').map(e => { try { return new URL(e.url).host } catch { return e.url.slice(0, 30) } }))]
      r.topNavBlocked = ev.filter(e => e.kind === 'nav_blocked').length
      r.frames = [...new Set(ev.filter(e => e.kind === 'frame_nav').map(e => new URL(e.url).host))]

      // --- Test 3: captured media -> master playlist ---
      const media = mediaEvents()
      const firstPl = media.find(isPlaylistEv)
      r.manifestKind = firstPl ? (/mpd|dash/i.test((firstPl.ctype ?? '') + firstPl.url.split('?')[0]) ? 'dash' : 'hls') : null
      r.timeToFirstM3u8Ms = firstPl ? firstPl.t - tMark : null
      r.mediaSeen = [...new Set(media.map(e => `${e.kind === 'media_resp' ? e.status + ' ' + (e.ctype || '?') : 'req'} ${new URL(e.url).host}${new URL(e.url).pathname.slice(0, 50)}`))].slice(0, 8)
      const candidates = [...new Map(media.filter(isPlaylistEv).map(e => [e.url, e])).values()].slice(0, 8)
      const frameOrigins = ev.filter(e => e.kind === 'frame_nav').map(e => new URL(e.url).origin)
      let chosen = null
      for (const e of candidates) {
        const h = e.headers ?? {}
        const get = k => Object.entries(h).find(([n]) => n.toLowerCase() === k)?.[1]
        const tries = [{ referer: get('referer') ?? '', origin: get('origin') ?? '' }]
        if (!get('referer')) for (const o of [...new Set(frameOrigins)]) tries.push({ referer: o + '/', origin: o })
        for (const hdr of tries) {
          const resp = await fetch(proxyUrl(e.url, { ...hdr, ua }, true), { signal: AbortSignal.timeout(30000) }).catch(err => ({ ok: false, status: 0, text: async () => String(err) }))
          const body = await resp.text()
          const info = { url: e.url, ...hdr, status: resp.status, capturedHeaders: Object.keys(h) }
          const isMpd = body.slice(0, 600).includes('<MPD')
          if (resp.ok && (isMpd || body.trimStart().startsWith('#EXTM3U'))) {
            const p = isMpd ? parseMpd(body) : parsePlaylist(body)
            if (!chosen || (p.isMaster && !chosen.playlist.isMaster)) chosen = { ...info, playlist: p }
            break
          } else if (!r.playlistFetchError) r.playlistFetchError = `HTTP ${resp.status} via proxy for ${new URL(e.url).host} (referer ${hdr.referer || 'none'}): ${body.slice(0, 80)}`
        }
        if (chosen?.playlist.isMaster) break
      }
      if (chosen) {
        r.master = { host: new URL(chosen.url).host, referer: chosen.referer, origin: chosen.origin, capturedHeaders: chosen.capturedHeaders, ...chosen.playlist }
        r.masterUrl = chosen.url
      } else if (!firstPl) {
        const mp4 = media.find(e => /video\/mp4/i.test(e.ctype ?? '') || /\.mp4/i.test(e.url))
        r.fail = mp4 ? `no HLS playlist, mp4 stream from ${new URL(mp4.url).host}` : `no m3u8/mp4 request within ${Math.round((Date.now() - t0) / 1000)} s`
      } else r.fail = r.playlistFetchError ?? 'playlist captured but not fetchable'

      // --- Test 4: own hls.js player through the local proxy ---
      if (withPlayer && chosen) {
        const pl = `${label}-player`
        await mark(pl)
        const dash = chosen.playlist.format === 'dash'
        const cu = new URL(chosen.url)
        const prox = dash
          ? `${PROXY}/d/${hex(JSON.stringify({ r: chosen.referer, o: chosen.origin, a: ua }))}/${cu.protocol.replace(':', '')}/${cu.host}${cu.pathname}${cu.search}`
          : proxyUrl(chosen.url, { referer: chosen.referer, origin: chosen.origin, ua })
        await cdp.navigate(`${PROXY}/player?type=${dash ? 'dash' : 'hls'}&src=${encodeURIComponent(prox)}`)
        let st = null
        for (let i = 0; i < 45; i++) {
          await sleep(1000)
          st = await cdp.eval(`(() => { const v = document.getElementById('v'); return { t: v.currentTime, w: v.videoWidth, h: v.videoHeight, paused: v.paused, s: window.__state } })()`).catch(() => null)
          if (st && (st.t > 6 || (st.s?.fatal && i > 8))) break
        }
        const pe = section(readLog(app.log), pl).filter(e => e.kind === 'proxy')
        const bad = pe.filter(e => e.status < 200 || e.status >= 300)
        r.ownPlayer = {
          ok: !!st && st.t > 5, currentTime: st ? +st.t.toFixed(1) : null, resolution: st ? `${st.w}x${st.h}` : null,
          levels: st?.s?.levels?.length ?? 0, audioTracks: st?.s?.audio?.length ?? 0, subtitleTracks: st?.s?.subs?.length ?? 0,
          fatal: st?.s?.fatal ?? null, errors: [...new Set(st?.s?.errors ?? [])].slice(0, 4),
          proxyRequests: pe.length, proxyNon2xx: bad.length, proxyBadSample: bad.slice(0, 2).map(e => `${e.status} ${e.error ?? ''} ${e.url.slice(0, 70)}`),
          segmentHosts: [...new Set(pe.map(e => { try { return new URL(e.url).host } catch { return '?' } }))].slice(0, 5),
        }
        r.shotPlayer = await cdp.shot(`t4-${label}.png`)
        await cdp.navigate('about:blank')
      }
    } catch (e) {
      r.error = e.message
    }
    saveJson(`run-${tag}.json`, results)
    console.log(JSON.stringify({ p: id, title, req: r.requests, blocked: r.blocked, popups: r.popupsBlocked, topNav: r.topNavBlocked, sandbox: r.iframe?.sandbox ?? null,
      provPlays: r.providerPlayer?.playing, provRes: r.providerPlayer?.resolution, m3u8ms: r.timeToFirstM3u8Ms, max: r.master?.maxResolution, bw: r.master?.maxBandwidth,
      fmt: r.manifestKind, master: r.master?.isMaster, codecs: r.master?.codecs, audio: r.master?.audio, subs: r.master?.subtitles, own: r.ownPlayer ? `${r.ownPlayer.ok} t=${r.ownPlayer.currentTime} ${r.ownPlayer.resolution} ${r.ownPlayer.fatal ?? ''}` : null, fail: r.fail, error: r.error }))
  }
} finally {
  await app.stop()
  if (server) server.kill()
}
