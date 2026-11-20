//! Stream sniffer: one hidden WebView2 window per provider page.
//!
//! The page is loaded top-level (no iframe sandbox for the provider to detect) with uBlock
//! Origin Lite, muted, with popups, foreign top navigations, downloads and permission prompts
//! denied. Every playlist / mp4 request of any frame or worker is reported with its request
//! headers; the server decides which one is the stream and then drops the channel.

use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use flimmer::sniff::{Candidate, CandidateKind, SniffFuture, Sniffer};
use tauri::{AppHandle, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tokio::sync::{mpsc::UnboundedSender, oneshot};
use webview2_com::{
    AddScriptToExecuteOnDocumentCreatedCompletedHandler, CallDevToolsProtocolMethodCompletedHandler,
    DownloadStartingEventHandler, Microsoft::Web::WebView2::Win32::*, NavigationCompletedEventHandler,
    NavigationStartingEventHandler,
    NewWindowRequestedEventHandler, PermissionRequestedEventHandler, WebResourceRequestedEventHandler,
    WebResourceResponseReceivedEventHandler, take_pwstr,
};
use windows::core::{BOOL, HSTRING, Interface, PWSTR};

use crate::Log;

/// Separate from the main window's profile: these flags and the extension are per browser process.
const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection \
    --autoplay-policy=no-user-gesture-required --mute-audio --disable-background-timer-throttling \
    --disable-renderer-backgrounding --disable-backgrounding-occluded-windows";
const SIZE: (f64, f64) = (1280.0, 720.0);
/// Real clicks on the player centre for pages that wait for one (seconds after load start).
const CLICK_AT: [u64; 3] = [7, 12, 18];
const GIVE_UP: Duration = Duration::from_secs(25);

/// Runs in every frame: start paused videos once the page had ~6 s to do it itself.
const PLAY_SCRIPT: &str = r#"(() => {
  let n = 0;
  const tick = () => {
    try {
      for (const v of document.querySelectorAll('video')) {
        if (v.paused) { v.muted = true; const p = v.play(); if (p) p.catch(() => {}); }
      }
    } catch {}
    if (++n < 7) setTimeout(tick, 2500);
  };
  setTimeout(tick, 6000);
})();"#;

pub struct WebviewSniffer {
    app: AppHandle,
    data_dir: PathBuf,
    /// Folder whose sub-folders are unpacked extensions (uBlock Origin Lite).
    ext_dir: Option<PathBuf>,
    log: Log,
    seq: AtomicU64,
    popups: Arc<AtomicU64>,
}

impl WebviewSniffer {
    pub fn new(app: AppHandle, data_dir: PathBuf, ext_dir: Option<PathBuf>, log: Log) -> Self {
        Self { app, data_dir, ext_dir, log, seq: AtomicU64::new(1), popups: Arc::new(AtomicU64::new(0)) }
    }
}

impl Sniffer for WebviewSniffer {
    fn sniff(&self, url: String, out: UnboundedSender<Candidate>) -> SniffFuture {
        let job = Job {
            app: self.app.clone(),
            data_dir: self.data_dir.clone(),
            ext_dir: self.ext_dir.clone(),
            log: self.log.clone(),
            n: self.seq.fetch_add(1, Ordering::Relaxed),
            popups: self.popups.clone(),
        };
        Box::pin(job.run(url, out))
    }
}

struct Job {
    app: AppHandle,
    data_dir: PathBuf,
    ext_dir: Option<PathBuf>,
    log: Log,
    n: u64,
    popups: Arc<AtomicU64>,
}

/// Closes the window however the sniff ends (also when the task is dropped).
struct CloseOnDrop(WebviewWindow);

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        let _ = self.0.destroy();
    }
}

fn host_of(u: &str) -> String {
    url::Url::parse(u).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default()
}

