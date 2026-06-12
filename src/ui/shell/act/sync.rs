//! W1 sync driver (hydrate-real / ssr no-op): an EventSource on /events
//! dispatches notify-and-fetch refreshes; the legacy 1.5s poll loop remains
//! the automatic fallback when SSE cannot hold a connection.
//!
//! Self-healing (UX evolution #2): the poll fallback is no longer terminal.
//! After a demotion a capped-backoff task keeps probing /events, and wake
//! listeners (`visibilitychange` → visible, `online`) probe immediately and
//! refetch the batched unread truth — the two moments a phone actually
//! regains network. A successful probe PROMOTES itself to the driver via a
//! generation bump that makes the poll loop and any stale EventSource
//! self-terminate, so drivers can never double-run. The `● LIVE` chip is set
//! only on a current-generation `onopen` and dropped on every error /
//! demotion / detected-dead stream, so it never lies through a transition.
//!
//! The events are id-only ([`crate::protocol::SyncEvent`]) — nothing here
//! trusts payload content; every reaction is a refetch through the existing
//! permission-checked endpoints in [`super::message`]. The recovery work
//! changes only when the client LISTENS, never what rides the bus.
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
use std::cell::{Cell, RefCell};
#[cfg(feature = "hydrate")]
use std::rc::Rc;
#[cfg(feature = "hydrate")]
use wasm_bindgen::closure::Closure;
#[cfg(feature = "hydrate")]
use wasm_bindgen::JsCast;

/// Consecutive `onerror` firings tolerated before giving up on SSE for this
/// driver. The browser auto-reconnects between errors, so five in a row
/// without a single delivered message means SSE genuinely cannot hold a
/// connection here (proxy buffering, hostile middlebox, …).
#[cfg(feature = "hydrate")]
const MAX_CONSECUTIVE_SSE_ERRORS: u32 = 5;

/// First resurrection-probe delay after demoting to the poll fallback…
#[cfg(feature = "hydrate")]
const SSE_RETRY_BASE_MS: u32 = 30_000;

/// …doubling per attempt up to this cap (5 min), so a long outage costs a
/// handful of one-shot probes rather than an eternal reconnect storm.
#[cfg(feature = "hydrate")]
const SSE_RETRY_CAP_MS: u32 = 300_000;

/// Wake refreshes (lists + unread + open channel) fire at most once per this
/// window — rapid visibility flapping (app-switcher swipes, alt-tabbing)
/// must not turn into a fetch storm. SSE resurrection is NOT throttled by
/// this; `PROBE_PENDING` + the backoff schedule already bound it.
#[cfg(feature = "hydrate")]
const WAKE_REFRESH_THROTTLE_MS: f64 = 3_000.0;

#[cfg(feature = "hydrate")]
thread_local! {
    /// Driver generation: bumped on every handover (mount-time connect,
    /// probe promotion, demotion to polling). Every driver — an EventSource's
    /// handlers, the poll loop, the backoff task — captures the generation it
    /// was installed under and self-terminates the moment the global moves
    /// on, so a handover can never leave two drivers running.
    static SYNC_GEN: Cell<u64> = const { Cell::new(0) };
    /// True while a resurrection probe is in flight — exactly one at a time,
    /// shared by the backoff task and the wake listeners.
    static PROBE_PENDING: Cell<bool> = const { Cell::new(false) };
    /// Handle to the PROMOTED (current-driver) EventSource, kept so the wake
    /// listener can detect a terminally CLOSED stream — a frozen mobile PWA
    /// can kill the connection without ever delivering a final error event.
    static CURRENT_ES: RefCell<Option<web_sys::EventSource>> = const { RefCell::new(None) };
    /// Timestamp (ms since epoch) of the last wake refresh, backing
    /// [`WAKE_REFRESH_THROTTLE_MS`].
    static LAST_WAKE_REFRESH: Cell<f64> = const { Cell::new(0.0) };
}

/// The current driver generation — read by [`message::start_poll`]'s loop so
/// a promoted SSE driver retires the poll fallback at its next tick.
#[cfg(feature = "hydrate")]
pub(super) fn current_gen() -> u64 {
    SYNC_GEN.get()
}

