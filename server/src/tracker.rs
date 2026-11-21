//! Tracker announces through the VPN tunnel (UDP per BEP 15, HTTP per BEP 3/23), plus the local
//! "shim" tracker that hands the found peers (trackers and DHT) to the torrent engine. The engine
//! itself only ever talks to the shim, so it cannot announce over the normal network.

use std::{
    collections::{HashMap, HashSet},
    net::{Ipv4Addr, SocketAddrV4},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};

use axum::{
    extract::{RawQuery, State},
    http::header,
    response::IntoResponse,
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::{api::St, vpn::Vpn, wg::Tunnel};

/// Well-known public trackers, added to whatever the magnet/search result brings.
pub const DEFAULT_TRACKERS: &[&str] = &[
    "udp://tracker.opentrackr.org:1337/announce",
    "udp://open.stealth.si:80/announce",
    "udp://tracker.torrent.eu.org:451/announce",
    "udp://exodus.desync.com:6969/announce",
    "udp://open.demonii.com:1337/announce",
    "udp://explodie.org:6969/announce",
    "udp://tracker.dler.org:6969/announce",
    "http://tracker.opentrackr.org:1337/announce",
];
const MAX_TRACKERS: usize = 60;
/// The community-maintained list of working public trackers (github.com/ngosang/trackerslist).
const PUBLIC_LIST_URL: &str = "https://raw.githubusercontent.com/ngosang/trackerslist/master/trackers_all.txt";
const PUBLIC_LIST_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const ANNOUNCE_TIMEOUT: Duration = Duration::from_secs(12);
const SHIM_WAIT: Duration = Duration::from_secs(8);
/// How often the engine re-asks the shim.
pub const SHIM_INTERVAL: Duration = Duration::from_secs(20);

/// Where a peer address came from. Peers the engine knows that are in neither list came by
/// peer exchange (PEX) or dialled in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Tracker,
    Dht,
}

/// Peers known for one torrent.
#[derive(Default)]
pub struct Swarm {
    peers: Mutex<HashMap<SocketAddrV4, Source>>,
    found: Notify,
}

impl Swarm {
    pub fn len(&self) -> usize {
        self.peers.lock().unwrap().len()
    }

    /// (from trackers, from the DHT); a peer both know counts for whoever named it first.
    pub fn counts(&self) -> (usize, usize) {
        let peers = self.peers.lock().unwrap();
        let dht = peers.values().filter(|s| **s == Source::Dht).count();
        (peers.len() - dht, dht)
    }

    pub fn source_of(&self, peer: &SocketAddrV4) -> Option<Source> {
        self.peers.lock().unwrap().get(peer).copied()
    }

    pub fn add(&self, peers: Vec<SocketAddrV4>, source: Source) {
        let mut map = self.peers.lock().unwrap();
        for p in peers.into_iter().filter(|p| p.port() != 0 && !p.ip().is_unspecified()) {
            map.entry(p).or_insert(source);
        }
        drop(map);
        self.found.notify_waiters();
    }
}

/// The public tracker list: fetched through the tunnel, kept in `data/cache/trackers.txt` for a day.
pub struct PublicList {
    file: PathBuf,
    cached: Mutex<Vec<String>>,
}

impl PublicList {
    pub fn new(file: PathBuf) -> Self {
        Self { file, cached: Mutex::default() }
    }

    fn parse(text: &str) -> Vec<String> {
        text.lines().map(str::trim).filter(|l| l.starts_with("udp://") || l.starts_with("http://") || l.starts_with("https://")).map(str::to_owned).collect()
    }

    async fn get(&self, vpn: &Vpn) -> Vec<String> {
        let known = self.cached.lock().unwrap().clone();
        if !known.is_empty() {
            return known;
        }
        let on_disk = tokio::fs::read_to_string(&self.file).await.map(|t| Self::parse(&t)).unwrap_or_default();
        let fresh = tokio::fs::metadata(&self.file)
            .await
            .ok()
            .and_then(|m| m.modified().ok())
            .is_some_and(|t| SystemTime::now().duration_since(t).is_ok_and(|age| age < PUBLIC_LIST_TTL));
        let mut list = on_disk;
        if !fresh || list.is_empty() {
            let fetched = async { vpn.http.get(PUBLIC_LIST_URL).send().await.ok()?.error_for_status().ok()?.text().await.ok() }.await;
            if let Some(found) = fetched.map(|t| Self::parse(&t)).filter(|l| l.len() >= 5) {
                let _ = tokio::fs::write(&self.file, found.join("\n")).await;
                list = found;
            }
        }
        if !list.is_empty() {
            *self.cached.lock().unwrap() = list.clone();
        }
        list
    }
}

