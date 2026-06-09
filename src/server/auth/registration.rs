//! Public account-creation + session-bound profile endpoints:
//! `POST /auth/register`, `POST /auth/login`, `POST /auth/logout`,
//! `GET /auth/me`. Split from `server/auth.rs` in Wave 3; behavior preserved.

use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use axum_extra::extract::cookie::{Cookie, CookieJar};
use surrealdb::types::SurrealValue;

use crate::protocol::{AuthResponse, LoginRequest, MeResponse, RegisterRequest};
use crate::server::db_helpers::IdRow;
use crate::server::errors::{error_response, json_rejection_response};
use crate::server::retry::{is_unique_violation, with_write_conflict_retry};
use crate::server::state::AppState;

use super::crypto::{hash_on_blocking_pool, validate_credentials, verify_on_blocking_pool};
use super::session::{issue_session, session_cookie, AuthAccount, SESSION_COOKIE};

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
    let row = match super::password::account_by_username_ci(&state, &username_ci).await {
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

pub(super) fn invalid_credentials() -> Response {
    error_response(StatusCode::UNAUTHORIZED, "invalid username or password")
}

// ---------------------------------------------------------------------------
// POST /auth/logout
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all)]
pub async fn logout(State(state): State<AppState>, jar: CookieJar) -> Response {
    if let Some(token) = jar.get(SESSION_COOKIE).map(|c| c.value().to_owned()) {
        let token_hash = super::crypto::sha256_hex(token.as_bytes());
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
    let (username, display_name) = match account_profile(&state, &account.0).await {
        Ok(Some(profile)) => profile,
        Ok(None) => return error_response(StatusCode::UNAUTHORIZED, "authentication required"),
        Err(e) => {
            tracing::error!(error = %e, "account_profile failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    // App-admin flag — gates the Nova DOT system-broadcast composer (and any
    // other admin-only UI) client-side. Fail-closed like every other admin check.
    let is_admin = match crate::server::permissions::is_admin(&state, &account.0).await {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "admin check failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    (
        StatusCode::OK,
        Json(MeResponse {
            account_id: account.0,
            username,
            display_name,
            is_admin,
        }),
    )
        .into_response()
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
    // Wrap the racy CREATE against the `account_username_ci` UNIQUE index in the
    // write-conflict retry: two concurrent registrations of the same username can
    // make the MVCC loser surface a (retryable) write conflict instead of the
    // UNIQUE violation. Retrying against a fresh snapshot then surfaces the clean
    // UNIQUE violation, which the caller maps to 409 (inv13) — never a 500.
    with_write_conflict_retry(|| async {
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
    })
    .await
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
