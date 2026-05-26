//! Guilds (servers), their channels, and membership (phase-1 build step 2).
//!
//! Server-side state machine for "which guilds exist, who's in them, and what
//! channels they have." Modelled directly on the retired `server::rooms`:
//! read-only prechecks before any write, privacy-404s, and the
//! concurrent-write race against the `guild_member_pair (guild, account)`
//! UNIQUE index handled via [`with_write_conflict_retry`] +
//! [`is_unique_violation`] → 409.
//!
//! ## Authorization
//! - Membership-gated reads (`GET /guilds/{id}`) return `404 "guild not
//!   found"` to non-members — same body as a genuinely missing guild, so
//!   membership stays non-leaky.
//! - Mutations (channel/guild edits, invites, kicks) require the caller to be
//!   the guild **owner** (`role = 'owner'`): non-members get 404, members get
//!   403. Roles are minimal in phase 1 (`owner` | `member`).

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use surrealdb::types::SurrealValue;

use crate::protocol::{
    ChannelListResponse, ChannelSummary, CreateChannelRequest, CreateGuildRequest, ErrorBody,
    GuildDetail, GuildSummary, InviteMemberRequest, ListGuildsResponse, PatchChannelRequest,
    PatchGuildRequest, SetMemberRoleRequest,
};
use crate::server::auth::AuthAccount;
use crate::server::retry::{is_unique_violation, with_write_conflict_retry};
use crate::server::state::AppState;

const MAX_NAME_CHARS: usize = 100;
const CHANNEL_KINDS: [&str; 2] = ["text", "lorebook"];

