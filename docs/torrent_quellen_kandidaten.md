# Torrent-Quellen: Kandidaten für Flimmer

Stand 20.11.2026. Alles per PowerShell `Invoke-WebRequest` vom Entwicklungsrechner aus geprüft (ohne VPN, ohne Login, ohne Cookies). Es wurden nur Suchantworten gelesen, nichts geladen, keine Magnets geöffnet. Testanfragen: "sintel", `tt1727587` (Sintel), `tt0133093` (Matrix), `tt0082096` und "das boot" (Deutsch-Test), `tt0944947:1:1` (Serie), "frieren" (Anime).

Ergebnis: **19 Quellen verifiziert**, rund 40 Kandidaten verworfen. Die 20. Quelle gab es nicht ohne Cloudflare oder Login. Alle Tests sind Momentaufnahmen. Rate-Limits habe ich nicht gemessen, nur beobachtet (siehe btdig unten).

Grundsatz für den Einbau: Stremio-Addons brauchen die IMDb-ID, alle anderen Quellen eine Textsuche (Titel + Jahr, bei Serien `S01E01`). Fast alle Hosts sind Drittinstanzen, die verschwinden können. Also pro Quelle kurzes Timeout und stilles Überspringen.

## 1. Verifizierte Quellen

Kürzel Deutsch: **ja** = im Test mit "das boot" deutlich viele GERMAN/DL-Treffer, **teils** = wenige, **nein** = praktisch keine.

