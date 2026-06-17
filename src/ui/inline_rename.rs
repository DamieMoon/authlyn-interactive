//! Shared inline-rename / inline-edit input + save/cancel pair.
//!
//! Three sites consume this today (W6/C7):
//!   - server name rename in `shell/mod.rs` (sidebar header)
//!   - channel name rename in `shell/mod.rs` (`ChannelRow`)
//!   - message body edit in `shell/channel/mod.rs` (the message-row inline
//!     edit textarea — `multiline=true`)
//!
//! Parent owns the "is editing" signal and toggles it on Edit (✎) and
//! after `on_save` / `on_cancel`. This component owns the local draft
//! buffer + the input ref so caret/focus state stays scoped.
//!
//! Lorebook entry edit stays hand-rolled — it's three fields (title, keys,
//! content) and uses Save/Cancel buttons only (no keydown contract), which
//! would dilute this component's narrow shape.

use leptos::callback::Callback;
use leptos::ev::KeyboardEvent;
use leptos::html::{Input, Textarea};
use leptos::prelude::*;

use crate::ui::icons::{IconCheck, IconClose};

/// `value` — initial buffer contents (the existing name / body).
///
/// `on_save` — called once with the final buffer when the user hits Enter or
/// clicks the ✓ button. Parent flips its editing flag back to false.
///
/// `on_cancel` — called on Esc or the ✕ button. Parent flips editing off.
///
/// Both callbacks are `Callback<T>` (Leptos `StoredValue` under the hood) so
/// they are `Copy` and can fan out to the keydown handler + the button click
/// without cloning. Pass plain closures at the call site — `#[prop(into)]`
/// upcasts them.
///
/// `class` — applied to the rendered input element. Defaults to
/// `"rename-input"` so existing CSS keeps working without per-site overrides.
///
/// `multiline` — `true` renders a `<textarea>` (used by the message body
/// edit); `false` renders an `<input type="text">` (the rename rows).
#[component]
pub fn InlineRename(
    value: String,
    #[prop(into)] on_save: Callback<String>,
    #[prop(into)] on_cancel: Callback<()>,
    #[prop(into, optional, default = "rename-input".to_string())] class: String,
    #[prop(optional)] multiline: bool,
    /// `true` makes Enter submit (Shift+Enter inserts a newline) in multiline
    /// mode — used by the message-body edit. Single-line Enter already submits.
    #[prop(optional)]
    submit_on_enter: bool,
) -> impl IntoView {
    let buf = RwSignal::new(value);
    // Mount Effect focuses + selects the input so the user can either type
    // a fresh name (auto-replaces selection) or edit the existing one.
    let input_ref = NodeRef::<Input>::new();
    let textarea_ref = NodeRef::<Textarea>::new();
    #[cfg(feature = "hydrate")]
    {
        Effect::new(move |_| {
            if multiline {
                if let Some(el) = textarea_ref.get() {
                    let _ = el.focus();
                    el.select();
                }
            } else if let Some(el) = input_ref.get() {
                let _ = el.focus();
                el.select();
            }
        });
    }

    // Shared keydown — Enter (single-line) saves, Esc cancels. In multiline
    // mode Enter inserts a newline (standard textarea behaviour); the user
    // saves via the ✓ button. This matches the audit's preservation rule
    // for the touch / coarse-pointer composer (Send is the sole send path).
    let on_keydown = move |ev: KeyboardEvent| {
        #[cfg(feature = "hydrate")]
        match ev.key().as_str() {
            "Enter" if !multiline => {
                ev.prevent_default();
                on_save.run(buf.get_untracked());
            }
            // Message edit (submit_on_enter): Enter saves, Shift+Enter inserts a
            // newline (feedback 7857wrqb…). Other multiline uses keep the
            // standard newline-on-Enter (save via the ✓ button).
            "Enter" if multiline && submit_on_enter && !ev.shift_key() => {
                ev.prevent_default();
                on_save.run(buf.get_untracked());
            }
            "Escape" => on_cancel.run(()),
            _ => {}
        }
        #[cfg(not(feature = "hydrate"))]
        let _ = (&ev, submit_on_enter);
    };

    let input_field = if multiline {
        view! {
            <textarea node_ref=textarea_ref class=class
                prop:value=move || buf.get()
                on:input=move |ev| buf.set(event_target_value(&ev))
                on:keydown=on_keydown></textarea>
        }
        .into_any()
    } else {
        view! {
            <input node_ref=input_ref class=class
                prop:value=move || buf.get()
                on:input=move |ev| buf.set(event_target_value(&ev))
                on:keydown=on_keydown/>
        }
        .into_any()
    };

    view! {
        {input_field}
        <button class="row-edit" title="save"
            on:click=move |_| on_save.run(buf.get_untracked())><IconCheck/></button>
        <button class="row-edit" title="cancel"
            on:click=move |_| on_cancel.run(())><IconClose/></button>
    }
}
