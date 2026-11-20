// Diagnose one URL as TOP-LEVEL page of the spike window (no Flimmer iframe around it).
//   node tools/probe.mjs <mode> <url> [click]
import { launch, CDP, sleep, readLog } from './lib.mjs'

const [mode, url, click] = process.argv.slice(2)
const name = `probe-${mode}-${new URL(url).host}`
const app = await launch({ mode, url, name })
try {
  await sleep(12000)
  if (click) { await app.cdp.clickCenter(); await sleep(10000) }
  const frames = []
  for (const t of (await CDP.targets()).filter(t => ['page', 'iframe'].includes(t.type))) {
    const c = new CDP()
    try {
      await c.open(t.webSocketDebuggerUrl)
      frames.push(await c.eval(`({ host: location.host, text: (document.body?.innerText ?? '').replace(/\\s+/g, ' ').slice(0, 120), videos: [...document.querySelectorAll('video')].map(v => ({ t: v.currentTime, w: v.videoWidth, h: v.videoHeight, paused: v.paused })) })`))
    } catch (e) { frames.push({ url: t.url.slice(0, 60), error: e.message }) }
    c.close()
  }
  const ev = readLog(app.log)
  const shot = await app.cdp.shot(`${name}.png`)
  console.log(JSON.stringify({
    frames, shot,
    requests: ev.filter(e => e.kind === 'req').length, blocked: ev.filter(e => e.kind === 'req' && e.blocked).length,
    popups: ev.filter(e => e.kind === 'popup_blocked').length,
    navs: ev.filter(e => e.kind === 'nav').map(e => e.url.slice(0, 80)),
    frameNavs: [...new Set(ev.filter(e => e.kind === 'frame_nav').map(e => new URL(e.url).host))],
    media: [...new Set(ev.filter(e => e.kind.startsWith('media_')).map(e => `${e.status ?? 'req'} ${e.ctype ?? ''} ${new URL(e.url).host}${new URL(e.url).pathname.slice(0, 40)}`))].slice(0, 6),
  }, null, 1))
} finally { await app.stop() }
