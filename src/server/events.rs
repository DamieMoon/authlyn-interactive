//! `GET /events` — the M1 SSE bus (ssr-only). Auth via the session cookie
//! ([`AuthAccount`]), exactly like every JSON route — and unlike every JSON
//! route, RE-CHECKED for the stream's lifetime: the session is re-validated
//! before every delivered frame AND at least once per
//! `AppState::sse_recheck_period` regardless of bus traffic — the re-check
//! runs on a DEADLINE that only an actual re-check (deadline lapse or frame
//! delivery) advances, so neither a fully quiet bus nor a busy bus whose
//! events are all filtered out for this connection can starve it (review
//! M-05 + follow-ups). Revocation (logout / password-reset lockout / expiry)
//! therefore ends the stream instead of leaving an
//! unkillable metadata feed. Wire format: unnamed
//! SSE `data:` frames each carrying one serialized [`SyncEvent`]. Filtering
//! (privacy) is per-connection: see `visible_channels` in `access`.
//!
//! ONE exception to both rules above: the dev hot-reload nudge
//! ([`SyncEvent::Reload`], emitted by `server::dev_reload`) is delivered to
//! EVERY connection — bypassing the visibility filter AND the M1.5 targeted
//! lane — as a DISTINCT NAMED `event: reload` frame (see `reload_frame`), so
//! the client can listen for it separately from message-notify frames. It
//! stays payload-free (id-only bus invariant): the frame itself is the signal.

use crate::protocol::SyncEvent;
use crate::server::access::visible_channels;
use crate::server::auth::{
    account_for_token_hash, session_token_hash, AuthAccount, SESSION_COOKIE,
};
use crate::server::errors::error_response;
use crate::server::state::{AppState, BusEvent};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::Response;
use axum_extra::extract::cookie::CookieJar;
use futures_util::stream::Stream;
use std::collections::HashSet;
use std::convert::Infallible;
use tokio::sync::broadcast;

/// Per-connection stream state for the unfold below.
struct Conn {
    rx: broadcast::Receiver<BusEvent>,
    visible: HashSet<String>,
    /// Set when the last visibility reload FAILED (and [`Conn::visible`] was
    /// cleared fail-closed); makes the next channel-scoped event retry the
    /// reload, so a transient DB error heals at the next event instead of
    /// staying silent until another lists_changed happens by.
    visible_stale: bool,
    state: AppState,
    account: String,
    /// SHA-256 of the caller's session token, re-checked before every
    /// delivered frame (review M-05) — see [`Conn::session_revoked`].
    session_token_hash: String,
    /// Deadline for the next forced session re-check (review M-05
    /// follow-up). Advanced ONLY when a re-check actually runs — never by
    /// merely RECEIVING a bus event — so it measures re-check silence, not
    /// bus activity: a stream fed nothing but events the privacy/target
    /// filters drop (which re-complete `recv()` without ever reaching the
    /// per-frame gate) still re-validates on the period.
    next_recheck: tokio::time::Instant,
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
            Ok(rows) => {
                self.visible = rows.into_iter().map(|r| r.channel_id).collect();
                self.visible_stale = false;
            }
            // On DB error: FAIL CLOSED (review M-07). A reload is how a
            // REVOCATION (kick / leave / guild-delete) reaches this
            // connection, so keeping the stale set would keep delivering ids
            // the caller may no longer see. Clearing it only costs silence,
            // and `visible_stale` schedules a retry on the next
            // channel-scoped event (one extra query per event while the DB
            // is erroring — same magnitude as the reload cost above).
            Err(e) => {
                self.visible.clear();
                self.visible_stale = true;
                tracing::error!(error = %e, "visible_channels reload failed — failing closed");
            }
        }
    }

    /// `true` when the session this stream was opened with no longer
    /// resolves (logout, password-reset lockout, expiry) — or when the check
    /// itself fails (fail-closed: ending the stream makes the client
    /// reconnect, and the reconnect re-authenticates through [`AuthAccount`]).
    ///
    /// Validity comes from the auth module's own hash-keyed lookup
    /// (`auth::session::account_for_token_hash` — the extractor's exact
    /// query), so this re-check can never drift from the per-request rule.
    /// Cost: one indexed point-select per DELIVERED frame per connection —
    /// strictly less than the authenticated follow-up fetch every delivered
    /// frame already triggers on the client.
    async fn session_revoked(&self) -> bool {
        match account_for_token_hash(&self.state, self.session_token_hash.clone()).await {
            Ok(Some(account_key)) => account_key != self.account,
            Ok(None) => true,
            Err(e) => {
                tracing::error!(error = %e, "session re-check failed — failing closed");
                true
            }
        }
    }
}

