//! What only the surrounding app can do. The Android app sets this once before [`crate::bind`]; the desktop app
//! and the standalone server leave it unset and the server goes its own ways (ffmpeg, Explorer, the user's
//! video folder).

use std::{path::PathBuf, sync::OnceLock};

/// Opens a URL in another app. `mime` is `None` for a web page, else the media type of a stream for a player.
pub type Open = Box<dyn Fn(&str, Option<&str>) -> Result<(), String> + Send + Sync>;

pub struct Host {
    pub open: Open,
    /// Removes cookies and the HTTP cache of the pages the source search loads ("Cache leeren").
    pub clear_browser: Box<dyn Fn() -> Result<(), String> + Send + Sync>,
    /// Where that browser keeps its cache; its size is what the settings show next to "Cache leeren".
    pub browser_cache_dir: PathBuf,
    /// Folder the downloads go to unless the user names another one.
    pub media_dir: PathBuf,
    /// Most provider pages that load at the same time.
    pub max_parallel: usize,
}

static HOST: OnceLock<Host> = OnceLock::new();

/// First call wins.
pub fn set(host: Host) {
    let _ = HOST.set(host);
}

pub(crate) fn get() -> Option<&'static Host> {
    HOST.get()
}
