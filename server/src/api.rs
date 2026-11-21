use std::{collections::HashMap, sync::Arc};

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    autodl,
    db::Db,
    play, progress,
    scan::{self, Scans},
    sniff::Sniffer,
    stream::{self, Streams},
    error::{ApiError, ApiResult},
    model::{self, Details, Genre, Item, MediaType, Page, SeasonDetails},
    tmdb::{TTL_LIST, TTL_TITLE, Tmdb},
    torrent, tracker, tsearch, vpn,
};

#[derive(Debug, Deserialize)]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub no_ads: bool,
    pub movie: String,
    pub tv: String,
}

pub struct AppState {
    pub tmdb: Tmdb,
    pub db: Db,
    pub providers: Vec<Provider>,
    /// Only present inside the desktop app.
    pub sniffer: Option<Arc<dyn Sniffer>>,
    pub scans: Scans,
    pub streams: Streams,
    pub torrents: Arc<crate::torrent::Torrents>,
    pub autodl: autodl::JobState,
}

pub(crate) type St = State<Arc<AppState>>;
pub(crate) type Params = Query<HashMap<String, String>>;

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/status", get(status))
        .route("/api/home", get(home))
        .route("/api/list/{type}/{list}", get(list))
        .route("/api/genres/{type}", get(genres))
        .route("/api/search", get(search))
        .route("/api/title/{type}/{id}", get(title))
        .route("/api/title/{type}/{id}/season/{n}", get(season))
        .route("/api/sources/{type}/{id}", get(sources))
        .route("/api/watchlist", get(watchlist_list))
        .route("/api/watchlist/{type}/{id}", axum::routing::put(watchlist_put).delete(watchlist_delete))
        .route("/api/caps", get(scan::caps))
        .route("/api/scan/{type}/{id}", get(scan::get).post(scan::start))
        .route("/api/stream/{id}/master.m3u8", get(stream::master))
        .route("/api/stream/{id}/r", get(stream::resource))
        .route("/api/stream/{id}/prefetch", axum::routing::post(stream::prefetch))
        .route("/api/stream/{id}/buffer", get(stream::buffer))
        .route("/api/stream/{id}", axum::routing::delete(stream::delete))
        .route("/api/settings", get(scan::settings_get).put(scan::settings_put))
        .route("/api/progress/{type}/{id}", get(progress::of_title).put(progress::put))
        .route("/api/watched/{type}/{id}", axum::routing::put(progress::watched).delete(progress::unwatched))
        .route("/api/continue", get(progress::continue_list))
        .route("/api/continue/{type}/{id}", axum::routing::delete(progress::continue_hide))
        .route("/api/history", get(progress::history).delete(progress::history_clear))
        .route("/api/history/{entry}", axum::routing::delete(progress::history_delete))
        .route("/api/new-episodes", get(progress::new_episodes))
        .route("/api/vpn", get(vpn::status))
        .route("/api/vpn/kill", post(vpn::kill))
        .route("/api/vpn/release", post(vpn::release))
        .route("/api/torrents", get(torrent::list))
        .route("/api/torrents/search", get(tsearch::free))
        .route("/api/torrents/sources", get(tsearch::list_sources))
        .route("/api/torrents/start", post(torrent::start))
        .route("/api/torrents/settings", get(torrent::settings_get).put(torrent::settings_put))
        .route("/api/torrents/announce", get(tracker::shim))
        .route("/api/torrents/events", get(torrent::events))
        .route("/api/torrents/local/{type}/{id}", get(torrent::local))
        .route("/api/autodl/rules", get(autodl::list))
        .route("/api/autodl/rules/{type}/{id}", axum::routing::put(autodl::put).delete(autodl::delete))
        .route("/api/autodl/history", get(autodl::history))
        .route("/api/autodl/history/{key}", axum::routing::delete(autodl::history_delete))
        .route("/api/autodl/run", post(autodl::run))
        .route("/api/autodl/preview/{type}/{id}", post(autodl::preview))
        .route("/api/torrents/{id}", get(torrent::get).delete(torrent::delete))
        .route("/api/torrents/{id}/pause", post(torrent::pause))
        .route("/api/torrents/{id}/resume", post(torrent::resume))
        .route("/api/torrents/{id}/files", axum::routing::put(torrent::put_files))
        .route("/api/torrents/{id}/priority", axum::routing::put(torrent::put_priority))
        .route("/api/torrents/{id}/next", get(torrent::next))
        .route("/api/torrents/{id}/subtitle", get(play::subtitle))
        .route("/api/torrents/{id}/seek", get(play::seek))
        .route("/api/torrents/{id}/open", post(torrent::open_folder))
        .route("/api/torrents/{id}/play", get(play::play))
        .route("/api/torrents/{id}/external", post(play::external))
        .route("/api/torrents/{id}/playinfo", get(play::playinfo))
        .route("/api/torrents/{id}/file/{index}", get(play::file))
        // `{id}` is "movie" | "tv" here: torrents of one title.
        .route("/api/torrents/{id}/{tmdb}", get(tsearch::by_title))
        .route("/api", axum::routing::any(api_not_found))
        .route("/api/{*rest}", axum::routing::any(api_not_found))
        .with_state(state)
        .layer(axum::middleware::from_fn(request_language))
}

