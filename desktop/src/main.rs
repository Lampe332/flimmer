//! Flimmer desktop app: runs the Flimmer server in-process on 127.0.0.1 (43300, or the next free port; `--port`
//! pins one), gives it a stream sniffer (hidden WebView2 windows) and shows the web UI in the main window.
//!
//! Folders (see `flimmer::paths`): resources next to the exe when installed, else `--root` or the first folder
//! above the exe with `providers.json` (development). Per-user data: `--data`, else `%LOCALAPPDATA%\<identifier>`
//! when installed (the folder the uninstaller offers to delete; `%LOCALAPPDATA%\Flimmer` is the install folder),
//! else the resource folder (development, as before).

#![allow(non_snake_case)] // binary name `Flimmer`
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod sniffer;

use std::{
    fmt::Display,
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use windows::{
    Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW},
    core::HSTRING,
};

/// Line log in `<home>\data\logs\desktop.log` (rewritten on every start).
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

/// The value after `flag`, if given.
fn arg(flag: &str) -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == flag {
            return args.next();
        }
    }
    None
}

/// Resources and per-user data folder: `(root, home)`.
fn find_folders(identifier: &str) -> Option<(PathBuf, PathBuf)> {
    let data = arg("--data").map(PathBuf::from);
    if let Some(root) = arg("--root").map(PathBuf::from) {
        return Some((root.clone(), data.unwrap_or(root)));
    }
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?.to_path_buf();
    if exe_dir.join("providers.json").is_file() {
        // Installed: resources sit next to the exe, data goes to the user's local app data.
        let home = data.or_else(|| std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join(identifier)))?;
        return Some((exe_dir, home));
    }
    let root = exe.ancestors().skip(1).find(|d| d.join("providers.json").is_file())?.to_path_buf();
    Some((root.clone(), data.unwrap_or(root)))
}

fn port_free(port: u16) -> bool {
    std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).is_ok()
}

/// `--port <n>` as given. Else the port of the last start (so the page keeps its origin), else 43300, else the
/// next free one up to 43399. Another program (or a second Flimmer with other data) on 43300 is no problem.
fn find_port(port_file: &Path) -> Result<u16, String> {
    if let Some(p) = arg("--port") {
        return p.parse().map_err(|_| "--port braucht eine Zahl zwischen 1 und 65535.".to_owned());
    }
    let last = std::fs::read_to_string(port_file).ok().and_then(|s| s.trim().parse::<u16>().ok());
    let base = flimmer::DEFAULT_PORT;
    let port = pick_port(last, base..base + 100).ok_or("Kein freier Port zwischen 43300 und 43399.")?;
    let _ = std::fs::write(port_file, port.to_string());
    Ok(port)
}

/// The last port if it is free, else the first free one in `range`.
fn pick_port(last: Option<u16>, range: std::ops::Range<u16>) -> Option<u16> {
    last.into_iter().chain(range).find(|&p| port_free(p))
}

/// Held for the whole run: a second start with the same data folder would share the database and torrents.
fn lock_data(data: &Path) -> Option<File> {
    use std::os::windows::fs::OpenOptionsExt;
    let _ = std::fs::create_dir_all(data);
    std::fs::OpenOptions::new().create(true).truncate(false).write(true).share_mode(0).open(data.join("flimmer.lock")).ok()
}

/// Set once the main window started closing; a second click on the X closes at once.
static CLOSING: AtomicBool = AtomicBool::new(false);
/// Time the page gets to send its last progress save to the in-process server before the window goes.
const SAVE_GRACE: Duration = Duration::from_millis(400);

fn fail(msg: &str) -> ! {
    // SAFETY: plain Win32 call with valid, NUL-terminated strings owned by this frame.
    unsafe {
        MessageBoxW(None, &HSTRING::from(msg), &HSTRING::from("Flimmer"), MB_OK | MB_ICONERROR);
    }
    std::process::exit(1)
}

