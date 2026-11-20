//! Interface between the server and whatever can watch a provider page load (the desktop app's
//! hidden WebView2 windows). The standalone server has no implementation.

use std::{future::Future, pin::Pin};

use tokio::sync::mpsc::UnboundedSender;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateKind {
    /// `.m3u8` URL or `application/vnd.apple.mpegurl` response.
    Hls,
    /// `.mp4` URL or `video/mp4` response.
    Mp4,
    /// `.mpd` URL or `application/dash+xml` response. Reported so the scan can say why a
    /// provider has no usable stream; the proxy only handles HLS and plain files.
    Dash,
}

/// A media request seen in any frame of the provider page.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub url: String,
    pub kind: CandidateKind,
    /// Request headers as sent by the page (Referer, Origin, User-Agent, ...).
    pub headers: Vec<(String, String)>,
}

pub type SniffFuture = Pin<Box<dyn Future<Output = Result<(), String>> + Send>>;

pub trait Sniffer: Send + Sync + 'static {
    /// Loads `url` as a top-level page and reports every media candidate through `out`.
    /// The future ends when the receiver of `out` is dropped (the server found what it needs) or
    /// when the sniffer gives up; either way it has released its page by then.
    /// `Err` means the page could not be loaded at all.
    fn sniff(&self, url: String, out: UnboundedSender<Candidate>) -> SniffFuture;
}
