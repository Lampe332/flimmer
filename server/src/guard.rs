//! Who may talk to the server, and where the server may connect to (`docs/security.md`).
//!
//! - Every request passes [`middleware`]: path sanity, `Host` allow-list (DNS rebinding), fetch-metadata and
//!   `Origin` checks (CSRF), then authentication. The desktop window holds a session cookie it got through a
//!   one-time ticket; a phone holds a device token it got by pairing; ffmpeg reads files with an internal key.
//! - "Handy & Tablet im WLAN": a second listener on one private LAN address, pairing by PIN or QR code,
//!   a device list, brute-force lockout, and a list of what a paired device may do.
//! - Outbound: the stream proxy only connects to public addresses and only fetches URLs the server signed.

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};

use axum::{
    Extension, Json, Router,
    extract::{ConnectInfo, Path, Query, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
    routing::{delete, get, post},
};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::Notify;
use url::{Host, Url};

use crate::{
    api::AppState,
    db::{Db, Table, now_ms},
    error::{ApiError, ApiResult},
};

/// The listener a request came in on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    Loopback,
    Lan,
}

/// Who a request is from, once it is authenticated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Access {
    /// The desktop window (or a browser on the PC that came in through a start ticket).
    Pc,
    /// A paired phone, tablet or browser: the device id.
    Device(String),
    /// Flimmer's own helpers on loopback (ffmpeg reading a file, the engine asking the tracker shim).
    Internal,
}

/// Set on every request that reaches a handler.
#[derive(Debug, Clone)]
pub struct Who {
    pub access: Option<Access>,
    pub via: Via,
    pub peer: IpAddr,
}

const LAN_KEY: &str = "lan";
const DEVICES_KEY: &str = "lan_devices";
const DEV_HOST: &str = "localhost:5173";
const MAX_DEVICES: usize = 20;
const PAIRING_TTL: Duration = Duration::from_secs(10 * 60);
/// Wrong PINs from one address before that address has to wait.
const FAILS_PER_ADDRESS: u32 = 5;
/// Wrong PINs from all addresses together before the PIN is thrown away.
const FAILS_PER_PIN: u32 = 15;
const LOCK_BASE: Duration = Duration::from_secs(5 * 60);
const LOCK_MAX: Duration = Duration::from_secs(60 * 60);
const DEVICE_COOKIE_AGE: u64 = 365 * 24 * 60 * 60;

// ---------- small crypto helpers ----------

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `n` random bytes as hex, from the thread-local CSPRNG.
fn secret(n: usize) -> String {
    let mut buf = vec![0u8; n];
    rand::fill(&mut buf[..]);
    hex(&buf)
}

fn sha(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))
}

/// Compares without an early exit, so the time does not tell how many characters matched.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

static STREAM_KEY: LazyLock<[u8; 32]> = LazyLock::new(rand::random);

/// Signature that ties a proxied URL to its stream: only URLs the server itself wrote into a playlist of
/// stream `id` pass `/api/stream/:id/r`. The key lives for one run of the server.
pub fn sign_stream(id: &str, url: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(&*STREAM_KEY).expect("HMAC takes any key length");
    mac.update(id.as_bytes());
    mac.update(&[0]);
    mac.update(url.as_bytes());
    hex(&mac.finalize().into_bytes()[..16])
}

pub fn stream_signed(id: &str, url: &str, sig: &str) -> bool {
    same(&sign_stream(id, url), sig)
}

// ---------- outbound requests (stream proxy) ----------

/// An address on the public internet: not loopback, private, link-local, carrier NAT, multicast or reserved.
pub fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => public_v4(v4),
        IpAddr::V6(v6) => public_v6(v6),
    }
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    !(ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || o[0] == 0
        || (o[0] == 100 && (o[1] & 0xc0) == 64)          // 100.64.0.0/10 carrier NAT
        || (o[0] == 192 && o[1] == 0 && o[2] == 0)       // 192.0.0.0/24 protocol assignments
        || (o[0] == 198 && (o[1] & 0xfe) == 18)          // 198.18.0.0/15 benchmarking
        || o[0] >= 240)
}

fn public_v6(ip: Ipv6Addr) -> bool {
    let s = ip.segments();
    // Addresses that carry an IPv4 address inside are judged by that one.
    if let Some(v4) = ip.to_ipv4_mapped() {
        return public_v4(v4);
    }
    let embedded = |hi: u16, lo: u16| public_v4(Ipv4Addr::new((hi >> 8) as u8, hi as u8, (lo >> 8) as u8, lo as u8));
    if s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        return embedded(s[6], s[7]); // NAT64
    }
    if s[0] == 0x2002 {
        return embedded(s[1], s[2]); // 6to4
    }
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || (s[0] & 0xfe00) == 0xfc00                     // fc00::/7 unique local
        || (s[0] & 0xffc0) == 0xfe80                     // fe80::/10 link-local
        || (s[0] == 0x2001 && s[1] == 0xdb8)             // documentation
        || s[..6] == [0, 0, 0, 0, 0, 0])                 // ::/96 (IPv4-compatible, deprecated)
}

/// `http(s)` to a host that is not a private address. Host names are checked again when they are resolved
/// (see [`PublicDns`]), so a name that points at the home network does not get through either.
pub fn outbound_url_ok(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https") && url.username().is_empty() && url.password().is_none() && public_host(url)
}

/// The host part of [`outbound_url_ok`], for any scheme (tracker addresses are `udp://` too).
pub fn public_host(url: &Url) -> bool {
    match url.host() {
        Some(Host::Ipv4(ip)) => public_v4(ip),
        Some(Host::Ipv6(ip)) => public_v6(ip),
        Some(Host::Domain(name)) => {
            // `udp://` is no "special" scheme for the URL parser: an address there arrives as a name.
            if let Ok(ip) = name.parse::<IpAddr>() {
                return public_ip(ip);
            }
            let name = name.trim_end_matches('.').to_ascii_lowercase();
            !(name.is_empty() || name == "localhost" || name.ends_with(".localhost") || name.ends_with(".local") || name.ends_with(".internal"))
        }
        None => false,
    }
}

/// DNS for the stream proxy: answers with public addresses only. Because the connection is made to exactly
/// the addresses returned here, a second lookup with another answer (DNS rebinding) has nothing to attack.
struct PublicDns;

