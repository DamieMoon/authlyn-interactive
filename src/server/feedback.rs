//! Feedback / bug-report endpoints (#31 — submit side).
//!
//! Two routes:
//!
//! - `POST /feedback`  — ANY authenticated account submits a feedback item.
//!   Body is validated (1–4000 chars) and `kind` is coerced to the allowed
//!   set (`bug` | `idea` | `other`). The caller's account id is taken from
//!   the session extractor, never from the request body.
//!
//! - `GET  /feedback`  — ADMIN-ONLY: returns all rows newest-first with the
//!   author's username joined. Admins are configured via the environment:
//!   `AUTHLYN_ADMIN_USERNAMES` (a comma/whitespace-separated list) plus the
//!   legacy singular `AUTHLYN_ADMIN_USERNAME` (both are unioned). A caller is
//!   authorized iff their username matches any configured entry
//!   case-insensitively. If neither var is set (or both empty) no one is
//!   authorized and the endpoint responds 403 (fail-closed).

use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use surrealdb::types::{Datetime, SurrealValue};

use crate::protocol::{ErrorBody, FeedbackItem, ListFeedbackResponse, SubmitFeedbackRequest};
use crate::server::auth::AuthAccount;
use crate::server::datetime::to_rfc3339_fixed;
use crate::server::state::AppState;

// ---------------------------------------------------------------------------
// POST /feedback
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0))]
pub async fn submit_feedback(
    State(state): State<AppState>,
    account: AuthAccount,
    payload: Result<Json<SubmitFeedbackRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };

    let body = req.body.trim().to_string();
    if body.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "body must not be empty");
    }
    if body.chars().count() > 4000 {
        return error_response(
            StatusCode::BAD_REQUEST,
            "body must be at most 4000 characters",
        );
    }

    let kind = coerce_kind(&req.kind);
    let context = req.context.filter(|s| !s.is_empty());

    let caller = account.0.clone();
    let result = state
        .db
        .query(
            "CREATE feedback SET
                author  = type::record('account', $author),
                kind    = $kind,
                body    = $body,
                context = $context;",
        )
        .bind(("author", caller))
        .bind(("kind", kind))
        .bind(("body", body))
        .bind(("context", context))
        .await
        .and_then(|r| r.check());

    match result {
        Ok(_) => StatusCode::CREATED.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "submit_feedback write failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// GET /feedback (admin only)
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0))]
pub async fn list_feedback(State(state): State<AppState>, account: AuthAccount) -> Response {
    match is_admin(&state, &account.0).await {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::FORBIDDEN, "forbidden"),
        Err(e) => {
            tracing::error!(error = %e, "admin check failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }

    match load_feedback(&state).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "load_feedback failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

async fn load_feedback(state: &AppState) -> surrealdb::Result<ListFeedbackResponse> {
    #[derive(SurrealValue)]
    struct Row {
        id: String,
        author_username: String,
        kind: String,
        body: String,
        context: Option<String>,
        status: String,
        created_at: Datetime,
    }

    let mut resp = state
        .db
        .query(
            "SELECT
                meta::id(id) AS id,
                author.username AS author_username,
                kind,
                body,
                context,
                status,
                created_at
            FROM feedback
            ORDER BY created_at DESC;",
        )
        .await?
        .check()?;

    let rows: Vec<Row> = resp.take(0)?;
    let items = rows
        .into_iter()
        .map(|r| FeedbackItem {
            id: r.id,
            author_username: r.author_username,
            kind: r.kind,
            body: r.body,
            context: r.context,
            status: r.status,
            created_at: to_rfc3339_fixed(r.created_at),
        })
        .collect();

    Ok(ListFeedbackResponse { items })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Coerce the caller-supplied kind to the allowed set; unknown values → "other".
fn coerce_kind(kind: &str) -> &'static str {
    match kind {
        "bug" => "bug",
        "idea" => "idea",
        _ => "other",
    }
}

/// Admin guard: fail-closed. The caller (identified by account id) is an admin
/// iff their stored `username_ci` is in the configured admin set. The set is the
/// union of `AUTHLYN_ADMIN_USERNAMES` (comma/whitespace-separated) and the legacy
/// singular `AUTHLYN_ADMIN_USERNAME`, each entry trimmed and lowercased. If the
/// set is empty (neither var set, or both blank) no one is authorized.
async fn is_admin(state: &AppState, account_id: &str) -> surrealdb::Result<bool> {
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

/// Build the lowercased admin-username set from the environment. Unions
/// `AUTHLYN_ADMIN_USERNAMES` (comma- and/or whitespace-separated) with the
/// legacy singular `AUTHLYN_ADMIN_USERNAME`. Entries are trimmed, lowercased,
/// and empties dropped.
fn admin_username_set() -> std::collections::HashSet<String> {
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
