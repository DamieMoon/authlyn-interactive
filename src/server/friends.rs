//! Friends — account-to-account relationships (phase-1 build step 6).
//!
//! Global (independent of guilds). One directed `friendship` row per request
//! (`requester` -> `addressee`), advancing `pending -> accepted`. The
//! `friendship_pair (requester, addressee)` UNIQUE index blocks duplicate
//! requests in the same direction; a request whose reverse is already pending
//! auto-accepts (the common "we both clicked add" sequential case).

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use surrealdb::types::SurrealValue;

use crate::protocol::{FriendRequest, FriendSummary, ListFriendsResponse};
use crate::server::auth::AuthAccount;
use crate::server::db_helpers::IdRow;
use crate::server::errors::{error_response, json_rejection_response};
use crate::server::retry::{is_unique_violation, with_write_conflict_retry};
use crate::server::state::AppState;

// ---------------------------------------------------------------------------
// GET /friends
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0))]
pub async fn list_friends(State(state): State<AppState>, account: AuthAccount) -> Response {
    match load_friends(&state, &account.0).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "load_friends failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

async fn load_friends(state: &AppState, account: &str) -> surrealdb::Result<ListFriendsResponse> {
    #[derive(SurrealValue)]
    struct Row {
        account_id: String,
        username: String,
    }
    // Three SELECTs in one round-trip: accepted (either direction → the other
    // party), incoming pending (others → me), outgoing pending (me → others).
    let sql = "
        SELECT
            (IF requester = type::record('account', $me) THEN meta::id(addressee)
             ELSE meta::id(requester) END) AS account_id,
            (IF requester = type::record('account', $me) THEN addressee.username
             ELSE requester.username END) AS username
        FROM friendship
        WHERE state = 'accepted'
          AND (requester = type::record('account', $me)
               OR addressee = type::record('account', $me));

        SELECT meta::id(requester) AS account_id, requester.username AS username
        FROM friendship
        WHERE state = 'pending' AND addressee = type::record('account', $me);

        SELECT meta::id(addressee) AS account_id, addressee.username AS username
        FROM friendship
        WHERE state = 'pending' AND requester = type::record('account', $me);
    ";
    let mut resp = state
        .db
        .query(sql)
        .bind(("me", account.to_string()))
        .await?
        .check()?;
    let friends: Vec<Row> = resp.take(0)?;
    let incoming: Vec<Row> = resp.take(1)?;
    let outgoing: Vec<Row> = resp.take(2)?;
    let map = |rows: Vec<Row>| {
        rows.into_iter()
            .map(|r| FriendSummary {
                account_id: r.account_id,
                username: r.username,
            })
            .collect()
    };
    Ok(ListFriendsResponse {
        friends: map(friends),
        incoming: map(incoming),
        outgoing: map(outgoing),
    })
}

// ---------------------------------------------------------------------------
// POST /friends
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0))]
pub async fn add_friend(
    State(state): State<AppState>,
    account: AuthAccount,
    payload: Result<Json<FriendRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    let username_ci = req.username.trim().to_lowercase();
    if username_ci.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "username required");
    }

    let target = match account_id_by_username_ci(&state, &username_ci).await {
        Ok(Some(id)) => id,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "user not found"),
        Err(e) => {
            tracing::error!(error = %e, "account lookup failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    if target == account.0 {
        return error_response(StatusCode::BAD_REQUEST, "cannot friend yourself");
    }

    // If they already requested me, accept that instead of creating a row.
    match pair_state(&state, &target, &account.0).await {
        Ok(Some(ref s)) if s == "accepted" => {
            return error_response(StatusCode::CONFLICT, "already friends");
        }
        Ok(Some(_)) => {
            // reverse pending → accept it
            return match set_accepted(&state, &target, &account.0).await {
                Ok(_) => {
                    // M7/P1 (review M2): the auto-accept path also unlocks a
                    // previously-locked 1:1 DM between the pair. Best-effort.
                    if let Err(e) =
                        crate::server::dms::set_one_to_one_lock(&state, &target, &account.0, false)
                            .await
                    {
                        tracing::error!(error = %e, "unlocking the 1:1 DM on re-friend failed");
                    }
                    emit_friends_changed(&state, &account.0, &target);
                    StatusCode::OK.into_response()
                }
                Err(e) => {
                    tracing::error!(error = %e, "auto-accept failed");
                    error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
                }
            };
        }
        Ok(None) => {}
        Err(e) => {
            tracing::error!(error = %e, "reverse pair lookup failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }

    // Already have a forward request/friendship?
    match pair_state(&state, &account.0, &target).await {
        Ok(Some(_)) => return error_response(StatusCode::CONFLICT, "request already exists"),
        Ok(None) => {}
        Err(e) => {
            tracing::error!(error = %e, "forward pair lookup failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }

    let caller = account.0.clone();
    let result = with_write_conflict_retry(|| async {
        state
            .db
            .query(
                "CREATE friendship SET
                    requester = type::record('account', $requester),
                    addressee = type::record('account', $addressee),
                    state = 'pending';",
            )
            .bind(("requester", caller.clone()))
            .bind(("addressee", target.clone()))
            .await?
            .check()?;
        Ok(())
    })
    .await;
    match result {
        Ok(()) => {
            emit_friends_changed(&state, &caller, &target);
            StatusCode::CREATED.into_response()
        }
        Err(e) if is_unique_violation(&e) => {
            error_response(StatusCode::CONFLICT, "request already exists")
        }
        Err(e) => {
            tracing::error!(error = %e, "add_friend write failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// POST /friends/{aid}/accept
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, requester = %aid))]
pub async fn accept_friend(
    State(state): State<AppState>,
    Path(aid): Path<String>,
    account: AuthAccount,
) -> Response {
    // Only a pending request *to* me from `aid` can be accepted.
    let updated = state
        .db
        .query(
            "UPDATE friendship SET state = 'accepted', updated_at = time::now()
                WHERE requester = type::record('account', $aid)
                  AND addressee = type::record('account', $me)
                  AND state = 'pending'
                RETURN meta::id(id) AS id_key;",
        )
        .bind(("aid", aid.clone()))
        .bind(("me", account.0.clone()))
        .await
        .and_then(|mut r| r.take::<Vec<IdRow>>(0));
    match updated {
        Ok(rows) if !rows.is_empty() => {
            // M7/P1 (review M2): re-friending unlocks the previously-locked 1:1 DM
            // (if one survived the unfriend), restoring posting. Best-effort.
            if let Err(e) =
                crate::server::dms::set_one_to_one_lock(&state, &account.0, &aid, false).await
            {
                tracing::error!(error = %e, "unlocking the 1:1 DM on re-friend failed");
            }
            emit_friends_changed(&state, &account.0, &aid);
            StatusCode::OK.into_response()
        }
        Ok(_) => error_response(StatusCode::NOT_FOUND, "no pending request from that user"),
        Err(e) => {
            tracing::error!(error = %e, "accept_friend failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// DELETE /friends/{aid}
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, other = %aid))]
pub async fn remove_friend(
    State(state): State<AppState>,
    Path(aid): Path<String>,
    account: AuthAccount,
) -> Response {
    // Removes the relationship in either direction: cancels an outgoing
    // request, declines an incoming one, or unfriends. Idempotent.
    match state
        .db
        .query(
            "DELETE FROM friendship WHERE
                (requester = type::record('account', $me)
                 AND addressee = type::record('account', $other))
                OR (requester = type::record('account', $other)
                 AND addressee = type::record('account', $me));",
        )
        .bind(("me", account.0.clone()))
        .bind(("other", aid.clone()))
        .await
        .and_then(|r| r.check())
    {
        Ok(_) => {
            // M7/P1 (review M2): unfriending makes the shared 1:1 DM read-only
            // (history preserved, posting server-rejected). Best-effort — the
            // unfriend already committed, so a lock failure is logged, not
            // surfaced; groups have no 1:1 pair and are untouched.
            if let Err(e) =
                crate::server::dms::set_one_to_one_lock(&state, &account.0, &aid, true).await
            {
                tracing::error!(error = %e, "locking the 1:1 DM on unfriend failed");
            }
            // M7/P2 (owner ruling): unfriending revokes any active Guest Cameo
            // between the two — the friend-gate fell, so access dies (past badged
            // messages stay; history is immutable). Best-effort + emits its own
            // ListsChanged to the affected guest.
            if let Err(e) =
                crate::server::cameos::revoke_cameos_between(&state, &account.0, &aid).await
            {
                tracing::error!(error = %e, "revoking the cameo on unfriend failed");
            }
            // Emitted even when nothing matched (idempotent DELETE): a spare
            // id-only nudge costs the target one refetch; detecting no-op
            // deletes would need a RETURN clause and buys nothing. Side
            // effect, accepted: `aid` is unvalidated here, so any
            // authenticated caller can nudge an arbitrary account into one
            // permission-checked `/friends` refetch — harmless (id-only,
            // rate-bounded by the caller's own requests).
            emit_friends_changed(&state, &account.0, &aid);
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "remove_friend failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// W1.5: every friend mutation nudges EXACTLY the two accounts of the
/// friendship edge over the SSE bus (account-targeted, never broadcast) —
/// an incoming request / accept / removal becomes visible live on the other
/// party's open clients instead of waiting for an unrelated event.
fn emit_friends_changed(state: &AppState, a: &str, b: &str) {
    state.emit_for(
        vec![a.to_string(), b.to_string()],
        crate::protocol::SyncEvent::FriendsChanged,
    );
}

/// The `state` of the directed `requester -> addressee` friendship, if any.
async fn pair_state(
    state: &AppState,
    requester: &str,
    addressee: &str,
) -> surrealdb::Result<Option<String>> {
    #[derive(SurrealValue)]
    struct Row {
        state: String,
    }
    let mut resp = state
        .db
        .query(
            "SELECT state FROM friendship
                WHERE requester = type::record('account', $requester)
                  AND addressee = type::record('account', $addressee);",
        )
        .bind(("requester", requester.to_string()))
        .bind(("addressee", addressee.to_string()))
        .await?
        .check()?;
    Ok(resp.take::<Option<Row>>(0)?.map(|r| r.state))
}

async fn set_accepted(state: &AppState, requester: &str, addressee: &str) -> surrealdb::Result<()> {
    state
        .db
        .query(
            "UPDATE friendship SET state = 'accepted', updated_at = time::now()
                WHERE requester = type::record('account', $requester)
                  AND addressee = type::record('account', $addressee);",
        )
        .bind(("requester", requester.to_string()))
        .bind(("addressee", addressee.to_string()))
        .await?
        .check()?;
    Ok(())
}

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
