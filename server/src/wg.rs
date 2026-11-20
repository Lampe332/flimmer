//! Userspace WireGuard tunnel: boringtun speaks the WireGuard protocol, smoltcp is the TCP/IP stack on top.
//! No network adapter, no routes, no admin rights: only sockets opened through [`Tunnel`] use the VPN,
//! everything else on the PC keeps its normal route.

use std::{
    collections::{HashMap, VecDeque},
    future::poll_fn,
    io,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    pin::Pin,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};

use base64::Engine;
use boringtun::{
    noise::{Tunn, TunnResult},
    x25519::{PublicKey, StaticSecret},
};
use smoltcp::{
    iface::{Config, Interface, SocketHandle, SocketSet},
    phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken},
    socket::{dns, tcp, udp},
    time::Instant as StackInstant,
    wire::{DnsQueryType, HardwareAddress, IpAddress, IpCidr, IpEndpoint},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::UdpSocket,
    sync::Notify,
};
use tokio_util::sync::CancellationToken;

const DEFAULT_MTU: usize = 1400;
const TCP_RX: usize = 256 * 1024;
const TCP_TX: usize = 64 * 1024;
const HANDSHAKE_WAIT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DNS_TIMEOUT: Duration = Duration::from_secs(8);

/// A parsed WireGuard `.conf`. Holds key material: no `Debug`, and no error ever quotes a value.
pub struct WgConfig {
    private: [u8; 32],
    peer: [u8; 32],
    preshared: Option<[u8; 32]>,
    address: Ipv4Addr,
    dns: Vec<Ipv4Addr>,
    endpoint: String,
    keepalive: u16,
    mtu: usize,
}

fn key(map: &HashMap<String, String>, name: &str, label: &str) -> Result<[u8; 32], String> {
    let text = map.get(name).ok_or_else(|| format!("VPN config: {label} is missing"))?;
    base64::engine::general_purpose::STANDARD
        .decode(text)
        .ok()
        .and_then(|b| <[u8; 32]>::try_from(b).ok())
        .ok_or_else(|| format!("VPN config: {label} is not a valid WireGuard key"))
}

impl WgConfig {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut section = String::new();
        let mut map = HashMap::new();
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or_default().trim();
            if let Some(name) = line.strip_prefix('[') {
                section = name.trim_end_matches(']').trim().to_ascii_lowercase();
            } else if let Some((k, v)) = line.split_once('=') {
                map.insert(format!("{section}.{}", k.trim().to_ascii_lowercase()), v.trim().to_owned());
            }
        }
        let list = |name: &str| -> Vec<Ipv4Addr> {
            map.get(name)
                .map(|v| v.split(',').filter_map(|a| a.trim().split('/').next()?.parse().ok()).collect())
                .unwrap_or_default()
        };
        let address =
            *list("interface.address").first().ok_or("VPN config: Interface Address (IPv4) is missing")?;
        let endpoint = map.get("peer.endpoint").cloned().ok_or("VPN config: Peer Endpoint is missing")?;
        Ok(Self {
            private: key(&map, "interface.privatekey", "PrivateKey")?,
            peer: key(&map, "peer.publickey", "PublicKey")?,
            preshared: match map.contains_key("peer.presharedkey") {
                true => Some(key(&map, "peer.presharedkey", "PresharedKey")?),
                false => None,
            },
            address,
            dns: list("interface.dns"),
            endpoint,
            keepalive: map.get("peer.persistentkeepalive").and_then(|v| v.parse().ok()).unwrap_or(25),
            mtu: map
                .get("interface.mtu")
                .and_then(|v| v.parse().ok())
                .filter(|m| (576..=1500).contains(m))
                .unwrap_or(DEFAULT_MTU),
        })
    }
}

// ---------- smoltcp device: two packet queues ----------

struct Queues {
    rx: VecDeque<Vec<u8>>,
    tx: VecDeque<Vec<u8>>,
    mtu: usize,
}

struct RxTok(Vec<u8>);
struct TxTok<'a>(&'a mut VecDeque<Vec<u8>>);

impl RxToken for RxTok {
    fn consume<R, F: FnOnce(&[u8]) -> R>(self, f: F) -> R {
        f(&self.0)
    }
}

impl TxToken for TxTok<'_> {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, len: usize, f: F) -> R {
        let mut packet = vec![0; len];
        let r = f(&mut packet);
        self.0.push_back(packet);
        r
    }
}

