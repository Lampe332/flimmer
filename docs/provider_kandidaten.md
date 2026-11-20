# Provider-Kandidaten fuer Flimmer (Stand 20.11.2026)

Getestet mit Shawshank (TMDB 278 / tt0111161), Breaking Bad S1E1 (TMDB 1396 / tt0903747), Endgame (TMDB 299534).
Prueflauf: HTTP-GET (alle Muster, Film + Serie + Endgame) plus Rendern im Browser-Pane (nur lesen, nichts angeklickt).

**Kernbefund Deutsch:** Kein einziger ID-basierter Embed-Player belegt deutsche Tonspur. Keine Doku nennt Deutsch als Audio. Wo ich die Quellenliste lesen konnte (vidzen), steht nur "Original". Deutsch-Untertitel gehen bei Embedbandit per `&ds_lang=de`. Einzige echte deutsche Quelle nach TMDB-ID: Einschalten (Sonderfall, siehe unten, passt nicht ins providers.json-Format).

## 1. Verifizierte Kandidaten (beste zuerst)

Test = HTTP-Status Film/Serie/Endgame, dann was die Seite im Browser zeigte. "Deutsch: nein/unbelegt" heisst: keine deutsche Tonspur nachweisbar.

| # | Name | Domain | ID | Film-Muster | Serien-Muster | Sprachen | Werbung | Test | Quelle |
|---|------|--------|----|-------------|---------------|----------|---------|------|--------|
| 1 | EmbedBandit | embedbandit.com | TMDB + IMDb | `/embed/movie?tmdb={tmdb}` | `/embed/tv?tmdb={tmdb}&season={s}&episode={e}` | Audio vermutlich nur Original; Untertitel inkl. Deutsch (`ds_lang=de`); Deutsch-Ton: unbelegt | nein (keine gesehen, Doku "no ads") | 200/200/200; Film und Serie spielten im Browser selbst an (Shawshank 2:22:29, Breaking Bad S1E1 58:06) | https://embedbandit.com/docs |
| 2 | Peachify | peachify.net | nur TMDB (IMDb = 404) | `/embed/movie/{tmdb}` | `/embed/tv/{tmdb}/{s}/{e}` | Menue "Audio & Subtitles"; Deutsch: unbelegt | nein (keine gesehen) | 200/200/200; Player mit Steuerleiste lud fuer Film und Serie | https://peachify.net/peachify-api |
| 3 | NxSha (Multi-Lang) | web.nxsha.app | TMDB + IMDb | `/embed/movie/{tmdb}` | `/embed/tv/{tmdb}/{s}/{e}` | Server heisst "Multi-Lang"; Deutsch: unbelegt | keine gesehen | 200/200/200; Seite zeigte Titel, Play-Button, Episodenliste, Untertitel-Menue | iframe-Ziel im HTML von embedwave.cc/embed/movie/278 (Muster per Test bestaetigt) |
| 4 | VidZen | vidzen.fun | TMDB + IMDb | `/movie/{tmdb}` | `/tv/{tmdb}/{s}/{e}` | "Multi-Audio"-Knopf, aber Quellenliste fuer Endgame/Shawshank nur "Original", 1080p HEVC (bis 4K beworben); Deutsch: nein | Doku "ad-free", keine gesehen | 200/200/200; Player lud ("Connecting to Planetary System"), Titel + Audio-Tracks-Knopf | https://vidzen.fun/ |
| 5 | VidPrime | vidprime.top | TMDB | `/movie/{tmdb}` | `/tv/{tmdb}/{s}/{e}` | Deutsch: unbelegt | unbekannt | 200/200/200; Poster + Ladekreis fuer Film und Serie. `vidsrc.pm` leitet hierher um | https://vidprime.top/ |
| 6 | VidSpark | vidspark.to | TMDB (Doku: auch IMDb) | `/movie/{tmdb}` | `/tv/{tmdb}/{s}/{e}` | Deutsch: unbelegt | unbekannt | 200/200/200; Poster + Ladekreis fuer Film und Serie. Gleiche Seite wie `moviesapi.to` | https://vidspark.to/ |
| 7 | Braflix | braflix.dev | TMDB + IMDb | `/embed/movie/{tmdb}` | `/embed/tv/{tmdb}/{s}/{e}` | `?audio=dub` nur fuer Englisch-Anime; Deutsch: nein | unbekannt | 200/200/200; Player mit Titel "The Shawshank Redemption 1994", lud | https://braflix.dev/ |
| 8 | Viduki | www.viduki.net | TMDB + IMDb | `/1/movie/{tmdb}` (auch `/2/`) | `/1/tv/{tmdb}/{s}/{e}` | Deutsch: unbelegt; wirbt mit 4K-Upscaling | "minimal ads" | 200/200/200 fuer `/1/`, `/2/` ebenso; Vidstack-Player mit Titel, Zeitleiste. Server-Nr. 1/2 aus Doku-Vorlage `{base}/{api}/movie/{id}`, Wert 1 und 2 per Test | https://www.viduki.net/ (`vidsrc.wtf/api/1/...` leitet hierher) |
| 9 | TGVID | tgvid.lovable.app | TMDB | `/embed/movie/{tmdb}` | `/embed/tv/{tmdb}/{s}/{e}` | Audio-Menue vorhanden, zeigte nur "Original"; Deutsch: nein | unbekannt | 200/200/200; voller Player (Titel, Qualitaet, Audio, Untertitel) | https://tgvid.lovable.app/docs. Achtung: gehostet auf lovable.app, kann jederzeit verschwinden |
| 10 | NontonGo | nontongo.win | TMDB + IMDb | `/embed/movie/{tmdb}` | `/embed/tv/{tmdb}/{s}/{e}` | Deutsch: unbelegt | unbekannt | 200/200/200 (Titel serverseitig: Shawshank, Breaking Bad, Endgame); Seite mit grossem "PLAY NOW"-Knopf, ob direkt abspielt: unklar | https://nontongo.win/ (Doku-Seite nennt beide Muster) |
| 11 | VidSrc Buzz | vidsrc.buzz | TMDB + IMDb | `/embed/movie/{tmdb}` | `/embed/tv/{tmdb}/{s}/{e}` | Untertitel mehrsprachig; Audio-Tracks "wo vorhanden"; Deutsch: unbelegt | JA (Banner "Chat with Singles ..." im Player) | 200/200/200; Player startete (0:03 / 2:22:33) | https://vidsrc.buzz/ |
| 12 | Embed-API Stream | watch.embed-api.stream | TMDB | `/embed/movie/{tmdb}` | `/embed/tv/{tmdb}/{s}/{e}` | 11 Provider im Player; Deutsch: unbelegt | JA (Werbe-Overlay "WhatsApp" im Film) | 200/200/200; Film spielte an | https://watch.embed-api.stream/ (`player.embed-api.stream` leitet hierher um) |

