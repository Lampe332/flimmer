# Torrent P6 Backend: Messwerte

Stand 20.11.2026, Branch `p6`. Vorher = Build von 36f356a, nachher = dieser Stand. Testinstanz auf Port 43307, Tunnel über `data\vpn\proton.conf` (Proton, Schweiz). Nur freie Inhalte (Big Buck Bunny, Sintel, Debian- und Ubuntu-ISO). Wenige Läufe pro Fall, die Streuung zwischen Läufen ist groß.

## Kurzfassung

- Gut versorgte Torrents liefen schon vorher an der Leitungsgrenze (Spitze etwa 850 bis 875 Mbit/s in beiden Builds). Dort bringt P6 etwas mehr Durchschnittstempo und deutlich mehr Peers, aber kein Vielfaches.
- Neu und messbar: DHT durch den Tunnel, weitergeleiteter Port mit eingehenden Verbindungen, 150 statt 50 Peer-Plätze, Facetten und Filter, Tonspur- und Untertitelwahl, HEVC zu H.264 per AMD-Hardware, Warteschlange, Speicherplatz-Check, Auto-Download.
- Bei einem dünnen Schwarm (2 Seeder) wurde der Download nicht schneller. Die Seeder begrenzen dort das Tempo.

## Download vorher / nachher

| Fall | Build | Fertig nach | Schnitt | Peers max | Erstes Byte | 30 s gepuffert |
|---|---|---|---|---|---|---|
| Big Buck Bunny 1080p H.264, 263 MB | vorher (4 Läufe) | 5,7 bis 12,1 s | 182 bis 385 Mbit/s | 6 bis 13 | 0,6 bis 3,8 s | 1,2 bis 4,7 s |
| | nachher, 50 Plätze (4 Läufe) | 3,7 bis 11,1 s | 199 bis 593 Mbit/s | 8 bis 15 | 0,6 bis 7,9 s | 1,4 bis 8,7 s |
| | nachher, 150 Plätze (1 Lauf) | 3,7 s | 597 Mbit/s | 27 | unter 1 s | 2,0 s |
| Debian netinst, 756 MB | vorher (2 Läufe) | 11,0 und 13,1 s | 574 und 483 Mbit/s | 10 und 15 | 1,4 und 0,7 s | 2,7 und 2,6 s (erste 32 MB) |
| | nachher, 50 Plätze (1 Lauf) | 10,0 s | 633 Mbit/s | 11 | 1,0 s | 2,8 s |
| Ubuntu Desktop, 5,96 GB | nachher, 50 Plätze | 90,6 s | 552 Mbit/s | 18 | 2,9 s | |
| | nachher, 150 Plätze | 74,3 s | 673 Mbit/s | 45 | 1,7 s | 3,6 s |
| Big Buck Bunny 1080p AVI, 886 MB, 2 Seeder | vorher | 75,9 s | 98 Mbit/s | 2 | 2,0 s | 6,9 s |
| | nachher | 80,8 s | 92 Mbit/s | 3 | 2,6 s | 7,5 s |

Für die Ubuntu-ISO gibt es keinen Vorher-Lauf. Der Ausreißer 7,9 s beim ersten Nachher-Lauf (Metadaten) trat in den Folgeläufen nicht mehr auf (0,5 bis 0,6 s in beiden Builds).

## Peers nach Herkunft

| Lauf | Tracker gefunden | DHT gefunden | PEX bekannt | Verbunden in der Spitze |
|---|---|---|---|---|
| Debian | 455 | 540 | 13 | 11 (Tracker) |
| Ubuntu, 50 Plätze | 1000 | 993 | 44 | 12 DHT, 6 Tracker |
| Ubuntu, 150 Plätze | 1006 | 1305 | 169 | 39 DHT, 9 Tracker, 1 eingehend |
| dünner Schwarm | 8 | 11 | 0 | 2 Tracker, 1 DHT |

Vorher gab es nur Tracker (bekannte Adressen 401 bis 461 bei BBB und Debian, 8 beim dünnen Schwarm). Die öffentliche Trackerliste (ngosang, durch den Tunnel geholt, 1 Tag im Cache) erhöht die Ankündigungen auf bis zu 60 Tracker.

## Port-Weiterleitung und eingehende Peers