impl Device for Queues {
    type RxToken<'a> = RxTok;
    type TxToken<'a> = TxTok<'a>;

    fn receive(&mut self, _: StackInstant) -> Option<(RxTok, TxTok<'_>)> {
        let packet = self.rx.pop_front()?;
        Some((RxTok(packet), TxTok(&mut self.tx)))
    }

    fn transmit(&mut self, _: StackInstant) -> Option<TxTok<'_>> {
        Some(TxTok(&mut self.tx))
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ip;
        caps.max_transmission_unit = self.mtu;
        caps
    }
}

struct Stack {
    iface: Interface,
    sockets: SocketSet<'static>,
    dev: Queues,
    dns: SocketHandle,
    /// Dropped sockets; removed after the next poll so their RST/FIN still goes out.
    closing: Vec<SocketHandle>,
    next_port: u16,
}

impl Stack {
    fn port(&mut self) -> u16 {
        self.next_port = if self.next_port >= 65000 { 49152 } else { self.next_port + 1 };
        self.next_port
    }
}

#[derive(Clone, Copy)]
pub struct TunnelStats {
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    /// Seconds since the last completed WireGuard handshake.
    pub handshake_age: Option<u64>,
}

pub struct Tunnel {
    stack: Mutex<Stack>,
    wake: Notify,
    cancel: CancellationToken,
    alive: AtomicBool,
    started: Instant,
    rx_bytes: AtomicU64,
    tx_bytes: AtomicU64,
    /// Milliseconds since `started` of the last handshake + 1; 0 = none yet.
    handshake_at: AtomicU64,
}

fn dead() -> io::Error {
    io::Error::new(io::ErrorKind::ConnectionAborted, "VPN tunnel is down")
}

