//! A client-only mainline DHT (BEP 5, read-only per BEP 43) that speaks through the VPN tunnel.
//! librqbit's own DHT would use a normal UDP socket of the PC, so it stays off; the peers found
//! here go into the torrent's [`Swarm`] and reach the engine through the tracker shim.

use std::{
    collections::HashSet,
    net::{Ipv4Addr, SocketAddrV4},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use tokio_util::sync::CancellationToken;

use crate::{
    benc::{self, Benc},
    tracker::{Source, Swarm},
    vpn::Vpn,
    wg::Tunnel,
};

const BOOTSTRAP: &[(&str, u16)] = &[
    ("router.bittorrent.com", 6881),
    ("dht.transmissionbt.com", 6881),
    ("router.utorrent.com", 6881),
    ("dht.libtorrent.org", 25401),
];
/// Queries waiting for an answer at once.
const PARALLEL: usize = 24;
const MAX_QUERIES: usize = 320;
const QUERY_TIMEOUT: Duration = Duration::from_millis(1500);
const LOOKUP_TIME: Duration = Duration::from_secs(12);
/// The lookup is done when this many of the closest nodes have answered.
const CLOSEST: usize = 8;
const BOOK_SIZE: usize = 200;

type Id = [u8; 20];

/// Nodes that answered lately: the next lookup starts from them instead of the bootstrap routers.
pub struct NodeBook {
    id: Id,
    nodes: Mutex<Vec<(Id, SocketAddrV4)>>,
}

impl Default for NodeBook {
    fn default() -> Self {
        Self { id: rand::random(), nodes: Mutex::default() }
    }
}

impl NodeBook {
    pub fn len(&self) -> usize {
        self.nodes.lock().unwrap().len()
    }

    fn remember(&self, fresh: Vec<(Id, SocketAddrV4)>) {
        let mut nodes = self.nodes.lock().unwrap();
        nodes.retain(|n| !fresh.iter().any(|f| f.1 == n.1));
        nodes.splice(0..0, fresh);
        nodes.truncate(BOOK_SIZE);
    }
}

#[derive(PartialEq)]
enum State {
    New,
    Sent(Instant),
    Replied,
    Failed,
}

struct Cand {
    addr: SocketAddrV4,
    /// Unknown for bootstrap routers.
    id: Option<Id>,
    state: State,
    token: Option<Vec<u8>>,
}

fn distance(id: &Option<Id>, target: &Id) -> Id {
    match id {
        Some(id) => std::array::from_fn(|i| id[i] ^ target[i]),
        None => [0xff; 20],
    }
}

fn get_peers(id: &Id, hash: &Id, tid: u16) -> Vec<u8> {
    let mut m = b"d1:ad2:id20:".to_vec();
    m.extend_from_slice(id);
    m.extend_from_slice(b"9:info_hash20:");
    m.extend_from_slice(hash);
    m.extend_from_slice(b"e1:q9:get_peers2:roi1e1:t2:");
    m.extend_from_slice(&tid.to_be_bytes());
    m.extend_from_slice(b"1:y1:qe");
    m
}

fn announce_peer(id: &Id, hash: &Id, port: u16, token: &[u8]) -> Vec<u8> {
    let mut m = b"d1:ad2:id20:".to_vec();
    m.extend_from_slice(id);
    m.extend_from_slice(b"12:implied_porti0e9:info_hash20:");
    m.extend_from_slice(hash);
    m.extend_from_slice(format!("4:porti{port}e5:token{}:", token.len()).as_bytes());
    m.extend_from_slice(token);
    m.extend_from_slice(b"e1:q13:announce_peer2:roi1e1:t2:ap1:y1:qe");
    m
}

/// What one `get_peers` answer carries.
#[derive(Default, Debug, PartialEq)]
struct Reply {
    tid: Option<u16>,
    id: Option<Id>,
    nodes: Vec<(Id, SocketAddrV4)>,
    peers: Vec<SocketAddrV4>,
    token: Option<Vec<u8>>,
}

fn addr(c: &[u8]) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::new(c[0], c[1], c[2], c[3]), u16::from_be_bytes([c[4], c[5]]))
}

fn usable(a: &SocketAddrV4) -> bool {
    let ip = a.ip();
    a.port() != 0 && !(ip.is_unspecified() || ip.is_loopback() || ip.is_private() || ip.is_multicast() || ip.is_broadcast() || ip.is_link_local())
}

