//! Stream sniffer: one Android WebView per provider page (the Kotlin class `SnifferPlugin` owns them).
//!
//! Same rules as the desktop sniffer (`desktop/src/sniffer.rs`): the page loads top-level, muted, with popups,
//! foreign top navigations, downloads and prompts denied and ads blocked; every playlist / mp4 request of any
//! frame is reported with its request headers; the server decides which one is the stream and drops the channel.
//! Android shows no responses, so playlists behind URLs that do not look like one are not seen here.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use flimmer::sniff::{Candidate, CandidateKind, SniffFuture, Sniffer};
use serde::{Deserialize, Serialize};
use tauri::ipc::{Channel, InvokeResponseBody};
use tokio::sync::mpsc::{self, UnboundedSender};

use crate::{Bridge, Log};

/// Real taps on the player centre for pages that wait for one (seconds after load start).
const CLICK_AT: [u64; 3] = [7, 10, 13];
/// Pause between two page starts (docs/scan-speed.md).
const START_GAP: Duration = Duration::from_millis(300);
/// A page that showed no media request at all by now has no stream for us.
const GIVE_UP_QUIET: Duration = Duration::from_secs(15);
/// A page that did show one gets this long after the latest one for a usable stream to follow ...
const CANDIDATE_GRACE: Duration = Duration::from_secs(6);
/// ... but never longer than this in total.
const GIVE_UP_HARD: Duration = Duration::from_secs(25);

/// Time after which a page is given up; `last_candidate_ms` = when it last reported a candidate (0 = never).
fn give_up_after(last_candidate_ms: u64) -> Duration {
    let wait = match last_candidate_ms {
        0 => GIVE_UP_QUIET,
        ms => GIVE_UP_QUIET.max(Duration::from_millis(ms) + CANDIDATE_GRACE),
    };
    wait.min(GIVE_UP_HARD)
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

fn host_of(u: &str) -> String {
    url::Url::parse(u).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default()
}

/// What the Kotlin side tells about a page.
#[derive(Deserialize)]
#[serde(tag = "e", rename_all = "snake_case")]
enum Event {
    /// A media-looking request of any frame.
    Req { url: String, headers: HashMap<String, String> },
    /// The provider page itself could not be loaded.
    Fail { reason: String },
    Loaded,
    Popup,
    Note { text: String },
}

#[derive(Serialize)]
struct Open {
    id: u64,
    url: String,
    channel: Channel<serde_json::Value>,
}

#[derive(Serialize)]
struct PageId {
    id: u64,
}

pub struct WebviewSniffer {
    bridge: Bridge,
    log: Log,
    seq: AtomicU64,
    popups: Arc<AtomicU64>,
    /// Held while a page is being started, so that no two start within `START_GAP`.
    start: Arc<tokio::sync::Mutex<()>>,
}

impl WebviewSniffer {
    pub fn new(bridge: Bridge, log: Log) -> Self {
        Self { bridge, log, seq: AtomicU64::new(1), popups: Arc::new(AtomicU64::new(0)), start: Arc::default() }
    }
}

impl Sniffer for WebviewSniffer {
    fn sniff(&self, url: String, out: UnboundedSender<Candidate>) -> SniffFuture {
        let job = Job {
            bridge: self.bridge.clone(),
            log: self.log.clone(),
            n: self.seq.fetch_add(1, Ordering::Relaxed),
            popups: self.popups.clone(),
            start: self.start.clone(),
        };
        Box::pin(job.run(url, out))
    }
}

struct Job {
    bridge: Bridge,
    log: Log,
    n: u64,
    popups: Arc<AtomicU64>,
    start: Arc<tokio::sync::Mutex<()>>,
}

/// Closes the page however the sniff ends (also when the task is dropped).
struct CloseOnDrop(Bridge, u64);

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        let (bridge, id) = (self.0.clone(), self.1);
        tauri::async_runtime::spawn(async move {
            let _ = bridge.run_mobile_plugin_async::<()>("close", PageId { id }).await;
        });
    }
}

impl Job {
    async fn run(self, url: String, out: UnboundedSender<Candidate>) -> Result<(), String> {
        let host = host_of(&url);
        if host.is_empty() {
            return Err("provider URL has no host".into());
        }
        let n = self.n;
        let starting = self.start.clone().lock_owned().await;
        let (tx, mut events) = mpsc::unbounded_channel::<Event>();
        let channel = Channel::new(move |body: InvokeResponseBody| {
            if let Ok(event) = body.deserialize::<Event>() {
                let _ = tx.send(event);
            }
            Ok(())
        });
        let _page = CloseOnDrop(self.bridge.clone(), n);
        self.bridge
            .run_mobile_plugin_async::<()>("open", Open { id: n, url, channel })
            .await
            .map_err(|e| format!("scan page: {e}"))?;
        let t0 = tokio::time::Instant::now();
        self.log.line(format_args!("scan-{n} open {host}"));
        tokio::time::sleep(START_GAP).await;
        drop(starting);

        let (mut next_click, mut last_candidate, mut loaded) = (0, 0u64, false);
        loop {
            let give_up = t0 + give_up_after(last_candidate);
            let click = CLICK_AT.get(next_click).map(|s| t0 + Duration::from_secs(*s)).filter(|c| *c < give_up);
            tokio::select! {
                _ = out.closed() => break,
                event = events.recv() => match event {
                    Some(Event::Req { url, headers }) => {
                        if let Some(kind) = kind_by_url(&url) {
                            last_candidate = (t0.elapsed().as_millis() as u64).max(1);
                            self.log.line(format_args!("scan-{n} candidate {kind:?} {}", host_of(&url)));
                            let _ = out.send(Candidate { url, kind, headers: headers.into_iter().collect() });
                        }
                    }
                    // Loaded once: later failures are the page's own business.
                    Some(Event::Fail { reason }) if !loaded => {
                        self.log.line(format_args!("scan-{n} failed {host}: {reason}"));
                        return Err(reason);
                    }
                    Some(Event::Fail { .. }) => {}
                    Some(Event::Loaded) => loaded = true,
                    Some(Event::Popup) => {
                        let total = self.popups.fetch_add(1, Ordering::Relaxed) + 1;
                        self.log.line(format_args!("scan-{n} popup denied (#{total})"));
                    }
                    Some(Event::Note { text }) => self.log.line(format_args!("scan-{n} {text}")),
                    None => break,
                },
                _ = tokio::time::sleep_until(click.unwrap_or(give_up)) => {
                    if click.is_some() {
                        next_click += 1;
                        let _ = self.bridge.run_mobile_plugin_async::<()>("click", PageId { id: n }).await;
                    } else {
                        break;
                    }
                }
            }
        }
        self.log.line(format_args!(
            "scan-{n} close {host} after {} ms, popups denied so far {}",
            t0.elapsed().as_millis(),
            self.popups.load(Ordering::Relaxed)
        ));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn give_up_times() {
        assert_eq!(give_up_after(0), GIVE_UP_QUIET);
        assert_eq!(give_up_after(1_000), GIVE_UP_QUIET);
        assert_eq!(give_up_after(12_000), Duration::from_secs(18));
        assert_eq!(give_up_after(23_000), GIVE_UP_HARD);
    }
}
