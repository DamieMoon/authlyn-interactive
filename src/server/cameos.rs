//! Guest cameos (M7/P2) — scoped, ephemeral guest access to one guild text channel.
//!
//! Spec §9.7 "Guest Cameos": a guild member brings an accepted friend in for a
//! cameo — the guest wears their OWN persona, posts (guest-badged) in exactly ONE
//! channel, and access dies when revoked / expires. A cameo is a `channel_guest`
//! row (the third channel-membership model after `guild_member` and `dm_member`);
//! the message / read-state / persona / SSE / unread / push stack is INHERITED via
//! the resolvers that now branch to `channel_guest` (`access::resolve_membership`,
//! `access::channel_access`, `access::visible_channels`). This module owns only the
//! cameo LIFECYCLE: invite, list (host + guest views), leave, revoke.
//!
//! Owner rulings (2026-06-19): any guild member may invite their own accepted
//! friend; ephemerality is manual revoke + an optional `expires_at` enforced as a
//! lazy-check at every membership query; unfriending revokes the active cameo.
//! Non-member access is the identical privacy-404 as a guild channel.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use surrealdb::types::{Datetime, SurrealValue};

use crate::protocol::{
    CameoSummary, GuestSummary, InviteGuestRequest, ListCameosResponse, ListGuestsResponse,
    SyncEvent,
};
use crate::server::access::{resolve_membership, Membership};
use crate::server::auth::AuthAccount;
use crate::server::datetime::to_rfc3339_fixed;
use crate::server::errors::{error_response, json_rejection_response};
use crate::server::retry::{is_unique_violation, with_write_conflict_retry};
use crate::server::state::AppState;

/// Same body as the guild / DM privacy-404, so a non-member can't tell a channel
/// they can't see apart from one that doesn't exist.
fn not_found() -> Response {
    error_response(StatusCode::NOT_FOUND, "channel not found")
}

fn storage_error() -> Response {
    error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
}

// ---------------------------------------------------------------------------
// POST /channels/{cid}/guests — invite a guest
// ---------------------------------------------------------------------------