impl Tunnel {
    /// Opens the tunnel and waits for the first handshake.
    pub async fn start(cfg: WgConfig) -> Result<Arc<Self>, String> {
        let endpoint = tokio::net::lookup_host(&cfg.endpoint)
            .await
            .ok()
            .and_then(|mut a| a.find(SocketAddr::is_ipv4))
            .ok_or("VPN config: Peer Endpoint cannot be resolved")?;
        let udp = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await.map_err(|e| format!("VPN socket: {e}"))?;
        udp.connect(endpoint).await.map_err(|e| format!("VPN socket: {e}"))?;

        let tunn = Tunn::new(
            StaticSecret::from(cfg.private),
            PublicKey::from(cfg.peer),
            cfg.preshared,
            Some(cfg.keepalive),
            rand::random::<u32>() >> 8,
            None,
        );

        let mut dev = Queues { rx: VecDeque::new(), tx: VecDeque::new(), mtu: cfg.mtu };
        let mut config = Config::new(HardwareAddress::Ip);
        config.random_seed = rand::random();
        let mut iface = Interface::new(config, &mut dev, StackInstant::from_millis(0));
        iface.update_ip_addrs(|addrs| {
            let _ = addrs.push(IpCidr::new(IpAddress::Ipv4(cfg.address), 32));
        });
        // Medium::Ip has no neighbours; the gateway only has to exist for the route lookup.
        let _ = iface.routes_mut().add_default_ipv4_route(cfg.address);
        let mut sockets = SocketSet::new(Vec::new());
        let servers: Vec<IpAddress> = cfg.dns.iter().take(1).map(|a| IpAddress::Ipv4(*a)).collect();
        let dns = sockets.add(dns::Socket::new(&servers, Vec::new()));

        let tunnel = Arc::new(Self {
            stack: Mutex::new(Stack { iface, sockets, dev, dns, closing: Vec::new(), next_port: 49152 }),
            wake: Notify::new(),
            cancel: CancellationToken::new(),
            alive: AtomicBool::new(true),
            started: Instant::now(),
            rx_bytes: AtomicU64::new(0),
            tx_bytes: AtomicU64::new(0),
            handshake_at: AtomicU64::new(0),
        });
        tokio::spawn(tunnel.clone().drive(tunn, udp));

        let deadline = Instant::now() + HANDSHAKE_WAIT;
        while tunnel.handshake_at.load(Ordering::Relaxed) == 0 {
            if Instant::now() > deadline || !tunnel.is_alive() {
                tunnel.stop();
                return Err("no WireGuard handshake (VPN server unreachable or key rejected)".into());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Ok(tunnel)
    }

    pub fn stop(&self) {
        self.alive.store(false, Ordering::SeqCst);
        self.cancel.cancel();
    }

    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    pub fn stats(&self) -> TunnelStats {
        let at = self.handshake_at.load(Ordering::Relaxed);
        TunnelStats {
            rx_bytes: self.rx_bytes.load(Ordering::Relaxed),
            tx_bytes: self.tx_bytes.load(Ordering::Relaxed),
            handshake_age: (at > 0).then(|| (self.started.elapsed().as_millis() as u64).saturating_sub(at - 1) / 1000),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Stack> {
        self.stack.lock().unwrap_or_else(|e| e.into_inner())
    }

    async fn send(&self, udp: &UdpSocket, wire: &[u8]) {
        if udp.send(wire).await.is_ok() {
            self.tx_bytes.fetch_add(wire.len() as u64, Ordering::Relaxed);
        }
    }

    /// One datagram from the VPN server: handshake traffic is answered, data lands in the stack's rx queue.
    async fn ingest(&self, tunn: &mut Tunn, udp: &UdpSocket, datagram: &[u8], buf: &mut [u8]) {
        self.rx_bytes.fetch_add(datagram.len() as u64, Ordering::Relaxed);
        let mut src = datagram;
        loop {
            match tunn.decapsulate(None, src, buf) {
                // boringtun wants to be called again with an empty datagram until its queue is flushed.
                TunnResult::WriteToNetwork(wire) => {
                    self.send(udp, wire).await;
                    src = &[];
                }
                TunnResult::WriteToTunnelV4(packet, _) => {
                    self.lock().dev.rx.push_back(packet.to_vec());
                    return;
                }
                _ => return,
            }
        }
    }

    async fn drive(self: Arc<Self>, mut tunn: Tunn, udp: UdpSocket) {
        let mut wire = vec![0u8; 65536];
        let mut buf = vec![0u8; 65536];
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        // boringtun only handshakes on its own once there is data to send: start it right away.
        if let TunnResult::WriteToNetwork(out) = tunn.format_handshake_initiation(&mut buf, false) {
            self.send(&udp, out).await;
        }
        loop {
            let (packets, delay) = {
                let mut guard = self.lock();
                let s = &mut *guard;
                let now = StackInstant::from_millis(self.started.elapsed().as_millis() as i64);
                let closing = std::mem::take(&mut s.closing);
                s.iface.poll(now, &mut s.dev, &mut s.sockets);
                for h in closing {
                    s.sockets.remove(h);
                }
                let packets: Vec<Vec<u8>> = s.dev.tx.drain(..).collect();
                let delay = s.iface.poll_delay(now, &s.sockets).map(|d| d.total_millis()).unwrap_or(1000);
                (packets, Duration::from_millis(delay.clamp(1, 1000)))
            };
            for packet in packets {
                if let TunnResult::WriteToNetwork(out) = tunn.encapsulate(&packet, &mut buf) {
                    self.send(&udp, out).await;
                }
            }

            tokio::select! {
                _ = self.cancel.cancelled() => break,
                _ = self.wake.notified() => {}
                _ = tokio::time::sleep(delay) => {}
                _ = tick.tick() => {
                    match tunn.update_timers(&mut buf) {
                        TunnResult::WriteToNetwork(out) => self.send(&udp, out).await,
                        // Handshake retries exhausted: the server is gone.
                        TunnResult::Err(_) => break,
                        _ => {}
                    }
                    if let Some(age) = tunn.stats().0 {
                        let at = (self.started.elapsed().saturating_sub(age)).as_millis() as u64;
                        self.handshake_at.store(at + 1, Ordering::Relaxed);
                    }
                }
                r = udp.recv(&mut wire) => {
                    // Windows reports ICMP "port unreachable" as a recv error; the handshake timers decide.
                    if let Ok(n) = r {
                        self.ingest(&mut tunn, &udp, &wire[..n], &mut buf).await;
                        for _ in 0..128 {
                            let Ok(n) = udp.try_recv(&mut wire) else { break };
                            self.ingest(&mut tunn, &udp, &wire[..n], &mut buf).await;
                        }
                    }
                }
            }
        }
        self.alive.store(false, Ordering::SeqCst);
        // Wake everything still waiting on a socket; they see `alive == false`.
        let mut guard = self.lock();
        for (_, socket) in guard.sockets.iter_mut() {
            if let smoltcp::socket::Socket::Tcp(t) = socket {
                t.abort();
            }
        }
    }

    pub async fn connect(self: &Arc<Self>, addr: SocketAddrV4) -> io::Result<TcpConn> {
        if !self.is_alive() {
            return Err(dead());
        }
        let handle = {
            let mut guard = self.lock();
            let s = &mut *guard;
            let mut socket =
                tcp::Socket::new(tcp::SocketBuffer::new(vec![0; TCP_RX]), tcp::SocketBuffer::new(vec![0; TCP_TX]));
            socket.set_nagle_enabled(false);
            socket.set_keep_alive(Some(smoltcp::time::Duration::from_secs(30)));
            socket.set_timeout(Some(smoltcp::time::Duration::from_secs(90)));
            let port = s.port();
            socket
                .connect(s.iface.context(), IpEndpoint::new(IpAddress::Ipv4(*addr.ip()), addr.port()), port)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid address"))?;
            s.sockets.add(socket)
        };
        // From here the guard in `conn` cleans up on every exit path.
        let conn = TcpConn { tunnel: self.clone(), handle };
        self.wake.notify_one();
        let established = poll_fn(|cx| {
            if !self.is_alive() {
                return Poll::Ready(Err(dead()));
            }
            let mut guard = self.lock();
            let socket = guard.sockets.get_mut::<tcp::Socket>(handle);
            match socket.state() {
                tcp::State::SynSent | tcp::State::SynReceived => {
                    socket.register_recv_waker(cx.waker());
                    socket.register_send_waker(cx.waker());
                    Poll::Pending
                }
                tcp::State::Established | tcp::State::CloseWait => Poll::Ready(Ok(())),
                _ => Poll::Ready(Err(io::Error::new(io::ErrorKind::ConnectionRefused, "connection refused"))),
            }
        });
        match tokio::time::timeout(CONNECT_TIMEOUT, established).await {
            Ok(Ok(())) => Ok(conn),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "connect timed out")),
        }
    }

    pub fn udp(self: &Arc<Self>) -> io::Result<UdpConn> {
        if !self.is_alive() {
            return Err(dead());
        }
        let mut guard = self.lock();
        let buffer = || udp::PacketBuffer::new(vec![udp::PacketMetadata::EMPTY; 16], vec![0; 16 * 1024]);
        let mut socket = udp::Socket::new(buffer(), buffer());
        let port = guard.port();
        socket.bind(port).map_err(|_| io::Error::other("udp bind failed"))?;
        let handle = guard.sockets.add(socket);
        Ok(UdpConn { tunnel: self.clone(), handle })
    }

    /// Resolves through the VPN's DNS server, so not even host names leave outside the tunnel.
    pub async fn resolve(self: &Arc<Self>, host: &str) -> io::Result<Ipv4Addr> {
        if let Ok(ip) = host.parse() {
            return Ok(ip);
        }
        if !self.is_alive() {
            return Err(dead());
        }
        let failed = || io::Error::new(io::ErrorKind::NotFound, "host not found");
        let query = {
            let mut guard = self.lock();
            let s = &mut *guard;
            let dns = s.dns;
            s.sockets
                .get_mut::<dns::Socket>(dns)
                .start_query(s.iface.context(), host, DnsQueryType::A)
                .map_err(|_| failed())?
        };
        self.wake.notify_one();
        let answer = poll_fn(|cx| {
            let mut guard = self.lock();
            let dns = guard.dns;
            let socket = guard.sockets.get_mut::<dns::Socket>(dns);
            match socket.get_query_result(query) {
                Ok(addrs) => Poll::Ready(addrs.first().map(|IpAddress::Ipv4(ip)| *ip)),
                Err(dns::GetQueryResultError::Pending) => {
                    socket.register_query_waker(query, cx.waker());
                    Poll::Pending
                }
                Err(dns::GetQueryResultError::Failed) => Poll::Ready(None),
            }
        });
        tokio::select! {
            r = answer => r.ok_or_else(failed),
            _ = tokio::time::sleep(DNS_TIMEOUT) => {
                let mut guard = self.lock();
                let dns = guard.dns;
                guard.sockets.get_mut::<dns::Socket>(dns).cancel_query(query);
                Err(io::Error::new(io::ErrorKind::TimedOut, "DNS timed out"))
            }
            _ = self.cancel.cancelled() => Err(dead()),
        }
    }
}

/// A TCP connection inside the tunnel.
pub struct TcpConn {
    tunnel: Arc<Tunnel>,
    handle: SocketHandle,
}

impl AsyncRead for TcpConn {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        if !self.tunnel.is_alive() {
            return Poll::Ready(Err(dead()));
        }
        let mut guard = self.tunnel.lock();
        let socket = guard.sockets.get_mut::<tcp::Socket>(self.handle);
        if socket.can_recv() {
            let n = socket.recv_slice(buf.initialize_unfilled()).map_err(|_| dead())?;
            buf.advance(n);
            drop(guard);
            // The receive window opened: let the stack tell the peer.
            self.tunnel.wake.notify_one();
            Poll::Ready(Ok(()))
        } else if !socket.may_recv() {
            Poll::Ready(Ok(()))
        } else {
            socket.register_recv_waker(cx.waker());
            Poll::Pending
        }
    }
}

impl AsyncWrite for TcpConn {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, data: &[u8]) -> Poll<io::Result<usize>> {
        if !self.tunnel.is_alive() {
            return Poll::Ready(Err(dead()));
        }
        let mut guard = self.tunnel.lock();
        let socket = guard.sockets.get_mut::<tcp::Socket>(self.handle);
        if !socket.may_send() {
            Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()))
        } else if socket.can_send() {
            let n = socket.send_slice(data).map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?;
            drop(guard);
            self.tunnel.wake.notify_one();
            Poll::Ready(Ok(n))
        } else {
            socket.register_send_waker(cx.waker());
            Poll::Pending
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.tunnel.lock().sockets.get_mut::<tcp::Socket>(self.handle).close();
        self.tunnel.wake.notify_one();
        Poll::Ready(Ok(()))
    }
}

