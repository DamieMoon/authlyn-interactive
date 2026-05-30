//! Native HTTP client — the `reqwest` port of `src/client/api.rs`'s transport.
//!
//! Mirrors the browser client's shape (`ApiError` + `get`/`post_json`/`decode`/
//! `error_message`) but swaps gloo-net for `reqwest` and carries the session
//! explicitly: a native client is not a same-origin browser, and the server's
//! `authlyn_session` cookie is set `Secure` (`server/auth/session.rs`), which a
//! reqwest cookie jar will NOT replay over dev `http://`. So we capture the
//! cookie's value from the login `Set-Cookie` and attach it manually on every
//! request — preserving the "identity only from the session token, DB stores
//! only its SHA-256" invariant with no backend change. DTOs are reused from
//! [`crate::protocol`], never redefined.

use serde::{de::DeserializeOwned, Serialize};
use std::sync::Mutex;

use crate::protocol::{
    AuthResponse, CreateGuildRequest, ErrorBody, GuildSummary, ListGuildsResponse, LoginRequest,
    MeResponse, RegisterRequest,
};

/// Name of the session cookie the server sets on login/register.
const SESSION_COOKIE: &str = "authlyn_session";

/// Transport/decoding failure — same three variants as the browser client's
/// `ApiError` so the action layer ports 1:1.
#[derive(Clone, Debug)]
pub enum ApiError {
    /// No HTTP response (connection refused, DNS, timeout).
    Network(String),
    /// A 4xx/5xx response, carrying the server's `{"error": …}` message.
    Status(u16, String),
    /// A 2xx response whose body failed to deserialize.
    Codec(String),
}

impl ApiError {
    /// The HTTP status, when this is a server error response.
    pub fn status(&self) -> Option<u16> {
        match self {
            ApiError::Status(code, _) => Some(*code),
            _ => None,
        }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Network(m) => write!(f, "network error: {m}"),
            ApiError::Status(code, m) => write!(f, "HTTP {code}: {m}"),
            ApiError::Codec(m) => write!(f, "decode error: {m}"),
        }
    }
}

/// A configured client for one backend. Holds the captured session token so
/// authed requests carry it; `base_url` is configurable for dev vs prod.
pub struct ApiClient {
    http: reqwest::Client,
    base_url: String,
    session: Mutex<Option<String>>,
}

impl Default for ApiClient {
    fn default() -> Self {
        Self::new()
    }
}

impl ApiClient {
    /// Build a client. Base URL comes from `AUTHLYN_NATIVE_URL`, defaulting to
    /// the dev server on loopback. (No TLS backend is compiled in yet — Phase 1
    /// targets loopback http; prod https adds `rustls-tls` later.)
    pub fn new() -> Self {
        let base_url = std::env::var("AUTHLYN_NATIVE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());
        Self {
            http: reqwest::Client::new(),
            base_url,
            session: Mutex::new(None),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    /// `Cookie: authlyn_session=<token>` when a session has been captured.
    fn cookie_header(&self) -> Option<String> {
        self.session
            .lock()
            .unwrap()
            .clone()
            .map(|tok| format!("{SESSION_COOKIE}={tok}"))
    }

    /// Pull the `authlyn_session` value out of a login/register `Set-Cookie`.
    fn capture_session(&self, resp: &reqwest::Response) {
        for hv in resp.headers().get_all(reqwest::header::SET_COOKIE) {
            let Ok(s) = hv.to_str() else { continue };
            if let Some(rest) = s.strip_prefix(&format!("{SESSION_COOKIE}=")) {
                let val = rest.split(';').next().unwrap_or("").to_string();
                if !val.is_empty() {
                    *self.session.lock().unwrap() = Some(val);
                }
                return;
            }
        }
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        let mut req = self.http.get(self.url(path));
        if let Some(cookie) = self.cookie_header() {
            req = req.header(reqwest::header::COOKIE, cookie);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;
        decode(resp).await
    }

    async fn post_json<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        let mut req = self.http.post(self.url(path)).json(body);
        if let Some(cookie) = self.cookie_header() {
            req = req.header(reqwest::header::COOKIE, cookie);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;
        // Login/register carry the session in Set-Cookie; harmless no-op otherwise.
        self.capture_session(&resp);
        decode(resp).await
    }

    /// POST /auth/login — authenticate and capture the session cookie.
    pub async fn login(&self, username: &str, password: &str) -> Result<AuthResponse, ApiError> {
        self.post_json(
            "/auth/login",
            &LoginRequest {
                username: username.to_string(),
                password: password.to_string(),
            },
        )
        .await
    }

    /// POST /auth/register — create the account and capture the session cookie.
    pub async fn register(&self, username: &str, password: &str) -> Result<AuthResponse, ApiError> {
        self.post_json(
            "/auth/register",
            &RegisterRequest {
                username: username.to_string(),
                password: password.to_string(),
            },
        )
        .await
    }

    /// Log in; bootstrap-register on the first run if the account doesn't exist
    /// yet (401). Mirrors the nova bridge's `ensure_session`.
    pub async fn ensure_session(
        &self,
        username: &str,
        password: &str,
    ) -> Result<AuthResponse, ApiError> {
        match self.login(username, password).await {
            Ok(auth) => Ok(auth),
            Err(e) if e.status() == Some(401) => self.register(username, password).await,
            Err(e) => Err(e),
        }
    }

    /// GET /auth/me — the authenticated caller's profile.
    pub async fn current_user(&self) -> Result<MeResponse, ApiError> {
        self.get("/auth/me").await
    }

    /// GET /guilds — the caller's guild rail.
    pub async fn list_guilds(&self) -> Result<ListGuildsResponse, ApiError> {
        self.get("/guilds").await
    }

    /// POST /guilds — create a guild, returning its summary.
    pub async fn create_guild(&self, name: &str) -> Result<GuildSummary, ApiError> {
        self.post_json(
            "/guilds",
            &CreateGuildRequest {
                name: name.to_string(),
            },
        )
        .await
    }
}

/// Decode a 2xx body as `T`, else surface the server's error message.
async fn decode<T: DeserializeOwned>(resp: reqwest::Response) -> Result<T, ApiError> {
    let status = resp.status().as_u16();
    if (200..300).contains(&status) {
        resp.json::<T>()
            .await
            .map_err(|e| ApiError::Codec(e.to_string()))
    } else {
        Err(ApiError::Status(status, error_message(resp).await))
    }
}

/// Extract `{"error": "…"}` from a failed response, with a generic fallback.
async fn error_message(resp: reqwest::Response) -> String {
    resp.json::<ErrorBody>()
        .await
        .map(|b| b.error)
        .unwrap_or_else(|_| "request failed".to_string())
}
