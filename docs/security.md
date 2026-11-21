# Sicherheit

Flimmer ist ein lokaler Server (`127.0.0.1:<Port>`) mit Web-Oberfläche. Diese Seite sagt, wovor er sich schützt, wie, und was offen bleibt. Belege: `server/tests/security.rs` (139 Angriffsproben gegen einen echten Server) und die Unit-Tests in `server/src/guard.rs`.

Starten der Proben: `cargo test --release --test security -- --nocapture`

## Wer darf mit dem Server reden

| Rolle | Wie sie sich ausweist | Was sie darf |
|---|---|---|
| PC (Flimmer-Fenster) | Sitzungs-Cookie `flimmer_s_<Port>` (HttpOnly, SameSite=Strict). Das Fenster bekommt es beim Start über einen Einmal-Link `/api/auth/enter?t=…`. Das Geheimnis dahinter gilt für einen Programmlauf. | alles |
| Gerät (Handy, Tablet, Browser) | Geräte-Cookie `flimmer_d_<Port>` nach der Kopplung per PIN oder QR-Code. Auf dem PC liegt nur der SHA-256-Wert des Tokens. | schauen, suchen, merken, Torrents starten (Liste unten) |
| intern | ffmpeg liest Dateien mit einem Schlüssel pro Programmlauf (`?ik=`), nur über Loopback und nur `/api/torrents/:id/file/:n`. Die Torrent-Engine fragt den Tracker-Ersatz `/api/torrents/announce` ohne Browser-Kopfzeilen. | genau diese zwei Wege |
| niemand | kein gültiges Cookie | nur die App-Hülle, `/api/session`, `/api/pair` |

Alles andere unter `/api` antwortet `401 { code: "pairing_required" }`.

## Bedrohungen und Gegenmaßnahmen

