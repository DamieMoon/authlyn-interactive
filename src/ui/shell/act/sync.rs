//! W1 sync driver (hydrate-real / ssr no-op): an EventSource on /events
//! dispatches notify-and-fetch refreshes; the legacy 1.5s poll loop remains
//! the automatic fallback when SSE cannot hold a connection.
//!
//! The events are id-only ([`crate::protocol::SyncEvent`]) — nothing here
//! trusts payload content; every reaction is a refetch through the existing
//! permission-checked endpoints in [`super::message`].
//!
//! The driver, its closures, and its timers assume the shell mounts once per
//! page load.

use super::super::Shell;

#[cfg(feature = "hydrate")]
use super::hum;
#[cfg(feature = "hydrate")]
use super::message;
#[cfg(feature = "hydrate")]
use crate::protocol::SyncEvent;
#[cfg(feature = "hydrate")]
use leptos::prelude::*;
#[cfg(feature = "hydrate")]
use leptos::task::spawn_local;
#[cfg(feature = "hydrate")]
use wasm_bindgen::closure::Closure;
#[cfg(feature = "hydrate")]
use wasm_bindgen::JsCast;

/// Consecutive `onerror` firings tolerated before giving up on SSE for this
/// session. The browser auto-reconnects between errors, so five in a row
/// without a single delivered message means SSE genuinely cannot hold a
/// connection here (proxy buffering, hostile middlebox, …).
#[cfg(feature = "hydrate")]
const MAX_CONSECUTIVE_SSE_ERRORS: u32 = 5;

/// Start the background sync driver (idempotent via the `s.sync.polling`
/// latch). Called on shell mount so the rail/sidebar/friends stay live before
/// any channel is opened.
///
/// Strategy: open an `EventSource` on `/events` and refresh reactively per
/// event. If the constructor fails (ancient browser) or
/// [`MAX_CONSECUTIVE_SSE_ERRORS`] errors arrive without an intervening
/// message, fall back to [`message::start_poll`] for the rest of the session.
#[cfg(feature = "hydrate")]
pub fn start_sync(s: Shell) {
    if s.sync.polling.get_untracked() {
        return;
    }
    // Initial sync: paint lists + unread state immediately rather than waiting
    // for the first event (or the poll loop's first ~6s list tick).
    message::refresh_lists(s);
    message::refresh_unread(s);
    let Ok(es) = web_sys::EventSource::new("/events") else {
        // No EventSource support — poll forever, as before W1. The live chip
        // shows ● POLLING for the session (defensive set: false is also the
        // initial value).
        s.sync.sse_live.set(false);
        message::start_poll(s);
        return;
    };
    s.sync.polling.set(true);

    // Consecutive-error counter, shared by the handlers below: any delivered
    // message or successful (re)open resets it, so only an unbroken error run
    // trips the fallback.
    let errors = std::rc::Rc::new(std::cell::Cell::new(0u32));

    let on_message = {
        let errors = std::rc::Rc::clone(&errors);
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |ev: web_sys::MessageEvent| {
            errors.set(0);
            let Some(text) = ev.data().as_string() else {
                return;
            };
            // Unparseable frames are ignored (version skew / garbage): the
            // `#[serde(other)] Unknown` catch-all absorbs unknown event TYPES,
            // and `dispatch` drops `Unknown` on the floor.
            let Ok(event) = serde_json::from_str::<SyncEvent>(&text) else {
                return;
            };
            dispatch(s, event);
        })
    };
    es.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

    let on_error = {
        let errors = std::rc::Rc::clone(&errors);
        let es = es.clone();
        Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
            // Every error means the stream is NOT currently connected: drop
            // the chip to ● POLLING for the disconnect/reconnect window (e.g.
            // a deploy restart) — `onopen` restores ● LIVE when the browser's
            // auto-reconnect lands. Without this the chip claims LIVE while
            // disconnected (W3 whole-wave review).
            s.sync.sse_live.set(false);
            let n = errors.get().saturating_add(1);
            errors.set(n);
            if n == MAX_CONSECUTIVE_SSE_ERRORS {
                // SSE can't hold a connection this session: stop reconnecting,
                // release the latch, and hand over to the poll loop (which
                // re-takes the latch). The `==` check trips exactly once; later
                // stray errors only increment past MAX. (Re-entry would NOT be
                // safe: this branch itself releases the latch, so a second
                // entry would spawn a second eternal poll loop.)
                es.close();
                s.sync.sse_live.set(false); // chip flips to ● POLLING
                s.sync.polling.set(false);
                message::start_poll(s);
            }
        })
    };
    es.set_onerror(Some(on_error.as_ref().unchecked_ref()));

    // A successful (re)connect proves SSE can hold; only five FAILED attempts
    // in a row without one open demote to polling. Without this, "consecutive"
    // would be counted across delivered messages — a quiet tab's routine
    // disconnects (laptop sleep, deploy restarts) would slowly trip the
    // fallback even though every reconnect succeeded.
    let on_open = {
        let errors = std::rc::Rc::clone(&errors);
        Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
            errors.set(0);
            // The stream is (re)connected: the topbar chip reads ● LIVE.
            // Set alongside the error reset — the two signals are one fact.
            s.sync.sse_live.set(true);
        })
    };
    es.set_onopen(Some(on_open.as_ref().unchecked_ref()));

    // Deliberate bounded leak: ONE EventSource per shell mount, alive for the
    // whole session — `forget()` keeps the handlers valid without us managing
    // their lifetime from Rust.
    on_message.forget();
    on_error.forget();
    on_open.forget();
}