| # | Name | Basis-URL | Abfrage | Format | Felder | Test | Deutsch |
|---|------|-----------|---------|--------|--------|------|---------|
| 1 | BitSearch (API) | `https://bitsearch.to` | `GET /api/v1/search?q=<q>&sort=seeders&category=2&subCategory=2&page=<n>` (Cat 2 = Movies, 3 = TV, beobachtet) | JSON | infohash, title, size (Bytes), seeders, leechers, category, updatedAt | 200, 20 pro Seite, "das boot german": 18 von 20 DE | ja |
| 2 | Comet (feels.legal) | `https://comet.feels.legal` | `GET /stream/movie/<imdb>.json`, `/stream/series/<imdb>:<s>:<e>.json` | JSON (Stremio) | infoHash, Dateiname, Größe, Seeders, Quelle, Sprachflaggen | 200, Matrix 1117, GoT S1E1 768, Das Boot 299 (49 DE) | ja |
| 3 | Comet (midnightignite) | `https://cometfortheweebs.midnightignite.me` | wie 2 | JSON (Stremio) | wie 2 | 200, Matrix 990, GoT S1E1 929, Das Boot 391 (66 DE) | ja |
| 4 | TorrentsDB | `https://torrentsdb.com` | `GET /stream/movie/<imdb>.json`, `/stream/series/<imdb>:<s>:<e>.json` | JSON (Stremio) | infoHash, title (Name, Seeders, Größe, Quelle), Dateiname, Tracker | 200, Sintel 5, Matrix 91, GoT S1E1 43, Das Boot 52 (20 DE) | ja |
| 5 | MediaFusion (midnightignite) | `https://mediafusionfortheweebs.midnightignite.me` | `GET /stream/movie/<imdb>.json`, `/stream/series/...` | JSON (Stremio) | infoHash, Größe, Seeders, Sprachzeile, Quelle | 200, hart auf 25 Treffer gekappt (Matrix 25, Das Boot 25 mit 7 DE, Sintel 25) | teils |
| 6 | Peerflix | `https://peerflix.mov` | `GET /stream/movie/<imdb>.json`, `/stream/series/...` | JSON (Stremio) | infoHash, Seeders, Größe, Quelle, Landesflagge | 200, Matrix 19, Das Boot 4, Sintel 0, GoT S1E1 12 | nein (Spanisch/Englisch) |
| 7 | Torrents-CSV | `https://torrents-csv.com` | `GET /service/search?q=<q>&size=<n>&after=<next>` | JSON | infohash, name, size_bytes, seeders, leechers, completed, created_unix | 200, Sintel 3, "das boot german" 3 von 3 DE | ja |
| 8 | Zilean (DMM-Hashliste) | `https://zileanfortheweebs.midnightignite.me` | `GET /dmm/filtered?Query=<q>&Category=movie\|tvSeries&Season=<n>&Episode=<n>&Language=de` | JSON | info_hash, raw_title, size (String, Bytes), category, languages[], resolution, year, imdb_id | 200, Sintel 4, "das boot" 100, mit `Language=de` 23 | ja |
| 9 | TorrentDownloads (RSS) | `https://www.torrentdownloads.pro` | `GET /rss.xml?type=search&search=<q>` | RSS | title, category, size (Bytes), seeders, leechers, info_hash, pubDate | 200, 50 Items, "das boot german" 97 DE-Treffer | ja |
| 10 | TorrentDownload.info (RSS) | `https://www.torrentdownload.info` | `GET /feed?q=<q>` | RSS | title, Hash im Link, Größe + Seeds + Peers im Text, pubDate | 200, 27 Items Sintel, "das boot german" 29 Items | ja |
| 11 | BT4G (Proxy) | `https://bt4gprx.com` | `GET /search?q=<q>&page=rss` (`category=movie`, `orderby=seeders`, `p=<n>` laut Site-URL, Wirkung nicht einzeln geprüft) | RSS | title, Magnet im `<link>`, Größe + Kategorie + Hash in description, pubDate | 200, 15 Items Sintel, 10 für Das Boot (33 DE) | ja |
| 12 | TorrentKitty | `https://www.torrentkitty.tv` | `GET /search/<q>/` | HTML | Name, Größe, Datum, Magnet. Keine Seeders | 200, 7 Treffer Sintel, "das boot german" 33 DE | ja |
| 13 | Rutor | `https://rutor.info` (auch `rutor.is`) | `GET /search/0/0/000/0/<q>` (Seite/Kat/Modus/Sort/Text, Kat und Sort nicht wirksam bestätigt) | HTML | Datum, Magnet, Titel, Größe, Seeders, Leechers | 200, 7 Magnets Sintel | nein (Russisch) |
| 14 | KickassTorrents | `https://kickasstorrents.to` | `GET /usearch/<q>/` dann Detailseite | HTML | Titel, Größe, Seeders, Leechers, Uploader. Magnet nur auf Detailseite | 200, 9 Zeilen, Detailseite liefert Magnet | teils |
| 15 | AnimeTosho | `https://animetosho.org` | `GET /search?q=<q>` | HTML | Titel, Größe (Bytes), Seeders/Leechers, Magnet, Datum | 200, 75 Magnets "frieren" | nein (Anime) |
| 16 | TokyoTosho | `https://www.tokyotosho.info` | `GET /rss.php?terms=<q>` | RSS | category, title, Magnet + Torrent-URL + Größe in description, Submitter, pubDate | 200, 149 Items "frieren" | nein (Anime) |
| 17 | SubsPlease | `https://subsplease.org` | `GET /api/?f=search&tz=UTC&s=<q>` | JSON | show, episode, release_date, downloads[].res, downloads[].magnet | 200, 90 Magnets "frieren" | nein (Anime, Englisch) |
| 18 | DMHY | `https://dmhy.org` | `GET /topics/rss/rss.xml?keyword=<q>` | RSS | title, enclosure (Magnet), link, pubDate, category | 200, 500 Items (1,8 MB) "frieren" | nein (Chinesisch) |
| 19 | Mikan Project | `https://mikanani.me` | `GET /RSS/Search?searchstr=<q>` | RSS | title, Hash im Link, enclosure (Größe, .torrent-URL), pubDate | 200, 100 Items "frieren" | nein (Chinesisch) |

