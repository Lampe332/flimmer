//! Flimmer feasibility spike: Tauri v2 window + WebView2 hooks.
//!
//! Modes (`--mode`):
//!   none        plain WebView2, only logging + popup blocking
//!   ext         extension via wry (`browser_extensions_enabled` + `extensions_path`)
//!   ext-direct  extension via ICoreWebView2Profile7::AddBrowserExtension
//!   adblock     Brave adblock-rust engine in WebResourceRequested (all frames / source kinds)
//!
//! Every mode logs requests, media candidates, popups and navigations as JSONL (`--log`)
//! and runs the local HLS proxy + player page on `--proxy-port`.

mod proxy;

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    fs::File,
    io::Write,
    path::PathBuf,
    rc::Rc,
    sync::{Arc, Mutex},
    time::Instant,
};

use serde_json::{json, Value};
use tauri::{WebviewUrl, WebviewWindowBuilder};
use webview2_com::{
    take_pwstr, Microsoft::Web::WebView2::Win32::*,
    NavigationCompletedEventHandler, NavigationStartingEventHandler,
    NewWindowRequestedEventHandler, ProfileAddBrowserExtensionCompletedHandler,
    ProfileGetBrowserExtensionsCompletedHandler, WebResourceRequestedEventHandler,
    WebResourceResponseReceivedEventHandler,
};
use windows::{
    core::{Interface, BOOL, HSTRING, PCWSTR, PWSTR},
    Win32::System::Com::IStream,
};

#[derive(Clone)]
pub struct Log {
    file: Arc<Mutex<File>>,
    t0: Instant,
}

impl Log {
    pub fn ev(&self, mut v: Value) {
        v["t"] = json!(self.t0.elapsed().as_millis() as u64);
        if let Ok(mut f) = self.file.lock() {
            let _ = writeln!(f, "{v}");
        }
    }
}

#[derive(Clone)]
struct Cfg {
    mode: String,
    url: String,
    root: PathBuf,
    ext_dir: Option<PathBuf>,
    data_dir: PathBuf,
    cdp_port: u16,
    proxy_port: u16,
    lock_origin: bool,
}

fn parse_args() -> (Cfg, PathBuf) {
    let mut m = HashMap::new();
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        if let Some(key) = k.strip_prefix("--") {
            if key == "lock-origin" {
                m.insert(key.to_string(), "1".to_string());
            } else if let Some(v) = it.next() {
                m.insert(key.to_string(), v);
            }
        }
    }
    let root = PathBuf::from(m.get("root").cloned().expect("--root <spike dir> required"));
    let mode = m.get("mode").cloned().unwrap_or_else(|| "none".into());
    let cfg = Cfg {
        url: m.get("url").cloned().unwrap_or_else(|| "about:blank".into()),
        ext_dir: m.get("ext-dir").map(PathBuf::from),
        data_dir: m
            .get("data-dir")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("userdata").join(&mode)),
        cdp_port: m.get("cdp-port").and_then(|s| s.parse().ok()).unwrap_or(9222),
        proxy_port: m.get("proxy-port").and_then(|s| s.parse().ok()).unwrap_or(43311),
        lock_origin: m.contains_key("lock-origin"),
        mode,
        root: root.clone(),
    };
    let log = m
        .get("log")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("logs").join("events.jsonl"));
    (cfg, log)
}

