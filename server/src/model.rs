use serde::{Deserialize, Serialize};
use serde_json::Value;

const IMG: &str = "https://image.tmdb.org/t/p";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaType {
    Movie,
    Tv,
}

impl MediaType {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "movie" => Some(Self::Movie),
            "tv" => Some(Self::Tv),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Tv => "tv",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub id: u64,
    #[serde(rename = "type")]
    pub kind: MediaType,
    pub title: String,
    pub year: Option<i32>,
    pub poster: Option<String>,
    pub backdrop: Option<String>,
    pub rating: Option<f64>,
    pub overview: String,
}

/// What the UI hands over with a progress save, kept so lists render without TMDB calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchMeta {
    pub item: Item,
    #[serde(default)]
    pub episode_name: Option<String>,
    #[serde(default)]
    pub still: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Genre {
    pub id: u64,
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct CastMember {
    pub name: String,
    pub character: String,
    pub profile: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Season {
    pub season_number: u32,
    pub name: String,
    pub episode_count: u32,
    pub poster: Option<String>,
    pub air_date: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Details {
    #[serde(flatten)]
    pub item: Item,
    pub original_title: String,
    pub imdb_id: Option<String>,
    pub genres: Vec<Genre>,
    pub runtime: Option<u32>,
    pub tagline: Option<String>,
    pub trailer_youtube_key: Option<String>,
    pub seasons: Vec<Season>,
    pub cast: Vec<CastMember>,
    pub recommendations: Vec<Item>,
}

#[derive(Debug, Serialize)]
pub struct Episode {
    pub episode_number: u32,
    pub name: String,
    pub overview: String,
    pub still: Option<String>,
    pub air_date: Option<String>,
    pub runtime: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct SeasonDetails {
    pub season_number: u32,
    pub name: String,
    pub episodes: Vec<Episode>,
}

#[derive(Debug, Serialize)]
pub struct Page {
    pub results: Vec<Item>,
    pub page: u64,
    pub total_pages: u64,
}

// ---------- helpers ----------

fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned)
}

fn string_or_empty(v: &Value, key: &str) -> String {
    str_of(v, key).unwrap_or_default()
}

fn u32_of(v: &Value, key: &str) -> Option<u32> {
    v.get(key).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok())
}

pub fn image(size: &str, v: &Value, key: &str) -> Option<String> {
    str_of(v, key).map(|p| format!("{IMG}/{size}{p}"))
}

fn year_of(date: Option<String>) -> Option<i32> {
    date.and_then(|d| d.get(..4).and_then(|y| y.parse().ok()))
}

/// Maps a TMDB movie/tv object to an `Item`. Uses `media_type` when present,
/// otherwise `fallback`. Returns `None` for people or unknown types.
pub fn item_from(v: &Value, fallback: Option<MediaType>) -> Option<Item> {
    let kind = match v.get("media_type").and_then(Value::as_str) {
        Some(t) => MediaType::parse(t)?,
        None => fallback?,
    };
    let id = v.get("id")?.as_u64()?;
    let (title, date) = match kind {
        MediaType::Movie => (string_or_empty(v, "title"), str_of(v, "release_date")),
        MediaType::Tv => (string_or_empty(v, "name"), str_of(v, "first_air_date")),
    };
    let votes = v.get("vote_count").and_then(Value::as_u64).unwrap_or(0);
    let rating = if votes == 0 {
        None
    } else {
        v.get("vote_average").and_then(Value::as_f64).map(|r| (r * 10.0).round() / 10.0)
    };
    Some(Item {
        id,
        kind,
        title,
        year: year_of(date),
        poster: image("w500", v, "poster_path"),
        backdrop: image("w1280", v, "backdrop_path"),
        rating,
        overview: string_or_empty(v, "overview"),
    })
}

pub fn items_from(v: &Value, fallback: Option<MediaType>) -> Vec<Item> {
    v.get("results")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(|r| item_from(r, fallback)).collect())
        .unwrap_or_default()
}