/// POST /channels/{cid}/guests — invite one accepted friend of the caller as a
/// guest in this guild text channel. The caller must be a guild MEMBER of the
/// channel's guild (a guest can't invite); the invitee must be an accepted friend
/// and not already a guild member. Idempotent (re-inviting a current guest is a
/// no-op). Returns the guest row.
#[tracing::instrument(skip_all, fields(account = %account.0, channel = %cid))]
pub async fn invite_guest(
    State(state): State<AppState>,
    Path(cid): Path<String>,
    account: AuthAccount,
    payload: Result<Json<InviteGuestRequest>, JsonRejection>,
) -> Response {
    // Resolve the channel → its guild + the caller's guild_member presence. Only a
    // live `kind='text'` channel whose guild is live, with the caller a member, may
    // host a cameo; anything else is the privacy-404 (a guest, a non-member, a DM,
    // a lorebook channel, a missing channel all collapse to it).
    let gid = match guild_of_member_text_channel(&state, &cid, &account.0).await {
        Ok(Some(gid)) => gid,
        Ok(None) => return not_found(),
        Err(e) => {
            tracing::error!(error = %e, "channel resolve failed");
            return storage_error();
        }
    };

    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    let invitee = req.account_id.trim().to_string();
    if invitee.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "account_id required");
    }
    if invitee == account.0 {
        return error_response(StatusCode::BAD_REQUEST, "cannot invite yourself");
    }

    // Optional expiry: a valid RFC3339 instant, else a clean 400 (validated in Rust
    // so a malformed value never reaches `type::datetime` as a 500).
    let expires_at: Option<String> = match req.expires_at.as_deref().map(str::trim) {
        Some(s) if !s.is_empty() => {
            if chrono::DateTime::parse_from_rfc3339(s).is_err() {
                return error_response(StatusCode::BAD_REQUEST, "invalid expires_at");
            }
            Some(s.to_string())
        }
        _ => None,
    };

    // Friend-gate: the invitee must be the caller's accepted friend.
    match is_accepted_friend(&state, &account.0, &invitee).await {
        Ok(true) => {}
        Ok(false) => {
            return error_response(StatusCode::FORBIDDEN, "you can only invite your friends")
        }
        Err(e) => {
            tracing::error!(error = %e, "friend-gate lookup failed");
            return storage_error();
        }
    }

    // A guild member already has full access — a cameo would be meaningless and
    // confusing (they'd get a guest badge on their own messages).
    match is_guild_member(&state, &gid, &invitee).await {
        Ok(true) => {
            return error_response(StatusCode::BAD_REQUEST, "already a member of this guild")
        }
        Ok(false) => {}
        Err(e) => {
            tracing::error!(error = %e, "guild-member check failed");
            return storage_error();
        }
    }

    // Create the cameo. Idempotent: re-inviting a current guest hits the UNIQUE
    // (channel, account) index → swallowed as a no-op (the DM invite pattern). The
    // optional expiry rides in via `type::datetime`, never spliced.
    let exp_set = if expires_at.is_some() {
        ",
            expires_at = type::datetime($exp)"
    } else {
        ""
    };
    let create_sql = format!(
        "CREATE channel_guest SET
            channel = type::record('channel', $cid),
            account = type::record('account', $a),
            invited_by = type::record('account', $by){exp_set};"
    );
    let cid_q = cid.clone();
    let inv_q = invitee.clone();
    let by_q = account.0.clone();
    let exp_q = expires_at.clone();
    let result = with_write_conflict_retry(|| async {
        let mut q = state
            .db
            .query(&create_sql)
            .bind(("cid", cid_q.clone()))
            .bind(("a", inv_q.clone()))
            .bind(("by", by_q.clone()));
        if let Some(exp) = exp_q.clone() {
            q = q.bind(("exp", exp));
        }
        q.await?.check()?;
        Ok(())
    })
    .await;
    match result {
        Ok(()) => {}
        // Already a guest → idempotent: fall through to the refetch + emit.
        Err(e) if is_unique_violation(&e) => {}
        Err(e) => {
            tracing::error!(error = %e, "invite_guest write failed");
            return storage_error();
        }
    }

    // Nudge the invitee (their cameo list + /events visibility refetch) and the
    // inviter (their host-side guest list).
    state.emit_for(
        vec![invitee.clone(), account.0.clone()],
        SyncEvent::ListsChanged,
    );
    match load_guest(&state, &cid, &invitee).await {
        Ok(Some(guest)) => (StatusCode::CREATED, Json(guest)).into_response(),
        Ok(None) => {
            tracing::error!(channel = %cid, "guest row missing right after invite");
            storage_error()
        }
        Err(e) => {
            tracing::error!(error = %e, "load_guest failed");
            storage_error()
        }
    }
}

// ---------------------------------------------------------------------------
// GET /channels/{cid}/guests — list a channel's active guests (host view)
// ---------------------------------------------------------------------------

/// GET /channels/{cid}/guests — the channel's active (unexpired) guests, with live
/// account identity. Any member of the channel may see it (privacy-404 otherwise).
#[tracing::instrument(skip_all, fields(account = %account.0, channel = %cid))]
pub async fn list_guests(
    State(state): State<AppState>,
    Path(cid): Path<String>,
    account: AuthAccount,
) -> Response {
    match resolve_membership(&state, &cid, &account.0, true).await {
        Ok(Membership::Member { kind }) if kind == "text" => {}
        Ok(_) => return not_found(),
        Err(e) => {
            tracing::error!(error = %e, "resolve_membership failed");
            return storage_error();
        }
    }
    match load_guests(&state, &cid).await {
        Ok(guests) => (StatusCode::OK, Json(ListGuestsResponse { guests })).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "load_guests failed");
            storage_error()
        }
    }
}

// ---------------------------------------------------------------------------
// DELETE /channels/{cid}/guests/me — leave a cameo
// ---------------------------------------------------------------------------

