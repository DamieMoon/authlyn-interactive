//! Per-message meta row — avatar + author name + send time + own-message
//! edit / delete actions. Used by the live message list in `mod.rs`.
//!
//! The deleted-message row in `mod.rs::deleted_message_row` has a different
//! shape (no actions, different selectors) and is kept separate by design:
//! folding the two would force a 3-branch conditional that loses more than
//! the dedup saves.

use leptos::prelude::*;

use super::super::{act, PendingDelete, Shell};
use super::avatar::{chat_avatar, format_local_time};
use super::display_name;
use crate::markup::Color;
use crate::protocol::MessageEnvelope;

/// Render the `<div class="meta">` block for a single message row.
///
/// - `s` — Shell handle for ask_delete / delete_message dispatch
/// - `m` — message envelope (cloned where needed for popups)
/// - `cid` — current channel id (None during pane-switch latency)
/// - `mine` — true when the viewer authored the message (gates the actions)
/// - `editing_msg` — which msg id is being edited; click ✎ stores `mid` here
/// - `info` — opens the persona-info popup when the author name is clicked
pub(super) fn message_meta(
    s: Shell,
    m: &MessageEnvelope,
    cid: &Option<String>,
    mine: bool,
    editing_msg: RwSignal<Option<String>>,
    info: RwSignal<Option<MessageEnvelope>>,
) -> impl IntoView {
    let who = display_name(m);
    // Tint the name with the persona's chosen palette colour (validated
    // against the markup palette before trusting it).
    let who_class = m
        .persona_color
        .as_deref()
        .filter(|c| Color::from_name(c).is_some())
        .map(|c| format!("who mk-{c}"))
        .unwrap_or_else(|| "who".to_string());
    let when = format_local_time(&m.sent_at);
    let avatar_el = chat_avatar(&m.persona_avatar_id, &who, false);

    let info_m = m.clone();
    let mid = m.id.clone();
    let cid = cid.clone();

    view! {
        <div class="meta">
            {avatar_el}
            <button class=who_class title="persona info"
                on:click=move |_| info.set(Some(info_m.clone()))>{who}</button>
            <time class="when">{when}</time>
            {mine.then(|| {
                let edit_mid = mid.clone();
                let del_mid = mid.clone();
                let del_cid = cid.clone();
                view! {
                    <span class="msg-actions">
                        <button class="row-edit" title="edit"
                            on:click=move |_| editing_msg.set(Some(edit_mid.clone()))>"✎"</button>
                        <button class="row-edit" title="delete"
                            on:click=move |_| {
                                if let Some(cid) = del_cid.clone() {
                                    // Message deletes confirm unless the user
                                    // opted out in account settings; other
                                    // deletes always confirm.
                                    if act::confirm_delete_message_enabled() {
                                        act::ask_delete(
                                            s,
                                            "Delete this message? This cannot be undone."
                                                .to_string(),
                                            PendingDelete::Message {
                                                cid,
                                                mid: del_mid.clone(),
                                            },
                                        );
                                    } else {
                                        act::delete_message(s, cid, del_mid.clone());
                                    }
                                }
                            }>"🗑"</button>
                    </span>
                }
            })}
        </div>
    }
}