impl Drop for TcpConn {
    fn drop(&mut self) {
        let mut guard = self.tunnel.lock();
        guard.sockets.get_mut::<tcp::Socket>(self.handle).abort();
        guard.closing.push(self.handle);
        drop(guard);
        self.tunnel.wake.notify_one();
    }
}

/// A UDP socket inside the tunnel (tracker announces).
pub struct UdpConn {
    tunnel: Arc<Tunnel>,
    handle: SocketHandle,
}

impl UdpConn {
    pub fn send_to(&self, data: &[u8], to: SocketAddrV4) -> io::Result<()> {
        if !self.tunnel.is_alive() {
            return Err(dead());
        }
        self.tunnel
            .lock()
            .sockets
            .get_mut::<udp::Socket>(self.handle)
            .send_slice(data, IpEndpoint::new(IpAddress::Ipv4(*to.ip()), to.port()))
            .map_err(|_| io::Error::other("udp send failed"))?;
        self.tunnel.wake.notify_one();
        Ok(())
    }

    /// Callers bound this with a timeout: a dead tunnel never wakes it.
    pub async fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        poll_fn(|cx| {
            if !self.tunnel.is_alive() {
                return Poll::Ready(Err(dead()));
            }
            let mut guard = self.tunnel.lock();
            let socket = guard.sockets.get_mut::<udp::Socket>(self.handle);
            match socket.recv_slice(buf) {
                Ok((n, _)) => Poll::Ready(Ok(n)),
                Err(_) => {
                    socket.register_recv_waker(cx.waker());
                    Poll::Pending
                }
            }
        })
        .await
    }
}