/// DELETE /channels/{cid}/guests/me — the caller (a guest) ends their own cameo.
/// Idempotent: deleting one's own (possibly absent) guest row is a 204 either way,
/// so it leaks nothing about a channel the caller can't see.
#[tracing::instrument(skip_all, fields(account = %account.0, channel = %cid))]
pub async fn leave_cameo(
    State(state): State<AppState>,
    Path(cid): Path<String>,
    account: AuthAccount,
) -> Response {
    let cid_q = cid.clone();
    let me_q = account.0.clone();
    let result = with_write_conflict_retry(|| async {
        state
            .db
            .query(
                "DELETE FROM channel_guest
                    WHERE channel = type::record('channel', $cid)
                      AND account = type::record('account', $me);",
            )
            .bind(("cid", cid_q.clone()))
            .bind(("me", me_q.clone()))
            .await?
            .check()?;
        Ok(())
    })
    .await;
    if let Err(e) = result {
        tracing::error!(error = %e, "leave_cameo write failed");
        return storage_error();
    }
    state.emit_for(vec![account.0.clone()], SyncEvent::ListsChanged);
    StatusCode::NO_CONTENT.into_response()
}

// ---------------------------------------------------------------------------
// DELETE /channels/{cid}/guests/{aid} — revoke a guest
// ---------------------------------------------------------------------------

/// DELETE /channels/{cid}/guests/{aid} — revoke a guest's cameo. Allowed if the
/// caller is the guest's inviter OR a guild manager (owner/admin). A non-member
/// caller is the privacy-404; a member who is neither inviter nor manager is 403.
#[tracing::instrument(skip_all, fields(account = %account.0, channel = %cid, guest = %aid))]
pub async fn revoke_guest(
    State(state): State<AppState>,
    Path((cid, aid)): Path<(String, String)>,
    account: AuthAccount,
) -> Response {
    #[derive(SurrealValue)]
    struct Authz {
        kind: Option<String>,
        guild_key: Option<String>,
        role: Option<String>,
        invited_by: Option<String>,
    }
    let authz = state
        .db
        .query(
            "LET $chan = (
                SELECT (IF guild != NONE THEN meta::id(guild) ELSE NONE END) AS guild_key, kind
                FROM ONLY type::record('channel', $cid)
                WHERE deleted_at = NONE AND (guild = NONE OR guild.deleted_at = NONE));
             RETURN {
                kind: $chan.kind,
                guild_key: $chan.guild_key,
                role: (IF $chan = NONE OR $chan.guild_key = NONE THEN NONE ELSE
                    (SELECT VALUE role FROM ONLY guild_member
                        WHERE guild = type::record('guild', $chan.guild_key)
                          AND account = type::record('account', $me)) END),
                invited_by: (SELECT VALUE meta::id(invited_by) FROM ONLY channel_guest
                    WHERE channel = type::record('channel', $cid)
                      AND account = type::record('account', $aid)),
             };",
        )
        .bind(("cid", cid.clone()))
        .bind(("aid", aid.clone()))
        .bind(("me", account.0.clone()))
        .await
        .and_then(|r| r.check());
    let authz: Authz = match authz.and_then(|mut r| r.take(1)) {
        Ok(Some(a)) => a,
        Ok(None) => return storage_error(),
        Err(e) => {
            tracing::error!(error = %e, "revoke authz query failed");
            return storage_error();
        }
    };
    // Non-text / missing channel, or the caller isn't a guild member → privacy-404.
    if authz.kind.as_deref() != Some("text") || authz.guild_key.is_none() || authz.role.is_none() {
        return not_found();
    }
    let is_manager = matches!(authz.role.as_deref(), Some("owner") | Some("admin"));
    let is_inviter = authz.invited_by.as_deref() == Some(account.0.as_str());
    if !is_manager && !is_inviter {
        return error_response(StatusCode::FORBIDDEN, "not allowed to revoke this guest");
    }

    let cid_q = cid.clone();
    let aid_q = aid.clone();
    let result = with_write_conflict_retry(|| async {
        state
            .db
            .query(
                "DELETE FROM channel_guest
                    WHERE channel = type::record('channel', $cid)
                      AND account = type::record('account', $aid);",
            )
            .bind(("cid", cid_q.clone()))
            .bind(("aid", aid_q.clone()))
            .await?
            .check()?;
        Ok(())
    })
    .await;
    if let Err(e) = result {
        tracing::error!(error = %e, "revoke_guest write failed");
        return storage_error();
    }
    state.emit_for(vec![aid], SyncEvent::ListsChanged);
    StatusCode::NO_CONTENT.into_response()
}