/// Magnet/search trackers first, then the defaults, then the public list; deduplicated,
/// announce-able schemes only.
pub fn tracker_list(given: &[String], public: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    given
        .iter()
        .map(String::as_str)
        .chain(DEFAULT_TRACKERS.iter().copied())
        .chain(public.iter().map(String::as_str))
        .filter(|t| t.starts_with("udp://") || t.starts_with("http://") || t.starts_with("https://"))
        // A magnet link comes from a search site: its trackers must not point into the home network.
        .filter(|t| Url::parse(t).is_ok_and(|u| crate::guard::public_host(&u)))
        .filter(|t| !t.contains(crate::vpn::SHIM_HOST) && seen.insert(t.to_ascii_lowercase()))
        .take(MAX_TRACKERS)
        .map(str::to_owned)
        .collect()
}

fn compact_peers(bytes: &[u8]) -> Vec<SocketAddrV4> {
    bytes
        .chunks_exact(6)
        .map(|c| SocketAddrV4::new(Ipv4Addr::new(c[0], c[1], c[2], c[3]), u16::from_be_bytes([c[4], c[5]])))
        .collect()
}

async fn announce_udp(tunnel: &Arc<Tunnel>, url: &Url, hash: &[u8; 20], peer_id: &[u8; 20], port: u16) -> Option<Vec<SocketAddrV4>> {
    let ip = tunnel.resolve(url.host_str()?).await.ok()?;
    let to = SocketAddrV4::new(ip, url.port()?);
    let socket = tunnel.udp().ok()?;
    let mut buf = [0u8; 2048];

    let tid: u32 = rand::random();
    let mut connect = Vec::with_capacity(16);
    connect.extend_from_slice(&0x0417_2710_1980u64.to_be_bytes());
    connect.extend_from_slice(&0u32.to_be_bytes());
    connect.extend_from_slice(&tid.to_be_bytes());
    socket.send_to(&connect, to).ok()?;
    let n = tokio::time::timeout(Duration::from_secs(5), socket.recv(&mut buf)).await.ok()?.ok()?;
    if n < 16 || buf[..4] != [0, 0, 0, 0] || buf[4..8] != tid.to_be_bytes() {
        return None;
    }
    let connection_id = buf[8..16].to_vec();

    let tid: u32 = rand::random();
    let mut req = Vec::with_capacity(98);
    req.extend_from_slice(&connection_id);
    req.extend_from_slice(&1u32.to_be_bytes());
    req.extend_from_slice(&tid.to_be_bytes());
    req.extend_from_slice(hash);
    req.extend_from_slice(peer_id);
    req.extend_from_slice(&0u64.to_be_bytes()); // downloaded
    req.extend_from_slice(&u64::from(u32::MAX).to_be_bytes()); // left (unknown before metadata)
    req.extend_from_slice(&0u64.to_be_bytes()); // uploaded
    req.extend_from_slice(&2u32.to_be_bytes()); // event: started
    req.extend_from_slice(&0u32.to_be_bytes()); // ip: sender
    req.extend_from_slice(&rand::random::<u32>().to_be_bytes()); // key
    req.extend_from_slice(&200i32.to_be_bytes()); // num_want
    req.extend_from_slice(&port.to_be_bytes()); // the VPN's forwarded port when there is one
    socket.send_to(&req, to).ok()?;
    let n = tokio::time::timeout(Duration::from_secs(5), socket.recv(&mut buf)).await.ok()?.ok()?;
    if n < 20 || buf[..4] != [0, 0, 0, 1] || buf[4..8] != tid.to_be_bytes() {
        return None;
    }
    Some(compact_peers(&buf[20..n]))
}

