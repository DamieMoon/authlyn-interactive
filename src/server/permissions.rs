//! Shared authorization helpers: guild role gates, persona edit-access, and
//! the fail-closed admin guard.
//!
//! These were previously private to the handler modules that introduced them
//! (`guilds`, `personas`, `auth`) but are called cross-module — `caller_role`
//! by `emoji`, `can_edit_persona` by `messages`, `is_admin` by `feedback`.
//! Consolidating them here keeps one definition of each authz predicate; the
//! signatures, visibility, return types, status codes, and error bodies are
//! unchanged from their previous homes.

use axum::http::StatusCode;
use axum::response::Response;
use surrealdb::types::SurrealValue;

use crate::server::db_helpers::IdRow;
use crate::server::errors::error_response;
use crate::server::state::AppState;

// ---------------------------------------------------------------------------
// Guild role gates (formerly guilds.rs)
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
pub(crate) async fn require_manager(
    state: &AppState,
    gid: &str,
    account: &str,
) -> Result<(), Response> {
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
pub(crate) async fn require_owner(
    state: &AppState,
    gid: &str,
    account: &str,
) -> Result<(), Response> {
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

// ---------------------------------------------------------------------------
// Persona edit-access (formerly personas.rs)
// ---------------------------------------------------------------------------

pub(crate) async fn owns_persona(
    state: &AppState,
    pid: &str,
    account: &str,
) -> surrealdb::Result<bool> {
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

/// True when a `persona_editor` row links this persona to the account.
pub(crate) async fn is_persona_editor(
    state: &AppState,
    pid: &str,
    account: &str,
) -> surrealdb::Result<bool> {
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
pub(crate) async fn can_edit_persona(
    state: &AppState,
    pid: &str,
    account: &str,
) -> surrealdb::Result<bool> {
    if owns_persona(state, pid, account).await? {
        return Ok(true);
    }
    is_persona_editor(state, pid, account).await
}

// ---------------------------------------------------------------------------
// Admin guard (formerly auth.rs)
// ---------------------------------------------------------------------------

/// Admin guard: fail-closed. The caller (by account id) is an admin iff their
/// stored `username_ci` is in the configured admin set — the union of
/// `AUTHLYN_ADMIN_USERNAMES` (comma/whitespace-separated) and the legacy
/// singular `AUTHLYN_ADMIN_USERNAME`, each trimmed and lowercased. An empty set
/// (neither var set, or both blank) authorizes no one.
pub(crate) async fn is_admin(state: &AppState, account_id: &str) -> surrealdb::Result<bool> {
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

/// Build the lowercased admin-username set from the environment (see [`is_admin`]).
pub(crate) fn admin_username_set() -> std::collections::HashSet<String> {
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
