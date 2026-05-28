//! Admin-only password-reset endpoint. Split from `server/auth.rs` in Wave 3;
//! behavior preserved verbatim.

use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::protocol::AdminResetPasswordRequest;
use crate::server::errors::{error_response, json_rejection_response};
use crate::server::permissions::is_admin;
use crate::server::state::AppState;

use super::crypto::{hash_on_blocking_pool, validate_password};
use super::password::{account_by_username_ci, update_password_hash};
use super::session::{delete_sessions_for_account, AuthAccount};

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