impl reqwest::dns::Resolve for PublicDns {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async move {
            let host = name.as_str().to_owned();
            let found = tokio::net::lookup_host((host.as_str(), 0)).await?;
            let public: Vec<SocketAddr> = found.filter(|a| public_ip(a.ip())).collect();
            if public.is_empty() {
                return Err("the host has no public address".into());
            }
            Ok(Box::new(public.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// The HTTP client for captured streams: public addresses only, on every redirect hop as well.
pub fn outbound_client() -> Result<reqwest::Client, String> {
    let redirects = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() > 5 {
            attempt.error("too many redirects")
        } else if outbound_url_ok(attempt.url()) {
            attempt.follow()
        } else {
            attempt.error("redirect to a blocked address")
        }
    });
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(30))
        .redirect(redirects)
        .dns_resolver(Arc::new(PublicDns))
        .build()
        .map_err(|e| format!("cannot build HTTP client: {e}"))
}

// ---------- LAN addresses ----------

/// The ranges home networks use. Only these are offered for the phone mode and accepted as its clients.
fn lan_ip(ip: Ipv4Addr) -> bool {
    ip.is_private()
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Candidate {
    pub ip: Ipv4Addr,
    pub name: String,
}

/// The address of the network card the default route uses. No packet is sent: `connect` on a UDP socket
/// only asks the system which local address it would use.
fn default_route_ip() -> Option<Ipv4Addr> {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(ip) => Some(ip),
        IpAddr::V6(_) => None,
    }
}

/// Private IPv4 addresses of this PC, the one the default route uses first.
pub fn candidates() -> Vec<Candidate> {
    let mut list: Vec<Candidate> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback())
        .filter_map(|i| match i.ip() {
            IpAddr::V4(ip) if lan_ip(ip) => Some(Candidate { ip, name: i.name.chars().filter(|c| !c.is_control()).take(60).collect() }),
            _ => None,
        })
        .collect();
    list.dedup_by_key(|c| c.ip);
    let main = default_route_ip();
    list.sort_by_key(|c| (Some(c.ip) != main, c.ip));
    list
}

// ---------- state ----------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct LanConfig {
    enabled: bool,
    /// The address to listen on; `None` = the PC's main private address.
    #[serde(default)]
    ip: Option<Ipv4Addr>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Device {
    id: String,
    name: String,
    /// SHA-256 of the device token; the token itself only exists in the device's cookie.
    hash: String,
    created_at: i64,
    last_seen: i64,
    last_ip: String,
}

struct Pairing {
    pin: String,
    token: String,
    expires: Instant,
    fails: u32,
}

#[derive(Default)]
struct Fails {
    count: u32,
    lockouts: u32,
    until: Option<Instant>,
}

struct Bucket {
    tokens: f64,
    at: Instant,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Debug, PartialEq, Eq)]
pub enum PairError {
    /// No pairing is open on the PC (never started, used up, expired or thrown away after too many tries).
    Closed,
    Wrong,
    /// Too many wrong tries from this address: seconds to wait.
    Locked(u64),
    /// 20 devices are paired already.
    Full,
}

pub struct Guard {
    port: u16,
    /// Also accept the Vite dev server (`localhost:5173`) as host and origin.
    dev: bool,
    session: String,
    internal: String,
    tickets: Mutex<Vec<(String, Instant)>>,
    config: Mutex<LanConfig>,
    /// The LAN address the server listens on right now.
    bound: Mutex<Option<Ipv4Addr>>,
    bind_error: Mutex<Option<String>>,
    devices: Mutex<Vec<Device>>,
    pairing: Mutex<Option<Pairing>>,
    fails: Mutex<HashMap<IpAddr, Fails>>,
    buckets: Mutex<HashMap<(Group, String), Bucket>>,
    /// Wakes the LAN listener task after a settings change.
    pub changed: Notify,
}

impl Guard {
    pub fn new(db: &Db, port: u16, dev: bool) -> Self {
        let read = |key: &str| db.blob_get(Table::Settings, key).ok().flatten();
        let config = read(LAN_KEY).and_then(|d| serde_json::from_str(&d).ok()).unwrap_or_default();
        let devices = read(DEVICES_KEY).and_then(|d| serde_json::from_str(&d).ok()).unwrap_or_default();
        Self {
            port,
            dev,
            session: secret(32),
            internal: secret(32),
            tickets: Mutex::default(),
            config: Mutex::new(config),
            bound: Mutex::default(),
            bind_error: Mutex::default(),
            devices: Mutex::new(devices),
            pairing: Mutex::default(),
            fails: Mutex::default(),
            buckets: Mutex::default(),
            changed: Notify::new(),
        }
    }

    /// The key Flimmer's own ffmpeg calls carry (`?ik=`).
    pub fn internal_key(&self) -> &str {
        &self.internal
    }

    // ----- PC session -----

    /// A one-time ticket: opening `/api/auth/enter?t=<ticket>` within `ttl` sets the session cookie.
    pub fn ticket(&self, ttl: Duration) -> String {
        let t = secret(24);
        let mut tickets = lock(&self.tickets);
        let now = Instant::now();
        tickets.retain(|(_, until)| *until > now);
        tickets.push((t.clone(), now + ttl));
        t
    }

    fn redeem(&self, ticket: &str) -> bool {
        let mut tickets = lock(&self.tickets);
        let now = Instant::now();
        tickets.retain(|(_, until)| *until > now);
        match tickets.iter().position(|(t, _)| same(t, ticket)) {
            Some(i) => {
                tickets.remove(i);
                true
            }
            None => false,
        }
    }

    fn session_cookie(&self) -> String {
        format!("flimmer_s_{}", self.port)
    }

    fn device_cookie(&self) -> String {
        format!("flimmer_d_{}", self.port)
    }

    // ----- hosts and origins -----

    fn host_ok(&self, via: Via, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        match via {
            Via::Loopback => {
                host == format!("127.0.0.1:{}", self.port) || host == format!("localhost:{}", self.port) || (self.dev && host == DEV_HOST)
            }
            Via::Lan => lock(&self.bound).is_some_and(|ip| host == format!("{ip}:{}", self.port)),
        }
    }

