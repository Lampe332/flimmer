# Aufräumen vor dem Teilen (21.11.2026)

Ziel: weniger toter Code, kleinere Downloads, gleiches Verhalten.

## Entfernt / vereinfacht

| Bereich | Zeilen | Was |
|---|---|---|
| Web | +75 / −231 | 4 ungenutzte i18n-Schlüssel (`set.section`, `set.open`, `set.found`, `set.notFound`) aus allen 12 Wörterbüchern, 10 ungenutzte CSS-Regelgruppen (altes Einstellungs-Popup, `fact-ms`, `set-file`), `prefsLoaded()` gelöscht, 26 nur intern genutzte Exporte privat gemacht (knip) |
| Rust | +9 / −14 | `lock()`-Helfer war dreimal kopiert (guard, scan, stream), jetzt einmal in `lib.rs` |
| Sonst | +14 / −8 | `.gitignore` ergänzt (Logs, Schlüssel, `.conf`, `.pdb`), `spike/logs/build.log` aus Git genommen, `.claude/launch.json` auf `flimmer-server.exe`, `docs/API.md`: Titel, Sprachzeile und interne Route `/api/torrents/announce` |

Abhängigkeiten: `anyhow` (Rust, ungenutzt laut cargo-machete) raus, `windows`-Features von 3 auf 1. npm: nichts zu entfernen (knip meldet nur `sw.js` und `@tauri-apps/cli`, beide werden zur Laufzeit bzw. von `build-installer.ps1` gebraucht).

## Performance

- **Sprachen werden nachgeladen:** vorher steckten alle 12 Wörterbücher im Start-Bundle, jetzt lädt nur die gewählte Sprache (eigener Chunk, ca. 36–55 kB). Ein Sprachwechsel lädt die neue Sprache und schaltet dann um.
- **Release-Profil:** `lto = "fat"`, `codegen-units = 1`, `strip = true`, `panic` bleibt `unwind`.
- hls.js (578 kB) bleibt nur auf den Player-Seiten, nicht im Start-Bundle.
- Start ohne blockierende Netzwerkaufrufe: `bind()` liest nur Dateien und SQLite und öffnet Loopback-Listener; VPN, Tracker-Liste und Auto-Download starten erst bei Bedarf bzw. im Hintergrund.

| Messung | vorher | nachher |
|---|---|---|
| JS beim ersten Laden (roh) | 836,5 KiB | 402,6 KiB |
| JS beim ersten Laden (gzip) | 271,9 KiB | 129,6 KiB |
| CSS | 85,7 kB | 83,9 kB |
| `Flimmer.exe` | 28,57 MB | 24,81 MB (−13 %) |
| `flimmer-server.exe` | 20,89 MB | 17,72 MB (−15 %) |
| Kaltstart Server (bis „running“, Median aus 9) | 271 ms | 277 ms (gleich, Rauschen) |
| Kaltstart Desktop (bis „server at“, Median aus 3) | 619 ms | 604 ms |
| Release-Build (`-j 4`, von null) | 3 min 36 s | 4 min 22 s |

## Tests

- `cargo test --workspace`: 88 grün (85 Bibliothek, 1 `tests/security.rs`, 2 Desktop).
- `cargo clippy --release --all-targets`: keine Warnung.
- `npm run build`: Exit 0, alle 12 Wörterbücher mit 756 Schlüsseln.
- Rauchtest (Scratch-Instanz, Port 43311): Start, Suche, Titelseite, Torrent-Suche, Sintel per Torrent abgespielt, Einstellungen mit Sprachwechsel, Einrichtung mit frischem `--data`.

## Bewusst gelassen

- `VideoPlayer` rendert bei jedem `timeupdate` (4×/s) neu; das Aufteilen der 1300-Zeilen-Komponente wäre ein Umbau mit Risiko.
- clippy `pedantic`: übrig sind Stilfragen (Casts, Doku-Backticks, `map_or`); umgestellt hätte es nur Diff-Rauschen erzeugt.
- Die ca. 270 ms Serverstart stecken in `bind()` (Torrent-Engine mit eigener Runtime); nicht angefasst.
- `spike/` bleibt im Repo, ist aber weder im Workspace noch im Installer.