// ---------------------------------------------------------------------------
// GET /guilds
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0))]
pub async fn list_guilds(State(state): State<AppState>, account: AuthAccount) -> Response {
    match load_my_guilds(&state, &account.0).await {
        Ok(guilds) => (StatusCode::OK, Json(ListGuildsResponse { guilds })).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "load_my_guilds failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

async fn load_my_guilds(state: &AppState, account: &str) -> surrealdb::Result<Vec<GuildSummary>> {
    #[derive(SurrealValue)]
    struct Row {
        id_key: String,
        name: String,
    }
    let mut resp = state
        .db
        .query(
            "SELECT meta::id(guild) AS id_key, guild.name AS name FROM guild_member
                WHERE account = type::record('account', $account)
                  AND guild.deleted_at = NONE;",
        )
        .bind(("account", account.to_string()))
        .await?
        .check()?;
    let rows: Vec<Row> = resp.take(0)?;
    Ok(rows
        .into_iter()
        .map(|r| GuildSummary {
            id: r.id_key,
            name: r.name,
        })
        .collect())
}

// ---------------------------------------------------------------------------
// POST /guilds
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, guild))]
pub async fn create_guild(
    State(state): State<AppState>,
    account: AuthAccount,
    payload: Result<Json<CreateGuildRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    let name = req.name.trim().to_string();
    if let Err(msg) = validate_name(&name) {
        return error_response(StatusCode::BAD_REQUEST, msg);
    }

    match persist_create_guild(&state, &account.0, &name).await {
        Ok(id) => {
            tracing::Span::current().record("guild", tracing::field::display(&id));
            (StatusCode::CREATED, Json(GuildSummary { id, name })).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "persist_create_guild failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

/// Atomically create the `guild`, the owner's `guild_member{role:'owner'}`,
/// and a default `'general'` text channel, in one BEGIN/COMMIT.
async fn persist_create_guild(
    state: &AppState,
    owner: &str,
    name: &str,
) -> surrealdb::Result<String> {
    #[derive(SurrealValue)]
    struct IdRow {
        id_key: String,
    }
    // Statement indices (BEGIN/COMMIT each consume one):
    //   0 BEGIN, 1 LET $owner, 2 LET $guild, 3 CREATE member,
    //   4 CREATE channel, 5 RETURN, 6 COMMIT.
    let sql = r#"
        BEGIN TRANSACTION;
        LET $owner = type::record("account", $owner_key);
        LET $guild = (CREATE guild SET name = $name, owner = $owner
            RETURN meta::id(id) AS id_key)[0].id_key;
        CREATE guild_member SET
            guild   = type::record("guild", $guild),
            account = $owner,
            role    = "owner";
        CREATE channel SET
            guild    = type::record("guild", $guild),
            name     = "general",
            kind     = "text",
            position = 0;
        RETURN { id_key: $guild };
        COMMIT TRANSACTION;
    "#;
    let row: Option<IdRow> = with_write_conflict_retry(|| async {
        let mut resp = state
            .db
            .query(sql)
            .bind(("owner_key", owner.to_string()))
            .bind(("name", name.to_string()))
            .await?
            .check()?;
        resp.take::<Option<IdRow>>(5)
    })
    .await?;
    row.map(|r| r.id_key)
        .ok_or_else(|| surrealdb::Error::thrown("persist_create_guild produced no row".to_string()))
}

// ---------------------------------------------------------------------------
// GET /guilds/{id}
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, guild = %gid))]
pub async fn get_guild(
    State(state): State<AppState>,
    Path(gid): Path<String>,
    account: AuthAccount,
) -> Response {
    // Membership gate (privacy 404 for non-members and missing guilds alike).
    match caller_role(&state, &gid, &account.0).await {
        Ok(Some(_)) => {}
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "guild not found"),
        Err(e) => {
            tracing::error!(error = %e, "caller_role failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }

    match load_guild_detail(&state, &gid).await {
        Ok(Some(detail)) => (StatusCode::OK, Json(detail)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "guild not found"),
        Err(e) => {
            tracing::error!(error = %e, "load_guild_detail failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

async fn load_guild_detail(state: &AppState, gid: &str) -> surrealdb::Result<Option<GuildDetail>> {
    #[derive(SurrealValue)]
    struct GuildRow {
        name: String,
        owner_key: String,
    }
    #[derive(SurrealValue)]
    struct ChanRow {
        id_key: String,
        name: String,
        kind: String,
        position: i64,
    }
    let mut resp = state
        .db
        .query(
            "SELECT name, meta::id(owner) AS owner_key FROM type::record('guild', $gid)
                WHERE deleted_at = NONE;
             SELECT meta::id(id) AS id_key, name, kind, position FROM channel
                WHERE guild = type::record('guild', $gid)
                  AND deleted_at = NONE ORDER BY position;",
        )
        .bind(("gid", gid.to_string()))
        .await?
        .check()?;
    let Some(g) = resp.take::<Option<GuildRow>>(0)? else {
        return Ok(None);
    };
    let chans: Vec<ChanRow> = resp.take(1)?;
    Ok(Some(GuildDetail {
        id: gid.to_string(),
        name: g.name,
        owner_id: g.owner_key,
        channels: chans
            .into_iter()
            .map(|c| ChannelSummary {
                id: c.id_key,
                name: c.name,
                kind: c.kind,
                position: c.position,
            })
            .collect(),
    }))
}

// ---------------------------------------------------------------------------
// PATCH /guilds/{id}
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, guild = %gid))]
pub async fn patch_guild(
    State(state): State<AppState>,
    Path(gid): Path<String>,
    account: AuthAccount,
    payload: Result<Json<PatchGuildRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    if let Err(r) = require_manager(&state, &gid, &account.0).await {
        return r;
    }

    if let Some(raw) = req.name {
        let name = raw.trim().to_string();
        if let Err(msg) = validate_name(&name) {
            return error_response(StatusCode::BAD_REQUEST, msg);
        }
        if let Err(e) = state
            .db
            .query("UPDATE type::record('guild', $gid) SET name = $name;")
            .bind(("gid", gid))
            .bind(("name", name))
            .await
            .and_then(|r| r.check())
        {
            tracing::error!(error = %e, "patch_guild update failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }
    StatusCode::NO_CONTENT.into_response()
}

// ---------------------------------------------------------------------------
// DELETE /guilds/{id}
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, guild = %gid))]
pub async fn delete_guild(
    State(state): State<AppState>,
    Path(gid): Path<String>,
    account: AuthAccount,
) -> Response {
    if let Err(r) = require_owner(&state, &gid, &account.0).await {
        return r;
    }
    // Soft-delete (#22): stamp deleted_at and leave channels/members/messages
    // intact so a restore brings the whole guild back. It vanishes from every
    // read (all filter deleted_at = NONE); the purge sweep hard-deletes it +
    // its channels/members after the 30d window.
    let result = with_write_conflict_retry(|| async {
        state
            .db
            .query("UPDATE type::record('guild', $gid) SET deleted_at = time::now();")
            .bind(("gid", gid.clone()))
            .await?
            .check()?;
        Ok(())
    })
    .await;
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "delete_guild failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// POST /guilds/{id}/channels
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, guild = %gid))]
pub async fn create_channel(
    State(state): State<AppState>,
    Path(gid): Path<String>,
    account: AuthAccount,
    payload: Result<Json<CreateChannelRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    if let Err(r) = require_manager(&state, &gid, &account.0).await {
        return r;
    }
    let name = req.name.trim().to_string();
    if let Err(msg) = validate_name(&name) {
        return error_response(StatusCode::BAD_REQUEST, msg);
    }
    if !CHANNEL_KINDS.contains(&req.kind.as_str()) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "channel kind must be text or lorebook",
        );
    }

    match insert_channel(&state, &gid, &name, &req.kind).await {
        Ok(summary) => (StatusCode::CREATED, Json(summary)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "insert_channel failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

async fn insert_channel(
    state: &AppState,
    gid: &str,
    name: &str,
    kind: &str,
) -> surrealdb::Result<ChannelSummary> {
    #[derive(SurrealValue)]
    struct IdRow {
        id_key: String,
    }
    // Append at the end: next position = current max + 1 (0 if no channels).
    let mut pos_resp = state
        .db
        .query(
            "SELECT VALUE position FROM channel
                WHERE guild = type::record('guild', $gid) ORDER BY position DESC LIMIT 1;",
        )
        .bind(("gid", gid.to_string()))
        .await?
        .check()?;
    let position = pos_resp.take::<Option<i64>>(0)?.map_or(0, |m| m + 1);

    let mut resp = state
        .db
        .query(
            "CREATE channel SET
                guild    = type::record('guild', $gid),
                name     = $name,
                kind     = $kind,
                position = $position
                RETURN meta::id(id) AS id_key;",
        )
        .bind(("gid", gid.to_string()))
        .bind(("name", name.to_string()))
        .bind(("kind", kind.to_string()))
        .bind(("position", position))
        .await?
        .check()?;
    let row: Option<IdRow> = resp.take(0)?;
    let id = row
        .map(|r| r.id_key)
        .ok_or_else(|| surrealdb::Error::thrown("insert_channel produced no row".to_string()))?;
    Ok(ChannelSummary {
        id,
        name: name.to_string(),
        kind: kind.to_string(),
        position,
    })
}

// ---------------------------------------------------------------------------
// PATCH /guilds/{id}/channels/{cid}
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, guild = %gid, channel = %cid))]
pub async fn patch_channel(
    State(state): State<AppState>,
    Path((gid, cid)): Path<(String, String)>,
    account: AuthAccount,
    payload: Result<Json<PatchChannelRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    if let Err(r) = require_manager(&state, &gid, &account.0).await {
        return r;
    }
    match channel_in_guild(&state, &gid, &cid).await {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::NOT_FOUND, "channel not found"),
        Err(e) => {
            tracing::error!(error = %e, "channel_in_guild failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }

    let mut sets: Vec<&str> = Vec::new();
    if let Some(ref raw) = req.name {
        if validate_name(raw.trim()).is_err() {
            return error_response(
                StatusCode::BAD_REQUEST,
                "channel name must be 1–100 characters",
            );
        }
        sets.push("name = $name");
    }
    if req.position.is_some() {
        sets.push("position = $position");
    }
    if sets.is_empty() {
        return StatusCode::NO_CONTENT.into_response();
    }

    let sql = format!(
        "UPDATE type::record('channel', $cid) SET {};",
        sets.join(", ")
    );
    let mut q = state.db.query(&sql).bind(("cid", cid));
    if let Some(raw) = req.name {
        q = q.bind(("name", raw.trim().to_string()));
    }
    if let Some(position) = req.position {
        q = q.bind(("position", position));
    }
    match q.await.and_then(|r| r.check()) {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "patch_channel update failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// DELETE /guilds/{id}/channels/{cid}
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, guild = %gid, channel = %cid))]
pub async fn delete_channel(
    State(state): State<AppState>,
    Path((gid, cid)): Path<(String, String)>,
    account: AuthAccount,
) -> Response {
    if let Err(r) = require_manager(&state, &gid, &account.0).await {
        return r;
    }
    match channel_in_guild(&state, &gid, &cid).await {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::NOT_FOUND, "channel not found"),
        Err(e) => {
            tracing::error!(error = %e, "channel_in_guild failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }
    // Soft-delete (#22): hidden by the deleted_at = NONE read filters; the
    // purge sweep removes it + its messages after the 1d window.
    match state
        .db
        .query("UPDATE type::record('channel', $cid) SET deleted_at = time::now();")
        .bind(("cid", cid))
        .await
        .and_then(|r| r.check())
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "delete_channel failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// Trash + restore (#22 soft-delete)
// ---------------------------------------------------------------------------

/// GET /guilds/trash — the caller's own soft-deleted guilds (owner only),
/// most-recently-trashed first. Recoverable until the purge sweep removes them.
#[tracing::instrument(skip_all, fields(account = %account.0))]
pub async fn list_deleted_guilds(State(state): State<AppState>, account: AuthAccount) -> Response {
    #[derive(SurrealValue)]
    struct Row {
        id_key: String,
        name: String,
    }
    let mut resp = match state
        .db
        .query(
            "SELECT meta::id(id) AS id_key, name FROM guild
                WHERE owner = type::record('account', $account)
                  AND deleted_at != NONE;",
        )
        .bind(("account", account.0))
        .await
        .and_then(|r| r.check())
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "list_deleted_guilds failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    let guilds = match resp.take::<Vec<Row>>(0) {
        Ok(rows) => rows
            .into_iter()
            .map(|r| GuildSummary {
                id: r.id_key,
                name: r.name,
            })
            .collect(),
        Err(e) => {
            tracing::error!(error = %e, "list_deleted_guilds decode failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    (StatusCode::OK, Json(ListGuildsResponse { guilds })).into_response()
}

/// POST /guilds/{id}/restore — un-delete a guild the caller owns (its
/// channels/members were left intact, so it returns whole).
#[tracing::instrument(skip_all, fields(account = %account.0, guild = %gid))]
pub async fn restore_guild(
    State(state): State<AppState>,
    Path(gid): Path<String>,
    account: AuthAccount,
) -> Response {
    // require_owner reads guild_member, which survives a soft-delete.
    if let Err(r) = require_owner(&state, &gid, &account.0).await {
        return r;
    }
    match state
        .db
        .query("UPDATE type::record('guild', $gid) SET deleted_at = NONE;")
        .bind(("gid", gid))
        .await
        .and_then(|r| r.check())
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "restore_guild failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

/// GET /guilds/{id}/trash/channels — soft-deleted channels in a guild (manager).
#[tracing::instrument(skip_all, fields(account = %account.0, guild = %gid))]
pub async fn list_deleted_channels(
    State(state): State<AppState>,
    Path(gid): Path<String>,
    account: AuthAccount,
) -> Response {
    if let Err(r) = require_manager(&state, &gid, &account.0).await {
        return r;
    }
    #[derive(SurrealValue)]
    struct ChanRow {
        id_key: String,
        name: String,
        kind: String,
        position: i64,
    }
    let mut resp = match state
        .db
        .query(
            "SELECT meta::id(id) AS id_key, name, kind, position FROM channel
                WHERE guild = type::record('guild', $gid)
                  AND deleted_at != NONE ORDER BY position;",
        )
        .bind(("gid", gid))
        .await
        .and_then(|r| r.check())
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "list_deleted_channels failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    let channels = match resp.take::<Vec<ChanRow>>(0) {
        Ok(rows) => rows
            .into_iter()
            .map(|c| ChannelSummary {
                id: c.id_key,
                name: c.name,
                kind: c.kind,
                position: c.position,
            })
            .collect(),
        Err(e) => {
            tracing::error!(error = %e, "list_deleted_channels decode failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    (StatusCode::OK, Json(ChannelListResponse { channels })).into_response()
}

/// POST /guilds/{id}/channels/{cid}/restore — un-delete a channel (manager).
#[tracing::instrument(skip_all, fields(account = %account.0, guild = %gid, channel = %cid))]
pub async fn restore_channel(
    State(state): State<AppState>,
    Path((gid, cid)): Path<(String, String)>,
    account: AuthAccount,
) -> Response {
    if let Err(r) = require_manager(&state, &gid, &account.0).await {
        return r;
    }
    // Scope the update to this guild so a manager can't revive an unrelated id.
    match state
        .db
        .query(
            "UPDATE type::record('channel', $cid) SET deleted_at = NONE
                WHERE guild = type::record('guild', $gid);",
        )
        .bind(("cid", cid))
        .bind(("gid", gid))
        .await
        .and_then(|r| r.check())
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "restore_channel failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// POST /guilds/{id}/members
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, guild = %gid, invitee))]
pub async fn invite_member(
    State(state): State<AppState>,
    Path(gid): Path<String>,
    account: AuthAccount,
    payload: Result<Json<InviteMemberRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    if let Err(r) = require_manager(&state, &gid, &account.0).await {
        return r;
    }
    let username_ci = req.username.trim().to_lowercase();
    if username_ci.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "username required");
    }
    tracing::Span::current().record("invitee", tracing::field::display(&username_ci));

    let target = match account_id_by_username_ci(&state, &username_ci).await {
        Ok(Some(id)) => id,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "user not found"),
        Err(e) => {
            tracing::error!(error = %e, "account lookup failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };

    // Pre-check: already a member? (Dual-path with the UNIQUE race below.)
    match caller_role(&state, &gid, &target).await {
        Ok(Some(_)) => return error_response(StatusCode::CONFLICT, "user is already a member"),
        Ok(None) => {}
        Err(e) => {
            tracing::error!(error = %e, "membership precheck failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }

    let result = with_write_conflict_retry(|| async {
        state
            .db
            .query(
                "CREATE guild_member SET
                    guild   = type::record('guild', $gid),
                    account = type::record('account', $target),
                    role    = 'member';",
            )
            .bind(("gid", gid.clone()))
            .bind(("target", target.clone()))
            .await?
            .check()?;
        Ok(())
    })
    .await;
    match result {
        Ok(()) => StatusCode::CREATED.into_response(),
        Err(e) if is_unique_violation(&e) => {
            error_response(StatusCode::CONFLICT, "user is already a member")
        }
        Err(e) => {
            tracing::error!(error = %e, "invite_member write failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// DELETE /guilds/{id}/members/{aid}
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, guild = %gid, target = %aid))]
pub async fn remove_member(
    State(state): State<AppState>,
    Path((gid, aid)): Path<(String, String)>,
    account: AuthAccount,
) -> Response {
    let caller_membership = match caller_role(&state, &gid, &account.0).await {
        Ok(Some(role)) => role,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "guild not found"),
        Err(e) => {
            tracing::error!(error = %e, "caller_role failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };

    if aid == account.0 {
        // Self-leave. The owner must delete the guild instead of orphaning it.
        if caller_membership == "owner" {
            return error_response(
                StatusCode::BAD_REQUEST,
                "owner cannot leave; delete the guild instead",
            );
        }
    } else {
        // Kicking someone else needs manage rights; the owner can't be kicked.
        if caller_membership != "owner" && caller_membership != "admin" {
            return error_response(StatusCode::FORBIDDEN, "admin only");
        }
        match caller_role(&state, &gid, &aid).await {
            Ok(Some(role)) if role == "owner" => {
                return error_response(StatusCode::FORBIDDEN, "cannot remove the owner")
            }
            Ok(Some(_)) => {}
            Ok(None) => return error_response(StatusCode::NOT_FOUND, "member not found"),
            Err(e) => {
                tracing::error!(error = %e, "target membership lookup failed");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
            }
        }
    }

    match state
        .db
        .query(
            "DELETE FROM guild_member
                WHERE guild = type::record('guild', $gid)
                  AND account = type::record('account', $aid);",
        )
        .bind(("gid", gid))
        .bind(("aid", aid))
        .await
        .and_then(|r| r.check())
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "remove_member failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// PUT /guilds/{id}/members/{aid}/role
// ---------------------------------------------------------------------------

/// Grant or revoke admin. Any manager (owner or admin) can promote a member
/// to `admin` or demote back to `member` — this is the easy, intended path to
/// share control. The owner's role is fixed (ownership transfer is out of
/// scope), so the owner can't be targeted.
#[tracing::instrument(skip_all, fields(account = %account.0, guild = %gid, target = %aid))]
pub async fn set_member_role(
    State(state): State<AppState>,
    Path((gid, aid)): Path<(String, String)>,
    account: AuthAccount,
    payload: Result<Json<SetMemberRoleRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    if req.role != "admin" && req.role != "member" {
        return error_response(StatusCode::BAD_REQUEST, "role must be admin or member");
    }
    if let Err(r) = require_manager(&state, &gid, &account.0).await {
        return r;
    }
    match caller_role(&state, &gid, &aid).await {
        Ok(Some(role)) if role == "owner" => {
            return error_response(StatusCode::FORBIDDEN, "cannot change the owner's role")
        }
        Ok(Some(_)) => {}
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "member not found"),
        Err(e) => {
            tracing::error!(error = %e, "target membership lookup failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }
    match state
        .db
        .query(
            "UPDATE guild_member SET role = $role
                WHERE guild = type::record('guild', $gid)
                  AND account = type::record('account', $aid);",
        )
        .bind(("role", req.role))
        .bind(("gid", gid))
        .bind(("aid", aid))
        .await
        .and_then(|r| r.check())
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "set_member_role failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// The caller's `role` in a guild, or `None` if they're not a member (which
/// callers map to a privacy-404 / 403 as appropriate).
pub(crate) async fn caller_role(
    state: &AppState,
    gid: &str,
    account: &str,
) -> surrealdb::Result<Option<String>> {
    #[derive(SurrealValue)]
    struct Row {
        role: String,
    }
    let mut resp = state
        .db
        .query(
            "SELECT role FROM guild_member
                WHERE guild = type::record('guild', $gid)
                  AND account = type::record('account', $account);",
        )
        .bind(("gid", gid.to_string()))
        .bind(("account", account.to_string()))
        .await?
        .check()?;
    let row: Option<Row> = resp.take(0)?;
    Ok(row.map(|r| r.role))
}

/// `Ok(())` if the caller can manage the guild (owner **or** admin);
/// otherwise an early-return response: 404 for non-members (privacy), 403 for
/// plain members. This gates the everyday management actions (channels,
/// invites, kicks, rename) — admins are deliberately near-peers of the owner
/// so granting admin is the easy, sufficient way to share control.
async fn require_manager(state: &AppState, gid: &str, account: &str) -> Result<(), Response> {
    match caller_role(state, gid, account).await {
        Ok(Some(role)) if role == "owner" || role == "admin" => Ok(()),
        Ok(Some(_)) => Err(error_response(StatusCode::FORBIDDEN, "admin only")),
        Ok(None) => Err(error_response(StatusCode::NOT_FOUND, "guild not found")),
        Err(e) => {
            tracing::error!(error = %e, "require_manager lookup failed");
            Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage error",
            ))
        }
    }
}

/// `Ok(())` only if the caller is the guild **owner**. Reserved for the few
/// irreversible/structural actions (deleting the guild).
async fn require_owner(state: &AppState, gid: &str, account: &str) -> Result<(), Response> {
    match caller_role(state, gid, account).await {
        Ok(Some(role)) if role == "owner" => Ok(()),
        Ok(Some(_)) => Err(error_response(StatusCode::FORBIDDEN, "owner only")),
        Ok(None) => Err(error_response(StatusCode::NOT_FOUND, "guild not found")),
        Err(e) => {
            tracing::error!(error = %e, "require_owner lookup failed");
            Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage error",
            ))
        }
    }
}

async fn channel_in_guild(state: &AppState, gid: &str, cid: &str) -> surrealdb::Result<bool> {
    #[derive(SurrealValue)]
    struct IdRow {
        id_key: String,
    }
    let mut resp = state
        .db
        .query(
            "SELECT meta::id(id) AS id_key FROM channel
                WHERE id = type::record('channel', $cid)
                  AND guild = type::record('guild', $gid)
                  AND deleted_at = NONE;",
        )
        .bind(("cid", cid.to_string()))
        .bind(("gid", gid.to_string()))
        .await?
        .check()?;
    Ok(resp.take::<Option<IdRow>>(0)?.is_some())
}

async fn account_id_by_username_ci(
    state: &AppState,
    username_ci: &str,
) -> surrealdb::Result<Option<String>> {
    #[derive(SurrealValue)]
    struct IdRow {
        id_key: String,
    }
    let mut resp = state
        .db
        .query("SELECT meta::id(id) AS id_key FROM account WHERE username_ci = $username_ci;")
        .bind(("username_ci", username_ci.to_string()))
        .await?
        .check()?;
    Ok(resp.take::<Option<IdRow>>(0)?.map(|r| r.id_key))
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