Hinweis zu 2, 3, 4: Alle drei sind Torrentio-ähnliche Stremio-Addons und liefern zusammen die größte Treffermenge. Comet und TorrentsDB geben auch ohne Debrid-Konfiguration `infoHash` zurück (0 von ~1100 Streams mit `url` statt Hash). Der Magnet entsteht aus `infoHash` plus `sources[]`.

## 2. Parsing-Hinweise

**1 BitSearch.** `results[]`: `infohash`, `title`, `size` (Bytes), `seeders`, `leechers`, `category`/`subCategory`, `updatedAt`. Seitenweise über `page`, Antwort hat `pagination`. `sort=seeders` sortiert absteigend (getestet). Undokumentierte API, kann sich ändern. `solidtorrents.to` liefert dieselbe Seite und zählt nicht extra.

**2/3 Comet.** `streams[]`: `infoHash`, `behaviorHints.videoSize` (Bytes), `behaviorHints.filename`. `description` ist mehrzeilig: Zeile 1 `📄 Dateiname`, dann `👤 <seeders> 💾 <größe> 🔎 <scraper>`. Letzte Zeile hat Sprachflaggen nach `🌎`. Seeders sind bei Debrid-Cache-Treffern oft 0 oder fehlen. Für Deutsch zusätzlich auf `GERMAN`/`.DL.`/`Ger` im Dateinamen prüfen.

**4 TorrentsDB.** `streams[]`: `infoHash`, `title` mit Zeilen `<Name>` und `👤 <seeders> 💾 <größe> ⚙️ <quelle>`, `behaviorHints.filename`, `sources[]` (Präfix `tracker:`, ohne Präfix entfernen). Serien: `tt0944947:1:1`. Keine Sprachflaggen, Sprache nur aus dem Titel ableiten.

**5 MediaFusion.** `streams[]`: `infoHash`, `behaviorHints.filename`, `behaviorHints.videoSize`, `description` mit `📦 <größe>`, `▶️ <zahl>` (vermutlich Seeders), `🌐 <Sprachen>`, `🔗 <Quelle>`. Die Sprachzeile ist brauchbar für einen DE-Filter. Nur 25 Treffer, also eher Ergänzung. Die ElfHosted-Instanz liefert ohne Konfiguration leere Listen, nur diese Instanz nutzen.

**6 Peerflix.** Wie Stremio-Format oben: `infoHash`, `description` mit `👤 <seeders> 💾 <größe> 🌐 <quelle>`. Titel nur Spanisch/Englisch, nur als Zusatz für Filme.

**7 Torrents-CSV.** `torrents[]`: `infohash`, `name`, `size_bytes`, `seeders`, `leechers`. Nächste Seite: Feld `next` als `after=<next>`. Die Domain `torrents-csv.ml` ist geparkt, nur `.com` nutzen.

**8 Zilean.** Antwort ist ein JSON-Array: `info_hash`, `raw_title`, `size` (als String), `category` (`movie`/`tvSeries`), `languages[]`, `seasons[]`, `episodes[]`, `imdb_id`. Keine Seeders und kein Tracker-Status, also nur Hash-Quelle. `ImdbId=` lieferte für Sintel 0 Treffer, deshalb mit `Query=` suchen. Instanz von Dritten, `zilean.elfhosted.com` antwortet auf den Pfad mit 404.

**9 TorrentDownloads.** `item`: `title`, `category` (CDATA: `Movies`, `TV Shows`, ...), `size` (Bytes), `seeders`, `leechers`, `info_hash`, `pubDate`. Encoding ISO-8859-1. `s_cat` im RSS filtert nicht (Antwort identisch), also nach `category` clientseitig filtern.

**10 TorrentDownload.info.** `item`: `title`, `link` (endet auf `/<HASH>`), `description` = `Size: 556.65 MB Seeds: 11 , Peers: 2 Hash: <HASH>` (Regex). Die HTML-Suche lief einmal in ein Timeout, nur den Feed nehmen.

