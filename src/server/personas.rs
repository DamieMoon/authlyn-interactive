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
    ListPersonasResponse, PatchPersonaRequest, PersonaDetail, PersonaSummary,
    SetActivePersonaRequest, SetAvatarRequest,
};
use crate::server::auth::AuthAccount;
use crate::server::state::AppState;

const MAX_NAME_CHARS: usize = 100;
const MAX_DESCRIPTION_CHARS: usize = 4000;

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
    }
    let mut resp = state
        .db
        .query(
            "SELECT meta::id(id) AS id_key, name, description,
                (IF avatar != NONE THEN meta::id(avatar) ELSE NONE END) AS avatar_id
                FROM persona WHERE owner = type::record('account', $account) ORDER BY name;",
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
    let mut resp = match state
        .db
        .query(
            "CREATE persona SET
                owner = type::record('account', $account),
                name = $name,
                description = $description
                RETURN meta::id(id) AS id_key;",
        )
        .bind(("account", account.0))
        .bind(("name", name.clone()))
        .bind(("description", description))
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
    match owns_persona(&state, &pid, &account.0).await {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::NOT_FOUND, "persona not found"),
        Err(e) => {
            tracing::error!(error = %e, "owns_persona failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }
    match load_persona_detail(&state, &pid).await {
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
) -> surrealdb::Result<Option<PersonaDetail>> {
    #[derive(SurrealValue)]
    struct PRow {
        name: String,
        description: String,
        avatar_id: Option<String>,
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
            "SELECT name, description,
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
    Ok(Some(PersonaDetail {
        id: pid.to_string(),
        name: p.name,
        description: p.description,
        avatar_id: p.avatar_id,
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
    match owns_persona(&state, &pid, &account.0).await {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::NOT_FOUND, "persona not found"),
        Err(e) => {
            tracing::error!(error = %e, "owns_persona failed");
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
        match owns_persona(&state, pid, &account.0).await {
            Ok(true) => {}
            Ok(false) => return error_response(StatusCode::NOT_FOUND, "persona not found"),
            Err(e) => {
                tracing::error!(error = %e, "owns_persona failed");
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
