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
/// - `info` — opens the persona-info popup when the author name is clicked
///
/// Clicking ✎ loads the message into the main composer for editing (see
/// `act::start_edit`); there is no inline edit widget.
pub(super) fn message_meta(
    s: Shell,
    m: &MessageEnvelope,
    cid: &Option<String>,
    mine: bool,
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
    // Affordances from the shared kind predicate (`message_actions` in
    // mod.rs) — the SAME one the touch radial uses, so the two surfaces can
    // never drift (immutable kinds like T6's `kind='roll'` are edit/delete-free
    // on both).
    let actions = super::message_actions(&m.kind, mine);

    view! {
        <div class="meta">
            {avatar_el}
            <button class=who_class title="persona info"
                on:click=move |_| info.set(Some(info_m.clone()))>{who}</button>
            <time class="when">{when}</time>
            // Action row — reply/copy are available on every user message
            // (own AND others): copy so the markup source can be re-pasted
            // under a different persona, reply stashing the parent in the
            // composer banner (L-3); edit + delete remain own-message only.
            <span class="msg-actions">
                {actions.reply.then(|| {
                    // The whole envelope, captured for the reply affordance —
                    // `start_reply` builds the banner preview from it (L-3).
                    let reply_m = m.clone();
                    view! {
                        <button class="row-edit" title="reply"
                            on:click=move |_| act::start_reply(s, reply_m.clone())>"↩"</button>
                    }
                })}
                {actions.copy.then(|| {
                    let copy_body = m.body.clone();
                    view! {
                        <button class="row-edit" title="copy markup (no color)"
                            on:click=move |_| {
                                act::copy_message_body(s, copy_body.clone())
                            }>"📋"</button>
                    }
                })}
                {actions.edit.then(|| {
                    let edit_mid = m.id.clone();
                    let edit_cid = cid.clone();
                    let edit_body = m.body.clone();
                    view! {
                        <button class="row-edit" title="edit"
                            on:click=move |_| {
                                if let Some(cid) = edit_cid.clone() {
                                    act::start_edit(
                                        s, cid, edit_mid.clone(), edit_body.clone(),
                                    );
                                }
                            }>"✎"</button>
                    }
                })}
                {actions.delete.then(|| {
                    let del_mid = m.id.clone();
                    let del_cid = cid.clone();
                    view! {
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
                    }
                })}
            </span>
        </div>
    }
}

/// Meta row for a `kind='system'` ("Nova DOT") message: avatar + name + a SYSTEM
/// badge + time, with NO action row and NO persona-info popup — system messages
/// are immutable, author-less in the persona sense, and not repliable/editable.
/// Kept separate from [`message_meta`] for the same reason as `deleted_message_row`:
/// folding the branches would tangle two unrelated shapes.
pub(super) fn system_message_meta(m: &MessageEnvelope) -> impl IntoView {
    // No persona on a system message, so `display_name` falls back to the bot's
    // account display name ("Nova DOT"); the avatar falls back to its monogram.
    let who = display_name(m);
    let when = format_local_time(&m.sent_at);
    let avatar_el = chat_avatar(&m.persona_avatar_id, &who, false);

    view! {
        <div class="meta">
            {avatar_el}
            <span class="who system-author">{who}</span>
            <span class="system-badge" title="System message">"SYSTEM"</span>
            <time class="when">{when}</time>
        </div>
    }
}
