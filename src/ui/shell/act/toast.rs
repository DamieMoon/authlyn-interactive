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

#[cfg(feature = "hydrate")]
thread_local! {
    /// Monotonic counter minting per-toast generation keys.
    static TOAST_KEY: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Show `toast`-shaped content, replacing any current toast, and detach its
/// keyed auto-dismiss timer. The single funnel every public helper uses.
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
