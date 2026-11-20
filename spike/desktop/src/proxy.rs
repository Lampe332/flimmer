//! Tiny local stream proxy + static player page.
//!
//!   GET /player            -> spike/player/index.html
//!   GET /adtest            -> spike/player/adtest.html
//!   GET /hls.min.js        -> hls.js bundle, /dash.all.min.js -> dash.js bundle
//!   GET /mark?name=x       -> writes a marker line into the event log
//!   GET /p?u=<url>&r=<referer>&o=<origin>&a=<user-agent>[&raw=1]
//!                          -> HLS: fetches <url> with those headers; playlists are rewritten so
//!                             every URI goes through /p again (unless raw=1)
//!   GET /d/<hex json {r,o,a}>/<scheme>/<host>/<path>?<query>
//!                          -> DASH: path-shaped so relative segment URLs of the MPD resolve
//!                             back into the proxy; absolute URLs inside the MPD are rewritten

use std::{
    io::{BufRead, BufReader, Read},
    path::PathBuf,
    time::Duration,
};

use serde_json::json;
use tiny_http::{Header, Method, Request, Response, Server};
use ureq::ResponseExt;
use url::Url;

use crate::Log;

pub fn spawn(port: u16, root: PathBuf, log: Log) {
    std::thread::spawn(move || {
        let server = match Server::http(("127.0.0.1", port)) {
            Ok(s) => s,
            Err(e) => {
                log.ev(json!({"kind": "proxy_error", "error": e.to_string()}));
                return;
            }
        };
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(40)))
            .build()
            .into();
        for req in server.incoming_requests() {
            let (root, log, agent) = (root.clone(), log.clone(), agent.clone());
            std::thread::spawn(move || handle(req, port, &root, &log, &agent));
        }
    });
}

fn hdr(k: &str, v: &str) -> Header {
    Header::from_bytes(k.as_bytes(), v.as_bytes()).unwrap()
}

fn cors() -> Vec<Header> {
    vec![
        hdr("Access-Control-Allow-Origin", "*"),
        hdr("Access-Control-Allow-Headers", "*"),
        hdr("Access-Control-Expose-Headers", "*"),
    ]
}

fn text(req: Request, status: u16, body: String, ctype: &str, extra: Vec<Header>) {
    let mut r = Response::from_string(body).with_status_code(status).with_header(hdr("Content-Type", ctype));
    for h in cors().into_iter().chain(extra) {
        r.add_header(h);
    }
    let _ = req.respond(r);
}

fn file(req: Request, path: PathBuf, ctype: &str) {
    match std::fs::read_to_string(&path) {
        Ok(s) => text(req, 200, s, ctype, vec![]),
        Err(e) => text(req, 404, format!("{}: {e}", path.display()), "text/plain", vec![]),
    }
}

fn unhex(s: &str) -> Option<String> {
    if s.len() % 2 != 0 {
        return None;
    }
    let bytes: Option<Vec<u8>> =
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect();
    String::from_utf8(bytes?).ok()
}

struct Ctx {
    port: u16,
    referer: String,
    origin: String,
    ua: String,
}

impl Ctx {
    fn wrap(&self, base: &Url, target: &str) -> String {
        let abs = base.join(target).map(|u| u.to_string()).unwrap_or_else(|_| target.to_string());
        let q = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("r", &self.referer)
            .append_pair("o", &self.origin)
            .append_pair("a", &self.ua)
            .append_pair("u", &abs)
            .finish();
        format!("http://127.0.0.1:{}/p?{q}", self.port)
    }

    fn rewrite_hls(&self, base: &Url, body: &str) -> String {
        let mut out = String::with_capacity(body.len() * 2);
        for line in body.lines() {
            let t = line.trim();
            if t.is_empty() {
                out.push('\n');
            } else if t.starts_with('#') {
                // rewrite every URI="..." attribute (keys, maps, audio/subtitle renditions)
                let mut rest = t;
                while let Some(i) = rest.find("URI=\"") {
                    let after = &rest[i + 5..];
                    let Some(end) = after.find('"') else { break };
                    out.push_str(&rest[..i + 5]);
                    let uri = &after[..end];
                    if uri.starts_with("data:") {
                        out.push_str(uri);
                    } else {
                        out.push_str(&self.wrap(base, uri));
                    }
                    rest = &after[end..];
                }
                out.push_str(rest);
                out.push('\n');
            } else {
                out.push_str(&self.wrap(base, t));
                out.push('\n');
            }
        }
        out
    }
}

/// Absolute http(s) URLs inside an MPD -> /d/<tok>/<scheme>/<host>/... (XML namespaces untouched).
fn rewrite_mpd(body: &str, port: u16, tok: &str, base: Option<&Url>) -> String {
    let mut out = String::with_capacity(body.len() + 256);
    let mut rest = body;
    loop {
        let next = ["https://", "http://"].iter().filter_map(|p| rest.find(p).map(|i| (i, *p))).min();
        let Some((i, p)) = next else { break };
        out.push_str(&rest[..i]);
        let after = &rest[i + p.len()..];
        if after.starts_with("www.w3.org") || after.starts_with("dashif.org") || after.starts_with("127.0.0.1") {
            out.push_str(p);
        } else {
            out.push_str(&format!("http://127.0.0.1:{port}/d/{tok}/{}/", p.trim_end_matches("://")));
        }
        rest = after;
    }
    out.push_str(rest);
    // root-relative URLs ("/path/seg.m4s") would otherwise resolve to the proxy root
    if let Some(b) = base {
        let root = format!("/d/{tok}/{}/{}/", b.scheme(), b.host_str().unwrap_or(""));
        for attr in ["<BaseURL>", "media=\"", "initialization=\"", "sourceURL=\""] {
            out = out.replace(&format!("{attr}/"), &format!("{attr}{root}"));
        }
    }
    out
}

