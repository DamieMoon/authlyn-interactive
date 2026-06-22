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

use crate::protocol::{
    AuthResponse, LoginRequest, MeResponse, PatchAccountRequest, RegisterRequest, SyncEvent,
};
use crate::server::db_helpers::IdRow;
use crate::server::errors::{error_response, json_rejection_response};
use crate::server::retry::{is_unique_violation, with_write_conflict_retry};
use crate::server::state::AppState;

use super::crypto::{hash_on_blocking_pool, validate_credentials, verify_on_blocking_pool};
use super::session::{issue_session, session_cookie, AuthAccount, SESSION_COOKIE};

// ---------------------------------------------------------------------------
// POST /auth/register
// ---------------------------------------------------------------------------

/// POST /auth/register — mint an account + a session, returning 201 with the
/// new identity in the body and an `HttpOnly; Secure; SameSite=Lax` session
/// cookie. Validates the credentials (`validate_credentials`), argon2id-hashes
/// the password off the async runtime, then `create_account` against the
/// `account_username_ci` UNIQUE index. A racing duplicate username surfaces as
/// `is_unique_violation` → 409 (never a 500), the same body the pre-check would
/// give; the raw session token is handed to the browser exactly once here while
/// the DB only ever holds its SHA-256. Round-trip pinned by
/// `tests/auth.rs::register_sets_cookie_and_me_resolves_it`; the 409-not-500
/// race by `tests/auth.rs::concurrent_register_same_username_never_500s`.
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

/// POST /auth/login — verify credentials and mint a fresh session (200 +
/// session cookie). No-enumeration is the load-bearing invariant: an unknown
/// username and a wrong password return the **same** 401 body
/// (`invalid_credentials`), and login still runs the full argon2 verify shape on
/// the failure paths so timing/response can't distinguish the two. Pinned by
/// `tests/auth.rs::login_good_and_bad_credentials` (asserts the two 401 bodies
/// are byte-identical).
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

/// The single 401 body for **both** login failure modes (unknown user / wrong
/// password). Centralised so the no-enumeration contract can't drift apart
/// across the two branches — adding a distinct "no such user" message here is
/// the easy way to break it. See `login`.
pub(super) fn invalid_credentials() -> Response {
    error_response(StatusCode::UNAUTHORIZED, "invalid username or password")
}

// ---------------------------------------------------------------------------
// POST /auth/logout
// ---------------------------------------------------------------------------

/// POST /auth/logout — delete the caller's one session row and clear the
/// cookie, returning 204. Best-effort on the DB side: a failed DELETE just
/// leaves a row that expires on its own TTL, and the cookie is cleared
/// regardless. Identity comes only from the cookie (server-trusted); no body is
/// read. Session invalidation pinned by
/// `tests/auth.rs::logout_invalidates_the_session`.
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

/// GET /auth/me — resolve the session cookie to the caller's identity
/// (`MeResponse`: account id, username, display name, `is_admin`, avatar id).
/// The `AuthAccount` extractor is the only identity source; a missing/expired/
/// garbage cookie is a 401 before this body runs. `is_admin` is the fail-closed
/// env-driven check from `server::permissions` (not stored on the account) and
/// gates admin-only UI client-side. Pinned by
/// `tests/auth.rs::me_without_cookie_is_401` and
/// `tests/auth.rs::me_with_garbage_cookie_is_401`.
#[tracing::instrument(skip_all, fields(account = %account.0))]
pub async fn me(State(state): State<AppState>, account: AuthAccount) -> Response {
    let (username, display_name, avatar_id) = match account_profile(&state, &account.0).await {
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
            avatar_id,
        }),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// PATCH /account
// ---------------------------------------------------------------------------

/// PATCH /account (auth-required) — update the caller's own profile (M6/P2):
/// `display_name` (trimmed, 1–32) and/or `avatar` (a media id from `POST /media`).
/// Account-scoped: the `AuthAccount` extractor proves the caller and the UPDATE
/// targets only their own row, so there is no membership/manager gate and no
/// privacy-404 surface (you can only edit yourself). An empty body is a 204
/// no-op. On any change, broadcast `ListsChanged`: account identity is
/// live-resolved on every message (`author_display`/`author_avatar_id`), so a
/// rename/re-avatar alters this account's OLD messages in every shared channel —
/// other members must refetch (the frame carries ids only, never the new name).
#[tracing::instrument(skip_all, fields(account = %account.0))]
pub async fn patch_account(
    State(state): State<AppState>,
    account: AuthAccount,
    payload: Result<Json<PatchAccountRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    let aid = account.0;
    let mut changed = false;

    if let Some(raw) = req.display_name {
        let name = raw.trim().to_string();
        if let Err(msg) = crate::server::validate::validate_display_name(&name) {
            return error_response(StatusCode::BAD_REQUEST, msg);
        }
        if let Err(e) = state
            .db
            .query("UPDATE type::record('account', $aid) SET display_name = $name;")
            .bind(("aid", aid.clone()))
            .bind(("name", name))
            .await
            .and_then(|r| r.check())
        {
            tracing::error!(error = %e, "patch_account display_name update failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
        changed = true;
    }

    if let Some(media_id) = req.avatar {
        // Existence-check the media (privacy-404), same contract as set_avatar.
        match media_exists(&state, &media_id).await {
            Ok(true) => {}
            Ok(false) => return error_response(StatusCode::NOT_FOUND, "media not found"),
            Err(e) => {
                tracing::error!(error = %e, "media_exists failed");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
            }
        }
        if let Err(e) = state
            .db
            .query(
                "UPDATE type::record('account', $aid) SET avatar = type::record('media_blob', $mid);",
            )
            .bind(("aid", aid.clone()))
            .bind(("mid", media_id))
            .await
            .and_then(|r| r.check())
        {
            tracing::error!(error = %e, "patch_account avatar update failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
        changed = true;
    }

    if changed {
        state.emit(SyncEvent::ListsChanged);
    }
    StatusCode::NO_CONTENT.into_response()
}

// ---------------------------------------------------------------------------
// DB helpers
// ---------------------------------------------------------------------------

/// True iff a `media_blob` row exists for `mid` (the privacy-404 probe for the
/// account-avatar set; mirrors the persona-gallery / guild-icon checks).
async fn media_exists(state: &AppState, mid: &str) -> surrealdb::Result<bool> {
    let mut resp = state
        .db
        .query("SELECT meta::id(id) AS id_key FROM type::record('media_blob', $mid);")
        .bind(("mid", mid.to_string()))
        .await?
        .check()?;
    Ok(resp.take::<Option<IdRow>>(0)?.is_some())
}

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
) -> surrealdb::Result<Option<(String, String, Option<String>)>> {
    #[derive(SurrealValue)]
    struct Row {
        username: String,
        display_name: String,
        avatar_id: Option<String>,
    }
    let mut resp = state
        .db
        .query(
            "SELECT username, display_name,
                (IF avatar != NONE THEN meta::id(avatar) ELSE NONE END) AS avatar_id
                FROM type::record('account', $account_id);",
        )
        .bind(("account_id", account_id.to_string()))
        .await?
        .check()?;
    let row: Option<Row> = resp.take(0)?;
    Ok(row.map(|r| (r.username, r.display_name, r.avatar_id)))
}
