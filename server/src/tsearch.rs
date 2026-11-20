//! Torrent search: all sources in parallel over the normal network (plain HTTPS to index sites,
//! no torrent traffic), merged and ranked by [`crate::release`]. The first set of sources follows
//! github.com/melqtx/tork (MIT), plus a Torrentio lookup by IMDb id; the rest is listed with the
//! test results in docs/torrent_quellen_kandidaten.md.

use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};

use axum::{
    Json,
    extract::{Path, Query, State},
};
use regex::Regex;
use serde::Serialize;
use serde_json::{Value, json};

use crate::{
    api::{AppState, Params, St, TitleRef, title_ref},
    db::now_ms,
    error::{ApiError, ApiResult},
    model::MediaType,
    release::{self, Hit, clean_hash, normalize, parse_magnet, parse_size},
    scan::load_settings,
    tmdb::TTL_TITLE,
};

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";
const SOURCE_TIMEOUT: Duration = Duration::from_secs(12);
const CACHE_TTL: Duration = Duration::from_secs(300);
/// A source that answered "blocked" (403/429/503) is left alone this long.
const COOLDOWN: Duration = Duration::from_secs(600);
const MAX_RESULTS: usize = 120;

/// One search source as the settings list it.
#[derive(Clone, Copy, Serialize)]
pub struct SourceInfo {
    pub id: &'static str,
    /// Only asked with an IMDb id (Stremio add-ons).
    pub imdb: bool,
    /// Only asked for anime titles (or a free search marked as anime).
    pub anime: bool,
    /// On unless the user turns it off; slow or odd sources start off.
    pub default_on: bool,
    /// Reports seeders; without them its hits rank below live swarms.
    pub seeders: bool,
}

const fn src(id: &'static str, imdb: bool, anime: bool, default_on: bool, seeders: bool) -> SourceInfo {
    SourceInfo { id, imdb, anime, default_on, seeders }
}

/// Every source, in the order of the status list.
pub const SOURCES: &[SourceInfo] = &[
    src("torrentio", true, false, true, true),
    src("comet", true, false, true, false),
    src("cometweebs", true, false, true, false),
    src("torrentsdb", true, false, true, true),
    src("mediafusion", true, false, true, true),
    src("peerflix", true, false, true, true),
    src("knaben", false, false, true, true),
    src("bitsearch", false, false, true, true),
    src("yts", false, false, true, true),
    src("eztv", true, false, true, true),
    src("piratebay", false, false, true, true),
    src("therarbg", false, false, true, true),
    src("limetorrents", false, false, true, true),
    src("1337x", false, false, true, true),
    src("torrentscsv", false, false, true, true),
    src("torrentdownloads", false, false, true, true),
    src("torrentdownload", false, false, true, true),
    src("zilean", false, false, true, false),
    src("bt4g", false, false, true, false),
    src("torrentkitty", false, false, false, false),
    src("rutor", false, false, false, true),
    src("nyaa", false, false, true, true),
    src("subsplease", false, true, true, false),
    src("animetosho", false, true, true, true),
    src("tokyotosho", false, true, true, false),
    src("dmhy", false, true, false, false),
    src("mikan", false, true, false, false),
];

/// Sources that start turned off: TorrentKitty (DHT index, wrong sizes, no seeders), Rutor (Russian releases),
/// DMHY and Mikan (Chinese fansubs, big feeds).
pub fn default_off() -> Vec<String> {
    SOURCES.iter().filter(|s| !s.default_on).map(|s| s.id.to_owned()).collect()
}

/// What to look for. Title lookups carry the IMDb id and the titles used to verify text hits.
#[derive(Default)]
pub struct Search {
    /// Free-text queries (first = main).
    pub texts: Vec<String>,
    /// The bare titles (no year or episode), for sources that take those (Zilean, anime sites).
    pub names: Vec<String>,
    pub imdb: Option<String>,
    pub kind: Option<MediaType>,
    pub episode: Option<(u32, u32)>,
    /// Normalized accepted titles; empty for a free search (nothing is filtered then).
    pub titles: Vec<String>,
    pub year: Option<u32>,
    /// Anime (TMDB animation from Japan, China or Korea, or a free search marked as anime): asks the anime sites too.
    pub anime: bool,
}

enum Fail {
    /// The source needs something this search does not have (e.g. an IMDb id).
    Skipped(&'static str),
    Blocked(String),
    Error(String),
    /// Refused a moment ago; minutes until it is asked again.
    Resting(u64),
}

type Hits = Result<Vec<Hit>, Fail>;

/// The merged, unfiltered and unranked result of one search.
pub struct Found {
    pub torrents: Vec<release::Torrent>,
    statuses: Vec<SourceStatus>,
}

pub struct Searcher {
    http: reqwest::Client,
    cache: Mutex<HashMap<String, (Instant, Arc<Found>)>>,
    /// Source → until when it is not asked (it answered "blocked").
    cooldown: Mutex<HashMap<&'static str, Instant>>,
}

impl Searcher {
    pub fn new() -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .user_agent(UA)
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| format!("http client: {e}"))?;
        Ok(Self { http, cache: Mutex::default(), cooldown: Mutex::default() })
    }

    /// Forgets cached searches (the set of sources changed).
    pub fn clear_cache(&self) {
        self.cache.lock().unwrap().clear();
    }
}

fn fail(e: reqwest::Error) -> Fail {
    if e.is_timeout() {
        Fail::Error("timeout".into())
    } else if e.is_connect() {
        Fail::Error("cannot connect (site down or blocked by the provider's DNS)".into())
    } else {
        Fail::Error("request failed".into())
    }
}

async fn text(req: reqwest::RequestBuilder) -> Result<String, Fail> {
    let resp = req.send().await.map_err(fail)?;
    let status = resp.status();
    let body = decode(&resp.bytes().await.map_err(fail)?);
    if status.is_success() {
        return Ok(body);
    }
    if matches!(status.as_u16(), 403 | 429 | 503) {
        let why = if body.contains("Just a moment") || body.contains("challenges.cloudflare.com") {
            "Cloudflare bot check"
        } else {
            "access denied"
        };
        return Err(Fail::Blocked(format!("HTTP {} ({why})", status.as_u16())));
    }
    Err(Fail::Error(format!("HTTP {}", status.as_u16())))
}

/// UTF-8 when the bytes are UTF-8, whatever the site declares (TorrentDownloads says ISO-8859-1 and sends
/// UTF-8 names); else Latin-1, byte by byte.
fn decode(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_owned(),
        Err(_) => bytes.iter().map(|b| char::from(*b)).collect(),
    }
}

async fn get_json(req: reqwest::RequestBuilder) -> Result<Value, Fail> {
    serde_json::from_str(&text(req).await?).map_err(|_| Fail::Error("unexpected answer (no JSON)".into()))
}

fn num(v: &Value) -> u64 {
    v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())).unwrap_or(0)
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// "2024-05-01…" → unix seconds (date precision).
fn date_secs(text: &str) -> Option<i64> {
    let mut parts = text.get(..10)?.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (parts.next()??, parts.next()??, parts.next()??);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Days from civil (Howard Hinnant).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468) * 86_400)
}

fn unix(v: &Value) -> Option<i64> {
    Some(num(v) as i64).filter(|n| *n > 0)
}

fn path_enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>().replace('+', "%20")
}

fn unescape(s: &str) -> String {
    s.trim()
        .trim_start_matches("<![CDATA[")
        .trim_end_matches("]]>")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#34;", "\"")
        .replace("&#39;", "'")
        .replace("&#039;", "'")
        .replace("&nbsp;", " ")
}

/// The items of an RSS feed (or the rows of a page split at `marker`).
fn chunks<'a>(text: &'a str, marker: &str) -> impl Iterator<Item = &'a str> {
    text.split(marker).skip(1)
}

/// Text of the first `<name>…</name>` (attributes allowed), CDATA and entities resolved.
fn xml_tag(item: &str, name: &str) -> Option<String> {
    let open = format!("<{name}");
    let close = format!("</{name}>");
    let mut from = 0;
    while let Some(i) = item[from..].find(&open).map(|i| i + from) {
        let after = i + open.len();
        if matches!(item[after..].chars().next(), Some('>' | ' ')) {
            let start = after + item[after..].find('>')? + 1;
            let end = start + item[start..].find(&close)?;
            return Some(unescape(&item[start..end]));
        }
        from = after;
    }
    None
}

/// Value of `attr` in the first `<name …>` tag.
fn tag_attr(item: &str, name: &str, attr: &str) -> Option<String> {
    let i = item.find(&format!("<{name} "))?;
    let tag = &item[i..i + item[i..].find('>')?];
    let key = format!("{attr}=\"");
    let a = tag.find(&key)? + key.len();
    Some(unescape(&tag[a..a + tag[a..].find('"')?]))
}