/// `?lang=xx-YY` on any `/api` call sets the TMDB language for that request
/// (titles, overviews, episode names, genres). Without it, `config.language` applies.
async fn request_language(req: axum::extract::Request, next: axum::middleware::Next) -> axum::response::Response {
    use axum::response::IntoResponse;
    let lang = req
        .uri()
        .query()
        .and_then(|q| url::form_urlencoded::parse(q.as_bytes()).find(|(k, _)| k == "lang"))
        .map(|(_, v)| v.into_owned());
    match lang {
        None => next.run(req).await,
        Some(l) if crate::tmdb::valid_language(&l) => crate::tmdb::REQUEST_LANG.scope(l, next.run(req)).await,
        Some(_) => ApiError::bad_request("'lang' must look like 'de-DE' or 'en-US'").into_response(),
    }
}

// ---------- validation ----------

pub(crate) fn media_type(s: &str) -> ApiResult<MediaType> {
    MediaType::parse(s).ok_or_else(|| ApiError::bad_request("type must be 'movie' or 'tv'"))
}

pub(crate) fn numeric_id(s: &str) -> ApiResult<u64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ApiError::bad_request("id must be numeric"));
    }
    s.parse().map_err(|_| ApiError::bad_request("id must be numeric"))
}

fn small_num(params: &HashMap<String, String>, key: &str) -> ApiResult<Option<u32>> {
    match params.get(key) {
        None => Ok(None),
        Some(v) => v
            .parse::<u32>()
            .map(Some)
            .map_err(|_| ApiError::bad_request(format!("'{key}' must be a non-negative number"))),
    }
}

fn page_param(params: &HashMap<String, String>) -> ApiResult<u32> {
    let page = small_num(params, "page")?.unwrap_or(1);
    if !(1..=500).contains(&page) {
        return Err(ApiError::bad_request("'page' must be between 1 and 500"));
    }
    Ok(page)
}

// ---------- handlers ----------

async fn api_not_found() -> ApiError {
    ApiError::not_found("unknown API endpoint")
}

async fn status(State(st): St) -> Json<Value> {
    Json(json!({ "tmdb_ok": st.tmdb.check().await, "needs_setup": !st.tmdb.has_token(), "version": env!("CARGO_PKG_VERSION") }))
}

#[derive(Serialize)]
struct Home {
    trending: Vec<Item>,
    popular_movies: Vec<Item>,
    popular_tv: Vec<Item>,
    top_rated_movies: Vec<Item>,
    top_rated_tv: Vec<Item>,
}

