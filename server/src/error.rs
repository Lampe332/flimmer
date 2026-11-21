use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

/// API error rendered as `{ "error": "message", "code": "reason" }`. Messages never contain secrets; the UI
/// only shows its own text for the code (an error without its own reason gets one by status).
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

    /// An error with a reason the UI acts on (`pc_only`, `pairing_required` …).
    pub fn coded(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self { code: Some(code), ..Self::new(status, message) }
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, message)
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

    /// No VPN file and torrents without VPN are not allowed: nothing may start.
    pub fn needs_vpn() -> Self {
        Self {
            code: Some("needs_vpn"),
            ..Self::new(StatusCode::CONFLICT, "no VPN file, and torrents without VPN are turned off in the settings")
        }
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
        let code = self.code.unwrap_or_else(|| status_code(self.status));
        (self.status, Json(json!({ "error": self.message, "code": code }))).into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

/// The code of an error that names no reason of its own.
fn status_code(status: StatusCode) -> &'static str {
    match status.as_u16() {
        400 => "bad_request",
        401 | 403 => "forbidden",
        404 => "not_found",
        409 => "conflict",
        502 => "bad_gateway",
        503 => "unavailable",
        504 => "timeout",
        _ => "internal",
    }
}

/// A failure the UI puts in its own words: `code` picks the text (one of [`CODES`]), `message` is English
/// for the log and the API docs and is never shown as it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fail {
    pub code: &'static str,
    pub message: String,
}

impl Fail {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }

    /// A tunnel failure, by what [`crate::vpn::Vpn::ensure_up`] said.
    pub fn vpn(message: impl Into<String>) -> Self {
        let message = message.into();
        Self::new(vpn_code(&message), message)
    }

    /// Whether trying again by itself can help (the tunnel comes back); other failures wait for the user.
    pub fn transient(&self) -> bool {
        self.code.starts_with("vpn_")
    }
}

/// The reason code of a tunnel error message.
pub fn vpn_code(message: &str) -> &'static str {
    let m = message.to_ascii_lowercase();
    if m.contains("kill-switch") {
        "vpn_blocked"
    } else if m.contains("no vpn file") {
        "needs_vpn"
    } else if m.contains("handshake") || m.contains("unreachable") {
        "vpn_handshake"
    } else if m.contains("config") || m.contains("wireguard") || m.contains("key") {
        "vpn_config"
    } else {
        "vpn_error"
    }
}

/// Every reason code a response, a torrent, an event or a decision can carry. The UI has a text for each
/// (`code.<name>` in every dictionary, checked by a test; the UI's list is web/src/lib/errors.ts); an unknown
/// one gets a general text.
#[cfg(test)]
const CODES: &[&str] = &[
    // any request
    "bad_request",
    "forbidden",
    "not_found",
    "conflict",
    "bad_gateway",
    "unavailable",
    "timeout",
    "internal",
    "needs_setup",
    "pc_only",
    // torrents
    "needs_vpn",
    "invalid_magnet",
    "invalid_infohash",
    "folder_outside",
    "folder_missing",
    "folder_local",
    "no_peers",
    "metadata_failed",
    "no_files",
    "disk_full",
    "write_failed",
    "start_failed",
    "download_failed",
    "vpn_blocked",
    "vpn_config",
    "vpn_handshake",
    "vpn_error",
    // auto-download decisions
    "not_on_watchlist",
    "tmdb_down",
    "already_there",
    "next_run",
    "search_failed",
    "nothing_fits",
    "dry_run",
    // source scan
    "scan_skipped",
    "scan_failed",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_without_a_reason_get_one_by_status() {
        assert_eq!(status_code(StatusCode::NOT_FOUND), "not_found");
        assert_eq!(status_code(StatusCode::IM_A_TEAPOT), "internal");
        for s in [400, 403, 404, 409, 500, 502, 503, 504] {
            assert!(CODES.contains(&status_code(StatusCode::from_u16(s).unwrap())), "{s}");
        }
    }

    #[test]
    fn tunnel_messages_map_to_codes() {
        assert_eq!(vpn_code("no WireGuard handshake (VPN server unreachable or key rejected)"), "vpn_handshake");
        assert_eq!(vpn_code("VPN kill-switch is engaged"), "vpn_blocked");
        assert_eq!(vpn_code("VPN config cannot be read (NotFound)"), "vpn_config");
        assert_eq!(vpn_code("no VPN file, and torrents without VPN are turned off in the settings"), "needs_vpn");
        assert_eq!(vpn_code("socket closed"), "vpn_error");
        assert!(Fail::vpn("socket closed").transient() && !Fail::new("no_peers", "x").transient());
    }

    /// Every code has a text in every UI language (the dictionaries are typed against de.ts, so de is enough).
    #[test]
    fn every_code_has_a_ui_text() {
        let de = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/i18n/de.ts")).unwrap();
        let missing: Vec<_> = CODES.iter().filter(|c| !de.contains(&format!("'code.{c}':"))).collect();
        assert!(missing.is_empty(), "no UI text for {missing:?}");
    }

    /// The UI knows the same codes as the server.
    #[test]
    fn the_ui_lists_the_same_codes() {
        let ui = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/src/lib/errors.ts")).unwrap();
        let missing: Vec<_> = CODES.iter().filter(|c| !ui.contains(&format!("'{c}',"))).collect();
        assert!(missing.is_empty(), "web/src/lib/errors.ts lacks {missing:?}");
    }
}
