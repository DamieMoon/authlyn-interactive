//! Guild membership: list, invite, kick/leave, role changes.
//! Split from `server/guilds.rs` in Wave 3; behavior preserved verbatim.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use surrealdb::types::SurrealValue;

use crate::protocol::{
    InviteMemberRequest, ListMembersResponse, MemberSummary, SetMemberRoleRequest, SyncEvent,
};
use crate::server::auth::AuthAccount;
use crate::server::db_helpers::IdRow;
use crate::server::errors::{error_response, json_rejection_response};
use crate::server::permissions::{caller_role, require_manager};
use crate::server::retry::{is_unique_violation, with_write_conflict_retry};
use crate::server::state::AppState;

// ---------------------------------------------------------------------------
// GET /guilds/{id}/members
// ---------------------------------------------------------------------------

/// List the guild's members. Membership-gated like `get_guild`: non-members
/// (and missing guilds) get a privacy-404 so membership stays non-leaky. Every
/// member can read the roster; the owner-only mutation controls live in the
/// client pane and the `/role` + DELETE endpoints.
#[tracing::instrument(skip_all, fields(account = %account.0, guild = %gid))]
pub async fn list_members(
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

    match load_members(&state, &gid).await {
        Ok(members) => (StatusCode::OK, Json(ListMembersResponse { members })).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "load_members failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

async fn load_members(state: &AppState, gid: &str) -> surrealdb::Result<Vec<MemberSummary>> {
    #[derive(SurrealValue)]
    struct MemberRow {
        account_key: String,
        username: String,
        display_name: String,
        role: String,
        avatar_id: Option<String>,
    }
    let mut resp = state
        .db
        .query(
            "SELECT
                meta::id(account) AS account_key,
                account.username AS username,
                account.display_name AS display_name,
                role,
                (IF account.avatar != NONE THEN meta::id(account.avatar) ELSE NONE END)
                    AS avatar_id
             FROM guild_member
             WHERE guild = type::record('guild', $gid)
             ORDER BY role, username;",
        )
        .bind(("gid", gid.to_string()))
        .await?
        .check()?;
    let rows: Vec<MemberRow> = resp.take(0)?;
    Ok(rows
        .into_iter()
        .map(|r| MemberSummary {
            account_id: r.account_key,
            username: r.username,
            display_name: r.display_name,
            role: r.role,
            avatar_id: r.avatar_id,
        })
        .collect())
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
        Ok(()) => {
            state.emit(SyncEvent::ListsChanged);
            StatusCode::CREATED.into_response()
        }
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
        Ok(_) => {
            state.emit(SyncEvent::ListsChanged);
            StatusCode::NO_CONTENT.into_response()
        }
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
        Ok(_) => {
            state.emit(SyncEvent::ListsChanged);
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "set_member_role failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// Shared helper
// ---------------------------------------------------------------------------

async fn account_id_by_username_ci(
    state: &AppState,
    username_ci: &str,
) -> surrealdb::Result<Option<String>> {
    let mut resp = state
        .db
        .query("SELECT meta::id(id) AS id_key FROM account WHERE username_ci = $username_ci;")
        .bind(("username_ci", username_ci.to_string()))
        .await?
        .check()?;
    Ok(resp.take::<Option<IdRow>>(0)?.map(|r| r.id_key))
}