async fn home(State(st): St) -> ApiResult<Json<Home>> {
    let t = &st.tmdb;
    let region = [("region", t.region.clone())];
    let (trending, pm, ptv, tm, ttv) = tokio::join!(
        t.get("/trending/all/day", &[], Some(TTL_LIST)),
        t.get("/movie/popular", &region, Some(TTL_LIST)),
        t.get("/tv/popular", &[], Some(TTL_LIST)),
        t.get("/movie/top_rated", &region, Some(TTL_LIST)),
        t.get("/tv/top_rated", &[], Some(TTL_LIST)),
    );
    Ok(Json(Home {
        trending: model::items_from(&trending?, None),
        popular_movies: model::items_from(&pm?, Some(MediaType::Movie)),
        popular_tv: model::items_from(&ptv?, Some(MediaType::Tv)),
        top_rated_movies: model::items_from(&tm?, Some(MediaType::Movie)),
        top_rated_tv: model::items_from(&ttv?, Some(MediaType::Tv)),
    }))
}

async fn list(State(st): St, Path((ty, list)): Path<(String, String)>, Query(q): Params) -> ApiResult<Json<Page>> {
    let kind = media_type(&ty)?;
    let page = page_param(&q)?;
    let t = &st.tmdb;
    let mut params = vec![("page", page.to_string())];
    if kind == MediaType::Movie {
        params.push(("region", t.region.clone()));
    }

    let path = if let Some(genre) = q.get("genre") {
        numeric_id(genre).map_err(|_| ApiError::bad_request("'genre' must be numeric"))?;
        params.push(("with_genres", genre.clone()));
        params.push(("sort_by", "popularity.desc".into()));
        format!("/discover/{}", kind.as_str())
    } else {
        match (kind, list.as_str()) {
            (_, "popular" | "top_rated") => format!("/{}/{list}", kind.as_str()),
            (_, "trending") => format!("/trending/{}/day", kind.as_str()),
            (MediaType::Movie, "now_playing") | (MediaType::Tv, "on_the_air") => {
                format!("/{}/{list}", kind.as_str())
            }
            _ => return Err(ApiError::bad_request("unknown list")),
        }
    };
    let v = t.get(&path, &params, Some(TTL_LIST)).await?;
    Ok(Json(model::page_from(&v, Some(kind))))
}

async fn genres(State(st): St, Path(ty): Path<String>) -> ApiResult<Json<Vec<Genre>>> {
    let kind = media_type(&ty)?;
    let v = st.tmdb.get(&format!("/genre/{}/list", kind.as_str()), &[], Some(TTL_LIST)).await?;
    Ok(Json(model::genres_from(&v)))
}

async fn search(State(st): St, Query(q): Params) -> ApiResult<Json<Page>> {
    let text = q.get("q").map(|s| s.trim()).unwrap_or_default();
    if text.is_empty() {
        return Err(ApiError::bad_request("'q' is required"));
    }
    let page = page_param(&q)?;
    let params = [
        ("query", text.to_owned()),
        ("page", page.to_string()),
        ("include_adult", "false".to_owned()),
    ];
    let v = st.tmdb.get("/search/multi", &params, Some(TTL_LIST)).await?;
    Ok(Json(model::page_from(&v, None)))
}

async fn title(State(st): St, Path((ty, id)): Path<(String, String)>) -> ApiResult<Json<Details>> {
    let kind = media_type(&ty)?;
    let id = numeric_id(&id)?;
    // Trailers: the request's language first, the other of de/en as fallback.
    let lang = st.tmdb.language();
    let primary = lang.get(..2).unwrap_or("de").to_owned();
    let fallback = if primary == "en" { "de" } else { "en" };
    let params = [
        ("append_to_response", "external_ids,videos,credits,recommendations".to_owned()),
        ("include_video_language", format!("{primary},{fallback}")),
    ];
    let v = st.tmdb.get(&format!("/{}/{id}", kind.as_str()), &params, Some(TTL_TITLE)).await?;
    model::details_from(&v, kind, &primary)
        .map(Json)
        .ok_or_else(|| ApiError::bad_gateway("TMDB returned incomplete data"))
}