// ---------------------------------------------------------------------------
// GET /cameos — the caller's active cameos (guest view)
// ---------------------------------------------------------------------------

/// GET /cameos — every cameo the caller is currently an active guest in, for the
/// standalone cameo list (the guest can't see the host guild's rail).
#[tracing::instrument(skip_all, fields(account = %account.0))]
pub async fn list_cameos(State(state): State<AppState>, account: AuthAccount) -> Response {
    match load_cameos(&state, &account.0).await {
        Ok(cameos) => (StatusCode::OK, Json(ListCameosResponse { cameos })).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "load_cameos failed");
            storage_error()
        }
    }
}

// ---------------------------------------------------------------------------
// Unfriend hook (called from server::friends::remove_friend)
// ---------------------------------------------------------------------------

/// Revoke any active cameo between `a` and `b` (owner ruling: unfriending kills the
/// cameo — the friend-gate fell). Deletes a guest row in either direction (a hosted
/// b, or b hosted a) and nudges the affected accounts so their cameo list +
/// visibility refetch. Past badged messages stay (history is immutable). Best-effort
/// from the friends lifecycle.
pub(crate) async fn revoke_cameos_between(
    state: &AppState,
    a: &str,
    b: &str,
) -> surrealdb::Result<()> {
    let mut resp = state
        .db
        .query(
            "LET $affected = (SELECT VALUE meta::id(account) FROM channel_guest
                WHERE (account = type::record('account', $a) AND invited_by = type::record('account', $b))
                   OR (account = type::record('account', $b) AND invited_by = type::record('account', $a)));
             DELETE channel_guest
                WHERE (account = type::record('account', $a) AND invited_by = type::record('account', $b))
                   OR (account = type::record('account', $b) AND invited_by = type::record('account', $a));
             RETURN $affected;",
        )
        .bind(("a", a.to_string()))
        .bind(("b", b.to_string()))
        .await?
        .check()?;
    // Statement 0 = LET, 1 = DELETE, 2 = RETURN $affected.
    let affected: Vec<String> = resp.take(2)?;
    if !affected.is_empty() {
        state.emit_for(affected, SyncEvent::ListsChanged);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// The guild id of `cid` IF it is a live `kind='text'` channel (live guild) AND
/// `account` is a `guild_member` of it; `None` otherwise (→ privacy-404). A guest
/// is deliberately excluded — only a real member may invite.
async fn guild_of_member_text_channel(
    state: &AppState,
    cid: &str,
    account: &str,
) -> surrealdb::Result<Option<String>> {
    #[derive(SurrealValue)]
    struct Row {
        guild_key: Option<String>,
        kind: Option<String>,
        is_member: bool,
    }
    let mut resp = state
        .db
        .query(
            "LET $chan = (
                SELECT (IF guild != NONE THEN meta::id(guild) ELSE NONE END) AS guild_key, kind
                FROM ONLY type::record('channel', $cid)
                WHERE deleted_at = NONE AND (guild = NONE OR guild.deleted_at = NONE));
             RETURN {
                guild_key: $chan.guild_key,
                kind: $chan.kind,
                is_member: (IF $chan = NONE OR $chan.guild_key = NONE THEN false ELSE
                    ((SELECT VALUE true FROM ONLY guild_member
                        WHERE guild = type::record('guild', $chan.guild_key)
                          AND account = type::record('account', $me)) == true) END),
             };",
        )
        .bind(("cid", cid.to_string()))
        .bind(("me", account.to_string()))
        .await?
        .check()?;
    let row: Option<Row> = resp.take(1)?;
    Ok(row.and_then(|r| {
        if r.kind.as_deref() == Some("text") && r.is_member {
            r.guild_key
        } else {
            None
        }
    }))
}

/// True iff `me` and `other` are accepted friends (either direction).
async fn is_accepted_friend(state: &AppState, me: &str, other: &str) -> surrealdb::Result<bool> {
    let mut resp = state
        .db
        .query(
            "SELECT VALUE true FROM friendship
                WHERE state = 'accepted'
                  AND ( (requester = type::record('account', $me) AND addressee = type::record('account', $other))
                        OR (requester = type::record('account', $other) AND addressee = type::record('account', $me)) );",
        )
        .bind(("me", me.to_string()))
        .bind(("other", other.to_string()))
        .await?
        .check()?;
    let hits: Vec<bool> = resp.take(0)?;
    Ok(!hits.is_empty())
}