fn percent(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("%{b:02X}")).collect()
}

/// Value of the bencoded byte string stored under `key` (enough for compact tracker replies).
fn bencode_bytes<'a>(body: &'a [u8], key: &str) -> Option<&'a [u8]> {
    let needle = format!("{}:{key}", key.len());
    let at = body.windows(needle.len()).position(|w| w == needle.as_bytes())? + needle.len();
    let colon = at + body[at..].iter().position(|b| *b == b':')?;
    let len: usize = std::str::from_utf8(&body[at..colon]).ok()?.parse().ok()?;
    body.get(colon + 1..colon + 1 + len)
}

async fn announce_http(vpn: &Vpn, url: &Url, hash: &[u8; 20], peer_id: &[u8; 20], port: u16) -> Option<Vec<SocketAddrV4>> {
    let sep = if url.query().is_some() { '&' } else { '?' };
    let full = format!(
        "{url}{sep}info_hash={}&peer_id={}&port={port}&uploaded=0&downloaded=0&left={}&compact=1&numwant=200&event=started",
        percent(hash),
        percent(peer_id),
        u32::MAX
    );
    let body = vpn.http.get(full).send().await.ok()?.bytes().await.ok()?;
    bencode_bytes(&body, "peers").map(compact_peers)
}

/// Announces until cancelled; quickly while no peer is known, then every few minutes.
pub async fn run(vpn: Arc<Vpn>, hash: [u8; 20], peer_id: [u8; 20], given: Vec<String>, swarm: Arc<Swarm>, cancel: CancellationToken) {
    // The first round does not wait for the public list: the torrent's own trackers go out at once.
    let mut urls: Vec<Url> = tracker_list(&given, &[]).iter().filter_map(|t| Url::parse(t).ok()).collect();
    let mut with_public = false;
    loop {
        let announced = vpn.forwarded_port();
        if let Some(tunnel) = vpn.current() {
            // 0 would tell trackers "no port"; 6881 is the customary placeholder without forwarding.
            let port = announced.unwrap_or(6881);
            let rounds = urls.iter().map(|url| {
                let (vpn, tunnel, swarm) = (&vpn, &tunnel, &swarm);
                async move {
                    let job = async {
                        match url.scheme() {
                            "udp" => announce_udp(tunnel, url, &hash, &peer_id, port).await,
                            _ => announce_http(vpn, url, &hash, &peer_id, port).await,
                        }
                    };
                    if let Ok(Some(peers)) = tokio::time::timeout(ANNOUNCE_TIMEOUT, job).await {
                        swarm.add(peers, Source::Tracker);
                    }
                }
            });
            tokio::select! {
                _ = futures_util::future::join_all(rounds) => {}
                _ = cancel.cancelled() => return,
            }
        }
        if !with_public && vpn.current().is_some() {
            let public = tokio::select! {
                list = vpn.trackers.get(&vpn) => list,
                _ = cancel.cancelled() => return,
            };
            urls = tracker_list(&given, &public).iter().filter_map(|t| Url::parse(t).ok()).collect();
            with_public = true;
            // The added trackers are asked right away.
            continue;
        }
        let pause = if swarm.len() == 0 { 10 } else { 180 };
        tokio::select! {
            _ = port_changed_or(&vpn, announced, Duration::from_secs(pause)) => {}
            _ = cancel.cancelled() => return,
        }
    }
}

/// Waits `pause`, but returns early when the VPN's forwarded port is not `announced` any more:
/// trackers and the DHT should learn the port incoming peers can reach as soon as it exists.
pub async fn port_changed_or(vpn: &Vpn, announced: Option<u16>, pause: Duration) {
    let until = tokio::time::Instant::now() + pause;
    while tokio::time::Instant::now() < until {
        let now = vpn.forwarded_port();
        if now.is_some() && now != announced {
            return;
        }
        tokio::time::sleep(Duration::from_secs(2).min(until - tokio::time::Instant::now())).await;
    }
}

fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(20);
    let mut i = 0;
    while i < b.len() {
        let hex = (b[i] == b'%').then(|| s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())).flatten();
        match hex {
            Some(v) => {
                out.push(v);
                i += 3;
            }
            None => {
                out.push(b[i]);
                i += 1;
            }
        }
    }
    out
}

