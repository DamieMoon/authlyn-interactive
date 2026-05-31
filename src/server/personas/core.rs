//! Persona core CRUD: list/create/get/patch/delete, share-key redemption,
//! and the editor's "leave" path. Split from `server/personas.rs` in Wave 3;
//! behavior preserved verbatim.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use surrealdb::types::SurrealValue;

use crate::protocol::{
    CreatePersonaRequest, GalleryImage, ListPersonasResponse, PatchPersonaRequest, PersonaDetail,
    PersonaSummary, RedeemPersonaKeyRequest,
};
use crate::server::auth::AuthAccount;
use crate::server::db_helpers::IdRow;
use crate::server::errors::{error_response, json_rejection_response};
use crate::server::permissions::{can_edit_persona, is_persona_editor, owns_persona};
use crate::server::retry::{is_unique_violation, with_write_conflict_retry};
use crate::server::state::AppState;
use crate::server::validate::validate_name;

use super::editors::load_persona_editors;

const MAX_DESCRIPTION_CHARS: usize = 50_000;

/// Mint a url-safe random share key (32 bytes → 43 base64url-no-pad chars).
fn random_share_key() -> String {
    use base64::Engine;
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

// ---------------------------------------------------------------------------
// GET /personas
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0))]
pub async fn list_personas(State(state): State<AppState>, account: AuthAccount) -> Response {
    match load_personas(&state, &account.0).await {
        Ok(personas) => (StatusCode::OK, Json(ListPersonasResponse { personas })).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "load_personas failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

async fn load_personas(state: &AppState, account: &str) -> surrealdb::Result<Vec<PersonaSummary>> {
    #[derive(SurrealValue)]
    struct Row {
        id_key: String,
        name: String,
        description: String,
        // Optional: personas created before the `color` field existed have no
        // value yet (SCHEMAFULL DEFAULT only applies on write), so a non-option
        // String would fail to deserialize and 500 the whole wardrobe.
        color: Option<String>,
        avatar_id: Option<String>,
        owned: bool,
        // Raw position (NONE on rows predating the field). Echoed to the client
        // so a reload preserves the persisted order without a re-fetch.
        position: Option<i64>,
        // Coalesced sort key: NONE → a sentinel that sorts after every real
        // position so unordered rows fall to the end of the grid.
        sort_pos: i64,
    }
    // Personas the caller owns OR can edit (a persona_editor row exists). The
    // editor set is resolved from the join table; `owned` flags which controls
    // the UI shows. Ordered by the persisted `position`; rows without one (old
    // data, NONE) coalesce to a large sentinel so they sort last, tie-broken by
    // name for a stable grid.
    let mut resp = state
        .db
        .query(
            "SELECT meta::id(id) AS id_key, name, description, (color ?? '') AS color,
                (IF avatar != NONE THEN meta::id(avatar) ELSE NONE END) AS avatar_id,
                (owner = type::record('account', $account)) AS owned,
                position,
                (position ?? 9223372036854775807) AS sort_pos
                FROM persona
                WHERE owner = type::record('account', $account)
                   OR id IN (SELECT VALUE persona FROM persona_editor
                             WHERE account = type::record('account', $account))
                ORDER BY sort_pos, name;",
        )
        .bind(("account", account.to_string()))
        .await?
        .check()?;
    let rows: Vec<Row> = resp.take(0)?;
    Ok(rows
        .into_iter()
        .map(|r| PersonaSummary {
            id: r.id_key,
            name: r.name,
            description: r.description,
            avatar_id: r.avatar_id,
            color: r.color.unwrap_or_default(),
            owned: r.owned,
            position: r.position,
        })
        .collect())
}

// ---------------------------------------------------------------------------
// POST /personas
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0))]
pub async fn create_persona(
    State(state): State<AppState>,
    account: AuthAccount,
    payload: Result<Json<CreatePersonaRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    let name = req.name.trim().to_string();
    if let Err(msg) = validate_name(&name) {
        return error_response(StatusCode::BAD_REQUEST, msg);
    }
    let description = req.description.unwrap_or_default();
    if description.chars().count() > MAX_DESCRIPTION_CHARS {
        return error_response(StatusCode::BAD_REQUEST, "description too long");
    }
    let description_echo = description.clone();
    let color = req.color.unwrap_or_default();
    if !valid_color(&color) {
        return error_response(StatusCode::BAD_REQUEST, "invalid color");
    }
    let color_echo = color.clone();

    let share_key = random_share_key();
    let mut resp = match state
        .db
        .query(
            "CREATE persona SET
                owner = type::record('account', $account),
                name = $name,
                description = $description,
                color = $color,
                share_key = $share_key
                RETURN meta::id(id) AS id_key;",
        )
        .bind(("account", account.0))
        .bind(("name", name.clone()))
        .bind(("description", description))
        .bind(("color", color))
        .bind(("share_key", share_key))
        .await
        .and_then(|r| r.check())
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "create_persona failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    match resp.take::<Option<IdRow>>(0) {
        Ok(Some(row)) => (
            StatusCode::CREATED,
            Json(PersonaSummary {
                id: row.id_key,
                name,
                description: description_echo,
                avatar_id: None,
                color: color_echo,
                owned: true,
                position: None,
            }),
        )
            .into_response(),
        Ok(None) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error"),
        Err(e) => {
            tracing::error!(error = %e, "create_persona take failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// GET /personas/{id}
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, persona = %pid))]
pub async fn get_persona(
    State(state): State<AppState>,
    Path(pid): Path<String>,
    account: AuthAccount,
) -> Response {
    // Owner sees the share key + editor list; an editor (redeemed via key) sees
    // the persona but neither the key nor the editor roster.
    let is_owner = match owns_persona(&state, &pid, &account.0).await {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "owns_persona failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    if !is_owner {
        match is_persona_editor(&state, &pid, &account.0).await {
            Ok(true) => {}
            Ok(false) => return error_response(StatusCode::NOT_FOUND, "persona not found"),
            Err(e) => {
                tracing::error!(error = %e, "is_persona_editor failed");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
            }
        }
    }
    match load_persona_detail(&state, &pid, is_owner).await {
        Ok(Some(detail)) => (StatusCode::OK, Json(detail)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "persona not found"),
        Err(e) => {
            tracing::error!(error = %e, "load_persona_detail failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

async fn load_persona_detail(
    state: &AppState,
    pid: &str,
    is_owner: bool,
) -> surrealdb::Result<Option<PersonaDetail>> {
    #[derive(SurrealValue)]
    struct PRow {
        name: String,
        description: String,
        avatar_id: Option<String>,
        share_key: String,
    }
    #[derive(SurrealValue)]
    struct GRow {
        id_key: String,
        media_id: String,
        position: i64,
    }
    let mut resp = state
        .db
        .query(
            "SELECT name, description, share_key,
                (IF avatar != NONE THEN meta::id(avatar) ELSE NONE END) AS avatar_id
                FROM type::record('persona', $pid);
             SELECT meta::id(id) AS id_key, meta::id(media) AS media_id, position
                FROM persona_image WHERE persona = type::record('persona', $pid)
                ORDER BY position;",
        )
        .bind(("pid", pid.to_string()))
        .await?
        .check()?;
    let Some(p) = resp.take::<Option<PRow>>(0)? else {
        return Ok(None);
    };
    let gallery: Vec<GRow> = resp.take(1)?;
    // Owner-only fields: the share key (so editors can't re-share) and the
    // editor roster. Editors get `None` / empty.
    let (share_key, editors) = if is_owner {
        (Some(p.share_key), load_persona_editors(state, pid).await?)
    } else {
        (None, Vec::new())
    };
    Ok(Some(PersonaDetail {
        id: pid.to_string(),
        name: p.name,
        description: p.description,
        avatar_id: p.avatar_id,
        share_key,
        editors,
        gallery: gallery
            .into_iter()
            .map(|g| GalleryImage {
                id: g.id_key,
                media_id: g.media_id,
                position: g.position,
            })
            .collect(),
    }))
}

// ---------------------------------------------------------------------------
// PATCH /personas/{id}
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, persona = %pid))]
pub async fn patch_persona(
    State(state): State<AppState>,
    Path(pid): Path<String>,
    account: AuthAccount,
    payload: Result<Json<PatchPersonaRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    // Owner OR a redeemed editor may edit name/description.
    match can_edit_persona(&state, &pid, &account.0).await {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::NOT_FOUND, "persona not found"),
        Err(e) => {
            tracing::error!(error = %e, "can_edit_persona failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }

    let mut sets: Vec<&str> = Vec::new();
    if let Some(ref raw) = req.name {
        if validate_name(raw.trim()).is_err() {
            return error_response(StatusCode::BAD_REQUEST, "name must be 1-100 characters");
        }
        sets.push("name = $name");
    }
    if let Some(ref desc) = req.description {
        if desc.chars().count() > MAX_DESCRIPTION_CHARS {
            return error_response(StatusCode::BAD_REQUEST, "description too long");
        }
        sets.push("description = $description");
    }
    if let Some(ref color) = req.color {
        if !valid_color(color) {
            return error_response(StatusCode::BAD_REQUEST, "invalid color");
        }
        sets.push("color = $color");
    }
    if let Some(pos) = req.position {
        if pos < 0 {
            return error_response(StatusCode::BAD_REQUEST, "position must be >= 0");
        }
        sets.push("position = $position");
    }
    if sets.is_empty() {
        return StatusCode::NO_CONTENT.into_response();
    }

    let sql = format!(
        "UPDATE type::record('persona', $pid) SET {};",
        sets.join(", ")
    );
    let mut q = state.db.query(&sql).bind(("pid", pid));
    if let Some(raw) = req.name {
        q = q.bind(("name", raw.trim().to_string()));
    }
    if let Some(desc) = req.description {
        q = q.bind(("description", desc));
    }
    if let Some(color) = req.color {
        q = q.bind(("color", color));
    }
    if let Some(pos) = req.position {
        q = q.bind(("position", pos));
    }
    match q.await.and_then(|r| r.check()) {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "patch_persona failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// DELETE /personas/{id}
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, persona = %pid))]
pub async fn delete_persona(
    State(state): State<AppState>,
    Path(pid): Path<String>,
    account: AuthAccount,
) -> Response {
    match owns_persona(&state, &pid, &account.0).await {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::NOT_FOUND, "persona not found"),
        Err(e) => {
            tracing::error!(error = %e, "owns_persona failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }
    // Drop gallery rows and clear any "worn" references along with the persona.
    let sql = r#"
        BEGIN TRANSACTION;
        DELETE FROM persona_image WHERE persona = type::record("persona", $pid);
        DELETE FROM persona_editor WHERE persona = type::record("persona", $pid);
        UPDATE guild_member SET active_persona = NONE
            WHERE active_persona = type::record("persona", $pid);
        DELETE FROM channel_active_persona WHERE persona = type::record("persona", $pid);
        DELETE type::record("persona", $pid);
        COMMIT TRANSACTION;
    "#;
    match state
        .db
        .query(sql)
        .bind(("pid", pid))
        .await
        .and_then(|r| r.check())
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "delete_persona failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// POST /personas/redeem  — gain editor access via a share key
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0))]
pub async fn redeem_persona_key(
    State(state): State<AppState>,
    account: AuthAccount,
    payload: Result<Json<RedeemPersonaKeyRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    let key = req.key.trim().to_string();
    if key.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "key required");
    }

    // Resolve the persona + its owner from the key.
    #[derive(SurrealValue)]
    struct Row {
        id_key: String,
        owner_id: String,
    }
    let found = match state
        .db
        .query(
            "SELECT meta::id(id) AS id_key, meta::id(owner) AS owner_id
                FROM persona WHERE share_key = $key LIMIT 1;",
        )
        .bind(("key", key))
        .await
        .and_then(|mut r| r.take::<Option<Row>>(0))
    {
        Ok(row) => row,
        Err(e) => {
            tracing::error!(error = %e, "redeem lookup failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    let Some(persona) = found else {
        return error_response(StatusCode::NOT_FOUND, "no such key");
    };
    if persona.owner_id == account.0 {
        return error_response(StatusCode::CONFLICT, "you own this persona");
    }

    let caller = account.0.clone();
    let pid = persona.id_key.clone();
    let result = with_write_conflict_retry(|| async {
        state
            .db
            .query(
                "CREATE persona_editor SET
                    persona = type::record('persona', $pid),
                    account = type::record('account', $account);",
            )
            .bind(("pid", pid.clone()))
            .bind(("account", caller.clone()))
            .await?
            .check()?;
        Ok(())
    })
    .await;
    match result {
        Ok(()) => StatusCode::CREATED.into_response(),
        Err(e) if is_unique_violation(&e) => {
            error_response(StatusCode::CONFLICT, "already an editor")
        }
        Err(e) => {
            tracing::error!(error = %e, "redeem_persona_key write failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// DELETE /personas/{id}/leave  — an editor drops a shared persona from their list
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, persona = %pid))]
pub async fn leave_persona(
    State(state): State<AppState>,
    Path(pid): Path<String>,
    account: AuthAccount,
) -> Response {
    // Only an editor can leave; the owner deletes instead (and a non-editor
    // gets the same privacy-404 as an unknown persona).
    match is_persona_editor(&state, &pid, &account.0).await {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::NOT_FOUND, "persona not found"),
        Err(e) => {
            tracing::error!(error = %e, "is_persona_editor failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }
    // Drop the editor link and stop the caller "wearing" it anywhere.
    let sql = r#"
        BEGIN TRANSACTION;
        DELETE FROM persona_editor
            WHERE persona = type::record("persona", $pid)
              AND account = type::record("account", $account);
        UPDATE guild_member SET active_persona = NONE
            WHERE active_persona = type::record("persona", $pid)
              AND account = type::record("account", $account);
        DELETE FROM channel_active_persona
            WHERE persona = type::record("persona", $pid)
              AND account = type::record("account", $account);
        COMMIT TRANSACTION;
    "#;
    match state
        .db
        .query(sql)
        .bind(("pid", pid))
        .bind(("account", account.0))
        .await
        .and_then(|r| r.check())
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "leave_persona failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// Shared validators
// ---------------------------------------------------------------------------

/// A persona color is either empty (default tint) or one of the shared markup
/// palette names (red…gray) — the same set the chat `[color]` markup uses.
fn valid_color(c: &str) -> bool {
    c.is_empty() || crate::markup::Color::from_name(c).is_some()
}
