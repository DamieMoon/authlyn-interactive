//! M1 sync driver (hydrate-real / ssr no-op): an EventSource on /events
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
//! Teardown (review M-10): the `forget()`-ed closures are permanent, so they
//! must never assume the Shell outlives them. Logout calls `shutdown`
//! (generation bump + stream close + probe-slot release), and every entry
//! point reachable from a forgotten closure or detached timer `try_`-reads
//! the Shell first, so a disposed shell degrades to a no-op — never a panic
//! (= WASM abort).

use super::super::Shell;

#[cfg(feature = "hydrate")]
use super::super::Pane;

#[cfg(feature = "hydrate")]
use super::message;
#[cfg(feature = "hydrate")]
use crate::client::api;
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

/// A resurrection probe that has neither opened nor errored within this bound
/// is considered wedged (review M-30): a frozen mobile PWA can kill a
/// CONNECTING probe without ever delivering a final event — the module's own
/// frozen-PWA model — which would hold `PROBE_PENDING` forever and
/// permanently disable SSE resurrection for the session. The next probe
/// attempt past this age closes the zombie and takes the slot over.
#[cfg(feature = "hydrate")]
const PROBE_TIMEOUT_MS: f64 = 15_000.0;

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
    /// Handle to the in-flight resurrection-probe EventSource, so a probe
    /// killed without a final event can be detected, closed, and replaced
    /// instead of wedging the probe slot forever (review M-30).
    static PROBE_ES: RefCell<Option<web_sys::EventSource>> = const { RefCell::new(None) };
    /// `Date::now()` at the moment the current probe was launched — backs
    /// [`PROBE_TIMEOUT_MS`].
    static PROBE_STARTED: Cell<f64> = const { Cell::new(0.0) };
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
        // No EventSource support — poll forever, as before M1. The live chip
        // shows ● POLLING for the session (defensive set: false is also the
        // initial value). Resurrection probes no-op here too: a browser
        // without the constructor will not grow one mid-session.
        s.sync.sse_live.set(false);
        message::start_poll(s);
        return;
    }
    s.sync.polling.set(true);
}