impl Job {
    async fn run(self, url: String, out: UnboundedSender<Candidate>) -> Result<(), String> {
        let host = host_of(&url);
        if host.is_empty() {
            return Err("provider URL has no host".into());
        }
        let label = format!("scan-{}", self.n);
        let blank = WebviewUrl::External("about:blank".parse().expect("static URL"));
        let mut b = WebviewWindowBuilder::new(&self.app, &label, blank)
            .title("Flimmer Scan")
            .inner_size(SIZE.0, SIZE.1)
            .visible(false)
            .focused(false)
            .skip_taskbar(true)
            .data_directory(self.data_dir.clone())
            .additional_browser_args(BROWSER_ARGS);
        if let Some(ext) = &self.ext_dir {
            b = b.browser_extensions_enabled(true).extensions_path(ext);
        }
        let win = b.build().map_err(|e| format!("scan window: {e}"))?;
        let win = CloseOnDrop(win);

        let (ready_tx, ready_rx) = oneshot::channel();
        let (fail_tx, mut fail_rx) = oneshot::channel();
        let page = Page {
            n: self.n,
            url: url.clone(),
            out: out.clone(),
            log: self.log.clone(),
            popups: self.popups.clone(),
            seen: RefCell::new(HashSet::new()),
            hosts: RefCell::new(HashSet::from([host.clone()])),
            ua: RefCell::new(String::new()),
            loading: Cell::new(false),
            fail: Cell::new(Some(fail_tx)),
        };
        win.0
            .with_webview(move |pw| {
                let res = hook(pw.controller(), page).map_err(|e| format!("webview hook: {e}"));
                let _ = ready_tx.send(res);
            })
            .map_err(|e| format!("scan window: {e}"))?;
        ready_rx.await.map_err(|_| "scan window closed early".to_string())??;
        self.log.line(format_args!("scan-{} open {host}", self.n));

        let t0 = tokio::time::Instant::now();
        let (mut next_click, mut page_alive) = (0, true);
        loop {
            let click = CLICK_AT.get(next_click);
            let at = t0 + click.map_or(GIVE_UP, |s| Duration::from_secs(*s));
            tokio::select! {
                _ = out.closed() => break,
                failed = &mut fail_rx, if page_alive => match failed {
                    Ok(reason) => {
                        self.log.line(format_args!("scan-{} failed {host}: {reason}", self.n));
                        return Err(reason);
                    }
                    Err(_) => page_alive = false,
                },
                _ = tokio::time::sleep_until(at) => {
                    if click.is_none() {
                        break;
                    }
                    next_click += 1;
                    let _ = win.0.with_webview(|pw| click_centre(&pw.controller()));
                }
            }
        }
        self.log.line(format_args!(
            "scan-{} close {host} after {} ms, popups denied so far {}",
            self.n,
            t0.elapsed().as_millis(),
            self.popups.load(Ordering::Relaxed)
        ));
        Ok(())
    }
}

/// State shared by the event handlers of one page (UI thread only).
struct Page {
    n: u64,
    url: String,
    out: UnboundedSender<Candidate>,
    log: Log,
    popups: Arc<AtomicU64>,
    seen: RefCell<HashSet<String>>,
    /// Hosts the top-level page may be on: the provider's and those it redirects to.
    hosts: RefCell<HashSet<String>>,
    ua: RefCell<String>,
    /// A top-level http(s) navigation is under way.
    loading: Cell<bool>,
    /// Reports that the provider page itself could not be loaded.
    fail: Cell<Option<oneshot::Sender<String>>>,
}

fn pw(f: impl FnOnce(*mut PWSTR) -> windows::core::Result<()>) -> String {
    let mut p = PWSTR::null();
    if f(&mut p).is_ok() { take_pwstr(p) } else { String::new() }
}

fn cut(s: &str) -> String {
    s.chars().take(160).collect()
}

fn kind_by_url(u: &str) -> Option<CandidateKind> {
    let lower = u.to_ascii_lowercase();
    let path = lower.split(['?', '#']).next().unwrap_or(&lower);
    if path.ends_with(".m3u8") || path.ends_with(".m3u") || lower.contains(".m3u8") {
        Some(CandidateKind::Hls)
    } else if path.ends_with(".mp4") {
        Some(CandidateKind::Mp4)
    } else if path.ends_with(".mpd") {
        Some(CandidateKind::Dash)
    } else {
        None
    }
}

fn kind_by_ctype(c: &str) -> Option<CandidateKind> {
    let c = c.to_ascii_lowercase();
    if c.contains("mpegurl") {
        Some(CandidateKind::Hls)
    } else if c.starts_with("video/mp4") {
        Some(CandidateKind::Mp4)
    } else if c.contains("dash+xml") {
        Some(CandidateKind::Dash)
    } else {
        None
    }
}

