// Shared helpers: launch the spike exe, talk CDP to its WebView2, read the JSONL event log.
import { spawn } from 'node:child_process'
import { readFileSync, writeFileSync, mkdirSync, existsSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

export const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..')
export const EXE = join(ROOT, 'desktop', 'target', 'release', 'flimmer-spike.exe')
export const CDP_PORT = 9222
export const PROXY = 'http://127.0.0.1:43311'
export const sleep = ms => new Promise(r => setTimeout(r, ms))

for (const d of ['logs', 'shots', 'userdata']) mkdirSync(join(ROOT, d), { recursive: true })

async function portBusy(port) {
  try { await fetch(`http://127.0.0.1:${port}/json/version`, { signal: AbortSignal.timeout(800) }); return true } catch { return false }
}

/** Starts the spike app. Returns { child, log, cdp, stop }. */
export async function launch({ mode = 'none', url = 'about:blank', name, extDir, lockOrigin = false, dataDir }) {
  if (await portBusy(CDP_PORT)) throw new Error(`port ${CDP_PORT} already in use - another spike instance running?`)
  const log = join(ROOT, 'logs', `${name}.jsonl`)
  const args = ['--root', ROOT, '--mode', mode, '--url', url, '--log', log,
    '--data-dir', dataDir ?? join(ROOT, 'userdata', mode)]
  if (extDir) args.push('--ext-dir', extDir)
  if (lockOrigin) args.push('--lock-origin')
  const child = spawn(EXE, args, { stdio: 'ignore' })
  let exited = false
  child.on('exit', () => { exited = true })
  const stop = async () => { if (!exited) { child.kill(); await sleep(1500) } }
  let cdp = null
  for (let i = 0; i < 60 && !exited; i++) {
    await sleep(500)
    try { cdp = await CDP.connectPage(); break } catch { /* not up yet */ }
  }
  if (!cdp) { await stop(); throw new Error(`spike app did not expose CDP (mode ${mode}); see ${log}`) }
  return { child, log, cdp, stop, pid: child.pid }
}

export class CDP {
  static async targets() {
    return (await fetch(`http://127.0.0.1:${CDP_PORT}/json/list`, { signal: AbortSignal.timeout(2000) })).json()
  }
  static async connectPage() {
    const list = await CDP.targets()
    const page = list.find(t => t.type === 'page' && !t.url.startsWith('chrome-extension://') && !t.url.startsWith('devtools://'))
    if (!page) throw new Error('no page target')
    const c = new CDP()
    await c.open(page.webSocketDebuggerUrl)
    await c.send('Page.enable')
    await c.send('Runtime.enable')
    return c
  }
  open(wsUrl) {
    this.id = 0; this.pending = new Map()
    this.ws = new WebSocket(wsUrl)
    this.ws.onmessage = ev => {
      const m = JSON.parse(ev.data)
      if (m.id && this.pending.has(m.id)) {
        const { res, rej } = this.pending.get(m.id); this.pending.delete(m.id)
        m.error ? rej(new Error(m.error.message)) : res(m.result)
      }
    }
    return new Promise((res, rej) => { this.ws.onopen = res; this.ws.onerror = () => rej(new Error('ws error')) })
  }
  send(method, params = {}) {
    const id = ++this.id
    return new Promise((res, rej) => {
      this.pending.set(id, { res, rej })
      this.ws.send(JSON.stringify({ id, method, params }))
      setTimeout(() => { if (this.pending.delete(id)) rej(new Error(`CDP timeout ${method}`)) }, 20000)
    })
  }
  async eval(expression) {
    const r = await this.send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true })
    if (r.exceptionDetails) throw new Error('eval: ' + (r.exceptionDetails.exception?.description ?? r.exceptionDetails.text))
    return r.result.value
  }
  navigate(url) { return this.send('Page.navigate', { url }) }
  async shot(file) {
    const r = await this.send('Page.captureScreenshot', { format: 'png' })
    const p = join(ROOT, 'shots', file)
    writeFileSync(p, Buffer.from(r.data, 'base64'))
    return p
  }
  /** Real mouse click at viewport coordinates (reaches cross-origin iframes too). */
  async click(x, y) {
    const base = { x, y, button: 'left', clickCount: 1 }
    await this.send('Input.dispatchMouseEvent', { ...base, type: 'mouseMoved', button: 'none' })
    await this.send('Input.dispatchMouseEvent', { ...base, type: 'mousePressed' })
    await this.send('Input.dispatchMouseEvent', { ...base, type: 'mouseReleased' })
  }
  async clickCenter() {
    const { w, h } = await this.eval('({w: innerWidth, h: innerHeight})')
    await this.click(Math.round(w / 2), Math.round(h / 2))
  }
  close() { try { this.ws.close() } catch { /* ignore */ } }
}

export function readLog(file) {
  if (!existsSync(file)) return []
  return readFileSync(file, 'utf8').split('\n').filter(Boolean).map(l => { try { return JSON.parse(l) } catch { return null } }).filter(Boolean)
}

export async function mark(name) {
  await fetch(`${PROXY}/mark?name=${encodeURIComponent(name)}`).catch(() => {})
}

/** Events between marker `name` and the next marker. */
export function section(events, name) {
  const i = events.findIndex(e => e.kind === 'mark' && e.name === name)
  if (i < 0) return []
  const rest = events.slice(i + 1)
  const j = rest.findIndex(e => e.kind === 'mark')
  return j < 0 ? rest : rest.slice(0, j)
}

export function saveJson(name, data) {
  const p = join(ROOT, 'logs', name)
  writeFileSync(p, JSON.stringify(data, null, 2))
  return p
}

export function proxyUrl(target, { referer = '', origin = '', ua = '' }, raw = false) {
  const q = new URLSearchParams({ r: referer, o: origin, a: ua, u: target })
  if (raw) q.set('raw', '1')
  return `${PROXY}/p?${q}`
}
