# Flimmer für Android

Stand 21.11.2026. Getestet auf einem Samsung S22 Ultra (Android 16, WebView Chrome 157).
Die App läuft ganz ohne PC: Server, Quellensuche, Torrents und Player stecken alle in der APK.

## Aufbau

- **Crate `android/`** (Tauri v2, Paket `local.flimmer.app`). Auf anderen Zielen ist die Crate leer, der Desktop baut wie bisher.
- **Server im Prozess.** Derselbe `flimmer`-Server wie am Desktop, gebunden an `127.0.0.1` auf einem freien Port. Der Port wird in `data/port` gemerkt, damit die Seite ihre Adresse und ihren Browser-Speicher behält. Die Oberfläche lädt die App aus diesem Server.
- **Oberfläche und `providers.json`** sind in die Bibliothek eingebaut und werden beim ersten Start eines Builds nach `<App-Daten>/root` entpackt.
- **Daten:** privat unter `/data/user/0/local.flimmer.app/data` (Datenbank, VPN-Datei, Cache, Werbefilter). Downloads landen in `Android/data/local.flimmer.app/files/Movies/Filme` und `/Serien`.
- **Quellensuche:** `android/src/sniffer.rs` steuert, die Kotlin-Klasse `SnifferPlugin` beobachtet. Pro Anbieter ein Android-WebView hinter dem Hauptfenster. `shouldInterceptRequest` sieht jede Anfrage aller Frames und meldet `.m3u8`, `.mp4` und `.mpd` samt Kopfzeilen an den Server. Popups, fremde Seitenwechsel, Downloads, Dialoge und Berechtigungen werden abgelehnt, Ton ist aus, nach 7, 10 und 13 Sekunden kommt ein echter Tipper in die Mitte.
- **Werbeblocker:** Braves `adblock`-Crate in Rust (`android/src/adblock.rs`), per JNI aus Kotlin gefragt. Die Listen (EasyList, uBlock-Filter) lädt die App selbst nach `data/adblock` und erneuert sie nach einer Woche. Ohne Listen läuft die Suche ohne Blocker.
- **Player:** der eigene Player mit hls.js im WebView. Vollbild dreht ins Querformat, Zurück verlässt zuerst das Vollbild. Am Handy bleibt die App sonst hochkant, Tablets drehen frei.
- **Torrents:** librqbit hinter dem WireGuard-Tunnel im Prozess, wie am Desktop. Die VPN-Datei kommt über die Android-Dateiauswahl. „Ohne VPN“ ist aus, bis man es einschaltet.
- **Server-Haken:** `server/src/host.rs`. Die App setzt dort den Download-Ordner, die Obergrenze von 4 Seiten gleichzeitig, das Öffnen anderer Apps und das Leeren des Browser-Caches. Desktop und Server-Exe setzen nichts und verhalten sich wie vorher.

## Was anders ist als am Desktop

| Thema | Desktop | Android |
|---|---|---|
| Quellensuche | WebView2, uBlock Origin Lite, 4 bis 12 Seiten | Android-WebView, `adblock`-Crate, fest 4 Seiten |
| Erkennung | Adresse und Antworttyp | nur die Adresse (Android zeigt keine Antworten) |
| Torrent-Dateien | ffprobe und ffmpeg wandeln alles um | kein ffmpeg. Codecs kommen aus dem Dateikopf (`server/src/container.rs`) |
| Tonspur wählen, Untertitel aus der Datei | ja | nein, es spielt die erste Tonspur |
| „Ordner öffnen“ | Explorer | ausgeblendet |
| „Cache leeren“ | Profil der Quellensuche, beim nächsten Start | Cookies und HTTP-Cache aller WebViews, sofort. Sprache und Einstellungen der Oberfläche bleiben |
| Bauen | `build-installer.ps1` | `android\build-apk.ps1` |

Neu in der API: `Caps.external_player`, `Caps.open_folder`, `Caps.parallel_max`, `PlayInfo.mode = "external"`, `POST /api/torrents/:id/external` (siehe `docs/API.md`).

## Was das Handy abspielt (gemessen)

