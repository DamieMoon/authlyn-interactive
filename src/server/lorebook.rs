//! Lorebook entries — SillyTavern-style world info (phase-1 build step 5).
//!
//! Entries live on a `kind='lorebook'` channel and are collaborative: any
//! guild member may read and write them (they have no per-user owner — they're
//! shared world-state). A future AI layer will use `keys` (trigger keywords),
//! `content` (injected text), `enabled`, and `position` (insertion order);
//! phase 1 just stores and orders them. Ordering is by the integer `position`,
//! so the datetime cursor gotcha never applies here.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use surrealdb::types::SurrealValue;

use crate::protocol::{
    CreateLorebookEntryRequest, CreateLorebookEntryResponse, ErrorBody, ListLorebookResponse,
    LorebookEntry, PatchLorebookEntryRequest,
};
use crate::server::auth::AuthAccount;
use crate::server::state::AppState;

const MAX_TITLE_CHARS: usize = 200;
const MAX_CONTENT_CHARS: usize = 8000;
const MAX_KEYS: usize = 64;
const MAX_KEY_CHARS: usize = 100;

// ---------------------------------------------------------------------------
// GET /channels/{cid}/lorebook
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, channel = %cid))]
pub async fn list_entries(
    State(state): State<AppState>,
    Path(cid): Path<String>,
    account: AuthAccount,
) -> Response {
    if let Err(resp) = check_access(&state, &cid, &account.0).await {
        return resp;
    }
    match load_entries(&state, &cid).await {
        Ok(entries) => (StatusCode::OK, Json(ListLorebookResponse { entries })).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "load_entries failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

async fn load_entries(state: &AppState, cid: &str) -> surrealdb::Result<Vec<LorebookEntry>> {
    #[derive(SurrealValue)]
    struct Row {
        id_key: String,
        title: String,
        keys: Vec<String>,
        content: String,
        enabled: bool,
        position: i64,
    }
    let mut resp = state
        .db
        .query(
            "SELECT meta::id(id) AS id_key, title, keys, content, enabled, position
                FROM lorebook_entry WHERE channel = type::record('channel', $cid)
                ORDER BY position;",
        )
        .bind(("cid", cid.to_string()))
        .await?
        .check()?;
    let rows: Vec<Row> = resp.take(0)?;
    Ok(rows
        .into_iter()
        .map(|r| LorebookEntry {
            id: r.id_key,
            title: r.title,
            keys: r.keys,
            content: r.content,
            enabled: r.enabled,
            position: r.position,
        })
        .collect())
}

// ---------------------------------------------------------------------------
// POST /channels/{cid}/lorebook
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, channel = %cid))]
pub async fn create_entry(
    State(state): State<AppState>,
    Path(cid): Path<String>,
    account: AuthAccount,
    payload: Result<Json<CreateLorebookEntryRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    if let Err(resp) = check_access(&state, &cid, &account.0).await {
        return resp;
    }

    let title = req.title.unwrap_or_default();
    let content = req.content;
    let keys = req.keys;
    if let Err(msg) = validate_fields(&title, &content, &keys) {
        return error_response(StatusCode::BAD_REQUEST, msg);
    }
    let keys = normalize_keys(keys);
    let enabled = req.enabled.unwrap_or(true);

    let position = match req.position {
        Some(p) => p,
        None => match next_position(&state, &cid).await {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(error = %e, "next_position failed");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
            }
        },
    };

    #[derive(SurrealValue)]
    struct IdRow {
        id_key: String,
    }
    let mut resp = match state
        .db
        .query(
            "CREATE lorebook_entry SET
                channel = type::record('channel', $cid),
                title = $title,
                keys = $keys,
                content = $content,
                enabled = $enabled,
                position = $position
                RETURN meta::id(id) AS id_key;",
        )
        .bind(("cid", cid))
        .bind(("title", title))
        .bind(("keys", keys))
        .bind(("content", content))
        .bind(("enabled", enabled))
        .bind(("position", position))
        .await
        .and_then(|r| r.check())
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "create_entry failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    match resp.take::<Option<IdRow>>(0) {
        Ok(Some(row)) => (
            StatusCode::CREATED,
            Json(CreateLorebookEntryResponse { id: row.id_key }),
        )
            .into_response(),
        Ok(None) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error"),
        Err(e) => {
            tracing::error!(error = %e, "create_entry take failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// PATCH /channels/{cid}/lorebook/{eid}
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, channel = %cid, entry = %eid))]
pub async fn patch_entry(
    State(state): State<AppState>,
    Path((cid, eid)): Path<(String, String)>,
    account: AuthAccount,
    payload: Result<Json<PatchLorebookEntryRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    if let Err(resp) = check_access(&state, &cid, &account.0).await {
        return resp;
    }
    match entry_in_channel(&state, &cid, &eid).await {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::NOT_FOUND, "entry not found"),
        Err(e) => {
            tracing::error!(error = %e, "entry_in_channel failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }

    let mut sets: Vec<&str> = Vec::new();
    if let Some(ref title) = req.title {
        if title.chars().count() > MAX_TITLE_CHARS {
            return error_response(StatusCode::BAD_REQUEST, "title too long");
        }
        sets.push("title = $title");
    }
    if let Some(ref content) = req.content {
        if content.trim().is_empty() || content.chars().count() > MAX_CONTENT_CHARS {
            return error_response(StatusCode::BAD_REQUEST, "content must be 1–8000 characters");
        }
        sets.push("content = $content");
    }
    if let Some(ref keys) = req.keys {
        if keys.len() > MAX_KEYS || keys.iter().any(|k| k.chars().count() > MAX_KEY_CHARS) {
            return error_response(StatusCode::BAD_REQUEST, "too many / too-long keys");
        }
        sets.push("keys = $keys");
    }
    if req.enabled.is_some() {
        sets.push("enabled = $enabled");
    }
    if req.position.is_some() {
        sets.push("position = $position");
    }
    if sets.is_empty() {
        return StatusCode::NO_CONTENT.into_response();
    }

    let sql = format!(
        "UPDATE type::record('lorebook_entry', $eid) SET {};",
        sets.join(", ")
    );
    let mut q = state.db.query(&sql).bind(("eid", eid));
    if let Some(title) = req.title {
        q = q.bind(("title", title));
    }
    if let Some(content) = req.content {
        q = q.bind(("content", content));
    }
    if let Some(keys) = req.keys {
        q = q.bind(("keys", normalize_keys(keys)));
    }
    if let Some(enabled) = req.enabled {
        q = q.bind(("enabled", enabled));
    }
    if let Some(position) = req.position {
        q = q.bind(("position", position));
    }
    match q.await.and_then(|r| r.check()) {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "patch_entry failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// DELETE /channels/{cid}/lorebook/{eid}
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, channel = %cid, entry = %eid))]
pub async fn delete_entry(
    State(state): State<AppState>,
    Path((cid, eid)): Path<(String, String)>,
    account: AuthAccount,
) -> Response {
    if let Err(resp) = check_access(&state, &cid, &account.0).await {
        return resp;
    }
    match state
        .db
        .query(
            "DELETE FROM lorebook_entry
                WHERE id = type::record('lorebook_entry', $eid)
                  AND channel = type::record('channel', $cid);",
        )
        .bind(("eid", eid))
        .bind(("cid", cid))
        .await
        .and_then(|r| r.check())
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "delete_entry failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Channel exists + caller is a member + channel is a lorebook channel.
/// Returns `Ok(())` to proceed, or the early-return response: privacy-404 for
/// missing channel / non-member, 400 for a non-lorebook channel.
async fn check_access(state: &AppState, cid: &str, account: &str) -> Result<(), Response> {
    #[derive(SurrealValue)]
    struct ChanRow {
        guild_key: String,
        kind: String,
    }
    #[derive(SurrealValue)]
    struct IdRow {
        id_key: String,
    }

    let mut resp = match state
        .db
        .query("SELECT meta::id(guild) AS guild_key, kind FROM type::record('channel', $cid);")
        .bind(("cid", cid.to_string()))
        .await
        .and_then(|r| r.check())
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "channel lookup failed");
            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage error",
            ));
        }
    };
    let chan: Option<ChanRow> = match resp.take(0) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "channel take failed");
            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage error",
            ));
        }
    };
    let Some(chan) = chan else {
        return Err(error_response(StatusCode::NOT_FOUND, "channel not found"));
    };

    let mut resp = match state
        .db
        .query(
            "SELECT meta::id(id) AS id_key FROM guild_member
                WHERE guild = type::record('guild', $gid)
                  AND account = type::record('account', $account);",
        )
        .bind(("gid", chan.guild_key))
        .bind(("account", account.to_string()))
        .await
        .and_then(|r| r.check())
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "membership lookup failed");
            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage error",
            ));
        }
    };
    let member: Option<IdRow> = resp.take(0).unwrap_or(None);
    if member.is_none() {
        return Err(error_response(StatusCode::NOT_FOUND, "channel not found"));
    }

    if chan.kind != "lorebook" {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "not a lorebook channel",
        ));
    }
    Ok(())
}

