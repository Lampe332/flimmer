//! Flimmer desktop app: runs the Flimmer server in-process on 127.0.0.1:43300, gives it a
//! stream sniffer (hidden WebView2 windows) and shows the web UI in the main window.

#![allow(non_snake_case)] // binary name `Flimmer`
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod sniffer;

use std::{
    fmt::Display,
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use windows::{
    Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW},
    core::HSTRING,
};

/// Line log in `data\logs\desktop.log` (rewritten on every start).
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

/// `--root <dir>`, else the first folder above the exe that holds `providers.json`.
fn find_root() -> Option<PathBuf> {
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--root" {
            return args.next().map(PathBuf::from);
        }
    }
    let exe = std::env::current_exe().ok()?;
    exe.ancestors().skip(1).find(|d| d.join("providers.json").is_file()).map(Path::to_path_buf)
}

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
    let Some(root) = find_root() else {
        fail("Flimmer-Ordner nicht gefunden (providers.json fehlt). Start mit --root <Ordner>.")
    };
    let log = Log::open(&root.join("data").join("logs").join("desktop.log"));
    let webview_data = root.join("data").join("webview");

    tauri::Builder::default()
        .setup(move |app| {
            let ext = extensions(&root);
            match &ext {
                Some(dir) => log.line(format_args!("extensions from {}", dir.display())),
                None => log.line("no extension in desktop\\extensions: scanning without ad blocker"),
            }
            let sniffer =
                sniffer::WebviewSniffer::new(app.handle().clone(), webview_data.join("scan"), ext, log.clone());
            let opts = flimmer::Options {
                root: root.clone(),
                port: flimmer::DEFAULT_PORT,
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
            // hidden scan windows must not keep the app alive
            if window.label() == "main" && matches!(event, WindowEvent::Destroyed) {
                window.app_handle().exit(0);
            }
        })
        .run(tauri::generate_context!())
        .expect("tauri run failed");
}