    /// A LAN request must come from a private address while the phone mode is on.
    fn lan_peer_ok(&self, peer: IpAddr) -> bool {
        lock(&self.config).enabled
            && match peer {
                IpAddr::V4(ip) => lan_ip(ip),
                IpAddr::V6(ip) => ip.to_ipv4_mapped().is_some_and(lan_ip),
            }
    }

    // ----- devices -----

    fn save_devices(&self, db: &Db, devices: &[Device]) {
        if let Ok(data) = serde_json::to_string(devices) {
            let _ = db.blob_put(Table::Settings, DEVICES_KEY, &data);
        }
    }

    /// The device a cookie value (`<id>.<token>`) belongs to. Notes when and from where it was seen.
    fn device_of(&self, db: &Db, cookie: &str, peer: IpAddr) -> Option<String> {
        let (id, token) = cookie.split_once('.')?;
        let hash = sha(token);
        let mut devices = lock(&self.devices);
        let d = devices.iter_mut().find(|d| d.id == id)?;
        if !same(&d.hash, &hash) {
            return None;
        }
        let now = now_ms();
        let ip = peer.to_string();
        // Not on every request: once a minute is enough for "zuletzt gesehen".
        if now - d.last_seen > 60_000 || d.last_ip != ip {
            d.last_seen = now;
            d.last_ip = ip;
            let copy = devices.clone();
            drop(devices);
            self.save_devices(db, &copy);
        }
        Some(id.to_owned())
    }

    pub fn revoke(&self, db: &Db, id: &str) -> bool {
        let mut devices = lock(&self.devices);
        let before = devices.len();
        devices.retain(|d| d.id != id);
        let removed = devices.len() != before;
        if removed {
            self.save_devices(db, &devices);
        }
        removed
    }

    /// After "Alles zurücksetzen": the settings table is empty, so the phone mode is off and no device is known.
    pub fn forget_all(&self) {
        *lock(&self.config) = LanConfig::default();
        lock(&self.devices).clear();
        *lock(&self.pairing) = None;
        self.changed.notify_one();
    }

    // ----- pairing -----

    fn pairing_start(&self) {
        let pin = format!("{:06}", rand::random_range(0..1_000_000u32));
        *lock(&self.pairing) = Some(Pairing { pin, token: secret(16), expires: Instant::now() + PAIRING_TTL, fails: 0 });
    }

    fn pairing_stop(&self) {
        *lock(&self.pairing) = None;
    }

    /// Seconds this address still has to wait after too many wrong PINs.
    fn locked_for(&self, peer: IpAddr) -> Option<u64> {
        let fails = lock(&self.fails);
        let until = fails.get(&peer)?.until?;
        let left = until.checked_duration_since(Instant::now())?;
        Some(left.as_secs().max(1))
    }

    fn note_fail(&self, peer: IpAddr) {
        let mut fails = lock(&self.fails);
        // The table cannot grow without bound: forget addresses that are neither locked nor recent.
        if fails.len() > 1000 {
            let now = Instant::now();
            fails.retain(|_, f| f.until.is_some_and(|u| u > now));
        }
        let f = fails.entry(peer).or_default();
        f.count += 1;
        if f.count >= FAILS_PER_ADDRESS {
            f.count = 0;
            let wait = LOCK_BASE.saturating_mul(1 << f.lockouts.min(4)).min(LOCK_MAX);
            f.lockouts += 1;
            f.until = Some(Instant::now() + wait);
        }
    }

    /// Pairs a device that knows the PIN or the QR code's token. On success: the device cookie's value.
    pub fn pair(&self, db: &Db, peer: IpAddr, pin: Option<&str>, token: Option<&str>, agent: &str) -> Result<String, PairError> {
        if let Some(wait) = self.locked_for(peer) {
            return Err(PairError::Locked(wait));
        }
        let mut slot = lock(&self.pairing);
        let open = slot.as_ref().is_some_and(|p| p.expires > Instant::now());
        if !open {
            *slot = None;
            return Err(PairError::Closed);
        }
        let p = slot.as_mut().expect("checked above");
        let pin_ok = pin.is_some_and(|v| same(v.trim(), &p.pin));
        let token_ok = token.is_some_and(|v| same(v.trim(), &p.token));
        if !(pin_ok || token_ok) {
            p.fails += 1;
            if p.fails >= FAILS_PER_PIN {
                *slot = None;
            }
            drop(slot);
            self.note_fail(peer);
            return Err(match self.locked_for(peer) {
                Some(wait) => PairError::Locked(wait),
                None => PairError::Wrong,
            });
        }
        let mut devices = lock(&self.devices);
        if devices.len() >= MAX_DEVICES {
            return Err(PairError::Full);
        }
        // PIN and token work once.
        *slot = None;
        drop(slot);
        lock(&self.fails).remove(&peer);
        let (id, token) = (secret(6), secret(32));
        let now = now_ms();
        devices.push(Device { id: id.clone(), name: device_name(agent), hash: sha(&token), created_at: now, last_seen: now, last_ip: peer.to_string() });
        self.save_devices(db, &devices);
        Ok(format!("{id}.{token}"))
    }

    // ----- LAN listener -----

    /// The address the phone mode should listen on now, if it is on and the PC has a private address.
    pub fn lan_target(&self) -> Option<Ipv4Addr> {
        let cfg = lock(&self.config).clone();
        if !cfg.enabled {
            return None;
        }
        let list = candidates();
        cfg.ip.filter(|ip| list.iter().any(|c| c.ip == *ip)).or_else(|| list.first().map(|c| c.ip))
    }

    pub fn set_bound(&self, ip: Option<Ipv4Addr>, error: Option<String>) {
        *lock(&self.bound) = ip;
        *lock(&self.bind_error) = error;
    }

    fn status(&self) -> Value {
        let cfg = lock(&self.config).clone();
        let bound = *lock(&self.bound);
        let url = bound.map(|ip| format!("http://{ip}:{}", self.port));
        let pairing = lock(&self.pairing).as_ref().filter(|p| p.expires > Instant::now()).map(|p| {
            let link = url.as_ref().map(|u| format!("{u}/pair#t={}", p.token));
            json!({
                "pin": p.pin,
                "link": link,
                "qr": link.as_deref().and_then(qr_svg),
                "expires_in": p.expires.saturating_duration_since(Instant::now()).as_secs(),
            })
        });
        let devices: Vec<Value> = lock(&self.devices)
            .iter()
            .map(|d| json!({ "id": d.id, "name": d.name, "created_at": crate::scan::iso(d.created_at), "last_seen": crate::scan::iso(d.last_seen), "last_ip": d.last_ip }))
            .collect();
        json!({
            "enabled": cfg.enabled,
            "ip": cfg.ip,
            "active_ip": bound,
            "url": url,
            "port": self.port,
            "candidates": candidates(),
            "error": *lock(&self.bind_error),
            "pairing": pairing,
            "devices": devices,
        })
    }