/// Advance the driver generation (invalidating every outstanding driver and
/// task) and return the new value for the incoming driver to hold.
#[cfg(feature = "hydrate")]
fn bump_gen() -> u64 {
    let next = SYNC_GEN.get() + 1;
    SYNC_GEN.set(next);
    next
}

/// Start the background sync driver (idempotent via the `s.sync.polling`
/// latch). Called on shell mount so the rail/sidebar/friends stay live before
/// any channel is opened.
///
/// Strategy: open an `EventSource` on `/events` and refresh reactively per
/// event. If the constructor fails (ancient browser) fall back to
/// [`message::start_poll`] for the rest of the session; if
/// [`MAX_CONSECUTIVE_SSE_ERRORS`] errors arrive without an intervening
/// message, [`demote`] hands over to the poll loop AND arms recovery —
/// backoff probes plus wake-event probes keep trying to resurrect SSE.
#[cfg(feature = "hydrate")]
pub fn start_sync(s: Shell) {
    if s.sync.polling.get_untracked() {
        return;
    }
    // Initial sync: paint lists + unread state immediately rather than waiting
    // for the first event (or the poll loop's first ~6s list tick).
    message::refresh_lists(s);
    message::refresh_unread(s);
    // PWA-resume / network-regain hooks, installed once before any driver so
    // even the no-EventSource poll path gets truthful wake refreshes.
    install_wake_listeners(s);
    if !connect(s, true) {
        // No EventSource support — poll forever, as before W1. The live chip
        // shows ● POLLING for the session (defensive set: false is also the
        // initial value). Resurrection probes no-op here too: a browser
        // without the constructor will not grow one mid-session.
        s.sync.sse_live.set(false);
        message::start_poll(s);
        return;
    }
    s.sync.polling.set(true);
}