fn parse_reply(data: &[u8]) -> Option<Reply> {
    let msg = benc::decode(data)?;
    if msg.get("y").and_then(Benc::bytes) != Some(b"r") {
        return None;
    }
    let r = msg.get("r")?;
    Some(Reply {
        tid: msg.get("t").and_then(Benc::bytes).filter(|t| t.len() == 2).map(|t| u16::from_be_bytes([t[0], t[1]])),
        id: r.get("id").and_then(Benc::bytes).and_then(|b| Id::try_from(b).ok()),
        nodes: r
            .get("nodes")
            .and_then(Benc::bytes)
            .unwrap_or_default()
            .chunks_exact(26)
            .filter_map(|c| Some((Id::try_from(&c[..20]).ok()?, addr(&c[20..]))))
            .filter(|(_, a)| usable(a))
            .collect(),
        peers: r
            .get("values")
            .map(Benc::list)
            .unwrap_or_default()
            .iter()
            .filter_map(Benc::bytes)
            .filter(|b| b.len() == 6)
            .map(addr)
            .filter(usable)
            .collect(),
        token: r.get("token").and_then(Benc::bytes).map(<[u8]>::to_vec),
    })
}

/// Iterative `get_peers` lookup for one info hash; announces `port` to the closest nodes when given.
pub async fn lookup(tunnel: &Arc<Tunnel>, book: &NodeBook, hash: Id, port: Option<u16>) -> Vec<SocketAddrV4> {
    let Ok(socket) = tunnel.udp_sized(256, 256 * 1024) else { return Vec::new() };
    let mut cands: Vec<Cand> = Vec::new();
    let mut seen: HashSet<SocketAddrV4> = HashSet::new();
    let mut add = |cands: &mut Vec<Cand>, addr: SocketAddrV4, id: Option<Id>| {
        if seen.insert(addr) && cands.len() < 4000 {
            cands.push(Cand { addr, id, state: State::New, token: None });
        }
    };
    for (id, addr) in book.nodes.lock().unwrap().iter().take(60) {
        add(&mut cands, *addr, Some(*id));
    }
    // The routers are asked every time: they are cheap and the book may be stale.
    for (host, port) in BOOTSTRAP {
        if let Ok(Ok(ip)) = tokio::time::timeout(Duration::from_secs(4), tunnel.resolve(host)).await {
            add(&mut cands, SocketAddrV4::new(ip, *port), None);
        }
    }

    let mut peers: HashSet<SocketAddrV4> = HashSet::new();
    let deadline = Instant::now() + LOOKUP_TIME;
    let mut sent = 0usize;
    let mut buf = vec![0u8; 4096];
    while Instant::now() < deadline && tunnel.is_alive() {
        for c in cands.iter_mut() {
            if matches!(c.state, State::Sent(at) if at.elapsed() > QUERY_TIMEOUT) {
                c.state = State::Failed;
            }
        }
        let mut order: Vec<usize> = (0..cands.len()).filter(|i| cands[*i].state != State::Failed).collect();
        order.sort_by_key(|i| distance(&cands[*i].id, &hash));
        let flying = cands.iter().filter(|c| matches!(c.state, State::Sent(_))).count();
        // Done when the closest known nodes have all answered.
        let settled = order.len() >= CLOSEST && order.iter().take(CLOSEST).all(|i| cands[*i].state == State::Replied);
        let fresh: Vec<usize> = order.iter().copied().filter(|i| cands[*i].state == State::New).take(PARALLEL.saturating_sub(flying)).collect();
        if (settled || fresh.is_empty() || sent >= MAX_QUERIES) && flying == 0 {
            break;
        }
        if !settled && sent < MAX_QUERIES {
            for i in fresh {
                // A full send buffer leaves the node for the next round.
                if socket.send_to(&get_peers(&book.id, &hash, i as u16), cands[i].addr).is_ok() {
                    cands[i].state = State::Sent(Instant::now());
                    sent += 1;
                }
            }
        }
        let Ok(Ok((n, from))) = tokio::time::timeout(Duration::from_millis(100), socket.recv_from(&mut buf)).await else { continue };
        let Some(reply) = parse_reply(&buf[..n]) else { continue };
        let Some(c) = reply.tid.and_then(|t| cands.get_mut(t as usize)).filter(|c| c.addr == from) else { continue };
        c.state = State::Replied;
        c.id = reply.id.or(c.id);
        c.token = reply.token;
        peers.extend(reply.peers);
        for (id, addr) in reply.nodes {
            add(&mut cands, addr, Some(id));
        }
    }

    let mut replied: Vec<&Cand> = cands.iter().filter(|c| c.state == State::Replied && c.id.is_some()).collect();
    replied.sort_by_key(|c| distance(&c.id, &hash));
    if let Some(port) = port {
        for c in replied.iter().take(CLOSEST) {
            if let Some(token) = &c.token {
                let _ = socket.send_to(&announce_peer(&book.id, &hash, port, token), c.addr);
            }
        }
        // Give the stack a moment to put the announces on the wire before the socket closes.
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    book.remember(replied.iter().take(64).filter_map(|c| Some((c.id?, c.addr))).collect());
    peers.into_iter().collect()
}

/// Looks the torrent up until cancelled: often while it has few peers, then every few minutes.
pub async fn run(vpn: Arc<Vpn>, hash: Id, swarm: Arc<Swarm>, cancel: CancellationToken) {
    loop {
        let port = vpn.forwarded_port();
        if let Some(tunnel) = vpn.current() {
            let found = tokio::select! {
                peers = lookup(&tunnel, &vpn.nodes, hash, port) => peers,
                _ = cancel.cancelled() => return,
            };
            swarm.add(found, Source::Dht);
        }
        let pause = if swarm.len() < 30 { 20 } else { 240 };
        tokio::select! {
            _ = crate::tracker::port_changed_or(&vpn, port, Duration::from_secs(pause)) => {}
            _ = cancel.cancelled() => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_queries_that_decode_as_krpc() {
        let (id, hash) = ([1u8; 20], [2u8; 20]);
        let q = get_peers(&id, &hash, 0x0102);
        let m = benc::decode(&q).unwrap();
        assert_eq!(m.get("q").and_then(Benc::bytes), Some(&b"get_peers"[..]));
        assert_eq!(m.get("t").and_then(Benc::bytes), Some(&[1u8, 2][..]));
        assert_eq!(m.get("ro"), Some(&Benc::Int(1)));
        assert_eq!(m.get("a").and_then(|a| a.get("info_hash")).and_then(Benc::bytes), Some(&hash[..]));

        let a = announce_peer(&id, &hash, 51413, b"tok");
        let m = benc::decode(&a).unwrap();
        let args = m.get("a").unwrap();
        assert_eq!(m.get("q").and_then(Benc::bytes), Some(&b"announce_peer"[..]));
        assert_eq!((args.get("port"), args.get("implied_port")), (Some(&Benc::Int(51413)), Some(&Benc::Int(0))));
        assert_eq!(args.get("token").and_then(Benc::bytes), Some(&b"tok"[..]));
    }

    #[test]
    fn reads_nodes_peers_and_token_from_a_reply() {
        let mut msg = b"d1:rd2:id20:".to_vec();
        msg.extend_from_slice(&[9u8; 20]);
        msg.extend_from_slice(b"5:nodes52:");
        msg.extend_from_slice(&[3u8; 20]);
        msg.extend_from_slice(&[8, 8, 8, 8, 0x1a, 0xe1]);
        // A node on a private address is dropped.
        msg.extend_from_slice(&[4u8; 20]);
        msg.extend_from_slice(&[192, 168, 1, 5, 0x1a, 0xe1]);
        msg.extend_from_slice(b"5:token2:tk6:valuesl6:");
        msg.extend_from_slice(&[1, 2, 3, 4, 0, 80]);
        msg.extend_from_slice(b"6:");
        msg.extend_from_slice(&[5, 6, 7, 8, 0, 0]);
        msg.extend_from_slice(b"ee1:t2:\x00\x051:y1:re");
        let r = parse_reply(&msg).unwrap();
        assert_eq!((r.tid, r.id, r.token.as_deref()), (Some(5), Some([9u8; 20]), Some(&b"tk"[..])));
        assert_eq!(r.nodes, vec![([3u8; 20], "8.8.8.8:6881".parse().unwrap())]);
        // Port 0 is not a peer.
        assert_eq!(r.peers, vec!["1.2.3.4:80".parse().unwrap()]);
        assert!(parse_reply(b"d1:eli201e5:errore1:t2:aa1:y1:ee").is_none());
    }

    #[test]
    fn orders_by_xor_distance_and_keeps_fresh_nodes_first() {
        let target = [0u8; 20];
        let mut near = [0u8; 20];
        near[19] = 1;
        assert!(distance(&Some(near), &target) < distance(&Some([0x80; 20]), &target));
        assert!(distance(&Some([0x80; 20]), &target) < distance(&None, &target));

        let book = NodeBook::default();
        let a = |n: u8| SocketAddrV4::new(Ipv4Addr::new(9, 9, 9, n), 1);
        book.remember(vec![([1; 20], a(1)), ([2; 20], a(2))]);
        book.remember(vec![([2; 20], a(2)), ([3; 20], a(3))]);
        assert_eq!(book.nodes.lock().unwrap().iter().map(|n| n.1).collect::<Vec<_>>(), vec![a(2), a(3), a(1)]);
    }
}
