//! User-driven password operations: self-service password change.
//! Split from `server/auth.rs` in Wave 3.

use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use surrealdb::types::SurrealValue;

use crate::protocol::ChangePasswordRequest;
use crate::server::errors::{error_response, json_rejection_response};
use crate::server::state::AppState;

use super::crypto::{hash_on_blocking_pool, validate_password, verify_on_blocking_pool};
use super::session::AuthAccount;

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
