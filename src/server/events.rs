//! `GET /events` — the W1 SSE bus (ssr-only). Auth via the session cookie
//! ([`AuthAccount`]), exactly like every JSON route. Wire format: unnamed SSE
//! `data:` frames each carrying one serialized [`SyncEvent`]. Filtering
//! (privacy) is per-connection: see [`visible_channels`] below.

use crate::protocol::SyncEvent;
use crate::server::auth::AuthAccount;
use crate::server::state::AppState;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::stream::Stream;
use std::collections::HashSet;
use std::convert::Infallible;
use surrealdb::types::SurrealValue;
use tokio::sync::broadcast;

/// Load the channel ids the account may currently see (live text channels in
/// guilds where they are a member). Two parameterized statements, one
/// round-trip. Returns `(channel_id, guild_id)` pairs; this module's consumer
/// only needs the channel ids but `/unread` (Task 8, same helper) wants the
/// guild mapping too.
pub(crate) async fn visible_channels(
    state: &AppState,
    account: &str,
) -> surrealdb::Result<Vec<(String, String)>> {
    #[derive(SurrealValue)]
    struct Row {
        channel_id: String,
        guild_id: String,
    }
    let mut resp = state
        .db
        .query(
            "LET $gids = (SELECT VALUE guild FROM guild_member
                 WHERE account = type::record('account', $account));
             SELECT meta::id(id) AS channel_id, meta::id(guild) AS guild_id FROM channel
                 WHERE deleted_at = NONE AND kind = 'text'
                   AND guild IN $gids AND guild.deleted_at = NONE;",
        )
        .bind(("account", account.to_string()))
        .await?
        .check()?;
    // Statement 0 is the LET (no materialized rows); the SELECT is take(1).
    let rows: Vec<Row> = resp.take(1)?;
    Ok(rows
        .into_iter()
        .map(|r| (r.channel_id, r.guild_id))
        .collect())
}

/// Per-connection stream state for the unfold below.
struct Conn {
    rx: broadcast::Receiver<SyncEvent>,
    visible: HashSet<String>,
    state: AppState,
    account: String,
}

impl Conn {
    async fn reload_visible(&mut self) {
        match visible_channels(&self.state, &self.account).await {
            Ok(rows) => self.visible = rows.into_iter().map(|(c, _g)| c).collect(),
            // On DB error: keep the stale set. Fail-closed enough (no new
            // grants leak in), and the next lists_changed retries.
            Err(e) => tracing::error!(error = %e, "visible_channels reload failed"),
        }
    }
}

fn sse_frame(ev: &SyncEvent) -> Event {
    // Serialization of a unit-tagged enum cannot fail.
    Event::default().data(serde_json::to_string(ev).expect("SyncEvent serializes"))
}

/// GET /events — long-lived SSE stream of id-only sync events, filtered to
/// what the caller may see. Subscribes EAGERLY in the handler body (before the
/// response returns) — the test contract posts a message immediately after the
/// response resolves and must not miss its event.
pub async fn events(
    State(state): State<AppState>,
    account: AuthAccount,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // Subscribe BEFORE loading visibility so no event in between is missed;
    // an event for a channel created in that gap is recovered by the
    // lists_changed → reload path (Task 7).
    let rx = state.events.subscribe();
    let mut conn = Conn {
        rx,
        visible: HashSet::new(),
        state,
        account: account.0,
    };
    conn.reload_visible().await;

    let stream = futures_util::stream::unfold(conn, |mut conn| async move {
        loop {
            match conn.rx.recv().await {
                Ok(ev) => match ev.channel_id() {
                    Some(cid) if !conn.visible.contains(cid) => continue, // privacy filter
                    Some(_) => return Some((Ok(sse_frame(&ev)), conn)),
                    None => {
                        // lists_changed (or forward-compat Unknown): visibility
                        // may have shifted under us.
                        conn.reload_visible().await;
                        return Some((Ok(sse_frame(&ev)), conn));
                    }
                },
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // Dropped events: nudge the client to a full resync.
                    conn.reload_visible().await;
                    return Some((Ok(sse_frame(&SyncEvent::ListsChanged)), conn));
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}