/// True iff `account` is a `guild_member` of `gid`.
async fn is_guild_member(state: &AppState, gid: &str, account: &str) -> surrealdb::Result<bool> {
    let mut resp = state
        .db
        .query(
            "SELECT VALUE true FROM guild_member
                WHERE guild = type::record('guild', $gid)
                  AND account = type::record('account', $a);",
        )
        .bind(("gid", gid.to_string()))
        .bind(("a", account.to_string()))
        .await?
        .check()?;
    let hits: Vec<bool> = resp.take(0)?;
    Ok(!hits.is_empty())
}

/// Live identity row of one active guest, or every active guest of a channel.
#[derive(SurrealValue)]
struct GuestRow {
    account_id: String,
    username: String,
    display_name: String,
    avatar_id: Option<String>,
    invited_by: String,
    expires_at: Option<Datetime>,
}

impl GuestRow {
    fn into_summary(self) -> GuestSummary {
        GuestSummary {
            account_id: self.account_id,
            username: self.username,
            display_name: self.display_name,
            avatar_id: self.avatar_id,
            invited_by: self.invited_by,
            expires_at: self.expires_at.map(to_rfc3339_fixed),
        }
    }
}

const GUEST_PROJECTION: &str = "
    meta::id(account) AS account_id,
    account.username  AS username,
    (account.display_name ?: account.username) AS display_name,
    (IF account.avatar != NONE THEN meta::id(account.avatar) ELSE NONE END) AS avatar_id,
    meta::id(invited_by) AS invited_by,
    expires_at";

async fn load_guest(
    state: &AppState,
    cid: &str,
    aid: &str,
) -> surrealdb::Result<Option<GuestSummary>> {
    let sql = format!(
        "SELECT {GUEST_PROJECTION} FROM ONLY channel_guest
            WHERE channel = type::record('channel', $cid)
              AND account = type::record('account', $aid);"
    );
    let mut resp = state
        .db
        .query(&sql)
        .bind(("cid", cid.to_string()))
        .bind(("aid", aid.to_string()))
        .await?
        .check()?;
    Ok(resp
        .take::<Option<GuestRow>>(0)?
        .map(GuestRow::into_summary))
}

async fn load_guests(state: &AppState, cid: &str) -> surrealdb::Result<Vec<GuestSummary>> {
    let sql = format!(
        "SELECT {GUEST_PROJECTION} FROM channel_guest
            WHERE channel = type::record('channel', $cid)
              AND (expires_at = NONE OR expires_at > time::now());"
    );
    let mut resp = state
        .db
        .query(&sql)
        .bind(("cid", cid.to_string()))
        .await?
        .check()?;
    let rows: Vec<GuestRow> = resp.take(0)?;
    Ok(rows.into_iter().map(GuestRow::into_summary).collect())
}

async fn load_cameos(state: &AppState, account: &str) -> surrealdb::Result<Vec<CameoSummary>> {
    #[derive(SurrealValue)]
    struct CameoRow {
        channel_id: String,
        channel_name: String,
        guild_name: Option<String>,
        invited_by: String,
        expires_at: Option<Datetime>,
    }
    let mut resp = state
        .db
        .query(
            "SELECT
                meta::id(channel)  AS channel_id,
                channel.name       AS channel_name,
                channel.guild.name AS guild_name,
                meta::id(invited_by) AS invited_by,
                expires_at
             FROM channel_guest
             WHERE account = type::record('account', $me)
               AND (expires_at = NONE OR expires_at > time::now())
               AND channel.kind = 'text'
               AND channel.deleted_at = NONE;",
        )
        .bind(("me", account.to_string()))
        .await?
        .check()?;
    let rows: Vec<CameoRow> = resp.take(0)?;
    Ok(rows
        .into_iter()
        .map(|r| CameoSummary {
            channel_id: r.channel_id,
            channel_name: r.channel_name,
            guild_name: r.guild_name,
            invited_by: r.invited_by,
            expires_at: r.expires_at.map(to_rfc3339_fixed),
        })
        .collect())
}