fn sse_frame(ev: &SyncEvent) -> Event {
    // SyncEvent is internally tagged (`#[serde(tag = "type")]`) with only
    // unit/struct variants, which cannot fail to serialize; a future NEWTYPE
    // variant wrapping a non-map COULD fail under internal tagging.
    Event::default().data(serde_json::to_string(ev).expect("SyncEvent serializes"))
}

/// The SSE event NAME the dev hot-reload nudge is delivered under, so the
/// client (`ui/shell/act/sync.rs`) listens for it SEPARATELY from the generic
/// data-only message frames. Kept in one place so server emit + the client
/// listener can never drift.
pub const RELOAD_EVENT_NAME: &str = "reload";

/// The dev hot-reload nudge's wire frame: a DISTINCT named `event: reload`
/// frame carrying a content-free empty-object `data:` sentinel. Payload-free by
/// design — the bus stays id-only, so the SIGNAL is the frame itself; nothing
/// rides it. `Event` requires a non-empty `data` line for the browser to
/// dispatch the named event, hence the `{}` placeholder (it carries nothing).
fn reload_frame() -> Event {
    Event::default().event(RELOAD_EVENT_NAME).data("{}")
}

/// GET /events — long-lived SSE stream of id-only sync events, filtered to
/// what the caller may see. Subscribes EAGERLY in the handler body (before the
/// response returns) — the test contract posts a message immediately after the
/// response resolves and must not miss its event.
pub async fn events(
    State(state): State<AppState>,
    jar: CookieJar,
    account: AuthAccount,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, Response> {
    // The raw token `AuthAccount` just resolved; its hash gates every
    // delivered frame below. The extractor guarantees the cookie exists —
    // but if that ever stops holding, fail closed rather than minting an
    // unkillable stream.
    let Some(token) = jar.get(SESSION_COOKIE).map(|c| c.value().to_owned()) else {
        return Err(error_response(
            StatusCode::UNAUTHORIZED,
            "authentication required",
        ));
    };

    // Subscribe BEFORE loading visibility so no event in between is missed;
    // an event for a channel created in that gap is recovered by the
    // lists_changed → reload path (Task 7).
    let rx = state.events.subscribe();

    // The INITIAL load failing must be an error response, not a deaf-but-200
    // stream (review M-45): the hydrate driver promotes SSE and retires its
    // poll fallback on a successful open, so a 200 carrying an empty set
    // would leave the client deaf-but-LIVE until some unrelated global
    // lists_changed happened by. A 500 makes EventSource fire `onerror` and
    // the client's backoff/poll-fallback machinery engage. (Mid-stream
    // reload failures are different — nothing can be "returned" then — and
    // fail closed inside `reload_visible`.)
    let visible: HashSet<String> = match visible_channels(&state, &account.0).await {
        Ok(rows) => rows.into_iter().map(|r| r.channel_id).collect(),
        Err(e) => {
            tracing::error!(error = %e, "initial visible_channels load failed");
            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage error",
            ));
        }
    };

    let conn = Conn {
        rx,
        visible,
        visible_stale: false,
        session_token_hash: session_token_hash(&token),
        next_recheck: tokio::time::Instant::now() + state.sse_recheck_period,
        state,
        account: account.0,
    };

    let stream = futures_util::stream::unfold(conn, |mut conn| async move {
        loop {
            // Deadline gate (review M-05 follow-up): re-validate the session
            // whenever `next_recheck` has lapsed, however we got here. The
            // per-frame gate below only fires on DELIVERY, and the filtered
            // `continue` paths complete `recv()` without delivering — so a
            // per-receive timer would re-arm forever under sustained
            // invisible traffic, and the `timeout_at` arm alone could be
            // starved by a backlogged receiver (tokio polls the inner future
            // before the timer). Checking the deadline eagerly at the top of
            // every iteration closes both holes. Revoked → end the stream;
            // still valid → advance the deadline and park again (nothing is
            // delivered, so no spurious frame reaches the client — axum's
            // KeepAlive layer owns the wire-level heartbeat independently of
            // this loop).
            if tokio::time::Instant::now() >= conn.next_recheck {
                if conn.session_revoked().await {
                    return None;
                }
                conn.next_recheck = tokio::time::Instant::now() + conn.state.sse_recheck_period;
            }
            // Decide what (if anything) this iteration delivers… The bus
            // `recv()` parks at most until the re-check DEADLINE — a fixed
            // instant `timeout_at` does not re-arm on receive, unlike a
            // per-receive `timeout(period, …)`.
            let received = tokio::time::timeout_at(conn.next_recheck, conn.rx.recv()).await;
            // Dev hot-reload (global, filter-bypassing): a `Reload` is the
            // test-deck "a new build landed — refresh" nudge. It is NOT
            // channel-scoped and NOT account-targeted: it must reach EVERY live
            // connection regardless of its visible-channel set or any `targets`
            // list, so short-circuit BOTH the per-connection visibility filter
            // and the M1.5 targeted lane before either can drop it. It still
            // passes through the per-frame session re-check below (a revoked
            // session ends the stream, never reloads), and rides the wire as a
            // DISTINCT named `event: reload` frame so the client listens for it
            // separately from the generic data-only message frames.
            if let Ok(Ok(BusEvent {
                event: SyncEvent::Reload,
                ..
            })) = &received
            {
                if conn.session_revoked().await {
                    return None;
                }
                conn.next_recheck = tokio::time::Instant::now() + conn.state.sse_recheck_period;
                return Some((Ok(reload_frame()), conn));
            }
            let event = match received {
                // Deadline lapsed with nothing to deliver: loop back — the
                // gate at the top runs the re-check and advances the
                // deadline before the next park.
                Err(_elapsed) => continue,
                Ok(Ok(BusEvent {
                    event,
                    targets: Some(targets),
                })) => {
                    // M1.5 account-targeted lane: deliver iff this connection's
                    // account is named, with NO visibility check — targeted
                    // events are id-only nudges about the target's own
                    // per-account state, not channel content.
                    if !targets.iter().any(|t| t == &conn.account) {
                        continue;
                    }
                    // Trap guard: a targeted ListsChanged (e.g. a future
                    // invite-accept nudging the new member) shifts what
                    // THIS connection may see. Without reloading here,
                    // `conn.visible` would go stale and the privacy
                    // filter below would silently drop this connection's
                    // subsequent channel events.
                    if matches!(event, SyncEvent::ListsChanged) {
                        conn.reload_visible().await;
                    }
                    event
                }
                Ok(Ok(BusEvent {
                    event,
                    targets: None,
                })) => {
                    let deliver = match event.channel_id() {
                        Some(cid) => {
                            // A previous reload failed and emptied the set
                            // fail-closed — retry before judging visibility.
                            if conn.visible_stale {
                                conn.reload_visible().await;
                            }
                            conn.visible.contains(cid)
                        }
                        None => {
                            // lists_changed (or forward-compat Unknown): visibility
                            // may have shifted under us.
                            conn.reload_visible().await;
                            true
                        }
                    };
                    if !deliver {
                        continue; // privacy filter
                    }
                    event
                }
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => {
                    // Dropped events: nudge the client to a full resync.
                    conn.reload_visible().await;
                    SyncEvent::ListsChanged
                }
                Ok(Err(broadcast::error::RecvError::Closed)) => return None,
            };
            // …then re-derive identity before delivering it (review M-05),
            // mirroring the per-request rule on JSON routes. A revoked
            // session ENDS the stream; the client's reconnect then 401s at
            // the extractor. (A stream that never delivers never reaches
            // this gate — the deadline gate at the top covers it on the
            // same period.)
            if conn.session_revoked().await {
                return None;
            }
            // This WAS a re-check — push the deadline out so an active
            // stream doesn't pay a redundant timeout-driven re-validation
            // right after a delivery-driven one.
            conn.next_recheck = tokio::time::Instant::now() + conn.state.sse_recheck_period;
            return Some((Ok(sse_frame(&event)), conn));
        }
    });

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

