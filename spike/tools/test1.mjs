// Test 1 (+ comparison for Test 2): does an MV3 ad-block extension load and work in the
// Tauri/WebView2 window? The public d3ward test page is archived (shows only a notice), so the
// spike uses its own page spike/player/adtest.html: 18 ad + 14 tracker + 2 control URLs.
import { join } from 'node:path'
import { launch, CDP, ROOT, PROXY, sleep, readLog, saveJson } from './lib.mjs'

const PAGE = `${PROXY}/adtest`
const runs = [
  { name: 't1-none', mode: 'none' },
  { name: 't1-ubol-wry', mode: 'ext', extDir: join(ROOT, 'extensions') },
  { name: 't1-ubol-direct', mode: 'ext-direct', extDir: join(ROOT, 'extensions', 'ubol') },
  { name: 't1-adguard-direct', mode: 'ext-direct', extDir: join(ROOT, 'extensions-adguard', 'adguard-mv3') },
  { name: 't1-adblock-crate', mode: 'adblock' },
]
const only = process.argv[2]
const out = []

for (const run of runs) {
  if (only && !run.name.includes(only)) continue
  const res = { ...run, page: PAGE }
  let app
  try {
    app = await launch({ ...run, url: PAGE, dataDir: join(ROOT, 'userdata', run.name) })
    // extensions need a moment to install their rulesets on first start; then reload
    await sleep(run.mode.startsWith('ext') ? 15000 : 3000)
    await app.cdp.send('Page.reload', { ignoreCache: true })
    await fetch(`${PROXY}/mark?name=measure`)
    for (let i = 0; i < 40; i++) {
      await sleep(1000)
      if (await app.cdp.eval('!!(window.__adtest && window.__adtest.done)').catch(() => false)) break
    }
    const r = await app.cdp.eval('window.__adtest')
    res.page_rejected = { ads: `${r.ads}/${r.adsTotal}`, trackers: `${r.trackers}/${r.trackersTotal}`, controls: `${r.controls}/${r.controlsTotal}` }
    res.not_rejected = r.items.filter(i => !i.rejected && i.group !== 'control').map(i => i.url)
    res.urls = r.items.map(i => i.url)
    res.targets = (await CDP.targets()).filter(t => t.url.startsWith('chrome-extension://')).map(t => ({ type: t.type, title: t.title, url: t.url.slice(0, 90) }))
    res.shot = await app.cdp.shot(`${run.name}.png`)
  } catch (e) {
    res.error = e.message
  }
  if (app) await app.stop()
  const ev = readLog(join(ROOT, 'logs', `${run.name}.jsonl`))
  res.extEvents = ev.filter(e => ['ext_add', 'ext_list', 'hook_error', 'build_error', 'engine'].includes(e.kind))
  // what the Rust WebResourceRequested hook saw / blocked after the reload
  const i = ev.findIndex(e => e.kind === 'mark' && e.name === 'measure')
  const reqs = ev.slice(i + 1).filter(e => e.kind === 'req' && (res.urls ?? []).some(u => e.url.startsWith(u.slice(0, 300))))
  const blockedUrls = new Set(reqs.filter(e => e.blocked).map(e => e.url))
  const isCtl = u => /jsdelivr|wikipedia/.test(u)
  res.rust_hook = { seen: new Set(reqs.map(e => e.url)).size, blocked: [...blockedUrls].filter(u => !isCtl(u)).length, controls_blocked: [...blockedUrls].filter(isCtl).length }
  res.rust_not_blocked = [...new Set(reqs.filter(e => !e.blocked && !isCtl(e.url)).map(e => e.url))]
  delete res.urls
  out.push(res)
  console.log(JSON.stringify({ name: res.name, page_rejected: res.page_rejected, rust_hook: res.rust_hook, error: res.error, targets: res.targets, ext: res.extEvents }))
}
saveJson(only ? `test1-${only}.json` : 'test1.json', out)
