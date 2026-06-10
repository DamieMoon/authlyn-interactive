//! `GET /unread` — W1 batched unread/ping summary for every visible text
//! channel. ssr-only.
//!
//! Three DB round-trips total (visible channels, read cursors, one
//! multi-statement batch), independent of channel count. The batch issues
//! unread ids `LIMIT 100` + a ping probe `LIMIT 1` per cursored channel and
//! the latest row per cursorless channel.
//! Bind names are loop-indexed (`$cid_0`, `$at_0`, …) — only the loop index
//! enters the query text, never a user value (the sanctioned splice form, like
//! `MSG_PROJECTION`). The unread predicate is the strict composite tie-break
//! the message cursor uses: `sent_at > $at OR (sent_at = $at AND meta::id(id)
//! > $mid)`, with the stored cursor bound as a true `datetime` (never a
//! `<string>` cast — see `server::datetime`). Soft-deleted messages never
//! count (`deleted_at = NONE` everywhere); visibility comes from
//! [`visible_channels`], so a non-member's channels simply don't appear.

use std::collections::HashMap;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use surrealdb::types::{Datetime, SurrealValue};

use crate::protocol::{ChannelUnread, UnreadResponse};
use crate::server::access::visible_channels;
use crate::server::auth::AuthAccount;
use crate::server::datetime::to_rfc3339_fixed;
use crate::server::errors::error_response;
use crate::server::state::AppState;

/// Cap on the per-channel unread count — past this the client renders "99+"
/// anyway, so counting further is wasted work. Compile-time const, spliced
/// into the query text like `MSG_PROJECTION` (never a user value).
const UNREAD_COUNT_CAP: usize = 100;

/// Which output row statement `i` of the batch feeds, and how to read it.
enum Stmt {
    /// Unread message ids (capped) → `out[i].unread`.
    Unread(usize),
    /// Ping probe (`LIMIT 1`) → `out[i].pinged`.
    Ping(usize),
    /// Latest live row of a cursorless channel → `out[i].latest_*`.
    Latest(usize),
}

/// GET /unread — batched unread/ping summary for every text channel the
/// caller can see, in one request (the SSE client refreshes badges from this
/// instead of polling every channel).
#[tracing::instrument(skip_all, fields(account = %account.0))]
pub async fn unread(State(state): State<AppState>, account: AuthAccount) -> Response {
    let result: surrealdb::Result<Vec<ChannelUnread>> = async {
        // 1) Visible channels seed the output rows (zero counts by default).
        let visible = visible_channels(&state, &account.0).await?;
        let mut out: Vec<ChannelUnread> = visible
            .iter()
            .map(|ch| ChannelUnread {
                channel_id: ch.channel_id.clone(),
                guild_id: ch.guild_id.clone(),
                unread: 0,
                pinged: false,
                latest_sent_at: None,
                latest_id: None,
            })
            .collect();
        // EMPTY-BATCH EDGE: no visible channels → nothing to query (an empty
        // query string would error), return the empty response as-is.
        if out.is_empty() {
            return Ok(out);
        }

        // 2) The caller's read cursors, keyed by channel id. The stored
        //    `last_seen_at` stays a raw `Datetime` so it re-binds as a true
        //    datetime below.
        #[derive(SurrealValue)]
        struct CursorRow {
            channel_id: String,
            last_seen_at: Datetime,
            last_seen_id: String,
        }
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
        let cursor_rows: Vec<CursorRow> = resp.take(0)?;
        let cursors: HashMap<String, (Datetime, String)> = cursor_rows
            .into_iter()
            .map(|r| (r.channel_id, (r.last_seen_at, r.last_seen_id)))
            .collect();

        // 3) One multi-statement batch over every visible channel. Only the
        //    loop index `i` is formatted into the SQL text; the channel id,
        //    cursor datetime, cursor id, and account all ride in as binds.
        let mut sql = String::new();
        let mut kinds: Vec<Stmt> = Vec::new();
        for (i, ch) in visible.iter().enumerate() {
            if cursors.contains_key(&ch.channel_id) {
                // Strict composite tie-break: the cursor row itself is READ.
                sql.push_str(&format!(
                    "SELECT VALUE meta::id(id) FROM message
                        WHERE channel = type::record('channel', $cid_{i})
                          AND deleted_at = NONE
                          AND (sent_at > $at_{i}
                               OR (sent_at = $at_{i} AND meta::id(id) > $mid_{i}))
                        LIMIT {UNREAD_COUNT_CAP};
                     SELECT VALUE meta::id(id) FROM message
                        WHERE channel = type::record('channel', $cid_{i})
                          AND deleted_at = NONE
                          AND (sent_at > $at_{i}
                               OR (sent_at = $at_{i} AND meta::id(id) > $mid_{i}))
                          AND type::record('account', $acct) IN (pinged_users ?? [])
                        LIMIT 1;"
                ));
                kinds.push(Stmt::Unread(i));
                kinds.push(Stmt::Ping(i));
            } else {
                // No cursor: surface the latest live row so the client can
                // baseline instead of glowing. Both ORDER BY keys are
                // projected (SurrealDB orders by selected aliases).
                sql.push_str(&format!(
                    "SELECT meta::id(id) AS id_key, sent_at FROM message
                        WHERE channel = type::record('channel', $cid_{i})
                          AND deleted_at = NONE
                        ORDER BY sent_at DESC, id_key DESC LIMIT 1;"
                ));
                kinds.push(Stmt::Latest(i));
            }
        }

        let mut q = state.db.query(sql).bind(("acct", account.0.clone()));
        for (i, ch) in visible.iter().enumerate() {
            q = q.bind((format!("cid_{i}"), ch.channel_id.clone()));
            if let Some((at, mid)) = cursors.get(&ch.channel_id) {
                q = q
                    .bind((format!("at_{i}"), *at))
                    .bind((format!("mid_{i}"), mid.clone()));
            }
        }
        let mut resp = q.await?.check()?;

        // 4) Walk the statements back into the output rows.
        #[derive(SurrealValue)]
        struct LatestRow {
            id_key: String,
            sent_at: Datetime,
        }
        for (stmt_idx, kind) in kinds.iter().enumerate() {
            match kind {
                Stmt::Unread(i) => {
                    let ids: Vec<String> = resp.take(stmt_idx)?;
                    out[*i].unread = ids.len();
                }
                Stmt::Ping(i) => {
                    let ids: Vec<String> = resp.take(stmt_idx)?;
                    out[*i].pinged = !ids.is_empty();
                }
                Stmt::Latest(i) => {
                    if let Some(latest) = resp.take::<Option<LatestRow>>(stmt_idx)? {
                        out[*i].latest_sent_at = Some(to_rfc3339_fixed(latest.sent_at));
                        out[*i].latest_id = Some(latest.id_key);
                    }
                }
            }
        }
        Ok(out)
    }
    .await;

    match result {
        Ok(channels) => Json(UnreadResponse { channels }).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "unread failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}