fn request_headers(req: &ICoreWebView2WebResourceRequest) -> Vec<(String, String)> {
    let mut out = Vec::new();
    // SAFETY: plain COM calls on a live request object on the UI thread; the returned strings
    // are owned by us and freed by `take_pwstr`.
    unsafe {
        let Ok(it) = req.Headers().and_then(|h| h.GetIterator()) else { return out };
        let mut has = BOOL(0);
        let _ = it.HasCurrentHeader(&mut has);
        while has.as_bool() {
            let (mut n, mut v) = (PWSTR::null(), PWSTR::null());
            if it.GetCurrentHeader(&mut n, &mut v).is_ok() {
                out.push((take_pwstr(n), take_pwstr(v)));
            }
            has = BOOL(0);
            if it.MoveNext(&mut has).is_err() {
                break;
            }
        }
    }
    out
}

impl Page {
    fn offer(&self, uri: String, kind: CandidateKind, req: &ICoreWebView2WebResourceRequest) {
        if !uri.starts_with("http") || !self.seen.borrow_mut().insert(uri.clone()) {
            return;
        }
        let mut headers = request_headers(req);
        if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("user-agent")) {
            headers.push(("User-Agent".into(), self.ua.borrow().clone()));
        }
        self.log.line(format_args!("scan-{} candidate {kind:?} {}", self.n, host_of(&uri)));
        let _ = self.out.send(Candidate { url: uri, kind, headers });
    }
}

