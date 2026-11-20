use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use axum::http::StatusCode;
use serde_json::Value;

use crate::error::{ApiError, ApiResult};

const BASE: &str = "https://api.themoviedb.org/3";
const MAX_CACHE_ENTRIES: usize = 2000;

pub const TTL_LIST: Duration = Duration::from_secs(30 * 60);
pub const TTL_TITLE: Duration = Duration::from_secs(24 * 60 * 60);
/// TV details / seasons for "Weiterschauen" and "Neue Folgen": new episodes show up within 6 h.
pub const TTL_SHOW: Duration = Duration::from_secs(6 * 60 * 60);

tokio::task_local! {
    /// TMDB language of the current request (`?lang=`), set by the API middleware.
    pub static REQUEST_LANG: String;
}

/// Accepts `xx` or `xx-YY` (ISO 639-1, optional ISO 3166-1 region), e.g. `de-DE`, `en-US`.
pub fn valid_language(s: &str) -> bool {
    let b = s.as_bytes();
    match b.len() {
        2 => b.iter().all(u8::is_ascii_lowercase),
        5 => b[..2].iter().all(u8::is_ascii_lowercase) && b[2] == b'-' && b[3..].iter().all(u8::is_ascii_uppercase),
        _ => false,
    }
}

/// Thin TMDB v3 client with a small in-memory TTL cache.
/// The bearer token is only ever placed in the Authorization header.
pub struct Tmdb {
    http: reqwest::Client,
    token: String,
    pub language: String,
    pub region: String,
    cache: Mutex<HashMap<String, (Instant, Value)>>,
}

impl Tmdb {
    pub fn new(token: String, language: String, region: String) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!("flimmer/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(15))
            .build()
            .expect("HTTP client must build");
        Self { http, token, language, region, cache: Mutex::new(HashMap::new()) }
    }

    /// The request's language (`?lang=`), else `config.language`.
    pub fn language(&self) -> String {
        REQUEST_LANG.try_with(Clone::clone).unwrap_or_else(|_| self.language.clone())
    }

    /// GET `path` with `language` plus `params`. `ttl = None` bypasses the cache.
    /// The language is part of the cache key, so each language is cached separately.
    pub async fn get(&self, path: &str, params: &[(&str, String)], ttl: Option<Duration>) -> ApiResult<Value> {
        let mut query: Vec<(&str, String)> = Vec::with_capacity(params.len() + 1);
        query.push(("language", self.language()));
        query.extend(params.iter().cloned());

        let key = format!(
            "{path}?{}",
            query.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&")
        );

        if ttl.is_some()
            && let Some((expires, value)) = self.cache.lock().expect("cache lock").get(&key)
            && *expires > Instant::now()
        {
            return Ok(value.clone());
        }

        let resp = self
            .http
            .get(format!("{BASE}{path}"))
            .bearer_auth(&self.token)
            .query(&query)
            .send()
            .await
            .map_err(|_| ApiError::bad_gateway("TMDB is not reachable"))?;

        let status = resp.status();
        if status == StatusCode::NOT_FOUND {
            return Err(ApiError::not_found("not found on TMDB"));
        }
        if !status.is_success() {
            return Err(ApiError::bad_gateway(format!("TMDB responded with status {}", status.as_u16())));
        }
        let value: Value = resp
            .json()
            .await
            .map_err(|_| ApiError::bad_gateway("TMDB returned an invalid response"))?;

        if let Some(ttl) = ttl {
            let mut cache = self.cache.lock().expect("cache lock");
            if cache.len() >= MAX_CACHE_ENTRIES {
                let now = Instant::now();
                cache.retain(|_, (exp, _)| *exp > now);
                if cache.len() >= MAX_CACHE_ENTRIES {
                    cache.clear();
                }
            }
            cache.insert(key, (Instant::now() + ttl, value.clone()));
        }
        Ok(value)
    }

    /// True if the token is accepted by TMDB.
    pub async fn check(&self) -> bool {
        self.get("/configuration", &[], None).await.is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_codes() {
        for ok in ["de", "en", "de-DE", "en-US"] {
            assert!(valid_language(ok), "{ok}");
        }
        for bad in ["", "e", "EN", "en_US", "en-us", "english", "de-DE&x=1"] {
            assert!(!valid_language(bad), "{bad}");
        }
    }

    #[tokio::test]
    async fn request_language_overrides_config() {
        let t = Tmdb::new(String::new(), "de-DE".into(), "DE".into());
        assert_eq!(t.language(), "de-DE");
        let inner = REQUEST_LANG.scope("en-US".into(), async { t.language() }).await;
        assert_eq!(inner, "en-US");
    }
}
