//! Ad blocker for the provider pages: Brave's `adblock` engine with EasyList and uBlock Origin's own filters.
//! (The desktop app loads uBlock Origin Lite as a browser extension; Android's WebView has no extensions.)
//! The Kotlin sniffer asks [`Java_local_flimmer_app_Adblock_blocked`] for every sub-request of a provider page.
//!
//! The lists are downloaded into `<data>/adblock` and refreshed after a week. Until the first download is done
//! (and whenever it fails without an older copy) the source search runs without a blocker.

use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{Duration, SystemTime},
};

use adblock::{Engine, lists::{FilterSet, ParseOptions}, request::Request};
use jni::{
    JNIEnv,
    objects::{JClass, JString},
    sys::jboolean,
};

use crate::Log;

const LISTS: [(&str, &str); 2] = [
    ("easylist.txt", "https://easylist.to/easylist/easylist.txt"),
    ("ublock-filters.txt", "https://raw.githubusercontent.com/uBlockOrigin/uAssets/master/filters/filters.txt"),
];
const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// A list is at most this big (EasyList has about 2 MB).
const MAX_BYTES: usize = 16 * 1024 * 1024;

static ENGINE: OnceLock<Engine> = OnceLock::new();

fn fresh(path: &Path) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .is_ok_and(|t| SystemTime::now().duration_since(t).is_ok_and(|age| age < MAX_AGE))
}

async fn download(url: &str) -> Result<String, String> {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(30)).build().map_err(|e| e.to_string())?;
    let resp = client.get(url).send().await.and_then(|r| r.error_for_status()).map_err(|e| e.to_string())?;
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if text.len() > MAX_BYTES || !text.contains("||") {
        return Err("not a filter list".into());
    }
    Ok(text)
}

/// Loads the lists (downloading stale or missing ones) and builds the engine, off the start-up path.
pub fn spawn(dir: PathBuf, log: Log) {
    tauri::async_runtime::spawn(async move {
        let _ = std::fs::create_dir_all(&dir);
        let mut set = FilterSet::new(false);
        let mut rules = 0;
        for (name, url) in LISTS {
            let path = dir.join(name);
            if !fresh(&path) {
                match download(url).await {
                    Ok(text) => {
                        let _ = std::fs::write(&path, text);
                    }
                    Err(e) => log.line(format_args!("adblock: {name} not downloaded ({e})")),
                }
            }
            if let Ok(text) = std::fs::read_to_string(&path) {
                rules += text.lines().count();
                set.add_filter_list(text, ParseOptions::default());
            }
        }
        if rules == 0 {
            log.line("adblock: no filter lists, the source search runs without ad blocker");
            return;
        }
        let built = tauri::async_runtime::spawn_blocking(move || Engine::new_with_filter_set(set)).await;
        if let Ok(engine) = built {
            let _ = ENGINE.set(engine);
            log.line(format_args!("adblock: ready, {rules} list lines"));
        }
    });
}

fn blocked(url: &str, source: &str, kind: &str) -> bool {
    let Some(engine) = ENGINE.get() else { return false };
    Request::new(url, source, kind, "GET").is_ok_and(|r| engine.check_network_request(&r).should_block())
}

/// `local.flimmer.app.Adblock.blocked(url, source, kind)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_local_flimmer_app_Adblock_blocked<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    url: JString<'l>,
    source: JString<'l>,
    kind: JString<'l>,
) -> jboolean {
    let mut text = |s: &JString<'l>| env.get_string(s).map(String::from).unwrap_or_default();
    let (url, source, kind) = (text(&url), text(&source), text(&kind));
    // A panic must not cross into the Java caller.
    let hit = std::panic::catch_unwind(|| blocked(&url, &source, &kind)).unwrap_or(false);
    jboolean::from(hit)
}
