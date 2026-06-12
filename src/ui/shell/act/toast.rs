//! Toast actions (UX evolution #11): push/dismiss for the one-at-a-time
//! toast capsule in `Toasts::current`, plus the action dispatcher the view
//! calls. Hydrate-real + ssr-stub co-located like every act submodule.
//!
//! Auto-dismiss uses the generation-key pattern (`Composer::sent_gen` /
//! radial `LongPress`): each push mints a key and detaches a timer that only
//! clears the signal while its OWN toast is still current — a replacing
//! toast's earlier timer can never truncate the newer one. The timer tail
//! uses `try_*` accessors so a toast outliving the shell (logout mid-toast)
//! degrades to a no-op, never a panic.

use super::super::Shell;

use super::super::state::ToastAction;
#[cfg(feature = "hydrate")]
use super::super::state::{Toast, Toasts};
#[cfg(feature = "hydrate")]
use leptos::prelude::*;
#[cfg(feature = "hydrate")]
use leptos::task::spawn_local;

/// Lifetime of an error toast (no action slot to linger for).
#[cfg(feature = "hydrate")]
const ERROR_TOAST_MS: u32 = 6000;

#[cfg(feature = "hydrate")]
thread_local! {
    /// Monotonic counter minting per-toast generation keys.
    static TOAST_KEY: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Show `toast`-shaped content, replacing any current toast, and detach its
/// keyed auto-dismiss timer. The single funnel every public helper uses.
#[cfg(feature = "hydrate")]
fn push(s: Shell, text: String, error: bool, action: Option<ToastAction>, duration_ms: u32) {
    let key = TOAST_KEY.with(|c| {
        let k = c.get().wrapping_add(1);
        c.set(k);
        k
    });
    s.toasts.current.set(Some(Toast {
        key,
        text,
        error,
        action,
        duration_ms,
    }));
    spawn_local(async move {
        gloo_timers::future::TimeoutFuture::new(duration_ms).await;
        dismiss_if_current(s.toasts, key);
    });
}

/// Clear the toast iff the one with `key` is still showing (keyed dismiss —
/// a replacing toast must not be cleared by its predecessor's timer).
#[cfg(feature = "hydrate")]
fn dismiss_if_current(toasts: Toasts, key: u64) {
    let showing = toasts
        .current
        .try_get_untracked()
        .flatten()
        .is_some_and(|t| t.key == key);
    if showing {
        let _ = toasts.current.try_set(None);
    }
}

/// The undo toast for an optimistically-hidden message delete: "Message
/// deleted" + an Undo action wired to the pending entry keyed `pending` in
/// `super::message`. `duration_ms` is the SAME grace window driving the
/// delayed DELETE, so the drain bar tells the truth.
#[cfg(feature = "hydrate")]
pub(super) fn show_undo_delete_toast(s: Shell, pending: u64, duration_ms: u32) {
    push(
        s,
        "Message deleted".to_string(),
        false,
        Some(ToastAction::UndoMessageDelete { pending }),
        duration_ms,
    );
}

/// An error toast (danger styling, no action). Honest-state surface for the
/// delayed delete's failure path.
#[cfg(feature = "hydrate")]
pub(super) fn show_error_toast(s: Shell, text: String) {
    push(s, text, true, None, ERROR_TOAST_MS);
}

/// Dismiss the undo toast bound to pending-delete `pending`, if it is the
/// current toast — called when the pending delete is flushed early so a dead
/// Undo button never lingers.
#[cfg(feature = "hydrate")]
pub(super) fn dismiss_undo_toast(s: Shell, pending: u64) {
    let matches = s.toasts.current.try_get_untracked().flatten().is_some_and(
        |t| matches!(t.action, Some(ToastAction::UndoMessageDelete { pending: p }) if p == pending),
    );
    if matches {
        let _ = s.toasts.current.try_set(None);
    }
}

/// Dispatch a toast's action slot (the capsule button), then dismiss the
/// toast it rode on. The view's only toast entry point.
#[cfg(feature = "hydrate")]
pub fn run_toast_action(s: Shell, action: ToastAction, toast_key: u64) {
    match action {
        ToastAction::UndoMessageDelete { pending } => {
            super::message::undo_pending_delete(s, pending)
        }
    }
    dismiss_if_current(s.toasts, toast_key);
}

// ---- ssr stubs ----

#[cfg(not(feature = "hydrate"))]
pub fn run_toast_action(_s: Shell, _action: ToastAction, _toast_key: u64) {}
