//! Cross-device read state (L-1): `POST /channels/{cid}/mark-read` +
//! `GET /channels/read-state`. ssr-only.
//!
//! Read/unread used to live only in each browser's localStorage, so a second
//! device had no idea what had been read. These two handlers persist the
//! caller's per-channel last-seen `(sent_at, id)` composite cursor server-side
//! (`channel_read_state`, one row per `(account, channel)`), so the state syncs
//! across devices.
//!
//! ## Mark-read keeps the MAX cursor
//! `mark-read` UPSERTs the row but never lets an OLDER cursor overwrite a NEWER
//! one — two devices reading the same channel at different scroll positions
//! converge to the furthest-read mark, not whichever POST landed last. The
//! comparison is the same strict composite the message cursor uses:
//! `last_seen_at` first, `last_seen_id` as the tie-break.
//!
//! ## Privacy 404s + parameterisation
//! Mark-read is membership-gated like every other channel route (privacy-404
//! for non-members / unknown channel, via the shared [`channel_access`]). The
//! datetime is bound through `type::datetime($p)` and the ids via `type::record`
//! — never spliced into query text — preserving the cursor + SQL invariants.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use surrealdb::types::{Datetime, SurrealValue};

use crate::protocol::{ChannelReadCursor, MarkReadRequest, ReadStateResponse};
use crate::server::auth::AuthAccount;
use crate::server::datetime::to_rfc3339_fixed;
use crate::server::errors::{error_response, json_rejection_response};
use crate::server::retry::with_write_conflict_retry;
use crate::server::state::AppState;

use super::{channel_access, AccessOutcome};

// ---------------------------------------------------------------------------
// POST /channels/{cid}/mark-read
// ---------------------------------------------------------------------------

/// POST /channels/{cid}/mark-read — record the caller's last-seen `(sent_at, id)`
/// cursor for this channel so read state syncs across their devices (L-1).
///
/// Membership-gated (privacy-404 for non-members / unknown channel). UPSERTs the
/// `(account, channel)` row keeping the MAX cursor: an older POST never regresses
/// a newer mark. The datetime is bound via `type::datetime`; ids via `type::record`.
#[tracing::instrument(skip_all, fields(account = %account.0, channel = %cid))]
pub async fn mark_read(
    State(state): State<AppState>,
    Path(cid): Path<String>,
    account: AuthAccount,
    payload: Result<Json<MarkReadRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };

    // Reject a malformed datetime with a deterministic 400 (mirrors the cursor
    // parse guard in `reading.rs`: a full RFC 3339 parse, not just a separator
    // probe) rather than letting `type::datetime($sent_at)` parse-error 500.
    if chrono::DateTime::parse_from_rfc3339(&req.sent_at).is_err() {
        return error_response(StatusCode::BAD_REQUEST, "invalid cursor");
    }

    match channel_access(&state, &cid, &account.0).await {
        Ok(AccessOutcome::Ok(_)) => {}
        Ok(AccessOutcome::ChannelNotFound) | Ok(AccessOutcome::NotMember) => {
            return error_response(StatusCode::NOT_FOUND, "channel not found");
        }
        Err(e) => {
            tracing::error!(error = %e, "channel_access failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }

    // Idempotent UPSERT on the (account, channel) UNIQUE pair, wrapped in the
    // write-conflict retry: two concurrent marks (two devices) converge to one
    // row instead of 500ing the MVCC loser (inv13). DELETE-then-CREATE only when
    // the incoming cursor is strictly NEWER than the stored one, so an older POST
    // never regresses a newer mark — the comparison is the same strict composite
    // the message cursor uses (sent_at, then id as tie-break).
    let outcome = with_write_conflict_retry(|| async {
        state
            .db
            .query(
                "BEGIN TRANSACTION;
                 LET $cur = (SELECT VALUE { at: last_seen_at, id: last_seen_id }
                     FROM ONLY channel_read_state
                     WHERE account = type::record('account', $account)
                       AND channel = type::record('channel', $cid)
                     LIMIT 1);
                 LET $new_at = type::datetime($sent_at);
                 IF $cur = NONE
                     OR $new_at > $cur.at
                     OR ($new_at = $cur.at AND $id > $cur.id) THEN {
                     DELETE FROM channel_read_state
                         WHERE account = type::record('account', $account)
                           AND channel = type::record('channel', $cid);
                     CREATE channel_read_state SET
                         account = type::record('account', $account),
                         channel = type::record('channel', $cid),
                         last_seen_at = $new_at,
                         last_seen_id = $id,
                         updated_at = time::now();
                 } END;
                 COMMIT TRANSACTION;",
            )
            .bind(("cid", cid.clone()))
            .bind(("account", account.0.clone()))
            .bind(("sent_at", req.sent_at.clone()))
            .bind(("id", req.id.clone()))
            .await?
            .check()?;
        Ok(())
    })
    .await;

    match outcome {
        Ok(()) => {
            // M1.5: nudge the caller's OTHER devices to refresh unread —
            // account-targeted, never broadcast (another account's read cursor
            // is none of your business and can't change your unread state).
            state.emit_for(
                vec![account.0.clone()],
                crate::protocol::SyncEvent::ReadStateChanged {
                    channel_id: cid.clone(),
                },
            );
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "mark_read failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// GET /channels/read-state
// ---------------------------------------------------------------------------

/// GET /channels/read-state — every channel the caller has a stored read cursor
/// for (L-1). The client hydrates its per-channel `last_seen` table from this on
/// shell mount. `sent_at` is projected raw and formatted to the fixed 9-digit
/// RFC 3339 shape so it matches the client cursor / message-envelope format.
#[tracing::instrument(skip_all, fields(account = %account.0))]
pub async fn read_state(State(state): State<AppState>, account: AuthAccount) -> Response {
    #[derive(SurrealValue)]
    struct Row {
        channel_id: String,
        last_seen_at: Datetime,
        last_seen_id: String,
    }

    let rows: Result<Vec<Row>, _> = async {
        let mut resp = state
            .db
            .query(
                "SELECT meta::id(channel) AS channel_id, last_seen_at, last_seen_id
                    FROM channel_read_state
                    WHERE account = type::record('account', $account);",
            )
            .bind(("account", account.0.clone()))
            .await?
            .check()?;
        resp.take::<Vec<Row>>(0)
    }
    .await;

    match rows {
        Ok(rows) => {
            let cursors = rows
                .into_iter()
                .map(|r| ChannelReadCursor {
                    channel_id: r.channel_id,
                    sent_at: to_rfc3339_fixed(r.last_seen_at),
                    id: r.last_seen_id,
                })
                .collect();
            Json(ReadStateResponse { cursors }).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "read_state failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}
