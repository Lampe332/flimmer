//! Attack probes against a real Flimmer server (real sockets, real database, real router): every line of the
//! table printed at the end is one attack from `docs/security.md` and what the server answered.
//!
//! Run: `cargo test --release --test security -- --nocapture`
//!
//! One test function on purpose: the server's folders are set once per process.

use std::{
    net::{Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

struct Table {
    rows: Vec<(String, String, String, bool)>,
}

impl Table {
    fn add(&mut self, probe: &str, want: &str, got: impl ToString, pass: bool) {
        self.rows.push((probe.to_owned(), want.to_owned(), got.to_string(), pass));
    }

    /// One probe that must answer with one of `want`.
    fn status(&mut self, probe: &str, want: &[u16], got: u16) {
        let list = want.iter().map(u16::to_string).collect::<Vec<_>>().join("/");
        self.add(probe, &list, got, want.contains(&got));
    }

    fn print_and_check(&self) {
        let width = self.rows.iter().map(|r| r.0.chars().count()).max().unwrap_or(10);
        println!("\n{:<width$}  {:<9}  {:<9}  result", "probe", "expected", "got");
        for (probe, want, got, pass) in &self.rows {
            println!("{probe:<width$}  {want:<9}  {got:<9}  {}", if *pass { "ok" } else { "FAILED" });
        }
        let failed: Vec<&str> = self.rows.iter().filter(|r| !r.3).map(|r| r.0.as_str()).collect();
        println!("\n{} probes, {} failed\n", self.rows.len(), failed.len());
        assert!(failed.is_empty(), "failed probes: {failed:?}");
    }
}

/// One HTTP/1.1 request written by hand, so paths and headers go out exactly as given
/// (an HTTP library would tidy up `..` and refuse a foreign `Host`).
async fn raw(addr: SocketAddr, method: &str, path: &str, host: &str, headers: &[(&str, &str)], body: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    if !body.is_empty() {
        req.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n", body.len()));
    }
    req.push_str("\r\n");
    req.push_str(body);
    stream.write_all(req.as_bytes()).await.expect("write");
    let mut answer = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(20), stream.read_to_end(&mut answer)).await;
    let text = String::from_utf8_lossy(&answer).into_owned();
    let status = text.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    (status, text)
}

fn header_of(answer: &str, name: &str) -> Option<String> {
    let head = answer.split("\r\n\r\n").next().unwrap_or_default();
    head.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.trim().to_owned()))
}

fn body_of(answer: &str) -> &str {
    answer.split_once("\r\n\r\n").map_or("", |(_, b)| b)
}

fn json_of(answer: &str) -> Value {
    let body = body_of(answer);
    // Bodies are small and may arrive chunked: take the JSON between the first and last brace.
    let (a, b) = (body.find(['{', '[']), body.rfind(['}', ']']));
    match (a, b) {
        (Some(a), Some(b)) if b >= a => serde_json::from_str(&body[a..=b]).unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

fn cookie_of(answer: &str) -> Option<String> {
    header_of(answer, "set-cookie").and_then(|c| c.split(';').next().map(str::to_owned))
}

fn free_port() -> u16 {
    std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port()
}

/// A local service that must never be reached: counts connections.
async fn canary() -> (u16, Arc<AtomicUsize>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            counter.fetch_add(1, Ordering::SeqCst);
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nSECRET").await;
        }
    });
    (port, hits)
}