fn handle(req: Request, port: u16, root: &PathBuf, log: &Log, agent: &ureq::Agent) {
    if *req.method() == Method::Options {
        return text(req, 204, String::new(), "text/plain", vec![]);
    }
    let Ok(full) = Url::parse(&format!("http://127.0.0.1{}", req.url())) else {
        return text(req, 400, "bad url".into(), "text/plain", vec![]);
    };
    let q = |k: &str| full.query_pairs().find(|(n, _)| n == k).map(|(_, v)| v.to_string()).unwrap_or_default();
    let js = |pkg: &[&str]| pkg.iter().fold(root.join("node_modules"), |p, s| p.join(s));

    let (target, ctx, raw, dash_tok);
    match full.path() {
        "/player" | "/player/" => return file(req, root.join("player").join("index.html"), "text/html; charset=utf-8"),
        "/adtest" => return file(req, root.join("player").join("adtest.html"), "text/html; charset=utf-8"),
        "/hls.min.js" => return file(req, js(&["hls.js", "dist", "hls.min.js"]), "text/javascript"),
        "/dash.all.min.js" => {
            return file(req, js(&["dashjs", "dist", "modern", "umd", "dash.all.min.js"]), "text/javascript")
        }
        "/mark" => {
            log.ev(json!({"kind": "mark", "name": q("name")}));
            return text(req, 200, "ok".into(), "text/plain", vec![]);
        }
        "/p" => {
            target = q("u");
            ctx = Ctx { port, referer: q("r"), origin: q("o"), ua: q("a") };
            raw = q("raw") == "1";
            dash_tok = None;
        }
        p if p.starts_with("/d/") => {
            // /d/<tok>/<scheme>/<host>/<rest>
            let raw_url = req.url();
            let path_q = &raw_url[3..];
            let mut it = path_q.splitn(3, '/');
            let (Some(tok), Some(scheme), Some(rest)) = (it.next(), it.next(), it.next()) else {
                return text(req, 400, "bad /d path".into(), "text/plain", vec![]);
            };
            let meta: serde_json::Value =
                unhex(tok).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_else(|| json!({}));
            let g = |k: &str| meta[k].as_str().unwrap_or("").to_string();
            target = format!("{scheme}://{rest}");
            ctx = Ctx { port, referer: g("r"), origin: g("o"), ua: g("a") };
            raw = false;
            dash_tok = Some(tok.to_string());
        }
        _ => return text(req, 404, "not found".into(), "text/plain", vec![]),
    }

    let range = req
        .headers()
        .iter()
        .find(|h| h.field.equiv("Range"))
        .map(|h| h.value.as_str().to_string());

    let mut rb = agent.get(&target).header("Accept", "*/*").header("Accept-Language", "en-US,en;q=0.9");
    if !ctx.ua.is_empty() {
        rb = rb.header("User-Agent", &ctx.ua);
    }
    if !ctx.referer.is_empty() {
        rb = rb.header("Referer", &ctx.referer);
    }
    if !ctx.origin.is_empty() {
        rb = rb.header("Origin", &ctx.origin);
    }
    if let Some(r) = &range {
        rb = rb.header("Range", r);
    }
    let resp = match rb.call() {
        Ok(r) => r,
        Err(e) => {
            log.ev(json!({"kind": "proxy", "status": 0, "error": e.to_string(), "url": target}));
            return text(req, 502, format!("upstream error: {e}"), "text/plain", vec![]);
        }
    };
    let status = resp.status().as_u16();
    let final_url = resp.get_uri().to_string();
    let get = |k: &str| resp.headers().get(k).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let (ctype, clen, crange, cenc) = (get("content-type"), get("content-length"), get("content-range"), get("content-encoding"));

    let mut reader = BufReader::with_capacity(64 * 1024, resp.into_body().into_reader());
    let head = reader
        .fill_buf()
        .map(|b| String::from_utf8_lossy(&b[..b.len().min(400)]).trim_start_matches('\u{feff}').trim_start().to_string())
        .unwrap_or_default();
    let is_playlist = head.starts_with("#EXTM3U");
    let is_mpd = head.contains("<MPD");
    log.ev(json!({"kind": "proxy", "status": status, "ctype": ctype, "playlist": is_playlist || is_mpd,
                  "len": clen, "url": target.chars().take(200).collect::<String>()}));

    if is_playlist || (is_mpd && dash_tok.is_some()) {
        let mut body = String::new();
        if let Err(e) = reader.read_to_string(&mut body) {
            return text(req, 502, format!("read error: {e}"), "text/plain", vec![]);
        }
        let base = Url::parse(&final_url).or_else(|_| Url::parse(&target));
        let (out, ct) = match (&dash_tok, raw, base) {
            (Some(tok), _, b) if is_mpd => (rewrite_mpd(&body, port, tok, b.as_ref().ok()), "application/dash+xml"),
            (None, false, Ok(base)) => (ctx.rewrite_hls(&base, &body), "application/vnd.apple.mpegurl"),
            _ => (body, "application/vnd.apple.mpegurl"),
        };
        return text(req, status, out, ct, vec![hdr("X-Final-Url", &final_url)]);
    }

    let mut headers = cors();
    headers.push(hdr("X-Final-Url", &final_url));
    if !ctype.is_empty() {
        headers.push(hdr("Content-Type", &ctype));
    }
    if !crange.is_empty() {
        headers.push(hdr("Content-Range", &crange));
    }
    let len = if cenc.is_empty() { clen.parse::<usize>().ok() } else { None };
    let _ = req.respond(Response::new(tiny_http::StatusCode(status), headers, reader, len, None));
}
