// Flimmer service worker: keeps the app shell (page, scripts, styles, fonts, icons) so the installed app opens
// at once. It never touches /api: data, streams and video always come straight from the PC.

const CACHE = 'flimmer-shell-v1'
const SHELL = ['/', '/manifest.webmanifest', '/icon-192.png']

self.addEventListener('install', (event) => {
  event.waitUntil(
    caches
      .open(CACHE)
      .then((cache) => cache.addAll(SHELL))
      .then(() => self.skipWaiting()),
  )
})

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) => Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k))))
      .then(() => self.clients.claim()),
  )
})

function keep(request, response) {
  if (response.ok && response.type === 'basic') {
    const copy = response.clone()
    caches.open(CACHE).then((cache) => cache.put(request, copy))
  }
  return response
}

self.addEventListener('fetch', (event) => {
  const request = event.request
  if (request.method !== 'GET') return
  const url = new URL(request.url)
  if (url.origin !== self.location.origin) return
  // API answers, playlists, segments and files are never cached and never answered from here.
  if (url.pathname.startsWith('/api/') || url.pathname === '/api' || request.headers.has('range')) return

  if (request.mode === 'navigate') {
    // The page itself: fresh when the PC answers, the kept copy when it does not.
    event.respondWith(
      fetch(request)
        .then((response) => keep('/', response))
        .catch(() => caches.match('/').then((hit) => hit || Response.error())),
    )
    return
  }
  if (url.pathname.startsWith('/assets/')) {
    // Built files carry a hash in their name: a kept copy never goes stale.
    event.respondWith(caches.match(request).then((hit) => hit || fetch(request).then((response) => keep(request, response))))
    return
  }
  if (/\.(png|svg|ico|webmanifest|woff2)$/.test(url.pathname)) {
    event.respondWith(
      caches.match(request).then((hit) => {
        const fresh = fetch(request).then((response) => keep(request, response))
        return hit || fresh
      }),
    )
  }
})