/// Tear the sync machinery down at logout (review M-10). The forgotten
/// closures (driver handlers, wake listeners, detached timers) cannot be
/// dropped, so teardown means disarming them: the generation bump retires
/// every outstanding driver/task at its next event or tick, closing the
/// promoted stream stops `/events` feeding a client whose session is gone,
/// and releasing the probe slot keeps a half-open probe from leaking into the
/// next login. The closures themselves then no-op forever — their entry
/// points `try_`-read the disposed Shell and bail.
#[cfg(feature = "hydrate")]
pub fn shutdown() {
    bump_gen();
    if let Some(es) = CURRENT_ES.take() {
        es.close();
    }
    if let Some(es) = PROBE_ES.take() {
        es.close();
    }
    PROBE_PENDING.set(false);
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
    } else {
        // Track the probe so [`probe`] can detect and close one that wedged
        // without ever delivering a final event (review M-30).
        PROBE_ES.set(Some(es.clone()));
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
            // disconnected (M3 whole-wave review). `try_set` (review M-10):
            // `Some` back means the shell was disposed without `shutdown`
            // running — this stream is orphaned, so stand down for good
            // instead of panicking on the dead signal.
            if s.sync.sse_live.try_set(false).is_some() {
                es.close();
                return;
            }
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
                let Some(live) = s.sync.sse_live.try_get_untracked() else {
                    // Shell disposed without `shutdown` (review M-10):
                    // this probe is orphaned — stand down for good.
                    es.close();
                    return;
                };
                if live {
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
                resync_truth(s);
            } else if errors.get() > 0 {
                // This open ENDS a disconnect gap on the PROMOTED stream
                // (review M-18): events emitted while the browser was
                // auto-reconnecting never rode this stream, and the server
                // bus keeps no history (no Last-Event-ID replay) — the
                // probe-promotion rationale above applies verbatim, so run
                // the same truth resync. Feeds the wake throttle so a
                // near-simultaneous visibility/online wake doesn't re-fetch
                // the same truth.
                if s.sync.sse_live.try_get_untracked().is_none() {
                    // Shell disposed (review M-10): orphaned stream.
                    es.close();
                    return;
                }
                LAST_WAKE_REFRESH.set(js_sys::Date::now());
                resync_truth(s);
            }
            errors.set(0);
            // The stream is (re)connected: the topbar chip reads ● LIVE.
            // Set alongside the error reset — the two signals are one fact.
            // (`try_`: a disposed shell just means there is no chip left.)
            let _ = s.sync.sse_live.try_set(true);
        })
    };
    es.set_onopen(Some(on_open.as_ref().unchecked_ref()));

    // Dev hot-reload (test-deck auto-refresh): the server emits a DISTINCT
    // NAMED `event: reload` frame when a new build is deployed (the deck runs
    // the compiled binary, so there is no cargo-leptos live-reload). It arrives
    // on its OWN event name — never `onmessage` (which fires only for unnamed
    // `message` frames) — so it can't be confused with a `SyncEvent` notify.
    // The reaction is the whole point: navigate onto the freshly deployed
    // bundle. Generation-guarded like every other handler so a retired stream
    // (handover/logout) can't trigger a reload; the payload is ignored entirely
    // (the frame's mere arrival is the signal — id-only bus).
    let on_reload = {
        let my_gen = Rc::clone(&my_gen);
        let es = es.clone();
        Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
            if current_gen() != my_gen.get() {
                es.close();
                return;
            }
            if let Some(loc) = web_sys::window().map(|w| w.location()) {
                let _ = loc.reload();
            }
        })
    };
    let _ = es.add_event_listener_with_callback("reload", on_reload.as_ref().unchecked_ref());

    // Deliberate bounded leak: one EventSource per driver handover —
    // `forget()` keeps the handlers valid without us managing their lifetime
    // from Rust. Probes are bounded by the backoff schedule (one per
    // 30s..5min while degraded), so the leak stays a few hundred bytes per
    // retry at worst.
    on_message.forget();
    on_error.forget();
    on_open.forget();
    on_reload.forget();
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
    // Chip flips to ● POLLING. `try_set` (review M-10): `Some` back means
    // the shell is disposed — there is no session left to hand to the poll
    // fallback, so bail before `start_poll` touches dead signals.
    if s.sync.sse_live.try_set(false).is_some() {
        return;
    }
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
        // Self-heal (review M-30): a probe killed without a final event (the
        // module's own frozen-PWA model) would otherwise hold this slot for
        // the rest of the session, permanently disabling resurrection — the
        // chip stuck on ● POLLING, Ghost Quill gone. Within the bound the
        // probe is legitimately in flight; past it, close the zombie and
        // take the slot over.
        if js_sys::Date::now() - PROBE_STARTED.get() < PROBE_TIMEOUT_MS {
            return;
        }
        if let Some(es) = PROBE_ES.take() {
            es.close();
        }
    }
    PROBE_PENDING.set(true);
    PROBE_STARTED.set(js_sys::Date::now());
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
    // `try_` (review M-10): the wake listeners are forgotten closures that
    // survive logout — a tab-switch on the login page still fires
    // visibilitychange, and the captured Shell's signals are disposed by
    // then. A dead shell means there is nothing to wake: bail. The remaining
    // reads below run on the same tick as this proof, so they stay plain.
    let Some(polling) = s.sync.polling.try_get_untracked() else {
        return;
    };
    if !polling || document_hidden() {
        // No driver mounted yet, or woke into a still-hidden tab.
        return;
    }
    // Truth refresh, throttled so visibility flapping can't fetch-storm.
    // (This pass is also what catches the read-mark up after the M-04
    // visibility gate deferred it: refresh_unread's open-channel prelude
    // re-runs set_last_seen now that the document is visible.)
    let now = js_sys::Date::now();
    if now - LAST_WAKE_REFRESH.get() >= WAKE_REFRESH_THROTTLE_MS {
        LAST_WAKE_REFRESH.set(now);
        resync_truth(s);
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
/// contexts where suppressing sync would be the wrong default. `pub(super)`:
/// [`message::set_last_seen`] gates the cross-device read-mark on it
/// (review M-04).
#[cfg(feature = "hydrate")]
pub(super) fn document_hidden() -> bool {
    web_sys::window()
        .and_then(|w| w.document())
        .is_some_and(|d| d.hidden())
}

/// Refetch the batched truth — lists, unread summary, and the open channel
/// page. The shared "events may have been missed" resync: probe promotion,
/// the post-gap reconnect (review M-18), the wake pass, and the server's
/// `ListsChanged` lag-nudge all funnel here so the four can never drift
/// apart. Callers must have proven the shell alive (a `try_` read) on the
/// same tick.
#[cfg(feature = "hydrate")]
fn resync_truth(s: Shell) {
    message::refresh_lists(s);
    message::refresh_unread(s);
    spawn_local(async move {
        message::refresh_open_channel(s).await;
    });
}

/// Route one parsed [`SyncEvent`] to the cheapest sufficient refresh.
#[cfg(feature = "hydrate")]
fn dispatch(s: Shell, event: SyncEvent) {
    // Disposal guard (review M-10): an event can race logout — the handler
    // was already queued when `shutdown` closed the stream. One `try_`
    // read proves the shell alive for the whole synchronous body below
    // (single-threaded WASM); the spawned tails re-prove after each await.
    if s.sync.polling.try_get_untracked().is_none() {
        return;
    }
    match &event {
        // Forward-compat: an event type this build doesn't know. MUST be
        // ignored gracefully (protocol contract).
        SyncEvent::Unknown => {}
        // Metadata changed somewhere visible: refetch the lists AND the
        // unread summary (a new/removed channel shifts both). ListsChanged is
        // ALSO the server's post-lag resync nudge (the bus dropped events on
        // us), so reconcile the open channel too — a dropped message_created
        // for the open pane would otherwise stay invisible until the next
        // event happens to arrive (M1.5: completes the documented contract).
        SyncEvent::ListsChanged => resync_truth(s),
        // M1.5 account-targeted nudges: the caller's read cursor moved on
        // ANOTHER device — refresh unread so this device's glow clears.
        SyncEvent::ReadStateChanged { .. } => message::refresh_unread(s),
        // M1.5: the friends/requests list changed for this account.
        // refresh_lists refetches friends (cheap, and keeps one entry point).
        SyncEvent::FriendsChanged => message::refresh_lists(s),
        // Channel-scoped events: a change in the OPEN channel reconciles the
        // message pane; anywhere else only the batched unread summary moves.
        _ => {
            let open = s.sel.sel_channel.get_untracked().map(|c| c.id);
            if event.channel_id().is_some() && event.channel_id() == open.as_deref() {
                if matches!(event, SyncEvent::Typing { .. }) {
                    // A Typing ping can never carry a new message (review
                    // M-13): refresh ONLY the typing/ghost surface. The full
                    // reconcile below used to run here too, costing an
                    // uncursored 100-envelope length probe + a cursored
                    // fetch + drafts per ~2s ping per viewer — the same
                    // storm the M1.5 review killed for NON-open channels.
                    spawn_local(async move {
                        refresh_typing_surface(s).await;
                    });
                } else {
                    spawn_local(async move {
                        message::refresh_open_channel(s).await;
                        // The open channel is always considered seen: advance
                        // its last-seen mark to the post-refresh cursor. The
                        // poll loop got this for free from `refresh_unread`'s
                        // ~6s prelude; under SSE nothing else advances it
                        // while the user sits reading. (`set_last_seen`
                        // itself gates on document visibility — review M-04 —
                        // so a hidden tab defers the mark to the
                        // foregrounding wake() pass.) `try_` re-read: the
                        // await may resolve after a channel switch
                        // (stale-guard) or after logout disposed the shell
                        // (review M-10).
                        let Some(sel) = s.sel.sel_channel.try_get_untracked() else {
                            return;
                        };
                        if let (Some(oc), Some(cur)) =
                            (sel.map(|c| c.id), s.msg.cursor.get_untracked())
                        {
                            message::set_last_seen(s, &oc, cur);
                        }
                        // Ghost Quill (M4/T7): a MessageCreated means the
                        // sender's draft was just cleared server-side —
                        // refetch (the helper no-ops with the pref off).
                        // Draft TEXT rides this permission-checked fetch
                        // only; the event itself stays id-only.
                        message::refresh_ghost_drafts(s).await;
                        // Typing names piggyback on the page response and
                        // only change when something triggers a refresh; arm
                        // the staleness clearer whenever the pane shows
                        // typists. (`try_`: post-await, review M-10.)
                        if s.msg.typing.try_with_untracked(|t| !t.is_empty()) == Some(true) {
                            schedule_typing_clear(s);
                        }
                    });
                }
            } else if !matches!(event, SyncEvent::Typing { .. }) {
                // M1.5: typing in a NON-open channel cannot change unread
                // state — skipping the refetch kills the 2s-cadence /unread
                // storm one busy typist used to inflict on every member.
                message::refresh_unread(s);
            }
        }
    }
}

/// Typing-surface refresh for the open channel (review M-13): a Typing ping
/// can never carry a new message — only the typing-names line and the ghost
/// drafts can have changed — so fetch ONE CURSORED page (it returns the live
/// `typing` names and, caught up, ~zero rows; never the uncursored
/// 100-envelope length probe) plus the pref-gated drafts. Rows that DO ride
/// the page (a send raced the ping) are deliberately left to their own
/// MessageCreated event, which takes the full reconcile path with
/// notification handling — ingesting them here would mark them seen and
/// swallow their notification.
#[cfg(feature = "hydrate")]
async fn refresh_typing_surface(s: Shell) {
    if document_hidden() {
        // Nothing this surface paints is visible from a hidden tab, and the
        // typist's next ~2s ping repaints it after foregrounding — skip the
        // fetches entirely (same reasoning as the M-04 read-mark gate).
        return;
    }
    if s.sync.pane.try_get_untracked() != Some(Pane::Channel) {
        return;
    }
    let Some(Some(ch)) = s.sel.sel_channel.try_get_untracked() else {
        return;
    };
    let cur = s.msg.cursor.get_untracked();
    let page = api::list_messages(&ch.id, cur.as_ref()).await;
    // Stale-guard + disposal guard: drop the pass if the channel changed —
    // or the shell died (review M-10) — while the fetch was in flight.
    if s.sel
        .sel_channel
        .try_get_untracked()
        .flatten()
        .map(|c| c.id)
        != Some(ch.id.clone())
    {
        return;
    }
    if let Ok(l) = page {
        s.msg.typing.set(l.typing);
        // Typing names linger without a follow-up event once the typist
        // stops — arm the staleness clearer, same as the full-refresh path.
        if s.msg.typing.with_untracked(|t| !t.is_empty()) {
            schedule_typing_clear(s);
        }
    }
    // Ghost Quill (M4/T7): a fresh ping may carry fresh draft text — refetch
    // (the helper no-ops with the pref off; draft TEXT rides only that
    // permission-checked fetch, the event itself stays id-only).
    message::refresh_ghost_drafts(s).await;
}

#[cfg(feature = "hydrate")]
thread_local! {
    /// True while a delayed typing-staleness refresh is already scheduled —
    /// keeps [`schedule_typing_clear`] from stacking one timer per keystroke.
    static TYPING_CLEAR_PENDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Clear a lingering typing indicator: under SSE a typist's periodic pings
/// each refresh the typing surface (keeping the names fresh — review M-13),
/// but when the typist STOPS no further event fires and the last name would
/// linger indefinitely (the poll loop used to provide this cadence).
/// Schedule ONE `refresh_open_channel` just past the server's 8s typing TTL;
/// if typing resumed meanwhile, the next Typing event re-arms this after the
/// flag clears.
#[cfg(feature = "hydrate")]
fn schedule_typing_clear(s: Shell) {
    if TYPING_CLEAR_PENDING.with(|c| c.replace(true)) {
        return; // one pending clearer at a time
    }
    spawn_local(async move {
        gloo_timers::future::TimeoutFuture::new(9_000).await;
        TYPING_CLEAR_PENDING.with(|c| c.set(false));
        // `try_` throughout (review M-10): this detached 9s timer can outlive
        // the shell (logout mid-typing) — a disposed shell degrades to a
        // no-op. Only fetch if there is still something to clear; a channel
        // switch already reset `typing`, making this a no-op.
        if s.msg.typing.try_with_untracked(|t| !t.is_empty()) == Some(true) {
            message::refresh_open_channel(s).await;
        }
        // Ghost Quill rows share the typing cadence: when the typist STOPS
        // (no further pings, no further events) their server entry prunes at
        // the same ~8s TTL, but nothing would tell this client — so the same
        // clearer refetches the drafts (now empty) just past it (M4/T7).
        if s.msg.ghost_drafts.try_with_untracked(|g| !g.is_empty()) == Some(true) {
            message::refresh_ghost_drafts(s).await;
        }
    });
}

// ---- ssr stub ----
//
// `shutdown` needs no stub: unlike the view-called actions, it is only
// invoked from `account::logout`'s hydrate-real impl, so the ssr graph never
// references it.

#[cfg(not(feature = "hydrate"))]
pub fn start_sync(_s: Shell) {}
