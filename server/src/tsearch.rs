//! Torrent search: all sources in parallel over the normal network (plain HTTPS to index sites,
//! no torrent traffic), merged and ranked by [`crate::release`]. The set of sources follows
//! github.com/melqtx/tork (MIT), plus a Torrentio lookup by IMDb id.

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
const MAX_RESULTS: usize = 120;

/// What to look for. Title lookups carry the IMDb id and the titles used to verify text hits.
#[derive(Default)]
pub struct Search {
    /// Free-text queries (first = main).
    pub texts: Vec<String>,
    pub imdb: Option<String>,
    pub kind: Option<MediaType>,
    pub episode: Option<(u32, u32)>,
    /// Normalized accepted titles; empty for a free search (nothing is filtered then).
    pub titles: Vec<String>,
    pub year: Option<u32>,
}

enum Fail {
    /// The source needs something this search does not have (e.g. an IMDb id).
    Skipped(&'static str),
    Blocked(String),
    Error(String),
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
}

impl Searcher {
    pub fn new() -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .user_agent(UA)
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| format!("http client: {e}"))?;
        Ok(Self { http, cache: Mutex::default() })
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
    let body = resp.text().await.map_err(fail)?;
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
}

fn hit(source: &'static str, hash: &str, name: &str, size: u64, seeders: u64, leechers: u64) -> Option<Hit> {
    Some(Hit {
        source,
        infohash: clean_hash(hash)?,
        name: name.trim().to_owned(),
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

// ---------- orchestration ----------

#[derive(Clone, Serialize)]
struct SourceStatus {
    id: &'static str,
    /// "ok" | "blocked" | "error" | "skipped"
    status: &'static str,
    count: usize,
    ms: u64,
    error: Option<String>,
}

type SourceFuture<'a> = Pin<Box<dyn Future<Output = Hits> + Send + 'a>>;

fn sources<'a>(http: &'a reqwest::Client, s: &'a Search) -> Vec<(&'static str, SourceFuture<'a>)> {
    vec![
        ("torrentio", Box::pin(torrentio(http, s))),
        ("knaben", Box::pin(knaben(http, s))),
        ("yts", Box::pin(yts(http, s))),
        ("eztv", Box::pin(eztv(http, s))),
        ("nyaa", Box::pin(nyaa(http, s))),
        ("piratebay", Box::pin(piratebay(http, s))),
        ("1337x", Box::pin(x1337(http, s))),
        ("therarbg", Box::pin(therarbg(http, s))),
        ("limetorrents", Box::pin(limetorrents(http, s))),
    ]
}

/// Whether a text hit really is the requested movie/episode (IMDb hits are trusted).
fn wanted(s: &Search, h: &Hit) -> bool {
    if h.exact || s.titles.is_empty() {
        return true;
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
    let jobs = sources(&searcher.http, &s).into_iter().map(|(id, job)| async move {
        let start = Instant::now();
        let found = tokio::time::timeout(SOURCE_TIMEOUT, job).await.unwrap_or(Err(Fail::Error("timeout".into())));
        (id, found, start.elapsed().as_millis() as u64)
    });
    let mut statuses = Vec::new();
    let mut hits = Vec::new();
    for (id, found, ms) in futures_util::future::join_all(jobs).await {
        let (status, count, error) = match found {
            Ok(list) => {
                let list: Vec<Hit> = list.into_iter().filter(|h| wanted(&s, h)).collect();
                let n = list.len();
                hits.extend(list);
                ("ok", n, None)
            }
            Err(Fail::Skipped(why)) => ("skipped", 0, Some(why.to_owned())),
            Err(Fail::Blocked(why)) => ("blocked", 0, Some(why)),
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

/// Filters (`lang`, `res`, `source` …), facets and ranking (`mode=fast|quality`) of one request.
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
    let found = collect(&st, format!("q:{}", text.to_lowercase()), Search { texts: vec![text.to_owned()], ..Search::default() }).await;
    respond(&st, &found, &q)
}

/// `GET /api/torrents/:type/:id?s=&e=`: torrents of one movie / episode.
pub async fn by_title(State(st): St, Path((ty, id)): Path<(String, String)>, Query(q): Params) -> ApiResult<Json<Value>> {
    let t = title_ref(&ty, &id, &q)?;
    let found = search_title(&st, &t).await?;
    respond(&st, &found, &q)
}

/// All torrents of one movie / episode (merged, unfiltered); also what the auto-download job searches.
pub async fn search_title(st: &AppState, t: &TitleRef) -> ApiResult<Arc<Found>> {
    let params = [("append_to_response", "external_ids".to_owned())];
    let v = st.tmdb.get(&format!("/{}/{}", t.kind.as_str(), t.id), &params, Some(TTL_TITLE)).await?;
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
    let search = Search {
        texts: names.iter().map(|n| format!("{n} {suffix}").trim().to_owned()).collect(),
        imdb,
        kind: Some(t.kind),
        episode: t.episode,
        titles: names.iter().map(|n| fold(n)).collect(),
        year,
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