Optional, nicht deutsch: **Frembed** (frembed.shop, nur Franzoesisch). Film `https://frembed.shop/embed/movie/{tmdb}` (200, Seite "Frembed : Voir des Films..."), Serie nur ueber `https://frembed.shop/api/serie.php?id={tmdb}&sa={s}&epi={e}` (200, leitet auf `/embed/serie/..` um; `/embed/tv/..` ist 404). Quelle: https://fmhy.net/non-english (Eintrag frembed.uno).

## 2. Ready-to-paste (providers.json-Format, beste zuerst)

`no_ads: true` nur wo ich keine Werbung gesehen habe und der Anbieter "keine Werbung" nennt. Rest `false`.

```json
[
  { "id": "embedbandit", "name": "EmbedBandit",      "no_ads": true,  "movie": "https://embedbandit.com/embed/movie?tmdb={tmdb}",   "tv": "https://embedbandit.com/embed/tv?tmdb={tmdb}&season={s}&episode={e}" },
  { "id": "peachify",    "name": "Peachify",         "no_ads": true,  "movie": "https://peachify.net/embed/movie/{tmdb}",           "tv": "https://peachify.net/embed/tv/{tmdb}/{s}/{e}" },
  { "id": "nxsha",       "name": "NxSha",            "no_ads": false, "movie": "https://web.nxsha.app/embed/movie/{tmdb}",          "tv": "https://web.nxsha.app/embed/tv/{tmdb}/{s}/{e}" },
  { "id": "vidzen",      "name": "VidZen",           "no_ads": false, "movie": "https://vidzen.fun/movie/{tmdb}",                   "tv": "https://vidzen.fun/tv/{tmdb}/{s}/{e}" },
  { "id": "vidprime",    "name": "VidPrime",         "no_ads": false, "movie": "https://vidprime.top/movie/{tmdb}",                 "tv": "https://vidprime.top/tv/{tmdb}/{s}/{e}" },
  { "id": "vidspark",    "name": "VidSpark",         "no_ads": false, "movie": "https://vidspark.to/movie/{tmdb}",                  "tv": "https://vidspark.to/tv/{tmdb}/{s}/{e}" },
  { "id": "braflix",     "name": "Braflix",          "no_ads": false, "movie": "https://braflix.dev/embed/movie/{tmdb}",            "tv": "https://braflix.dev/embed/tv/{tmdb}/{s}/{e}" },
  { "id": "viduki",      "name": "Viduki",           "no_ads": false, "movie": "https://www.viduki.net/1/movie/{tmdb}",             "tv": "https://www.viduki.net/1/tv/{tmdb}/{s}/{e}" },
  { "id": "tgvid",       "name": "TGVID",            "no_ads": false, "movie": "https://tgvid.lovable.app/embed/movie/{tmdb}",      "tv": "https://tgvid.lovable.app/embed/tv/{tmdb}/{s}/{e}" },
  { "id": "nontongo",    "name": "NontonGo",         "no_ads": false, "movie": "https://nontongo.win/embed/movie/{tmdb}",           "tv": "https://nontongo.win/embed/tv/{tmdb}/{s}/{e}" },
  { "id": "vidsrcbuzz",  "name": "VidSrc Buzz",      "no_ads": false, "movie": "https://vidsrc.buzz/embed/movie/{tmdb}",            "tv": "https://vidsrc.buzz/embed/tv/{tmdb}/{s}/{e}" },
  { "id": "embedapi",    "name": "Embed-API Stream", "no_ads": false, "movie": "https://watch.embed-api.stream/embed/movie/{tmdb}", "tv": "https://watch.embed-api.stream/embed/tv/{tmdb}/{s}/{e}" }
]
```