fn hook(controller: ICoreWebView2Controller, page: Page) -> windows::core::Result<()> {
    // SAFETY: called on the UI thread that owns the controller; all interfaces used are obtained
    // from it right here and the handlers only touch `Rc<Page>` on that same thread.
    unsafe {
        let core = controller.CoreWebView2()?;
        // The window stays hidden, but the page must believe it is visible or players do not start.
        let _ = controller.SetIsVisible(true);
        if let Ok(c8) = core.cast::<ICoreWebView2_8>() {
            let _ = c8.SetIsMuted(true);
        }
        let settings = core.Settings()?;
        let _ = settings.SetAreDefaultScriptDialogsEnabled(false);
        if let Ok(s2) = settings.cast::<ICoreWebView2Settings2>() {
            *page.ua.borrow_mut() = pw(|p| s2.UserAgent(p));
        }
        let page = Rc::new(page);
        let mut token = Default::default();

        // --- requests of all frames and workers ---------------------------------------------
        core.cast::<ICoreWebView2_22>()?.AddWebResourceRequestedFilterWithRequestSourceKinds(
            &HSTRING::from("*"),
            COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
            COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL,
        )?;
        let p = page.clone();
        core.add_WebResourceRequested(
            &WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let req = args.Request()?;
                let uri = pw(|s| req.Uri(s));
                if let Some(kind) = kind_by_url(&uri) {
                    p.offer(uri, kind, &req);
                }
                Ok(())
            })),
            &mut token,
        )?;

        // --- responses: playlists behind URLs that do not look like one ---------------------
        let p = page.clone();
        core.cast::<ICoreWebView2_2>()?.add_WebResourceResponseReceived(
            &WebResourceResponseReceivedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let (req, resp) = (args.Request()?, args.Response()?);
                let mut status = 0;
                let _ = resp.StatusCode(&mut status);
                let ctype =
                    resp.Headers().map(|h| pw(|s| h.GetHeader(&HSTRING::from("Content-Type"), s))).unwrap_or_default();
                if let (200 | 206, Some(kind)) = (status, kind_by_ctype(&ctype)) {
                    p.offer(pw(|s| req.Uri(s)), kind, &req);
                }
                Ok(())
            })),
            &mut token,
        )?;

        // --- popups: never ------------------------------------------------------------------
        let p = page.clone();
        core.add_NewWindowRequested(
            &NewWindowRequestedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                args.SetHandled(true)?;
                let total = p.popups.fetch_add(1, Ordering::Relaxed) + 1;
                p.log.line(format_args!(
                    "scan-{} popup denied (NewWindowRequested #{total}) {}",
                    p.n,
                    host_of(&pw(|s| args.Uri(s)))
                ));
                Ok(())
            })),
            &mut token,
        )?;

        // --- top-level navigation stays on the provider -------------------------------------
        let p = page.clone();
        core.add_NavigationStarting(
            &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let uri = pw(|s| args.Uri(s));
                if uri.starts_with("about:") {
                    return Ok(());
                }
                let host = host_of(&uri);
                let mut redirected = BOOL(0);
                let _ = args.IsRedirected(&mut redirected);
                let known = p.hosts.borrow().contains(&host);
                if uri.starts_with("http") && (known || redirected.as_bool()) {
                    // server redirects of an allowed navigation may move to another host
                    p.hosts.borrow_mut().insert(host);
                    p.loading.set(true);
                } else {
                    args.SetCancel(true)?;
                    p.log.line(format_args!("scan-{} navigation blocked {}", p.n, cut(&uri)));
                }
                Ok(())
            })),
            &mut token,
        )?;

        // --- a provider that is offline should not cost the full waiting time ---------------
        let p = page.clone();
        core.add_NavigationCompleted(
            &NavigationCompletedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                if !p.loading.replace(false) {
                    return Ok(());
                }
                let mut ok = BOOL(0);
                let _ = args.IsSuccess(&mut ok);
                if ok.as_bool() {
                    p.fail.take(); // loaded once: later failures are the page's own business
                    return Ok(());
                }
                let mut status = COREWEBVIEW2_WEB_ERROR_STATUS(0);
                let _ = args.WebErrorStatus(&mut status);
                // Only plain network failures end the scan early. Anything else (HTTP error
                // pages, bot checks that reload, our own cancelled navigations) keeps waiting.
                let reason = match status {
                    COREWEBVIEW2_WEB_ERROR_STATUS_HOST_NAME_NOT_RESOLVED => "provider host not found",
                    COREWEBVIEW2_WEB_ERROR_STATUS_TIMEOUT => "provider page timed out",
                    COREWEBVIEW2_WEB_ERROR_STATUS_SERVER_UNREACHABLE
                    | COREWEBVIEW2_WEB_ERROR_STATUS_CANNOT_CONNECT
                    | COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_ABORTED
                    | COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_RESET
                    | COREWEBVIEW2_WEB_ERROR_STATUS_DISCONNECTED => "provider page not reachable",
                    _ => return Ok(()),
                };
                if let Some(tx) = p.fail.take() {
                    let _ = tx.send(reason.to_string());
                }
                Ok(())
            })),
            &mut token,
        )?;

        // --- no downloads, no permission prompts --------------------------------------------
        let p = page.clone();
        core.cast::<ICoreWebView2_4>()?.add_DownloadStarting(
            &DownloadStartingEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args {
                    args.SetCancel(true)?;
                    args.SetHandled(true)?;
                    p.log.line(format_args!("scan-{} download cancelled", p.n));
                }
                Ok(())
            })),
            &mut token,
        )?;
        core.add_PermissionRequested(
            &PermissionRequestedEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args {
                    args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)?;
                }
                Ok(())
            })),
            &mut token,
        )?;

        // --- go ----------------------------------------------------------------------------
        let (core2, target) = (core.clone(), HSTRING::from(page.url.as_str()));
        core.AddScriptToExecuteOnDocumentCreated(
            &HSTRING::from(PLAY_SCRIPT),
            &AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(move |_, _| {
                core2.Navigate(&target)
            })),
        )?;
    }
    Ok(())
}

/// A real (trusted) mouse click in the middle of the page; reaches cross-origin iframes too.
fn click_centre(controller: &ICoreWebView2Controller) {
    let (x, y) = (SIZE.0 / 2.0, SIZE.1 / 2.0);
    // SAFETY: UI thread, live controller (see `hook`).
    unsafe {
        let Ok(core) = controller.CoreWebView2() else { return };
        for (kind, button) in [("mouseMoved", "none"), ("mousePressed", "left"), ("mouseReleased", "left")] {
            let params =
                format!(r#"{{"type":"{kind}","x":{x},"y":{y},"button":"{button}","clickCount":1}}"#);
            let _ = core.CallDevToolsProtocolMethod(
                &HSTRING::from("Input.dispatchMouseEvent"),
                &HSTRING::from(params),
                &CallDevToolsProtocolMethodCompletedHandler::create(Box::new(|_, _| Ok(()))),
            );
        }
    }
}