/// The first magnet link in a piece of HTML/XML (entities resolved).
fn magnet_in(text: &str) -> Option<String> {
    static MAGNET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(magnet:\?xt=urn:btih:[^"'<\s]+)"#).expect("static regex"));
    MAGNET.captures(text).map(|c| unescape(&c[1]))
}

/// "Fri, 26 Oct 2018 14:37:42 +0200", "22 Jul 2025 22:40:09", "Fri,20 Nov 2026" → unix seconds (date precision).
fn rss_date(text: &str) -> Option<i64> {
    static DATE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)(\d{1,2})\s*(jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]*\s+(\d{4})").expect("static regex"));
    let c = DATE.captures(text)?;
    let month = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"]
        .iter()
        .position(|m| c[2].eq_ignore_ascii_case(m))?
        + 1;
    date_secs(&format!("{}-{month:02}-{:02}", &c[3], c[1].parse::<u32>().ok()?))
}

/// Audio language of a country flag (two regional indicator letters), as release codes.
fn flag_lang(country: &str) -> Option<&'static str> {
    Some(match country {
        "DE" | "AT" | "CH" => "de",
        "GB" | "US" | "AU" | "CA" => "en",
        "FR" | "BE" => "fr",
        "IT" => "it",
        "ES" | "MX" | "AR" => "es",
        "RU" => "ru",
        "UA" => "uk",
        "IN" => "hi",
        "CN" | "TW" | "HK" => "zh",
        "JP" => "ja",
        "KR" => "ko",
        "PL" => "pl",
        "NL" => "nl",
        "PT" | "BR" => "pt",
        "TR" => "tr",
        "SE" => "sv",
        "DK" => "da",
        "NO" => "no",
        "FI" => "fi",
        "CZ" => "cs",
        "HU" => "hu",
        "SA" | "AE" | "EG" => "ar",
        _ => return None,
    })
}

/// Languages from flags (🇩🇪), Comet's 🌎 (multi) and 💃 (Latino), and "🌐 English + German" lines.
fn stated_languages(text: &str) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    let mut add = |l: &'static str| {
        if !out.contains(&l) {
            out.push(l);
        }
    };
    let chars: Vec<char> = text.chars().collect();
    let indicator = |c: char| ('\u{1F1E6}'..='\u{1F1FF}').contains(&c);
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if indicator(c) && chars.get(i + 1).is_some_and(|d| indicator(*d)) {
            let letter = |c: char| char::from(b'A' + (c as u32 - 0x1F1E6) as u8);
            if let Some(l) = flag_lang(&format!("{}{}", letter(c), letter(chars[i + 1]))) {
                add(l);
            }
            i += 2;
            continue;
        }
        match c {
            '🌎' => add("multi"),
            '💃' => add("es"),
            _ => {}
        }
        i += 1;
    }
    for line in text.lines() {
        if let Some(rest) = line.trim().strip_prefix('🌐') {
            release::languages_in(rest).into_iter().for_each(&mut add);
        }
    }
    out
}

/// A language code as a source writes it ("de", "German", "multi") → the release code.
fn lang_code(code: &str) -> Option<&'static str> {
    let c = code.trim().to_lowercase();
    ["de", "en", "fr", "it", "es", "ru", "uk", "hi", "zh", "ja", "ko", "pl", "nl", "pt", "tr", "sv", "da", "no", "fi", "cs", "hu", "ar", "multi"]
        .into_iter()
        .find(|l| *l == c)
        .or_else(|| release::languages_in(&c).first().copied())
}

/// Names some sites store twice encoded ("Ð¡Ð¸Ð½Ñ‚ÐµÐ»" for "Синтел"): UTF-8 bytes read as Windows-1252.
/// Turned back when every character maps to one byte and those bytes are valid UTF-8; anything else stays.
fn fix_mojibake(s: &str) -> String {
    const CP1252: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—',
        '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
    ];
    if s.is_ascii() {
        return s.to_owned();
    }
    let bytes: Option<Vec<u8>> = s
        .chars()
        .map(|c| match c as u32 {
            n @ (0..=0x7f | 0xa0..=0xff) => Some(n as u8),
            _ => CP1252.iter().position(|x| *x == c).map(|i| 0x80 + i as u8),
        })
        .collect();
    bytes.and_then(|b| String::from_utf8(b).ok()).filter(|fixed| fixed.chars().count() < s.chars().count()).unwrap_or_else(|| s.to_owned())
}

fn hit(source: &'static str, hash: &str, name: &str, size: u64, seeders: u64, leechers: u64) -> Option<Hit> {
    Some(Hit {
        source,
        infohash: clean_hash(hash)?,
        name: fix_mojibake(name.trim()),
        size,
        seeders: seeders.min(u64::from(u32::MAX)) as u32,
        leechers: leechers.min(u64::from(u32::MAX)) as u32,
        ..Hit::default()
    })
    .filter(|h| !h.name.is_empty())
}

/// IMDb id and text queries, most precise first.
fn queries(s: &Search) -> Vec<String> {
    s.imdb.iter().chain(&s.texts).cloned().collect()
}

// ---------- sources ----------

async fn knaben(http: &reqwest::Client, s: &Search) -> Hits {
    let mut out = Vec::new();
    for q in &s.texts {
        let body = json!({
            "search_type": "100%", "search_field": "title", "query": q,
            "order_by": "seeders", "order_direction": "desc", "from": 0, "size": 80,
            "hide_unsafe": true, "hide_xxx": true,
        });
        let v = get_json(http.post("https://api.knaben.org/v1").json(&body)).await?;
        for h in v.get("hits").and_then(Value::as_array).into_iter().flatten() {
            let mut found = hit("knaben", str_of(h, "hash"), str_of(h, "title"), num(&h["bytes"]), num(&h["seeders"]), num(&h["peers"]));
            if let Some(f) = &mut found {
                f.added = date_secs(str_of(h, "date"));
            }
            out.extend(found);
        }
    }
    Ok(out)
}

async fn yts(http: &reqwest::Client, s: &Search) -> Hits {
    if s.kind == Some(MediaType::Tv) {
        return Err(Fail::Skipped("movies only"));
    }
    let term = s.imdb.as_ref().or(s.texts.first()).ok_or(Fail::Skipped("no query"))?;
    let mut last = Fail::Error("no mirror answered".into());
    for base in ["https://movies-api.accel.li", "https://yts.lt", "https://yts.gg"] {
        let req = http.get(format!("{base}/api/v2/list_movies.json")).query(&[("query_term", term.as_str()), ("limit", "20")]);
        let v = match get_json(req).await {
            Ok(v) => v,
            Err(e) => {
                last = e;
                continue;
            }
        };
        let mut out = Vec::new();
        for m in v.pointer("/data/movies").and_then(Value::as_array).into_iter().flatten() {
            for t in m.get("torrents").and_then(Value::as_array).into_iter().flatten() {
                let name = format!(
                    "{} {} {} {} YTS",
                    str_of(m, "title_long"),
                    str_of(t, "quality"),
                    str_of(t, "type"),
                    str_of(t, "video_codec")
                );
                let mut h = hit("yts", str_of(t, "hash"), &name, num(&t["size_bytes"]), num(&t["seeds"]), num(&t["peers"]));
                if let Some(h) = &mut h {
                    h.exact = s.imdb.is_some();
                    h.added = unix(&t["date_uploaded_unix"]);
                }
                out.extend(h);
            }
        }
        return Ok(out);
    }
    Err(last)
}

async fn eztv(http: &reqwest::Client, s: &Search) -> Hits {
    let (Some(imdb), Some((season, episode))) = (&s.imdb, s.episode) else {
        return Err(Fail::Skipped("needs an episode with IMDb id"));
    };
    let mut out = Vec::new();
    for page in 1..=4 {
        let req = http.get("https://eztvx.to/api/get-torrents").query(&[
            ("imdb_id", imdb.trim_start_matches("tt")),
            ("limit", "100"),
            ("page", &page.to_string()),
        ]);
        let v = get_json(req).await?;
        let list = v.get("torrents").and_then(Value::as_array).cloned().unwrap_or_default();
        for t in &list {
            if num(&t["season"]) != u64::from(season) || ![0, u64::from(episode)].contains(&num(&t["episode"])) {
                continue;
            }
            let name = str_of(t, "filename").trim_end_matches(".mkv").trim_end_matches(".mp4");
            let mut h = hit("eztv", str_of(t, "hash"), name, num(&t["size_bytes"]), num(&t["seeds"]), num(&t["peers"]));
            if let Some(h) = &mut h {
                h.exact = true;
                h.added = unix(&t["date_released_unix"]);
                h.trackers = parse_magnet(str_of(t, "magnet_url")).map(|m| m.2).unwrap_or_default();
            }
            out.extend(h);
        }
        if list.len() < 100 {
            break;
        }
    }
    Ok(out)
}

async fn nyaa(http: &reqwest::Client, s: &Search) -> Hits {
    static FIELD: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?s)<title>(.*?)</title>.*?<nyaa:seeders>(\d+)<.*?<nyaa:leechers>(\d+)<.*?<nyaa:infoHash>([0-9a-fA-F]{40})<.*?<nyaa:size>([^<]+)<")
            .expect("static regex")
    });
    let mut out = Vec::new();
    for q in &s.texts {
        let xml = text(http.get("https://nyaa.si/").query(&[("page", "rss"), ("q", q.as_str()), ("c", "1_0"), ("f", "0")])).await?;
        for item in xml.split("<item>").skip(1) {
            if let Some(c) = FIELD.captures(item) {
                let n = |i: usize| c[i].parse().unwrap_or(0);
                out.extend(hit("nyaa", &c[4], &unescape(&c[1]), parse_size(&c[5]), n(2), n(3)));
            }
        }
    }
    Ok(out)
}

