//! Direct messages (M7/P1) — guild-less DM threads, 1:1 + groups.
//!
//! A DM thread is a `channel` with `kind='dm'` and `guild=NONE`; membership lives
//! in `dm_member` (the dm-kind analog of `guild_member`). Messages, read-state,
//! and per-channel persona wear are channel-scoped (`/channels/{id}/…`) and gated
//! by [`crate::server::access::resolve_membership`], which already branches on
//! kind — so this module owns only the THREAD LIFECYCLE: create, list, invite,
//! leave. No parallel message API.
//!
//! Invites are friend-gated: you can only start a DM with, or add to a group,
//! accounts who are your accepted friends (anti-spam on a self-hosted friend
//! group). Non-member access to a thread is the identical privacy-404 as a guild
//! channel — both from `resolve_membership` (messages) and the explicit checks
//! here (invite/leave), with a byte-identical `"channel not found"` body.

use std::collections::HashMap;

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use surrealdb::types::SurrealValue;

use crate::protocol::{
    CreateDmRequest, DmMemberSummary, DmSummary, InviteToDmRequest, ListDmsResponse, SyncEvent,
};
use crate::server::access::{resolve_membership, Membership};
use crate::server::auth::AuthAccount;
use crate::server::errors::{error_response, json_rejection_response};
use crate::server::retry::{is_unique_violation, with_write_conflict_retry};
use crate::server::state::AppState;

/// Max participants in one thread (creator + invitees). A self-hosted friend
/// group; this is a sanity bound, not a product limit.
const DM_MAX_MEMBERS: usize = 16;
/// Max group title length (chars), trimmed; empty = untitled (always for 1:1).
const DM_TITLE_MAX: usize = 64;

/// Same body as the guild privacy-404 (`messages` / `personas`), so a non-member
/// can't tell a thread they're not in apart from one that doesn't exist.
fn not_found() -> Response {
    error_response(StatusCode::NOT_FOUND, "channel not found")
}

fn storage_error() -> Response {
    error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
}

// ---------------------------------------------------------------------------
// GET /dms
// ---------------------------------------------------------------------------

/// GET /dms — every DM thread the caller is a member of, with live member rows.
#[tracing::instrument(skip_all, fields(account = %account.0))]
pub async fn list_dms(State(state): State<AppState>, account: AuthAccount) -> Response {
    match load_dms(&state, &account.0).await {
        Ok(dms) => (StatusCode::OK, Json(ListDmsResponse { dms })).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "load_dms failed");
            storage_error()
        }
    }
}

async fn load_dms(state: &AppState, account: &str) -> surrealdb::Result<Vec<DmSummary>> {
    #[derive(SurrealValue)]
    struct ThreadRow {
        id: String,
        title: Option<String>,
    }
    let mut resp = state
        .db
        .query(
            "LET $tids = (SELECT VALUE channel FROM dm_member
                WHERE account = type::record('account', $me));
             SELECT meta::id(id) AS id, (IF name = '' THEN NONE ELSE name END) AS title
                FROM channel
                WHERE id IN $tids AND kind = 'dm' AND deleted_at = NONE;",
        )
        .bind(("me", account.to_string()))
        .await?
        .check()?;
    let threads: Vec<ThreadRow> = resp.take(1)?;
    if threads.is_empty() {
        return Ok(Vec::new());
    }

    let tids: Vec<String> = threads.iter().map(|t| t.id.clone()).collect();
    let mut members = load_members_for(state, &tids).await?;
    Ok(threads
        .into_iter()
        .map(|t| DmSummary {
            members: members.remove(&t.id).unwrap_or_default(),
            id: t.id,
            title: t.title,
        })
        .collect())
}