/// `GET /api/torrents/announce`: the shim tracker. Waits briefly for the first peers so a fresh
/// torrent does not lose a whole interval.
pub async fn shim(State(st): St, RawQuery(query): RawQuery) -> impl IntoResponse {
    let hash = query
        .as_deref()
        .unwrap_or_default()
        .split('&')
        .find_map(|kv| kv.strip_prefix("info_hash="))
        .map(percent_decode)
        .unwrap_or_default();
    let id: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    let Some(swarm) = st.torrents.swarm(&id) else {
        return ([(header::CONTENT_TYPE, "text/plain")], b"d14:failure reason15:unknown torrente".to_vec());
    };
    if swarm.len() == 0 {
        let _ = tokio::time::timeout(SHIM_WAIT, swarm.found.notified()).await;
    }
    let peers: Vec<u8> = swarm
        .peers
        .lock()
        .unwrap()
        .keys()
        .flat_map(|p| p.ip().octets().into_iter().chain(p.port().to_be_bytes()))
        .collect();
    let mut body = format!("d8:intervali{}e5:peers{}:", SHIM_INTERVAL.as_secs(), peers.len()).into_bytes();
    body.extend_from_slice(&peers);
    body.push(b'e');
    ([(header::CONTENT_TYPE, "text/plain")], body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_compact_peers_from_a_tracker_reply() {
        let mut body = b"d8:completei5e8:intervali1800e5:peers12:".to_vec();
        body.extend_from_slice(&[1, 2, 3, 4, 0x1a, 0xe1, 5, 6, 7, 8, 0, 80]);
        body.push(b'e');
        let peers = compact_peers(bencode_bytes(&body, "peers").unwrap());
        assert_eq!(peers, vec!["1.2.3.4:6881".parse().unwrap(), "5.6.7.8:80".parse().unwrap()]);
    }

    #[test]
    fn tracker_list_drops_unusable_and_duplicate_urls() {
        let public = PublicList::parse("udp://b.example:2/announce\n\nudp://a.example:1/announce\nwss://c.example\nhttps://d.example/announce\n");
        assert_eq!(public.len(), 3);
        let list = tracker_list(
            &["wss://tracker.example/".into(), "udp://Tracker.Opentrackr.org:1337/announce".into(), "udp://a.example:1/announce".into()],
            &public,
        );
        assert_eq!(list[0], "udp://Tracker.Opentrackr.org:1337/announce");
        assert_eq!(list[1], "udp://a.example:1/announce");
        // given (2 usable) + defaults without the duplicate + 2 new public ones
        assert_eq!(list.len(), DEFAULT_TRACKERS.len() + 1 + 2);
        assert!(list.ends_with(&["udp://b.example:2/announce".to_owned(), "https://d.example/announce".to_owned()]));
        assert!(!list.iter().any(|t| t.starts_with("wss")));
        let many: Vec<String> = (0..200).map(|i| format!("udp://t{i}.example:1/announce")).collect();
        assert_eq!(tracker_list(&[], &many).len(), MAX_TRACKERS);
    }

    #[test]
    fn swarm_remembers_who_named_a_peer_first() {
        let swarm = Swarm::default();
        let p = |n: u8| SocketAddrV4::new(Ipv4Addr::new(1, 1, 1, n), 6881);
        swarm.add(vec![p(1), p(2), SocketAddrV4::new(Ipv4Addr::new(1, 1, 1, 9), 0)], Source::Tracker);
        swarm.add(vec![p(2), p(3), p(4)], Source::Dht);
        assert_eq!((swarm.len(), swarm.counts()), (4, (2, 2)));
        assert_eq!((swarm.source_of(&p(2)), swarm.source_of(&p(3)), swarm.source_of(&p(7))), (Some(Source::Tracker), Some(Source::Dht), None));
    }

    #[test]
    fn decodes_binary_info_hash() {
        assert_eq!(percent_decode("%08%ADa%ff~"), vec![0x08, 0xad, b'a', 0xff, b'~']);
    }
}