/// Open an EventSource on /events and wire the shared driver handlers.
/// Returns false when the constructor itself fails (no EventSource support).
///
/// `promote_at_birth` — true for the mount-time connect: it IS the driver
/// from creation, tolerating [`MAX_CONSECUTIVE_SSE_ERRORS`] browser
/// auto-retries before demoting (the pre-evolution behavior). False for a
/// resurrection probe: one-shot — its first error closes it (polling keeps
/// providing sync, and the browser's own auto-retry must not bypass the
/// backoff schedule), while a successful open PROMOTES it to the driver.
#[cfg(feature = "hydrate")]
fn connect(s: Shell, promote_at_birth: bool) -> bool {
    let Ok(es) = web_sys::EventSource::new("/events") else {
        return false;
    };
    // The generation this stream considers itself valid under. A probe holds
    // the generation it was SPAWNED under (so any handover while it connects
    // invalidates it); promotion installs a fresh bump.
    let my_gen = Rc::new(Cell::new(if promote_at_birth {
        bump_gen()
    } else {
        current_gen()
    }));
    let promoted = Rc::new(Cell::new(promote_at_birth));
    if promote_at_birth {
        CURRENT_ES.set(Some(es.clone()));
    }

    // Consecutive-error counter, shared by the handlers below: any delivered
    // message or successful (re)open resets it, so only an unbroken error run
    // trips the fallback.
    let errors = Rc::new(Cell::new(0u32));

    let on_message = {
        let errors = Rc::clone(&errors);
        let my_gen = Rc::clone(&my_gen);
        let es = es.clone();
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |ev: web_sys::MessageEvent| {
            if current_gen() != my_gen.get() {
                // A newer driver took over: this stream retires itself on its
                // next event of any kind.
                es.close();
                return;
            }
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
        let errors = Rc::clone(&errors);
        let my_gen = Rc::clone(&my_gen);
        let promoted = Rc::clone(&promoted);
        let es = es.clone();
        Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
            if current_gen() != my_gen.get() {
                es.close();
                if !promoted.get() {
                    PROBE_PENDING.set(false);
                }
                return;
            }
            if !promoted.get() {
                // One-shot probe failed: close before the browser's built-in
                // auto-retry can hammer outside the backoff schedule, and
                // release the probe slot. The poll loop is still the driver;
                // the chip already reads ● POLLING.
                es.close();
                PROBE_PENDING.set(false);
                return;
            }
            // Every error means the stream is NOT currently connected: drop
            // the chip to ● POLLING for the disconnect/reconnect window (e.g.
            // a deploy restart) — `onopen` restores ● LIVE when the browser's
            // auto-reconnect lands. Without this the chip claims LIVE while
            // disconnected (W3 whole-wave review).
            s.sync.sse_live.set(false);
            let n = errors.get().saturating_add(1);
            errors.set(n);
            if n == MAX_CONSECUTIVE_SSE_ERRORS {
                // SSE can't hold a connection right now: stop reconnecting
                // and hand over to the poll loop + recovery probes. The `==`
                // check trips exactly once per driver; later stray errors
                // only increment past MAX (and hit the staleness guard above,
                // since `demote` bumps the generation).
                demote(s);
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
        let errors = Rc::clone(&errors);
        let my_gen = Rc::clone(&my_gen);
        let promoted = Rc::clone(&promoted);
        let es = es.clone();
        Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
            if current_gen() != my_gen.get() {
                // A newer driver took over while this stream (re)connected:
                // stand down instead of double-running.
                es.close();
                if !promoted.get() {
                    PROBE_PENDING.set(false);
                }
                return;
            }
            if !promoted.get() {
                PROBE_PENDING.set(false);
                if s.sync.sse_live.get_untracked() {
                    // An existing driver's auto-reconnect beat this probe to
                    // a healthy stream — don't churn drivers over it.
                    es.close();
                    return;
                }
                // PROMOTION: this probe is the driver now. The bump retires
                // the poll loop (at its next tick) and the backoff task; the
                // `polling` latch stays held — ownership transfers, so
                // channel.rs's belt-and-braces `start_poll` keeps no-opping.
                my_gen.set(bump_gen());
                promoted.set(true);
                CURRENT_ES.set(Some(es.clone()));
                // Truth resync: events from the polling era never rode this
                // stream — refetch the batched state once.
                message::refresh_lists(s);
                message::refresh_unread(s);
                spawn_local(async move {
                    message::refresh_open_channel(s).await;
                });
            }
            errors.set(0);
            // The stream is (re)connected: the topbar chip reads ● LIVE.
            // Set alongside the error reset — the two signals are one fact.
            s.sync.sse_live.set(true);
        })
    };
    es.set_onopen(Some(on_open.as_ref().unchecked_ref()));

    // Deliberate bounded leak: one EventSource per driver handover —
    // `forget()` keeps the handlers valid without us managing their lifetime
    // from Rust. Probes are bounded by the backoff schedule (one per
    // 30s..5min while degraded), so the leak stays a few hundred bytes per
    // retry at worst.
    on_message.forget();
    on_error.forget();
    on_open.forget();
    true
}

/// Hand the session to the poll fallback and arm recovery: close the
/// (possibly already dead) promoted stream, flip the chip to ● POLLING,
/// invalidate every outstanding driver/task via a generation bump, then start
/// the poll loop and the capped-backoff resurrection task under the new
/// generation. Reached from the driver's MAX-errors branch and from [`wake`]
/// when it finds the stream terminally CLOSED.
#[cfg(feature = "hydrate")]
fn demote(s: Shell) {
    if let Some(es) = CURRENT_ES.take() {
        es.close();
    }
    s.sync.sse_live.set(false); // chip flips to ● POLLING
    let gen = bump_gen();
    // Release-and-retake the latch across the handover (start_poll asserts
    // it), exactly like the pre-evolution fallback did.
    s.sync.polling.set(false);
    message::start_poll(s);
    start_retry(s, gen);
}

