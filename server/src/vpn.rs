//! The torrent VPN: owns the WireGuard tunnel ([`crate::wg`]) and a loopback SOCKS5 server that is the
//! only network path the torrent engine gets. Tunnel down = SOCKS refuses = no torrent traffic at all
//! (kill-switch by construction, there is no fallback route).

use std::{
    net::{Ipv4Addr, SocketAddrV4},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use axum::{Json, extract::State, http::StatusCode};
use serde::Serialize;
use serde_json::Value;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

use crate::{
    api::St,
    db::now_ms,
    scan::iso,
    wg::{Tunnel, WgConfig},
};

/// Host name of the in-process tracker shim; the SOCKS server routes it to Flimmer itself.
pub const SHIM_HOST: &str = "tracker.flimmer.invalid";
const SOCKS_USER: &str = "flimmer";

#[derive(Clone, Serialize)]
struct PublicIp {
    ip: String,
    country: Option<String>,
}

#[derive(Default)]
struct Shared {
    tunnel: Option<Arc<Tunnel>>,
    since: Option<i64>,
    error: Option<String>,
    public: Option<PublicIp>,
}

pub struct Vpn {
    conf: Mutex<PathBuf>,
    shared: Mutex<Shared>,
    gate: tokio::sync::Mutex<()>,
    blocked: AtomicBool,
    connecting: AtomicBool,
    socks_port: u16,
    socks_pass: String,
    /// Port of Flimmer's own HTTP server (target of the tracker shim).
    local_port: u16,
    /// HTTP client whose every request (DNS included) goes through the tunnel.
    pub http: reqwest::Client,
}

#[derive(Serialize)]
pub struct VpnStatus {
    /// "down" | "connecting" | "up" | "blocked"
    state: &'static str,
    since: Option<String>,
    public_ip: Option<String>,
    country: Option<String>,
    handshake_age: Option<u64>,
    rx_bytes: u64,
    tx_bytes: u64,
    error: Option<String>,
    config_found: bool,
}

impl Vpn {
    pub async fn new(conf: PathBuf, local_port: u16) -> Result<Arc<Self>, String> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.map_err(|e| format!("socks bind: {e}"))?;
        let socks_port = listener.local_addr().map_err(|e| format!("socks bind: {e}"))?.port();
        let socks_pass = format!("{:032x}", rand::random::<u128>());
        let proxy = reqwest::Proxy::all(format!("socks5h://{SOCKS_USER}:{socks_pass}@127.0.0.1:{socks_port}"))
            .map_err(|e| format!("socks proxy: {e}"))?;
        let http = reqwest::Client::builder()
            .proxy(proxy)
            .timeout(Duration::from_secs(12))
            .build()
            .map_err(|e| format!("http client: {e}"))?;
        let vpn = Arc::new(Self {
            conf: Mutex::new(conf),
            shared: Mutex::default(),
            gate: tokio::sync::Mutex::new(()),
            blocked: AtomicBool::new(false),
            connecting: AtomicBool::new(false),
            socks_port,
            socks_pass,
            local_port,
            http,
        });
        let accept = vpn.clone();
        tokio::spawn(async move {
            while let Ok((client, _)) = listener.accept().await {
                let vpn = accept.clone();
                tokio::spawn(async move {
                    let _ = vpn.socks(client).await;
                });
            }
        });
        Ok(vpn)
    }

    /// Proxy URL for the torrent engine (`socks5h`: host names are resolved inside the tunnel).
    pub fn proxy_url(&self) -> String {
        format!("socks5h://{SOCKS_USER}:{}@127.0.0.1:{}", self.socks_pass, self.socks_port)
    }

    pub fn set_conf(&self, conf: PathBuf) {
        *self.conf.lock().unwrap() = conf;
    }

    pub fn is_blocked(&self) -> bool {
        self.blocked.load(Ordering::SeqCst)
    }

    /// The tunnel, if it is up right now.
    pub fn current(&self) -> Option<Arc<Tunnel>> {
        self.shared.lock().unwrap().tunnel.as_ref().filter(|t| t.is_alive()).cloned()
    }

    /// Opens the tunnel unless it is up already. Fails while the kill-switch is engaged.
    pub async fn ensure_up(self: &Arc<Self>) -> Result<Arc<Tunnel>, String> {
        let _gate = self.gate.lock().await;
        if self.is_blocked() {
            return Err("VPN kill-switch is engaged".into());
        }
        if let Some(t) = self.current() {
            return Ok(t);
        }
        self.connecting.store(true, Ordering::SeqCst);
        let conf = self.conf.lock().unwrap().clone();
        let result = async {
            // Only the error kind: the message could quote the path, never the content.
            let text = tokio::fs::read_to_string(&conf)
                .await
                .map_err(|e| format!("VPN config cannot be read ({})", e.kind()))?;
            Tunnel::start(WgConfig::parse(&text)?).await
        }
        .await;
        self.connecting.store(false, Ordering::SeqCst);
        let mut shared = self.shared.lock().unwrap();
        match result {
            Ok(tunnel) => {
                *shared = Shared { tunnel: Some(tunnel.clone()), since: Some(now_ms()), ..Shared::default() };
                drop(shared);
                tokio::spawn(self.clone().lookup_public_ip(tunnel.clone()));
                Ok(tunnel)
            }
            Err(e) => {
                shared.error = Some(e.clone());
                Err(e)
            }
        }
    }

    pub fn down(&self) {
        let mut shared = self.shared.lock().unwrap();
        if let Some(t) = shared.tunnel.take() {
            t.stop();
        }
        shared.since = None;
        shared.public = None;
    }

    /// Kill-switch: tunnel down and held down until [`Vpn::release`].
    pub fn kill(&self) {
        self.blocked.store(true, Ordering::SeqCst);
        self.down();
    }

    pub fn release(&self) {
        self.blocked.store(false, Ordering::SeqCst);
    }

    /// Asks an IP echo service through the tunnel which address the internet sees.
    async fn lookup_public_ip(self: Arc<Self>, tunnel: Arc<Tunnel>) {
        for (url, country_key) in [("https://ipinfo.io/json", "country"), ("https://ifconfig.co/json", "country_iso")] {
            let Ok(resp) = self.http.get(url).header("accept", "application/json").send().await else { continue };
            let Ok(v) = resp.json::<Value>().await else { continue };
            let Some(ip) = v.get("ip").and_then(Value::as_str) else { continue };
            let public = PublicIp { ip: ip.to_owned(), country: v.get(country_key).and_then(Value::as_str).map(str::to_owned) };
            let mut shared = self.shared.lock().unwrap();
            if shared.tunnel.as_ref().is_some_and(|t| Arc::ptr_eq(t, &tunnel)) {
                shared.public = Some(public);
            }
            return;
        }
    }

    pub fn status(&self) -> VpnStatus {
        let shared = self.shared.lock().unwrap();
        let tunnel = shared.tunnel.as_ref().filter(|t| t.is_alive());
        let stats = tunnel.map(|t| t.stats());
        VpnStatus {
            state: if self.is_blocked() {
                "blocked"
            } else if tunnel.is_some() {
                "up"
            } else if self.connecting.load(Ordering::SeqCst) {
                "connecting"
            } else {
                "down"
            },
            since: tunnel.and(shared.since).map(iso),
            public_ip: tunnel.and(shared.public.as_ref()).map(|p| p.ip.clone()),
            country: tunnel.and(shared.public.as_ref()).and_then(|p| p.country.clone()),
            handshake_age: stats.and_then(|s| s.handshake_age),
            rx_bytes: stats.map_or(0, |s| s.rx_bytes),
            tx_bytes: stats.map_or(0, |s| s.tx_bytes),
            error: shared.error.clone(),
            config_found: self.conf.lock().unwrap().is_file(),
        }
    }

    /// One SOCKS5 client (RFC 1928 CONNECT with RFC 1929 password auth).
    async fn socks(self: Arc<Self>, mut client: TcpStream) -> std::io::Result<()> {
        let _ = client.set_nodelay(true);
        let mut head = [0u8; 2];
        client.read_exact(&mut head).await?;
        let mut methods = vec![0u8; head[1] as usize];
        client.read_exact(&mut methods).await?;
        if head[0] != 5 || !methods.contains(&2) {
            return client.write_all(&[5, 0xff]).await;
        }
        client.write_all(&[5, 2]).await?;

        client.read_exact(&mut head).await?;
        let mut user = vec![0u8; head[1] as usize];
        client.read_exact(&mut user).await?;
        let mut pass = vec![0u8; client.read_u8().await? as usize];
        client.read_exact(&mut pass).await?;
        if pass != self.socks_pass.as_bytes() {
            return client.write_all(&[1, 1]).await;
        }
        client.write_all(&[1, 0]).await?;

        let mut req = [0u8; 4];
        client.read_exact(&mut req).await?;
        let reply = |code: u8| [5, code, 0, 1, 0, 0, 0, 0, 0, 0];
        if req[1] != 1 {
            return client.write_all(&reply(7)).await;
        }
        let host = match req[3] {
            1 => {
                let mut ip = [0u8; 4];
                client.read_exact(&mut ip).await?;
                Ipv4Addr::from(ip).to_string()
            }
            3 => {
                let mut name = vec![0u8; client.read_u8().await? as usize];
                client.read_exact(&mut name).await?;
                String::from_utf8_lossy(&name).into_owned()
            }
            // The tunnel is IPv4 only.
            _ => return client.write_all(&reply(8)).await,
        };
        let port = client.read_u16().await?;

        if host == SHIM_HOST {
            let mut local = TcpStream::connect((Ipv4Addr::LOCALHOST, self.local_port)).await?;
            client.write_all(&reply(0)).await?;
            tokio::io::copy_bidirectional(&mut client, &mut local).await?;
            return Ok(());
        }
        let Some(tunnel) = self.current() else {
            return client.write_all(&reply(1)).await;
        };
        let conn = match tunnel.resolve(&host).await {
            Ok(ip) => tunnel.connect(SocketAddrV4::new(ip, port)).await,
            Err(e) => Err(e),
        };
        let Ok(mut conn) = conn else {
            return client.write_all(&reply(5)).await;
        };
        client.write_all(&reply(0)).await?;
        tokio::io::copy_bidirectional(&mut client, &mut conn).await?;
        Ok(())
    }
}

// ---------- API ----------

pub async fn status(State(st): St) -> Json<VpnStatus> {
    Json(st.torrents.vpn.status())
}

pub async fn kill(State(st): St) -> StatusCode {
    st.torrents.vpn.kill();
    StatusCode::NO_CONTENT
}

pub async fn release(State(st): St) -> StatusCode {
    st.torrents.vpn.release();
    StatusCode::NO_CONTENT
}
