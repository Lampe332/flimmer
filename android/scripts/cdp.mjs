// Drives the app's WebView on a phone over adb (test builds with `--features inspect` only).
//   node cdp.mjs eval "<js expression>"     prints the JSON result (awaits promises)
//   node cdp.mjs fill "<css selector>" <config.json> <key>
//                                            types a value from a JSON file into an input without printing it
//   node cdp.mjs targets                    lists the inspectable pages
// Env: ANDROID_SERIAL (adb device), CDP_PORT (default 9333), CDP_MATCH (substring of the page URL, default 127.0.0.1)
import { execFileSync } from 'node:child_process'
import { readFileSync } from 'node:fs'

const port = process.env.CDP_PORT ?? '9333'
const match = process.env.CDP_MATCH ?? 'http://127.0.0.1'
const adb = (...args) => execFileSync('adb', args, { encoding: 'utf8' })

function forward() {
  const pid = adb('shell', 'pidof', 'local.flimmer.app').trim().split(/\s+/)[0]
  if (!pid) throw new Error('local.flimmer.app is not running')
  adb('forward', `tcp:${port}`, `localabstract:webview_devtools_remote_${pid}`)
}

async function targets() {
  forward()
  const list = await (await fetch(`http://127.0.0.1:${port}/json`)).json()
  return list.filter((t) => t.type === 'page')
}

async function evaluate(expression) {
  const page = (await targets()).find((t) => t.url.includes(match))
  if (!page) throw new Error(`no page matching ${match}`)
  const ws = new WebSocket(page.webSocketDebuggerUrl)
  await new Promise((ok, fail) => {
    ws.onopen = ok
    ws.onerror = () => fail(new Error('cannot connect to the page'))
  })
  const result = await new Promise((ok) => {
    ws.onmessage = (m) => {
      const msg = JSON.parse(m.data)
      if (msg.id === 1) ok(msg.result)
    }
    ws.send(JSON.stringify({ id: 1, method: 'Runtime.evaluate', params: { expression, awaitPromise: true, returnByValue: true, userGesture: true } }))
  })
  ws.close()
  if (result.exceptionDetails) throw new Error(result.exceptionDetails.exception?.description ?? result.exceptionDetails.text)
  return result.result.value
}

const [cmd, a, b, c] = process.argv.slice(2)
try {
  if (cmd === 'targets') {
    for (const t of await targets()) console.log(t.url.slice(0, 120))
  } else if (cmd === 'eval') {
    console.log(JSON.stringify(await evaluate(a)))
  } else if (cmd === 'fill') {
    const value = JSON.parse(readFileSync(b, 'utf8'))[c]
    if (typeof value !== 'string' || !value) throw new Error(`no "${c}" in ${b}`)
    // React tracks the value itself: use the native setter, then tell it.
    const js = `(() => { const el = document.querySelector(${JSON.stringify(a)}); if (!el) return 'no such element';
      const proto = el.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
      Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, ${JSON.stringify(value)});
      el.dispatchEvent(new Event('input', { bubbles: true })); return 'filled ' + el.value.length + ' characters'; })()`
    console.log(await evaluate(js))
  } else if (cmd === 'tap') {
    // A real touch (adb input) on the first visible element that matches the selector, or whose text is `a`.
    const js = `(async () => { const want = ${JSON.stringify(a)};
      const shown = (e) => { const r = e.getBoundingClientRect(); return r.width > 0 && r.height > 0 }
      let el = null; try { el = [...document.querySelectorAll(want)].find(shown) ?? null } catch {}
      el ??= [...document.querySelectorAll('button,a,[role=button],label,input,summary')].find((e) => e.textContent.trim() === want && e.getClientRects().length)
      if (!el) return null;
      el.scrollIntoView({ block: 'center', behavior: 'instant' });
      await new Promise((r) => setTimeout(r, 250));
      const r = el.getBoundingClientRect();
      return { x: (r.left + r.width / 2) * devicePixelRatio, y: (r.top + r.height / 2) * devicePixelRatio }; })()`
    const at = await evaluate(js)
    if (!at) throw new Error(`nothing to tap for ${a}`)
    // The WebView starts below the status bar (CDP_TOP device pixels, see `uiautomator dump`).
    const top = Number(process.env.CDP_TOP ?? 100)
    adb('shell', 'input', 'tap', String(Math.round(at.x)), String(Math.round(at.y + top)))
    console.log(`tapped ${a}`)
  } else {
    console.error('usage: node cdp.mjs eval|fill|tap|targets ...')
    process.exit(2)
  }
} catch (e) {
  console.error(String(e.message ?? e))
  process.exit(1)
}