/// While polling, keep trying to resurrect SSE: probe /events on a doubling
/// 30s→5min schedule with ±20% jitter (so a fleet of clients doesn't
/// reconnect in lockstep after an outage). The task self-terminates when its
/// generation goes stale — a probe got promoted, or a newer demotion armed a
/// fresh task. Probes are skipped while the document is hidden: a background
/// tab can't usefully hold the stream, and [`wake`] probes immediately on
/// return to the foreground.
#[cfg(feature = "hydrate")]
fn start_retry(s: Shell, gen: u64) {
    spawn_local(async move {
        let mut delay = SSE_RETRY_BASE_MS;
        loop {
            let jitter = 0.8 + 0.4 * js_sys::Math::random();
            gloo_timers::future::TimeoutFuture::new((f64::from(delay) * jitter) as u32).await;
            if current_gen() != gen {
                break;
            }
            if !document_hidden() {
                probe(s);
            }
            delay = delay.saturating_mul(2).min(SSE_RETRY_CAP_MS);
        }
    });
}

/// Fire one SSE resurrection probe; no-op while another is in flight (the
/// backoff task and both wake listeners share the slot).
#[cfg(feature = "hydrate")]
fn probe(s: Shell) {
    if PROBE_PENDING.get() {
        return;
    }
    PROBE_PENDING.set(true);
    if !connect(s, false) {
        // No EventSource support — there is nothing to resurrect, ever.
        PROBE_PENDING.set(false);
    }
}

/// One wake pass, shared by `visibilitychange` → visible and `online`:
/// refetch the batched truth (lists + unread + the open channel) so the UI is
/// honest the second the app foregrounds, then resurrect SSE if the stream is
/// not healthily open. A frozen mobile PWA can leave the EventSource
/// terminally CLOSED with `sse_live` still true (no error event ever fired) —
/// detect that, demote so the chip stops lying, and probe immediately.
#[cfg(feature = "hydrate")]
fn wake(s: Shell) {
    if !s.sync.polling.get_untracked() || document_hidden() {
        // No driver mounted yet, or woke into a still-hidden tab.
        return;
    }
    // Truth refresh, throttled so visibility flapping can't fetch-storm.
    let now = js_sys::Date::now();
    if now - LAST_WAKE_REFRESH.get() >= WAKE_REFRESH_THROTTLE_MS {
        LAST_WAKE_REFRESH.set(now);
        message::refresh_lists(s);
        message::refresh_unread(s);
        spawn_local(async move {
            message::refresh_open_channel(s).await;
        });
    }
    // Resurrection — never throttled: the device just told us conditions
    // changed, and `probe` is already single-flight + constructor-cheap.
    if s.sync.sse_live.get_untracked() {
        let closed = CURRENT_ES.with_borrow(|c| {
            c.as_ref()
                .is_none_or(|es| es.ready_state() == web_sys::EventSource::CLOSED)
        });
        if closed {
            demote(s);
            probe(s);
        }
    } else {
        // Erroring or polling: don't wait for the browser auto-retry or the
        // backoff schedule.
        probe(s);
    }
}

/// Attach the PWA-resume listeners once per shell mount:
/// `document.visibilitychange` (filtered to → visible inside [`wake`]) and
/// `window.online` both funnel into the same wake pass. visibilitychange is
/// registered on the DOCUMENT — it is dispatched there, and relying on it
/// bubbling to window is exactly the kind of subtlety old mobile WebKit gets
/// wrong. Deliberate bounded leak, same as the driver handlers: two closures
/// alive for the whole session.
#[cfg(feature = "hydrate")]
fn install_wake_listeners(s: Shell) {
    let Some(win) = web_sys::window() else {
        return;
    };
    let Some(doc) = win.document() else {
        return;
    };
    let on_visible = Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| wake(s));
    let _ = doc
        .add_event_listener_with_callback("visibilitychange", on_visible.as_ref().unchecked_ref());
    let on_online = Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| wake(s));
    let _ = win.add_event_listener_with_callback("online", on_online.as_ref().unchecked_ref());
    on_visible.forget();
    on_online.forget();
}

/// True when the document is hidden (background tab / backgrounded PWA).
/// Fail-open to visible: a missing document only happens in non-browser
/// contexts where suppressing sync would be the wrong default.
#[cfg(feature = "hydrate")]
fn document_hidden() -> bool {
    web_sys::window()
        .and_then(|w| w.document())
        .is_some_and(|d| d.hidden())
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
