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
use super::{act, PendingDelete, Shell};
use crate::ui::icons::{IconClose, IconTrash};
use crate::ui::inline_rename::InlineRename;
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

    // Identity-section edit toggle, and the open guild's name derived live from
    // the rail list (mirrors `accent_name`) so an InlineRename → rename_server
    // in-place patch (act/guild.rs) re-renders the name without a refetch.
    let editing_name = RwSignal::new(false);
    let server_name = move || {
        let sid = s.sel.sel_server.get();
        s.sel
            .guilds
            .get()
            .into_iter()
            .find(|g| Some(&g.id) == sid.as_ref())
            .map(|g| g.name)
            .unwrap_or_default()
    };

    // The open guild's icon media id (None = no icon → monogram), derived live
    // from the rail list like `accent_name`/`server_name` so an upload's
    // refresh_guilds re-renders the preview without extra wiring.
    let icon_id = move || {
        let sid = s.sel.sel_server.get();
        s.sel
            .guilds
            .get()
            .into_iter()
            .find(|g| Some(&g.id) == sid.as_ref())
            .and_then(|g| g.icon_id)
    };

    // Trashed-channels disclosure.
    let chan_trash_open = RwSignal::new(false);

    view! {
        <Modal class="server-modal" swipe_close=true close=move || open.set(false)>
            <header class="account-head">
                <h2>"Server"</h2>
                <button class="row-edit" title="Close"
                    on:click=move |_| open.set(false)><IconClose/></button>
            </header>

            // ---- Server icon ----
            // Upload a guild icon (owner/admin); the server re-derives the
            // per-server accent from the image, so the swatches below update
            // after an upload. Preview shows the current icon or a monogram.
            <section class="account-section">
                <h3>"Server icon"</h3>
                <div class="server-icon-row">
                    <span class="server-icon-preview" aria-hidden="true">
                        {move || match icon_id() {
                            Some(id) => view! {
                                <img src=format!("/media/{id}?w=128") alt=""/>
                            }
                            .into_any(),
                            None => view! {
                                <span class="server-icon-mono">
                                    {crate::ui::avatar::monogram(&server_name(), '#')}
                                </span>
                            }
                            .into_any(),
                        }}
                    </span>
                    <label class="server-icon-upload">
                        <span>"Upload icon"</span>
                        <input type="file" accept="image/*"
                            on:change=move |_ev| {
                                #[cfg(feature = "hydrate")]
                                {
                                    use leptos::wasm_bindgen::JsCast;
                                    if let Some(input) = _ev.target().and_then(|t| {
                                        t.dyn_into::<leptos::web_sys::HtmlInputElement>().ok()
                                    }) {
                                        if let Some(file) =
                                            input.files().and_then(|fl| fl.get(0))
                                        {
                                            if let Some(gid) = s.sel.sel_server.get_untracked() {
                                                act::set_guild_icon(s, gid, file);
                                            }
                                        }
                                    }
                                }
                            }/>
                    </label>
                </div>
            </section>

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

            // ---- Server identity ----
            // Rename (InlineRename → act::rename_server) + delete (queues the
            // shared confirm modal via PendingDelete::Server; on confirm,
            // delete_server clears sel_owner so this modal's is_owner gate in
            // shell/mod.rs unmounts it — no explicit close needed). Mirrors the
            // W3 sidebar header (shell/mod.rs:541-584). The whole ServerModal is
            // already owner-gated at the call site, so no inner is_owner Show.
            <section class="account-section">
                <h3>"Server identity"</h3>
                {move || if editing_name.get() {
                    view! {
                        <div class="identity-row">
                            <InlineRename
                                value=server_name()
                                on_save=move |v| {
                                    if let Some(gid) = s.sel.sel_server.get_untracked() {
                                        act::rename_server(s, gid, v);
                                    }
                                    editing_name.set(false);
                                }
                                on_cancel=move || editing_name.set(false)
                            />
                        </div>
                    }.into_any()
                } else {
                    view! {
                        <div class="identity-row">
                            <span class="identity-name">{server_name()}</span>
                            <button class="account-save" title="Rename server"
                                on:click=move |_| editing_name.set(true)>"Rename"</button>
                            <button class="account-save danger" title="Delete server"
                                on:click=move |_| {
                                    if let Some(gid) = s.sel.sel_server.get_untracked() {
                                        act::ask_delete(
                                            s,
                                            format!(
                                                "Delete the server “{}” and all its channels \
                                                 and messages? This cannot be undone.",
                                                server_name()
                                            ),
                                            PendingDelete::Server { gid },
                                        );
                                    }
                                }>"Delete server"</button>
                        </div>
                    }.into_any()
                }}
            </section>

            // ---- Trashed channels ----
            // Disclosure that loads soft-deleted channels (load_deleted_channels)
            // and lists them with Restore (restore_channel reloads both the trash
            // list and the live channel list via open_server). Mirrors the W3
            // sidebar trash-section (shell/mod.rs:650-695).
            <section class="account-section">
                <h3>"Trashed channels"</h3>
                <div class="trash-section">
                    <button class="trash-toggle"
                        class:active=move || chan_trash_open.get()
                        on:click=move |_| {
                            let now_open = !chan_trash_open.get_untracked();
                            chan_trash_open.set(now_open);
                            if now_open {
                                if let Some(gid) = s.sel.sel_server.get_untracked() {
                                    act::load_deleted_channels(s, gid);
                                }
                            }
                        }>
                        <IconTrash/>" Show trashed channels"
                    </button>
                    {move || chan_trash_open.get().then(|| {
                        let chans = s.trash.deleted_channels.get();
                        if chans.is_empty() {
                            view! {
                                <p class="muted trash-empty">"No trashed channels."</p>
                            }.into_any()
                        } else {
                            view! {
                                <ul class="trash-list">
                                    {chans.into_iter().map(|c| {
                                        let cid = c.id.clone();
                                        let name = c.name.clone();
                                        view! {
                                            <li class="trash-item">
                                                <span class="trash-name">"# "{name}</span>
                                                <button class="trash-restore"
                                                    on:click=move |_| {
                                                        if let Some(gid) = s.sel.sel_server.get_untracked() {
                                                            act::restore_channel(s, gid, cid.clone());
                                                        }
                                                    }>"Restore"</button>
                                            </li>
                                        }
                                    }).collect_view()}
                                </ul>
                            }.into_any()
                        }
                    })}
                </div>
            </section>

            <p class="account-status">{move || s.composer.status.get()}</p>
        </Modal>
    }
}
