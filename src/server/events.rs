//! `GET /events` — the W1 SSE bus (ssr-only). Auth via the session cookie
//! ([`AuthAccount`]), exactly like every JSON route. Wire format: unnamed SSE
//! `data:` frames each carrying one serialized [`SyncEvent`]. Filtering
//! (privacy) is per-connection: see [`visible_channels`] in `access`.

use crate::protocol::SyncEvent;
use crate::server::access::visible_channels;
use crate::server::auth::AuthAccount;
use crate::server::state::{AppState, BusEvent};
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::stream::Stream;
use std::collections::HashSet;
use std::convert::Infallible;
use tokio::sync::broadcast;

/// Per-connection stream state for the unfold below.
struct Conn {
    rx: broadcast::Receiver<BusEvent>,
    visible: HashSet<String>,
    state: AppState,
    account: String,
}

impl Conn {
    /// Re-derive the visible-channel set from the DB.
    ///
    /// Amplification cost: one DB query per connection per lists_changed /
    /// Lagged event — N connections × M list mutations. Fine at this
    /// instance's scale (N≈10); if that ever changes, coalesce by draining
    /// the receiver via `try_recv` before reloading.
    async fn reload_visible(&mut self) {
        match visible_channels(&self.state, &self.account).await {
            Ok(rows) => self.visible = rows.into_iter().map(|r| r.channel_id).collect(),
            // On DB error: keep the stale set. Fail-closed enough (no new
            // grants leak in), and the next lists_changed retries.
            Err(e) => tracing::error!(error = %e, "visible_channels reload failed"),
        }
    }
}

fn sse_frame(ev: &SyncEvent) -> Event {
    // SyncEvent is internally tagged (`#[serde(tag = "type")]`) with only
    // unit/struct variants, which cannot fail to serialize; a future NEWTYPE
    // variant wrapping a non-map COULD fail under internal tagging.
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
                Ok(be) => {
                    // W1.5 account-targeted lane: deliver iff this connection's
                    // account is named, with NO visibility check — targeted
                    // events are id-only nudges about the target's own
                    // per-account state, not channel content.
                    if let Some(targets) = &be.targets {
                        if targets.iter().any(|t| t == &conn.account) {
                            // Trap guard: a targeted ListsChanged (e.g. a future
                            // invite-accept nudging the new member) shifts what
                            // THIS connection may see. Without reloading here,
                            // `conn.visible` would go stale and the privacy
                            // filter below would silently drop this connection's
                            // subsequent channel events.
                            if matches!(be.event, SyncEvent::ListsChanged) {
                                conn.reload_visible().await;
                            }
                            return Some((Ok(sse_frame(&be.event)), conn));
                        }
                        continue;
                    }
                    match be.event.channel_id() {
                        Some(cid) if !conn.visible.contains(cid) => continue, // privacy filter
                        Some(_) => return Some((Ok(sse_frame(&be.event)), conn)),
                        None => {
                            // lists_changed (or forward-compat Unknown): visibility
                            // may have shifted under us.
                            conn.reload_visible().await;
                            return Some((Ok(sse_frame(&be.event)), conn));
                        }
                    }
                }
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