/// The folder for `extensions_path`, if it holds at least one unpacked extension.
fn extensions(root: &Path) -> Option<PathBuf> {
    let dir = root.join("desktop").join("extensions");
    let found = std::fs::read_dir(&dir).ok()?.flatten().any(|e| e.path().join("manifest.json").is_file());
    found.then_some(dir)
}

fn main() {
    let context = tauri::generate_context!();
    let Some((root, home)) = find_folders(&context.config().identifier) else {
        fail("Flimmer-Ordner nicht gefunden (providers.json fehlt). Start mit --root <Ordner>.")
    };
    let data = home.join("data");
    let Some(_lock) = lock_data(&data) else {
        fail("Flimmer läuft schon. Schau in der Taskleiste nach dem offenen Fenster.

Flimmer is already running. Look for its window in the taskbar.")
    };
    let port = find_port(&data.join("port")).unwrap_or_else(|e| fail(&e));
    let log = Log::open(&data.join("logs").join("desktop.log"));
    let webview_data = data.join("webview");
    // "Cache leeren" in the settings: the source-search profile is only free before its windows exist.
    let marker = data.join(flimmer::setup::CLEAR_SCAN_PROFILE);
    if marker.is_file() {
        let _ = std::fs::remove_dir_all(webview_data.join("scan"));
        let _ = std::fs::remove_file(&marker);
        log.line("cleared the source-search browser profile");
    }

    tauri::Builder::default()
        .setup(move |app| {
            let ext = extensions(&root);
            match &ext {
                Some(dir) => log.line(format_args!("extensions from {}", dir.display())),
                None => log.line("no extension in desktop\\extensions: scanning without ad blocker"),
            }
            let sniffer =
                sniffer::WebviewSniffer::new(app.handle().clone(), webview_data.join("scan"), ext, log.clone());
            sniffer.warm();
            let opts = flimmer::Options {
                root: root.clone(),
                home: home.clone(),
                port,
                sniffer: Some(Arc::new(sniffer)),
            };
            let server = match tauri::async_runtime::block_on(flimmer::bind(opts)) {
                Ok(s) => s,
                Err(e) => {
                    log.line(format_args!("start failed: {e}"));
                    fail(&format!("Flimmer kann nicht starten:\n{e}\n\nLäuft Flimmer schon?"))
                }
            };
            let url = server.url.clone();
            log.line(format_args!("server at {url}"));
            let serve_log = log.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = server.serve(std::future::pending()).await {
                    serve_log.line(e);
                }
            });

            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url.parse()?))
                .title("Flimmer")
                .inner_size(1400.0, 900.0)
                .data_directory(webview_data.join("main"))
                .build()?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            match event {
                // WebView2 fires no pagehide when the window closes, so the player would lose up to 10 s of
                // progress: ask the page to save first, then close for real.
                WindowEvent::CloseRequested { api, .. } if !CLOSING.swap(true, Ordering::SeqCst) => {
                    api.prevent_close();
                    if let Some(main) = window.app_handle().get_webview_window("main") {
                        let _ = main.eval("window.dispatchEvent(new Event('flimmer:beforeclose'))");
                        tauri::async_runtime::spawn(async move {
                            tokio::time::sleep(SAVE_GRACE).await;
                            let _ = main.destroy();
                        });
                    } else {
                        let _ = window.destroy();
                    }
                }
                // hidden scan windows must not keep the app alive
                WindowEvent::Destroyed => window.app_handle().exit(0),
                _ => {}
            }
        })
        .run(context)
        .expect("tauri run failed");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busy_ports_are_skipped() {
        // Another program holds a port: the pick moves on to the next free one.
        let busy = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let p = busy.local_addr().unwrap().port();
        assert!(!port_free(p));
        let picked = pick_port(Some(p), p..p.saturating_add(20)).unwrap();
        assert_ne!(picked, p);
        assert!(picked > p);
        // A free last port is kept, so the page keeps its origin (and its browser storage).
        drop(busy);
        assert_eq!(pick_port(Some(p), p.saturating_add(1)..p.saturating_add(20)), Some(p));
    }
}
