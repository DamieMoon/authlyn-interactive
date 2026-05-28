//! User-driven password operations: self-service change, set/look up security
//! question, and the public password-reset flow.
//! Split from `server/auth.rs` in Wave 3; behavior preserved verbatim.

use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use surrealdb::types::SurrealValue;

use crate::protocol::{
    ChangePasswordRequest, ConfirmResetRequest, ResetQuestionResponse, SetSecurityQuestionRequest,
};
use crate::server::errors::{error_response, json_rejection_response};
use crate::server::state::AppState;

use super::crypto::{hash_on_blocking_pool, validate_password, verify_on_blocking_pool};
use super::session::{delete_sessions_for_account, AuthAccount};

const MIN_SECURITY_ANSWER_CHARS: usize = 3;

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
// DB helpers (shared with login + admin)
// ---------------------------------------------------------------------------

/// Returns `(account_id, password_hash, stored_username)` for a username
/// (matched case-insensitively), if one exists.
pub(super) async fn account_by_username_ci(
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

pub(super) async fn update_password_hash(
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

/// Normalize a security answer before hashing/verification: trim + lowercase,
/// so "Fluffy " and "fluffy" match. A deliberate usability-for-entropy trade.
fn normalize_answer(answer: &str) -> String {
    answer.trim().to_lowercase()
}