    // ----- rate limits -----

    /// One request of `group` by `who`: `false` when the bucket is empty.
    fn allow(&self, group: Group, who: &str) -> bool {
        let (burst, per_minute) = group.limits();
        let mut buckets = lock(&self.buckets);
        let now = Instant::now();
        if buckets.len() > 2000 {
            buckets.retain(|_, b| now.duration_since(b.at) < Duration::from_secs(120));
        }
        let b = buckets.entry((group, who.to_owned())).or_insert(Bucket { tokens: burst, at: now });
        b.tokens = (b.tokens + now.duration_since(b.at).as_secs_f64() * per_minute / 60.0).min(burst);
        b.at = now;
        if b.tokens < 1.0 {
            return false;
        }
        b.tokens -= 1.0;
        true
    }
}

/// "Android, Chrome" from a User-Agent: enough to tell devices apart in the list.
fn device_name(agent: &str) -> String {
    let has = |s: &str| agent.contains(s);
    let system = if has("Android") {
        "Android"
    } else if has("iPhone") {
        "iPhone"
    } else if has("iPad") {
        "iPad"
    } else if has("Windows") {
        "Windows"
    } else if has("Mac OS") {
        "Mac"
    } else if has("Linux") {
        "Linux"
    } else {
        "Gerät"
    };
    let browser = if has("Edg/") || has("EdgA/") {
        "Edge"
    } else if has("Firefox/") || has("FxiOS/") {
        "Firefox"
    } else if has("SamsungBrowser/") {
        "Samsung Internet"
    } else if has("Chrome/") || has("CriOS/") {
        "Chrome"
    } else if has("Safari/") {
        "Safari"
    } else {
        "Browser"
    };
    format!("{system}, {browser}")
}

fn qr_svg(text: &str) -> Option<String> {
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    Some(
        code.render::<qrcode::render::svg::Color>()
            .min_dimensions(240, 240)
            .quiet_zone(true)
            .dark_color(qrcode::render::svg::Color("#000000"))
            .light_color(qrcode::render::svg::Color("#ffffff"))
            .build(),
    )
}

// ---------- rules: paths, devices, rate limits ----------

/// No `..`, backslash, NUL or drive/stream colon after decoding: such a path never names something Flimmer serves.
fn path_ok(path: &str) -> bool {
    let api = path == "/api" || path.starts_with("/api/");
    let Ok(text) = percent_decode(path) else { return false };
    // A second encoding layer (`%252e`) would only reach the file system if someone decoded twice; refuse it anyway.
    let twice = percent_decode(&text).unwrap_or_default();
    [text.as_str(), twice.as_str()].iter().all(|p| {
        !p.contains('\\') && !p.contains('\0') && p.split('/').all(|seg| seg != ".." && seg != ".") && (api || !p.contains(':'))
    })
}

fn percent_decode(s: &str) -> Result<String, ()> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).ok_or(())?;
            out.push(u8::from_str_radix(hex, 16).map_err(|_| ())?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| ())
}

fn segments(path: &str) -> Vec<&str> {
    path.trim_matches('/').split('/').collect()
}

/// What a paired device may do. Everything that changes how the PC behaves (setup, token, VPN file, folders,
/// torrents without VPN, auto-download rules, export/import, cache, the phone mode itself), opens something
/// on the PC or deletes files stays with the PC.
fn device_allowed(method: &Method, path: &str, query: &str) -> bool {
    let seg = segments(path);
    let seg = &seg[1..]; // after "api"
    if method == Method::GET || method == Method::HEAD {
        return !matches!(seg, ["lan", ..] | ["maintenance", ..] | ["auth", ..] | ["torrents", "announce"]);
    }
    let has = |key: &str, value: &str| url::form_urlencoded::parse(query.as_bytes()).any(|(k, v)| k == key && v == value);
    match (method.as_str(), seg) {
        ("PUT" | "DELETE", ["watchlist", _, _]) => true,
        ("POST", ["scan", _, _]) => true,
        ("POST", ["stream", _, "prefetch"]) | ("DELETE", ["stream", _]) => true,
        ("PUT", ["settings"]) | ("PUT", ["prefs"]) => true,
        ("PUT", ["progress", _, _]) | ("PUT" | "DELETE", ["watched", _, _]) => true,
        ("DELETE", ["continue", _, _]) | ("DELETE", ["history"]) | ("DELETE", ["history", _]) => true,
        // Cutting the tunnel is always allowed; bringing it back is the PC's call.
        ("POST", ["vpn", "kill"]) => true,
        ("POST", ["torrents", "start"]) => true,
        ("POST", ["torrents", _, "pause" | "resume"]) => true,
        ("PUT", ["torrents", _, "files" | "priority"]) => true,
        // Removing a torrent from the list is fine, deleting its files is not.
        ("DELETE", ["torrents", id]) => *id != "settings" && !has("files", "1"),
        ("POST", ["pair", "logout"]) => true,
        _ => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Group {
    Scan,
    TorrentSearch,
    TorrentStart,
    Ffmpeg,
    Prefetch,
    AutoDl,
    Setup,
    Pair,
}

impl Group {
    /// (burst, refill per minute)
    fn limits(self) -> (f64, f64) {
        match self {
            Self::Scan => (15.0, 30.0),
            Self::TorrentSearch => (60.0, 120.0),
            Self::TorrentStart => (15.0, 30.0),
            Self::Ffmpeg => (60.0, 180.0),
            Self::Prefetch => (40.0, 120.0),
            Self::AutoDl => (5.0, 10.0),
            Self::Setup => (10.0, 20.0),
            Self::Pair => (20.0, 30.0),
        }
    }

    /// The expensive endpoints: they start browser windows, ffmpeg, searches on 27 sites or outside calls.
    fn of(method: &Method, path: &str) -> Option<Self> {
        let seg = segments(path);
        Some(match (method.as_str(), &seg[1..]) {
            ("POST", ["scan", _, _]) => Self::Scan,
            ("GET", ["torrents", "search"]) | ("GET", ["torrents", "movie" | "tv", _]) => Self::TorrentSearch,
            ("POST", ["torrents", "start"]) => Self::TorrentStart,
            ("GET", ["torrents", _, "play" | "playinfo" | "seek" | "subtitle"]) => Self::Ffmpeg,
            ("POST", ["stream", _, "prefetch"]) => Self::Prefetch,
            ("POST", ["autodl", "run"]) | ("POST", ["autodl", "preview", _, _]) => Self::AutoDl,
            ("PUT", ["setup", "tmdb"]) | ("PUT", ["setup", "vpn"]) | ("POST", ["setup", "reset"]) | ("POST", ["export" | "import"]) => Self::Setup,
            ("POST", ["pair"]) => Self::Pair,
            _ => return None,
        })
    }
}

// ---------- middleware ----------

fn refuse(status: StatusCode, code: &'static str, message: &str) -> Response {
    (status, Json(json!({ "error": message, "code": code }))).into_response()
}

fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
}

fn text(headers: &HeaderMap, name: impl header::AsHeaderName) -> Option<&str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: https://image.tmdb.org; \
    media-src 'self' blob:; connect-src 'self'; worker-src 'self' blob:; font-src 'self' data:; frame-src https://www.youtube-nocookie.com; \
    manifest-src 'self'; object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'none'";

/// Headers every answer gets: no sniffing, no framing, no reading from another origin.
fn harden(mut resp: Response, api: bool, path: &str) -> Response {
    let h = resp.headers_mut();
    let mut set = |name: header::HeaderName, value: &'static str| {
        h.insert(name, HeaderValue::from_static(value));
    };
    set(header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    set(header::X_FRAME_OPTIONS, "DENY");
    set(header::REFERRER_POLICY, "strict-origin-when-cross-origin");
    set(header::HeaderName::from_static("cross-origin-resource-policy"), "same-origin");
    set(header::HeaderName::from_static("cross-origin-opener-policy"), "same-origin");
    set(header::CONTENT_SECURITY_POLICY, CSP);
    let json = h.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|c| c.starts_with("application/json"));
    // API data never goes into a browser cache; the service worker and the page itself are always asked for again.
    let fresh = (api && json) || matches!(path, "/sw.js" | "/manifest.webmanifest") || !(api || path.starts_with("/assets/") || path.contains('.'));
    if fresh && !h.contains_key(header::CACHE_CONTROL) {
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static(if api { "no-store" } else { "no-cache" }));
    }
    resp
}