- NAT-PMP durch den Tunnel: Port 37325 erhalten, alle 45 s erneuert, blieb über Tunnel-Neustarts gleich.
- Vom Tunnel-Listener angenommene Verbindungen: 5 (BBB), 19 (dünner Schwarm), 70 und 44 (Ubuntu).
- In der Engine als Peer verbunden: 1 (Ubuntu, 150 Plätze). Mit 50 Plätzen kam kein eingehender Peer durch, weil ausgehende Versuche alle Plätze belegten. Das war der Grund für die Erhöhung auf 150.
- Gegenprobe: ein lokaler Handshake am Loopback-Listener der Engine wird beantwortet und als eingehender Peer gezählt.

## Leak-Test

Momentaufnahme bei 810 Mbit/s (Ubuntu, 19 Peers): 493 TCP-Sockets des Prozesses, alle Loopback oder lauschend, 0 externe TCP-Verbindungen. Lauschende Sockets nur auf 127.0.0.1 (HTTP, SOCKS, Engine). UDP: der WireGuard-Socket zum Proton-Endpunkt und ein zweiter, den die Engine beim Start immer anlegt (ihr UDP-Tracker-Client). Die Engine kennt nur den internen Shim-Tracker: die gespeicherte `.torrent`-Datei eines Laufs enthielt nur die Shim-Adresse, obwohl der Magnet drei echte Tracker nannte. Neu erzwingt ein Check das: eine gespeicherte Datei mit fremdem Tracker oder Web-Seed wird nicht an die Engine gegeben. Die Momentaufnahme stammt vom Build vor den letzten Änderungen (Seek, Untertitel, Vorschau), die das Netzwerk nicht berühren.

## Streaming

- Sprung auf 70 %, Zeit bis 2 MB lesbar: vorher 0,13 bis 1,01 s (BBB), 0,46 s (Debian), 1,81 s (dünner Schwarm). Nachher 0,12 bis 1,36 s (BBB), 0,69 s (Debian), 0,22 und 0,70 s (Ubuntu), 0,43 s (dünner Schwarm). Bei gut versorgten Torrents kein klarer Unterschied.
- `stream_health` lieferte beim BBB-Lauf bis 604,6 s gepuffert (ganze Datei).
- Anfang und Ende zuerst sowie das mitwachsende Lesefenster sind eingebaut und per Unit-Test geprüft. Ein eigener Nutzen ließ sich mit diesen schnellen Schwärmen nicht getrennt messen.

## Wiedergabe (Testdatei aus BBB: H.264, AC3 Deutsch, AAC Englisch, SRT Deutsch; zweite Datei HEVC 10 Bit)

- Spuren: beide Tonspuren und der Textuntertitel werden gelistet. `audio=0` liefert den Filmton, `audio=1` den Testton (440 Hz, anderer Pegel, 1 Kanal). Mit gespeicherter Sprache Englisch wird ohne Parameter Spur 1 gewählt.
- Untertitel als WebVTT mit Umlauten. Mit `t=10` läuft der erste Cue von 0 bis 1,867 s, der zweite beginnt bei 7,867 s (exakt 10 s verschoben).
- Seek im Kopiermodus: Wunsch 40 s, Stream beginnt am Keyframe 32,267 s. `/seek` liefert `start` 32,267 und `t` 32,407. Streamlänge danach 35,73 s, erwartet 35,73 s.
- HEVC zu H.264 mit `h264_amf`: 900 Frames 1080p in 2,3 bis 2,5 s, also 362 bis 395 fps inklusive HTTP. Mit `t=15` genau 450 Frames.

## Robustheit

- Neustart: Ubuntu bei 20 % hart beendet. Nach dem Start „checking", nach 13,1 s wieder „downloading", Fortschritt lief weiter (51 %, dann 67 %). Fertige Torrents blieben erhalten.
- Warteschlange: `max_active=1` ergibt 1 ladend und 2 wartend (Position 1 und 2). Priorität 50 zieht den dritten nach vorn. Bei 3 laden alle.
- Speicherplatz: mit 100000 GB Reserve Fehler „not enough disk space on D:: 0.7 GB needed, 369.3 GB free", nichts geladen, genau 1 Ereignis „failed" trotz Wiederholversuchen.
- Ereignisse: „grabbed", „finished", „failed" über `/api/torrents/events`.

## Auto-Download

- Regel für Sintel (TMDB 45745, mindestens 720p, höchstens 3 GB) plus Merkliste. Trockenlauf: 49 Treffer, Kandidat „Sintel.2010.1080p.WEB.H264-CLASSiCALHD" (584 MB, 105 Seeder). Echter Lauf: eingereiht, „metadata", „downloading", „completed" (550 MB), Verlaufseintrag, Ereignisse „grabbed" und „finished". Zweiter Lauf: nichts mehr.
- Ohne Merkliste: „skip: not on the watchlist".
- Serien (TMDB-Ausstrahlungsdaten) nur als Vorschau ohne Speichern: 3 laufende Serien, 26 fällige Folgen erkannt, 0 eingereiht.