async fn entry_in_channel(state: &AppState, cid: &str, eid: &str) -> surrealdb::Result<bool> {
    #[derive(SurrealValue)]
    struct IdRow {
        id_key: String,
    }
    let mut resp = state
        .db
        .query(
            "SELECT meta::id(id) AS id_key FROM lorebook_entry
                WHERE id = type::record('lorebook_entry', $eid)
                  AND channel = type::record('channel', $cid);",
        )
        .bind(("eid", eid.to_string()))
        .bind(("cid", cid.to_string()))
        .await?
        .check()?;
    Ok(resp.take::<Option<IdRow>>(0)?.is_some())
}

async fn next_position(state: &AppState, cid: &str) -> surrealdb::Result<i64> {
    let mut resp = state
        .db
        .query(
            "SELECT VALUE position FROM lorebook_entry
                WHERE channel = type::record('channel', $cid) ORDER BY position DESC LIMIT 1;",
        )
        .bind(("cid", cid.to_string()))
        .await?
        .check()?;
    Ok(resp.take::<Option<i64>>(0)?.map_or(0, |m| m + 1))
}

fn validate_fields(title: &str, content: &str, keys: &[String]) -> Result<(), &'static str> {
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err("title too long");
    }
    if content.trim().is_empty() {
        return Err("content must not be empty");
    }
    if content.chars().count() > MAX_CONTENT_CHARS {
        return Err("content too long");
    }
    if keys.len() > MAX_KEYS {
        return Err("too many keys");
    }
    if keys.iter().any(|k| k.chars().count() > MAX_KEY_CHARS) {
        return Err("a key is too long");
    }
    Ok(())
}

/// Trim keys and drop empties.
fn normalize_keys(keys: Vec<String>) -> Vec<String> {
    keys.into_iter()
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .collect()
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
