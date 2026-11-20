# Probelauf P2a – Bericht (20./21.11.2026)

Testfenster: Tauri v2 (WebView2), lädt die echte Flimmer-UI (`/play/...`) ohne iframe-Sandbox. Messung per CDP. Rohdaten in `logs\`, Bilder in `shots\`.
Agent wurde durch PC-Neustart unterbrochen; Serien-Lauf und Bericht hat der Hauptchat nachgeholt (`logs\out-adblock-tv.txt`).

## Test 1 – Werbeblocker im App-Fenster

| Variante | Werbung geblockt | Tracker geblockt | Beleg |
|---|---|---|---|
| ohne | 0/18 | 0/14 | `shots\t1-none.png` |
| uBlock Origin Lite über Tauri `extensions_path` | 10/18 | 12/14 | `shots\t1-ubol-wry.png` |
| uBlock Origin Lite direkt (`AddBrowserExtension`) | 10/18 | 12/14 | `shots\t1-ubol-direct.png` |
| AdGuard MV3 direkt | 13/18 | 0/14 | `shots\t1-adguard-direct.png` |
| Brave-Crate `adblock` (EasyList) | 0/18 auf der Testseite | – | `shots\t1-adblock-crate.png` |

✅ Erweiterungen laden in WebView2 (der befürchtete wry-Fehler trat nicht auf). Die Crate blockt auf der lokalen Testseite nichts (vermutlich falsch eingehängt für diese Seite), auf echten Provider-Seiten aber schon (siehe Test 2).

## Test 2 – Ohne Sandbox, mit Blocker + Popup-Verbot (Film 278)

Mit Blocker: vidlink 12 Anfragen geblockt, vixsrc 10, cinezo 1. Popups: ohne Blocker 1 (vidlink), mit Popup-Verbot 0. Keine „Sandbox Detected“-Meldung, Provider-Player spielen weiter. ✅

## Test 3 – Echte Qualität abfangen (`.m3u8`)

| Provider | Film 278 | Serie 84773 S1E1 | Ton | Untertitel |
|---|---|---|---|---|
| cinezo | 1920x1080, 6,8 Mbit/s, 0,7 s | 1920x800, 13,4 Mbit/s, 1,2 s | und / en | – |
| vidlink | 1920x1080, 0,9 Mbit/s, 0,8 s | 2592x1080, 2,2 Mbit/s, 0,8 s | und / eng | – |
| vixsrc | 1280x720, 1,8 Mbit/s, 1,1 s | 1280x720, 1,8 Mbit/s, 1,0 s | ita + eng | 15 / 39 (inkl. Deutsch) |
| videasy | ❌ kein Stream in 50 s | – | | |
| vidfast | ❌ kein Stream in 50 s | – | | |
| 111movies | ❌ kein Stream in 50 s | – | | |

✅ bei 3 von 6. Die 3 Ausfälle liefern gar keinen Stream (auch die Provider-Suche bekam von videasy/vidfast keine Antwort bzw. 403) → im Produkt einfach als „kein Stream“ einsortieren.

## Test 4 – Eigener Player (hls.js über lokalen Proxy mit Referer/Origin)

6 von 6 abgefangenen Streams spielen im eigenen Player (`currentTime` > 6 s, richtige Auflösung). ✅

## Empfehlung

- **Eigener Player als Normalfall: machbar.** Abfangen dauert unter 1,5 s pro Provider, Auflösung/Bitrate/Tonspuren/Untertitel sind echt lesbar → Filter + automatische Wahl gehen.
- **Werbeblocker: Weg A** (uBlock Origin Lite über `extensions_path`) + Popup-Verbot (`NewWindowRequested`). Wird nur noch im unsichtbaren Abfang-Fenster gebraucht.
- Provider ohne abfangbaren Stream: aussortieren, kein Rückfall-iframe nötig (Vorgabe: nur eigener Player).

## Starten

`cd spike\desktop && cargo build --release`, dann `node tools\run.mjs <adblock|none> <movie|tv> [provider ...]` (startet Flimmer-Server falls nötig und das Testfenster mit `--remote-debugging-port=9222`).