fn identify(st: &AppState, via: Via, peer: IpAddr, req: &Request) -> Option<Access> {
    let g = &st.guard;
    let headers = req.headers();
    let path = req.uri().path();
    if via == Via::Loopback {
        let from_page = headers.contains_key(header::ORIGIN) || headers.contains_key("sec-fetch-site") || headers.contains_key(header::COOKIE);
        if path == "/api/torrents/announce" && !from_page {
            return Some(Access::Internal);
        }
        if matches!(segments(path)[..], ["api", "torrents", _, "file", _]) {
            let key = url::form_urlencoded::parse(req.uri().query().unwrap_or_default().as_bytes()).find(|(k, _)| k == "ik").map(|(_, v)| v.into_owned());
            if key.is_some_and(|k| same(&k, &g.internal)) {
                return Some(Access::Internal);
            }
        }
        if cookie(headers, &g.session_cookie()).is_some_and(|c| same(c, &g.session)) {
            return Some(Access::Pc);
        }
    }
    cookie(headers, &g.device_cookie()).and_then(|c| g.device_of(&st.db, c, peer)).map(Access::Device)
}

/// Runs in front of every route, the web UI's files included.
pub async fn middleware(State(st): State<Arc<AppState>>, mut req: Request, next: Next) -> Response {
    let g = &st.guard;
    let path = req.uri().path().to_owned();
    let api = path == "/api" || path.starts_with("/api/");
    let done = |resp: Response| harden(resp, api, &path);
    let (Some(via), Some(peer)) = (req.extensions().get::<Via>().copied(), req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip())) else {
        return done(refuse(StatusCode::INTERNAL_SERVER_ERROR, "no_listener", "the request has no listener information"));
    };
    if !path_ok(&path) {
        return done(refuse(StatusCode::BAD_REQUEST, "bad_path", "this path is not allowed"));
    }

    // DNS rebinding: a foreign page whose name points at this PC still sends its own name as Host.
    let host = text(req.headers(), header::HOST).unwrap_or_default().to_owned();
    let authority_ok = req.uri().authority().is_none_or(|a| g.host_ok(via, a.as_str()));
    // The torrent engine asks the tracker shim under a name that exists nowhere (`.invalid` never resolves, so
    // no web page can have it); Flimmer's own SOCKS server routes that name to this listener.
    let shim = via == Via::Loopback && path == "/api/torrents/announce" && host.split(':').next() == Some(crate::vpn::SHIM_HOST);
    if !(shim || g.host_ok(via, &host)) || !authority_ok {
        return done(refuse(StatusCode::FORBIDDEN, "forbidden_host", "this host name is not Flimmer's address"));
    }
    if via == Via::Lan && !g.lan_peer_ok(peer) {
        return done(refuse(StatusCode::FORBIDDEN, "forbidden_peer", "only devices in the home network may connect"));
    }
    if !api {
        return done(next.run(req).await);
    }

    // CSRF: a browser says where a request comes from. Only Flimmer's own page (or no page at all) may call the API.
    if text(req.headers(), "sec-fetch-site").is_some_and(|site| !matches!(site, "same-origin" | "none")) {
        return done(refuse(StatusCode::FORBIDDEN, "cross_site", "requests from other pages are not allowed"));
    }
    if let Some(origin) = text(req.headers(), header::ORIGIN) {
        let own = origin.eq_ignore_ascii_case(&format!("http://{host}"));
        if !own {
            return done(refuse(StatusCode::FORBIDDEN, "cross_origin", "requests from other pages are not allowed"));
        }
    }

    let method = req.method().clone();
    let access = identify(&st, via, peer, &req);
    let open = matches!((method.as_str(), path.as_str()), ("GET", "/api/session") | ("POST", "/api/pair") | ("GET", "/api/auth/enter"));
    match &access {
        None if !open => {
            return done(refuse(StatusCode::UNAUTHORIZED, "pairing_required", "this device is not paired with Flimmer"));
        }
        Some(Access::Device(_)) if !open && !device_allowed(&method, &path, req.uri().query().unwrap_or_default()) => {
            return done(refuse(StatusCode::FORBIDDEN, "pc_only", "this can only be done in Flimmer on the PC"));
        }
        _ => {}
    }
    if let Some(group) = Group::of(&method, &path) {
        let who = match &access {
            Some(Access::Internal) => None,
            Some(Access::Pc) => Some("pc".to_owned()),
            Some(Access::Device(id)) => Some(id.clone()),
            None => Some(peer.to_string()),
        };
        if who.is_some_and(|w| !g.allow(group, &w)) {
            let mut resp = refuse(StatusCode::TOO_MANY_REQUESTS, "rate_limited", "too many requests, wait a moment");
            resp.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from_static("10"));
            return done(resp);
        }
    }
    req.extensions_mut().insert(Who { access, via, peer });
    done(next.run(req).await)
}