async fn piratebay(http: &reqwest::Client, s: &Search) -> Hits {
    let mut out = Vec::new();
    for q in queries(s) {
        let v = get_json(http.get("https://apibay.org/q.php").query(&[("q", q.as_str()), ("cat", "200")])).await?;
        for t in v.as_array().into_iter().flatten() {
            if num(&t["id"]) == 0 {
                continue;
            }
            let mut h = hit("piratebay", str_of(t, "info_hash"), str_of(t, "name"), num(&t["size"]), num(&t["seeders"]), num(&t["leechers"]));
            if let Some(h) = &mut h {
                h.exact = s.kind == Some(MediaType::Movie) && s.imdb.as_deref().is_some_and(|i| i == str_of(t, "imdb"));
                h.added = unix(&t["added"]);
            }
            out.extend(h);
        }
    }
    Ok(out)
}

/// Rows of a 1337x search page: (detail path, name, seeders, leechers, size).
fn x1337_rows(html: &str) -> Vec<(String, String, u64, u64, u64)> {
    static ROW: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?s)<a href="(/torrent/\d+/[^"]+)">([^<]+)</a>.*?class="coll-2 seeds">(\d+)<.*?class="coll-3 leeches">(\d+)<.*?class="coll-4 size[^"]*">([^<]+)<"#)
            .expect("static regex")
    });
    ROW.captures_iter(html)
        .map(|c| (c[1].to_owned(), unescape(&c[2]), c[3].parse().unwrap_or(0), c[4].parse().unwrap_or(0), parse_size(&c[5])))
        .collect()
}

fn first_magnet(html: &str) -> Option<String> {
    static MAGNET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"href="(magnet:\?xt=urn:btih:[^"]+)""#).expect("static regex"));
    MAGNET.captures(html).map(|c| unescape(&c[1]))
}

async fn x1337(http: &reqwest::Client, s: &Search) -> Hits {
    const BASE: &str = "https://1337x.to";
    let q = s.texts.first().ok_or(Fail::Skipped("no query"))?;
    let html = text(http.get(format!("{BASE}/search/{}/1/", path_enc(q)))).await?;
    // The magnet is only on the detail page: fetch the best few.
    let rows: Vec<_> = x1337_rows(&html).into_iter().take(8).collect();
    let details = rows.iter().map(|r| text(http.get(format!("{BASE}{}", r.0))));
    let pages = futures_util::future::join_all(details).await;
    Ok(rows
        .into_iter()
        .zip(pages)
        .filter_map(|((_, name, se, le, size), page)| {
            let (hash, _, trackers) = parse_magnet(&first_magnet(&page.ok()?)?)?;
            let mut h = hit("1337x", &hash, &name, size, se, le)?;
            h.trackers = trackers;
            Some(h)
        })
        .collect())
}

async fn therarbg(http: &reqwest::Client, s: &Search) -> Hits {
    let mut out = Vec::new();
    for q in queries(s) {
        let v = get_json(http.get(format!("https://therarbg.to/get-posts/keywords:{}:format:json/", path_enc(&q)))).await?;
        for t in v.get("results").and_then(Value::as_array).into_iter().flatten() {
            if !matches!(str_of(t, "c"), "Movies" | "TV" | "Anime" | "Documentaries") {
                continue;
            }
            let mut h = hit("therarbg", str_of(t, "h"), str_of(t, "n"), num(&t["s"]), num(&t["se"]), num(&t["le"]));
            if let Some(h) = &mut h {
                h.exact = s.kind == Some(MediaType::Movie) && s.imdb.as_deref().is_some_and(|i| i == str_of(t, "i"));
                h.added = unix(&t["a"]);
            }
            out.extend(h);
        }
    }
    Ok(out)
}

/// Rows of a LimeTorrents search page.
fn lime_rows(html: &str) -> Vec<Hit> {
    static ROW: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?s)/torrent/([0-9A-Fa-f]{40})\.torrent[^"]*"[^>]*></a><a href="[^"]*">([^<]+)</a>.*?<td class="tdnormal">([\d.,]+ [KMGT]i?B)</td><td class="tdseed">([\d,]+)</td><td class="tdleech">([\d,]+)</td>"#)
            .expect("static regex")
    });
    let n = |s: &str| s.replace(',', "").parse().unwrap_or(0);
    ROW.captures_iter(html).filter_map(|c| hit("limetorrents", &c[1], &unescape(&c[2]), parse_size(&c[3]), n(&c[4]), n(&c[5]))).collect()
}

async fn limetorrents(http: &reqwest::Client, s: &Search) -> Hits {
    let mut out = Vec::new();
    for q in &s.texts {
        let html = text(http.get(format!("https://www.limetorrents.fun/search/all/{}/seeds/1/", path_enc(q)))).await?;
        out.extend(lime_rows(&html));
    }
    Ok(out)
}

/// One Torrentio stream → hit. Title lines: release name, (file name for packs,) "👤 12 💾 1.2 GB ⚙️ Source".
fn torrentio_hit(stream: &Value) -> Option<Hit> {
    static SEEDS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"👤\s*(\d+)").expect("static regex"));
    static SIZE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"💾\s*([\d.,]+\s*[KMGT]i?B)").expect("static regex"));
    let title = str_of(stream, "title");
    let seeders = SEEDS.captures(title).and_then(|c| c[1].parse().ok()).unwrap_or(0);
    let size = SIZE.captures(title).map(|c| parse_size(&c[1])).unwrap_or(0);
    let mut h = hit("torrentio", str_of(stream, "infoHash"), title.lines().next()?, size, seeders, 0)?;
    h.exact = true;
    h.file_index = stream.get("fileIdx").and_then(Value::as_u64).map(|i| i as usize);
    h.trackers = stream
        .get("sources")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str()?.strip_prefix("tracker:").map(str::to_owned))
        .collect();
    Some(h)
}

async fn torrentio(http: &reqwest::Client, s: &Search) -> Hits {
    let imdb = s.imdb.as_ref().ok_or(Fail::Skipped("needs an IMDb id"))?;
    let path = match s.episode {
        Some((se, ep)) => format!("series/{imdb}:{se}:{ep}"),
        None => format!("movie/{imdb}"),
    };
    let v = get_json(http.get(format!("https://torrentio.strem.fun/sort=seeders/stream/{path}.json"))).await?;
    Ok(v.get("streams").and_then(Value::as_array).into_iter().flatten().filter_map(torrentio_hit).collect())
}

// ---------- Stremio add-ons (Comet, TorrentsDB, MediaFusion, Peerflix) ----------

/// One Stremio stream of a torrent add-on → hit. The release name is the first line that does not start with an
/// emoji (Comet: "📄 name"), else `behaviorHints.filename`; seeders after 👤 (MediaFusion also ▶️), size after 💾/📦.
fn stremio_hit(source: &'static str, stream: &Value) -> Option<Hit> {
    static SEEDS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:👤|▶️)\s*(\d+)").expect("static regex"));
    static SIZE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:💾|📦)\s*([\d.,]+\s*[KMGT]i?B)").expect("static regex"));
    let text = stream.get("description").or(stream.get("title")).and_then(Value::as_str).unwrap_or_default();
    let hints = stream.get("behaviorHints");
    let filename = hints.and_then(|h| h.get("filename")).and_then(Value::as_str).unwrap_or_default();
    let name = text
        .lines()
        .map(|l| l.trim().trim_start_matches('📄').trim())
        .find(|l| l.chars().next().is_some_and(|c| c.is_alphanumeric() || "[(【".contains(c)))
        .unwrap_or(filename);
    let seeders: Option<u64> = SEEDS.captures(text).and_then(|c| c[1].parse().ok());
    let size = hints
        .and_then(|h| h.get("videoSize"))
        .map(num)
        .filter(|n| *n > 0)
        .or_else(|| SIZE.captures(text).map(|c| parse_size(&c[1])))
        .unwrap_or(0);
    let mut h = hit(source, str_of(stream, "infoHash"), name, size, seeders.unwrap_or(0), 0)?;
    h.exact = true;
    // Comet's numbers come from debrid caches: 0 there means "not known".
    h.no_seeds = seeders.is_none_or(|n| n == 0 && source.starts_with("comet"));
    h.languages = stated_languages(text);
    h.file_index = stream.get("fileIdx").and_then(Value::as_u64).map(|i| i as usize);
    h.trackers = stream
        .get("sources")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str()?.strip_prefix("tracker:").map(str::to_owned))
        .collect();
    Some(h)
}

fn stremio_hits(source: &'static str, v: &Value) -> Vec<Hit> {
    v.get("streams").and_then(Value::as_array).into_iter().flatten().filter_map(|s| stremio_hit(source, s)).collect()
}