fn main() {
    let (cfg, log_path) = parse_args();
    let file = File::create(&log_path).expect("cannot create log file");
    let log = Log { file: Arc::new(Mutex::new(file)), t0: Instant::now() };
    log.ev(json!({"kind": "start", "mode": cfg.mode, "url": cfg.url}));

    proxy::spawn(cfg.proxy_port, cfg.root.clone(), log.clone());

    tauri::Builder::default()
        .setup(move |app| {
            let args = format!(
                "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection \
                 --remote-debugging-port={} --autoplay-policy=no-user-gesture-required",
                cfg.cdp_port
            );
            let mut b = WebviewWindowBuilder::new(
                app,
                "main",
                WebviewUrl::External("about:blank".parse().unwrap()),
            )
            .title(format!("Flimmer Spike [{}]", cfg.mode))
            .inner_size(1280.0, 720.0)
            .focused(false)
            .data_directory(cfg.data_dir.clone())
            .additional_browser_args(&args);

            if cfg.mode == "ext" || cfg.mode == "ext-direct" {
                b = b.browser_extensions_enabled(true);
            }
            if cfg.mode == "ext" {
                // wry loads every sub-folder of this path as an unpacked extension
                let parent = cfg.ext_dir.clone().expect("--ext-dir required");
                b = b.extensions_path(parent);
            }
            let win = match b.build() {
                Ok(w) => w,
                Err(e) => {
                    log.ev(json!({"kind": "build_error", "error": e.to_string()}));
                    return Err(e.into());
                }
            };
            let (cfg2, log2) = (cfg.clone(), log.clone());
            win.with_webview(move |pw| {
                if let Err(e) = unsafe { hook(pw.controller(), pw.environment(), cfg2, log2.clone()) }
                {
                    log2.ev(json!({"kind": "hook_error", "error": e.to_string(), "hresult": format!("{:#010x}", e.code().0)}));
                }
            })?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("tauri run failed");
}

struct State {
    core: ICoreWebView2,
    env: ICoreWebView2Environment,
    engine: Option<adblock::Engine>,
    log: Log,
    top_nav: RefCell<String>,
    #[allow(dead_code)]
    start_host: String,
    lock_origin: bool,
    listed_ext: Cell<bool>,
}

fn pw(f: impl FnOnce(*mut PWSTR) -> windows::core::Result<()>) -> String {
    let mut p = PWSTR::null();
    if f(&mut p).is_ok() {
        take_pwstr(p)
    } else {
        String::new()
    }
}

fn host_of(u: &str) -> String {
    url::Url::parse(u)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default()
}

fn is_local(u: &str) -> bool {
    matches!(host_of(u).as_str(), "localhost" | "127.0.0.1" | "[::1]")
}

fn cut(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

unsafe fn req_headers(req: &ICoreWebView2WebResourceRequest) -> Value {
    let mut out = serde_json::Map::new();
    if let Ok(h) = req.Headers() {
        if let Ok(it) = h.GetIterator() {
            let mut has = BOOL(0);
            let _ = it.HasCurrentHeader(&mut has);
            while has.as_bool() {
                let (mut n, mut v) = (PWSTR::null(), PWSTR::null());
                if it.GetCurrentHeader(&mut n, &mut v).is_ok() {
                    out.insert(take_pwstr(n), Value::String(take_pwstr(v)));
                }
                has = BOOL(0);
                if it.MoveNext(&mut has).is_err() {
                    break;
                }
            }
        }
    }
    Value::Object(out)
}

fn media_url(u: &str) -> bool {
    let path = u.split(['?', '#']).next().unwrap_or(u).to_ascii_lowercase();
    path.ends_with(".m3u8") || path.ends_with(".mpd") || path.ends_with(".mp4") || path.ends_with(".m3u")
        || u.to_ascii_lowercase().contains(".m3u8")
}

fn media_ctype(c: &str) -> bool {
    let c = c.to_ascii_lowercase();
    c.contains("mpegurl") || c.contains("dash+xml") || c.starts_with("video/mp4")
}

fn load_engine(root: &PathBuf, log: &Log) -> adblock::Engine {
    let t = Instant::now();
    let mut set = adblock::lists::FilterSet::new(false);
    let mut lines = 0usize;
    for name in ["easylist.txt", "easyprivacy.txt", "ublock-filters.txt"] {
        match std::fs::read_to_string(root.join("lists").join(name)) {
            Ok(text) => {
                lines += text.lines().count();
                set.add_filter_list(text, adblock::lists::ParseOptions::default());
            }
            Err(e) => log.ev(json!({"kind": "list_error", "list": name, "error": e.to_string()})),
        }
    }
    let engine = adblock::Engine::new_with_filter_set(set);
    log.ev(json!({"kind": "engine", "list_lines": lines, "build_ms": t.elapsed().as_millis() as u64}));
    engine
}

unsafe fn hook(
    controller: ICoreWebView2Controller,
    env: ICoreWebView2Environment,
    cfg: Cfg,
    log: Log,
) -> windows::core::Result<()> {
    let core = controller.CoreWebView2()?;
    // never play provider audio on the user's speakers during the spike
    if let Ok(c8) = core.cast::<ICoreWebView2_8>() {
        let _ = c8.SetIsMuted(true);
    }

    let engine = (cfg.mode == "adblock").then(|| load_engine(&cfg.root, &log));
    let st = Rc::new(State {
        core: core.clone(),
        env,
        engine,
        log: log.clone(),
        top_nav: RefCell::new(String::new()),
        start_host: host_of(&cfg.url),
        lock_origin: cfg.lock_origin,
        listed_ext: Cell::new(false),
    });

    // --- all requests of all frames, workers included -------------------------------
    let core22 = core.cast::<ICoreWebView2_22>()?;
    core22.AddWebResourceRequestedFilterWithRequestSourceKinds(
        &HSTRING::from("*"),
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
        COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL,
    )?;
    let mut token = Default::default();
    let s = st.clone();
    core.add_WebResourceRequested(
        &WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
            if let Some(args) = args {
                on_request(&s, &args)?;
            }
            Ok(())
        })),
        &mut token,
    )?;

    // --- responses: find playlists by content-type even if the URL is obfuscated ----
    let s = st.clone();
    core.cast::<ICoreWebView2_2>()?.add_WebResourceResponseReceived(
        &WebResourceResponseReceivedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let req = args.Request()?;
            let resp = args.Response()?;
            let uri = pw(|p| req.Uri(p));
            let ctype = resp
                .Headers()
                .map(|h| pw(|p| h.GetHeader(&HSTRING::from("Content-Type"), p)))
                .unwrap_or_default();
            if media_ctype(&ctype) || media_url(&uri) {
                let mut status = 0;
                let _ = resp.StatusCode(&mut status);
                s.log.ev(json!({"kind": "media_resp", "url": uri, "status": status,
                                "ctype": ctype, "headers": req_headers(&req)}));
            }
            Ok(())
        })),
        &mut token,
    )?;

    // --- popups: never --------------------------------------------------------------
    let s = st.clone();
    core.add_NewWindowRequested(
        &NewWindowRequestedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let uri = pw(|p| args.Uri(p));
            let mut user = BOOL(0);
            let _ = args.IsUserInitiated(&mut user);
            args.SetHandled(true)?;
            s.log.ev(json!({"kind": "popup_blocked", "url": cut(&uri, 300), "user_initiated": user.as_bool()}));
            Ok(())
        })),
        &mut token,
    )?;

    // --- top-level navigation -------------------------------------------------------
    let s = st.clone();
    core.add_NavigationStarting(
        &NavigationStartingEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let uri = pw(|p| args.Uri(p));
            let (mut user, mut redir) = (BOOL(0), BOOL(0));
            let _ = args.IsUserInitiated(&mut user);
            let _ = args.IsRedirected(&mut redir);
            let foreign = s.lock_origin
                && uri.starts_with("http")
                && !is_local(&uri);
            if foreign {
                args.SetCancel(true)?;
                s.log.ev(json!({"kind": "nav_blocked", "url": cut(&uri, 300), "user_initiated": user.as_bool()}));
            } else {
                *s.top_nav.borrow_mut() = uri.clone();
                s.log.ev(json!({"kind": "nav", "url": cut(&uri, 300), "user_initiated": user.as_bool(), "redirected": redir.as_bool()}));
            }
            Ok(())
        })),
        &mut token,
    )?;

    let s = st.clone();
    core.add_FrameNavigationStarting(
        &NavigationStartingEventHandler::create(Box::new(move |_, args| {
            if let Some(args) = args {
                let uri = pw(|p| args.Uri(p));
                if uri.starts_with("http") {
                    s.log.ev(json!({"kind": "frame_nav", "url": cut(&uri, 300)}));
                }
            }
            Ok(())
        })),
        &mut token,
    )?;

    let s = st.clone();
    let ext_mode = cfg.mode.starts_with("ext");
    core.add_NavigationCompleted(
        &NavigationCompletedEventHandler::create(Box::new(move |_, args| {
            let mut ok = BOOL(0);
            if let Some(args) = args {
                let _ = args.IsSuccess(&mut ok);
            }
            s.log.ev(json!({"kind": "nav_done", "success": ok.as_bool()}));
            if ext_mode && !s.listed_ext.replace(true) {
                let _ = list_extensions(&s.core, &s.log, "after_first_nav");
            }
            Ok(())
        })),
        &mut token,
    )?;

    // --- extensions -----------------------------------------------------------------
    let start = HSTRING::from(cfg.url.as_str());
    if cfg.mode == "ext-direct" {
        let dir = cfg.ext_dir.clone().expect("--ext-dir required");
        let profile = core.cast::<ICoreWebView2_13>()?.Profile()?.cast::<ICoreWebView2Profile7>()?;
        let (core2, log2) = (core.clone(), log.clone());
        profile.AddBrowserExtension(
            &HSTRING::from(dir.as_path()),
            &ProfileAddBrowserExtensionCompletedHandler::create(Box::new(move |hr, ext| {
                let mut ev = json!({"kind": "ext_add", "ok": hr.is_ok()});
                if let Err(e) = &hr {
                    ev["hresult"] = json!(format!("{:#010x}", e.code().0));
                    ev["error"] = json!(e.message());
                }
                if let Some(ext) = ext {
                    ev["id"] = json!(pw(|p| ext.Id(p)));
                    ev["name"] = json!(pw(|p| ext.Name(p)));
                    let mut en = BOOL(0);
                    let _ = ext.IsEnabled(&mut en);
                    ev["enabled"] = json!(en.as_bool());
                }
                log2.ev(ev);
                core2.Navigate(&start)?;
                Ok(())
            })),
        )?;
    } else {
        if ext_mode {
            let _ = list_extensions(&core, &log, "at_hook");
        }
        core.Navigate(&start)?;
    }
    Ok(())
}

