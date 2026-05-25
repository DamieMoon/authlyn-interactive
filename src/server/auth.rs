//! Username/password accounts + server-side sessions (phase-1 build step 1).
//!
//! Replaces the retired `X-Device-Id` auth stub. The trust model is now a
//! classic server-side session: `POST /auth/register` and `/auth/login`
//! mint a random opaque token, store only its SHA-256 in the `session`
//! table, and hand the token to the browser in an `HttpOnly; Secure;
//! SameSite=Lax` cookie. Every protected handler takes the [`AuthAccount`]
//! extractor, which resolves that cookie back to an account id (or rejects
//! with 401). Passwords are argon2id-hashed; hashing/verification run on the
//! blocking pool so they don't stall the async runtime.
//!
//! ## Stance
//! - Defensive at the boundary: JSON-shape errors → typed 400; unknown
//!   user and wrong password return the **same** 401 body (no enumeration).
//! - The `account_username_ci UNIQUE` index is the source of truth for
//!   "username taken"; a racing duplicate register surfaces as
//!   [`is_unique_violation`] → 409, same body as the pre-check would give.

use argon2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
};
use argon2::Argon2;
use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use surrealdb::types::SurrealValue;

use crate::protocol::{AuthResponse, ErrorBody, LoginRequest, MeResponse, RegisterRequest};
use crate::server::retry::is_unique_violation;
use crate::server::state::AppState;

const SESSION_COOKIE: &str = "authlyn_session";
const SESSION_TTL_DAYS: i64 = 30;

const MIN_USERNAME_CHARS: usize = 3;
const MAX_USERNAME_CHARS: usize = 32;
const MIN_PASSWORD_BYTES: usize = 8;
const MAX_PASSWORD_BYTES: usize = 4096;

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

fn unauthorized() -> (StatusCode, Json<ErrorBody>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(ErrorBody::new("authentication required")),
    )
}

// ---------------------------------------------------------------------------
// POST /auth/register
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(username))]
pub async fn register(
    State(state): State<AppState>,
    jar: CookieJar,
    payload: Result<Json<RegisterRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    let username = req.username.trim().to_string();
    tracing::Span::current().record("username", tracing::field::display(&username));

    if let Err(msg) = validate_credentials(&username, &req.password) {
        return error_response(StatusCode::BAD_REQUEST, msg);
    }
    let username_ci = username.to_lowercase();

    let password = req.password.clone();
    let password_hash = match hash_on_blocking_pool(password).await {
        Ok(h) => h,
        Err(resp) => return resp,
    };

    let account_id = match create_account(&state, &username, &username_ci, &password_hash).await {
        Ok(id) => id,
        Err(e) if is_unique_violation(&e) => {
            return error_response(StatusCode::CONFLICT, "username already taken");
        }
        Err(e) => {
            tracing::error!(error = %e, "create_account failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };

    let token = match issue_session(&state, &account_id).await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "issue_session failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };

    (
        StatusCode::CREATED,
        jar.add(session_cookie(token)),
        Json(AuthResponse {
            account_id,
            username,
        }),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// POST /auth/login
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(username))]
pub async fn login(
    State(state): State<AppState>,
    jar: CookieJar,
    payload: Result<Json<LoginRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    let username = req.username.trim().to_string();
    tracing::Span::current().record("username", tracing::field::display(&username));
    let username_ci = username.to_lowercase();

    // Same 401 body for "no such user" and "wrong password" — no enumeration.
    let row = match account_by_username_ci(&state, &username_ci).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "account lookup failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    let Some((account_id, password_hash, stored_username)) = row else {
        return invalid_credentials();
    };

    match verify_on_blocking_pool(req.password.clone(), password_hash).await {
        Ok(true) => {}
        Ok(false) => return invalid_credentials(),
        Err(resp) => return resp,
    }

    let token = match issue_session(&state, &account_id).await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "issue_session failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };

    (
        StatusCode::OK,
        jar.add(session_cookie(token)),
        Json(AuthResponse {
            account_id,
            username: stored_username,
        }),
    )
        .into_response()
}

fn invalid_credentials() -> Response {
    error_response(StatusCode::UNAUTHORIZED, "invalid username or password")
}

// ---------------------------------------------------------------------------
// POST /auth/logout
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all)]
pub async fn logout(State(state): State<AppState>, jar: CookieJar) -> Response {
    if let Some(token) = jar.get(SESSION_COOKIE).map(|c| c.value().to_owned()) {
        let token_hash = sha256_hex(token.as_bytes());
        // Best-effort: a failed delete just leaves a row that expires on its
        // own; the cookie is cleared regardless.
        if let Err(e) = state
            .db
            .query("DELETE FROM session WHERE token_hash = $th;")
            .bind(("th", token_hash))
            .await
        {
            tracing::warn!(error = %e, "logout session delete failed");
        }
    }
    let jar = jar.remove(Cookie::build((SESSION_COOKIE, "")).path("/").build());
    (jar, StatusCode::NO_CONTENT).into_response()
}