/// `GET <base>/stream/movie/<imdb>.json` or `/stream/series/<imdb>:<s>:<e>.json`.
async fn stremio(http: &reqwest::Client, s: &Search, source: &'static str, base: &str) -> Hits {
    let imdb = s.imdb.as_ref().ok_or(Fail::Skipped("needs an IMDb id"))?;
    let path = match (s.kind, s.episode) {
        (_, Some((se, ep))) => format!("series/{imdb}:{se}:{ep}"),
        (Some(MediaType::Tv), None) => return Err(Fail::Skipped("needs an episode")),
        _ => format!("movie/{imdb}"),
    };
    Ok(stremio_hits(source, &get_json(http.get(format!("{base}/stream/{path}.json"))).await?))
}

// ---------- JSON APIs ----------

fn bitsearch_hits(v: &Value) -> Vec<Hit> {
    v.get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| hit("bitsearch", str_of(t, "infohash"), str_of(t, "title"), num(&t["size"]), num(&t["seeders"]), num(&t["leechers"])))
        .collect()
}

async fn bitsearch(http: &reqwest::Client, s: &Search) -> Hits {
    let mut out = Vec::new();
    for q in &s.texts {
        // The site moves between domains (bitsearch.to answers with a redirect); redirects are followed.
        let req = http.get("https://bitsearch.to/api/v1/search").query(&[("q", q.as_str()), ("sort", "seeders")]);
        out.extend(bitsearch_hits(&get_json(req).await?));
    }
    Ok(out)
}

fn torrents_csv_hits(v: &Value) -> Vec<Hit> {
    v.get("torrents")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| {
            let mut h = hit("torrentscsv", str_of(t, "infohash"), str_of(t, "name"), num(&t["size_bytes"]), num(&t["seeders"]), num(&t["leechers"]))?;
            h.added = unix(&t["created_unix"]);
            Some(h)
        })
        .collect()
}

async fn torrents_csv(http: &reqwest::Client, s: &Search) -> Hits {
    let mut out = Vec::new();
    for q in &s.texts {
        out.extend(torrents_csv_hits(&get_json(http.get("https://torrents-csv.com/service/search").query(&[("q", q.as_str()), ("size", "50")])).await?));
    }
    Ok(out)
}

/// Zilean (DMM hash list): no seeders, but languages and the IMDb id per entry.
fn zilean_hits(v: &Value, imdb: Option<&str>) -> Vec<Hit> {
    static SITE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^\s*www[ .][^-]{2,40}?\s+-\s+").expect("static regex"));
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| {
            let name = SITE.replace(str_of(t, "raw_title"), "");
            let mut h = hit("zilean", str_of(t, "info_hash"), &name, num(&t["size"]), 0, 0)?;
            h.no_seeds = true;
            h.exact = imdb.is_some_and(|i| i == str_of(t, "imdb_id"));
            h.languages = t.get("languages").and_then(Value::as_array).into_iter().flatten().filter_map(|l| lang_code(l.as_str()?)).collect();
            Some(h)
        })
        .collect()
}

async fn zilean(http: &reqwest::Client, s: &Search) -> Hits {
    let mut out = Vec::new();
    for q in s.names.iter().take(2) {
        let mut params = vec![("Query", q.clone())];
        match (s.kind, s.episode) {
            (Some(MediaType::Movie), _) => params.push(("Category", "movie".into())),
            (Some(MediaType::Tv), ep) => {
                params.push(("Category", "tvSeries".into()));
                if let Some((se, e)) = ep {
                    params.extend([("Season", se.to_string()), ("Episode", e.to_string())]);
                }
            }
            _ => {}
        }
        let v = get_json(http.get("https://zileanfortheweebs.midnightignite.me/dmm/filtered").query(&params)).await?;
        out.extend(zilean_hits(&v, s.imdb.as_deref()));
    }
    Ok(out)
}

/// SubsPlease: one entry per release with a magnet per resolution; the size is the magnet's `xl`.
fn subsplease_hits(v: &Value) -> Vec<Hit> {
    let mut out = Vec::new();
    for release in v.as_object().into_iter().flat_map(|o| o.values()) {
        for d in release.get("downloads").and_then(Value::as_array).into_iter().flatten() {
            let link = str_of(d, "magnet");
            let Some((hash, name, trackers)) = parse_magnet(link) else { continue };
            let size = url::Url::parse(link).ok().and_then(|u| u.query_pairs().find(|(k, _)| k == "xl").and_then(|(_, v)| v.parse().ok())).unwrap_or(0);
            let name = name.unwrap_or_else(|| format!("{} - {} ({}p)", str_of(release, "show"), str_of(release, "episode"), str_of(d, "res")));
            if let Some(mut h) = hit("subsplease", &hash, &name, size, 0, 0) {
                h.no_seeds = true;
                h.trackers = trackers;
                h.added = rss_date(str_of(release, "release_date"));
                out.push(h);
            }
        }
    }
    out
}

async fn subsplease(http: &reqwest::Client, s: &Search) -> Hits {
    let mut out = Vec::new();
    for q in anime_queries(s) {
        let v = get_json(http.get("https://subsplease.org/api/").query(&[("f", "search"), ("tz", "UTC"), ("s", q.as_str())])).await?;
        out.extend(subsplease_hits(&v));
    }
    Ok(out)
}

// ---------- RSS feeds ----------

/// TorrentDownloads: everything in tags; the feed's category filter does not work, so other kinds are dropped here.
fn torrentdownloads_hits(xml: &str) -> Vec<Hit> {
    chunks(xml, "<item>")
        .filter_map(|item| {
            let category = xml_tag(item, "category").unwrap_or_default();
            if ["Music", "Games", "Software", "Books", "Pictures"].contains(&category.as_str()) || category.to_lowercase().contains("xxx") {
                return None;
            }
            let n = |t: &str| xml_tag(item, t).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
            let mut h = hit("torrentdownloads", &xml_tag(item, "info_hash")?, &xml_tag(item, "title")?, n("size"), n("seeders"), n("leechers"))?;
            h.added = xml_tag(item, "pubDate").and_then(|d| rss_date(&d));
            Some(h)
        })
        .collect()
}

async fn torrentdownloads(http: &reqwest::Client, s: &Search) -> Hits {
    let mut out = Vec::new();
    for q in &s.texts {
        let xml = text(http.get("https://www.torrentdownloads.pro/rss.xml").query(&[("type", "search"), ("search", q.as_str())])).await?;
        out.extend(torrentdownloads_hits(&xml));
    }
    Ok(out)
}

/// TorrentDownload.info: "Size: 556.65 MB Seeds: 11 , Peers: 2 Hash: 43F4…" in the description.
fn torrentdownload_hits(xml: &str) -> Vec<Hit> {
    static DESC: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"Size:\s*([\d.,]+\s*[KMGT]?i?B)\s*Seeds:\s*([\d,]+)\s*,\s*Peers:\s*([\d,]+)\s*Hash:\s*([0-9A-Fa-f]{40})").expect("static regex"));
    chunks(xml, "<item>")
        .filter_map(|item| {
            let desc = xml_tag(item, "description")?;
            let c = DESC.captures(&desc)?;
            let n = |i: usize| c[i].replace(',', "").parse().unwrap_or(0);
            let mut h = hit("torrentdownload", &c[4], &xml_tag(item, "title")?, parse_size(&c[1]), n(2), n(3))?;
            h.added = xml_tag(item, "pubDate").and_then(|d| rss_date(&d));
            Some(h)
        })
        .collect()
}

async fn torrentdownload(http: &reqwest::Client, s: &Search) -> Hits {
    let mut out = Vec::new();
    for q in &s.texts {
        out.extend(torrentdownload_hits(&text(http.get("https://www.torrentdownload.info/feed").query(&[("q", q.as_str())])).await?));
    }
    Ok(out)
}

/// BT4G: the magnet is the link, the description is "name<br>size<br>category<br>hash". No seeders.
fn bt4g_hits(xml: &str) -> Vec<Hit> {
    chunks(xml, "<item>")
        .filter_map(|item| {
            let (hash, _, trackers) = parse_magnet(&xml_tag(item, "link")?)?;
            let desc = xml_tag(item, "description").unwrap_or_default();
            let size = desc.split("<br>").nth(1).map(parse_size).unwrap_or(0);
            let mut h = hit("bt4g", &hash, &xml_tag(item, "title")?, size, 0, 0)?;
            h.no_seeds = true;
            h.trackers = trackers;
            h.added = xml_tag(item, "pubDate").and_then(|d| rss_date(&d));
            Some(h)
        })
        .collect()
}

async fn bt4g(http: &reqwest::Client, s: &Search) -> Hits {
    let mut out = Vec::new();
    for q in &s.texts {
        out.extend(bt4g_hits(&text(http.get("https://bt4gprx.com/search").query(&[("q", q.as_str()), ("page", "rss")])).await?));
    }
    Ok(out)
}