**11 BT4G.** `item`: `title`, `link` (Magnet, `&amp;` entschärfen), `description` = `Titel<br>Größe<br>Kategorie<br>Hash`, `pubDate`. Keine Seeders. Der Host `bt4gprx.com` ist ein Proxy, `bt4g.org` selbst gibt 403.

**12 TorrentKitty.** `table#archiveResult tr`: `td.name`, `td.size`, `td.date`, `td.action a[href^="magnet:"]`. DHT-Index, Größen sind teils falsch (Sintel zeigte "20.79 kb"). Keine Seeders, nur als Fallback.

**13 Rutor.** Zeilen `tr.gai` und `tr.tum`. Pro Zeile: Magnet `a[href^="magnet:"]`, Titel `a[href^="/torrent/"]`, Größe `td[align=right]`, Seeders `span.green` (nach dem `&nbsp;`), Leechers `span.red`. Meist russische Releases, für westliche Titel mit Original-Titel in der Suche. Datum ist russisch (`25 Авг 14`).

**14 KAT.** Liste: `tr.odd, tr.even`, Titel `a.cellMainLink`, Größe `td.nobr.center`, Seeders `td.green.center`, Leechers `td.red.lasttd`. Magnet erst auf der Detailseite (`/<slug>-t<id>.html`, Regex `magnet:\?xt=urn:btih:[0-9A-F]{40}`). Heißt N+1 Requests, deshalb nur für die Top 3 bis 5 nachladen oder ganz weglassen.

**15 AnimeTosho.** Je Treffer `div.home_list_entry`: `div.size[title]` ("Total file size: N bytes"), `div.link a` (Titel), `a[href^="magnet:"]`, `span[title="Seeders: 1 / Leechers: 242"]`. Das RSS- und JSON-Feed von der Seite gab 403, nur HTML geht. 75 Einträge pro Seite.

**16 TokyoTosho.** `item`: `category`, `title`, `description` (CDATA mit `<a href="magnet:...">`, `Size: 25.47MB`), `pubDate`. Der Parameter `type=` ändert nichts, `category` clientseitig filtern. Die Suche liefert auch Hentai/Doujin-Kategorien, die Kategorien `Hentai*` und `Other` aussortieren.

**17 SubsPlease.** Objekt mit Schlüssel pro Release: `show`, `episode`, `release_date`, `downloads[]` mit `res` ("480", "720", "1080") und `magnet`. Größe steht im Magnet als `xl=<Bytes>`. Keine Seeders. Nur SubsPlease-Releases, aber sauber.

**18 DMHY.** `item`: `title` (CDATA, Chinesisch/Romaji), `enclosure@url` = Magnet, `link`, `pubDate`. Keine Größe, keine Seeders. Große Antwort, früh abbrechen (z.B. erste 50 Items).

**19 Mikan.** `item`: `title`, `link` (`.../Home/Episode/<40-Hex-Hash>`, der Hash ist der Infohash), `enclosure@length` (Bytes), `enclosure@url` (.torrent). Keine Seeders. Ordentlich für Fansub-Batches.

## 3. Verworfen

