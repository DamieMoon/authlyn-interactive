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

use crate::protocol::{
    AdminResetPasswordRequest, AuthResponse, ChangePasswordRequest, ConfirmResetRequest, ErrorBody,
    LoginRequest, MeResponse, RegisterRequest, ResetQuestionResponse, SetSecurityQuestionRequest,
};
use crate::server::retry::is_unique_violation;
use crate::server::state::AppState;

const SESSION_COOKIE: &str = "authlyn_session";
const SESSION_TTL_DAYS: i64 = 30;

const MIN_USERNAME_CHARS: usize = 3;
const MAX_USERNAME_CHARS: usize = 32;
const MIN_PASSWORD_BYTES: usize = 8;
const MAX_PASSWORD_BYTES: usize = 4096;
const MIN_SECURITY_ANSWER_CHARS: usize = 3;

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
// POST /auth/change-password
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0))]
pub async fn change_password(
    State(state): State<AppState>,
    account: AuthAccount,
    payload: Result<Json<ChangePasswordRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };

    // Validate the new password against the same rule register enforces,
    // before doing the (expensive) verify of the current one.
    if let Err(msg) = validate_password(&req.new_password) {
        return error_response(StatusCode::BAD_REQUEST, msg);
    }

    let password_hash = match account_password_hash(&state, &account.0).await {
        Ok(Some(h)) => h,
        // Session resolved but the row is gone — treat as unauthenticated.
        Ok(None) => return error_response(StatusCode::UNAUTHORIZED, "authentication required"),
        Err(e) => {
            tracing::error!(error = %e, "account_password_hash failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };

    match verify_on_blocking_pool(req.current_password.clone(), password_hash).await {
        Ok(true) => {}
        Ok(false) => {
            return error_response(StatusCode::UNAUTHORIZED, "current password is incorrect")
        }
        Err(resp) => return resp,
    }

    let new_hash = match hash_on_blocking_pool(req.new_password.clone()).await {
        Ok(h) => h,
        Err(resp) => return resp,
    };

    match update_password_hash(&state, &account.0, &new_hash).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "update_password_hash failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// POST /auth/admin/reset-password  (admin only)
// ---------------------------------------------------------------------------

/// Admin-only: set another account's password without the target's current one.
/// Gated by [`is_admin`]; the target is looked up by username. Invalidates the
/// target's sessions so a reset always forces a fresh login.
#[tracing::instrument(skip_all, fields(admin = %account.0))]
pub async fn admin_reset_password(
    State(state): State<AppState>,
    account: AuthAccount,
    payload: Result<Json<AdminResetPasswordRequest>, JsonRejection>,
) -> Response {
    match is_admin(&state, &account.0).await {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::FORBIDDEN, "forbidden"),
        Err(e) => {
            tracing::error!(error = %e, "admin check failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }

    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    if let Err(msg) = validate_password(&req.new_password) {
        return error_response(StatusCode::BAD_REQUEST, msg);
    }

    let username_ci = req.username.trim().to_lowercase();
    let target = match account_by_username_ci(&state, &username_ci).await {
        Ok(Some((id, _hash, _username))) => id,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "no such user"),
        Err(e) => {
            tracing::error!(error = %e, "account lookup failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };

    let new_hash = match hash_on_blocking_pool(req.new_password.clone()).await {
        Ok(h) => h,
        Err(resp) => return resp,
    };
    if let Err(e) = update_password_hash(&state, &target, &new_hash).await {
        tracing::error!(error = %e, "update_password_hash failed");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
    }
    if let Err(e) = delete_sessions_for_account(&state, &target).await {
        tracing::warn!(error = %e, "post-reset session invalidation failed");
    }

    tracing::info!(admin = %account.0, target_account = %target, "admin reset a user's password");
    StatusCode::NO_CONTENT.into_response()
}

// ---------------------------------------------------------------------------
// POST /auth/security-question  (auth required)
// ---------------------------------------------------------------------------

/// Set (or replace) the caller's self-service recovery question + answer. The
/// answer is normalized then argon2id-hashed — never stored in the clear.
#[tracing::instrument(skip_all, fields(account = %account.0))]
pub async fn set_security_question(
    State(state): State<AppState>,
    account: AuthAccount,
    payload: Result<Json<SetSecurityQuestionRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };

    let question = req.question.trim().to_string();
    if !(1..=200).contains(&question.chars().count()) {
        return error_response(StatusCode::BAD_REQUEST, "question must be 1–200 characters");
    }
    let answer = normalize_answer(&req.answer);
    if answer.chars().count() < MIN_SECURITY_ANSWER_CHARS {
        return error_response(
            StatusCode::BAD_REQUEST,
            "answer must be at least 3 characters",
        );
    }

    let answer_hash = match hash_on_blocking_pool(answer).await {
        Ok(h) => h,
        Err(resp) => return resp,
    };

    let result = state
        .db
        .query(
            "UPDATE type::record('account', $account_id) SET
                security_question = $question,
                security_answer_hash = $answer_hash;",
        )
        .bind(("account_id", account.0.clone()))
        .bind(("question", question))
        .bind(("answer_hash", answer_hash))
        .await
        .and_then(|r| r.check());

    match result {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "set_security_question failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// GET /auth/reset/question?username=…   (public)
// POST /auth/reset/confirm              (public)
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
pub struct ResetQuestionQuery {
    username: String,
}

/// Public: return the security question for a username so the unauthenticated
/// reset form can show it. Returns `None` for both "no such user" and "no
/// question set" so the response can't be used to enumerate accounts.
#[tracing::instrument(skip_all)]
pub async fn get_reset_question(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<ResetQuestionQuery>,
) -> Response {
    let username_ci = q.username.trim().to_lowercase();
    let question = match security_question_for_username(&state, &username_ci).await {
        Ok(q) => q,
        Err(e) => {
            tracing::error!(error = %e, "security_question lookup failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    (StatusCode::OK, Json(ResetQuestionResponse { question })).into_response()
}

/// Public: reset a password by answering the security question. Unknown user,
/// no-question-set, and wrong-answer all return the same generic 401.
#[tracing::instrument(skip_all)]
pub async fn confirm_password_reset(
    State(state): State<AppState>,
    payload: Result<Json<ConfirmResetRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    if let Err(msg) = validate_password(&req.new_password) {
        return error_response(StatusCode::BAD_REQUEST, msg);
    }

    let username_ci = req.username.trim().to_lowercase();
    let (account_id, answer_hash) = match account_security(&state, &username_ci).await {
        Ok(Some((id, Some(hash)))) => (id, hash),
        Ok(Some((_, None))) | Ok(None) => return reset_rejected(),
        Err(e) => {
            tracing::error!(error = %e, "account_security lookup failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };

    match verify_on_blocking_pool(normalize_answer(&req.answer), answer_hash).await {
        Ok(true) => {}
        Ok(false) => return reset_rejected(),
        Err(resp) => return resp,
    }

    let new_hash = match hash_on_blocking_pool(req.new_password.clone()).await {
        Ok(h) => h,
        Err(resp) => return resp,
    };
    if let Err(e) = update_password_hash(&state, &account_id, &new_hash).await {
        tracing::error!(error = %e, "update_password_hash failed");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
    }
    if let Err(e) = delete_sessions_for_account(&state, &account_id).await {
        tracing::warn!(error = %e, "post-reset session invalidation failed");
    }

    tracing::info!(account = %account_id, "self-service password reset");
    StatusCode::NO_CONTENT.into_response()
}

/// The single generic rejection for the public reset path (no enumeration).
fn reset_rejected() -> Response {
    error_response(StatusCode::UNAUTHORIZED, "could not verify your answer")
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

/// The stored argon2 `password_hash` for an account id, if the row exists.
async fn account_password_hash(
    state: &AppState,
    account_id: &str,
) -> surrealdb::Result<Option<String>> {
    #[derive(SurrealValue)]
    struct Row {
        password_hash: String,
    }
    let mut resp = state
        .db
        .query("SELECT password_hash FROM type::record('account', $account_id);")
        .bind(("account_id", account_id.to_string()))
        .await?
        .check()?;
    let row: Option<Row> = resp.take(0)?;
    Ok(row.map(|r| r.password_hash))
}

async fn update_password_hash(
    state: &AppState,
    account_id: &str,
    password_hash: &str,
) -> surrealdb::Result<()> {
    state
        .db
        .query("UPDATE type::record('account', $account_id) SET password_hash = $password_hash;")
        .bind(("account_id", account_id.to_string()))
        .bind(("password_hash", password_hash.to_string()))
        .await?
        .check()?;
    Ok(())
}

/// The security question for a username (case-insensitive), if the account
/// exists AND has one set; `None` otherwise. The two cases are deliberately
/// indistinguishable to the caller.
async fn security_question_for_username(
    state: &AppState,
    username_ci: &str,
) -> surrealdb::Result<Option<String>> {
    #[derive(SurrealValue)]
    struct Row {
        security_question: Option<String>,
    }
    let mut resp = state
        .db
        .query("SELECT security_question FROM account WHERE username_ci = $username_ci;")
        .bind(("username_ci", username_ci.to_string()))
        .await?
        .check()?;
    let row: Option<Row> = resp.take(0)?;
    Ok(row.and_then(|r| r.security_question))
}

/// `(account_id, security_answer_hash)` for a username (case-insensitive), if
/// the account exists. The hash is `None` when no question has been set.
async fn account_security(
    state: &AppState,
    username_ci: &str,
) -> surrealdb::Result<Option<(String, Option<String>)>> {
    #[derive(SurrealValue)]
    struct Row {
        id_key: String,
        security_answer_hash: Option<String>,
    }
    let mut resp = state
        .db
        .query(
            "SELECT meta::id(id) AS id_key, security_answer_hash FROM account
                WHERE username_ci = $username_ci;",
        )
        .bind(("username_ci", username_ci.to_string()))
        .await?
        .check()?;
    let row: Option<Row> = resp.take(0)?;
    Ok(row.map(|r| (r.id_key, r.security_answer_hash)))
}

/// Delete every session row for `account_id`. Called after a password reset so
/// any pre-existing cookie (possibly an attacker's) stops authenticating.
async fn delete_sessions_for_account(state: &AppState, account_id: &str) -> surrealdb::Result<()> {
    state
        .db
        .query("DELETE FROM session WHERE account = type::record('account', $account_id);")
        .bind(("account_id", account_id.to_string()))
        .await?
        .check()?;
    Ok(())
}

/// Admin guard: fail-closed. The caller (by account id) is an admin iff their
/// stored `username_ci` is in the configured admin set — the union of
/// `AUTHLYN_ADMIN_USERNAMES` (comma/whitespace-separated) and the legacy
/// singular `AUTHLYN_ADMIN_USERNAME`, each trimmed and lowercased. An empty set
/// (neither var set, or both blank) authorizes no one.
pub(crate) async fn is_admin(state: &AppState, account_id: &str) -> surrealdb::Result<bool> {
    let admins = admin_username_set();
    if admins.is_empty() {
        return Ok(false);
    }
    #[derive(SurrealValue)]
    struct Row {
        username_ci: String,
    }
    let mut resp = state
        .db
        .query("SELECT username_ci FROM type::record('account', $account_id);")
        .bind(("account_id", account_id.to_string()))
        .await?
        .check()?;
    let row: Option<Row> = resp.take(0)?;
    Ok(row
        .map(|r| admins.contains(&r.username_ci))
        .unwrap_or(false))
}

/// Build the lowercased admin-username set from the environment (see [`is_admin`]).
pub(crate) fn admin_username_set() -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    if let Ok(list) = std::env::var("AUTHLYN_ADMIN_USERNAMES") {
        for entry in list.split([',', ' ', '\t', '\n', '\r']) {
            let e = entry.trim();
            if !e.is_empty() {
                set.insert(e.to_lowercase());
            }
        }
    }
    if let Ok(single) = std::env::var("AUTHLYN_ADMIN_USERNAME") {
        let e = single.trim();
        if !e.is_empty() {
            set.insert(e.to_lowercase());
        }
    }
    set
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
    validate_password(password)
}

/// The password length rule shared by register and change-password.
fn validate_password(password: &str) -> Result<(), &'static str> {
    if password.len() < MIN_PASSWORD_BYTES {
        return Err("password must be at least 8 characters");
    }
    if password.len() > MAX_PASSWORD_BYTES {
        return Err("password too long");
    }
    Ok(())
}

/// Normalize a security answer before hashing/verification: trim + lowercase,
/// so "Fluffy " and "fluffy" match. A deliberate usability-for-entropy trade.
fn normalize_answer(answer: &str) -> String {
    answer.trim().to_lowercase()
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