/// Live member rows for the given threads, grouped by thread id. Account
/// identity (display_name/avatar) resolves LIVE, like everywhere else.
async fn load_members_for(
    state: &AppState,
    tids: &[String],
) -> surrealdb::Result<HashMap<String, Vec<DmMemberSummary>>> {
    #[derive(SurrealValue)]
    struct MemRow {
        thread_id: String,
        account_id: String,
        username: String,
        display_name: String,
        avatar_id: Option<String>,
    }
    let mut resp = state
        .db
        .query(
            "LET $chs = $tids.map(|$t| type::record('channel', $t));
             SELECT
                meta::id(channel) AS thread_id,
                meta::id(account) AS account_id,
                account.username  AS username,
                (account.display_name ?: account.username) AS display_name,
                (IF account.avatar != NONE THEN meta::id(account.avatar) ELSE NONE END) AS avatar_id
             FROM dm_member WHERE channel IN $chs;",
        )
        .bind(("tids", tids.to_vec()))
        .await?
        .check()?;
    let rows: Vec<MemRow> = resp.take(1)?;
    let mut out: HashMap<String, Vec<DmMemberSummary>> = HashMap::new();
    for r in rows {
        out.entry(r.thread_id).or_default().push(DmMemberSummary {
            account_id: r.account_id,
            username: r.username,
            display_name: r.display_name,
            avatar_id: r.avatar_id,
        });
    }
    Ok(out)
}

async fn dm_summary(state: &AppState, tid: &str) -> surrealdb::Result<Option<DmSummary>> {
    #[derive(SurrealValue)]
    struct ThreadRow {
        title: Option<String>,
    }
    let mut resp = state
        .db
        .query(
            "SELECT (IF name = '' THEN NONE ELSE name END) AS title
                FROM ONLY type::record('channel', $cid)
                WHERE kind = 'dm' AND deleted_at = NONE;",
        )
        .bind(("cid", tid.to_string()))
        .await?
        .check()?;
    let Some(t) = resp.take::<Option<ThreadRow>>(0)? else {
        return Ok(None);
    };
    let mut members = load_members_for(state, std::slice::from_ref(&tid.to_string())).await?;
    Ok(Some(DmSummary {
        id: tid.to_string(),
        title: t.title,
        members: members.remove(tid).unwrap_or_default(),
    }))
}

async fn dm_summary_response(state: &AppState, tid: &str, status: StatusCode) -> Response {
    match dm_summary(state, tid).await {
        Ok(Some(dm)) => (status, Json(dm)).into_response(),
        Ok(None) => {
            tracing::error!(thread = %tid, "dm_summary missing right after a successful mutation");
            storage_error()
        }
        Err(e) => {
            tracing::error!(error = %e, "dm_summary failed");
            storage_error()
        }
    }
}

// ---------------------------------------------------------------------------
// POST /dms
// ---------------------------------------------------------------------------

/// POST /dms — start a 1:1 (one other member) or group (2+) DM. Every member
/// must be an accepted friend of the creator. A 1:1 is deduped to the existing
/// thread between the two if one is live.
#[tracing::instrument(skip_all, fields(account = %account.0))]
pub async fn create_dm(
    State(state): State<AppState>,
    account: AuthAccount,
    payload: Result<Json<CreateDmRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };

    // Normalize: trim, drop blanks + the creator, dedup.
    let mut members: Vec<String> = req
        .members
        .iter()
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty() && m != &account.0)
        .collect();
    members.sort();
    members.dedup();
    if members.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "a DM needs at least one other member",
        );
    }
    if members.len() + 1 > DM_MAX_MEMBERS {
        return error_response(StatusCode::BAD_REQUEST, "too many members");
    }
    let title: String = req
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| t.chars().take(DM_TITLE_MAX).collect())
        .unwrap_or_default();

    // Friend-gate: every requested member must be an accepted friend.
    match accepted_friends_among(&state, &account.0, &members).await {
        Ok(friends) => {
            if !members.iter().all(|m| friends.contains(m)) {
                return error_response(StatusCode::FORBIDDEN, "you can only DM your friends");
            }
        }
        Err(e) => {
            tracing::error!(error = %e, "friend-gate lookup failed");
            return storage_error();
        }
    }

    // 1:1 dedup — return the existing live thread between the two, if any.
    if members.len() == 1 {
        match existing_one_to_one(&state, &account.0, &members[0]).await {
            Ok(Some(cid)) => return dm_summary_response(&state, &cid, StatusCode::OK).await,
            Ok(None) => {}
            Err(e) => {
                tracing::error!(error = %e, "1:1 dedup lookup failed");
                return storage_error();
            }
        }
    }

    // Create the thread + a dm_member per participant atomically.
    let mut participants = members.clone();
    participants.push(account.0.clone());
    let title_q = title.clone();
    let parts_q = participants.clone();
    let created = with_write_conflict_retry(|| async {
        let mut resp = state
            .db
            .query(
                "LET $cid = (CREATE ONLY channel SET
                    guild = NONE, kind = 'dm', name = $title, position = 0).id;
                 FOR $a IN $participants {
                    CREATE dm_member SET channel = $cid, account = type::record('account', $a);
                 };
                 RETURN meta::id($cid);",
            )
            .bind(("title", title_q.clone()))
            .bind(("participants", parts_q.clone()))
            .await?
            .check()?;
        // Statements: 0 LET, 1 FOR, 2 RETURN (the new channel id).
        resp.take::<Option<String>>(2)
    })
    .await;

    let cid = match created {
        Ok(Some(cid)) => cid,
        Ok(None) => {
            tracing::error!("create_dm produced no channel id");
            return storage_error();
        }
        Err(e) => {
            tracing::error!(error = %e, "create_dm write failed");
            return storage_error();
        }
    };

    state.emit_for(participants, SyncEvent::ListsChanged);
    dm_summary_response(&state, &cid, StatusCode::CREATED).await
}