// ---------- endpoints ----------

type St = State<Arc<AppState>>;

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/auth/enter", get(enter))
        .route("/api/session", get(session))
        .route("/api/pair", post(pair))
        .route("/api/pair/logout", post(logout))
        .route("/api/lan", get(lan_get).put(lan_put))
        .route("/api/lan/pairing", post(pairing_start).delete(pairing_stop))
        .route("/api/lan/devices/{id}", delete(device_delete))
        .with_state(state)
}

fn pc_only(who: &Who) -> ApiResult<()> {
    match who.access {
        Some(Access::Pc) => Ok(()),
        _ => Err(ApiError::coded(StatusCode::FORBIDDEN, "pc_only", "this can only be done in Flimmer on the PC")),
    }
}

/// `GET /api/auth/enter?t=<ticket>`: how the desktop window gets its session. The ticket works once.
async fn enter(State(st): St, Extension(who): Extension<Who>, Query(q): Query<HashMap<String, String>>) -> Response {
    let g = &st.guard;
    let ok = who.via == Via::Loopback && q.get("t").is_some_and(|t| g.redeem(t));
    if !ok {
        return refuse(StatusCode::FORBIDDEN, "bad_ticket", "this start link is used up; open Flimmer again");
    }
    let value = format!("{}={}; HttpOnly; SameSite=Strict; Path=/", g.session_cookie(), g.session);
    let mut resp = Redirect::to("/").into_response();
    if let Ok(v) = HeaderValue::from_str(&value) {
        resp.headers_mut().insert(header::SET_COOKIE, v);
    }
    resp
}

/// `GET /api/session`: who the caller is. The UI shows the pairing page for `"none"`.
async fn session(State(st): St, Extension(who): Extension<Who>) -> Json<Value> {
    let (role, device) = match &who.access {
        Some(Access::Pc) | Some(Access::Internal) => ("pc", None),
        Some(Access::Device(id)) => ("device", lock(&st.guard.devices).iter().find(|d| d.id == *id).map(|d| d.name.clone())),
        None => ("none", None),
    };
    Json(json!({ "role": role, "via": if who.via == Via::Lan { "lan" } else { "local" }, "device": device }))
}

#[derive(Deserialize)]
struct PairBody {
    #[serde(default)]
    pin: Option<String>,
    #[serde(default)]
    token: Option<String>,
}

/// `POST /api/pair {pin}` or `{token}`: trades the PIN from the PC's settings (or the QR code's token) for a device cookie.
async fn pair(State(st): St, Extension(who): Extension<Who>, headers: HeaderMap, Json(body): Json<PairBody>) -> Response {
    let g = &st.guard;
    let agent = text(&headers, header::USER_AGENT).unwrap_or_default();
    match g.pair(&st.db, who.peer, body.pin.as_deref(), body.token.as_deref(), agent) {
        Ok(value) => {
            let cookie = format!("{}={value}; HttpOnly; SameSite=Strict; Path=/; Max-Age={DEVICE_COOKIE_AGE}", g.device_cookie());
            let mut resp = Json(json!({ "ok": true })).into_response();
            if let Ok(v) = HeaderValue::from_str(&cookie) {
                resp.headers_mut().insert(header::SET_COOKIE, v);
            }
            resp
        }
        Err(PairError::Closed) => refuse(StatusCode::CONFLICT, "closed", "no pairing is open; start one in Flimmer on the PC"),
        Err(PairError::Wrong) => refuse(StatusCode::FORBIDDEN, "wrong_pin", "this PIN is not right"),
        Err(PairError::Full) => refuse(StatusCode::CONFLICT, "full", "too many devices are paired; remove one on the PC"),
        Err(PairError::Locked(wait)) => {
            let mut resp = (StatusCode::TOO_MANY_REQUESTS, Json(json!({ "error": "too many wrong tries", "code": "locked", "retry_after": wait }))).into_response();
            if let Ok(v) = HeaderValue::from_str(&wait.to_string()) {
                resp.headers_mut().insert(header::RETRY_AFTER, v);
            }
            resp
        }
    }
}

/// `POST /api/pair/logout`: the device removes itself.
async fn logout(State(st): St, Extension(who): Extension<Who>) -> Response {
    let g = &st.guard;
    if let Some(Access::Device(id)) = &who.access {
        g.revoke(&st.db, id);
    }
    let gone = format!("{}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0", g.device_cookie());
    let mut resp = StatusCode::NO_CONTENT.into_response();
    if let Ok(v) = HeaderValue::from_str(&gone) {
        resp.headers_mut().insert(header::SET_COOKIE, v);
    }
    resp
}

/// `GET /api/lan` (PC only): the phone mode's state, the open pairing and the paired devices.
async fn lan_get(State(st): St, Extension(who): Extension<Who>) -> ApiResult<Json<Value>> {
    pc_only(&who)?;
    Ok(Json(st.guard.status()))
}

#[derive(Deserialize)]
struct LanBody {
    enabled: bool,
    #[serde(default)]
    ip: Option<Ipv4Addr>,
}