/// Route one parsed [`SyncEvent`] to the cheapest sufficient refresh.
#[cfg(feature = "hydrate")]
fn dispatch(s: Shell, event: SyncEvent) {
    match &event {
        // Forward-compat: an event type this build doesn't know. MUST be
        // ignored gracefully (protocol contract).
        SyncEvent::Unknown => {}
        // Metadata changed somewhere visible: refetch the lists AND the
        // unread summary (a new/removed channel shifts both). ListsChanged is
        // ALSO the server's post-lag resync nudge (the bus dropped events on
        // us), so reconcile the open channel too — a dropped message_created
        // for the open pane would otherwise stay invisible until the next
        // event happens to arrive (W1.5: completes the documented contract).
        SyncEvent::ListsChanged => {
            message::refresh_lists(s);
            message::refresh_unread(s);
            spawn_local(async move {
                message::refresh_open_channel(s).await;
            });
        }
        // W1.5 account-targeted nudges: the caller's read cursor moved on
        // ANOTHER device — refresh unread so this device's glow clears.
        SyncEvent::ReadStateChanged { .. } => message::refresh_unread(s),
        // W1.5: the friends/requests list changed for this account.
        // refresh_lists refetches friends (cheap, and keeps one entry point).
        SyncEvent::FriendsChanged => message::refresh_lists(s),
        // Channel-scoped events: a change in the OPEN channel reconciles the
        // message pane; anywhere else only the batched unread summary moves.
        _ => {
            // Corridor hum (UX evolution #4): a Typing ping or a just-created
            // message IS "someone is active here right now" — light the
            // channel-row mark straight from the already-received id-only
            // event. No fetch follows (the bus stays the only input);
            // `act::hum` decays the mark past the typing TTL. The row render
            // suppresses it on the OPEN channel (the typing line announces
            // the same fact louder there), so no open-channel branch here.
            if let SyncEvent::Typing { channel_id } | SyncEvent::MessageCreated { channel_id } =
                &event
            {
                hum::mark_hum(s, channel_id.clone());
            }
            let open = s.sel.sel_channel.get_untracked().map(|c| c.id);
            if event.channel_id().is_some() && event.channel_id() == open.as_deref() {
                spawn_local(async move {
                    message::refresh_open_channel(s).await;
                    // The open channel is always considered seen: advance its
                    // last-seen mark to the post-refresh cursor. The poll loop
                    // got this for free from `refresh_unread`'s ~6s prelude;
                    // under SSE nothing else advances it while the user sits
                    // reading. Re-read the selection (stale-guard: the refresh
                    // may have been dropped after a channel switch).
                    if let (Some(oc), Some(cur)) = (
                        s.sel.sel_channel.get_untracked().map(|c| c.id),
                        s.msg.cursor.get_untracked(),
                    ) {
                        message::set_last_seen(s, &oc, cur);
                    }
                    // Ghost Quill (W4/T7): a Typing event may mean a fresh
                    // draft, a MessageCreated means the sender's draft was
                    // just cleared server-side — refetch either way (the
                    // helper no-ops with the pref off). Draft TEXT rides this
                    // permission-checked fetch only; the event itself stays
                    // id-only.
                    message::refresh_ghost_drafts(s).await;
                    // Typing names piggyback on the page response and only
                    // change when something triggers a refresh; arm the
                    // staleness clearer whenever the pane shows typists.
                    if s.msg.typing.with_untracked(|t| !t.is_empty()) {
                        schedule_typing_clear(s);
                    }
                });
            } else if !matches!(event, SyncEvent::Typing { .. }) {
                // W1.5: typing in a NON-open channel cannot change unread
                // state — skipping the refetch kills the 2s-cadence /unread
                // storm one busy typist used to inflict on every member.
                message::refresh_unread(s);
            }
        }
    }
}

#[cfg(feature = "hydrate")]
thread_local! {
    /// True while a delayed typing-staleness refresh is already scheduled —
    /// keeps [`schedule_typing_clear`] from stacking one timer per keystroke.
    static TYPING_CLEAR_PENDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Clear a lingering typing indicator: under SSE a typist's periodic pings
/// each refresh the open channel (keeping the names fresh), but when the
/// typist STOPS no further event fires and the last name would linger
/// indefinitely (the poll loop used to provide this cadence). Schedule ONE
/// `refresh_open_channel` just past the server's 8s typing TTL; if typing
/// resumed meanwhile, the next Typing event re-arms this after the flag
/// clears.
#[cfg(feature = "hydrate")]
fn schedule_typing_clear(s: Shell) {
    if TYPING_CLEAR_PENDING.with(|c| c.replace(true)) {
        return; // one pending clearer at a time
    }
    spawn_local(async move {
        gloo_timers::future::TimeoutFuture::new(9_000).await;
        TYPING_CLEAR_PENDING.with(|c| c.set(false));
        // Only fetch if there is still something to clear; a channel switch
        // already reset `typing`, making this a no-op.
        if s.msg.typing.with_untracked(|t| !t.is_empty()) {
            message::refresh_open_channel(s).await;
        }
        // Ghost Quill rows share the typing cadence: when the typist STOPS
        // (no further pings, no further events) their server entry prunes at
        // the same ~8s TTL, but nothing would tell this client — so the same
        // clearer refetches the drafts (now empty) just past it (W4/T7).
        if s.msg.ghost_drafts.with_untracked(|g| !g.is_empty()) {
            message::refresh_ghost_drafts(s).await;
        }
    });
}

// ---- ssr stub ----

#[cfg(not(feature = "hydrate"))]
pub fn start_sync(_s: Shell) {}