// ---------------------------------------------------------------------------
// POST /dms/{tid}/members
// ---------------------------------------------------------------------------

/// POST /dms/{tid}/members — add one accepted friend of the caller to a thread
/// the caller belongs to. Idempotent (re-adding a current member is a no-op).
#[tracing::instrument(skip_all, fields(account = %account.0, thread = %tid))]
pub async fn invite_to_dm(
    State(state): State<AppState>,
    Path(tid): Path<String>,
    account: AuthAccount,
    payload: Result<Json<InviteToDmRequest>, JsonRejection>,
) -> Response {
    // Caller must be a member of this DM thread (privacy-404 otherwise).
    match resolve_membership(&state, &tid, &account.0, true).await {
        Ok(Membership::Member { kind }) if kind == "dm" => {}
        Ok(_) => return not_found(),
        Err(e) => {
            tracing::error!(error = %e, "resolve_membership failed");
            return storage_error();
        }
    }

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

    // Friend-gate: the invitee must be the inviter's accepted friend.
    match accepted_friends_among(&state, &account.0, std::slice::from_ref(&invitee)).await {
        Ok(friends) if friends.contains(&invitee) => {}
        Ok(_) => return error_response(StatusCode::FORBIDDEN, "you can only invite your friends"),
        Err(e) => {
            tracing::error!(error = %e, "friend-gate lookup failed");
            return storage_error();
        }
    }

    let tid_q = tid.clone();
    let inv_q = invitee.clone();
    let result = with_write_conflict_retry(|| async {
        state
            .db
            .query(
                "CREATE dm_member SET
                    channel = type::record('channel', $cid),
                    account = type::record('account', $a);",
            )
            .bind(("cid", tid_q.clone()))
            .bind(("a", inv_q.clone()))
            .await?
            .check()?;
        Ok(())
    })
    .await;
    match result {
        // Already a member → idempotent: fall through to the refetch + emit.
        Ok(()) => {}
        Err(e) if is_unique_violation(&e) => {}
        Err(e) => {
            tracing::error!(error = %e, "invite_to_dm write failed");
            return storage_error();
        }
    }

    notify_members(&state, &tid).await;
    dm_summary_response(&state, &tid, StatusCode::OK).await
}

// ---------------------------------------------------------------------------
// DELETE /dms/{tid}/members/me
// ---------------------------------------------------------------------------