| Kandidat | Grund |
|----------|-------|
| 1337x (`1337x.to`, `1337xx.to`) | 403 Cloudflare |
| ext.to, uindex.org, idope.se, YourBittorrent (beide Domains), TorLock (beide Domains, auch RSS), KAT `kickass.torrentbay.st`, RARGB `rargb.to`, `bt4g.org`, `nyaa.land`, `eztv.re`, TokyoTosho `search.php`, Comet ElfHosted | 403 Cloudflare/Blockade |
| Torrentfunk (Suche) | 403. Nur `rss.xml` mit Neuheiten geht, nicht suchbar |
| GloDLS, `rarbgmirror.com` | JS-Weiterleitung mit Token ("Loading..."), ohne Browser nicht lösbar |
| BTDigg (`btdig.com`, `www.`, `api.`) | Sofort "Too Many Requests" schon bei der ersten Anfrage, unbrauchbar |
| MagnetDL (`magnetdl.com`, `.hair`), Zooqle, TorrentGalaxy `.mx`, `tgx.rs` | 522/521/TLS-Abbruch, nicht erreichbar |
| TorrentGalaxy `.to`/`.one`, TorrentProject2, `torrentcsv.com`, `search-api.torbox.app`, `feed.animetosho.org`, `torrentz2.nz`, `btsow.mobi`, `toshocat.com` | DNS löst nicht auf |
| `torrentz2.eu`, `skrbt.top`, Erai-raws, AniDex | Verbindung/TLS abgebrochen, nicht erreichbar |
| `yts.mx` | DNS tot. `yts.bz` antwortet noch, sagt aber selbst, die Basis ziehe nach `movies-api.accel.li` um (steht schon in `tsearch.rs`) |
| TorrentGalaxy-Klon `torrentgalaxy.hair` | Ignoriert die Suche, 0 Treffer für "sintel" |
| `ilcorsaronero.pw` / `.link` | Geparkte Domain / 502 |
| BTSOW (`btsow.com`) | Seite per Template/JS, im HTML 0 Treffer |
| `torrent-paradise.ml` | Liefert eine unbeteiligte Seite, keine API |
| Snowfl | Token wird per JS erzeugt, nur im Browser |
| `kat.am`, `thekickass.com` | JS-Weiterleitung beziehungsweise Lander-Seite. Nur `kickasstorrents.to` geht |
| ETTV (`ettvcentral.com`) | JS-Weiterleitungsseite |
| TorrentProject (`torrentproject.cc`) | Liste hat Seeders/Größe, aber weder Hash noch Magnet (auch Detailseite ohne Magnet) |
| ShanaProject | Keine Magnets, kein Feed |
| Bangumi.moe | RSS nur mit `.torrent`-URL und ohne Hash |
| AniRena | RSS-Abfrage gibt Fehler (`Query deserialize error`), HTML ohne Magnets |
| isohunt.nz | "Seite nicht gefunden" |
| Rutracker | 403 und Login nötig |
| Sukebei | Nur Erwachseneninhalte, für Flimmer nicht gedacht |
| Annatar, Jackettio, StremThru-Torz, Knightcrawler, Torrentio-ElfHosted, `mediafusion.fun`, `mediafusion.elfhosted.com`, `comet.stremio.ru` | Landingpage statt Stremio-JSON, Konfiguration nötig, leere Liste, 404 oder 504 |
| Bitmagnet | Nur zum Selbsthosten, keine öffentliche Instanz gefunden |

Nicht neu gezählt: `solidtorrents.to` (gleiche Seite wie BitSearch), TPB-HTML-Mirrors (`piratebay.live`, `tpb.party`, `thehiddenbay.com`, `pirateproxy.live`) und `nyaa.iss.one` (gleiche Daten wie vorhandene Quellen), `limetorrents.lol` (vorhanden).

## 4. Empfehlung zum Einbau

Zuerst für Deutsch und Menge: BitSearch (1), Comet (2, 3), TorrentsDB (4), Zilean (8), TorrentDownload.info (10), TorrentDownloads (9), BT4G (11), Torrents-CSV (7). Für Anime: SubsPlease (17), AnimeTosho (15), TokyoTosho (16, mit Kategorie-Filter). Der Rest ist Ergänzung. Quellen ohne Seeders (Zilean, BT4G, TorrentKitty, DMHY, Mikan) im Ranking nach unten, Duplikate über den Infohash zusammenführen.

Quellen der Endpunkte: eigene Tests 20.11.2026. Stremio-Stream-Format: Stremio-Addon-SDK (`/stream/<type>/<id>.json`). Torznab-/Indexer-Kataloge wie Jackett wurden nicht gebraucht, jeder Endpunkt oben ist direkt getestet.