pub fn page_from(v: &Value, fallback: Option<MediaType>) -> Page {
    Page {
        results: items_from(v, fallback),
        page: v.get("page").and_then(Value::as_u64).unwrap_or(1),
        // TMDB refuses pages above 500.
        total_pages: v.get("total_pages").and_then(Value::as_u64).unwrap_or(0).min(500),
    }
}

pub fn genres_from(v: &Value) -> Vec<Genre> {
    v.get("genres")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|g| Some(Genre { id: g.get("id")?.as_u64()?, name: string_or_empty(g, "name") }))
                .collect()
        })
        .unwrap_or_default()
}

/// Picks a YouTube trailer: `pref` language first (e.g. "de"), then English, then any language.
fn trailer_key(v: &Value, pref: &str) -> Option<String> {
    let videos = v.pointer("/videos/results")?.as_array()?;
    let trailers: Vec<&Value> = videos
        .iter()
        .filter(|x| {
            x.get("site").and_then(Value::as_str) == Some("YouTube")
                && x.get("type").and_then(Value::as_str) == Some("Trailer")
        })
        .collect();
    let lang = |l: &str| {
        trailers
            .iter()
            .filter(|x| x.get("iso_639_1").and_then(Value::as_str) == Some(l))
            // official trailers first
            .max_by_key(|x| x.get("official").and_then(Value::as_bool).unwrap_or(false))
            .copied()
    };
    lang(pref)
        .or_else(|| lang("en"))
        .or_else(|| lang("de"))
        .or_else(|| trailers.first().copied())
        .and_then(|x| str_of(x, "key"))
}

/// `trailer_lang`: ISO 639-1 code of the preferred trailer language.
pub fn details_from(v: &Value, kind: MediaType, trailer_lang: &str) -> Option<Details> {
    let item = item_from(v, Some(kind))?;

    let (original_title, imdb_id, runtime) = match kind {
        MediaType::Movie => (
            string_or_empty(v, "original_title"),
            str_of(v, "imdb_id"),
            u32_of(v, "runtime").filter(|r| *r > 0),
        ),
        MediaType::Tv => (
            string_or_empty(v, "original_name"),
            v.get("external_ids").and_then(|e| str_of(e, "imdb_id")),
            v.get("episode_run_time")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok()),
        ),
    };

    let seasons = match kind {
        MediaType::Movie => Vec::new(),
        MediaType::Tv => {
            let all: Vec<Season> = v
                .get("seasons")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|s| {
                            Some(Season {
                                season_number: u32_of(s, "season_number")?,
                                name: string_or_empty(s, "name"),
                                episode_count: u32_of(s, "episode_count").unwrap_or(0),
                                poster: image("w500", s, "poster_path"),
                                air_date: str_of(s, "air_date"),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            if all.iter().any(|s| s.season_number != 0) {
                all.into_iter().filter(|s| s.season_number != 0).collect()
            } else {
                all
            }
        }
    };

    let cast = v
        .pointer("/credits/cast")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .take(12)
                .map(|c| CastMember {
                    name: string_or_empty(c, "name"),
                    character: string_or_empty(c, "character"),
                    profile: image("w300", c, "profile_path"),
                })
                .collect()
        })
        .unwrap_or_default();

    let recommendations = v
        .get("recommendations")
        .map(|r| items_from(r, Some(kind)).into_iter().take(20).collect())
        .unwrap_or_default();

    Some(Details {
        item,
        original_title,
        imdb_id,
        genres: genres_from(v),
        runtime,
        tagline: str_of(v, "tagline"),
        trailer_youtube_key: trailer_key(v, trailer_lang),
        seasons,
        cast,
        recommendations,
    })
}

pub fn season_from(v: &Value) -> Option<SeasonDetails> {
    let episodes = v
        .get("episodes")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|e| {
                    Some(Episode {
                        episode_number: u32_of(e, "episode_number")?,
                        name: string_or_empty(e, "name"),
                        overview: string_or_empty(e, "overview"),
                        still: image("w300", e, "still_path"),
                        air_date: str_of(e, "air_date"),
                        runtime: u32_of(e, "runtime"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Some(SeasonDetails {
        season_number: u32_of(v, "season_number")?,
        name: string_or_empty(v, "name"),
        episodes,
    })
}