// ---------------------------------------------------------------------------
// GET /auth/me
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0))]
pub async fn me(State(state): State<AppState>, account: AuthAccount) -> Response {
    match account_profile(&state, &account.0).await {
        Ok(Some((username, display_name))) => (
            StatusCode::OK,
            Json(MeResponse {
                account_id: account.0,
                username,
                display_name,
            }),
        )
            .into_response(),
        Ok(None) => error_response(StatusCode::UNAUTHORIZED, "authentication required"),
        Err(e) => {
            tracing::error!(error = %e, "account_profile failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// DB helpers
// ---------------------------------------------------------------------------

async fn create_account(
    state: &AppState,
    username: &str,
    username_ci: &str,
    password_hash: &str,
) -> surrealdb::Result<String> {
    #[derive(SurrealValue)]
    struct IdRow {
        id_key: String,
    }
    let mut resp = state
        .db
        .query(
            "CREATE account SET
                username = $username,
                username_ci = $username_ci,
                password_hash = $password_hash
                RETURN meta::id(id) AS id_key;",
        )
        .bind(("username", username.to_string()))
        .bind(("username_ci", username_ci.to_string()))
        .bind(("password_hash", password_hash.to_string()))
        .await?
        .check()?;
    let row: Option<IdRow> = resp.take(0)?;
    row.map(|r| r.id_key)
        .ok_or_else(|| surrealdb::Error::thrown("create_account produced no row".to_string()))
}

/// Mint a fresh session for `account_id` and return the raw token to set in
/// the cookie. The DB stores only the token's SHA-256.
async fn issue_session(state: &AppState, account_id: &str) -> surrealdb::Result<String> {
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

async fn account_for_token(state: &AppState, token: &str) -> surrealdb::Result<Option<String>> {
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

/// Returns `(account_id, password_hash, stored_username)` for a username
/// (matched case-insensitively), if one exists.
async fn account_by_username_ci(
    state: &AppState,
    username_ci: &str,
) -> surrealdb::Result<Option<(String, String, String)>> {
    #[derive(SurrealValue)]
    struct Row {
        id_key: String,
        password_hash: String,
        username: String,
    }
    let mut resp = state
        .db
        .query(
            "SELECT meta::id(id) AS id_key, password_hash, username FROM account
                WHERE username_ci = $username_ci;",
        )
        .bind(("username_ci", username_ci.to_string()))
        .await?
        .check()?;
    let row: Option<Row> = resp.take(0)?;
    Ok(row.map(|r| (r.id_key, r.password_hash, r.username)))
}

async fn account_profile(
    state: &AppState,
    account_id: &str,
) -> surrealdb::Result<Option<(String, String)>> {
    #[derive(SurrealValue)]
    struct Row {
        username: String,
        display_name: String,
    }
    let mut resp = state
        .db
        .query("SELECT username, display_name FROM type::record('account', $account_id);")
        .bind(("account_id", account_id.to_string()))
        .await?
        .check()?;
    let row: Option<Row> = resp.take(0)?;
    Ok(row.map(|r| (r.username, r.display_name)))
}

// ---------------------------------------------------------------------------
// Crypto / token helpers
// ---------------------------------------------------------------------------

fn random_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn sha256_hex(input: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(input);
    hex::encode(hasher.finalize())
}

/// argon2id hash on the blocking pool (it's tens of ms of CPU). Maps task /
/// hashing failures to a 500 response so callers can `?`-style early-return.
async fn hash_on_blocking_pool(password: String) -> Result<String, Response> {
    match tokio::task::spawn_blocking(move || {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|h| h.to_string())
    })
    .await
    {
        Ok(Ok(hash)) => Ok(hash),
        Ok(Err(e)) => {
            tracing::error!(error = %e, "argon2 hashing failed");
            Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "hashing failed",
            ))
        }
        Err(e) => {
            tracing::error!(error = %e, "hash task join failed");
            Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "hashing failed",
            ))
        }
    }
}

async fn verify_on_blocking_pool(password: String, phc: String) -> Result<bool, Response> {
    match tokio::task::spawn_blocking(move || match PasswordHash::new(&phc) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    })
    .await
    {
        Ok(verified) => Ok(verified),
        Err(e) => {
            tracing::error!(error = %e, "verify task join failed");
            Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "verification failed",
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// Shaping
// ---------------------------------------------------------------------------

fn validate_credentials(username: &str, password: &str) -> Result<(), &'static str> {
    let n = username.chars().count();
    if !(MIN_USERNAME_CHARS..=MAX_USERNAME_CHARS).contains(&n) {
        return Err("username must be 3–32 characters");
    }
    if username.chars().any(char::is_whitespace) {
        return Err("username must not contain whitespace");
    }
    if password.len() < MIN_PASSWORD_BYTES {
        return Err("password must be at least 8 characters");
    }
    if password.len() > MAX_PASSWORD_BYTES {
        return Err("password too long");
    }
    Ok(())
}

fn session_cookie(token: String) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, token))
        .path("/")
        .http_only(true)
        .secure(true)
        .same_site(SameSite::Lax)
        .max_age(time::Duration::days(SESSION_TTL_DAYS))
        .build()
}

fn error_response(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(ErrorBody::new(msg))).into_response()
}

fn json_rejection_response(rej: JsonRejection) -> Response {
    let reason: &'static str = match rej {
        JsonRejection::JsonDataError(_) => "invalid JSON body shape",
        JsonRejection::JsonSyntaxError(_) => "malformed JSON",
        JsonRejection::MissingJsonContentType(_) => "missing Content-Type: application/json",
        JsonRejection::BytesRejection(_) => "could not read request body",
        _ => "invalid JSON request",
    };
    error_response(StatusCode::BAD_REQUEST, reason)
}