Optional (nur Franzoesisch):

```json
[
  { "id": "frembed", "name": "Frembed (FR)", "no_ads": false, "movie": "https://frembed.shop/embed/movie/{tmdb}", "tv": "https://frembed.shop/api/serie.php?id={tmdb}&sa={s}&epi={e}" }
]
```

Hinweise zum Einbau: EmbedBandit verweigert iframes mit `sandbox`-Attribut (Doku "Rules for embedding") und zeigt dann nur einen Hinweis. Viduki hat mehrere Server-Nummern (`/1/`, `/2/`), jede als eigener Eintrag moeglich.

## 3. Sonderfall: Einschalten (echt deutsch, TMDB-Schluessel, aber zweistufig)

Passt nicht ins Muster, weil erst eine JSON-API den Hoster-Link liefert:

- `GET https://einschalten.in/api/movies/{tmdb}/watch` liefert `{"releaseName":"...German...","streamUrl":"https://vide0.net/e/..."}` (nur Filme, keine Serien).
- Getestet: Endgame 299534 = `Avengers.4.Endgame.2019.BDRip.AC3D.5.1.German.XViD-PS`, Matrix 603, Inception 27205, Interstellar 157336, Dark Knight 155 alle mit "German"-Release. Metadaten (`/api/movies/{tmdb}`) kommen auf Deutsch.
- Probleme: (1) Fuer Shawshank 278 kommt ein falscher Release ("Redemption.2004"), Treffer also nicht garantiert richtig. (2) Qualitaet eher BDRip/XviD, nicht 1080p. (3) `streamUrl` ist ein DoodStream-Hoster: `vide0.net` liegt hinter Cloudflare-Pruefung (403 per Skript) und der Browser wurde auf `playmogo.com` weitergeleitet, also mit Werbung/Weiterleitungen rechnen. (4) API sendet keinen CORS-Header, Aufruf muss aus dem App-Backend kommen.
- Quelle: https://fmhy.net/non-english (German, Streaming).
- Entscheidung liegt bei dir: lohnt sich nur als eigener "Deutsch (Einschalten)"-Pfad im Code, nicht als providers.json-Zeile.

