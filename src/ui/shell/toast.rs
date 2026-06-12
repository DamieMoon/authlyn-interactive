//! The toast layer (UX evolution #11): one transient glass capsule at a time,
//! fixed bottom-center above the composer/tab bar (`style/_toast.scss` owns
//! the safe-area math). Always-on module — the view compiles in both graphs;
//! the action button dispatches through `act::run_toast_action`, which
//! carries an ssr no-op stub like every other act fn.
//!
//! The host `<div role="status" aria-live="polite">` is permanently mounted
//! (empty between toasts) so screen readers announce each toast's text as a
//! content change; `pointer-events` are disabled on the host and re-enabled
//! on the capsule so the empty layer never eats taps.

use leptos::prelude::*;

use super::state::{ToastAction, ToastTone};
use super::{act, Shell};

/// Render the toast layer. The capsule is recreated whenever
/// `s.toasts.current` changes, so the CSS drain-bar animation restarts per
/// toast; `--toast-ms` feeds the toast's lifetime into the drain duration.
pub(super) fn toast_host(s: Shell) -> impl IntoView {
    view! {
        <div class="toast-host" role="status" aria-live="polite">
            {move || s.toasts.current.get().map(|t| {
                let key = t.key;
                let action_btn = t.action.map(|a| {
                    // Labels live with the variants — the action is data
                    // (state.rs), the view names it.
                    let label = match a {
                        ToastAction::UndoMessageDelete { .. } => "Undo",
                    };
                    view! {
                        <button class="toast-action" type="button"
                            on:click=move |_| act::run_toast_action(s, a.clone(), key)>
                            {label}
                        </button>
                    }
                });
                view! {
                    <div class="toast"
                        class:error=matches!(t.tone, ToastTone::Danger)
                        class:success=matches!(t.tone, ToastTone::Success)
                        style=("--toast-ms", format!("{}ms", t.duration_ms))>
                        <span class="toast-text">{t.text}</span>
                        {action_btn}
                        // Draining lifetime bar — decorative restatement of
                        // the auto-dismiss timer (hidden under reduced
                        // motion; the timer itself is JS and always runs).
                        <div class="toast-drain" aria-hidden="true"></div>
                    </div>
                }
            })}
        </div>
    }
}