| Bedrohung | Gegenmaßnahme |
|---|---|
| Fremde Webseite schickt Anfragen an `127.0.0.1` (CSRF) | Jede `/api`-Anfrage braucht das Cookie. Dazu: `Sec-Fetch-Site` muss `same-origin` oder `none` sein, `Origin` muss Flimmers eigene Adresse sein. Das gilt auch für andere Programme auf demselben PC mit eigenem Port, denn SameSite schaut nicht auf den Port. Kein CORS: Der Server schickt keine `Access-Control-*`-Kopfzeilen. |
| DNS-Rebinding (fremder Name zeigt auf den PC) | `Host` muss genau `127.0.0.1:<Port>` oder `localhost:<Port>` sein, im WLAN-Modus zusätzlich `<LAN-Adresse>:<Port>`. Sonst `403`. |
| SSRF über den Stream-Proxy (`/api/stream/:id/r?u=`) | Der Proxy holt nur Adressen, die der Server selbst in eine Playlist dieses Streams geschrieben hat: `&s=` ist ein HMAC über Stream-Kennung und Adresse. Zusätzlich für jede ausgehende Anfrage, auch für mitgeschnittene Adressen und jede Weiterleitung: nur `http(s)`, keine Zugangsdaten in der Adresse, keine privaten, Loopback-, Link-Local-, CGNAT- oder reservierten Adressen (IPv4, IPv6, eingebettete IPv4). Namen werden beim Verbinden geprüft, also greift auch ein Name, der auf `127.0.0.1` zeigt, ins Leere. Playlists höchstens 8 MB, ein Segment im Cache höchstens 512 MB, 10 s Verbindungs- und 30 s Lese-Timeout, höchstens 5 Weiterleitungen. |
| Schadseite über den Proxy in Flimmers Herkunft | Der Proxy reicht nur Medien-Typen durch, alles andere als `application/octet-stream`. Dazu `X-Content-Type-Options: nosniff` und eine Content-Security-Policy (`script-src 'self'`, Bilder nur von TMDB). |
| Path-Traversal | Pfade mit `..`, `\`, NUL oder doppelter Kodierung werden vor jeder Route mit `400` abgewiesen. Torrent-Dateien, Untertitel und Tonspuren werden über Nummern angesprochen, Streams über Hex-Kennungen. |
| Ordner öffnen, Dateien löschen | Nur innerhalb der Download-Ordner (aktuelle und frühere), nach Auflösen von Links. |
| Download-Ordner oder VPN-Datei auf fremdem Rechner (`\\server\freigabe`) | Nur Pfade auf lokalen Laufwerken. Windows würde sich sonst mit deinem Konto bei dem fremden Rechner anmelden. Gilt auch beim Import. |
| Export und Import | Höchstens 1 MB, Art und Version geprüft, erst prüfen, dann anwenden. Jeder Teil läuft durch die normale Prüfung. Bildadressen müssen von `image.tmdb.org` sein, Texte werden gekürzt, höchstens 5000 Merklisten-Einträge. Token und VPN-Datei stehen nie im Export. |
| Befehle einschleusen | ffmpeg, ffprobe und Explorer starten ohne Shell. Argumente bestehen aus Zahlen, festen Wörtern und der eigenen Loopback-Adresse. Text aus Anfragen landet nie in einem Argument. |
| SQL einschleusen | Alle Werte laufen über Parameter. Tabellennamen kommen aus einer festen Liste im Code. |
| Teure Endpunkte in einer Schleife | Kontingent pro Aufrufer: Scan, Torrent-Suche, Torrent-Start, ffmpeg, Vorladen, Auto-Download, Einrichtung, Kopplung. Danach `429`. |
| Geheimnisse | TMDB-Token und VPN-Schlüssel gehen nie an die Oberfläche und nie ins Log. Fehlertexte zitieren sie nicht. Der Start-Link steht nicht im Log. |
| Clickjacking, fremde Einbettung | `X-Frame-Options: DENY`, `frame-ancestors 'none'`, `Cross-Origin-Resource-Policy: same-origin`. |
| Tracker-Adressen aus Magnet-Links | Tracker mit privater Adresse oder `localhost` werden verworfen. |

## Handy & Tablet im WLAN

Standard: aus. Dann lauscht Flimmer nur auf `127.0.0.1`.

- **An:** Flimmer lauscht zusätzlich auf genau einer privaten Adresse des PCs (10.x, 172.16 bis 172.31, 192.168.x), nie auf `0.0.0.0`. Anfragen von öffentlichen Adressen werden abgewiesen. Ändert sich die Adresse (DHCP), zieht Flimmer innerhalb von 15 Sekunden um.
- **Koppeln:** Am PC „Gerät verbinden“ wählen. Das öffnet für 10 Minuten eine Kopplung mit sechsstelliger PIN und QR-Code. Der QR-Code enthält ein Einmal-Token im `#`-Teil der Adresse, der nie an einen Server oder in ein Log geht. PIN und Token gelten einmal.
- **Schutz gegen Raten:** 5 falsche PINs von einer Adresse sperren diese für 5 Minuten, danach länger (bis 60 Minuten). 15 falsche PINs insgesamt werfen die PIN weg. Chance pro Kopplung: höchstens 15 zu 1.000.000.
- **Geräteliste:** Name, zuletzt gesehen, Adresse. „Entfernen“ wirkt sofort (`401` bei der nächsten Anfrage). Das Gerät kann sich auch selbst trennen. Höchstens 20 Geräte.
- **Nur am PC:** Einrichtung, TMDB-Token, VPN-Datei, Download-Ordner, Torrents ohne VPN, Kill-Switch lösen, Auto-Download-Regeln, Export und Import, Cache, Zurücksetzen, Ordner öffnen, Dateien löschen, der WLAN-Modus selbst samt PIN und Geräteliste. Ein Gerät bekommt dafür `403 { code: "pc_only" }`.
- **Ein Gerät darf:** stöbern, Quellen suchen und abspielen, Merkliste, Verlauf und Fortschritt, Quellen- und Oberflächen-Einstellungen, Torrents suchen, starten, pausieren und aus der Liste nehmen, den Kill-Switch auslösen.
- **Windows-Firewall:** Flimmer ändert sie nicht. Beim ersten Einschalten fragt Windows, ob Flimmer im Netzwerk kommunizieren darf. Erlaube es für private Netzwerke. Der Installer legt keine Regel an, weil das Administratorrechte braucht. Von Hand: Windows-Sicherheit, Firewall, „Zugriff von App durch Firewall zulassen“, Flimmer, Haken bei „Privat“.
- **App aufs Handy:** Manifest (`/manifest.webmanifest`) und Service Worker (`/sw.js`) sind dabei. Der Service Worker hält nur die App-Hülle vor und fasst `/api` nie an. Er wird nur für gekoppelte Geräte angemeldet, nicht im Flimmer-Fenster.

