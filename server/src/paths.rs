//! Where Flimmer reads and writes. Two folders, set once at startup:
//! - `root`: read-only resources (`providers.json`, `web/dist`, `tools/`, `desktop/extensions`). Installed: the
//!   folder of the exe. Development: the repo checkout (`--root`).
//! - `home`: per-user data. Everything lives below `home/data` (SQLite, stream cache, WebView2 profiles, VPN file,
//!   logs, torrent state). Installed: `%LOCALAPPDATA%\Flimmer`. Development: same as `root`.

use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

struct Paths {
    root: PathBuf,
    home: PathBuf,
}

static PATHS: OnceLock<Paths> = OnceLock::new();

/// First call wins; the server binds once per process.
pub fn init(root: &Path, home: &Path) {
    let _ = PATHS.set(Paths { root: root.to_path_buf(), home: home.to_path_buf() });
}

fn get() -> &'static Paths {
    PATHS.get().expect("paths::init runs before the server starts")
}

/// Bundled helper programs (ffmpeg).
pub fn tools() -> PathBuf {
    get().root.join("tools")
}

/// `home/data`: the per-user data folder.
pub fn data() -> PathBuf {
    get().home.join("data")
}

/// The WireGuard file the setup imports. Only Flimmer reads it; it is never sent to the UI or logged.
pub fn vpn_file() -> PathBuf {
    data().join("vpn").join("proton.conf")
}

/// Bytes below `dir` (0 if it does not exist). Unreadable entries count as 0.
pub fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            Ok(_) => e.metadata().map(|m| m.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}

/// Removes everything inside `dir` that is not in use; returns the bytes that are left.
pub fn clear_dir(dir: &Path) -> u64 {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let path = e.path();
            let _ = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        }
    }
    dir_size(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_clears_a_folder() {
        let dir = std::env::temp_dir().join(format!("flimmer-paths-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("a").join("b")).unwrap();
        std::fs::write(dir.join("x.bin"), [0u8; 100]).unwrap();
        std::fs::write(dir.join("a").join("b").join("y.bin"), [0u8; 50]).unwrap();
        assert_eq!(dir_size(&dir), 150);
        assert_eq!(clear_dir(&dir), 0);
        assert!(dir.is_dir(), "the folder itself stays");
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(dir_size(&dir), 0);
    }
}