/// DELETE /dms/{tid}/members/me — leave a thread. The caller's own client
/// refetches and drops it; when the last member leaves, the thread is
/// soft-deleted (the existing purge sweep reclaims it).
#[tracing::instrument(skip_all, fields(account = %account.0, thread = %tid))]
pub async fn leave_dm(
    State(state): State<AppState>,
    Path(tid): Path<String>,
    account: AuthAccount,
) -> Response {
    match resolve_membership(&state, &tid, &account.0, true).await {
        Ok(Membership::Member { kind }) if kind == "dm" => {}
        Ok(_) => return not_found(),
        Err(e) => {
            tracing::error!(error = %e, "resolve_membership failed");
            return storage_error();
        }
    }

    // Members BEFORE leaving — the emit list includes the leaver (so their own
    // open clients drop the thread).
    let recipients = match dm_member_ids(&state, &tid).await {
        Ok(ids) => ids,
        Err(e) => {
            tracing::error!(error = %e, "dm_member_ids failed");
            return storage_error();
        }
    };

    let tid_q = tid.clone();
    let me_q = account.0.clone();
    let result = with_write_conflict_retry(|| async {
        state
            .db
            .query(
                "DELETE FROM dm_member
                    WHERE channel = type::record('channel', $cid)
                      AND account = type::record('account', $me);
                 LET $left = (SELECT VALUE id FROM dm_member
                    WHERE channel = type::record('channel', $cid));
                 IF array::len($left) = 0 {
                    UPDATE type::record('channel', $cid) SET deleted_at = time::now();
                 };",
            )
            .bind(("cid", tid_q.clone()))
            .bind(("me", me_q.clone()))
            .await?
            .check()?;
        Ok(())
    })
    .await;
    if let Err(e) = result {
        tracing::error!(error = %e, "leave_dm write failed");
        return storage_error();
    }

    state.emit_for(recipients, SyncEvent::ListsChanged);
    StatusCode::NO_CONTENT.into_response()
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Of `targets`, the subset that are accepted friends of `me` (either
/// direction). Used to gate DM creation + invites.
async fn accepted_friends_among(
    state: &AppState,
    me: &str,
    targets: &[String],
) -> surrealdb::Result<std::collections::HashSet<String>> {
    if targets.is_empty() {
        return Ok(std::collections::HashSet::new());
    }
    let mut resp = state
        .db
        .query(
            "LET $targets = $ids.map(|$m| type::record('account', $m));
             SELECT VALUE
                (IF requester = type::record('account', $me)
                 THEN meta::id(addressee) ELSE meta::id(requester) END)
             FROM friendship
             WHERE state = 'accepted'
               AND ( (requester = type::record('account', $me) AND addressee IN $targets)
                     OR (addressee = type::record('account', $me) AND requester IN $targets) );",
        )
        .bind(("me", me.to_string()))
        .bind(("ids", targets.to_vec()))
        .await?
        .check()?;
    // Statement 0 is the LET; the SELECT VALUE is take(1).
    let ids: Vec<String> = resp.take(1)?;
    Ok(ids.into_iter().collect())
}

/// The live 1:1 DM thread between `me` and `other` (a dm channel both belong to
/// with exactly two members), if any.
async fn existing_one_to_one(
    state: &AppState,
    me: &str,
    other: &str,
) -> surrealdb::Result<Option<String>> {
    #[derive(SurrealValue)]
    struct Row {
        id: String,
        n: i64,
    }
    let mut resp = state
        .db
        .query(
            "LET $both = array::intersect(
                (SELECT VALUE channel FROM dm_member WHERE account = type::record('account', $me)),
                (SELECT VALUE channel FROM dm_member WHERE account = type::record('account', $other))
             );
             SELECT meta::id(channel) AS id, count() AS n FROM dm_member
                WHERE channel IN $both AND channel.deleted_at = NONE
                GROUP BY id;",
        )
        .bind(("me", me.to_string()))
        .bind(("other", other.to_string()))
        .await?
        .check()?;
    // Statement 0 is the LET; the grouped SELECT is take(1).
    let rows: Vec<Row> = resp.take(1)?;
    Ok(rows.into_iter().find(|r| r.n == 2).map(|r| r.id))
}

/// Every member account-id of a thread (for the SSE nudge).
async fn dm_member_ids(state: &AppState, tid: &str) -> surrealdb::Result<Vec<String>> {
    let mut resp = state
        .db
        .query(
            "SELECT VALUE meta::id(account) FROM dm_member
                WHERE channel = type::record('channel', $cid);",
        )
        .bind(("cid", tid.to_string()))
        .await?
        .check()?;
    resp.take(0)
}

/// Nudge every current member's clients to refetch their DM list (id-only).
async fn notify_members(state: &AppState, tid: &str) {
    match dm_member_ids(state, tid).await {
        Ok(ids) => state.emit_for(ids, SyncEvent::ListsChanged),
        Err(e) => tracing::error!(error = %e, "notify_members lookup failed (nudge skipped)"),
    }
}