/// TokyoTosho: magnet and "Size: 25.47MB" in the description; only anime categories (no hentai, "Other", music).
fn tokyotosho_hits(xml: &str) -> Vec<Hit> {
    static SIZE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Size:\s*([\d.,]+\s*[KMGT]i?B)").expect("static regex"));
    chunks(xml, "<item>")
        .filter_map(|item| {
            let category = xml_tag(item, "category").unwrap_or_default();
            if !["Anime", "Batch", "Raws", "Non-English", "Drama"].contains(&category.as_str()) {
                return None;
            }
            let desc = xml_tag(item, "description").unwrap_or_default();
            let (hash, _, trackers) = parse_magnet(&magnet_in(&desc)?)?;
            let size = SIZE.captures(&desc).map(|c| parse_size(&c[1])).unwrap_or(0);
            let mut h = hit("tokyotosho", &hash, &xml_tag(item, "title")?, size, 0, 0)?;
            h.no_seeds = true;
            h.trackers = trackers;
            h.added = xml_tag(item, "pubDate").and_then(|d| rss_date(&d));
            Some(h)
        })
        .collect()
}

async fn tokyotosho(http: &reqwest::Client, s: &Search) -> Hits {
    let mut out = Vec::new();
    for q in anime_queries(s) {
        out.extend(tokyotosho_hits(&text(http.get("https://www.tokyotosho.info/rss.php").query(&[("terms", q.as_str())])).await?));
    }
    Ok(out)
}

/// DMHY: the magnet is the enclosure; no size, no seeders. The feed is big, the first 60 items are enough.
fn dmhy_hits(xml: &str) -> Vec<Hit> {
    chunks(xml, "<item>")
        .take(60)
        .filter_map(|item| {
            let (hash, _, _) = parse_magnet(&tag_attr(item, "enclosure", "url")?)?;
            let mut h = hit("dmhy", &hash, &xml_tag(item, "title")?, 0, 0, 0)?;
            h.no_seeds = true;
            h.added = xml_tag(item, "pubDate").and_then(|d| rss_date(&d));
            Some(h)
        })
        .collect()
}

async fn dmhy(http: &reqwest::Client, s: &Search) -> Hits {
    let mut out = Vec::new();
    for q in anime_queries(s) {
        out.extend(dmhy_hits(&text(http.get("https://dmhy.org/topics/rss/rss.xml").query(&[("keyword", q.as_str())])).await?));
    }
    Ok(out)
}

/// Mikan: the infohash ends the episode link, the size is the enclosure's length.
fn mikan_hits(xml: &str) -> Vec<Hit> {
    chunks(xml, "<item>")
        .filter_map(|item| {
            let link = xml_tag(item, "link")?;
            let hash = link.rsplit('/').next()?;
            let size = tag_attr(item, "enclosure", "length").and_then(|l| l.parse().ok()).unwrap_or(0);
            let mut h = hit("mikan", hash, &xml_tag(item, "title")?, size, 0, 0)?;
            h.no_seeds = true;
            h.added = xml_tag(item, "pubDate").and_then(|d| date_secs(&d));
            Some(h)
        })
        .collect()
}

async fn mikan(http: &reqwest::Client, s: &Search) -> Hits {
    let mut out = Vec::new();
    for q in anime_queries(s) {
        out.extend(mikan_hits(&text(http.get("https://mikanani.me/RSS/Search").query(&[("searchstr", q.as_str())])).await?));
    }
    Ok(out)
}

// ---------- HTML pages ----------

/// TorrentKitty (DHT index): name, size (often wrong), magnet; no seeders.
fn torrentkitty_hits(html: &str) -> Vec<Hit> {
    static ROW: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?s)<td class="name">([^<]+)</td><td class="size">([^<]+)</td><td class="date">([^<]*)</td>.*?href="magnet:\?xt=urn:btih:([0-9A-Fa-f]{40})"#)
            .expect("static regex")
    });
    ROW.captures_iter(html)
        .filter_map(|c| {
            let mut h = hit("torrentkitty", &c[4], &unescape(&c[1]), parse_size(&c[2]), 0, 0)?;
            h.no_seeds = true;
            h.added = date_secs(&c[3]);
            Some(h)
        })
        .collect()
}

async fn torrentkitty(http: &reqwest::Client, s: &Search) -> Hits {
    let q = s.texts.first().ok_or(Fail::Skipped("no query"))?;
    Ok(torrentkitty_hits(&text(http.get(format!("https://www.torrentkitty.tv/search/{}/", path_enc(q)))).await?))
}

/// Rutor: rows `tr.gai` / `tr.tum` with magnet, title, size and seeders/leechers after `&nbsp;`.
fn rutor_hits(html: &str) -> Vec<Hit> {
    static TITLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"<a href="/torrent/[^"]+">([^<]+)</a>"#).expect("static regex"));
    static SIZE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"<td align="right">([\d.,]+&nbsp;[KMGT]?B)</td>"#).expect("static regex"));
    static PEERS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?s)class="green">.*?&nbsp;(\d+)</span>.*?class="red">&nbsp;(\d+)<"#).expect("static regex"));
    html.split("<tr class=\"")
        .skip(1)
        .filter(|row| row.starts_with("gai\"") || row.starts_with("tum\""))
        .filter_map(|row| {
            let (hash, _, _) = parse_magnet(&magnet_in(row)?)?;
            let title = unescape(&TITLE.captures(row)?[1]);
            let size = SIZE.captures(row).map(|c| parse_size(&c[1].replace("&nbsp;", " "))).unwrap_or(0);
            let (se, le) = PEERS.captures(row).map_or((0, 0), |c| (c[1].parse().unwrap_or(0), c[2].parse().unwrap_or(0)));
            hit("rutor", &hash, &title, size, se, le)
        })
        .collect()
}

async fn rutor(http: &reqwest::Client, s: &Search) -> Hits {
    let mut out = Vec::new();
    for q in &s.texts {
        out.extend(rutor_hits(&text(http.get(format!("https://rutor.info/search/0/0/000/0/{}", path_enc(q)))).await?));
    }
    Ok(out)
}

/// AnimeTosho: one `home_list_entry` per release with exact size, title, magnet and "Seeders: 1 / Leechers: 242".
fn animetosho_hits(html: &str) -> Vec<Hit> {
    static SIZE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Total file size: ([\d,]+) bytes").expect("static regex"));
    static TITLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"<div class="link"><a href="[^"]*">([^<]+)</a>"#).expect("static regex"));
    static PEERS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Seeders: (\d+) / Leechers: (\d+)").expect("static regex"));
    chunks(html, "home_list_entry")
        .filter_map(|row| {
            let (hash, _, trackers) = parse_magnet(&magnet_in(row)?)?;
            let title = unescape(&TITLE.captures(row)?[1]);
            let size = SIZE.captures(row).and_then(|c| c[1].replace(',', "").parse().ok()).unwrap_or(0);
            let peers = PEERS.captures(row);
            let n = |i: usize| peers.as_ref().and_then(|c| c[i].parse().ok()).unwrap_or(0);
            let mut h = hit("animetosho", &hash, &title, size, n(1), n(2))?;
            h.no_seeds = peers.is_none();
            h.trackers = trackers;
            Some(h)
        })
        .collect()
}

async fn animetosho(http: &reqwest::Client, s: &Search) -> Hits {
    let mut out = Vec::new();
    for q in anime_queries(s) {
        out.extend(animetosho_hits(&text(http.get("https://animetosho.org/search").query(&[("q", q.as_str())])).await?));
    }
    Ok(out)
}

/// Anime sites are asked with the bare titles (romaji first), not with "S01E05": for an episode the first title
/// plus its number ("Sousou no Frieren 05"; their search lists newest first, older episodes need the number).
fn anime_queries(s: &Search) -> Vec<String> {
    let list = if s.names.is_empty() { &s.texts } else { &s.names };
    match (s.episode, list.first()) {
        (Some((_, e)), Some(first)) => vec![format!("{first} {e:02}"), first.clone()],
        _ => list.iter().take(2).cloned().collect(),
    }
}

// ---------- orchestration ----------

#[derive(Clone, Serialize)]
struct SourceStatus {
    id: &'static str,
    /// "ok" | "blocked" | "error" | "skipped" | "off" (turned off in the settings)
    status: &'static str,
    count: usize,
    ms: u64,
    error: Option<String>,
}

type SourceFuture<'a> = Pin<Box<dyn Future<Output = Hits> + Send + 'a>>;

fn source<'a>(id: &'static str, http: &'a reqwest::Client, s: &'a Search) -> SourceFuture<'a> {
    match id {
        "torrentio" => Box::pin(torrentio(http, s)),
        "comet" => Box::pin(stremio(http, s, "comet", "https://comet.feels.legal")),
        "cometweebs" => Box::pin(stremio(http, s, "cometweebs", "https://cometfortheweebs.midnightignite.me")),
        "torrentsdb" => Box::pin(stremio(http, s, "torrentsdb", "https://torrentsdb.com")),
        "mediafusion" => Box::pin(stremio(http, s, "mediafusion", "https://mediafusionfortheweebs.midnightignite.me")),
        "peerflix" => Box::pin(stremio(http, s, "peerflix", "https://peerflix.mov")),
        "knaben" => Box::pin(knaben(http, s)),
        "bitsearch" => Box::pin(bitsearch(http, s)),
        "yts" => Box::pin(yts(http, s)),
        "eztv" => Box::pin(eztv(http, s)),
        "piratebay" => Box::pin(piratebay(http, s)),
        "therarbg" => Box::pin(therarbg(http, s)),
        "limetorrents" => Box::pin(limetorrents(http, s)),
        "1337x" => Box::pin(x1337(http, s)),
        "torrentscsv" => Box::pin(torrents_csv(http, s)),
        "torrentdownloads" => Box::pin(torrentdownloads(http, s)),
        "torrentdownload" => Box::pin(torrentdownload(http, s)),
        "zilean" => Box::pin(zilean(http, s)),
        "bt4g" => Box::pin(bt4g(http, s)),
        "torrentkitty" => Box::pin(torrentkitty(http, s)),
        "rutor" => Box::pin(rutor(http, s)),
        "nyaa" => Box::pin(nyaa(http, s)),
        "subsplease" => Box::pin(subsplease(http, s)),
        "animetosho" => Box::pin(animetosho(http, s)),
        "tokyotosho" => Box::pin(tokyotosho(http, s)),
        "dmhy" => Box::pin(dmhy(http, s)),
        "mikan" => Box::pin(mikan(http, s)),
        _ => Box::pin(async { Err(Fail::Error("unknown source".into())) }),
    }
}