## 4. Abgelehnt

| Kandidat | Grund |
|----------|-------|
| embedwave.cc | Nur Huelle um `web.nxsha.app`, blieb im Browser bei "Loading player" haengen. Nxsha direkt nehmen (Nr. 3) |
| apiplayer.ru | Seite zeigt "Verification Failed" ohne Einbettungs-Whitelist, nicht pruefbar. Schwesterdienst von vaplayer.ru (schon in Liste) |
| vidcore.org | Gleiches Template/Anbieter-Familie wie vidcore.io (schon in Liste), Duplikat |
| videasy.xyz | Duplikat Videasy. Hinweis: `player.videasy.net` aus der Liste antwortet bei mir nicht, Doku nennt jetzt `videasy.xyz/embed/movie/{tmdb}` (200) |
| vidbolt.xyz | Identisch mit vidbolt.pro (gleiche Seite, schon in Liste); wirbt mit Hindi/Tamil-Audio |
| 2embed.org / 2embed.skin | Duplikat 2Embed (`2embed.skin` = offizielle Zweitdomain) bzw. Weiterleitungs-Ladeseite mit Werbe-Overlay |
| vidsrc.net, xprime.tv, madplay.site, vidsrc.rip | Alle liefern dieselbe "Redirecting..."-Ladeseite mit Anti-Bot-/Werbe-Skript, kein Player |
| rgshows.ru, flickystream.ru | "This domain is for sale" |
| vidjoy.pro | Leitet im Browser auf eine Werbe-/Cloudflare-Domain (`bigdatafuturedev.buzz`) um |
| cinemaos.live, vidplus.to, player.vidsrc.co | Cloudflare-Sicherheitspruefung (403 "Just a moment") vor dem Player, im iframe nicht nutzbar |
| vidify.top | 521 Web server is down |
| vidup.to | Seite laedt, blieb nach 12 s bei "Getting things ready", Muster nirgends dokumentiert |
| vidrock.net | Keine Doku, stark verschleierter Code mit VAST-Werbeframework |
| anyembed.xyz | 451 aus rechtlichen Gruenden |
| ezvidapi.com | 404 |
| vidsrc.pm | Leitet auf vidprime.top um (dort als Nr. 5 gefuehrt) |
| moviesapi.to | Gleiche Seite wie vidspark.to (Nr. 6) |
| embed.su, vidsrc.icu/.vip/.xyz/.in, vidsrc.cc, hexa.watch, rivestream.org/.app, player.smashy.stream, embed.warezcdn.link, zxcstream.xyz, godriveplayer.com, vidora.su, flixer.sh, cineby.app, uembed.xyz, vidstreamapi.com, moviekex.online, primewire.tf | Domain antwortet nicht (Verbindungsfehler/DNS) |
| nhdapi.st, embed.streammafia.to | Nicht erreichbar |
| spencerdevs.xyz, bingeflix.tv, madplay-API | "Coming Soon" bzw. 404 auf dem Muster |
| Kinoger, Moflix, S.TO, Movie2k, FilmPalast, Kinoking, Megakino, Streamkiste (FMHY-Deutsch-Liste) | Keine Embed-URL nach TMDB/IMDb-ID (Kinoger hinter Cloudflare, Moflix reine JS-App mit eigenen IDs); nicht einbettbar im Sinn dieser Liste |

## 5. Nebenbefund zur bestehenden Liste (nichts geaendert)

Bei diesem Lauf antworteten nicht: videasy (`player.videasy.net`), autoembed, moviesapi (`.club`), vidking, vidlove, vidgod (`vidsrc.cc`); Timeout: 1embed, peestream. Cloudflare-Sperre/403: mapple, vidfast (`.to`; `vidfast.vc/movie/{tmdb}` antwortet 200, `/embed/...` dort 404). `vares` (`vidsrc.rip`) liefert nur die Werbe-Ladeseite. Ob das an meinem Netz liegt, ist nicht geklaert; vor Loeschen auf deinem Rechner gegenpruefen.
