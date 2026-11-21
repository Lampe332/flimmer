fn main() {
    // Seconds since 1970 at build time: the app unpacks its bundled web UI again when this changes.
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    println!("cargo:rustc-env=FLIMMER_BUILD_STAMP={stamp}");
    println!("cargo:rerun-if-changed=../web/dist");
    println!("cargo:rerun-if-changed=../providers.json");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        tauri_build::build()
    }
}
