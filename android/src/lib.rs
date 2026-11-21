//! Flimmer for Android: the same server as on the desktop, in-process on 127.0.0.1, with the web UI in the app's
//! WebView. Nothing outside the phone is needed. The stream sniffer is `sniffer` (Android WebViews driven by the
//! Kotlin class `SnifferPlugin`), ads on provider pages are blocked by `adblock`. See `docs/android.md`.
//!
//! Folders (see `flimmer::paths`): the web UI and `providers.json` are part of the library and unpacked to
//! `<app data>/root` when the build changes; per-user data lives in `<app data>/data`; downloads go to the app's
//! folder on the shared storage (`Android/data/local.flimmer.app/files/Movies`).
#![cfg(target_os = "android")]

mod adblock;
mod sniffer;

use std::{
    fmt::Display,
    fs::File,
    io::Write,
    path::Path,
    sync::{Arc, Mutex},
    time::Instant,
};

use include_dir::{Dir, include_dir};
use serde::Serialize;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder, Wry, plugin::PluginHandle};

static WEB: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../web/dist");
const PROVIDERS: &str = include_str!("../../providers.json");
const BUILD: &str = env!("FLIMMER_BUILD_STAMP");
/// Provider pages that load at the same time on a phone.
const MAX_PARALLEL: usize = 4;

/// The Kotlin class `local.flimmer.app.SnifferPlugin`.
pub(crate) type Bridge = PluginHandle<Wry>;

/// Line log in `<app data>/data/logs/android.log` (rewritten on every start). It names provider hosts, no URLs.
#[derive(Clone)]
pub struct Log {
    file: Arc<Mutex<Option<File>>>,
    t0: Instant,
}

impl Log {
    fn open(path: &Path) -> Self {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        Self { file: Arc::new(Mutex::new(File::create(path).ok())), t0: Instant::now() }
    }

    pub fn line(&self, msg: impl Display) {
        if let Ok(mut guard) = self.file.lock()
            && let Some(f) = guard.as_mut()
        {
            let _ = writeln!(f, "{:>8} {msg}", self.t0.elapsed().as_millis());
        }
    }
}

/// Writes the bundled web UI and provider list to `root` unless this build's copy is there already.
fn unpack(root: &Path) -> std::io::Result<()> {
    let stamp = root.join("build");
    if std::fs::read_to_string(&stamp).is_ok_and(|s| s == BUILD) {
        return Ok(());
    }
    let dist = root.join("web").join("dist");
    let _ = std::fs::remove_dir_all(&dist);
    std::fs::create_dir_all(&dist)?;
    WEB.extract(&dist)?;
    std::fs::write(root.join("providers.json"), PROVIDERS)?;
    std::fs::write(stamp, BUILD)
}

/// The port of the last start if it is still free (the page keeps its origin and with it its browser storage),
/// else any free one.
fn find_port(port_file: &Path) -> Result<u16, String> {
    let free = |p: u16| std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, p)).and_then(|l| l.local_addr());
    let last = std::fs::read_to_string(port_file).ok().and_then(|s| s.trim().parse::<u16>().ok());
    let port = last
        .and_then(|p| free(p).ok())
        .or_else(|| free(0).ok())
        .ok_or("Kein freier Port auf 127.0.0.1.")?
        .port();
    let _ = std::fs::write(port_file, port.to_string());
    Ok(port)
}

#[derive(Serialize)]
struct External<'a> {
    url: &'a str,
    mime: Option<&'a str>,
}

/// Binds the server; the address of the UI on success.
fn start(app: &tauri::App, bridge: &Bridge, home: &Path, log: &Log) -> Result<String, String> {
    let root = home.join("root");
    let data = home.join("data");
    unpack(&root).map_err(|e| format!("Die App-Dateien lassen sich nicht entpacken: {e}"))?;
    let media = app.path().video_dir().unwrap_or_else(|_| data.join("Downloads"));
    let _ = std::fs::create_dir_all(&media);

    let (opener, cleaner) = (bridge.clone(), bridge.clone());
    flimmer::host::set(flimmer::host::Host {
        open: Box::new(move |url, mime| {
            opener.run_mobile_plugin::<()>("openExternal", External { url, mime }).map_err(|e| e.to_string())
        }),
        clear_browser: Box::new(move || cleaner.run_mobile_plugin::<()>("clearBrowserData", ()).map_err(|e| e.to_string())),
        browser_cache_dir: app.path().app_cache_dir().unwrap_or_else(|_| home.join("cache")),
        media_dir: media,
        max_parallel: MAX_PARALLEL,
    });
    adblock::spawn(data.join("adblock"), log.clone());

    let port = find_port(&data.join("port"))?;
    let opts = flimmer::Options {
        root,
        home: home.to_path_buf(),
        port,
        sniffer: Some(Arc::new(sniffer::WebviewSniffer::new(bridge.clone(), log.clone()))),
    };
    let server = tauri::async_runtime::block_on(flimmer::bind(opts))?;
    let url = server.url.clone();
    log.line(format_args!("server at {url}"));
    let serve_log = log.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = server.serve(std::future::pending()).await {
            serve_log.line(e);
        }
    });
    Ok(url)
}

/// A page that only says why Flimmer could not start.
fn error_page(msg: &str) -> String {
    let safe: String = msg.chars().filter(|c| !matches!(c, '<' | '>' | '&' | '#' | '%')).collect();
    format!(
        "data:text/html;charset=utf-8,<meta name=viewport content='width=device-width'>\
         <body style='background:%23090c17;color:%23f4f0e8;font:16px sans-serif;padding:24px'>\
         <h3>Flimmer kann nicht starten</h3><p>{safe}</p>"
    )
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let sniffer_plugin = tauri::plugin::Builder::<Wry, ()>::new("sniffer")
        .setup(|app, api| {
            let handle = api.register_android_plugin("local.flimmer.app", "SnifferPlugin")?;
            app.manage(handle);
            Ok(())
        })
        .build();

    tauri::Builder::default()
        .plugin(sniffer_plugin)
        .setup(|app| {
            let home = app.path().app_data_dir()?;
            // Test builds keep the log where adb can read it (`Android/data/local.flimmer.app/files/logs`).
            let shared = app.path().video_dir().ok().and_then(|d| d.parent().map(|p| p.join("logs")));
            let log_dir = shared.filter(|_| cfg!(feature = "inspect")).unwrap_or_else(|| home.join("data").join("logs"));
            let log = Log::open(&log_dir.join("android.log"));
            let bridge: Bridge = app.state::<Bridge>().inner().clone();
            let url = start(app, &bridge, &home, &log).unwrap_or_else(|e| {
                log.line(format_args!("start failed: {e}"));
                error_page(&e)
            });
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url.parse()?)).build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("tauri run failed");
}
