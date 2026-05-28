//! The per-guild custom-emoji manager pane: list the open guild's emoji,
//! upload a new image + name it, and (owner only) delete one.
//!
//! Image upload mirrors the composer's attach flow (`channel.rs`): a hidden
//! `<input type="file">` whose change handler is cfg-split so `web_sys::File`
//! never enters the ssr graph. The picked image is uploaded immediately and
//! its media id staged in `pending_media`; "Add" then creates the named emoji.
//! Name validation is client-side (`^[a-z0-9_]{2,32}$`, manual scan — no regex
//! crate); the server enforces the same rule regardless.

use leptos::prelude::*;

use super::{act, Shell};
use crate::ui::AuthCtx;

/// A custom emoji name is 2..=32 chars, each lowercase ascii / digit / `_`.
/// Mirrors the server rule `^[a-z0-9_]{2,32}$` without pulling in a regex crate.
fn valid_emoji_name(name: &str) -> bool {
    let len = name.chars().count();
    (2..=32).contains(&len)
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

#[component]
pub(crate) fn EmojiManagerPane(s: Shell) -> impl IntoView {
    let new_name = RwSignal::new(String::new());
    let pending_media = RwSignal::new(None::<String>);

    // Live name validity, derived from the staged name. Empty is "not yet typed"
    // (no error shown, but Add stays disabled); a non-empty invalid name shows
    // the `.emoji-mgr-err` message and keeps Add disabled.
    let name_valid = move || valid_emoji_name(&new_name.get());
    let name_error = move || {
        let n = new_name.get();
        (!n.is_empty() && !valid_emoji_name(&n))
            .then(|| "Name must be 2–32 chars: a–z, 0–9, _".to_string())
    };

    let gid = move || s.sel.sel_server.get().unwrap_or_default();

    let auth = use_context::<AuthCtx>().expect("AuthCtx");
    let is_owner = move || {
        let me = auth.user.get().map(|u| u.account_id);
        me.is_some() && me == s.sel.sel_owner.get()
    };

    view! {
        <div class="pane">
            <h3>"Custom emoji"</h3>
            <div class="emoji-mgr-list">
                {move || s.sel.guild_emoji.get().into_iter().map(|e| {
                    let media_id = e.media_id.clone();
                    let name = e.name.clone();
                    let del_name = e.name.clone();
                    view! {
                        <div class="emoji-mgr-row">
                            <img src=format!("/media/{media_id}?w=64")
                                alt=format!(":{name}:")/>
                            <span class="emoji-mgr-name">{format!(":{name}:")}</span>
                            {is_owner().then(|| {
                                let name = del_name.clone();
                                view! {
                                    <button class="emoji-mgr-delete" title="delete"
                                        on:click=move |_| act::delete_guild_emoji(s, gid(), name.clone())>
                                        "✕"
                                    </button>
                                }
                            })}
                        </div>
                    }
                }).collect_view()}
            </div>
            <div class="emoji-mgr-add">
                <label>
                    "upload image"
                    <input type="file" accept="image/*" style="display:none"
                        on:change=move |_ev| {
                            #[cfg(feature = "hydrate")]
                            {
                                use leptos::wasm_bindgen::JsCast;
                                if let Some(input) = _ev.target().and_then(|t| {
                                    t.dyn_into::<leptos::web_sys::HtmlInputElement>().ok()
                                }) {
                                    if let Some(file) = input.files().and_then(|f| f.get(0)) {
                                        act::upload_emoji_image(s, file, pending_media);
                                    }
                                    // Clear so re-picking the same file re-fires.
                                    input.set_value("");
                                }
                            }
                            #[cfg(not(feature = "hydrate"))]
                            {
                                let _ = &_ev;
                                act::upload_emoji_image(s, pending_media);
                            }
                        }/>
                </label>
                <input prop:value=move || new_name.get()
                    on:input=move |ev| new_name.set(event_target_value(&ev))
                    placeholder="name (a-z 0-9 _)"/>
                {move || {
                    name_error().map(|e| view! { <div class="emoji-mgr-err">{e}</div> })
                }}
                <button
                    disabled=move || pending_media.get().is_none() || !name_valid()
                    on:click=move |_| {
                        let name = new_name.get_untracked();
                        if valid_emoji_name(&name) {
                            if let Some(mid) = pending_media.get_untracked() {
                                act::create_guild_emoji(s, gid(), name, mid);
                                new_name.set(String::new());
                                pending_media.set(None);
                            }
                        }
                    }>
                    "Add"
                </button>
            </div>
        </div>
    }
}
