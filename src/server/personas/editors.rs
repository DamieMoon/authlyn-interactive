//! Persona editor roster: list, share-with-friend (PUT), revoke.
//! Split from `server/personas.rs` in Wave 3; behavior preserved verbatim.
//!
//! Owner-only endpoints — non-owners get the same privacy-404 as an unknown
//! persona. `add_editor` further restricts the target to an accepted friend.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use surrealdb::types::SurrealValue;

use crate::protocol::{ListPersonaEditorsResponse, PersonaEditor};
use crate::server::auth::AuthAccount;
use crate::server::db_helpers::IdRow;
use crate::server::errors::error_response;
use crate::server::permissions::owns_persona;
use crate::server::retry::{is_unique_violation, with_write_conflict_retry};
use crate::server::state::AppState;

// ---------------------------------------------------------------------------
// GET /personas/{id}/editors
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, persona = %pid))]
pub async fn list_editors(
    State(state): State<AppState>,
    Path(pid): Path<String>,
    account: AuthAccount,
) -> Response {
    // Owner-only roster.
    match owns_persona(&state, &pid, &account.0).await {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::NOT_FOUND, "persona not found"),
        Err(e) => {
            tracing::error!(error = %e, "owns_persona failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }
    match load_persona_editors(&state, &pid).await {
        Ok(editors) => {
            (StatusCode::OK, Json(ListPersonaEditorsResponse { editors })).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "load_persona_editors failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// DELETE /personas/{id}/editors/{aid}
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, persona = %pid, editor = %aid))]
pub async fn remove_editor(
    State(state): State<AppState>,
    Path((pid, aid)): Path<(String, String)>,
    account: AuthAccount,
) -> Response {
    // Owner-only: revoke an editor's access.
    match owns_persona(&state, &pid, &account.0).await {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::NOT_FOUND, "persona not found"),
        Err(e) => {
            tracing::error!(error = %e, "owns_persona failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }
    // Also stop the removed editor from "wearing" it anywhere.
    let sql = r#"
        BEGIN TRANSACTION;
        DELETE FROM persona_editor
            WHERE persona = type::record("persona", $pid)
              AND account = type::record("account", $aid);
        UPDATE guild_member SET active_persona = NONE
            WHERE active_persona = type::record("persona", $pid)
              AND account = type::record("account", $aid);
        DELETE FROM channel_active_persona
            WHERE persona = type::record("persona", $pid)
              AND account = type::record("account", $aid);
        COMMIT TRANSACTION;
    "#;
    match state
        .db
        .query(sql)
        .bind(("pid", pid))
        .bind(("aid", aid))
        .await
        .and_then(|r| r.check())
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "remove_editor failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// PUT /personas/{id}/editors/{aid}  — owner shares the persona with a friend
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, persona = %pid, editor = %aid))]
pub async fn add_editor(
    State(state): State<AppState>,
    Path((pid, aid)): Path<(String, String)>,
    account: AuthAccount,
) -> Response {
    // Owner-only, and you can only share with an accepted friend (the UI offers
    // exactly that set). `aid` is an opaque account id from the friends list.
    match owns_persona(&state, &pid, &account.0).await {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::NOT_FOUND, "persona not found"),
        Err(e) => {
            tracing::error!(error = %e, "owns_persona failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }
    match is_accepted_friend(&state, &account.0, &aid).await {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::BAD_REQUEST, "can only share with friends"),
        Err(e) => {
            tracing::error!(error = %e, "is_accepted_friend failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }

    let result = with_write_conflict_retry(|| async {
        state
            .db
            .query(
                "CREATE persona_editor SET
                    persona = type::record('persona', $pid),
                    account = type::record('account', $aid);",
            )
            .bind(("pid", pid.clone()))
            .bind(("aid", aid.clone()))
            .await?
            .check()?;
        Ok(())
    })
    .await;
    match result {
        // Already shared is success — the checkbox is just confirming the state.
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) if is_unique_violation(&e) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "add_editor write failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// True when `me` and `other` have an accepted friendship (either direction).
async fn is_accepted_friend(state: &AppState, me: &str, other: &str) -> surrealdb::Result<bool> {
    let mut resp = state
        .db
        .query(
            "SELECT meta::id(id) AS id_key FROM friendship
                WHERE state = 'accepted'
                  AND ((requester = type::record('account', $me)
                        AND addressee = type::record('account', $other))
                    OR (requester = type::record('account', $other)
                        AND addressee = type::record('account', $me)));",
        )
        .bind(("me", me.to_string()))
        .bind(("other", other.to_string()))
        .await?
        .check()?;
    Ok(resp.take::<Option<IdRow>>(0)?.is_some())
}

/// The editor roster for a persona (owner-only view).
pub(super) async fn load_persona_editors(
    state: &AppState,
    pid: &str,
) -> surrealdb::Result<Vec<PersonaEditor>> {
    #[derive(SurrealValue)]
    struct Row {
        account_id: String,
        username: String,
    }
    // Order by the projected username (SurrealDB requires the ORDER BY idiom to
    // be a selected field); editor ordering isn't otherwise load-bearing.
    let mut resp = state
        .db
        .query(
            "SELECT meta::id(account) AS account_id, account.username AS username
                FROM persona_editor WHERE persona = type::record('persona', $pid)
                ORDER BY username;",
        )
        .bind(("pid", pid.to_string()))
        .await?
        .check()?;
    let rows: Vec<Row> = resp.take(0)?;
    Ok(rows
        .into_iter()
        .map(|r| PersonaEditor {
            account_id: r.account_id,
            username: r.username,
        })
        .collect())
}
