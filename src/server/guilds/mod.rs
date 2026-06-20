//! Guilds (servers), their channels, and membership.
//!
//! Wave-3 split of the original `server/guilds.rs` into focused submodules.
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
//!
//! ## Layout
//! - this module: list/create/get/patch + per-user rail order + guild trash list.
//! - [`channels`] — channel CRUD + trash/restore.
//! - [`membership`] — list/invite/kick/role changes.
//! - [`deletion`] — guild soft-delete + restore.

mod channels;
mod deletion;
mod icon;
mod membership;

// Route-table handlers keep their `crate::server::guilds::<fn>` paths via these
// re-exports.
pub use self::channels::{
    create_channel, delete_channel, list_deleted_channels, patch_channel, restore_channel,
};
pub use self::deletion::{delete_guild, restore_guild};
pub use self::icon::set_guild_icon;
pub use self::membership::{invite_member, list_members, remove_member, set_member_role};

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use surrealdb::types::SurrealValue;

use crate::protocol::{
    ChannelSummary, CreateGuildRequest, GuildDetail, GuildSummary, ListGuildsResponse,
    PatchGuildRequest, RailOrderRequest, SyncEvent,
};
use crate::server::auth::AuthAccount;
use crate::server::db_helpers::IdRow;
use crate::server::errors::{error_response, json_rejection_response};
use crate::server::permissions::{caller_role, require_manager};
use crate::server::retry::with_write_conflict_retry;
use crate::server::state::AppState;
use crate::server::validate::validate_name;

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
        accent_color: Option<String>,
        icon_id: Option<String>,
    }
    #[derive(SurrealValue)]
    struct OrderRow {
        guild_key: String,
        position: i64,
    }
    // Personal rail order (#17/FB2). Two reads in one round-trip: the caller's
    // memberships, and their `user_guild_order` rows. We sort the memberships in
    // Rust by the per-guild position (guilds with no order row sort last via a
    // sentinel, `name` is the stable tiebreak). Sorting server-side would need a
    // correlated subquery; doing it here keeps the query trivially correct.
    let mut resp = state
        .db
        .query(
            "SELECT meta::id(guild) AS id_key, guild.name AS name, guild.accent_color AS accent_color,
                    (IF guild.icon != NONE THEN meta::id(guild.icon) ELSE NONE END) AS icon_id FROM guild_member
                WHERE account = type::record('account', $account)
                  AND guild.deleted_at = NONE;
             SELECT meta::id(guild) AS guild_key, position FROM user_guild_order
                WHERE account = type::record('account', $account);",
        )
        .bind(("account", account.to_string()))
        .await?
        .check()?;
    let rows: Vec<Row> = resp.take(0)?;
    let orders: Vec<OrderRow> = resp.take(1)?;
    let pos_of = |gid: &str| -> i64 {
        orders
            .iter()
            .find(|o| o.guild_key == gid)
            .map_or(i64::MAX, |o| o.position)
    };
    let mut guilds: Vec<GuildSummary> = rows
        .into_iter()
        .map(|r| GuildSummary {
            id: r.id_key,
            name: r.name,
            accent_color: r.accent_color.unwrap_or_default(),
            icon_id: r.icon_id,
        })
        .collect();
    guilds.sort_by(|a, b| {
        pos_of(&a.id)
            .cmp(&pos_of(&b.id))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(guilds)
}

// ---------------------------------------------------------------------------
// PUT /rail/order
// ---------------------------------------------------------------------------

/// Replace the caller's personal guild-rail order (#17/FB2). The body is the
/// full rail in display order; we validate every id is a guild the caller is a
/// member of (drops junk/stale ids), then delete the caller's existing
/// `user_guild_order` rows and insert one per id with its index as position —
/// all in one transaction so the rail never reads half-updated.
#[tracing::instrument(skip_all, fields(account = %account.0))]
pub async fn set_rail_order(
    State(state): State<AppState>,
    account: AuthAccount,
    payload: Result<Json<RailOrderRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };

    // Keep only ids the caller is actually a member of, preserving request order.
    let members: Vec<String> = match my_guild_ids(&state, &account.0).await {
        Ok(ids) => ids,
        Err(e) => {
            tracing::error!(error = %e, "my_guild_ids failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    let ordered: Vec<String> = req
        .guild_ids
        .into_iter()
        .filter(|gid| members.contains(gid))
        .collect();

    match persist_rail_order(&state, &account.0, &ordered).await {
        Ok(()) => {
            // M1.5: the rail order is a PER-USER preference — target the actor
            // so their other devices refresh, instead of broadcasting a global
            // ListsChanged to every connection (N×M amplification for a change
            // nobody else can even observe).
            state.emit_for(vec![account.0.clone()], SyncEvent::ListsChanged);
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "persist_rail_order failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

/// The record-id keys of every (live) guild the caller is a member of.
async fn my_guild_ids(state: &AppState, account: &str) -> surrealdb::Result<Vec<String>> {
    let mut resp = state
        .db
        .query(
            "SELECT VALUE meta::id(guild) FROM guild_member
                WHERE account = type::record('account', $account)
                  AND guild.deleted_at = NONE;",
        )
        .bind(("account", account.to_string()))
        .await?
        .check()?;
    resp.take::<Vec<String>>(0)
}

/// Wipe the caller's order rows and re-insert one per id (index = position),
/// in one transaction so the rail never reads half-updated. The CREATEs are
/// generated with per-row bind names (`$g0`, `$g1`, …) since the count is
/// dynamic; positions are the literal array index. Mirrors the BEGIN/COMMIT
/// shape of `persist_create_guild`.
async fn persist_rail_order(
    state: &AppState,
    account: &str,
    ordered: &[String],
) -> surrealdb::Result<()> {
    let mut sql = String::from(
        "BEGIN TRANSACTION;\n\
         DELETE user_guild_order WHERE account = type::record('account', $account);\n",
    );
    for i in 0..ordered.len() {
        sql.push_str(&format!(
            "CREATE user_guild_order SET \
                account = type::record('account', $account), \
                guild = type::record('guild', $g{i}), \
                position = {i};\n"
        ));
    }
    sql.push_str("COMMIT TRANSACTION;");

    let ordered = ordered.to_vec();
    let account = account.to_string();
    with_write_conflict_retry(|| {
        let sql = sql.clone();
        let ordered = ordered.clone();
        let account = account.clone();
        async move {
            let mut q = state.db.query(&sql).bind(("account", account));
            for (i, gid) in ordered.iter().enumerate() {
                q = q.bind((format!("g{i}"), gid.clone()));
            }
            q.await?.check()?;
            Ok(())
        }
    })
    .await
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
            // Review M-31: at creation the caller is the ONLY member, so no
            // other account's lists or visibility can change — target the
            // actor (their other devices) like `set_rail_order` above, never
            // the global lane (N connections × a visibility reload + three
            // client refetches for an event nobody else can observe). The
            // targeted lane reloads the recipient's visible set on
            // ListsChanged (events.rs), so the new guild's channel events
            // reach their pre-existing streams. Contrast invite/kick/
            // channel-create, which genuinely change ANOTHER party's lists
            // and stay broadcast.
            state.emit_for(vec![account.0.clone()], SyncEvent::ListsChanged);
            (
                StatusCode::CREATED,
                Json(GuildSummary {
                    id,
                    name,
                    accent_color: String::new(),
                    icon_id: None,
                }),
            )
                .into_response()
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
        accent_color: Option<String>,
        icon_id: Option<String>,
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
            "SELECT name, meta::id(owner) AS owner_key, accent_color,
                    (IF icon != NONE THEN meta::id(icon) ELSE NONE END) AS icon_id FROM type::record('guild', $gid)
                WHERE deleted_at = NONE;
             SELECT meta::id(id) AS id_key, name, kind, position, created_at FROM channel
                WHERE guild = type::record('guild', $gid)
                  AND deleted_at = NONE ORDER BY position, created_at;",
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
        accent_color: g.accent_color.unwrap_or_default(),
        icon_id: g.icon_id,
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
            .bind(("gid", gid.clone()))
            .bind(("name", name))
            .await
            .and_then(|r| r.check())
        {
            tracing::error!(error = %e, "patch_guild update failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
        // Inside the `if let`: a body without `name` mutates nothing → no emit.
        state.emit(SyncEvent::ListsChanged);
    }
    if let Some(raw) = req.accent_color {
        let Some(accent) = crate::server::accent::normalize_accent(&raw) else {
            return error_response(StatusCode::BAD_REQUEST, "invalid accent color");
        };
        if let Err(e) = state
            .db
            .query("UPDATE type::record('guild', $gid) SET accent_color = $accent;")
            .bind(("gid", gid.clone()))
            .bind(("accent", accent))
            .await
            .and_then(|r| r.check())
        {
            tracing::error!(error = %e, "patch_guild accent update failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
        state.emit(SyncEvent::ListsChanged);
    }
    StatusCode::NO_CONTENT.into_response()
}

// ---------------------------------------------------------------------------
// GET /guilds/trash  (#22 soft-delete listing)
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
                accent_color: String::new(),
                icon_id: None,
            })
            .collect(),
        Err(e) => {
            tracing::error!(error = %e, "list_deleted_guilds decode failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    (StatusCode::OK, Json(ListGuildsResponse { guilds })).into_response()
}
