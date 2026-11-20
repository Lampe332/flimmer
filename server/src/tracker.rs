//! Tracker announces through the VPN tunnel (UDP per BEP 15, HTTP per BEP 3/23), plus the local
//! "shim" tracker that hands the found peers to the torrent engine. The engine itself only ever
//! talks to the shim, so it cannot announce over the normal network.

use std::{
    collections::HashSet,
    net::{Ipv4Addr, SocketAddrV4},
    sync::{Arc, Mutex},
    time::Duration,
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
const MAX_TRACKERS: usize = 24;
const ANNOUNCE_TIMEOUT: Duration = Duration::from_secs(12);
const SHIM_WAIT: Duration = Duration::from_secs(8);
/// How often the engine re-asks the shim.
pub const SHIM_INTERVAL: Duration = Duration::from_secs(20);

/// Peers known for one torrent.
#[derive(Default)]
pub struct Swarm {
    peers: Mutex<HashSet<SocketAddrV4>>,
    found: Notify,
}

impl Swarm {
    pub fn len(&self) -> usize {
        self.peers.lock().unwrap().len()
    }

    fn add(&self, peers: Vec<SocketAddrV4>) {
        let mut set = self.peers.lock().unwrap();
        set.extend(peers.into_iter().filter(|p| p.port() != 0 && !p.ip().is_unspecified()));
        drop(set);
        self.found.notify_waiters();
    }
}

/// Magnet/search trackers first, then the defaults; deduplicated, announce-able schemes only.
pub fn tracker_list(given: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    given
        .iter()
        .map(String::as_str)
        .chain(DEFAULT_TRACKERS.iter().copied())
        .filter(|t| t.starts_with("udp://") || t.starts_with("http://") || t.starts_with("https://"))
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

async fn announce_udp(tunnel: &Arc<Tunnel>, url: &Url, hash: &[u8; 20], peer_id: &[u8; 20]) -> Option<Vec<SocketAddrV4>> {
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
    req.extend_from_slice(&6881u16.to_be_bytes()); // port (we accept no incoming connections)
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

async fn announce_http(vpn: &Vpn, url: &Url, hash: &[u8; 20], peer_id: &[u8; 20]) -> Option<Vec<SocketAddrV4>> {
    let sep = if url.query().is_some() { '&' } else { '?' };
    let full = format!(
        "{url}{sep}info_hash={}&peer_id={}&port=6881&uploaded=0&downloaded=0&left={}&compact=1&numwant=200&event=started",
        percent(hash),
        percent(peer_id),
        u32::MAX
    );
    let body = vpn.http.get(full).send().await.ok()?.bytes().await.ok()?;
    bencode_bytes(&body, "peers").map(compact_peers)
}

/// Announces until cancelled; quickly while no peer is known, then every few minutes.
pub async fn run(vpn: Arc<Vpn>, hash: [u8; 20], peer_id: [u8; 20], trackers: Vec<String>, swarm: Arc<Swarm>, cancel: CancellationToken) {
    let urls: Vec<Url> = trackers.iter().filter_map(|t| Url::parse(t).ok()).collect();
    loop {
        if let Some(tunnel) = vpn.current() {
            let rounds = urls.iter().map(|url| {
                let (vpn, tunnel, swarm) = (&vpn, &tunnel, &swarm);
                async move {
                    let job = async {
                        match url.scheme() {
                            "udp" => announce_udp(tunnel, url, &hash, &peer_id).await,
                            _ => announce_http(vpn, url, &hash, &peer_id).await,
                        }
                    };
                    if let Ok(Some(peers)) = tokio::time::timeout(ANNOUNCE_TIMEOUT, job).await {
                        swarm.add(peers);
                    }
                }
            });
            tokio::select! {
                _ = futures_util::future::join_all(rounds) => {}
                _ = cancel.cancelled() => return,
            }
        }
        let pause = if swarm.len() == 0 { 10 } else { 180 };
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(pause)) => {}
            _ = cancel.cancelled() => return,
        }
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
        .iter()
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
        let list = tracker_list(&[
            "wss://tracker.example/".into(),
            "udp://Tracker.Opentrackr.org:1337/announce".into(),
            "udp://a.example:1/announce".into(),
        ]);
        assert_eq!(list[0], "udp://Tracker.Opentrackr.org:1337/announce");
        assert_eq!(list[1], "udp://a.example:1/announce");
        assert!(list.len() == DEFAULT_TRACKERS.len() + 1 && !list.iter().any(|t| t.starts_with("wss")));
    }

    #[test]
    fn decodes_binary_info_hash() {
        assert_eq!(percent_decode("%08%ADa%ff~"), vec![0x08, 0xad, b'a', 0xff, b'~']);
    }
}