/// `PUT /api/lan {enabled, ip?}` (PC only). `ip` must be one of the PC's private addresses; without it the main one is used.
async fn lan_put(State(st): St, Extension(who): Extension<Who>, Json(body): Json<LanBody>) -> ApiResult<StatusCode> {
    pc_only(&who)?;
    if body.ip.is_some_and(|ip| !candidates().iter().any(|c| c.ip == ip)) {
        return Err(ApiError::bad_request("'ip' must be one of this PC's private network addresses"));
    }
    let g = &st.guard;
    let cfg = LanConfig { enabled: body.enabled, ip: body.ip };
    let data = serde_json::to_string(&cfg).map_err(|_| ApiError::internal("could not encode settings"))?;
    st.db.blob_put(Table::Settings, LAN_KEY, &data)?;
    *lock(&g.config) = cfg;
    if !body.enabled {
        g.pairing_stop();
    }
    g.changed.notify_one();
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/lan/pairing` (PC only): opens a pairing for 10 minutes with a new PIN and QR code.
async fn pairing_start(State(st): St, Extension(who): Extension<Who>) -> ApiResult<Json<Value>> {
    pc_only(&who)?;
    if !lock(&st.guard.config).enabled {
        return Err(ApiError::bad_request("turn the phone mode on first"));
    }
    st.guard.pairing_start();
    Ok(Json(st.guard.status()))
}

/// `DELETE /api/lan/pairing` (PC only): closes the pairing; PIN and QR code stop working.
async fn pairing_stop(State(st): St, Extension(who): Extension<Who>) -> ApiResult<StatusCode> {
    pc_only(&who)?;
    st.guard.pairing_stop();
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/lan/devices/:id` (PC only): the device's token stops working at once.
async fn device_delete(State(st): St, Extension(who): Extension<Who>, Path(id): Path<String>) -> ApiResult<StatusCode> {
    pc_only(&who)?;
    if st.guard.revoke(&st.db, &id) { Ok(StatusCode::NO_CONTENT) } else { Err(ApiError::not_found("unknown device")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guard() -> (Guard, Db) {
        let path = std::env::temp_dir().join(format!("flimmer-guard-{}-{}.db", std::process::id(), secret(4)));
        let db = Db::open(&path).unwrap();
        (Guard::new(&db, 43300, false), db)
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn only_public_addresses_are_fetched() {
        for bad in [
            "http://127.0.0.1/", "http://127.1/", "http://2130706433/", "http://0x7f000001/", "http://0177.0.0.1/", "http://localhost/",
            "http://[::1]/", "http://[::ffff:127.0.0.1]/", "http://[::ffff:169.254.169.254]/", "http://169.254.169.254/latest/meta-data/",
            "http://10.0.0.5/", "http://172.16.3.4/", "http://192.168.1.1/", "http://100.64.0.1/", "http://0.0.0.0/", "http://[fc00::1]/",
            "http://[fe80::1]/", "http://[64:ff9b::7f00:1]/", "http://[2002:7f00:1::]/", "http://printer.local/", "http://x.localhost/",
            "file:///C:/Windows/win.ini", "ftp://example.com/", "gopher://127.0.0.1:6379/_INFO", "http://user:pw@example.com/",
        ] {
            let url = Url::parse(bad).unwrap();
            assert!(!outbound_url_ok(&url), "{bad} must be blocked");
        }
        for good in ["https://cdn.example.com/a.m3u8", "http://93.184.216.34/x.ts", "https://[2606:2800:220:1::1]/"] {
            assert!(outbound_url_ok(&Url::parse(good).unwrap()), "{good} must pass");
        }
        assert!(!public_ip(ip("198.18.0.1")) && !public_ip(ip("240.0.0.1")) && public_ip(ip("8.8.8.8")));
        // Tracker addresses: same rule for the host, whatever the scheme.
        assert!(!public_host(&Url::parse("udp://192.168.1.1:6969/announce").unwrap()));
        assert!(!public_host(&Url::parse("udp://localhost:6969/announce").unwrap()));
        assert!(public_host(&Url::parse("udp://tracker.opentrackr.org:1337/announce").unwrap()));
    }

    #[test]
    fn stream_urls_are_signed_per_stream() {
        let sig = sign_stream("ab12", "https://cdn.example/seg.ts");
        assert!(stream_signed("ab12", "https://cdn.example/seg.ts", &sig));
        assert!(!stream_signed("ab13", "https://cdn.example/seg.ts", &sig));
        assert!(!stream_signed("ab12", "https://cdn.example/seg.ts?x", &sig));
        assert!(!stream_signed("ab12", "http://127.0.0.1/", ""));
    }

    #[test]
    fn hosts_are_an_allow_list() {
        let (g, _db) = guard();
        assert!(g.host_ok(Via::Loopback, "127.0.0.1:43300") && g.host_ok(Via::Loopback, "LOCALHOST:43300"));
        for bad in ["evil.example:43300", "127.0.0.1", "127.0.0.1:80", "127.0.0.1:43300.evil.example", "localhost:5173", ""] {
            assert!(!g.host_ok(Via::Loopback, bad), "{bad}");
        }
        // The LAN listener only knows the address it is bound to.
        assert!(!g.host_ok(Via::Lan, "192.168.1.20:43300"));
        g.set_bound(Some(Ipv4Addr::new(192, 168, 1, 20)), None);
        assert!(g.host_ok(Via::Lan, "192.168.1.20:43300"));
        assert!(!g.host_ok(Via::Lan, "127.0.0.1:43300") && !g.host_ok(Via::Lan, "rebind.example:43300"));
        assert!(!g.host_ok(Via::Loopback, "192.168.1.20:43300"));
    }

    #[test]
    fn traversal_paths_are_refused() {
        for bad in [
            "/../x", "/a/../../Windows/win.ini", "/..%2f..%2fWindows/win.ini", "/%2e%2e/%2e%2e/Windows/win.ini", "/..%5c..%5cWindows%5cwin.ini",
            "/a\\..\\b", "/%252e%252e/x", "/C:/Windows/win.ini", "/api/torrents/x/file/..%2f..%2fwin.ini", "/x%00.js", "/%zz", "/./x",
        ] {
            assert!(!path_ok(bad), "{bad}");
        }
        for good in ["/", "/assets/index-abc.js", "/titel/tv/123", "/api/autodl/history/tv:1:2:3", "/api/stream/ab/r", "/brand/app-icon.svg"] {
            assert!(path_ok(good), "{good}");
        }
    }

    #[test]
    fn a_ticket_works_once() {
        let (g, _db) = guard();
        let t = g.ticket(Duration::from_secs(60));
        assert!(!g.redeem("nope"));
        assert!(g.redeem(&t));
        assert!(!g.redeem(&t));
        let old = g.ticket(Duration::ZERO);
        assert!(!g.redeem(&old));
    }

    #[test]
    fn pairing_pin_token_and_lockout() {
        let (g, db) = guard();
        let phone = ip("192.168.1.50");
        assert_eq!(g.pair(&db, phone, Some("000000"), None, "x"), Err(PairError::Closed));
        g.pairing_start();
        let (pin, token) = lock(&g.pairing).as_ref().map(|p| (p.pin.clone(), p.token.clone())).unwrap();
        assert_eq!(pin.len(), 6);
        let wrong = if pin == "123456" { "654321" } else { "123456" };
        // Four wrong tries are answered "wrong", the fifth locks the address, and then even the right PIN waits.
        for _ in 0..4 {
            assert_eq!(g.pair(&db, phone, Some(wrong), None, "x"), Err(PairError::Wrong));
        }
        assert!(matches!(g.pair(&db, phone, Some(wrong), None, "x"), Err(PairError::Locked(_))));
        assert!(matches!(g.pair(&db, phone, Some(&pin), None, "x"), Err(PairError::Locked(_))));
        // Another device pairs with the QR token; the cookie value names a device whose token is stored hashed.
        let tablet = ip("192.168.1.51");
        let value = g.pair(&db, tablet, None, Some(&token), "Mozilla/5.0 (Linux; Android 14) Chrome/130.0 Mobile Safari/537.36").unwrap();
        let id = g.device_of(&db, &value, tablet).unwrap();
        let stored = lock(&g.devices)[0].clone();
        assert_eq!((stored.id.as_str(), stored.name.as_str()), (id.as_str(), "Android, Chrome"));
        assert!(!value.contains(&stored.hash));
        // PIN and token are used up.
        assert_eq!(g.pair(&db, tablet, None, Some(&token), "x"), Err(PairError::Closed));
        // A forged or revoked token is nobody.
        assert!(g.device_of(&db, &format!("{id}.{}", "0".repeat(64)), tablet).is_none());
        assert!(g.revoke(&db, &id));
        assert!(g.device_of(&db, &value, tablet).is_none());
        // Devices survive a restart (and a revoked one stays gone).
        assert!(Guard::new(&db, 43300, false).devices.lock().unwrap().is_empty());
    }

    #[test]
    fn a_pin_dies_after_too_many_guesses_from_many_addresses() {
        let (g, db) = guard();
        g.pairing_start();
        let pin = lock(&g.pairing).as_ref().unwrap().pin.clone();
        let wrong = if pin == "111111" { "222222" } else { "111111" };
        for n in 0..FAILS_PER_PIN {
            let from = IpAddr::V4(Ipv4Addr::new(10, 0, 0, n as u8 + 1));
            assert_eq!(g.pair(&db, from, Some(wrong), None, "x"), Err(PairError::Wrong));
        }
        assert_eq!(g.pair(&db, ip("10.0.1.1"), Some(&pin), None, "x"), Err(PairError::Closed));
    }

    #[test]
    fn what_a_paired_device_may_do() {
        let ok = |m: &str, p: &str, q: &str| device_allowed(&m.parse().unwrap(), p, q);
        assert!(ok("GET", "/api/home", "") && ok("GET", "/api/torrents", "") && ok("GET", "/api/torrents/abc/play", "file=0"));
        assert!(ok("POST", "/api/scan/movie/45745", "") && ok("POST", "/api/torrents/start", "") && ok("PUT", "/api/progress/movie/1", ""));
        assert!(ok("POST", "/api/vpn/kill", "") && ok("DELETE", "/api/torrents/abc", "") && ok("PUT", "/api/watchlist/tv/1", ""));
        for (m, p, q) in [
            ("PUT", "/api/setup/tmdb", ""), ("PUT", "/api/setup/vpn", ""), ("DELETE", "/api/setup/vpn", ""), ("POST", "/api/setup/reset", ""),
            ("POST", "/api/setup/open", ""), ("PUT", "/api/torrents/settings", ""), ("POST", "/api/vpn/release", ""), ("POST", "/api/torrents/abc/open", ""),
            ("DELETE", "/api/torrents/abc", "files=1"), ("POST", "/api/export", ""), ("POST", "/api/import", ""), ("DELETE", "/api/maintenance/cache", ""),
            ("GET", "/api/maintenance", ""), ("GET", "/api/lan", ""), ("PUT", "/api/lan", ""), ("POST", "/api/lan/pairing", ""),
            ("DELETE", "/api/lan/devices/abc", ""), ("PUT", "/api/autodl/rules/tv/1", ""), ("POST", "/api/autodl/run", ""), ("GET", "/api/torrents/announce", ""),
        ] {
            assert!(!ok(m, p, q), "{m} {p}?{q} must stay with the PC");
        }
    }

    #[test]
    fn expensive_endpoints_run_dry() {
        let (g, _db) = guard();
        assert_eq!(Group::of(&Method::POST, "/api/scan/movie/1"), Some(Group::Scan));
        assert_eq!(Group::of(&Method::GET, "/api/torrents/tv/1"), Some(Group::TorrentSearch));
        assert_eq!(Group::of(&Method::GET, "/api/torrents/abc/play"), Some(Group::Ffmpeg));
        assert_eq!(Group::of(&Method::GET, "/api/home"), None);
        let burst = Group::AutoDl.limits().0 as usize;
        assert!((0..burst).all(|_| g.allow(Group::AutoDl, "pc")));
        assert!(!g.allow(Group::AutoDl, "pc"));
        assert!(g.allow(Group::AutoDl, "phone"), "each caller has its own bucket");
    }

    #[test]
    fn qr_code_is_an_svg() {
        let svg = qr_svg("http://192.168.1.20:43300/pair#t=abcdef").unwrap();
        assert!(svg.starts_with("<?xml") || svg.starts_with("<svg"));
        assert!(!svg.contains("<script"));
    }
}