/// Anime hits name episodes "Show - 05", "Show S2 - 01-10" or "[29-38]": (season, first, last).
fn anime_episodes(name: &str) -> Option<(u32, u32, u32)> {
    static DASH: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?:^|[\s_\]])-\s*(\d{1,4})(?:\s*[-~]\s*(\d{1,4}))?(?:v\d)?(?:[\s_\[(.]|$)").expect("static regex"));
    static BRACKET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[(\d{1,4})(?:\s*[-~]\s*(\d{1,4}))?(?:v\d)?(?:\s*END)?\]").expect("static regex"));
    static SEASON: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)\b(?:s(\d{1,2})|season\s*(\d{1,2})|(\d{1,2})(?:st|nd|rd|th)\s+season)\b").expect("static regex"));
    let c = DASH.captures(name).or_else(|| BRACKET.captures(name))?;
    let first: u32 = c[1].parse().ok()?;
    let last = c.get(2).and_then(|m| m.as_str().parse().ok()).unwrap_or(first);
    let season = SEASON
        .captures(name)
        .and_then(|s| s.get(1).or(s.get(2)).or(s.get(3)).and_then(|m| m.as_str().parse().ok()))
        .unwrap_or(1);
    Some((season, first, last.max(first)))
}

/// Anime naming has no "S01E05": the title must appear somewhere in the name, the episode as an absolute number.
fn wanted_anime(s: &Search, h: &Hit) -> bool {
    let name = format!(" {} ", fold(&h.name));
    if !s.titles.iter().any(|t| name.contains(&format!(" {t} "))) {
        return false;
    }
    let Some((season, episode)) = s.episode else { return true };
    let r = release::parse(&h.name);
    if let (Some(rs), Some(re)) = (r.season, r.episode) {
        return rs == season && re == episode;
    }
    if let Some((se, a, z)) = anime_episodes(&h.name) {
        return se == season && (a..=z).contains(&episode);
    }
    r.pack && r.season == Some(season)
}

/// Whether a text hit really is the requested movie/episode (IMDb hits are trusted).
fn wanted(s: &Search, h: &Hit) -> bool {
    if h.exact || s.titles.is_empty() {
        return true;
    }
    if SOURCES.iter().any(|x| x.id == h.source && x.anime) {
        return wanted_anime(s, h);
    }
    let r = release::parse(&h.name);
    let title = fold(&r.title);
    if !s.titles.iter().any(|t| title == *t || title.ends_with(&format!(" {t}"))) {
        return false;
    }
    match s.episode {
        Some((season, episode)) => r.season == Some(season) && (r.episode == Some(episode) || r.pack),
        None => r.season.is_none() && s.year.is_none_or(|y| r.year.is_none_or(|ry| ry.abs_diff(y) <= 1)),
    }
}

/// Normalized title without accents, so "Amélie" and "Amelie" compare equal.
pub fn fold(title: &str) -> String {
    let plain: String = title
        .chars()
        .filter(|c| !matches!(c, '\'' | '’'))
        .map(|c| match c.to_lowercase().next().unwrap_or(c) {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' | 'ø' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            c => c,
        })
        .collect();
    normalize(&plain.replace('ß', "ss").replace('&', " and "))
}

/// Runs all sources (or takes the 5-minute cache) and merges their hits.
async fn collect(st: &AppState, key: String, s: Search) -> Arc<Found> {
    let searcher = &st.torrents.searcher;
    if let Some((at, found)) = searcher.cache.lock().unwrap().get(&key)
        && at.elapsed() < CACHE_TTL
    {
        return found.clone();
    }
    let off = crate::torrent::load_settings(st).map(|x| x.sources_off).unwrap_or_else(|_| default_off());
    let resting: HashMap<&'static str, u64> = {
        let mut cd = searcher.cooldown.lock().unwrap();
        cd.retain(|_, until| *until > Instant::now());
        cd.iter().map(|(id, until)| (*id, until.saturating_duration_since(Instant::now()).as_secs() / 60 + 1)).collect()
    };
    let s = &s;
    let jobs = SOURCES.iter().map(|info| {
        let id = info.id;
        let off = off.iter().any(|o| o == id);
        let resting = resting.get(id).copied();
        async move {
            if off {
                return (id, Err(Fail::Skipped("turned off in the settings")), 0, true);
            }
            if info.anime && !s.anime {
                return (id, Err(Fail::Skipped("anime only")), 0, false);
            }
            if let Some(min) = resting {
                return (id, Err(Fail::Resting(min)), 0, false);
            }
            let start = Instant::now();
            let found = tokio::time::timeout(SOURCE_TIMEOUT, source(id, &searcher.http, s)).await.unwrap_or(Err(Fail::Error("timeout".into())));
            (id, found, start.elapsed().as_millis() as u64, false)
        }
    });
    let mut statuses = Vec::new();
    let mut hits = Vec::new();
    for (id, found, ms, off) in futures_util::future::join_all(jobs).await {
        let (status, count, error) = match found {
            Ok(list) => {
                let list: Vec<Hit> = list.into_iter().filter(|h| wanted(s, h)).collect();
                let n = list.len();
                hits.extend(list);
                ("ok", n, None)
            }
            Err(Fail::Resting(min)) => ("blocked", 0, Some(format!("refused a moment ago, asked again in {min} min"))),
            Err(Fail::Skipped(why)) => (if off { "off" } else { "skipped" }, 0, Some(why.to_owned())),
            Err(Fail::Blocked(why)) => {
                // Rate-limit friendly: a site that refused is left alone for a while.
                searcher.cooldown.lock().unwrap().insert(id, Instant::now() + COOLDOWN);
                ("blocked", 0, Some(why))
            }
            Err(Fail::Error(why)) => ("error", 0, Some(why)),
        };
        statuses.push(SourceStatus { id, status, count, ms, error });
    }
    let found = Arc::new(Found { torrents: release::merge(hits, now_ms() / 1000), statuses });
    let mut cache = searcher.cache.lock().unwrap();
    cache.retain(|_, (at, _)| at.elapsed() < CACHE_TTL);
    cache.insert(key, (Instant::now(), found.clone()));
    found
}

/// Filters (`language`, `res`, `source` …; not `lang`: that is the TMDB language of every request), facets and ranking (`mode=fast|quality`) of one request.
fn respond(st: &AppState, found: &Found, q: &HashMap<String, String>) -> ApiResult<Json<Value>> {
    let query = release::Query::from_params(q).map_err(ApiError::bad_request)?;
    let mode = match q.get("mode").map(String::as_str) {
        None => None,
        Some(m @ ("fast" | "quality")) => Some(m),
        Some(_) => return Err(ApiError::bad_request("'mode' must be 'fast' or 'quality'")),
    };
    let filter = load_settings(st)?.filter;
    let (mut results, facets) = release::filter_with_facets(&found.torrents, &query);
    release::rank(&mut results, &release::Ranking { filter: &filter, mode });
    let matched = results.len();
    results.truncate(MAX_RESULTS);
    Ok(Json(json!({
        "results": results, "sources": found.statuses, "facets": facets,
        "total": found.torrents.len(), "matched": matched,
    })))
}

/// `GET /api/torrents/search?q=`
pub async fn free(State(st): St, Query(q): Params) -> ApiResult<Json<Value>> {
    let text = q.get("q").map(|s| s.trim()).unwrap_or_default();
    if text.len() < 2 {
        return Err(ApiError::bad_request("'q' is required (at least 2 characters)"));
    }
    // `anime=1`: the free search also asks the anime sites.
    let anime = q.get("anime").is_some_and(|v| v == "1" || v == "true");
    let search = Search { texts: vec![text.to_owned()], names: vec![text.to_owned()], anime, ..Search::default() };
    let found = collect(&st, format!("q:{}:{anime}", text.to_lowercase()), search).await;
    respond(&st, &found, &q)
}

