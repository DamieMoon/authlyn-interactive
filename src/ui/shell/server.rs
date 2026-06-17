//! The server-management modal (owner-gated).
//!
//! Mirrors [`AccountModal`](super::account::AccountModal): one shared,
//! skeleton-independent window with stacked `<section>`s. Where the account
//! modal is the user-scoped settings home (password, prefs, feedback, logout),
//! this is the guild-owner home — server accent, invitations, and channel
//! management in one place, reached by the orbit station's "⚙ Server settings"
//! button (and, until the W3 shell retires at P6, the W3 sidebar's scattered
//! gear controls cover the same surfaces).
//!
//! Every control drives an existing owner-gated server route; the server
//! re-validates `require_manager` (which also rejects a soft-deleted guild) on
//! each call, so this view never trusts its own gating — the caller's
//! `is_owner` render-gate is a UX affordance, not the security boundary.

use leptos::prelude::*;

use super::channel::ChannelManagerBody;
use super::{act, Shell};
use crate::ui::icons::IconClose;
use crate::ui::modal::Modal;

/// The server-management window. Renders the shared `.modal`/`.modal-backdrop`
/// over the shell; `open` is the caller's visibility signal (the ✕, the
/// backdrop, and — under Omloppsbana — the swipe-right gesture all flip it to
/// `false`). `swipe_close` opts this dialog into the orbit full-screen
/// slide-over treatment exactly as `AccountModal` does (a no-op outside
/// `.app.sk-orbit`).
#[component]
pub(crate) fn ServerModal(s: Shell, open: RwSignal<bool>) -> impl IntoView {
    // ---- invitations section: local form state ----
    let new_invite = RwSignal::new(String::new());

    // The open guild's accent name (empty = default), derived live from the
    // rail list so a set-accent patch re-marks the active swatch without a
    // refetch (mirrors `shell/mod.rs::accent_name`).
    let accent_name = move || {
        let sid = s.sel.sel_server.get();
        s.sel
            .guilds
            .get()
            .into_iter()
            .find(|g| Some(&g.id) == sid.as_ref())
            .map(|g| g.accent_color)
            .unwrap_or_default()
    };

    view! {
        <Modal class="server-modal" swipe_close=true close=move || open.set(false)>
            <header class="account-head">
                <h2>"Server"</h2>
                <button class="row-edit" title="Close"
                    on:click=move |_| open.set(false)><IconClose/></button>
            </header>

            // ---- Server accent ----
            // The 8 palette swatches + a Default clear; each calls
            // act::set_guild_accent on the open guild (same block the W3
            // accent-modal renders, shell/mod.rs). The picker stays open so the
            // owner can preview a few accents in a row; the active swatch
            // tracks `accent_name` live.
            <section class="account-section">
                <h3>"Server accent"</h3>
                <div class="accent-swatches">
                    <button class="accent-swatch accent-default"
                        class:active=move || accent_name().is_empty()
                        title="Default (electric blue)"
                        on:click=move |_| {
                            if let Some(gid) = s.sel.sel_server.get_untracked() {
                                act::set_guild_accent(s, gid, String::new());
                            }
                        }>"Default"</button>
                    {move || {
                        let names = [
                            "red", "orange", "yellow", "green", "blue", "purple", "pink", "gray",
                        ];
                        let cur = accent_name();
                        names
                            .into_iter()
                            .map(|n| {
                                let n_owned = n.to_string();
                                let is_cur = cur == n;
                                view! {
                                    <button
                                        class="accent-swatch"
                                        class:active=is_cur
                                        style:background=format!("var(--tint-{n})")
                                        title=n
                                        on:click=move |_| {
                                            if let Some(gid) = s.sel.sel_server.get_untracked() {
                                                act::set_guild_accent(s, gid, n_owned.clone());
                                            }
                                        }>
                                    </button>
                                }
                            })
                            .collect_view()
                    }}
                </div>
            </section>

            // ---- Invitations ----
            // Invite a member by username (owner-gated route; the server
            // re-checks require_manager). Reuses act::invite_member — the same
            // helper the W3 sidebar invite-row drives.
            <section class="account-section">
                <h3>"Invite a member"</h3>
                <div class="invite-row">
                    <input prop:value=move || new_invite.get()
                        on:input=move |ev| new_invite.set(event_target_value(&ev))
                        placeholder="invite by username"/>
                    <button class="account-save" on:click=move |_| {
                        let gid = s.sel.sel_server.get_untracked();
                        let u = new_invite.get_untracked();
                        new_invite.set(String::new());
                        if let Some(gid) = gid {
                            act::invite_member(s, gid, u);
                        }
                    }>"Invite"</button>
                </div>
            </section>

            // ---- Channels ----
            // The full channel manager (create / rename / delete / finger-drag
            // reorder) inlined as a section — the SAME ChannelManagerBody the W3
            // sidebar's "⚙ Manage" modal wraps. The `.channel-manager` div is the
            // ancestor its scoped styling needs.
            <section class="account-section">
                <h3>"Channels"</h3>
                <div class="channel-manager">
                    <ChannelManagerBody s=s/>
                </div>
            </section>

            <p class="account-status">{move || s.composer.status.get()}</p>
        </Modal>
    }
}