## Was offen bleibt

- **Kein HTTPS im WLAN.** Wer im selben WLAN mitliest, sieht den Verkehr und kann das Geräte-Cookie abgreifen. Nur im eigenen Netz nutzen, nicht in Gäste- oder Firmen-WLANs. Ein selbst signiertes Zertifikat würde auf jedem Handy eine Warnung zeigen und wurde deshalb nicht eingebaut.
- **Installierbare App nur eingeschränkt.** Browser erlauben Service Worker und die echte Installation nur über HTTPS oder `localhost`. Geprüft in Chromium auf dem PC über `127.0.0.1`: Service Worker aktiv, Manifest ohne Fehler, keine Installationshindernisse. Über `http://192.168.…` auf dem Handy ist nur „Zum Startbildschirm hinzufügen“ zu erwarten, ohne Offline-Hülle. Das muss am Gerät geprüft werden.
- **Programme auf dem PC selbst.** Ein Programm mit deinen Rechten kann die Datenbank und die VPN-Datei direkt lesen. Davor schützt ein lokaler Server nicht.
- **System-Proxy.** Ist in Windows ein Proxy eingestellt, löst dieser die Namen auf. Die Adressprüfung des Stream-Proxys greift dann nur für Adressen in Zahlenform.
- **Tracker-Namen.** Tracker werden nur nach Schreibweise gefiltert, nicht nach aufgelöster Adresse. Ohne VPN (ausdrücklich erlaubter Modus) könnte ein Tracker-Name auf ein Gerät im Heimnetz zeigen und dort eine Anfrage ohne Antwort auslösen.
- **Wechselnde Adresse.** Bekommt der PC eine neue LAN-Adresse, müssen Geräte neu koppeln, weil Cookies an der Adresse hängen. Abhilfe: feste Adresse im Router.
- **Abhängigkeiten:** siehe unten.

## Abhängigkeiten

Geprüft am 21.11.2026 mit `cargo audit` und `npm audit --omit=dev`.

| Meldung | Stand |
|---|---|
| `npm audit --omit=dev` | 0 Meldungen |
| RUSTSEC-2026-0459, `libsqlite3-sys` 0.35 (SQLite) | behoben: `rusqlite` 0.43 mit `libsqlite3-sys` 0.40 |
| RUSTSEC-2026-0293, `ringbuf` 0.4.8 | offen. Kommt über `librqbit-utp` aus `librqbit` 9.1.0 (neueste Version). Der Fehler tritt nur auf, wenn ein Element beim Aufräumen abbricht, und Flimmer nutzt uTP nicht (nur TCP über den eigenen SOCKS-Server). Fällt mit dem nächsten `librqbit`-Update weg. |
| RUSTSEC-2026-0543, `chrono` (nicht mehr gepflegt) | Hinweis, kein Fehler. Kommt über `librqbit-dht`. |
| RUSTSEC-2024-0370 `proc-macro-error`, RUSTSEC-2024-0429 `glib` | nur im Linux-Zweig von Tauri (GTK). Nicht im Windows-Programm. |