/// `GET /api/torrents/sources`: every search source with its kind and whether it is on.
pub async fn list_sources(State(st): St) -> ApiResult<Json<Value>> {
    let off = crate::torrent::load_settings(&st)?.sources_off;
    let list: Vec<Value> = SOURCES
        .iter()
        .map(|s| {
            let mut v = serde_json::to_value(s).unwrap_or_default();
            v["on"] = json!(!off.iter().any(|o| o == s.id));
            v
        })
        .collect();
    Ok(Json(json!(list)))
}

/// `GET /api/torrents/:type/:id?s=&e=`: torrents of one movie / episode.
pub async fn by_title(State(st): St, Path((ty, id)): Path<(String, String)>, Query(q): Params) -> ApiResult<Json<Value>> {
    let t = title_ref(&ty, &id, &q)?;
    let found = search_title(&st, &t).await?;
    respond(&st, &found, &q)
}

/// All torrents of one movie / episode (merged, unfiltered); also what the auto-download job searches.
pub async fn search_title(st: &AppState, t: &TitleRef) -> ApiResult<Arc<Found>> {
    let params = [("append_to_response", "external_ids,alternative_titles".to_owned())];
    let v = st.tmdb.get(&format!("/{}/{}", t.kind.as_str(), t.id), &params, Some(TTL_TITLE)).await?;
    // Anime: TMDB animation from Japan, China or Korea. Their release names use the romaji title.
    let animation = v.get("genres").and_then(Value::as_array).into_iter().flatten().any(|g| g["id"].as_u64() == Some(16));
    let anime = animation && matches!(v["original_language"].as_str(), Some("ja" | "zh" | "ko"));
    let romaji: Vec<String> = if anime {
        let alt = v.pointer("/alternative_titles/results").or(v.pointer("/alternative_titles/titles"));
        alt.and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|a| matches!(a["iso_3166_1"].as_str(), Some("JP" | "US" | "GB")))
            .filter_map(|a| a["title"].as_str())
            .filter(|n| n.is_ascii())
            .take(3)
            .map(str::to_owned)
            .collect()
    } else {
        Vec::new()
    };
    let pick = |keys: [&str; 2]| keys.iter().find_map(|k| v.get(*k).and_then(Value::as_str)).unwrap_or_default().to_owned();
    let (local, original) = (pick(["title", "name"]), pick(["original_title", "original_name"]));
    let year = pick(["release_date", "first_air_date"]).get(..4).and_then(|y| y.parse().ok());
    let imdb = [v.get("imdb_id"), v.pointer("/external_ids/imdb_id")]
        .into_iter()
        .flatten()
        .find_map(Value::as_str)
        .filter(|s| s.starts_with("tt"))
        .map(str::to_owned);

    let mut names = vec![original];
    if !names.contains(&local) {
        names.push(local);
    }
    names.retain(|n| !n.is_empty());
    let suffix = match t.episode {
        Some((s, e)) => format!("S{s:02}E{e:02}"),
        None => year.map(|y: u32| y.to_string()).unwrap_or_default(),
    };
    // Romaji alternative titles come first for the anime sites; Latin-script names before others.
    let mut bare: Vec<String> = romaji.clone();
    for n in &names {
        if !bare.contains(n) {
            bare.push(n.clone());
        }
    }
    bare.sort_by_key(|n| !n.is_ascii());
    let search = Search {
        texts: names.iter().map(|n| format!("{n} {suffix}").trim().to_owned()).collect(),
        titles: names.iter().chain(&romaji).map(|n| fold(n)).filter(|n| !n.is_empty()).collect(),
        names: bare,
        imdb,
        kind: Some(t.kind),
        episode: t.episode,
        year,
        anime,
    };
    Ok(collect(st, t.key(), search).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_upload_dates() {
        assert_eq!(date_secs("1970-01-02T00:00:00+00:00"), Some(86_400));
        assert_eq!(date_secs("2024-03-01 10:00"), Some(1_709_251_200));
        assert_eq!(date_secs("soon"), None);
    }

    #[test]
    fn parses_1337x_pages() {
        let search = r#"<tr>
  <td class="coll-1 name"><a href="/sub/2/0/" class="icon"><i class="flaticon-hd"></i></a><a href="/torrent/5555555/Ubuntu-24-04-LTS-Desktop-amd64/">Ubuntu 24.04 LTS Desktop amd64</a></td>
  <td class="coll-2 seeds">981</td>
  <td class="coll-3 leeches">37</td>
  <td class="coll-date">Apr. 25th '24</td>
  <td class="coll-4 size mob-uploader">5.7 GB<span class="seeds">981</span></td>
</tr>"#;
        let rows = x1337_rows(search);
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].0.as_str(), rows[0].1.as_str(), rows[0].2, rows[0].3), ("/torrent/5555555/Ubuntu-24-04-LTS-Desktop-amd64/", "Ubuntu 24.04 LTS Desktop amd64", 981, 37));
        assert_eq!(rows[0].4, parse_size("5.7 GB"));
        let detail = r#"<li><a class="btn btn-magnet" href="magnet:?xt=urn:btih:deadbeefdeadbeefdeadbeefdeadbeefdeadbeef&dn=Ubuntu&tr=udp%3A%2F%2Ftracker.opentrackr.org%3A1337%2Fannounce" onclick="x">Magnet</a></li>"#;
        let (hash, _, trackers) = parse_magnet(&first_magnet(detail).unwrap()).unwrap();
        assert_eq!(hash, "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef");
        assert_eq!(trackers, vec!["udp://tracker.opentrackr.org:1337/announce"]);
    }

    #[test]
    fn parses_limetorrents_rows() {
        let html = r##"<tr bgcolor="#F4F4F4"><td class="tdleft"><div class="tt-name"><a href="http://itorrents.net/torrent/43F4001DE4AB25D521C63684E2B69804193ED9D9.torrent?title=Sintel-2010-1080p" rel="nofollow" class="csprite_dl14"></a><a href="/Sintel-2010-1080p-torrent-19099188.html">Sintel 2010 1080p WEB H264-CLASSiCALHD</a></div><div class="tt-options"></div></td><td class="tdnormal">1 Year+ - in Movies</a></td><td class="tdnormal">556.65 MB</td><td class="tdseed">1,204</td><td class="tdleech">4</td><td class="tdright"><div class="hb1"></div></td></tr>"##;
        let rows = lime_rows(html);
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].infohash.as_str(), rows[0].name.as_str(), rows[0].seeders, rows[0].leechers), ("43f4001de4ab25d521c63684e2b69804193ed9d9", "Sintel 2010 1080p WEB H264-CLASSiCALHD", 1204, 4));
    }

    #[test]
    fn parses_torrentio_streams() {
        let v = json!({
            "title": "Sintel 2010 720p BluRay\n👤 30 💾 136.47 MB ⚙️ YTS",
            "infoHash": "a779fa758f2229586eebc2e5960eb32fe5659596", "fileIdx": 2,
            "sources": ["tracker:udp://a.example:1/announce", "dht:a779"]
        });
        let h = torrentio_hit(&v).unwrap();
        assert_eq!((h.name.as_str(), h.seeders, h.file_index, h.exact), ("Sintel 2010 720p BluRay", 30, Some(2), true));
        assert_eq!((h.size, h.trackers.as_slice()), (parse_size("136.47 MB"), &["udp://a.example:1/announce".to_owned()][..]));
    }

    const FEEDS: &str = include_str!("../tests/fixtures/tsearch/feeds.xml");
    const PAGES: &str = include_str!("../tests/fixtures/tsearch/pages.html");

    /// The part of a fixture file after "=== name" up to the next section.
    fn section<'a>(file: &'a str, name: &str) -> &'a str {
        let head = file.find(&format!("=== {name}")).expect("fixture section");
        let start = head + file[head..].find('\n').expect("section line") + 1;
        file[start..].split("=== ").next().unwrap_or_default()
    }

    fn fixture_json(name: &str) -> Value {
        let file = match name {
            "comet" => include_str!("../tests/fixtures/tsearch/comet.json"),
            "stremio" => include_str!("../tests/fixtures/tsearch/stremio_other.json"),
            _ => include_str!("../tests/fixtures/tsearch/json_apis.json"),
        };
        serde_json::from_str(file).expect("fixture json")
    }

    #[test]
    fn parses_comet_streams_with_flags_and_unknown_seeders() {
        let hits = stremio_hits("comet", &fixture_json("comet"));
        assert_eq!(hits.len(), 3);
        let multi = &hits[0];
        assert_eq!(multi.name, "Fight.Club.1999.MULTi.VFF.2160p.HYBRID.UHD.BluRay.REMUX.DV.HDR10+.DTS.HD.MA.5.1.HEVC-TTK.mkv");
        assert_eq!((multi.size, multi.seeders, multi.no_seeds, multi.exact, multi.file_index), (77_824_083_783, 0, true, true, Some(0)));
        assert_eq!(multi.languages, ["multi", "fr"]);
        assert_eq!(hits[1].languages, ["de"]);
        assert_eq!((hits[2].seeders, hits[2].no_seeds, hits[2].size), (23, false, parse_size("9.8 GB")));
    }

    #[test]
    fn parses_other_stremio_addons() {
        let v = fixture_json("stremio");
        let tdb = &stremio_hits("torrentsdb", &v["torrentsdb"])[0];
        assert_eq!(tdb.name, "Fight Club (1999) [REPACK] [2160p] [BluRay] [x265] [10bit] [5.1]");
        assert_eq!((tdb.seeders, tdb.size, tdb.no_seeds), (310, parse_size("6.49 GB"), false));
        assert_eq!(tdb.trackers, ["udp://tracker.opentrackr.org:1337/announce"]);
        let mf = &stremio_hits("mediafusion", &v["mediafusion"])[0];
        assert!(mf.name.starts_with("Fight Club [1999, BDRemux 2160p"));
        assert_eq!((mf.seeders, mf.size), (43, 115_796_361_216));
        assert_eq!(mf.languages, ["en", "ru"]);
        let pf = &stremio_hits("peerflix", &v["peerflix"])[0];
        assert!(pf.name.starts_with("El club de la Lucha [4K"));
        assert_eq!((pf.seeders, pf.size, pf.languages.as_slice()), (12, parse_size("17.63 GB"), &["es"][..]));
    }

    #[test]
    fn parses_json_apis() {
        let v = fixture_json("apis");
        let bs = &bitsearch_hits(&v["bitsearch"])[0];
        assert_eq!((bs.infohash.as_str(), bs.seeders, bs.leechers, bs.size), ("a779fa758f2229586eebc2e5960eb32fe5659596", 751, 495, 143_156_284));
        let csv = &torrents_csv_hits(&v["torrentscsv"])[0];
        assert_eq!((csv.name.as_str(), csv.seeders, csv.added), ("Sintel", 192, Some(1_682_871_531)));
        let z = zilean_hits(&v["zilean"], Some("tt0082096"));
        assert_eq!(z[0].name, "Sintel 2010 1080p WEB H264-CLASSiCALHD");
        assert!(z[0].no_seeds && !z[0].exact);
        assert!(z[1].exact);
        assert_eq!(z[1].languages, ["en", "de", "ru"]);
        let sp = &subsplease_hits(&v["subsplease"])[0];
        assert_eq!(sp.name, "[SubsPlease] Sousou no Frieren S2 (01-10) (480p) [Batch]");
        assert_eq!((sp.size, sp.no_seeds, sp.trackers.len()), (3_895_551_825, true, 2));
        assert_eq!(sp.infohash.len(), 40);
    }

    #[test]
    fn parses_rss_feeds() {
        let td = torrentdownloads_hits(section(FEEDS, "torrentdownloads"));
        assert_eq!(td.len(), 2);
        assert_eq!((td[0].name.as_str(), td[0].size, td[0].seeders, td[0].leechers), ("Sintel", 129_302_391, 109, 74));
        assert_eq!(td[0].added, date_secs("2018-10-26"));
        let info = &torrentdownload_hits(section(FEEDS, "torrentdownload.info"))[0];
        assert_eq!((info.infohash.as_str(), info.seeders, info.leechers), ("43f4001de4ab25d521c63684e2b69804193ed9d9", 11, 2));
        assert_eq!((info.size, info.added), (parse_size("556.65 MB"), date_secs("2025-07-22")));
        let bt = &bt4g_hits(section(FEEDS, "bt4g"))[0];
        assert_eq!((bt.name.as_str(), bt.size, bt.no_seeds), ("LibreUniMatchl-flow-sintel", parse_size("29.00MB"), true));
        assert_eq!(bt.added, date_secs("2026-11-20"));
        // "Other" (here an adult doujin) is dropped, the anime entry stays.
        let tt = tokyotosho_hits(section(FEEDS, "tokyotosho"));
        assert_eq!(tt.len(), 1);
        assert_eq!((tt[0].name.as_str(), tt[0].size), ("Sousou no Frieren S01 [1080p VVC xHE-AAC]", parse_size("5.29GB")));
        let dm = &dmhy_hits(section(FEEDS, "dmhy"))[0];
        assert!(dm.name.contains("Sousou no Frieren][29-38]"));
        assert_eq!((dm.infohash.len(), dm.size, dm.added), (40, 0, date_secs("2026-10-17")));
        let mk = &mikan_hits(section(FEEDS, "mikan"))[0];
        assert_eq!((mk.infohash.as_str(), mk.size), ("8e4e8471b226e311eb7b3b77476eec7834052dbd", 8_160_437_760));
    }

    #[test]
    fn parses_html_pages() {
        let kitty = &torrentkitty_hits(section(PAGES, "torrentkitty"))[0];
        assert_eq!((kitty.infohash.as_str(), kitty.name.as_str(), kitty.no_seeds), ("08ada5a7a6183aae1e09d831df6748d566095a10", "Sintel", true));
        let ru = &rutor_hits(section(PAGES, "rutor"))[0];
        assert_eq!(ru.name, "Синтел / Sintel (2010) BDRip 720 от SuperMin");
        assert_eq!((ru.size, ru.seeders, ru.leechers), (parse_size("777.66 MB"), 0, 1));
        let at = &animetosho_hits(section(PAGES, "animetosho"))[0];
        assert!(at.name.ends_with("Frieren: Beyond Journey's End (2026) (Batch S02 S2)"));
        assert_eq!((at.size, at.seeders, at.leechers, at.no_seeds), (33_343_733_854, 1, 242, false));
    }

    #[test]
    fn anime_hits_match_by_title_and_absolute_episode() {
        let s = Search { titles: vec![fold("Sousou no Frieren"), fold("Frieren")], episode: Some((1, 5)), anime: true, ..Search::default() };
        let named = |name: &str| Hit { source: "subsplease", name: name.into(), ..Hit::default() };
        assert!(wanted(&s, &named("[SubsPlease] Sousou no Frieren - 05 (1080p) [1AB2C3D4].mkv")));
        assert!(wanted(&s, &named("[Group] Sousou no Frieren [01-28][1080p]")));
        assert!(!wanted(&s, &named("[SubsPlease] Sousou no Frieren - 06 (1080p)")));
        assert!(!wanted(&s, &named("[SubsPlease] Sousou no Frieren S2 - 05 (1080p)")));
        assert!(!wanted(&s, &named("[SubsPlease] Dungeon Meshi - 05 (1080p)")));
        assert_eq!(anime_episodes("Sousou no Frieren S2 (01-10) (480p) [Batch]"), None);
        assert_eq!(anime_episodes("Sousou no Frieren S2 - 01-10 (480p)"), Some((2, 1, 10)));
    }

    #[test]
    fn reads_rss_dates_and_flags() {
        assert_eq!(rss_date("Fri, 26 Oct 2018 14:37:42 +0200"), date_secs("2018-10-26"));
        assert_eq!(rss_date("Fri,20 Nov 2026 11:00:16 -0000"), date_secs("2026-11-20"));
        assert_eq!(stated_languages("🌎/🇬🇧/🇮🇳"), ["multi", "en", "hi"]);
        assert_eq!(stated_languages("🌎/💃🏻/🇩🇪"), ["multi", "es", "de"]);
        assert!(default_off().contains(&"dmhy".to_owned()) && !default_off().contains(&"comet".to_owned()));
        assert_eq!(SOURCES.len(), 27);
        assert_eq!(decode("Синтел".as_bytes()), "Синтел");
        assert_eq!(decode(&[0x53, 0xe9]), "Sé");
        assert_eq!(fix_mojibake("Ð¡Ð¸Ð½Ñ‚ÐµÐ» / Sintel (2010)"), "Синтел / Sintel (2010)");
        assert_eq!(fix_mojibake("[philosophy-raws][è¾›ç‰¹å°”][sintel]"), "[philosophy-raws][辛特尔][sintel]");
        assert_eq!(fix_mojibake("Die fabelhafte Welt der Amélie"), "Die fabelhafte Welt der Amélie");
        assert_eq!(fix_mojibake("Sintel"), "Sintel");
    }

    #[test]
    fn text_hits_must_be_the_requested_title() {
        let movie = Search { titles: vec![fold("Amélie"), fold("Die fabelhafte Welt der Amélie")], year: Some(2001), kind: Some(MediaType::Movie), ..Search::default() };
        let named = |name: &str| Hit { name: name.into(), ..Hit::default() };
        assert!(wanted(&movie, &named("Amelie.2001.German.DL.1080p.BluRay.x264")));
        assert!(wanted(&movie, &named("Die.fabelhafte.Welt.der.Amelie.2001.German.720p")));
        assert!(!wanted(&movie, &named("Amelie.1996.DVDRip")));
        assert!(!wanted(&movie, &named("Amelie.Returns.2001.1080p")));

        let ep = Search { titles: vec![fold("Dark")], episode: Some((1, 3)), kind: Some(MediaType::Tv), ..Search::default() };
        assert!(wanted(&ep, &named("Dark.S01E03.German.1080p.WEB.x264")));
        assert!(wanted(&ep, &named("Dark.S01.German.1080p.WEB.x264")));
        assert!(!wanted(&ep, &named("Dark.S01E04.German.1080p.WEB.x264")));
        assert!(!wanted(&ep, &named("Dark.Matter.S01E03.1080p.WEB.x264")));
        assert!(wanted(&ep, &Hit { name: "anything".into(), exact: true, ..Hit::default() }));
    }
}
