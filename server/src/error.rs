use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

/// API error rendered as `{ "error": "message" }`. Messages never contain secrets.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
    /// Machine-readable reason the UI acts on, e.g. `needs_setup`.
    code: Option<&'static str>,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self { status, message: message.into(), code: None }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }

    pub fn bad_gateway(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_GATEWAY, message)
    }

    /// No TMDB token yet: the UI shows the first-run setup.
    pub fn needs_setup() -> Self {
        Self { code: Some("needs_setup"), ..Self::new(StatusCode::SERVICE_UNAVAILABLE, "Flimmer is not set up yet (TMDB token missing)") }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = match self.code {
            Some(code) => json!({ "error": self.message, "code": code }),
            None => json!({ "error": self.message }),
        };
        (self.status, Json(body)).into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
