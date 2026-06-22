//! Per-message meta row — avatar + author name + send time + own-message
//! edit / delete actions. Used by the live message list in `mod.rs`.
//!
//! The deleted-message row in `mod.rs::deleted_message_row` has a different
//! shape (no actions, different selectors) and is kept separate by design:
//! folding the two would force a 3-branch conditional that loses more than
//! the dedup saves.

use leptos::prelude::*;

use super::super::{act, Shell};
use super::avatar::{chat_avatar, format_clock_time};
use super::display_name;
use crate::markup::Color;
use crate::protocol::MessageEnvelope;
use crate::ui::icons::{IconCopy, IconEdit, IconReply, IconTrash, NovaOrb};

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
    // Orbit (the sole release shell) shows the prototype's terse HH:MM clock.
    // The deck/hud verbose date+time variant (`format_local_time`) lives behind
    // the test-pinned skeleton API in `act/prefs.rs`; restore the pref branch
    // here if a deck/hud skeleton is re-enabled.
    let when = format_clock_time(&m.sent_at);
    // Avatar: a worn persona keeps its send-time SNAPSHOT (frozen); a bare
    // account shows its LIVE avatar (author_avatar_id, resolved at read). M6/P2.
    let avatar_id = if m.persona_name.is_some() {
        &m.persona_avatar_id
    } else {
        &m.author_avatar_id
    };
    let avatar_el = chat_avatar(avatar_id, &who, false);
    // When a persona dominates, surface the controlling account subtly ("· name")
    // so the speaker stays identifiable without stealing from the persona.
    let account_marker = m.persona_name.is_some().then(|| m.author_display.clone());

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
            {account_marker.map(|name| view! {
                <span class="who-account">{format!(" · {name}")}</span>
            })}
            // M7/P2: a guest cameo's messages carry a "GUEST" chip (snapshotted at
            // send time, so it persists after the cameo is revoked/expired).
            {m.guest_cameo.then(|| view! {
                <span class="badge-guest" title="guest cameo">"GUEST"</span>
            })}
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
                            on:click=move |_| act::start_reply(s, reply_m.clone())><IconReply/></button>
                    }
                })}
                {actions.copy.then(|| {
                    let copy_body = m.body.clone();
                    view! {
                        <button class="row-edit" title="copy markup (no color)"
                            on:click=move |_| {
                                act::copy_message_body(s, copy_body.clone())
                            }><IconCopy/></button>
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
                            }><IconEdit/></button>
                    }
                })}
                {actions.delete.then(|| {
                    let del_mid = m.id.clone();
                    let del_cid = cid.clone();
                    view! {
                        <button class="row-edit" title="delete (undo for 6s)"
                            on:click=move |_| {
                                if let Some(cid) = del_cid.clone() {
                                    // Instant + undoable (UX evolution #11):
                                    // the soft-DELETE fires at once, and the
                                    // undo toast offers POST .../restore for
                                    // 6s — no confirm modal in the flow.
                                    act::delete_message(s, cid, del_mid.clone());
                                }
                            }><IconTrash/></button>
                    }
                })}
            </span>
        </div>
    }
}

/// Meta row for a `kind='system'` ("Nova DOT") message: the Nova orb + name + a
/// SYSTEM badge chip + time, with NO action row and NO persona-info popup —
/// system messages are immutable, author-less in the persona sense, and not
/// repliable/editable. Kept separate from [`message_meta`] for the same reason
/// as `deleted_message_row`: folding the branches would tangle two unrelated
/// shapes.
///
/// The avatar is the [`NovaOrb`] brand asset, NOT `chat_avatar`: it carries the
/// `.nova-orb` class, NOT `.chat-avatar`, on purpose — `_sk_orbit_chat.scss`
/// hides `.chat-avatar` under `.app.sk-orbit` (the sole release shell), so an
/// orb on the avatar class would vanish on the only shipping surface. The orb
/// is the deliberate exception to orbit's name-only chat (M6/P3).
pub(super) fn system_message_meta(s: Shell, m: &MessageEnvelope) -> impl IntoView {
    // No persona on a system message, so `display_name` falls back to the bot's
    // account display name ("Nova DOT").
    let who = display_name(m);
    // Deck-finputs (2026-06-20): orbit (the sole release shell) shows the terse
    // HH:MM clock — parity with every other bubble (the verbose date+time
    // overran the meta on a 13-mini and wrapped the time below the badges). The
    // deck/hud verbose variant (`format_local_time`) lives behind the test-pinned
    // skeleton API in `act/prefs.rs`; restore the pref branch if it's re-enabled.
    // `s` is kept on the signature for that re-enable (the pref read needs it).
    let _ = &s;
    let when = format_clock_time(&m.sent_at);

    view! {
        <div class="meta">
            <NovaOrb/>
            <span class="who system-author">{who}</span>
            // Deck-finputs (2026-06-20): the time sits right after the name
            // (inline with the identity), and the two role badges are grouped so
            // they wrap together as a tidy row BELOW on narrow widths — instead
            // of the name + both badges filling the line and orphaning the time
            // under them (the bubble content area on a 13-mini is too narrow for
            // name + SYSTEM + COMMENTATOR + time on one line).
            <time class="when">{when}</time>
            <span class="system-badges">
                <span class="system-badge" title="System message">"SYSTEM"</span>
                // B6 (owner deck-finding 2026-06-20): Nova DOT is the prose-rich
                // COMMENTATOR (design spec §7 / bot gateway) — messages it posts
                // through the system pipeline carry a clear COMMENTATOR badge.
                <span class="system-badge commentator" title="Nova DOT — commentator">
                    "COMMENTATOR"
                </span>
            </span>
        </div>
    }
}