## Facetten (`/api/torrents/movie/550`, nur Zähler)

494 Treffer. Auflösung: 1080 166, 720 88, 2160 35, SD 9, unbekannt 196. Quelle: BluRay 232, DVD 54, Remux 20, WEB-DL 16, HDTV 7, WEBRip 3, unbekannt 162. Codec: H.264 142, HEVC 83, XviD 45, AV1 8. HDR: SDR 474, HDR 9, HDR10 8, DV 5, HDR10+ 3. Sprache: ohne Angabe 358, en 77, ru 22, it 21, multi 21. Kein deutscher Treffer in diesen Quellen, „Deutsch + 1080p + BluRay" ergibt dort 0.
Gegenprobe mit der Freitextsuche „Fight Club German" (54 Treffer): „Deutsch + BluRay" Facette 2 = Treffer 2 = Handzählung 2. `language=de,multi&res=1080,2160` 6 = Handzählung 6.

## Vorfall beim Testen

Für den Serien-Trockenlauf hatte ich sechs echte Serienregeln in der Testinstanz gespeichert. Der Hintergrundjob (erster Lauf 2 Minuten nach Start) lief in diesem Moment und reihte 19 echte Folgen ein. Bis zum Abbruch wurden zusammen etwa 2,9 GB geladen. Alle 19 Torrents samt Dateien, Regeln und Verlauf sind gelöscht. Das war ein Verstoß gegen die Testregel. Folge: Es gibt jetzt `POST /api/autodl/preview`, das eine Regel prüft, ohne sie zu speichern oder etwas einzureihen.

## Offen

- Der Sprachfilter heißt `language`, nicht `lang`: `lang` ist schon die TMDB-Sprache jeder Anfrage.
- Staffel-Paket: Zuordnung, „nächste Folge" und lokale Suche sind an einem fertigen Testpaket und per Unit-Test geprüft. Das Nachwählen einer noch nicht geladenen Folge aus einem laufenden Paket ist nicht an einem echten Schwarm getestet (kein freies Paket mit Seedern gefunden).
- Bild-Untertitel (PGS, VobSub) gehen nicht als Text, die API meldet 415. Externe `.srt`-Dateien im Torrent werden nicht angeboten.
- Nach einem Neustart prüft die Engine vorhandene Daten neu (13 s bei etwa 3 GB).
- HEVC wird in Software dekodiert, nur das Kodieren läuft auf der Grafikkarte.
- Quelle 1337x ist weiterhin durch Cloudflare blockiert. Die 18 neuen Quellen und ihre Messung stehen in [torrent_quellen_kandidaten.md](torrent_quellen_kandidaten.md), Abschnitt 5.

## Nachtrag 21.11.: Oberfläche und Torrents ohne VPN

- Filterleiste mit Zählern aus den Facetten im Player und in der Torrent-Suche. Fight Club, „Deutsch + 1080p + BluRay“: Chips 3 / 3 / 3, Liste 3, API `matched` 3.
- Tonspur- und Untertitelmenü, Sprung über `/seek`: geprüft an einer lokal aus dem freien Sintel gebauten MKV (Englisch AAC, Deutsch AC3 als Testspur, Untertitel Rumänisch aus dem YTS-Paket). Spurwechsel bei 0:14 startete am Keyframe 0:10, Sprung auf 7:24 lud `play?t=444.5`, Untertitel mit `t=444.333` verschoben und sichtbar. Freie Torrents mit mehreren eingebetteten Tonspuren hatten keine Seeder (sieben Versuche).
- Warteschlange: `max_active=1` → 1 lädt, 2 warten; „nach oben“ zweimal machte den dritten zum aktiven; auf 3 gestellt → 3 laden, 0 warten.
- Torrents ohne VPN (nur nach Freigabe): ohne Freigabe und ohne VPN-Datei `409 needs_vpn`. Mit Freigabe lud Tears of Steel (frei) direkt, 33 externe TCP-Verbindungen des Prozesses, Schild „Ohne VPN“. Danach wieder mit VPN-Datei: nur Verbindungen zu Port 443 (die Suche über HTTPS, wie vorgesehen), keine Peer-Verbindung außerhalb des Tunnels.
- Offen: Untertitel aus Dateien in Windows-1250 kommen mit falschen Umlauten (der Server liest sie als UTF-8). Die Auto-Download-Vorschau für eine Serie mit 10 fälligen Folgen dauerte etwa 2 Minuten (eine Suche pro Folge).
