//! Personas + gallery, and the per-guild "worn" persona (phase-1 build step 4).
//!
//! Personas are account-global: a user builds a library (image + name +
//! description + gallery) and "wears" one per guild via
//! `PUT /guilds/{id}/active-persona` (stored on `guild_member.active_persona`,
//! which `server::messages` stamps onto each message). All persona routes are
//! owner-scoped: another account's persona reads/writes as a privacy-404.
//! Images reuse `server::media` — endpoints here take an already-uploaded
//! `media_id`.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use surrealdb::types::SurrealValue;

use crate::protocol::{
    AddGalleryImageRequest, AddGalleryImageResponse, CreatePersonaRequest, ErrorBody, GalleryImage,
    ListPersonaEditorsResponse, ListPersonasResponse, PatchPersonaRequest, PersonaDetail,
    PersonaEditor, PersonaSummary, RedeemPersonaKeyRequest, SetActivePersonaRequest,
    SetAvatarRequest,
};
use crate::server::auth::AuthAccount;
use crate::server::retry::{is_unique_violation, with_write_conflict_retry};
use crate::server::state::AppState;

const MAX_NAME_CHARS: usize = 100;
const MAX_DESCRIPTION_CHARS: usize = 4000;

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
        avatar_id: Option<String>,
        owned: bool,
    }
    // Personas the caller owns OR can edit (a persona_editor row exists). The
    // editor set is resolved from the join table; `owned` flags which controls
    // the UI shows. Ordered by name for a stable wardrobe grid.
    let mut resp = state
        .db
        .query(
            "SELECT meta::id(id) AS id_key, name, description,
                (IF avatar != NONE THEN meta::id(avatar) ELSE NONE END) AS avatar_id,
                (owner = type::record('account', $account)) AS owned
                FROM persona
                WHERE owner = type::record('account', $account)
                   OR id IN (SELECT VALUE persona FROM persona_editor
                             WHERE account = type::record('account', $account))
                ORDER BY name;",
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
            owned: r.owned,
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

    #[derive(SurrealValue)]
    struct IdRow {
        id_key: String,
    }
    let share_key = random_share_key();
    let mut resp = match state
        .db
        .query(
            "CREATE persona SET
                owner = type::record('account', $account),
                name = $name,
                description = $description,
                share_key = $share_key
                RETURN meta::id(id) AS id_key;",
        )
        .bind(("account", account.0))
        .bind(("name", name.clone()))
        .bind(("description", description))
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
                owned: true,
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
            return error_response(StatusCode::BAD_REQUEST, "name must be 1–100 characters");
        }
        sets.push("name = $name");
    }
    if let Some(ref desc) = req.description {
        if desc.chars().count() > MAX_DESCRIPTION_CHARS {
            return error_response(StatusCode::BAD_REQUEST, "description too long");
        }
        sets.push("description = $description");
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
// PUT /personas/{id}/avatar
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, persona = %pid))]
pub async fn set_avatar(
    State(state): State<AppState>,
    Path(pid): Path<String>,
    account: AuthAccount,
    payload: Result<Json<SetAvatarRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    if let Err(resp) =
        require_owned_persona_and_media(&state, &pid, &account.0, &req.media_id).await
    {
        return resp;
    }
    match state
        .db
        .query(
            "UPDATE type::record('persona', $pid) SET avatar = type::record('media_blob', $mid);",
        )
        .bind(("pid", pid))
        .bind(("mid", req.media_id))
        .await
        .and_then(|r| r.check())
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "set_avatar failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// POST /personas/{id}/gallery  +  DELETE /personas/{id}/gallery/{img}
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, persona = %pid))]
pub async fn add_gallery_image(
    State(state): State<AppState>,
    Path(pid): Path<String>,
    account: AuthAccount,
    payload: Result<Json<AddGalleryImageRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    if let Err(resp) =
        require_owned_persona_and_media(&state, &pid, &account.0, &req.media_id).await
    {
        return resp;
    }

    match insert_gallery_image(&state, &pid, &req.media_id).await {
        Ok(id) => (StatusCode::CREATED, Json(AddGalleryImageResponse { id })).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "insert_gallery_image failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

async fn insert_gallery_image(
    state: &AppState,
    pid: &str,
    media_id: &str,
) -> surrealdb::Result<String> {
    #[derive(SurrealValue)]
    struct IdRow {
        id_key: String,
    }
    let mut pos_resp = state
        .db
        .query(
            "SELECT VALUE position FROM persona_image
                WHERE persona = type::record('persona', $pid) ORDER BY position DESC LIMIT 1;",
        )
        .bind(("pid", pid.to_string()))
        .await?
        .check()?;
    let position = pos_resp.take::<Option<i64>>(0)?.map_or(0, |m| m + 1);

    let mut resp = state
        .db
        .query(
            "CREATE persona_image SET
                persona = type::record('persona', $pid),
                media = type::record('media_blob', $mid),
                position = $position
                RETURN meta::id(id) AS id_key;",
        )
        .bind(("pid", pid.to_string()))
        .bind(("mid", media_id.to_string()))
        .bind(("position", position))
        .await?
        .check()?;
    resp.take::<Option<IdRow>>(0)?
        .map(|r| r.id_key)
        .ok_or_else(|| surrealdb::Error::thrown("insert_gallery_image produced no row".to_string()))
}

#[tracing::instrument(skip_all, fields(account = %account.0, persona = %pid, image = %img))]
pub async fn remove_gallery_image(
    State(state): State<AppState>,
    Path((pid, img)): Path<(String, String)>,
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
    match state
        .db
        .query(
            "DELETE FROM persona_image
                WHERE id = type::record('persona_image', $img)
                  AND persona = type::record('persona', $pid);",
        )
        .bind(("img", img))
        .bind(("pid", pid))
        .await
        .and_then(|r| r.check())
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "remove_gallery_image failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// PUT /guilds/{id}/active-persona
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, guild = %gid))]
pub async fn set_active_persona(
    State(state): State<AppState>,
    Path(gid): Path<String>,
    account: AuthAccount,
    payload: Result<Json<SetActivePersonaRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };

    // Caller must be a member of the guild (privacy-404 otherwise).
    match is_guild_member(&state, &gid, &account.0).await {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::NOT_FOUND, "guild not found"),
        Err(e) => {
            tracing::error!(error = %e, "is_guild_member failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }

    if let Some(ref pid) = req.persona_id {
        // Editors (key-redeemed) may also wear the persona, not just the owner.
        match can_edit_persona(&state, pid, &account.0).await {
            Ok(true) => {}
            Ok(false) => return error_response(StatusCode::NOT_FOUND, "persona not found"),
            Err(e) => {
                tracing::error!(error = %e, "can_edit_persona failed");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
            }
        }
    }

    let outcome = match req.persona_id {
        Some(pid) => state
            .db
            .query(
                "UPDATE guild_member SET active_persona = type::record('persona', $pid)
                        WHERE guild = type::record('guild', $gid)
                          AND account = type::record('account', $account);",
            )
            .bind(("pid", pid))
            .bind(("gid", gid))
            .bind(("account", account.0))
            .await
            .and_then(|r| r.check()),
        None => state
            .db
            .query(
                "UPDATE guild_member SET active_persona = NONE
                        WHERE guild = type::record('guild', $gid)
                          AND account = type::record('account', $account);",
            )
            .bind(("gid", gid))
            .bind(("account", account.0))
            .await
            .and_then(|r| r.check()),
    };
    match outcome {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "set_active_persona failed");
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
// GET /personas/{id}/editors  +  DELETE /personas/{id}/editors/{aid}
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
// Shared helpers
// ---------------------------------------------------------------------------

async fn owns_persona(state: &AppState, pid: &str, account: &str) -> surrealdb::Result<bool> {
    #[derive(SurrealValue)]
    struct IdRow {
        id_key: String,
    }
    let mut resp = state
        .db
        .query(
            "SELECT meta::id(id) AS id_key FROM persona
                WHERE id = type::record('persona', $pid)
                  AND owner = type::record('account', $account);",
        )
        .bind(("pid", pid.to_string()))
        .bind(("account", account.to_string()))
        .await?
        .check()?;
    Ok(resp.take::<Option<IdRow>>(0)?.is_some())
}

/// True when `me` and `other` have an accepted friendship (either direction).
async fn is_accepted_friend(state: &AppState, me: &str, other: &str) -> surrealdb::Result<bool> {
    #[derive(SurrealValue)]
    struct IdRow {
        id_key: String,
    }
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

/// True when a `persona_editor` row links this persona to the account.
async fn is_persona_editor(state: &AppState, pid: &str, account: &str) -> surrealdb::Result<bool> {
    #[derive(SurrealValue)]
    struct IdRow {
        id_key: String,
    }
    let mut resp = state
        .db
        .query(
            "SELECT meta::id(id) AS id_key FROM persona_editor
                WHERE persona = type::record('persona', $pid)
                  AND account = type::record('account', $account);",
        )
        .bind(("pid", pid.to_string()))
        .bind(("account", account.to_string()))
        .await?
        .check()?;
    Ok(resp.take::<Option<IdRow>>(0)?.is_some())
}

/// Edit access = owner OR a redeemed editor. Used to gate PATCH + wear.
async fn can_edit_persona(state: &AppState, pid: &str, account: &str) -> surrealdb::Result<bool> {
    if owns_persona(state, pid, account).await? {
        return Ok(true);
    }
    is_persona_editor(state, pid, account).await
}

/// The editor roster for a persona (owner-only view).
async fn load_persona_editors(
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

async fn media_exists(state: &AppState, mid: &str) -> surrealdb::Result<bool> {
    #[derive(SurrealValue)]
    struct IdRow {
        id_key: String,
    }
    let mut resp = state
        .db
        .query("SELECT meta::id(id) AS id_key FROM type::record('media_blob', $mid);")
        .bind(("mid", mid.to_string()))
        .await?
        .check()?;
    Ok(resp.take::<Option<IdRow>>(0)?.is_some())
}

/// Owner-check the persona and existence-check the media id; map either miss
/// to the appropriate 404.
async fn require_owned_persona_and_media(
    state: &AppState,
    pid: &str,
    account: &str,
    media_id: &str,
) -> Result<(), Response> {
    match owns_persona(state, pid, account).await {
        Ok(true) => {}
        Ok(false) => return Err(error_response(StatusCode::NOT_FOUND, "persona not found")),
        Err(e) => {
            tracing::error!(error = %e, "owns_persona failed");
            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage error",
            ));
        }
    }
    match media_exists(state, media_id).await {
        Ok(true) => Ok(()),
        Ok(false) => Err(error_response(StatusCode::NOT_FOUND, "media not found")),
        Err(e) => {
            tracing::error!(error = %e, "media_exists failed");
            Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage error",
            ))
        }
    }
}

async fn is_guild_member(state: &AppState, gid: &str, account: &str) -> surrealdb::Result<bool> {
    #[derive(SurrealValue)]
    struct IdRow {
        id_key: String,
    }
    let mut resp = state
        .db
        .query(
            "SELECT meta::id(id) AS id_key FROM guild_member
                WHERE guild = type::record('guild', $gid)
                  AND account = type::record('account', $account);",
        )
        .bind(("gid", gid.to_string()))
        .bind(("account", account.to_string()))
        .await?
        .check()?;
    Ok(resp.take::<Option<IdRow>>(0)?.is_some())
}

fn validate_name(name: &str) -> Result<(), &'static str> {
    let n = name.chars().count();
    if n == 0 {
        return Err("name must not be empty");
    }
    if n > MAX_NAME_CHARS {
        return Err("name too long");
    }
    Ok(())
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