impl Drop for UdpConn {
    fn drop(&mut self) {
        let mut guard = self.tunnel.lock();
        guard.sockets.get_mut::<udp::Socket>(self.handle).close();
        guard.closing.push(self.handle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_wireguard_config() {
        let k = base64::engine::general_purpose::STANDARD.encode([7u8; 32]);
        let text = format!(
            "[Interface]\n# comment\nPrivateKey = {k}\nAddress = 10.2.0.2/32\nDNS = 10.2.0.1\n\n[Peer]\nPublicKey = {k}\nAllowedIPs = 0.0.0.0/0, ::/0\nEndpoint = 192.0.2.7:51820\n"
        );
        let cfg = WgConfig::parse(&text).unwrap();
        assert_eq!(cfg.address, Ipv4Addr::new(10, 2, 0, 2));
        assert_eq!(cfg.dns, vec![Ipv4Addr::new(10, 2, 0, 1)]);
        assert_eq!(cfg.endpoint, "192.0.2.7:51820");
        assert_eq!((cfg.keepalive, cfg.mtu, cfg.preshared.is_none()), (25, DEFAULT_MTU, true));
    }

    /// A WireGuard peer on localhost (boringtun + smoltcp with a TCP echo on 10.9.0.1:7).
    async fn echo_peer(udp: UdpSocket, mut tunn: Tunn) {
        let ip = Ipv4Addr::new(10, 9, 0, 1);
        let mut dev = Queues { rx: VecDeque::new(), tx: VecDeque::new(), mtu: DEFAULT_MTU };
        let mut iface = Interface::new(Config::new(HardwareAddress::Ip), &mut dev, StackInstant::from_millis(0));
        iface.update_ip_addrs(|a| a.push(IpCidr::new(IpAddress::Ipv4(ip), 32)).unwrap());
        iface.routes_mut().add_default_ipv4_route(ip).unwrap();
        let mut sockets = SocketSet::new(Vec::new());
        let mut listener = tcp::Socket::new(tcp::SocketBuffer::new(vec![0; 65536]), tcp::SocketBuffer::new(vec![0; 65536]));
        listener.listen(7).unwrap();
        let handle = sockets.add(listener);
        let started = Instant::now();
        let (mut wire, mut buf) = (vec![0u8; 65536], vec![0u8; 65536]);
        let mut client = None;
        loop {
            if let Ok(Ok((n, from))) = tokio::time::timeout(Duration::from_millis(5), udp.recv_from(&mut wire)).await {
                client = Some(from);
                let mut src = &wire[..n];
                loop {
                    match tunn.decapsulate(None, src, &mut buf) {
                        TunnResult::WriteToNetwork(out) => {
                            udp.send_to(out, from).await.unwrap();
                            src = &[];
                        }
                        TunnResult::WriteToTunnelV4(packet, _) => {
                            dev.rx.push_back(packet.to_vec());
                            break;
                        }
                        _ => break,
                    }
                }
            }
            let now = StackInstant::from_millis(started.elapsed().as_millis() as i64);
            iface.poll(now, &mut dev, &mut sockets);
            let socket = sockets.get_mut::<tcp::Socket>(handle);
            let mut data = [0u8; 4096];
            while socket.can_recv() && socket.can_send() {
                let n = socket.recv_slice(&mut data).unwrap();
                socket.send_slice(&data[..n]).unwrap();
            }
            iface.poll(now, &mut dev, &mut sockets);
            for packet in dev.tx.drain(..) {
                if let (TunnResult::WriteToNetwork(out), Some(to)) = (tunn.encapsulate(&packet, &mut buf), client) {
                    udp.send_to(out, to).await.unwrap();
                }
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tunnel_carries_tcp_end_to_end() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (client_key, server_key) = ([3u8; 32], [5u8; 32]);
        let public = |k: [u8; 32]| PublicKey::from(&StaticSecret::from(k)).to_bytes();
        let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let endpoint = udp.local_addr().unwrap().to_string();
        let server = Tunn::new(StaticSecret::from(server_key), PublicKey::from(public(client_key)), None, None, 1, None);
        tokio::spawn(echo_peer(udp, server));

        let cfg = WgConfig {
            private: client_key,
            peer: public(server_key),
            preshared: None,
            address: Ipv4Addr::new(10, 9, 0, 2),
            dns: vec![],
            endpoint,
            keepalive: 25,
            mtu: DEFAULT_MTU,
        };
        let tunnel = Tunnel::start(cfg).await.expect("handshake");
        let mut conn = tunnel.connect("10.9.0.1:7".parse().unwrap()).await.expect("connect");
        // More than one window, so segmentation, ACKs and window updates are exercised.
        let sent: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let mut got = vec![0u8; sent.len()];
        let (mut r, mut w) = tokio::io::split(&mut conn);
        let (wrote, read) = tokio::join!(w.write_all(&sent), tokio::time::timeout(Duration::from_secs(20), r.read_exact(&mut got)));
        wrote.unwrap();
        read.expect("echo in time").unwrap();
        assert!(got == sent);
        assert!(tunnel.stats().handshake_age.is_some());

        // Kill-switch: a stopped tunnel fails every socket instead of falling back to anything.
        tunnel.stop();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let mut one = [0u8; 1];
        assert!(conn.read(&mut one).await.is_err());
        assert!(tunnel.connect("10.9.0.1:7".parse().unwrap()).await.is_err());
    }

    #[test]
    fn config_errors_never_quote_values() {
        let err = WgConfig::parse("[Interface]\nPrivateKey = geheim\nAddress = 10.2.0.2/32\n[Peer]\nEndpoint = a:1\n")
            .err()
            .unwrap();
        assert!(!err.contains("geheim"));
    }
}