#[cfg(test)]
mod tests {
    //! Unit coverage for the DB-FAILURE arms, which the integration suite
    //! cannot reach (it would need `visible_channels` / the session lookup to
    //! fail while the rest of the arena works): an uninitialized SurrealDB
    //! client errors every query immediately, without any I/O.

    use super::*;
    use axum::response::IntoResponse;
    use surrealdb::engine::remote::ws::Client;
    use surrealdb::Surreal;

    /// An `AppState` whose every DB query fails (`Surreal::init()` is never
    /// connected) — fault injection for the error arms.
    fn dead_state() -> AppState {
        let db: Surreal<Client> = Surreal::init();
        AppState::new(db, std::env::temp_dir())
    }

    fn dead_conn(state: &AppState) -> Conn {
        Conn {
            rx: state.events.subscribe(),
            visible: HashSet::from(["channel-from-before-the-kick".to_string()]),
            visible_stale: false,
            state: state.clone(),
            account: "acct".into(),
            session_token_hash: session_token_hash("some-token"),
            next_recheck: tokio::time::Instant::now() + state.sse_recheck_period,
        }
    }

    /// Review M-07: a failed visibility reload must FAIL CLOSED. Keeping the
    /// stale set would keep delivering channel ids after a revocation
    /// (kick/leave/guild-delete) the failed reload was meant to apply.
    #[tokio::test]
    async fn reload_visible_failure_clears_the_set_instead_of_keeping_stale_grants() {
        let state = dead_state();
        let mut conn = dead_conn(&state);
        conn.reload_visible().await;
        assert!(
            conn.visible.is_empty(),
            "a failed reload must not keep possibly-revoked grants (fail closed)"
        );
        assert!(
            conn.visible_stale,
            "a failed reload must schedule a retry on the next channel event"
        );
    }

