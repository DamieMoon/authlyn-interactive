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
///
/// Review M-52 (WCAG 2.2.1 Timing Adjustable): the auto-dismiss countdown
/// pauses while the capsule is HELD — pointer hover on the capsule, or
/// focus on the action button — reported to `act::toast::set_toast_held`
/// via two component-local latches; the drain bar pauses with it so the
/// visual lifetime never lies. And because a keyboard-driven delete
/// unmounts the focused row (dropping focus to `<body>` and turning the
/// timed Undo into a blind race), an ACTION toast adopts the orphaned focus
/// onto its button on mount — Enter undoes immediately, and the focus-hold
/// pauses the countdown. Actionless toasts never steal focus, and neither
/// does an action toast when focus survived elsewhere (e.g. mid-typing in
/// the composer).
pub(super) fn toast_host(s: Shell) -> impl IntoView {
    view! {
        <div class="toast-host" role="status" aria-live="polite">
            {move || s.toasts.current.get().map(|t| {
                let key = t.key;
                // Hold latches (M-52): hover and focus toggle independently;
                // the countdown pauses while EITHER is set.
                let hovered = RwSignal::new(false);
                let focused = RwSignal::new(false);
                let update_held = move || {
                    act::toast::set_toast_held(
                        key,
                        hovered.get_untracked() || focused.get_untracked(),
                    );
                };
                let action_btn = t.action.map(|a| {
                    // Labels live with the variants — the action is data
                    // (state.rs), the view names it.
                    let label = match a {
                        ToastAction::UndoMessageDelete { .. } => "Undo",
                    };
                    let btn_ref = NodeRef::<leptos::html::Button>::new();
                    // Focus adoption (M-52): only when the prior focus was
                    // destroyed with the deleted row (active element fell to
                    // <body>). Hydrate-only like every DOM-reading effect.
                    #[cfg(feature = "hydrate")]
                    Effect::new(move |_| {
                        let Some(btn) = btn_ref.get() else {
                            return;
                        };
                        let orphaned = leptos::web_sys::window()
                            .and_then(|w| w.document())
                            .and_then(|d| d.active_element())
                            .map(|el| el.tag_name() == "BODY")
                            .unwrap_or(true);
                        if orphaned {
                            let _ = btn.focus();
                        }
                    });
                    view! {
                        <button class="toast-action" type="button" node_ref=btn_ref
                            on:focus=move |_| { focused.set(true); update_held(); }
                            on:blur=move |_| { focused.set(false); update_held(); }
                            on:click=move |_| act::run_toast_action(s, a.clone(), key)>
                            {label}
                        </button>
                    }
                });
                view! {
                    <div class="toast"
                        class:error=matches!(t.tone, ToastTone::Danger)
                        class:success=matches!(t.tone, ToastTone::Success)
                        style=("--toast-ms", format!("{}ms", t.duration_ms))
                        on:pointerenter=move |_| { hovered.set(true); update_held(); }
                        on:pointerleave=move |_| { hovered.set(false); update_held(); }>
                        <span class="toast-text">{t.text}</span>
                        {action_btn}
                        // Draining lifetime bar — decorative restatement of
                        // the auto-dismiss countdown (hidden under reduced
                        // motion; the countdown itself is JS and always
                        // runs). Pauses in lockstep with the held countdown
                        // (M-52) so it never claims time the timer isn't
                        // spending.
                        <div class="toast-drain" aria-hidden="true"
                            style:animation-play-state=move || {
                                if hovered.get() || focused.get() { "paused" } else { "running" }
                            }></div>
                    </div>
                }
            })}
        </div>
    }
}