async fn season(
    State(st): St,
    Path((ty, id, n)): Path<(String, String, String)>,
) -> ApiResult<Json<SeasonDetails>> {
    if media_type(&ty)? != MediaType::Tv {
        return Err(ApiError::bad_request("seasons exist only for type 'tv'"));
    }
    let id = numeric_id(&id)?;
    let n = numeric_id(&n)?;
    let v = st.tmdb.get(&format!("/tv/{id}/season/{n}"), &[], Some(TTL_TITLE)).await?;
    model::season_from(&v)
        .map(Json)
        .ok_or_else(|| ApiError::bad_gateway("TMDB returned incomplete data"))
}

#[derive(Serialize)]
pub struct Source {
    pub id: String,
    pub name: String,
    pub no_ads: bool,
    pub url: String,
}

/// A movie or one episode, as addressed by `/:type/:id?s=&e=`.
pub struct TitleRef {
    pub(crate) kind: MediaType,
    pub(crate) id: u64,
    pub(crate) episode: Option<(u32, u32)>,
}

impl TitleRef {
    pub fn key(&self) -> String {
        match self.episode {
            Some((s, e)) => format!("{}:{}:{s}:{e}", self.kind.as_str(), self.id),
            None => format!("{}:{}", self.kind.as_str(), self.id),
        }
    }
}

pub fn title_ref(ty: &str, id: &str, q: &HashMap<String, String>) -> ApiResult<TitleRef> {
    let kind = media_type(ty)?;
    let id = numeric_id(id)?;
    let episode = match kind {
        MediaType::Movie => None,
        MediaType::Tv => {
            let s = small_num(q, "s")?.ok_or_else(|| ApiError::bad_request("'s' is required for tv"))?;
            let e = small_num(q, "e")?.ok_or_else(|| ApiError::bad_request("'e' is required for tv"))?;
            Some((s, e))
        }
    };
    Ok(TitleRef { kind, id, episode })
}

pub async fn build_sources(st: &AppState, t: &TitleRef) -> ApiResult<Vec<Source>> {
    let (kind, id) = (t.kind, t.id);
    let template = |p: &Provider| -> String {
        match kind {
            MediaType::Movie => p.movie.clone(),
            MediaType::Tv => p.tv.clone(),
        }
    };

    let imdb = if st.providers.iter().any(|p| template(p).contains("{imdb}")) {
        let v = st.tmdb.get(&format!("/{}/{id}/external_ids", kind.as_str()), &[], None).await?;
        v.get("imdb_id").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned)
    } else {
        None
    };

    Ok(st
        .providers
        .iter()
        .filter_map(|p| {
            let mut url = template(p).replace("{tmdb}", &id.to_string());
            if url.contains("{imdb}") {
                url = url.replace("{imdb}", imdb.as_deref()?);
            }
            if let Some((s, e)) = t.episode {
                url = url.replace("{s}", &s.to_string()).replace("{e}", &e.to_string());
            }
            Some(Source { id: p.id.clone(), name: p.name.clone(), no_ads: p.no_ads, url })
        })
        .collect())
}

async fn sources(
    State(st): St,
    Path((ty, id)): Path<(String, String)>,
    Query(q): Params,
) -> ApiResult<Json<Vec<Source>>> {
    build_sources(&st, &title_ref(&ty, &id, &q)?).await.map(Json)
}

async fn watchlist_list(State(st): St) -> ApiResult<Json<Vec<Item>>> {
    Ok(Json(st.db.list()?))
}

async fn watchlist_put(State(st): St, Path((ty, id)): Path<(String, String)>, body: Bytes) -> ApiResult<StatusCode> {
    let kind = media_type(&ty)?;
    let id = numeric_id(&id)?;
    let mut item: Item =
        serde_json::from_slice(&body).map_err(|e| ApiError::bad_request(format!("invalid Item body: {e}")))?;
    // The path is authoritative for the key.
    item.kind = kind;
    item.id = id;
    st.db.put(&item)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn watchlist_delete(State(st): St, Path((ty, id)): Path<(String, String)>) -> ApiResult<StatusCode> {
    let kind = media_type(&ty)?;
    let id = numeric_id(&id)?;
    st.db.delete(kind, id)?;
    Ok(StatusCode::NO_CONTENT)
}
