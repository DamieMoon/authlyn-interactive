//! The `AuthAccount` extractor + the session-token lifecycle (issue, resolve,
//! revoke) + the `authlyn_session` cookie helper.
//!
//! Split from `server/auth.rs` in Wave 3; behavior preserved verbatim.
//! `AuthAccount` is re-exported from `server::auth` so its public path
//! `crate::server::auth::AuthAccount` remains stable for ~9 caller modules.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::Json;
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use surrealdb::types::SurrealValue;

use crate::protocol::ErrorBody;
use crate::server::state::AppState;

use super::crypto::{random_token, sha256_hex};

/// Name of the session cookie. `pub(crate)` because the long-lived
/// `GET /events` stream (`server::events`) reads the raw token at connect to
/// re-derive identity for the stream's lifetime (review M-05).
pub(crate) const SESSION_COOKIE: &str = "authlyn_session";
const SESSION_TTL_DAYS: i64 = 30;

// ---------------------------------------------------------------------------
// Extractor: cookie -> account id
// ---------------------------------------------------------------------------

/// The authenticated caller's `account` id (the bare key, e.g. `"abc123"`,
/// matching `meta::id(id)` form). Add it to any handler signature to require
/// (and resolve) a valid session; absence/expiry/garbage → 401.
pub struct AuthAccount(pub String);

impl FromRequestParts<AppState> for AuthAccount {
    type Rejection = (StatusCode, Json<ErrorBody>);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let jar = CookieJar::from_headers(&parts.headers);
        let Some(token) = jar.get(SESSION_COOKIE).map(|c| c.value().to_owned()) else {
            return Err(unauthorized());
        };
        match account_for_token(state, &token).await {
            Ok(Some(account)) => Ok(AuthAccount(account)),
            Ok(None) => Err(unauthorized()),
            Err(e) => {
                tracing::error!(error = %e, "session lookup failed");
                Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorBody::new("storage error")),
                ))
            }
        }
    }
}

/// The 401 rejection the [`AuthAccount`] extractor returns for any
/// no-valid-session case (absent/expired/garbage cookie). Shared so every miss
/// path returns one identical body; storage errors are a separate 500.
pub(super) fn unauthorized() -> (StatusCode, Json<ErrorBody>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(ErrorBody::new("authentication required")),
    )
}

// ---------------------------------------------------------------------------
// Session-token lifecycle
// ---------------------------------------------------------------------------

/// Mint a fresh session for `account_id` and return the raw token to set in
/// the cookie. The DB stores only the token's SHA-256.
pub(super) async fn issue_session(state: &AppState, account_id: &str) -> surrealdb::Result<String> {
    let token = random_token();
    let token_hash = sha256_hex(token.as_bytes());
    state
        .db
        .query(
            "CREATE session SET
                account = type::record('account', $account_id),
                token_hash = $token_hash,
                expires_at = time::now() + 30d;",
        )
        .bind(("account_id", account_id.to_string()))
        .bind(("token_hash", token_hash))
        .await?
        .check()?;
    Ok(token)
}

/// Resolve a RAW session token to its live account key (`None` = no such
/// session, or expired). Hashes the token then defers to
/// [`account_for_token_hash`] — the per-request convenience wrapper used by the
/// extractor; the SSE stream takes the hash-keyed twin directly so both share
/// one definition of "valid session".
pub(super) async fn account_for_token(
    state: &AppState,
    token: &str,
) -> surrealdb::Result<Option<String>> {
    account_for_token_hash(state, session_token_hash(token)).await
}

/// SHA-256 hex of a raw session token — the form `session.token_hash` stores
/// (the DB never sees the raw token). `pub(crate)` so a long-lived consumer
/// (the SSE stream, review M-05) hashes once at connect and re-checks via
/// [`account_for_token_hash`] without owning a mirror of the transform.
pub(crate) fn session_token_hash(token: &str) -> String {
    sha256_hex(token.as_bytes())
}

/// Resolve a STORED token hash to its live account key (`None` = no such
/// session, or expired). The hash-keyed twin of [`account_for_token`]: the
/// per-request extractor path and the SSE per-frame re-check (review M-05)
/// share this exact lookup, so "is this session valid" can never mean two
/// different things.
pub(crate) async fn account_for_token_hash(
    state: &AppState,
    token_hash: String,
) -> surrealdb::Result<Option<String>> {
    #[derive(SurrealValue)]
    struct Row {
        account_key: String,
    }
    let mut resp = state
        .db
        .query(
            "SELECT meta::id(account) AS account_key FROM session
                WHERE token_hash = $token_hash AND expires_at > time::now();",
        )
        .bind(("token_hash", token_hash))
        .await?
        .check()?;
    let row: Option<Row> = resp.take(0)?;
    Ok(row.map(|r| r.account_key))
}

/// Delete every session row for `account_id`. Called after a password reset so
/// any pre-existing cookie (possibly an attacker's) stops authenticating.
pub(super) async fn delete_sessions_for_account(
    state: &AppState,
    account_id: &str,
) -> surrealdb::Result<()> {
    state
        .db
        .query("DELETE FROM session WHERE account = type::record('account', $account_id);")
        .bind(("account_id", account_id.to_string()))
        .await?
        .check()?;
    Ok(())
}

/// Build the session cookie carrying the raw token: `HttpOnly` (no JS read),
/// `Secure`, `SameSite=Lax`, `Path=/`, 30-day `Max-Age`.
///
/// WebKit Secure-cookie trap (the subsystem's highest-surprise line): Safari/
/// WebKit silently **drops** a `Secure` cookie set over `http://localhost`
/// (Chromium accepts it), so the browser "logs in" 200 but the next `/auth/me`
/// stays 401. `secure(true)` is correct for prod and must NOT be relaxed — test
/// WebKit/iOS over HTTPS at the deck domain `https://authlyndev.damienmoon.sh`
/// (publicly-trusted cert) instead. NOT covered by any integration test (no
/// `tests/*.rs` asserts the cookie attributes); this is an owner-deck-only
/// check, guarded by CLAUDE.md doctrine.
pub(super) fn session_cookie(token: String) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, token))
        .path("/")
        .http_only(true)
        .secure(true)
        .same_site(SameSite::Lax)
        .max_age(time::Duration::days(SESSION_TTL_DAYS))
        .build()
}