| Format | Ergebnis |
|---|---|
| MP4 mit H.264 und AAC | spielt in der App (Sintel 720p aus dem Torrent, durch den Tunnel) |
| MKV mit H.264 und AAC | spielt in der App (Sintel 1080p WEB) |
| HEVC 8 und 10 Bit, AV1, VP9, Opus, FLAC | laut WebView abspielbar, HEVC mit Hardware |
| AC3, E-AC3, DTS | nicht abspielbar |

Entscheidung: kein ffmpeg in der APK. Kann der eingebaute Player eine Datei nicht (Ton in AC3, E-AC3 oder DTS, andere Container), zeigt die App „In externem Player öffnen“ und reicht den lokalen Stream an eine Player-App weiter. Auf dem Testgerät würden das der Samsung-Videoplayer und Google Fotos annehmen, VLC ist nicht installiert.

## Messwerte vom Testgerät

- APK 27,0 MB (nur arm64), Server bereit 130 ms nach dem Start.
- Quellensuche für einen Film: 15 von 41 Anbietern mit Stream in rund 95 Sekunden, 11 Popups und 2 fremde Seitenwechsel abgelehnt. 5 Anbieter-Adressen ließen sich in dem Lauf nicht auflösen, Ursache offen.
- Torrent durch den Tunnel: Ausgang CH, DHT 83 Knoten, Portweiterleitung an, bis 19 MB/s.

## Grenzen

- **Downloads laufen nur, solange die App offen ist.** Zurück auf der Startseite beendet die App und damit den Server. Ein Dienst für den Hintergrund fehlt.
- **Download-Ordner** müssen echte Pfade sein, in die die App schreiben darf. Ordner aus der Android-Ordnerauswahl gehen nicht, weil die Torrent-Bibliothek Dateipfade braucht.
- **Der Player lädt Streams voraus** in den App-Cache (im Test 256 MB in 40 Sekunden bei 4K). Beim Verlassen des Players wird das wieder gelöscht.
- **„Weiterschauen“** zeigt Filme erst ab 30 Sekunden, wie am Desktop. Torrents aus der freien Suche haben keinen Titel und merken sich keine Stelle.
- **Nicht am Gerät ausgelöst:** das Weiterreichen an den externen Player und die Karte dafür. Zwei Sintel-Torrents mit AC3 und DTS fanden keine Gegenstellen. Die Codec-Erkennung selbst ist durch Tests gedeckt.
- **Dateiauswahl:** Android blendet Dateien mit unbekanntem Typ aus, wenn die Seite einen Filter setzt. Darum hat die Auswahl auf Android keinen Filter.
- **Ton der Anbieter-Seiten** schaltet ein Skript in jeder Seite stumm. Ob dabei wirklich nie etwas zu hören ist, wurde nicht gemessen.
- **Nur arm64.** Für ältere 32-Bit-Geräte und Emulatoren gibt es keine APK.

## Bauen und testen

- `android\build-apk.ps1` baut die APK nach `android\gen\android\app\build\outputs\apk\arm64\release\`. Mit `-Inspect` entsteht ein Testbuild, dessen WebViews sich über adb untersuchen lassen und dessen Log unter `Android/data/local.flimmer.app/files/logs` liegt.
- `tauri android build` geht auf diesem Rechner nicht: es will einen symbolischen Link anlegen, das erlaubt Windows nur im Entwicklermodus. Das Skript kopiert die Bibliothek stattdessen.
- Signiert wird mit `android\gen\android\keystore.properties` und dem Schlüssel unter `android\keys\`. Beides liegt nur lokal und steht in `.gitignore`. Ohne die Datei bleibt die APK unsigniert. Wer den Schlüssel verliert, kann die installierte App nicht mehr aktualisieren, nur neu installieren.
- `android\scripts\cdp.mjs` steuert einen Testbuild am Gerät (`eval`, `tap`, `fill`).
- Prüfen: `cargo clippy -p flimmer -p flimmer-android --target aarch64-linux-android` (braucht die Umgebungsvariablen, die `build-apk.ps1` setzt), `cargo test -p flimmer`, `npm run build` in `web\`.
- `cargo check -p flimmer-desktop` braucht `tools\ffmpeg` und `desktop\extensions\ubol`. In einem frischen Arbeitsbaum fehlen beide. Dann hilft `TAURI_CONFIG={"bundle":{"resources":null}}` für die reine Prüfung.
