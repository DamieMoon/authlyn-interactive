//! Toast actions (UX evolution #11): push/dismiss for the one-at-a-time
//! toast capsule in `Toasts::current`, plus the action dispatcher the view
//! calls. Hydrate-real + ssr-stub co-located like every act submodule.
//!
//! Auto-dismiss uses the generation-key pattern (`Composer::sent_gen` /
//! radial `LongPress`): each push mints a key and detaches a countdown that
//! only clears the signal while its OWN toast is still current — a replacing
//! toast's earlier timer can never truncate the newer one. The countdown is
//! PAUSABLE (review M-52, WCAG 2.2.1 Timing Adjustable): while the capsule
//! is held — pointer hover or focus within, reported by the view via
//! [`set_toast_held`] — the remaining time freezes, so a keyboard user never
//! races a fixed window to reach Undo. The timer tail uses `try_*` accessors
//! so a toast outliving the shell (logout mid-toast) degrades to a no-op,
//! never a panic.

use super::super::Shell;

use super::super::state::ToastAction;
#[cfg(feature = "hydrate")]
use super::super::state::{Toast, ToastTone, Toasts};
#[cfg(feature = "hydrate")]
use crate::protocol::MessageEnvelope;
#[cfg(feature = "hydrate")]
use leptos::prelude::*;
#[cfg(feature = "hydrate")]
use leptos::task::spawn_local;

/// Lifetime of an error toast (no action slot to linger for).
#[cfg(feature = "hydrate")]
const ERROR_TOAST_MS: u32 = 6000;

/// Lifetime of a success toast ("Copied", "invited X") — shorter than the
/// error/undo ones: pure confirmation, nothing to act on or study.
#[cfg(feature = "hydrate")]
const SUCCESS_TOAST_MS: u32 = 3200;

/// Granularity of the pausable auto-dismiss countdown (review M-52). Coarse
/// enough to cost nothing (≤ 24 wakeups for the longest toast), fine enough
/// that a hold/release lands within one perceptual beat.
#[cfg(feature = "hydrate")]
const TOAST_TICK_MS: u32 = 250;

#[cfg(feature = "hydrate")]
thread_local! {
    /// Monotonic counter minting per-toast generation keys.
    static TOAST_KEY: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// Key of the toast currently HELD by pointer hover or keyboard focus
    /// (review M-52): its countdown does not run down while set. Keyed so a
    /// stale handler from a replaced capsule can neither hold nor release
    /// its successor.
    static HELD_KEY: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

/// Show `toast`-shaped content, replacing any current toast, and detach its
/// keyed auto-dismiss countdown. The single funnel every public helper uses.
#[cfg(feature = "hydrate")]
fn push(s: Shell, text: String, tone: ToastTone, action: Option<ToastAction>, duration_ms: u32) {
    let key = TOAST_KEY.with(|c| {
        let k = c.get().wrapping_add(1);
        c.set(k);
        k
    });
    s.toasts.current.set(Some(Toast {
        key,
        text,
        tone,
        action,
        duration_ms,
    }));
    // Pausable keyed countdown (review M-52): tick down in TOAST_TICK_MS
    // steps and freeze the remaining time while the capsule is held. The
    // generation key keeps the old guarantee — a replaced toast's loop
    // self-terminates on its first tick, never dismissing its successor.
    spawn_local(async move {
        let mut remaining = duration_ms;
        loop {
            gloo_timers::future::TimeoutFuture::new(TOAST_TICK_MS.min(remaining)).await;
            let showing = s
                .toasts
                .current
                .try_get_untracked()
                .flatten()
                .is_some_and(|t| t.key == key);
            if !showing {
                return; // replaced or already dismissed: this loop is done
            }
            if HELD_KEY.with(|h| h.get()) == Some(key) {
                continue; // paused: held time never counts against the window
            }
            remaining = remaining.saturating_sub(TOAST_TICK_MS);
            if remaining == 0 {
                dismiss_if_current(s.toasts, key);
                return;
            }
        }
    });
}

/// Clear the toast iff the one with `key` is still showing (keyed dismiss —
/// a replacing toast must not be cleared by its predecessor's timer). Also
/// drops a matching hold latch: the capsule unmounts without firing its
/// blur/leave handlers, so the latch must not linger (review M-52).
#[cfg(feature = "hydrate")]
fn dismiss_if_current(toasts: Toasts, key: u64) {
    let showing = toasts
        .current
        .try_get_untracked()
        .flatten()
        .is_some_and(|t| t.key == key);
    if showing {
        let _ = toasts.current.try_set(None);
        HELD_KEY.with(|h| {
            if h.get() == Some(key) {
                h.set(None);
            }
        });
    }
}

/// The view reports the capsule's hold state — pointer hover or focus
/// within (review M-52): while the CURRENT toast is held its auto-dismiss
/// countdown freezes, so keyboard Undo is never a blind race against a
/// fixed window. Keyed like the dismiss path; a stale capsule's handler
/// can't touch its successor's latch.
#[cfg(feature = "hydrate")]
pub fn set_toast_held(key: u64, held: bool) {
    HELD_KEY.with(|h| {
        if held {
            h.set(Some(key));
        } else if h.get() == Some(key) {
            h.set(None);
        }
    });
}

/// The undo toast after a COMMITTED soft-delete: "Message deleted" + an Undo
/// action wired to the existing POST `.../restore`
/// (`act::message::undo_message_delete`). The delete already happened, so
/// the toast expiring (or being replaced by a newer toast) loses nothing —
/// the trash pane can still restore until the 1h purge. `envelope` is the
/// hidden row's snapshot for the in-place reinsert on Undo.
#[cfg(feature = "hydrate")]
pub(super) fn show_undo_delete_toast(
    s: Shell,
    cid: String,
    mid: String,
    envelope: MessageEnvelope,
    duration_ms: u32,
) {
    push(
        s,
        "Message deleted".to_string(),
        ToastTone::Info,
        Some(ToastAction::UndoMessageDelete {
            cid,
            mid,
            envelope: Box::new(envelope),
        }),
        duration_ms,
    );
}

/// An error toast (danger styling, no action). Honest-state surface for a
/// failed delete or restore.
#[cfg(feature = "hydrate")]
pub(super) fn show_error_toast(s: Shell, text: String) {
    push(s, text, ToastTone::Danger, None, ERROR_TOAST_MS);
}

/// A success toast (UX evolution #11, second clause): absorbs the status
/// line's success traffic ("Copied", "invited X") in success styling, so the
/// red status `<p>` carries errors only.
#[cfg(feature = "hydrate")]
pub(super) fn show_success_toast(s: Shell, text: String) {
    push(s, text, ToastTone::Success, None, SUCCESS_TOAST_MS);
}

/// Dispatch a toast's action slot (the capsule button), then dismiss the
/// toast it rode on. The view's only toast entry point.
#[cfg(feature = "hydrate")]
pub fn run_toast_action(s: Shell, action: ToastAction, toast_key: u64) {
    match action {
        ToastAction::UndoMessageDelete { cid, mid, envelope } => {
            super::message::undo_message_delete(s, cid, mid, *envelope)
        }
    }
    dismiss_if_current(s.toasts, toast_key);
}

// ---- ssr stubs ----

#[cfg(not(feature = "hydrate"))]
pub fn run_toast_action(_s: Shell, _action: ToastAction, _toast_key: u64) {}
#[cfg(not(feature = "hydrate"))]
pub fn set_toast_held(_key: u64, _held: bool) {}
