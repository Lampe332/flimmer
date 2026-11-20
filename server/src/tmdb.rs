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

    /// GET `path` with `language` plus `params`. `ttl = None` bypasses the cache.
    pub async fn get(&self, path: &str, params: &[(&str, String)], ttl: Option<Duration>) -> ApiResult<Value> {
        let mut query: Vec<(&str, String)> = Vec::with_capacity(params.len() + 1);
        query.push(("language", self.language.clone()));
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