unsafe fn list_extensions(core: &ICoreWebView2, log: &Log, when: &'static str) -> windows::core::Result<()> {
    let profile = core.cast::<ICoreWebView2_13>()?.Profile()?.cast::<ICoreWebView2Profile7>()?;
    let log = log.clone();
    profile.GetBrowserExtensions(&ProfileGetBrowserExtensionsCompletedHandler::create(Box::new(
        move |hr, list| {
            let mut items = vec![];
            if let Some(list) = list {
                let mut n = 0u32;
                let _ = list.Count(&mut n);
                for i in 0..n {
                    if let Ok(ext) = list.GetValueAtIndex(i) {
                        let mut en = BOOL(0);
                        let _ = ext.IsEnabled(&mut en);
                        items.push(json!({"id": pw(|p| ext.Id(p)), "name": pw(|p| ext.Name(p)), "enabled": en.as_bool()}));
                    }
                }
            }
            log.ev(json!({"kind": "ext_list", "when": when, "ok": hr.is_ok(), "extensions": items}));
            Ok(())
        },
    )))
}

unsafe fn on_request(
    st: &State,
    args: &ICoreWebView2WebResourceRequestedEventArgs,
) -> windows::core::Result<()> {
    let req = args.Request()?;
    let uri = pw(|p| req.Uri(p));
    if !uri.starts_with("http") {
        return Ok(());
    }
    let mut ctx = COREWEBVIEW2_WEB_RESOURCE_CONTEXT(0);
    args.ResourceContext(&mut ctx)?;
    let referer = req
        .Headers()
        .map(|h| pw(|p| h.GetHeader(&HSTRING::from("Referer"), p)))
        .unwrap_or_default();
    let top = st.top_nav.borrow().clone();
    let is_top_doc = ctx == COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT && uri == top;

    let rtype = match ctx {
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT => "subdocument",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_STYLESHEET => "stylesheet",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_IMAGE => "image",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_MEDIA => "media",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FONT => "font",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_SCRIPT => "script",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_XML_HTTP_REQUEST | COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FETCH => "xmlhttprequest",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_WEBSOCKET => "websocket",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_PING => "ping",
        _ => "other",
    };

    let mut blocked = false;
    if let Some(engine) = &st.engine {
        if !is_top_doc && !is_local(&uri) {
            let source = if referer.is_empty() { top.clone() } else { referer.clone() };
            if let Ok(r) = adblock::request::Request::new(&uri, &source, rtype, "GET") {
                blocked = engine.check_network_request(&r).should_block();
            }
        }
    }
    if blocked {
        let resp = st.env.CreateWebResourceResponse(
            None::<&IStream>,
            403,
            &HSTRING::from("Blocked"),
            PCWSTR::null(),
        )?;
        args.SetResponse(&resp)?;
    }
    st.log.ev(json!({"kind": "req", "type": if is_top_doc { "document" } else { rtype },
                     "url": cut(&uri, 300), "blocked": blocked, "ref": host_of(&referer)}));
    if media_url(&uri) {
        st.log.ev(json!({"kind": "media_req", "url": uri, "headers": req_headers(&req)}));
    }
    Ok(())
}
