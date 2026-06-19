//! `GET /unread` — W1 batched unread/ping summary for every visible text
//! channel. ssr-only.
//!
//! Three DB round-trips total (visible channels, read cursors, one
//! multi-statement batch), independent of channel count. The batch issues
//! capped unread ids + a ping probe per cursored channel and the latest row
//! per cursorless channel.
//! Bind names are loop-indexed (`$cid_0`, `$at_0`, …) — only the loop index
//! enters the query text, never a user value (the sanctioned splice form, like
//! `MSG_PROJECTION`). The unread predicate is the strict composite tie-break
//! the message cursor uses — `sent_at > $at OR (sent_at = $at AND
//! meta::id(id) > $mid)` — but each probe is issued as TWO statements (an
//! equality statement covering the cursor's own instant, refined by the
//! strict id tie-break, plus an open `sent_at > $at` range) because the
//! planner cannot push the OR form into the `(channel, sent_at)` index
//! (review M-11): EXPLAIN FULL on the 3.1.3 dev binary plans the OR as an
//! unbounded per-channel IndexScan — the caught-up case (the common one)
//! fetched EVERY row in the channel per call — while the split halves plan as
//! a `[channel, $at]` point access and a `MoreThan` range. The halves are
//! disjoint by construction (`sent_at =` vs `sent_at >`), so re-merging them
//! in step 4 (sum the counts, OR the ping flags) reproduces the OR predicate
//! exactly; the equal-`sent_at` tie-group test in `tests/unread.rs` pins the
//! boundary. The cursorless Latest probe is split the same way (review M-12
//! follow-up): a `LET` boundary probe finds the newest live instant via an
//! early-exiting backward index walk, then a `[channel, $lat]` point
//! statement cuts that instant's tie group by id — the single-statement
//! ORDER BY on the computed `id_key` alias fed the whole channel through
//! SortTopKByKey instead. The stored cursor is bound as a true `datetime` (never a
//! `<string>` cast — see `server::datetime`). Soft-deleted messages never
//! count (`deleted_at = NONE` in every statement); visibility comes from
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
/// A cursored channel emits TWO `Unread` and TWO `Ping` statements (the
/// tie-boundary + open-range split, see the module doc); step 4 accumulates
/// them onto the same output row.
enum Stmt {
    /// Unread message ids (capped) → summed into `out[i].unread`.
    Unread(usize),
    /// Ping probe (`LIMIT 1`) → OR-ed into `out[i].pinged`.
    Ping(usize),
    /// A `LET` boundary probe feeding the next statement — occupies a result
    /// slot but produces no output row; the walk skips it.
    Boundary,
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
                kind: ch.kind.clone(),
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
                // Strict composite tie-break (the cursor row itself is READ),
                // split per probe into the tie-boundary statement (`sent_at =
                // $at`, ids strictly past the cursor's) + the open-range
                // statement (`sent_at > $at`) so both ride the (channel,
                // sent_at) index instead of walking the channel — see the
                // module doc. The pairs are disjoint; step 4 re-merges them.
                sql.push_str(&format!(
                    "SELECT VALUE meta::id(id) FROM message
                        WHERE channel = type::record('channel', $cid_{i})
                          AND deleted_at = NONE
                          AND sent_at = $at_{i} AND meta::id(id) > $mid_{i}
                        LIMIT {UNREAD_COUNT_CAP};
                     SELECT VALUE meta::id(id) FROM message
                        WHERE channel = type::record('channel', $cid_{i})
                          AND deleted_at = NONE
                          AND sent_at > $at_{i}
                        LIMIT {UNREAD_COUNT_CAP};
                     SELECT VALUE meta::id(id) FROM message
                        WHERE channel = type::record('channel', $cid_{i})
                          AND deleted_at = NONE
                          AND sent_at = $at_{i} AND meta::id(id) > $mid_{i}
                          AND type::record('account', $acct) IN (pinged_users ?? [])
                        LIMIT 1;
                     SELECT VALUE meta::id(id) FROM message
                        WHERE channel = type::record('channel', $cid_{i})
                          AND deleted_at = NONE
                          AND sent_at > $at_{i}
                          AND type::record('account', $acct) IN (pinged_users ?? [])
                        LIMIT 1;"
                ));
                kinds.push(Stmt::Unread(i));
                kinds.push(Stmt::Unread(i));
                kinds.push(Stmt::Ping(i));
                kinds.push(Stmt::Ping(i));
            } else {
                // No cursor: surface the latest live row so the client can
                // baseline instead of glowing — strict (sent_at, id) order.
                // Issued as a LET boundary probe plus a tie-group point
                // statement (review M-12 follow-up): ORDER BY the computed
                // alias `id_key` cannot ride the (channel, sent_at) index,
                // so the single-statement `ORDER BY sent_at DESC, id_key
                // DESC LIMIT 1` form fed EVERY live row of the channel
                // through SortTopKByKey per call. The probe orders by the
                // RAW column (early-exiting backward index walk), then the
                // point statement resolves the newest instant's tie group
                // by id — SortTopK over the tie group only. Both ORDER BY
                // keys stay projected (SurrealDB orders by selected
                // aliases). An empty channel makes the boundary NONE and
                // the point statement match nothing, leaving the output
                // row's latest_* fields None as before. This LET-probe
                // shape (and its 3.1.3-only EXPLAIN evidence) falls under
                // the MSG_PROJECTION VERSION-SKEW runbook gate in
                // reading.rs — prod still runs 3.0.4.
                sql.push_str(&format!(
                    "LET $lat_{i} = array::first((SELECT VALUE sent_at FROM message
                        WHERE channel = type::record('channel', $cid_{i})
                          AND deleted_at = NONE
                        ORDER BY sent_at DESC LIMIT 1));
                     SELECT meta::id(id) AS id_key, sent_at FROM message
                        WHERE channel = type::record('channel', $cid_{i})
                          AND deleted_at = NONE
                          AND sent_at = $lat_{i}
                        ORDER BY id_key DESC LIMIT 1;"
                ));
                kinds.push(Stmt::Boundary);
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
                    // Two statements feed one channel (tie-boundary + open
                    // range); their row sets are disjoint by construction
                    // (`sent_at =` vs `>`), so summing is exact. Re-capped:
                    // each half is LIMITed independently, so the raw sum
                    // could otherwise exceed the cap the single-statement
                    // form enforced.
                    out[*i].unread = (out[*i].unread + ids.len()).min(UNREAD_COUNT_CAP);
                }
                Stmt::Ping(i) => {
                    let ids: Vec<String> = resp.take(stmt_idx)?;
                    out[*i].pinged |= !ids.is_empty();
                }
                // LET statements occupy a result slot (NONE) but feed no
                // output row — skip without a `take` (indexing is positional,
                // untaken slots are simply dropped).
                Stmt::Boundary => {}
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