    /// Review M-05 (fail-closed arm): when the session re-check itself cannot
    /// reach the DB, the connection must count as revoked — the client's
    /// reconnect re-authenticates, so a transient error costs one reconnect,
    /// never an unauthenticated stream.
    #[tokio::test]
    async fn session_recheck_db_failure_counts_as_revoked() {
        let state = dead_state();
        let conn = dead_conn(&state);
        assert!(
            conn.session_revoked().await,
            "an unverifiable session must count as revoked (fail closed)"
        );
    }

    /// Review M-45: when the INITIAL visible-set load fails, the handler must
    /// return an ERROR response — never a deaf-but-200 stream. A 200 promotes
    /// the client's SSE driver and retires its poll fallback, so a deaf
    /// stream is sticky and self-masking; a 500 makes EventSource fire
    /// `onerror` and the fallback machinery engage.
    #[tokio::test]
    async fn initial_visible_set_load_failure_returns_500_instead_of_a_deaf_stream() {
        let jar = CookieJar::new().add(axum_extra::extract::cookie::Cookie::new(
            SESSION_COOKIE,
            "some-token",
        ));
        let resp = events(State(dead_state()), jar, AuthAccount("ghost".into()))
            .await
            .into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    /// Review M-05 belt-and-braces: a connect with no session cookie must be
    /// rejected pre-stream. Unreachable through the router (`AuthAccount`
    /// 401s first), but the handler must not assume that.
    #[tokio::test]
    async fn missing_session_cookie_is_rejected_before_any_stream_exists() {
        let resp = events(
            State(dead_state()),
            CookieJar::new(),
            AuthAccount("ghost".into()),
        )
        .await
        .into_response();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
