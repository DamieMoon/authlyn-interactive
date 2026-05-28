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

pub(super) const SESSION_COOKIE: &str = "authlyn_session";
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

pub(super) async fn account_for_token(
    state: &AppState,
    token: &str,
) -> surrealdb::Result<Option<String>> {
    #[derive(SurrealValue)]
    struct Row {
        account_key: String,
    }
    let token_hash = sha256_hex(token.as_bytes());
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

pub(super) fn session_cookie(token: String) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, token))
        .path("/")
        .http_only(true)
        .secure(true)
        .same_site(SameSite::Lax)
        .max_age(time::Duration::days(SESSION_TTL_DAYS))
        .build()
}