fn insert_stream(db: &std::path::Path, id: &str, url: &str, kind: &str) {
    let conn = rusqlite::Connection::open(db).unwrap();
    conn.busy_timeout(Duration::from_secs(5)).unwrap();
    let row = json!({ "id": id, "url": url, "headers": [["Referer", "https://provider.example/"]], "kind": kind }).to_string();
    conn.execute("INSERT OR REPLACE INTO streams (key, data, updated_at) VALUES (?1, ?2, ?3)", rusqlite::params![id, row, i64::MAX / 2]).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn attack_probes() {
    // ---------- a fresh install in a temp folder ----------
    let root: PathBuf = std::env::temp_dir().join(format!("flimmer-sectest-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dist = root.join("web").join("dist");
    std::fs::create_dir_all(dist.join("assets")).unwrap();
    std::fs::write(root.join("providers.json"), "[]").unwrap();
    std::fs::write(dist.join("index.html"), "<!doctype html><title>Flimmer</title><div id=root></div>").unwrap();
    std::fs::write(dist.join("assets").join("app.js"), "console.log('flimmer')").unwrap();
    // The data folder holds things a traversal would go for.
    let port = free_port();
    let server = flimmer::bind(flimmer::Options { root: root.clone(), home: root.clone(), port, sniffer: None, dev: false })
        .await
        .expect("server starts");
    let entry = server.entry_url(Duration::from_secs(60));
    tokio::spawn(server.serve(std::future::pending()));
    let local = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let host = format!("127.0.0.1:{port}");
    let origin = format!("http://{host}");
    let db = root.join("data").join("flimmer.db");
    let mut t = Table { rows: Vec::new() };

    // ---------- no session ----------
    let (s, a) = raw(local, "GET", "/api/home", &host, &[], "").await;
    t.add("API without session (loopback)", "401", s, s == 401 && json_of(&a)["code"] == "pairing_required");
    let (s, a) = raw(local, "GET", "/", &host, &[], "").await;
    t.add("app shell without session", "200", s, s == 200 && a.contains("id=root"));
    let csp = header_of(&a, "content-security-policy").unwrap_or_default();
    let hardened = header_of(&a, "x-content-type-options").as_deref() == Some("nosniff")
        && header_of(&a, "x-frame-options").as_deref() == Some("DENY")
        && header_of(&a, "cross-origin-resource-policy").as_deref() == Some("same-origin")
        && csp.contains("frame-ancestors 'none'")
        && csp.contains("script-src 'self'");
    t.add("security headers on the page", "set", if hardened { "set" } else { "missing" }, hardened);
    let (_, a) = raw(local, "GET", "/api/session", &host, &[], "").await;
    t.add("session of a stranger", "none", json_of(&a)["role"].as_str().unwrap_or("?"), json_of(&a)["role"] == "none");

    // ---------- the desktop window's ticket ----------
    let ticket_path = entry.strip_prefix(&origin).unwrap().to_owned();
    let (s, a) = raw(local, "GET", &ticket_path, &host, &[], "").await;
    let pc = cookie_of(&a).unwrap_or_default();
    let flags = header_of(&a, "set-cookie").unwrap_or_default();
    t.add("start ticket sets the session", "303", s, s == 303 && flags.contains("HttpOnly") && flags.contains("SameSite=Strict"));
    let (s, _) = raw(local, "GET", &ticket_path, &host, &[], "").await;
    t.status("start ticket a second time", &[403], s);
    let (s, _) = raw(local, "GET", "/api/auth/enter?t=0000", &host, &[], "").await;
    t.status("guessed start ticket", &[403], s);
    let pc_cookie = [("Cookie", pc.as_str())];
    let (s, a) = raw(local, "GET", "/api/session", &host, &pc_cookie, "").await;
    t.add("session of the desktop window", "pc", json_of(&a)["role"].as_str().unwrap_or("?"), s == 200 && json_of(&a)["role"] == "pc");
    let (s, a) = raw(local, "GET", "/api/setup", &host, &pc_cookie, "").await;
    t.add("API with session", "200", s, s == 200 && json_of(&a)["tmdb"] == "missing");
    t.add("token never in /api/setup", "absent", "absent", !body_of(&a).contains("eyJ") && json_of(&a).get("token").is_none());

    // ---------- CSRF: a foreign page posts to the local server ----------
    for (name, headers) in [
        ("cross-origin POST (Origin of a fake page)", vec![("Origin", "http://evil.example")]),
        ("cross-origin POST (Origin null)", vec![("Origin", "null")]),
        ("cross-site POST (Sec-Fetch-Site)", vec![("Sec-Fetch-Site", "cross-site")]),
        ("same-site POST from another local port", vec![("Sec-Fetch-Site", "same-site"), ("Origin", "http://127.0.0.1:8080")]),
    ] {
        // Worst case: the browser even sent the session cookie along.
        let mut with_cookie = headers.clone();
        with_cookie.push(("Cookie", pc.as_str()));
        let (s, _) = raw(local, "POST", "/api/setup/done", &host, &with_cookie, "").await;
        t.status(name, &[403], s);
        let (s, _) = raw(local, "POST", "/api/setup/reset", &host, &with_cookie, r#"{"confirm":"reset"}"#).await;
        t.status(&format!("{name}: reset"), &[403], s);
    }
    let (s, _) = raw(local, "GET", "/api/setup", &host, &[("Sec-Fetch-Site", "cross-site"), ("Cookie", pc.as_str())], "").await;
    t.status("cross-site GET (image or script tag)", &[403], s);
    let (s, _) = raw(local, "POST", "/api/setup/done", &host, &[("Origin", "http://evil.example")], "").await;
    t.status("cross-origin POST without cookie", &[403], s);
    let (s, a) = raw(local, "OPTIONS", "/api/setup/done", &host, &[("Origin", "http://evil.example"), ("Access-Control-Request-Method", "POST")], "").await;
    t.add("CORS preflight of a foreign page", "no allow", s, header_of(&a, "access-control-allow-origin").is_none() && s != 200 && s != 204);
    let own = [("Origin", origin.as_str()), ("Sec-Fetch-Site", "same-origin"), ("Cookie", pc.as_str())];
    let (s, _) = raw(local, "POST", "/api/setup/done", &host, &own, "").await;
    t.status("the app's own POST", &[204], s);

    // ---------- DNS rebinding: the attacker's name points at 127.0.0.1 ----------
    for (name, bad_host) in [
        ("rebinding Host (page)", format!("evil.example:{port}")),
        ("rebinding Host without port", "127.0.0.1".to_owned()),
        ("rebinding Host suffix trick", format!("127.0.0.1:{port}.evil.example")),
    ] {
        let (s, _) = raw(local, "GET", "/", &bad_host, &[], "").await;
        t.status(name, &[403], s);
    }
    let (s, _) = raw(local, "GET", "/api/setup", &format!("evil.example:{port}"), &pc_cookie, "").await;
    t.status("rebinding Host (API, with cookie)", &[403], s);
    let (s, _) = raw(local, "GET", &format!("http://evil.example:{port}/api/setup"), &host, &pc_cookie, "").await;
    t.status("rebinding via absolute request URL", &[403, 400], s);

    // The torrent engine reaches the tracker shim under its own name, and only the shim.
    let (s, a) = raw(local, "GET", "/api/torrents/announce?info_hash=%00", "tracker.flimmer.invalid", &[], "").await;
    t.add("engine asks the tracker shim", "200", s, s == 200 && a.contains("failure reason"));
    let (s, _) = raw(local, "GET", "/api/torrents", "tracker.flimmer.invalid", &pc_cookie, "").await;
    t.status("shim name on another endpoint", &[403], s);
    let (s, _) = raw(local, "GET", "/api/torrents/announce?info_hash=%00", &host, &[("Origin", "http://evil.example")], "").await;
    t.status("tracker shim from a web page", &[403], s);
    let (s, _) = raw(local, "GET", "/api/torrents/announce?info_hash=%00", &host, &[("Sec-Fetch-Site", "same-origin")], "").await;
    t.status("tracker shim from Flimmer's own page", &[401], s);

    // ---------- SSRF through the stream proxy ----------
    let (canary_port, hits) = canary().await;
    let enc = |u: &str| url::form_urlencoded::byte_serialize(u.as_bytes()).collect::<String>();
    insert_stream(&db, "aa01", "https://cdn.example.com/master.m3u8", "hls");
    for (name, target) in [
        ("proxy r?u=http://127.0.0.1", format!("http://127.0.0.1:{canary_port}/secret")),
        ("proxy r?u=http://169.254.169.254", "http://169.254.169.254/latest/meta-data/".to_owned()),
        ("proxy r?u=file:///", "file:///C:/Windows/win.ini".to_owned()),
        ("proxy r?u=http://[::1]", format!("http://[::1]:{canary_port}/")),
        ("proxy r?u=router address", "http://192.168.1.1/".to_owned()),
        ("proxy r?u=foreign public host", "https://example.org/".to_owned()),
    ] {
        let (s, _) = raw(local, "GET", &format!("/api/stream/aa01/r?u={}", enc(&target)), &host, &pc_cookie, "").await;
        t.status(&format!("{name} (unsigned)"), &[403], s);
        let (s, _) = raw(local, "GET", &format!("/api/stream/aa01/r?u={}&s={}", enc(&target), "0".repeat(32)), &host, &pc_cookie, "").await;
        t.status(&format!("{name} (forged signature)"), &[403], s);
    }
    // Even an address the server signed itself is not fetched when it is not public: a captured "stream" that
    // points at this PC, the cloud metadata address, a file, or a name that resolves to loopback.
    let captured = [
        ("captured stream = http://127.0.0.1", format!("http://127.0.0.1:{canary_port}/secret.mp4"), &[403u16][..]),
        ("captured stream = decimal loopback", format!("http://2130706433:{canary_port}/secret.mp4"), &[403][..]),
        ("captured stream = IPv4-mapped loopback", format!("http://[::ffff:127.0.0.1]:{canary_port}/secret.mp4"), &[403][..]),
        ("captured stream = 169.254.169.254", "http://169.254.169.254/latest/meta-data/x.mp4".to_owned(), &[403][..]),
        ("captured stream = file:///", "file:///C:/Windows/win.ini".to_owned(), &[403, 404][..]),
        ("captured stream = name resolving to 127.0.0.1", format!("http://localtest.me:{canary_port}/secret.mp4"), &[502, 403][..]),
    ];
    for (i, (name, url, want)) in captured.iter().enumerate() {
        let id = format!("bb{i:02}");
        insert_stream(&db, &id, url, "mp4");
        let (s, a) = raw(local, "GET", &format!("/api/stream/{id}/master.m3u8"), &host, &pc_cookie, "").await;
        // A plain file answers with a redirect to the signed proxy address: follow it.
        let s = match header_of(&a, "location") {
            Some(next) if s == 307 => raw(local, "GET", &next, &host, &pc_cookie, "").await.0,
            _ => s,
        };
        t.status(name, want, s);
        insert_stream(&db, &format!("cc{i:02}"), url, "hls");
        let (s, a) = raw(local, "GET", &format!("/api/stream/cc{i:02}/master.m3u8"), &host, &pc_cookie, "").await;
        t.add(&format!("{name} as playlist"), "blocked", s, matches!(s, 403 | 404 | 502) && !a.contains("SECRET") && !a.contains("[fonts]"));
    }
    t.add("local service reached through the proxy", "0 times", format!("{} times", hits.load(Ordering::SeqCst)), hits.load(Ordering::SeqCst) == 0);

    // ---------- path traversal on everything that serves files ----------
    let win_ini = |a: &str| a.contains("[fonts]") || a.contains("for 16-bit app support") || a.contains("SQLite format");
    let traversal = [
        "/../../Windows/win.ini",
        "/..%2f..%2f..%2fWindows%2fwin.ini",
        "/..%5c..%5c..%5cWindows%5cwin.ini",
        "/%2e%2e/%2e%2e/%2e%2e/Windows/win.ini",
        "/assets/..%5c..%5c..%5c..%5cWindows%5cwin.ini",
        "/assets/../../data/flimmer.db",
        "/assets/%2e%2e%2f%2e%2e%2fdata%2fflimmer.db",
        "/%252e%252e/%252e%252e/Windows/win.ini",
        "/C:/Windows/win.ini",
        "/brand/..\\..\\..\\Windows\\win.ini",
        "/api/torrents/0000000000000000000000000000000000000000/file/..%5c..%5cWindows%5cwin.ini",
        "/api/torrents/..%5c..%5cWindows%5cwin.ini/file/0",
        "/api/torrents/..%2f..%2f..%2fWindows%2fwin.ini/play",
        "/api/torrents/0000000000000000000000000000000000000000/play?file=..%5c..%5cWindows%5cwin.ini",
        "/api/torrents/0000000000000000000000000000000000000000/subtitle?file=0&track=..%5c..%5cWindows%5cwin.ini",
        "/api/torrents/0000000000000000000000000000000000000000/file/0",
        "/api/stream/..%5c..%5cflimmer.db/master.m3u8",
        "/api/stream/..%2f..%2fflimmer.db/buffer",
        "/api/stream/aa01/..%2f..%2f..%2fdata%2fflimmer.db",
    ];
    for path in traversal {
        let (s, a) = raw(local, "GET", path, &host, &pc_cookie, "").await;
        t.add(&format!("traversal GET {path}"), "400/403/404", s, matches!(s, 400 | 403 | 404) && !win_ini(&a));
    }
    // Deleting through a stream id must not leave the cache folder.
    std::fs::write(root.join("data").join("keep.txt"), "x").unwrap();
    let (s, _) = raw(local, "DELETE", "/api/stream/..%5c..%5cdata", &host, &own, "").await;
    t.add("traversal DELETE /api/stream/..\\..\\data", "400/404", s, matches!(s, 400 | 404) && root.join("data").join("keep.txt").is_file());
    // Download folders: no network share (Windows would log in there), no relative path.
    let settings = |movies: &str| json!({ "vpn_config": root.join("data").join("vpn").join("proton.conf"), "movies_dir": movies, "series_dir": "D:\\Serien", "upload_kbps": 30 }).to_string();
    for (name, dir) in [("UNC share", "\\\\evil.example\\share"), ("device path", "\\\\?\\C:\\Windows"), ("relative path", "..\\..\\Windows"), ("parent hops", "D:\\a\\..\\..\\Windows")] {
        let (s, _) = raw(local, "PUT", "/api/torrents/settings", &host, &own, &settings(dir)).await;
        t.status(&format!("download folder = {name}"), &[400], s);
    }
    let unc_vpn = json!({ "vpn_config": "\\\\evil.example\\share\\wg.conf", "movies_dir": "D:\\Filme", "series_dir": "D:\\Serien", "upload_kbps": 30 }).to_string();
    let (s, _) = raw(local, "PUT", "/api/torrents/settings", &host, &own, &unc_vpn).await;
    t.status("VPN file on a network share", &[400], s);
    let (s, _) = raw(local, "POST", "/api/torrents/0000000000000000000000000000000000000000/open", &host, &own, "").await;
    t.status("open folder of an unknown torrent", &[404], s);

    // ---------- import ----------
    let import = |body: Value| body.to_string();
    let evil_dirs = import(json!({ "kind": "flimmer-settings", "version": 1, "torrents": { "movies_dir": "\\\\evil.example\\share", "series_dir": "D:\\x" } }));
    let (s, _) = raw(local, "POST", "/api/import", &host, &own, &evil_dirs).await;
    t.status("import with a network share as folder", &[400], s);
    let (s, _) = raw(local, "POST", "/api/import", &host, &own, &import(json!({ "kind": "flimmer-settings", "version": 99 }))).await;
    t.status("import of an unknown version", &[400], s);
    let (s, _) = raw(local, "POST", "/api/import", &host, &own, &import(json!({ "kind": "flimmer-settings", "version": 1, "watchlist": "x" }))).await;
    t.status("import with a wrong shape", &[400], s);
    let big = format!(r#"{{"kind":"flimmer-settings","version":1,"pad":"{}"}}"#, "x".repeat(2 * 1024 * 1024));
    let (s, _) = raw(local, "POST", "/api/import", &host, &own, &big).await;
    t.status("import of 2 MB", &[413], s);
    let sintel = json!({ "id": 45745, "type": "movie", "title": "Sintel", "year": 2010, "poster": "https://evil.example/track.png", "backdrop": "https://image.tmdb.org/t/p/w1280/x.jpg", "rating": 7.2, "overview": "" });
    let (s, _) = raw(local, "POST", "/api/import", &host, &own, &import(json!({ "kind": "flimmer-settings", "version": 1, "watchlist": [sintel] }))).await;
    let (_, a) = raw(local, "GET", "/api/watchlist", &host, &pc_cookie, "").await;
    let item = &json_of(&a)[0];
    t.add("import: foreign image address is dropped", "null", item["poster"].to_string(), s == 200 && item["poster"].is_null() && item["backdrop"].is_string() && item["title"] == "Sintel");

    // ---------- rate limit ----------
    let mut codes = Vec::new();
    for _ in 0..7 {
        codes.push(raw(local, "POST", "/api/autodl/run?dry=1", &host, &own, "").await.0);
    }
    t.add("expensive endpoint in a loop", "429", codes.last().copied().unwrap_or(0), codes[0] == 200 && codes.contains(&429));

    // ---------- phone mode ----------
    let (s, _) = raw(local, "POST", "/api/pair", &host, &[("Origin", origin.as_str())], r#"{"pin":"000000"}"#).await;
    t.status("pairing while the phone mode is off", &[409], s);
    let (s, _) = raw(local, "POST", "/api/lan/pairing", &host, &own, "").await;
    t.status("open a pairing while the phone mode is off", &[400], s);
    let (s, _) = raw(local, "PUT", "/api/lan", &host, &own, r#"{"enabled":true,"ip":"8.8.8.8"}"#).await;
    t.status("phone mode on a public address", &[400], s);
    let (s, _) = raw(local, "PUT", "/api/lan", &host, &own, r#"{"enabled":true}"#).await;
    t.status("turn the phone mode on (PC)", &[204], s);
    let mut lan_ip = None;
    for _ in 0..40 {
        let (_, a) = raw(local, "GET", "/api/lan", &host, &pc_cookie, "").await;
        if let Some(ip) = json_of(&a)["active_ip"].as_str() {
            lan_ip = ip.parse::<Ipv4Addr>().ok();
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let Some(lan_ip) = lan_ip else {
        // A PC without a private network address: the LAN probes cannot run here.
        t.add("LAN probes", "private IP", "none on this PC", false);
        t.print_and_check();
        return;
    };
    t.add("listens on a private address only", "private", if lan_ip.is_private() { "private" } else { "public" }, lan_ip.is_private());
    let lan = SocketAddr::from((lan_ip, port));
    let lan_host = format!("{lan_ip}:{port}");
    let lan_origin = format!("http://{lan_host}");
    let wildcard = TcpStream::connect(lan).await.is_ok();
    t.add("LAN address answers", "yes", if wildcard { "yes" } else { "no" }, wildcard);

    let (s, a) = raw(lan, "GET", "/api/home", &lan_host, &[], "").await;
    t.add("LAN request without device token", "401", s, s == 401 && json_of(&a)["code"] == "pairing_required");
    let (s, _) = raw(lan, "GET", "/api/torrents", &lan_host, &[], "").await;
    t.status("LAN: torrent list without token", &[401], s);
    let (s, _) = raw(lan, "GET", "/api/setup", &lan_host, &pc_cookie, "").await;
    t.status("LAN: the PC's session cookie", &[401], s);
    let (s, _) = raw(lan, "GET", "/api/lan", &lan_host, &[], "").await;
    t.status("LAN: read PIN and devices without token", &[401], s);
    let (s, a) = raw(lan, "GET", "/", &lan_host, &[], "").await;
    t.add("LAN: app shell (pairing page)", "200", s, s == 200 && a.contains("id=root"));
    let (s, _) = raw(lan, "GET", "/", &format!("evil.example:{port}"), &[], "").await;
    t.status("LAN: rebinding Host", &[403], s);
    let (s, _) = raw(lan, "GET", "/", &host, &[], "").await;
    t.status("LAN: Host 127.0.0.1", &[403], s);
    let (s, _) = raw(local, "GET", "/", &lan_host, &[], "").await;
    t.status("loopback: Host = LAN address", &[403], s);
    let (s, _) = raw(lan, "POST", "/api/pair", &lan_host, &[("Origin", lan_origin.as_str())], r#"{"pin":"000000"}"#).await;
    t.status("LAN: pairing before the PC opened one", &[409], s);

    // The PC opens a pairing: PIN and QR code.
    let (s, a) = raw(local, "POST", "/api/lan/pairing", &host, &own, "").await;
    let pairing = json_of(&a)["pairing"].clone();
    let pin = pairing["pin"].as_str().unwrap_or_default().to_owned();
    let link = pairing["link"].as_str().unwrap_or_default().to_owned();
    let qr_ok = pairing["qr"].as_str().is_some_and(|q| q.contains("<svg"));
    t.add("PC opens a pairing (PIN, QR)", "200", s, s == 200 && pin.len() == 6 && link.starts_with(&format!("{lan_origin}/pair#t=")) && qr_ok);
    let (s, _) = raw(lan, "POST", "/api/pair", &lan_host, &[("Origin", "http://evil.example")], &json!({ "pin": pin }).to_string()).await;
    t.status("LAN: pairing from a foreign page", &[403], s);

    // Valid pairing over the LAN address with the PIN.
    let lan_page = [("Origin", lan_origin.as_str()), ("Sec-Fetch-Site", "same-origin"), ("User-Agent", "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 Chrome/131.0 Mobile Safari/537.36")];
    let (s, a) = raw(lan, "POST", "/api/pair", &lan_host, &lan_page, &json!({ "pin": pin }).to_string()).await;
    let device = cookie_of(&a).unwrap_or_default();
    let flags = header_of(&a, "set-cookie").unwrap_or_default();
    t.add("LAN: valid pairing with the PIN", "200", s, s == 200 && flags.contains("HttpOnly") && flags.contains("SameSite=Strict") && flags.contains("Max-Age="));
    let dev = [("Cookie", device.as_str()), ("Origin", lan_origin.as_str()), ("Sec-Fetch-Site", "same-origin")];
    let (s, a) = raw(lan, "GET", "/api/session", &lan_host, &dev, "").await;
    t.add("LAN: session after pairing", "device", json_of(&a)["role"].as_str().unwrap_or("?"), s == 200 && json_of(&a)["role"] == "device" && json_of(&a)["device"] == "Android, Chrome");
    let (s, _) = raw(lan, "GET", "/api/setup", &lan_host, &dev, "").await;
    t.status("LAN: access with device token", &[200], s);
    let (s, _) = raw(lan, "GET", "/api/torrents", &lan_host, &dev, "").await;
    t.status("LAN: torrent list with device token", &[200], s);
    let big_buck = json!({ "id": 10378, "type": "movie", "title": "Big Buck Bunny", "year": 2008, "poster": null, "backdrop": null, "rating": null, "overview": "" }).to_string();
    let (s, _) = raw(lan, "PUT", "/api/watchlist/movie/10378", &lan_host, &dev, &big_buck).await;
    t.status("LAN: device saves a title", &[204], s);
    let (s, _) = raw(lan, "POST", "/api/pair", &lan_host, &lan_page, &json!({ "pin": pin }).to_string()).await;
    t.status("LAN: the same PIN a second time", &[409], s);
    for (name, method, path, body) in [
        ("device changes download folders", "PUT", "/api/torrents/settings", settings("D:\\Filme")),
        ("device resets Flimmer", "POST", "/api/setup/reset", r#"{"confirm":"reset"}"#.to_owned()),
        ("device sets a TMDB token", "PUT", "/api/setup/tmdb", r#"{"token":"x"}"#.to_owned()),
        ("device imports a VPN file", "PUT", "/api/setup/vpn", r#"{"text":"x"}"#.to_owned()),
        ("device releases the VPN kill switch", "POST", "/api/vpn/release", String::new()),
        ("device opens a folder on the PC", "POST", "/api/torrents/0000000000000000000000000000000000000000/open", String::new()),
        ("device deletes files", "DELETE", "/api/torrents/0000000000000000000000000000000000000000?files=1", String::new()),
        ("device reads the pairing PIN", "GET", "/api/lan", String::new()),
        ("device turns the phone mode", "PUT", "/api/lan", r#"{"enabled":true}"#.to_owned()),
        ("device opens a pairing", "POST", "/api/lan/pairing", String::new()),
        ("device exports settings", "POST", "/api/export", String::new()),
        ("device saves an auto-download rule", "PUT", "/api/autodl/rules/movie/45745", r#"{"title":"Sintel"}"#.to_owned()),
        ("device reads the data folder", "GET", "/api/maintenance", String::new()),
    ] {
        let (s, a) = raw(lan, method, path, &lan_host, &dev, &body).await;
        t.add(&format!("LAN: {name}"), "403", s, s == 403 && json_of(&a)["code"] == "pc_only");
    }
    let (s, _) = raw(lan, "PUT", "/api/watchlist/movie/1", &lan_host, &[("Cookie", device.as_str()), ("Origin", "http://evil.example")], &big_buck).await;
    t.status("LAN: cross-origin PUT with device token", &[403], s);
    let forged = format!("{}.{}", device.split('.').next().unwrap_or_default(), "0".repeat(64));
    let (s, _) = raw(lan, "GET", "/api/setup", &lan_host, &[("Cookie", forged.as_str())], "").await;
    t.status("LAN: forged device token", &[401], s);

    // The device shows up on the PC and is revoked there.
    let (_, a) = raw(local, "GET", "/api/lan", &host, &pc_cookie, "").await;
    let listed = json_of(&a)["devices"].clone();
    let id = listed[0]["id"].as_str().unwrap_or_default().to_owned();
    t.add("device list on the PC", "1 device", listed.as_array().map_or(0, Vec::len), listed.as_array().is_some_and(|d| d.len() == 1) && !body_of(&a).contains(device.split('.').nth(1).unwrap_or("?")));
    let (s, _) = raw(local, "DELETE", &format!("/api/lan/devices/{id}"), &host, &own, "").await;
    t.status("PC revokes the device", &[204], s);
    let (s, _) = raw(lan, "GET", "/api/setup", &lan_host, &dev, "").await;
    t.status("LAN: access after revoke", &[401], s);

    // PIN brute force from the LAN.
    let (_, a) = raw(local, "POST", "/api/lan/pairing", &host, &own, "").await;
    let pin = json_of(&a)["pairing"]["pin"].as_str().unwrap_or_default().to_owned();
    let token = json_of(&a)["pairing"]["link"].as_str().and_then(|l| l.split("#t=").nth(1)).unwrap_or_default().to_owned();
    let wrong = if pin == "123456" { "654321" } else { "123456" };
    let mut answers = Vec::new();
    for _ in 0..6 {
        answers.push(raw(lan, "POST", "/api/pair", &lan_host, &lan_page, &json!({ "pin": wrong }).to_string()).await.0);
    }
    t.add("LAN: PIN brute force", "403x4,429", answers.iter().map(u16::to_string).collect::<Vec<_>>().join(","), answers[..4] == [403; 4] && answers[4..] == [429; 2]);
    let (s, a) = raw(lan, "POST", "/api/pair", &lan_host, &lan_page, &json!({ "pin": pin }).to_string()).await;
    t.add("LAN: right PIN during the lockout", "429", s, s == 429 && json_of(&a)["retry_after"].as_u64().is_some_and(|w| w > 200) && cookie_of(&a).is_none());
    // Another device (here: a browser on the PC itself) still pairs, with the QR code's one-time token.
    let local_page = [("Origin", origin.as_str()), ("Sec-Fetch-Site", "same-origin")];
    let (s, a) = raw(local, "POST", "/api/pair", &host, &local_page, &json!({ "token": token }).to_string()).await;
    let browser = cookie_of(&a).unwrap_or_default();
    t.status("browser on the PC pairs with the QR token", &[200], s);
    let (s, a) = raw(local, "GET", "/api/session", &host, &[("Cookie", browser.as_str())], "").await;
    t.add("paired browser on the PC is a device", "device", json_of(&a)["role"].as_str().unwrap_or("?"), s == 200 && json_of(&a)["role"] == "device");
    let (s, _) = raw(local, "POST", "/api/pair", &host, &local_page, &json!({ "token": token }).to_string()).await;
    t.status("QR token a second time", &[409], s);
    let (s, _) = raw(local, "POST", "/api/pair/logout", &host, &[("Cookie", browser.as_str()), ("Origin", origin.as_str())], "").await;
    t.status("device signs itself out", &[204], s);
    let (s, _) = raw(local, "GET", "/api/setup", &host, &[("Cookie", browser.as_str())], "").await;
    t.status("access after signing out", &[401], s);

    // Phone mode off: the LAN address is closed again.
    let (s, _) = raw(local, "PUT", "/api/lan", &host, &own, r#"{"enabled":false}"#).await;
    t.status("turn the phone mode off (PC)", &[204], s);
    let mut closed = false;
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if TcpStream::connect(lan).await.is_err() {
            closed = true;
            break;
        }
    }
    t.add("LAN address after turning it off", "closed", if closed { "closed" } else { "open" }, closed);
    let (s, _) = raw(local, "GET", "/api/setup", &host, &pc_cookie, "").await;
    t.status("desktop window still works", &[200], s);

    t.print_and_check();
    let _ = std::fs::remove_dir_all(&root);
}
