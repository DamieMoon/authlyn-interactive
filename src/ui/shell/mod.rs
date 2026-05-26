//! The authed Discord-style shell: a server rail, a channel sidebar, and a
//! content pane that switches between channel messages, the lorebook editor,
//! the wardrobe, and the friends list.
//!
//! State is signal-driven (a `Copy` [`Shell`] handle); deep-link URLs are a
//! later polish. All data-fetching lives in the `act` module, defined twice —
//! real on hydrate, no-op stubs on ssr — so the view's handlers call it
//! ungated and the gloo-net client never enters the ssr graph.
//!
//! The content panes each live in their own submodule (`channel`, `wardrobe`,
//! `lorebook`, `friends`); this module owns the shared [`Shell`] state, the
//! rail/sidebar layout ([`AppShell`]), and the [`act`] action layer.

use std::collections::{HashMap, HashSet};

use leptos::prelude::*;

use crate::protocol::{
    ChannelSummary, GuildSummary, ListFriendsResponse, LorebookEntry, MessageEnvelope,
    PersonaSummary,
};

// Trash DTOs reused from protocol (no new types needed — server returns the
// existing GuildSummary / ChannelSummary / MessageEnvelope shapes for trash too).
use crate::ui::AuthCtx;

mod account;
mod channel;
mod friends;
mod lorebook;
mod wardrobe;

use account::AccountModal;
use channel::ChannelPane;
use friends::FriendsPane;
use lorebook::LorebookPane;
use wardrobe::WardrobePane;

#[component]
pub fn Home() -> impl IntoView {
    let auth = use_context::<AuthCtx>().expect("AuthCtx provided at root");

    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        if !auth.loading.get() && auth.user.get().is_none() {
            leptos_router::hooks::use_navigate()("/login", Default::default());
        }
    });

    view! {
        <Show
            when=move || auth.is_authed()
            fallback=|| view! { <p class="loading">"Loading…"</p> }
        >
            <AppShell/>
        </Show>
    }
}

#[derive(Clone, Copy, PartialEq)]
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
enum Pane {
    Friends,
    Channel,
    Lorebook,
    Wardrobe,
}

/// A destructive action awaiting confirmation. Stored in `Shell::pending_delete`
/// (with a human prompt in `confirm_prompt`); the top-level confirm modal in
/// `AppShell` dispatches the matching `act::` fn when the user confirms. Storing
/// a closure in a signal is awkward in Leptos, so we describe the action as data.
#[derive(Clone)]
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
enum PendingDelete {
    Message { cid: String, mid: String },
    Channel { gid: String, cid: String },
    Server { gid: String },
    Persona { pid: String },
}

/// All of the shell's reactive state, bundled into one `Copy` handle.
/// `pub(crate)` so the pane submodules can take it as a prop; the fields stay
/// private (submodules are descendants and can still read them).
#[derive(Clone, Copy)]
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(crate) struct Shell {
    guilds: RwSignal<Vec<GuildSummary>>,
    sel_server: RwSignal<Option<String>>,
    /// owner account id of the currently-open server (gates the invite control).
    sel_owner: RwSignal<Option<String>>,
    channels: RwSignal<Vec<ChannelSummary>>,
    sel_channel: RwSignal<Option<ChannelSummary>>,
    messages: RwSignal<Vec<MessageEnvelope>>,
    cursor: RwSignal<Option<(String, String)>>,
    /// Oldest `(sent_at, id)` currently loaded — the cursor for scroll-up
    /// backfill of older history. `None` until the first page lands.
    oldest: RwSignal<Option<(String, String)>>,
    /// Guards against overlapping scroll-up backfills.
    loading_older: RwSignal<bool>,
    /// `false` once a backfill returns a short page (start of history reached).
    more_history: RwSignal<bool>,
    /// After an older-history prepend, the message id to re-anchor to the top
    /// so the viewport doesn't jump; the channel pane scrolls it into view.
    anchor_to: RwSignal<Option<String>>,
    seen: RwSignal<HashSet<String>>,
    compose: RwSignal<String>,
    /// Media ids already uploaded and staged to send with the next message
    /// (the composer's pending image attachments, in pick order).
    compose_attachments: RwSignal<Vec<String>>,
    status: RwSignal<String>,
    polling: RwSignal<bool>,
    pane: RwSignal<Pane>,
    /// Mobile-only: whether the off-canvas rail+sidebar drawer is open.
    nav_open: RwSignal<bool>,
    friends: RwSignal<ListFriendsResponse>,
    personas: RwSignal<Vec<PersonaSummary>>,
    active_persona: RwSignal<Option<String>>,
    lore: RwSignal<Vec<LorebookEntry>>,
    /// A destructive action awaiting confirmation, with its human prompt; the
    /// top-level confirm modal renders whenever this is `Some`.
    pending_delete: RwSignal<Option<PendingDelete>>,
    confirm_prompt: RwSignal<Option<String>>,
    /// Channel ids the user has muted (no new-message notifications). Mirrored
    /// to localStorage so it survives reloads.
    muted: RwSignal<HashSet<String>>,
    /// Channel ids with unread messages — drives the sidebar glow (#23).
    /// Recomputed by the background poll against `last_seen`.
    unread: RwSignal<HashSet<String>>,
    /// Per-channel high-water mark this client has seen: channel id →
    /// (sent_at, id) of the last seen message. Persisted to localStorage;
    /// unread = the channel has messages past this mark.
    last_seen: RwSignal<HashMap<String, (String, String)>>,
    // ---- Trash (#22 Phase 2) ----
    /// Caller's own soft-deleted guilds (populated on demand, guild rail trash view).
    deleted_guilds: RwSignal<Vec<GuildSummary>>,
    /// Soft-deleted channels in the open guild (populated on demand, sidebar trash list).
    deleted_channels: RwSignal<Vec<ChannelSummary>>,
    /// Soft-deleted messages in the open channel (populated on demand).
    deleted_messages: RwSignal<Vec<MessageEnvelope>>,
    /// Whether the channel's trash overlay is open.
    show_msg_trash: RwSignal<bool>,
}

#[component]
fn AppShell() -> impl IntoView {
    let auth = use_context::<AuthCtx>().expect("AuthCtx provided at root");
    let s = Shell {
        guilds: RwSignal::new(Vec::new()),
        sel_server: RwSignal::new(None),
        sel_owner: RwSignal::new(None),
        channels: RwSignal::new(Vec::new()),
        sel_channel: RwSignal::new(None),
        messages: RwSignal::new(Vec::new()),
        cursor: RwSignal::new(None),
        oldest: RwSignal::new(None),
        loading_older: RwSignal::new(false),
        more_history: RwSignal::new(true),
        anchor_to: RwSignal::new(None),
        seen: RwSignal::new(HashSet::new()),
        compose: RwSignal::new(String::new()),
        compose_attachments: RwSignal::new(Vec::new()),
        status: RwSignal::new(String::new()),
        polling: RwSignal::new(false),
        pane: RwSignal::new(Pane::Friends),
        nav_open: RwSignal::new(false),
        friends: RwSignal::new(ListFriendsResponse {
            friends: Vec::new(),
            incoming: Vec::new(),
            outgoing: Vec::new(),
        }),
        personas: RwSignal::new(Vec::new()),
        active_persona: RwSignal::new(None),
        lore: RwSignal::new(Vec::new()),
        pending_delete: RwSignal::new(None),
        confirm_prompt: RwSignal::new(None),
        muted: RwSignal::new(HashSet::new()),
        unread: RwSignal::new(HashSet::new()),
        last_seen: RwSignal::new(HashMap::new()),
        deleted_guilds: RwSignal::new(Vec::new()),
        deleted_channels: RwSignal::new(Vec::new()),
        deleted_messages: RwSignal::new(Vec::new()),
        show_msg_trash: RwSignal::new(false),
    };
    let new_server = RwSignal::new(String::new());
    let new_channel = RwSignal::new(String::new());
    let new_invite = RwSignal::new(String::new());
    // Account-management modal visibility (change password, future options).
    let account_open = RwSignal::new(false);
    // Guild-trash panel open/closed (rail trash button toggles it).
    let guild_trash_open = RwSignal::new(false);
    // Deleted-channel list open/closed in the sidebar (owner-only).
    let chan_trash_open = RwSignal::new(false);
    // Inline-rename edit state (owner only): the server title and per-channel rows.
    let editing_server = RwSignal::new(false);
    let server_edit_buf = RwSignal::new(String::new());
    let editing_channel = RwSignal::new(None::<String>);
    let channel_edit_buf = RwSignal::new(String::new());
    // The invite/manage controls show only to the owner of the open server.
    let is_owner = move || {
        let me = auth.user.get().map(|u| u.account_id);
        me.is_some() && me == s.sel_owner.get()
    };
    // The open server's name, derived from the rail list (auto-updates on rename).
    let server_name = move || {
        let sid = s.sel_server.get();
        s.guilds
            .get()
            .into_iter()
            .find(|g| Some(&g.id) == sid.as_ref())
            .map(|g| g.name)
            .unwrap_or_default()
    };

    // On mount: load the guild rail, then try to restore the last session.
    // If nothing was stored, fall back to the Friends home. When a session is
    // restored, its channel/pane wins (we don't show Friends over it).
    // (No-ops on ssr; the stub `restore_session` returns false so ssr still
    // lands on Friends.)
    Effect::new(move |_| {
        act::refresh_guilds(s);
        if !act::restore_session(s) {
            act::show_friends(s);
        }
        // Keep the rail/sidebar/friends + open channel live (idempotent).
        act::start_sync(s);
        act::load_muted(s);
        act::load_last_seen(s);
    });

    let username = move || auth.user.get().map(|u| u.username).unwrap_or_default();

    view! {
        <div class="app" class:nav-open=move || s.nav_open.get()>
            <nav class="rail">
                <button class="rail-home" title="Friends"
                    on:click=move |_| { act::show_friends(s); s.nav_open.set(false); }>"@"</button>
                {move || s.guilds.get().into_iter().map(|g| {
                    let gid = g.id.clone();
                    let initial = g.name.chars().next().unwrap_or('#').to_uppercase().to_string();
                    let gid_active = gid.clone();
                    view! {
                        <button class="rail-guild" title=g.name
                            class:active=move || s.sel_server.get().as_deref() == Some(gid_active.as_str())
                            on:click=move |_| act::open_server(s, gid.clone())>
                            {initial}
                        </button>
                    }
                }).collect_view()}
                // Guild trash button — loads + opens the deleted-guilds panel.
                <button class="rail-trash" title="Trashed servers"
                    class:active=move || guild_trash_open.get()
                    on:click=move |_| {
                        let now_open = !guild_trash_open.get_untracked();
                        guild_trash_open.set(now_open);
                        if now_open {
                            act::load_deleted_guilds(s);
                        }
                        s.nav_open.set(false);
                    }>"🗑"</button>
                <div class="rail-add">
                    <input prop:value=move || new_server.get()
                        on:input=move |ev| new_server.set(event_target_value(&ev))
                        placeholder="new server"/>
                    <button on:click=move |_| {
                        let name = new_server.get_untracked();
                        new_server.set(String::new());
                        act::create_server(s, name);
                    }>"+"</button>
                </div>
            </nav>

            // Guild trash panel — shown in the sidebar slot when the rail trash button is active.
            {move || guild_trash_open.get().then(|| {
                let guilds = s.deleted_guilds.get();
                view! {
                    <aside class="sidebar sidebar-trash">
                        <div class="server-header">
                            <span class="server-title">"🗑 Trashed Servers"</span>
                            <button class="row-edit" title="close"
                                on:click=move |_| guild_trash_open.set(false)>"✕"</button>
                        </div>
                        {if guilds.is_empty() {
                            view! { <p class="muted pad">"No trashed servers."</p> }.into_any()
                        } else {
                            view! {
                                <ul class="trash-list">
                                    {guilds.into_iter().map(|g| {
                                        let gid = g.id.clone();
                                        let name = g.name.clone();
                                        view! {
                                            <li class="trash-item">
                                                <span class="trash-name">{name}</span>
                                                <button class="trash-restore"
                                                    on:click=move |_| {
                                                        act::restore_deleted_guild(s, gid.clone());
                                                    }>"Restore"</button>
                                            </li>
                                        }
                                    }).collect_view()}
                                </ul>
                            }.into_any()
                        }}
                    </aside>
                }
            })}

            <aside class="sidebar">
                <Show when=move || s.sel_server.get().is_some()
                    fallback=|| view! {
                        <p class="muted pad">"Pick or create a server, or visit Friends (@)."</p>
                    }>
                    <div class="server-header">
                        {move || if editing_server.get() {
                            view! {
                                <input class="rename-input" prop:value=move || server_edit_buf.get()
                                    on:input=move |ev| server_edit_buf.set(event_target_value(&ev))
                                    on:keydown=move |ev| {
                                        #[cfg(feature = "hydrate")]
                                        match ev.key().as_str() {
                                            "Enter" => {
                                                ev.prevent_default();
                                                if let Some(gid) = s.sel_server.get_untracked() {
                                                    act::rename_server(s, gid, server_edit_buf.get_untracked());
                                                }
                                                editing_server.set(false);
                                            }
                                            "Escape" => editing_server.set(false),
                                            _ => {}
                                        }
                                        #[cfg(not(feature = "hydrate"))]
                                        let _ = &ev;
                                    }/>
                                <button class="row-edit" title="save" on:click=move |_| {
                                    if let Some(gid) = s.sel_server.get_untracked() {
                                        act::rename_server(s, gid, server_edit_buf.get_untracked());
                                    }
                                    editing_server.set(false);
                                }>"✓"</button>
                                <button class="row-edit" title="cancel"
                                    on:click=move |_| editing_server.set(false)>"✕"</button>
                            }.into_any()
                        } else {
                            view! {
                                <span class="server-title">{server_name()}</span>
                                <Show when=is_owner fallback=|| ()>
                                    <button class="row-edit" title="rename server" on:click=move |_| {
                                        server_edit_buf.set(server_name());
                                        editing_server.set(true);
                                    }>"✎"</button>
                                    <button class="row-edit danger" title="delete server"
                                        on:click=move |_| {
                                            if let Some(gid) = s.sel_server.get_untracked() {
                                                act::ask_delete(
                                                    s,
                                                    format!(
                                                        "Delete the server “{}” and all its \
                                                         channels and messages? This cannot be undone.",
                                                        server_name()
                                                    ),
                                                    PendingDelete::Server { gid },
                                                );
                                            }
                                        }>"🗑"</button>
                                </Show>
                            }.into_any()
                        }}
                    </div>
                    <button class="wardrobe-btn"
                        on:click=move |_| { act::show_wardrobe(s); s.nav_open.set(false); }>
                        "🎭 Wardrobe"
                    </button>
                    <ul class="channels">
                        {move || s.channels.get().into_iter().map(|c| {
                            view! { <ChannelRow s=s ch=c editing=editing_channel buf=channel_edit_buf/> }
                        }).collect_view()}
                    </ul>
                    <Show when=is_owner fallback=|| ()>
                        <div class="channel-add">
                            <input prop:value=move || new_channel.get()
                                on:input=move |ev| new_channel.set(event_target_value(&ev))
                                placeholder="new text channel"/>
                            <button on:click=move |_| {
                                let name = new_channel.get_untracked();
                                new_channel.set(String::new());
                                act::create_channel(s, name);
                            }>"+"</button>
                        </div>
                    </Show>
                    // Deleted-channels panel (owner only).
                    <Show when=is_owner fallback=|| ()>
                        <div class="trash-section">
                            <button class="trash-toggle"
                                class:active=move || chan_trash_open.get()
                                on:click=move |_| {
                                    let now_open = !chan_trash_open.get_untracked();
                                    chan_trash_open.set(now_open);
                                    if now_open {
                                        if let Some(gid) = s.sel_server.get_untracked() {
                                            act::load_deleted_channels(s, gid);
                                        }
                                    }
                                }>
                                "🗑 Trashed channels"
                            </button>
                            {move || chan_trash_open.get().then(|| {
                                let chans = s.deleted_channels.get();
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
                                                                if let Some(gid) = s.sel_server.get_untracked() {
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
                    </Show>
                    <Show when=is_owner fallback=|| ()>
                        <div class="invite-row">
                            <input prop:value=move || new_invite.get()
                                on:input=move |ev| new_invite.set(event_target_value(&ev))
                                placeholder="invite by username"/>
                            <button on:click=move |_| {
                                let gid = s.sel_server.get_untracked();
                                let u = new_invite.get_untracked();
                                new_invite.set(String::new());
                                if let Some(gid) = gid {
                                    act::invite_member(s, gid, u);
                                }
                            }>"Invite"</button>
                        </div>
                    </Show>
                </Show>
            </aside>

            <section class="content">
                <header class="topbar">
                    <button class="nav-toggle" title="Menu"
                        on:click=move |_| s.nav_open.update(|o| *o = !*o)>"☰"</button>
                    <span class="muted">"Signed in as " <strong>{username}</strong></span>
                    // Mute toggle for the open channel (suppresses its
                    // new-message notifications); 🔔 = active, 🔕 = muted.
                    {move || s.sel_channel.get()
                        .filter(|_| s.pane.get() == Pane::Channel)
                        .map(|c| {
                            let cid = c.id.clone();
                            let cid_t = c.id.clone();
                            let cid_b = c.id.clone();
                            let cid_trash = c.id.clone();
                            view! {
                                <button class="row-edit"
                                    title=move || if s.muted.get().contains(&cid_t) { "Unmute channel" } else { "Mute channel" }
                                    on:click=move |_| act::toggle_mute(s, cid.clone())>
                                    {move || if s.muted.get().contains(&cid_b) { "🔕" } else { "🔔" }}
                                </button>
                                // Trash toggle: load and show deleted messages in this channel.
                                <button class="row-edit"
                                    title=move || if s.show_msg_trash.get() { "Hide deleted" } else { "Show deleted" }
                                    on:click=move |_| {
                                        let now_open = !s.show_msg_trash.get_untracked();
                                        s.show_msg_trash.set(now_open);
                                        if now_open {
                                            act::load_deleted_messages(s, cid_trash.clone());
                                        } else {
                                            s.deleted_messages.set(Vec::new());
                                        }
                                    }>
                                    {move || if s.show_msg_trash.get() { "🗑✓" } else { "🗑" }}
                                </button>
                            }
                        })
                    }
                    <span class="spacer"></span>
                    <button title="Account"
                        on:click=move |_| { s.status.set(String::new()); account_open.set(true); }>
                        "⚙"
                    </button>
                    <button on:click=move |_| act::logout(auth)>"Log out"</button>
                </header>
                {move || match s.pane.get() {
                    Pane::Friends => view! { <FriendsPane s=s/> }.into_any(),
                    Pane::Channel => view! { <ChannelPane s=s/> }.into_any(),
                    Pane::Lorebook => view! { <LorebookPane s=s/> }.into_any(),
                    Pane::Wardrobe => view! { <WardrobePane s=s/> }.into_any(),
                }}
                <p class="error">{move || s.status.get()}</p>
            </section>

            // Mobile drawer backdrop: tap to close (hidden off mobile via CSS).
            <div class="scrim" on:click=move |_| s.nav_open.set(false)></div>

            {move || if account_open.get() {
                view! { <AccountModal s=s open=account_open/> }.into_any()
            } else {
                ().into_any()
            }}

            // Top-level confirm dialog for destructive actions. Shown whenever a
            // `PendingDelete` is queued; backdrop/Cancel clears it without acting,
            // "Delete" dispatches the queued action (see `act::confirm_delete`).
            {move || s.pending_delete.get().is_some().then(|| {
                let prompt = s.confirm_prompt.get().unwrap_or_default();
                view! {
                    <div class="modal-backdrop" on:click=move |_| act::cancel_delete(s)>
                        <div class="modal confirm-modal" on:click=move |_ev| {
                            #[cfg(feature = "hydrate")]
                            _ev.stop_propagation();
                        }>
                            <h3>"Confirm delete"</h3>
                            <p>{prompt}</p>
                            <div class="confirm-actions">
                                <button on:click=move |_| act::cancel_delete(s)>"Cancel"</button>
                                <button class="danger"
                                    on:click=move |_| act::confirm_delete(s)>"Delete"</button>
                            </div>
                        </div>
                    </div>
                }
            })}
        </div>
    }
}

/// One channel row in the sidebar: the open-channel button, plus an owner-only
/// inline rename (✎ → input + ✓/✕). Edit state is shared across rows via the
/// `editing` (which cid, if any) and `buf` signals owned by `AppShell`.
#[component]
fn ChannelRow(
    s: Shell,
    ch: ChannelSummary,
    editing: RwSignal<Option<String>>,
    buf: RwSignal<String>,
) -> impl IntoView {
    let auth = use_context::<AuthCtx>().expect("AuthCtx provided at root");
    let is_owner = move || {
        let me = auth.user.get().map(|u| u.account_id);
        me.is_some() && me == s.sel_owner.get()
    };
    let cid = ch.id.clone();
    let name0 = ch.name.clone();
    let sigil = if ch.kind == "lorebook" { "📖 " } else { "# " };
    view! {
        <li>
            {move || {
                let cid = cid.clone();
                let name0 = name0.clone();
                let ch = ch.clone();
                if editing.get().as_deref() == Some(cid.as_str()) {
                    let cid_save = cid.clone();
                    view! {
                        <input class="rename-input" prop:value=move || buf.get()
                            on:input=move |ev| buf.set(event_target_value(&ev))
                            on:keydown={
                                let cid_kd = cid.clone();
                                move |ev| {
                                    #[cfg(feature = "hydrate")]
                                    match ev.key().as_str() {
                                        "Enter" => {
                                            ev.prevent_default();
                                            if let Some(gid) = s.sel_server.get_untracked() {
                                                act::rename_channel(s, gid, cid_kd.clone(), buf.get_untracked());
                                            }
                                            editing.set(None);
                                        }
                                        "Escape" => editing.set(None),
                                        _ => {}
                                    }
                                    #[cfg(not(feature = "hydrate"))]
                                    let _ = (&ev, &cid_kd);
                                }
                            }/>
                        <button class="row-edit" title="save" on:click=move |_| {
                            if let Some(gid) = s.sel_server.get_untracked() {
                                act::rename_channel(s, gid, cid_save.clone(), buf.get_untracked());
                            }
                            editing.set(None);
                        }>"✓"</button>
                        <button class="row-edit" title="cancel"
                            on:click=move |_| editing.set(None)>"✕"</button>
                    }.into_any()
                } else {
                    let active_cid = cid.clone();
                    let active = move || s.sel_channel.get().map(|c| c.id) == Some(active_cid.clone());
                    let unread_cid = cid.clone();
                    let unread = move || s.unread.get().contains(&unread_cid);
                    let start_cid = cid.clone();
                    let start_name = name0.clone();
                    view! {
                        <button class="channel" class:active=active class:unread=unread
                            on:click=move |_| { act::open_channel(s, ch.clone()); s.nav_open.set(false); }>
                            {sigil}{name0.clone()}
                        </button>
                        <Show when=is_owner fallback=|| ()>
                            <button class="row-edit" title="rename channel" on:click={
                                let start_cid = start_cid.clone();
                                let start_name = start_name.clone();
                                move |_| {
                                    buf.set(start_name.clone());
                                    editing.set(Some(start_cid.clone()));
                                }
                            }>"✎"</button>
                            <button class="row-edit danger" title="delete channel" on:click={
                                let del_cid = start_cid.clone();
                                let del_name = start_name.clone();
                                move |_| {
                                    if let Some(gid) = s.sel_server.get_untracked() {
                                        act::ask_delete(
                                            s,
                                            format!(
                                                "Delete the channel “{del_name}” and all its \
                                                 messages? This cannot be undone."
                                            ),
                                            PendingDelete::Channel {
                                                gid,
                                                cid: del_cid.clone(),
                                            },
                                        );
                                    }
                                }
                            }>"🗑"</button>
                        </Show>
                    }.into_any()
                }
            }}
        </li>
    }
}

// ---------------------------------------------------------------------------
// Actions — real on hydrate, no-op stubs on ssr (so the view calls them
// ungated and gloo-net never enters the ssr graph).
// ---------------------------------------------------------------------------

#[cfg(feature = "hydrate")]
mod act {
    use super::{Pane, PendingDelete, Shell};
    use crate::client::api;
    use crate::protocol::{ChannelSummary, MessageEnvelope};
    use crate::ui::AuthCtx;
    use gloo_storage::{LocalStorage, Storage};
    use leptos::prelude::*;
    use leptos::task::spawn_local;

    // localStorage keys for the last-used selection, restored on reload.
    const KEY_SERVER: &str = "authlyn.last_server";
    const KEY_CHANNEL: &str = "authlyn.last_channel";
    const KEY_PERSONA: &str = "authlyn.active_persona";

    pub fn logout(auth: AuthCtx) {
        let nav = leptos_router::hooks::use_navigate();
        spawn_local(async move {
            let _ = api::logout().await;
            auth.user.set(None);
            nav("/login", Default::default());
        });
    }

    pub fn refresh_guilds(s: Shell) {
        spawn_local(async move {
            if let Ok(r) = api::list_guilds().await {
                s.guilds.set(r.guilds);
            }
        });
    }

    /// Change the signed-in account's password. The new==confirm check is the
    /// caller's (the modal's) job; this just hits the API and reports.
    pub fn change_password(s: Shell, current: String, new: String) {
        s.status.set(String::new());
        spawn_local(async move {
            match api::change_password(&current, &new).await {
                Ok(()) => s.status.set("password changed".to_string()),
                Err(e) => s.status.set(api::humanize(&e)),
            }
        });
    }

    pub fn open_server(s: Shell, gid: String) {
        let _ = LocalStorage::set(KEY_SERVER, &gid);
        s.sel_server.set(Some(gid.clone()));
        s.sel_owner.set(None);
        s.channels.set(Vec::new());
        spawn_local(async move {
            if let Ok(d) = api::get_guild(&gid).await {
                s.sel_owner.set(Some(d.owner_id.clone()));
                s.channels.set(d.channels.clone());
                if let Some(first) = d
                    .channels
                    .iter()
                    .find(|c| c.kind == "text")
                    .or_else(|| d.channels.first())
                {
                    open_channel(s, first.clone());
                }
            }
        });
    }

    pub fn open_channel(s: Shell, ch: ChannelSummary) {
        let cid = ch.id.clone();
        let kind = ch.kind.clone();
        let _ = LocalStorage::set(KEY_CHANNEL, &cid);
        s.sel_channel.set(Some(ch));
        if kind == "lorebook" {
            s.pane.set(Pane::Lorebook);
            load_lore(s, cid);
        } else {
            s.pane.set(Pane::Channel);
            s.messages.set(Vec::new());
            s.cursor.set(None);
            s.oldest.set(None);
            s.loading_older.set(false);
            s.more_history.set(true);
            s.anchor_to.set(None);
            s.seen.update(|h| h.clear());
            // Opening clears the unread glow at once; the high-water mark
            // advances once messages load below.
            s.unread.update(|u| {
                u.remove(&cid);
            });
            start_poll(s);
            let seen_cid = cid.clone();
            spawn_local(async move {
                if let Ok(l) = api::list_messages(&cid, None).await {
                    // The initial page is the NEWEST messages (ASC); remember the
                    // oldest of it as the scroll-up cursor, and whether a full page
                    // came back (i.e. older history may exist).
                    let oldest = l
                        .messages
                        .first()
                        .map(|m| (m.sent_at.clone(), m.id.clone()));
                    let full_page = l.messages.len() == MESSAGES_PAGE_LIMIT;
                    ingest(s, l.messages);
                    s.oldest.set(oldest);
                    s.more_history.set(full_page);
                    if let Some(cur) = s.cursor.get_untracked() {
                        set_last_seen(s, &seen_cid, cur);
                    }
                }
            });
        }
    }

    /// Restore the last server / channel / worn persona from localStorage.
    ///
    /// Runs after `refresh_guilds` on mount. Returns `true` if a server was
    /// restored, so the caller can leave the Friends pane as the default only
    /// when there was nothing to restore. The whole restore is one spawned task
    /// so it never races the default `open_server` path (it doesn't call it):
    /// it fetches the guild itself, sets `sel_owner` + `channels`, then opens
    /// the *specific* stored channel (falling back to the first text channel,
    /// then any channel) via the existing `open_channel`.
    pub fn restore_session(s: Shell) -> bool {
        let Ok(gid) = LocalStorage::get::<String>(KEY_SERVER) else {
            return false;
        };
        let stored_channel = LocalStorage::get::<String>(KEY_CHANNEL).ok();
        let stored_persona = LocalStorage::get::<String>(KEY_PERSONA).ok();

        spawn_local(async move {
            let Ok(d) = api::get_guild(&gid).await else {
                // The stored server is gone — drop the stale keys and bail.
                LocalStorage::delete(KEY_SERVER);
                LocalStorage::delete(KEY_CHANNEL);
                return;
            };
            s.sel_server.set(Some(gid.clone()));
            s.sel_owner.set(Some(d.owner_id.clone()));
            s.channels.set(d.channels.clone());

            // Prefer the stored channel; fall back to the first text channel,
            // then to the first channel of any kind (matches `open_server`).
            let target = stored_channel
                .as_deref()
                .and_then(|cid| d.channels.iter().find(|c| c.id == cid))
                .or_else(|| d.channels.iter().find(|c| c.kind == "text"))
                .or_else(|| d.channels.first())
                .cloned();
            if let Some(ch) = target {
                open_channel(s, ch);
            }

            // Re-assert the worn persona for the restored server.
            if let Some(pid) = stored_persona {
                s.active_persona.set(Some(pid.clone()));
                let _ = api::set_active_persona(&gid, Some(pid)).await;
            }
        });
        true
    }

    pub fn create_server(s: Shell, name: String) {
        if name.trim().is_empty() {
            return;
        }
        spawn_local(async move {
            match api::create_guild(&name).await {
                Ok(g) => {
                    refresh_guilds(s);
                    open_server(s, g.id);
                }
                Err(e) => s.status.set(api::humanize(&e)),
            }
        });
    }

    pub fn create_channel(s: Shell, name: String) {
        let Some(gid) = s.sel_server.get_untracked() else {
            return;
        };
        if name.trim().is_empty() {
            return;
        }
        spawn_local(async move {
            match api::create_channel(&gid, &name, "text").await {
                Ok(_) => open_server(s, gid),
                Err(e) => s.status.set(api::humanize(&e)),
            }
        });
    }

    pub fn send_message(s: Shell) {
        let Some(ch) = s.sel_channel.get_untracked() else {
            return;
        };
        let body = s.compose.get_untracked();
        let attachments = s.compose_attachments.get_untracked();
        // A message needs text OR at least one image.
        if body.trim().is_empty() && attachments.is_empty() {
            return;
        }
        s.compose.set(String::new());
        s.compose_attachments.set(Vec::new());
        s.status.set(String::new());
        // Sending is a user gesture — a reliable point to request notification
        // permission so background channels can notify later.
        request_notify_permission();
        spawn_local(async move {
            match api::post_message(&ch.id, &body, attachments).await {
                Ok(_) => {
                    let cur = s.cursor.get_untracked();
                    if let Ok(l) = api::list_messages(&ch.id, cur.as_ref()).await {
                        ingest(s, l.messages);
                    }
                }
                Err(e) => s.status.set(api::humanize(&e)),
            }
        });
    }

    /// Upload a picked/pasted image and stage it as a pending composer
    /// attachment (its media id is sent with the next message).
    pub fn add_compose_attachment(s: Shell, file: web_sys::File) {
        s.status.set(String::new());
        spawn_local(async move {
            match api::upload_media(&file).await {
                Ok(id) => s.compose_attachments.update(|v| v.push(id)),
                Err(e) => s.status.set(api::humanize(&e)),
            }
        });
    }

    /// Drop one staged attachment before sending.
    pub fn remove_compose_attachment(s: Shell, id: String) {
        s.compose_attachments.update(|v| v.retain(|x| *x != id));
    }

    /// Edit one of the caller's own messages, then patch `s.messages` in
    /// place. `ingest` only appends (dedupes by id), so an edit needs a direct
    /// in-place body update — the row's id and cursor position don't change.
    pub fn edit_message(s: Shell, cid: String, mid: String, body: String) {
        let body = body.trim_end().to_string();
        if body.trim().is_empty() {
            return;
        }
        s.status.set(String::new());
        spawn_local(async move {
            match api::edit_message(&cid, &mid, &body).await {
                Ok(()) => s.messages.update(|v| {
                    if let Some(m) = v.iter_mut().find(|m| m.id == mid) {
                        m.body = body.clone();
                    }
                }),
                Err(e) => s.status.set(api::humanize(&e)),
            }
        });
    }

    /// Delete one of the caller's own messages, then drop it from `s.messages`
    /// and `s.seen` so a subsequent catch-up poll doesn't treat it as already
    /// seen (it won't reappear regardless — the server row is gone — but
    /// clearing `seen` keeps the dedupe set tidy). `s.cursor` is left as-is:
    /// it still marks the high-water mark for the poll, so deleting a row never
    /// rewinds the catch-up window.
    pub fn delete_message(s: Shell, cid: String, mid: String) {
        s.status.set(String::new());
        spawn_local(async move {
            match api::delete_message(&cid, &mid).await {
                Ok(()) => {
                    s.messages.update(|v| v.retain(|m| m.id != mid));
                    s.seen.update(|h| {
                        h.remove(&mid);
                    });
                }
                Err(e) => s.status.set(api::humanize(&e)),
            }
        });
    }

    // ---- destructive-action confirmation ----

    // localStorage key for the "ask before deleting a message" toggle. Absent or
    // any value other than "0" means ON (confirm); "0" means the user opted out.
    const KEY_CONFIRM_DELETE_MSG: &str = "authlyn.confirm_delete_message";

    /// Whether message deletes should ask for confirmation (default ON).
    pub fn confirm_delete_message_enabled() -> bool {
        LocalStorage::get::<String>(KEY_CONFIRM_DELETE_MSG)
            .map(|v| v != "0")
            .unwrap_or(true)
    }

    /// Persist the message-delete confirmation toggle.
    pub fn set_confirm_delete_message(on: bool) {
        let _ = LocalStorage::set(KEY_CONFIRM_DELETE_MSG, if on { "1" } else { "0" });
    }

    /// Queue a destructive action behind the top-level confirm modal: stash the
    /// action plus its human prompt. The modal dispatches it via `confirm_delete`.
    pub fn ask_delete(s: Shell, prompt: String, pending: PendingDelete) {
        s.confirm_prompt.set(Some(prompt));
        s.pending_delete.set(Some(pending));
    }

    /// Clear a pending confirm without acting (Cancel / backdrop).
    pub fn cancel_delete(s: Shell) {
        s.pending_delete.set(None);
        s.confirm_prompt.set(None);
    }

    /// Run the pending destructive action (the modal's "Delete"), then clear it.
    pub fn confirm_delete(s: Shell) {
        let pending = s.pending_delete.get_untracked();
        cancel_delete(s);
        match pending {
            Some(PendingDelete::Message { cid, mid }) => delete_message(s, cid, mid),
            Some(PendingDelete::Channel { gid, cid }) => delete_channel(s, gid, cid),
            Some(PendingDelete::Server { gid }) => delete_server(s, gid),
            Some(PendingDelete::Persona { pid }) => remove_persona(s, pid),
            None => {}
        }
    }

    /// Delete a channel (owner only). On success, clear the selection if it was
    /// the open channel and reload the server so the sidebar drops the dead row.
    pub fn delete_channel(s: Shell, gid: String, cid: String) {
        s.status.set(String::new());
        spawn_local(async move {
            match api::delete_channel(&gid, &cid).await {
                Ok(()) => {
                    if s.sel_channel.get_untracked().map(|c| c.id).as_deref() == Some(cid.as_str())
                    {
                        s.sel_channel.set(None);
                        s.pane.set(Pane::Friends);
                    }
                    open_server(s, gid);
                }
                Err(e) => s.status.set(api::humanize(&e)),
            }
        });
    }

    /// Delete a guild (owner only). On success, clear the server selection and
    /// refresh the rail so it no longer points at a dead id.
    pub fn delete_server(s: Shell, gid: String) {
        s.status.set(String::new());
        spawn_local(async move {
            match api::delete_guild(&gid).await {
                Ok(()) => {
                    if s.sel_server.get_untracked().as_deref() == Some(gid.as_str()) {
                        s.sel_server.set(None);
                        s.sel_owner.set(None);
                        s.channels.set(Vec::new());
                        s.sel_channel.set(None);
                        s.pane.set(Pane::Friends);
                        LocalStorage::delete(KEY_SERVER);
                        LocalStorage::delete(KEY_CHANNEL);
                    }
                    refresh_guilds(s);
                }
                Err(e) => s.status.set(api::humanize(&e)),
            }
        });
    }

    pub fn show_friends(s: Shell) {
        s.pane.set(Pane::Friends);
        reload_friends(s);
    }

    pub fn show_wardrobe(s: Shell) {
        s.pane.set(Pane::Wardrobe);
        spawn_local(async move {
            if let Ok(r) = api::list_personas().await {
                s.personas.set(r.personas);
            }
        });
    }

    pub fn create_persona(s: Shell, name: String, desc: String) {
        if name.trim().is_empty() {
            return;
        }
        spawn_local(async move {
            match api::create_persona(&name, &desc).await {
                Ok(_) => {
                    if let Ok(r) = api::list_personas().await {
                        s.personas.set(r.personas);
                    }
                }
                Err(e) => s.status.set(api::humanize(&e)),
            }
        });
    }

    /// Save edits to a persona (name + description + color), then reload the
    /// wardrobe grid so the card reflects the change. `done` is set true on
    /// success so the caller can close the detail editor.
    pub fn update_persona(
        s: Shell,
        pid: String,
        name: String,
        description: String,
        color: String,
        done: RwSignal<bool>,
    ) {
        if name.trim().is_empty() {
            s.status.set("name must not be empty".to_string());
            return;
        }
        spawn_local(async move {
            match api::patch_persona(&pid, Some(name), Some(description), Some(color)).await {
                Ok(()) => {
                    if let Ok(r) = api::list_personas().await {
                        s.personas.set(r.personas);
                    }
                    done.set(true);
                }
                Err(e) => s.status.set(api::humanize(&e)),
            }
        });
    }

    pub fn remove_persona(s: Shell, pid: String) {
        spawn_local(async move {
            match api::delete_persona(&pid).await {
                Ok(()) => {
                    // If the removed persona was being worn, take it off locally.
                    if s.active_persona.get_untracked().as_deref() == Some(pid.as_str()) {
                        LocalStorage::delete(KEY_PERSONA);
                        s.active_persona.set(None);
                    }
                    if let Ok(r) = api::list_personas().await {
                        s.personas.set(r.personas);
                    }
                }
                Err(e) => s.status.set(api::humanize(&e)),
            }
        });
    }

    /// Leave a shared persona (editor only): drop it from the caller's list.
    /// Mirrors `remove_persona`'s local cleanup, then reloads the grid.
    pub fn leave_shared_persona(s: Shell, pid: String) {
        spawn_local(async move {
            match api::leave_persona(&pid).await {
                Ok(()) => {
                    if s.active_persona.get_untracked().as_deref() == Some(pid.as_str()) {
                        LocalStorage::delete(KEY_PERSONA);
                        s.active_persona.set(None);
                    }
                    if let Ok(r) = api::list_personas().await {
                        s.personas.set(r.personas);
                    }
                }
                Err(e) => s.status.set(api::humanize(&e)),
            }
        });
    }

    /// Load the owner-only sharing state for the detail editor's friends
    /// checklist: the caller's friends, plus who already has editor access.
    pub fn load_persona_sharing(
        s: Shell,
        pid: String,
        friends: RwSignal<Vec<crate::protocol::FriendSummary>>,
        editors: RwSignal<Vec<crate::protocol::PersonaEditor>>,
    ) {
        spawn_local(async move {
            match api::list_friends().await {
                Ok(r) => friends.set(r.friends),
                Err(e) => s.status.set(api::humanize(&e)),
            }
            if let Ok(r) = api::list_persona_editors(&pid).await {
                editors.set(r.editors);
            }
        });
    }

    /// Toggle whether a friend may edit/wear this persona (owner only): check =
    /// grant, uncheck = revoke. Refreshes the editor set the checklist binds to.
    pub fn set_persona_share(
        s: Shell,
        pid: String,
        aid: String,
        share: bool,
        editors: RwSignal<Vec<crate::protocol::PersonaEditor>>,
    ) {
        spawn_local(async move {
            let res = if share {
                api::add_persona_editor(&pid, &aid).await
            } else {
                api::remove_persona_editor(&pid, &aid).await
            };
            match res {
                Ok(()) => {
                    if let Ok(r) = api::list_persona_editors(&pid).await {
                        editors.set(r.editors);
                    }
                }
                Err(e) => s.status.set(api::humanize(&e)),
            }
        });
    }

    /// Upload a picture and set it as the persona's avatar: POST the file to
    /// `/media`, then PUT the returned id as the avatar, then reload the grid so
    /// the new portrait shows. Errors surface via `s.status`.
    pub fn set_persona_avatar(s: Shell, pid: String, file: web_sys::File) {
        s.status.set(String::new());
        spawn_local(async move {
            let media_id = match api::upload_media(&file).await {
                Ok(id) => id,
                Err(e) => {
                    s.status.set(api::humanize(&e));
                    return;
                }
            };
            match api::set_persona_avatar(&pid, &media_id).await {
                Ok(()) => {
                    if let Ok(r) = api::list_personas().await {
                        s.personas.set(r.personas);
                    }
                }
                Err(e) => s.status.set(api::humanize(&e)),
            }
        });
    }

    pub fn wear_persona(s: Shell, pid: String) {
        let _ = LocalStorage::set(KEY_PERSONA, &pid);
        s.active_persona.set(Some(pid.clone()));
        if let Some(gid) = s.sel_server.get_untracked() {
            spawn_local(async move {
                let _ = api::set_active_persona(&gid, Some(pid)).await;
            });
        }
    }

    pub fn unwear(s: Shell) {
        LocalStorage::delete(KEY_PERSONA);
        s.active_persona.set(None);
        if let Some(gid) = s.sel_server.get_untracked() {
            spawn_local(async move {
                let _ = api::set_active_persona(&gid, None).await;
            });
        }
    }

    pub fn add_friend(s: Shell, username: String) {
        if username.trim().is_empty() {
            return;
        }
        spawn_local(async move {
            match api::add_friend(&username).await {
                Ok(()) => reload_friends(s),
                Err(e) => s.status.set(api::humanize(&e)),
            }
        });
    }

    pub fn invite_member(s: Shell, gid: String, username: String) {
        let username = username.trim().to_string();
        if username.is_empty() {
            return;
        }
        spawn_local(async move {
            match api::invite_member(&gid, &username).await {
                Ok(()) => s.status.set(format!("invited {username}")),
                Err(e) => s.status.set(api::humanize(&e)),
            }
        });
    }

    pub fn rename_server(s: Shell, gid: String, name: String) {
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        spawn_local(async move {
            match api::patch_guild(&gid, &name).await {
                // Patch the rail list in place; the sidebar title derives from it.
                Ok(()) => s.guilds.update(|gs| {
                    if let Some(g) = gs.iter_mut().find(|g| g.id == gid) {
                        g.name = name.clone();
                    }
                }),
                Err(e) => s.status.set(api::humanize(&e)),
            }
        });
    }

    pub fn rename_channel(s: Shell, gid: String, cid: String, name: String) {
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        spawn_local(async move {
            match api::patch_channel(&gid, &cid, &name).await {
                Ok(()) => {
                    s.channels.update(|cs| {
                        if let Some(c) = cs.iter_mut().find(|c| c.id == cid) {
                            c.name = name.clone();
                        }
                    });
                    s.sel_channel.update(|sc| {
                        if let Some(c) = sc {
                            if c.id == cid {
                                c.name = name.clone();
                            }
                        }
                    });
                }
                Err(e) => s.status.set(api::humanize(&e)),
            }
        });
    }

    pub fn accept_friend(s: Shell, aid: String) {
        spawn_local(async move {
            let _ = api::accept_friend(&aid).await;
            reload_friends(s);
        });
    }

    pub fn remove_friend(s: Shell, aid: String) {
        spawn_local(async move {
            let _ = api::remove_friend(&aid).await;
            reload_friends(s);
        });
    }

    pub fn create_lore(s: Shell, cid: String, keys: Vec<String>, content: String) {
        if cid.is_empty() || content.trim().is_empty() {
            return;
        }
        spawn_local(async move {
            match api::create_lore(&cid, keys, &content).await {
                Ok(_) => load_lore(s, cid),
                Err(e) => s.status.set(api::humanize(&e)),
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn patch_lore(
        s: Shell,
        cid: String,
        eid: String,
        title: Option<String>,
        keys: Option<Vec<String>>,
        content: Option<String>,
        enabled: Option<bool>,
        position: Option<i64>,
    ) {
        use crate::protocol::PatchLorebookEntryRequest;
        spawn_local(async move {
            let req = PatchLorebookEntryRequest {
                title,
                keys,
                content,
                enabled,
                position,
            };
            match api::patch_lore(&cid, &eid, &req).await {
                Ok(()) => load_lore(s, cid),
                Err(e) => s.status.set(api::humanize(&e)),
            }
        });
    }

    /// Swap `eid` with the neighbor above (`up = true`) or below (`up = false`)
    /// by exchanging their `position` values, then reload the list.
    pub fn swap_lore(s: Shell, cid: String, eid: String, position: i64, up: bool) {
        use crate::protocol::PatchLorebookEntryRequest;
        let entries = s.lore.get_untracked();
        let neighbor = if up {
            entries
                .iter()
                .filter(|e| e.position < position)
                .max_by_key(|e| e.position)
                .cloned()
        } else {
            entries
                .iter()
                .filter(|e| e.position > position)
                .min_by_key(|e| e.position)
                .cloned()
        };
        let Some(nbr) = neighbor else { return };
        let nbr_pos = nbr.position;
        let nbr_id = nbr.id.clone();
        let cid2 = cid.clone();
        spawn_local(async move {
            let r1 = api::patch_lore(
                &cid,
                &eid,
                &PatchLorebookEntryRequest {
                    position: Some(nbr_pos),
                    ..Default::default()
                },
            )
            .await;
            let r2 = api::patch_lore(
                &cid2,
                &nbr_id,
                &PatchLorebookEntryRequest {
                    position: Some(position),
                    ..Default::default()
                },
            )
            .await;
            match (r1, r2) {
                (Ok(()), Ok(())) => load_lore(s, cid),
                (Err(e), _) | (_, Err(e)) => s.status.set(api::humanize(&e)),
            }
        });
    }

    pub fn delete_lore(s: Shell, cid: String, eid: String) {
        spawn_local(async move {
            let _ = api::delete_lore(&cid, &eid).await;
            load_lore(s, cid);
        });
    }

    /// Submit a feedback item. Closes the modal on success; surfaces the error
    /// via `s.status` on failure.
    pub fn submit_feedback(
        s: Shell,
        kind: String,
        body: String,
        context: Option<String>,
        modal_open: RwSignal<bool>,
    ) {
        use crate::protocol::SubmitFeedbackRequest;
        s.status.set(String::new());
        spawn_local(async move {
            match api::submit_feedback(&SubmitFeedbackRequest {
                kind,
                body,
                context,
            })
            .await
            {
                Ok(()) => modal_open.set(false),
                Err(e) => s.status.set(api::humanize(&e)),
            }
        });
    }

    // ---- Trash (#22 Phase 2) ----

    /// Load the caller's own soft-deleted guilds into `s.deleted_guilds`.
    pub fn load_deleted_guilds(s: Shell) {
        spawn_local(async move {
            match api::list_deleted_guilds().await {
                Ok(r) => s.deleted_guilds.set(r.guilds),
                Err(e) => s.status.set(api::humanize(&e)),
            }
        });
    }

    /// Build the context JSON string to attach to a feedback submission.
    /// Reads the currently-selected channel id (from Shell signals), the app
    /// version (compile-time constant), and navigator.userAgent (browser API).
    /// All three are wrapped in a small JSON object and returned as a String.
    pub fn build_feedback_context(s: Shell) -> Option<String> {
        let channel_id = s
            .sel_channel
            .get_untracked()
            .map(|c| c.id)
            .unwrap_or_default();
        let version = env!("CARGO_PKG_VERSION");
        // navigator.userAgent via reflection — `navigator()` is behind the
        // `Navigator` web-sys feature which isn't enabled in this build;
        // the same reflection pattern used by `ensure_push_subscription`.
        let user_agent = (|| {
            use wasm_bindgen::JsValue;
            let win = leptos::web_sys::window()?;
            let nav = js_sys::Reflect::get(&win, &JsValue::from_str("navigator")).ok()?;
            let ua = js_sys::Reflect::get(&nav, &JsValue::from_str("userAgent")).ok()?;
            ua.as_string()
        })()
        .unwrap_or_default();
        // Minimal hand-built JSON — no serde dependency needed for a small static shape.
        let ctx = format!(
            r#"{{"channel_id":{:?},"version":{:?},"user_agent":{:?}}}"#,
            channel_id, version, user_agent
        );
        Some(ctx)
    }

    /// Restore a soft-deleted guild (owner). On success, refresh the rail and
    /// the deleted-guilds list so the restored server reappears and leaves trash.
    pub fn restore_deleted_guild(s: Shell, gid: String) {
        spawn_local(async move {
            match api::restore_guild(&gid).await {
                Ok(()) => {
                    refresh_guilds(s);
                    load_deleted_guilds(s);
                }
                Err(e) => s.status.set(api::humanize(&e)),
            }
        });
    }

    /// Load soft-deleted channels for the given guild into `s.deleted_channels`.
    pub fn load_deleted_channels(s: Shell, gid: String) {
        spawn_local(async move {
            match api::list_deleted_channels(&gid).await {
                Ok(r) => s.deleted_channels.set(r.channels),
                Err(e) => s.status.set(api::humanize(&e)),
            }
        });
    }

    /// Restore a soft-deleted channel (owner/admin). On success, reload the
    /// server so the channel reappears in the sidebar, and refresh the deleted list.
    pub fn restore_channel(s: Shell, gid: String, cid: String) {
        spawn_local(async move {
            match api::restore_channel(&gid, &cid).await {
                Ok(()) => {
                    load_deleted_channels(s, gid.clone());
                    open_server(s, gid);
                }
                Err(e) => s.status.set(api::humanize(&e)),
            }
        });
    }

    /// Load soft-deleted messages for the given channel into `s.deleted_messages`.
    pub fn load_deleted_messages(s: Shell, cid: String) {
        spawn_local(async move {
            match api::list_deleted_messages(&cid).await {
                Ok(r) => s.deleted_messages.set(r.messages),
                Err(e) => s.status.set(api::humanize(&e)),
            }
        });
    }

    /// Restore one of the caller's own deleted messages. On success, remove it
    /// from the trash list and reload the channel messages.
    pub fn restore_deleted_message(s: Shell, cid: String, mid: String) {
        spawn_local(async move {
            match api::restore_message(&cid, &mid).await {
                Ok(()) => {
                    // Drop from the trash list immediately (no re-load needed).
                    s.deleted_messages.update(|v| v.retain(|m| m.id != mid));
                    // Reload channel messages so the restored one reappears.
                    if let Ok(l) = api::list_messages(&cid, None).await {
                        s.messages.set(l.messages.clone());
                        s.seen.update(|h| {
                            h.clear();
                            for m in &l.messages {
                                h.insert(m.id.clone());
                            }
                        });
                        s.cursor
                            .set(l.messages.last().map(|m| (m.sent_at.clone(), m.id.clone())));
                    }
                }
                Err(e) => s.status.set(api::humanize(&e)),
            }
        });
    }

    // ---- internal ----

    fn reload_friends(s: Shell) {
        spawn_local(async move {
            if let Ok(f) = api::list_friends().await {
                s.friends.set(f);
            }
        });
    }

    fn load_lore(s: Shell, cid: String) {
        spawn_local(async move {
            if let Ok(l) = api::list_lore(&cid).await {
                s.lore.set(l.entries);
            }
        });
    }

    fn ingest(s: Shell, incoming: Vec<MessageEnvelope>) {
        for m in incoming {
            if s.seen.with_untracked(|h| h.contains(&m.id)) {
                continue;
            }
            s.seen.update(|h| {
                h.insert(m.id.clone());
            });
            s.cursor.set(Some((m.sent_at.clone(), m.id.clone())));
            s.messages.update(|v| v.push(m));
        }
    }

    // localStorage key for the muted-channel id list.
    const KEY_MUTED: &str = "authlyn.muted_channels";

    /// Load muted channels from localStorage into the reactive set (on mount).
    pub fn load_muted(s: Shell) {
        let ids = LocalStorage::get::<Vec<String>>(KEY_MUTED).unwrap_or_default();
        s.muted.set(ids.into_iter().collect());
    }

    /// localStorage key for the per-channel last-seen high-water marks (#23).
    const KEY_LAST_SEEN: &str = "authlyn.last_seen";

    /// Load last-seen marks from localStorage into the reactive map (on mount).
    pub fn load_last_seen(s: Shell) {
        s.last_seen
            .set(LocalStorage::get(KEY_LAST_SEEN).unwrap_or_default());
    }

    /// Record `cur = (sent_at, id)` as the last message seen in `cid`, and
    /// persist the whole map. Idempotent.
    fn set_last_seen(s: Shell, cid: &str, cur: (String, String)) {
        s.last_seen.update(|m| {
            m.insert(cid.to_string(), cur);
        });
        let _ = LocalStorage::set(KEY_LAST_SEEN, s.last_seen.get_untracked());
    }

    /// Recompute the unread set for the open server's channels (#23). The open
    /// channel is always considered seen (advance its mark to the live cursor);
    /// every other text channel is "unread" iff it has any message past its
    /// last-seen mark. A never-seen channel is baselined to its current latest
    /// (no retroactive glow on first sight). Runs on the ~6s list tick.
    fn refresh_unread(s: Shell) {
        let open = s.sel_channel.get_untracked().map(|c| c.id);
        if let Some(ref oc) = open {
            if let Some(cur) = s.cursor.get_untracked() {
                set_last_seen(s, oc, cur);
            }
            s.unread.update(|u| {
                u.remove(oc);
            });
        }
        let channels = s.channels.get_untracked();
        spawn_local(async move {
            for ch in channels {
                if ch.kind != "text" || Some(&ch.id) == open.as_ref() {
                    continue;
                }
                match s.last_seen.with_untracked(|m| m.get(&ch.id).cloned()) {
                    Some(cur) => {
                        let Ok(l) = api::list_messages(&ch.id, Some(&cur)).await else {
                            continue;
                        };
                        let has_new = !l.messages.is_empty();
                        let marked = s.unread.with_untracked(|u| u.contains(&ch.id));
                        if has_new != marked {
                            s.unread.update(|u| {
                                if has_new {
                                    u.insert(ch.id.clone());
                                } else {
                                    u.remove(&ch.id);
                                }
                            });
                        }
                    }
                    // First sight: baseline to the current latest, don't glow.
                    None => {
                        if let Ok(l) = api::list_messages(&ch.id, None).await {
                            if let Some(last) = l.messages.last() {
                                set_last_seen(s, &ch.id, (last.sent_at.clone(), last.id.clone()));
                            }
                        }
                    }
                }
            }
        });
    }

    /// Toggle mute for a channel: flip the reactive set + persist. A click is a
    /// user gesture, so it's also a good moment to ask for notification permission.
    pub fn toggle_mute(s: Shell, cid: String) {
        s.muted.update(|m| {
            if !m.remove(&cid) {
                m.insert(cid.clone());
            }
        });
        let ids: Vec<String> = s.muted.with_untracked(|m| m.iter().cloned().collect());
        let _ = LocalStorage::set(KEY_MUTED, &ids);
        request_notify_permission();
    }

    /// True only when `window.Notification` actually exists. iOS Safari outside
    /// an installed PWA has no `Notification` global at all, and *touching*
    /// `web_sys::Notification::permission()` there traps the WASM (the binding
    /// dereferences an undefined global). Feature-detect via reflection first so
    /// the whole notification path can never throw / abort the send-receive flow.
    fn notifications_available() -> bool {
        let Some(win) = leptos::web_sys::window() else {
            return false;
        };
        match js_sys::Reflect::get(&win, &wasm_bindgen::JsValue::from_str("Notification")) {
            Ok(v) => !v.is_undefined() && !v.is_null(),
            Err(_) => false,
        }
    }

    /// Ask for Web Notification permission if undecided, and once it is (or
    /// already is) granted, register a Web Push subscription so notifications
    /// arrive even when the PWA is backgrounded/closed (#30). Must run from a
    /// user gesture — `request_permission` is gesture-bound, and on iOS the
    /// subscribe that follows it is too, so both ride the same tap. No-ops
    /// (never throws) where the API is missing — e.g. iOS Safari outside an
    /// installed PWA.
    fn request_notify_permission() {
        use leptos::web_sys::{Notification, NotificationPermission};
        if !notifications_available() {
            return;
        }
        match Notification::permission() {
            NotificationPermission::Default => {
                // Ask; subscribe only after the user actually grants.
                if let Ok(promise) = Notification::request_permission() {
                    spawn_local(async move {
                        if let Ok(v) = wasm_bindgen_futures::JsFuture::from(promise).await {
                            if v.as_string().as_deref() == Some("granted") {
                                ensure_push_subscription().await;
                            }
                        }
                    });
                }
            }
            NotificationPermission::Granted => {
                // Already granted (a prior session, or the first send/mute after
                // push shipped): make sure a subscription exists. Idempotent —
                // getSubscription() short-circuits if we already have one. Runs
                // from this gesture, so iOS is satisfied.
                spawn_local(async move {
                    ensure_push_subscription().await;
                });
            }
            _ => {}
        }
    }

    /// Ensure this browser has a Web Push subscription registered with the
    /// server. Idempotent: reuses an existing subscription, else subscribes
    /// using the server's VAPID public key and POSTs the result. Entirely
    /// reflection-driven (no extra web-sys features) and all-or-nothing — any
    /// missing API (no `serviceWorker`, no `pushManager`, e.g. iOS Safari
    /// outside an installed PWA) just makes it a silent no-op. Call only after
    /// Notification permission is granted.
    async fn ensure_push_subscription() {
        use wasm_bindgen::{JsCast, JsValue};
        use wasm_bindgen_futures::JsFuture;

        let _ = async {
            let win = leptos::web_sys::window()?;
            let nav = js_sys::Reflect::get(&win, &JsValue::from_str("navigator")).ok()?;
            let sw = js_sys::Reflect::get(&nav, &JsValue::from_str("serviceWorker")).ok()?;
            if sw.is_undefined() || sw.is_null() {
                return None;
            }
            // navigator.serviceWorker.ready : Promise<ServiceWorkerRegistration>
            let ready: js_sys::Promise = js_sys::Reflect::get(&sw, &JsValue::from_str("ready"))
                .ok()?
                .dyn_into()
                .ok()?;
            let reg = JsFuture::from(ready).await.ok()?;
            let pm = js_sys::Reflect::get(&reg, &JsValue::from_str("pushManager")).ok()?;
            if pm.is_undefined() || pm.is_null() {
                return None; // no Push API (e.g. iOS Safari outside an installed PWA)
            }

            // Reuse an existing subscription if the browser already has one.
            let get_sub: js_sys::Function =
                js_sys::Reflect::get(&pm, &JsValue::from_str("getSubscription"))
                    .ok()?
                    .dyn_into()
                    .ok()?;
            let existing = JsFuture::from(
                get_sub
                    .call0(&pm)
                    .ok()?
                    .dyn_into::<js_sys::Promise>()
                    .ok()?,
            )
            .await
            .ok()?;

            let subscription = if existing.is_null() || existing.is_undefined() {
                // Fresh subscribe: needs the server's VAPID public key as a
                // Uint8Array applicationServerKey (a base64url string fails on iOS).
                let key_b64 = api::push_vapid_key().await.ok()?.key;
                let key_bytes = base64url_to_bytes(&key_b64)?;
                let key_arr = js_sys::Uint8Array::from(key_bytes.as_slice());

                let opts = js_sys::Object::new();
                js_sys::Reflect::set(&opts, &JsValue::from_str("userVisibleOnly"), &JsValue::TRUE)
                    .ok()?;
                js_sys::Reflect::set(&opts, &JsValue::from_str("applicationServerKey"), &key_arr)
                    .ok()?;

                let subscribe: js_sys::Function =
                    js_sys::Reflect::get(&pm, &JsValue::from_str("subscribe"))
                        .ok()?
                        .dyn_into()
                        .ok()?;
                let p: js_sys::Promise = subscribe.call1(&pm, &opts).ok()?.dyn_into().ok()?;
                JsFuture::from(p).await.ok()?
            } else {
                existing
            };

            // subscription.toJSON() -> { endpoint, keys: { p256dh, auth } }
            let to_json: js_sys::Function =
                js_sys::Reflect::get(&subscription, &JsValue::from_str("toJSON"))
                    .ok()?
                    .dyn_into()
                    .ok()?;
            let json = to_json.call0(&subscription).ok()?;
            let endpoint = js_sys::Reflect::get(&json, &JsValue::from_str("endpoint"))
                .ok()?
                .as_string()?;
            let keys = js_sys::Reflect::get(&json, &JsValue::from_str("keys")).ok()?;
            let p256dh = js_sys::Reflect::get(&keys, &JsValue::from_str("p256dh"))
                .ok()?
                .as_string()?;
            let auth = js_sys::Reflect::get(&keys, &JsValue::from_str("auth"))
                .ok()?
                .as_string()?;

            api::push_subscribe(&crate::protocol::PushSubscribeRequest {
                endpoint,
                keys: crate::protocol::PushSubscriptionKeys { p256dh, auth },
            })
            .await
            .ok()?;
            Some(())
        }
        .await;
    }

    /// Decode a base64url-unpadded string (the VAPID public key) to bytes.
    fn base64url_to_bytes(s: &str) -> Option<Vec<u8>> {
        use base64::Engine;
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(s)
            .ok()
    }

    /// Show `title` as a notification, preferring the service-worker
    /// `registration.showNotification()` path so it works when the app runs as
    /// an installed PWA (standalone display mode), where the `new Notification()`
    /// constructor is unavailable / throws. Falls back to the constructor for a
    /// plain browser tab. Every step is fallible and swallowed: this function
    /// must never throw and never block the caller (a notification failure must
    /// not break message send/receive).
    fn show_notification(title: &str) {
        use wasm_bindgen::closure::Closure;
        use wasm_bindgen::{JsCast, JsValue};

        // SW path: navigator.serviceWorker.ready -> reg.showNotification(title).
        // Driven entirely by reflection (`Reflect::get` + `Function::call`) so
        // it needs no extra web-sys features (no Navigator / ServiceWorker*
        // bindings) and any missing member just yields `None` -> silent
        // fallback. The promise is chained with `.then(onFulfilled, onRejected)`
        // so a rejected `ready`/`showNotification` is swallowed too.
        let sw_dispatched = (|| -> Option<()> {
            let win = leptos::web_sys::window()?;
            // `window.navigator` by reflection (the Navigator web-sys feature
            // isn't enabled in this build).
            let nav = js_sys::Reflect::get(&win, &JsValue::from_str("navigator")).ok()?;
            let sw = js_sys::Reflect::get(&nav, &JsValue::from_str("serviceWorker")).ok()?;
            if sw.is_undefined() || sw.is_null() {
                return None;
            }
            let ready = js_sys::Reflect::get(&sw, &JsValue::from_str("ready")).ok()?;
            let ready: js_sys::Promise = ready.dyn_into().ok()?;
            let title = title.to_owned();
            // `reg.showNotification(title)` once the registration resolves.
            let on_ready = Closure::once_into_js(move |reg: JsValue| {
                let _ = (|| -> Option<()> {
                    let show =
                        js_sys::Reflect::get(&reg, &JsValue::from_str("showNotification")).ok()?;
                    let show: js_sys::Function = show.dyn_into().ok()?;
                    // Returns a Promise; swallow a rejection so it never
                    // surfaces as an unhandled rejection.
                    let p = show.call1(&reg, &JsValue::from_str(&title)).ok()?;
                    if let Ok(p) = p.dyn_into::<js_sys::Promise>() {
                        let noop = js_sys::Function::new_no_args("");
                        let _ = then_via_reflect(&p, &on_ready_noop(), &noop);
                    }
                    Some(())
                })();
            });
            let on_ready: js_sys::Function = on_ready.dyn_into().ok()?;
            let noop = js_sys::Function::new_no_args("");
            // `ready.then(on_ready, noop)` — invoked reflectively so we pass
            // plain `Function`s instead of typed wasm-bindgen `Closure`s.
            then_via_reflect(&ready, &on_ready, &noop)?;
            Some(())
        })();

        if sw_dispatched.is_some() {
            return;
        }

        // Fallback: plain browser tab. Guard so a throwing constructor (some
        // standalone contexts) can't propagate.
        if notifications_available() {
            let _ = leptos::web_sys::Notification::new(title);
        }
    }

    /// A no-op fulfilment callback for the inner `showNotification` promise.
    fn on_ready_noop() -> js_sys::Function {
        js_sys::Function::new_no_args("")
    }

    /// Call `promise.then(on_fulfilled, on_rejected)` via reflection so the
    /// callbacks can be plain `js_sys::Function`s. Returns `None` if `then`
    /// is missing or the call traps. Never throws.
    fn then_via_reflect(
        promise: &js_sys::Promise,
        on_fulfilled: &js_sys::Function,
        on_rejected: &js_sys::Function,
    ) -> Option<()> {
        use wasm_bindgen::JsCast;
        let then = js_sys::Reflect::get(promise, &wasm_bindgen::JsValue::from_str("then")).ok()?;
        let then: js_sys::Function = then.dyn_into().ok()?;
        then.call2(promise, on_fulfilled, on_rejected).ok()?;
        Some(())
    }

    /// True when the tab/PWA is backgrounded (so the user would miss messages).
    fn tab_hidden() -> bool {
        leptos::web_sys::window()
            .and_then(|w| w.document())
            .map(|d| d.hidden())
            .unwrap_or(false)
    }

    /// The subset of `msgs` not yet in `s.seen` — genuinely new this tick.
    fn unseen(s: Shell, msgs: &[MessageEnvelope]) -> Vec<MessageEnvelope> {
        msgs.iter()
            .filter(|m| !s.seen.with_untracked(|h| h.contains(&m.id)))
            .cloned()
            .collect()
    }

    /// Fire a Web Notification for new messages in `ch` — but only when the tab
    /// is backgrounded (you'd see them otherwise), the channel isn't muted, and
    /// permission was granted. Title-only to keep the web-sys surface minimal.
    fn notify_messages(s: Shell, ch: &ChannelSummary, fresh: &[MessageEnvelope]) {
        use leptos::web_sys::{Notification, NotificationPermission};
        if fresh.is_empty() || !tab_hidden() {
            return;
        }
        if s.muted.with_untracked(|m| m.contains(&ch.id)) {
            return;
        }
        // Feature-detect before reading permission: on iOS Safari outside an
        // installed PWA the Notification global is absent and the permission
        // read itself would trap.
        if !notifications_available() {
            return;
        }
        if Notification::permission() != NotificationPermission::Granted {
            return;
        }
        let title = if fresh.len() > 1 {
            format!("{} new messages in #{}", fresh.len(), ch.name)
        } else {
            let last = &fresh[0];
            let who = last
                .persona_name
                .clone()
                .unwrap_or_else(|| last.author_display.clone());
            format!("{who} in #{}", ch.name)
        };
        show_notification(&title);
    }

    /// Server page size for messages (mirrors `MESSAGES_PAGE_LIMIT` on the
    /// server). Below this, the whole channel is loaded in one page and can be
    /// reconciled wholesale; at/above it we only append.
    const MESSAGES_PAGE_LIMIT: usize = 100;

    /// Full-set reconcile for a channel that fits in one page: reflects new,
    /// edited, and deleted messages (including from other people), writing the
    /// signal only when something actually changed so an idle poll causes no
    /// re-render or scroll jump.
    fn sync_messages(s: Shell, fresh: Vec<MessageEnvelope>) {
        let changed = s.messages.with_untracked(|cur| {
            cur.len() != fresh.len()
                || cur.iter().zip(fresh.iter()).any(|(a, b)| {
                    a.id != b.id || a.body != b.body || a.persona_name != b.persona_name
                })
        });
        if !changed {
            return;
        }
        s.seen.update(|h| {
            h.clear();
            for m in &fresh {
                h.insert(m.id.clone());
            }
        });
        s.cursor
            .set(fresh.last().map(|m| (m.sent_at.clone(), m.id.clone())));
        s.messages.set(fresh);
    }

    /// In-place refresh of the guild rail, the open server's channels, and the
    /// friends list — each written only when it changed, so things created or
    /// removed elsewhere appear/disappear without a manual reload.
    fn refresh_lists(s: Shell) {
        let sel = s.sel_server.get_untracked();
        spawn_local(async move {
            if let Ok(r) = api::list_guilds().await {
                if s.guilds.with_untracked(|g| *g != r.guilds) {
                    s.guilds.set(r.guilds);
                }
            }
            if let Ok(f) = api::list_friends().await {
                if s.friends.with_untracked(|cur| *cur != f) {
                    s.friends.set(f);
                }
            }
            if let Some(gid) = sel {
                if let Ok(d) = api::get_guild(&gid).await {
                    if s.channels.with_untracked(|c| *c != d.channels) {
                        s.channels.set(d.channels);
                    }
                }
            }
        });
    }

    /// The background sync loop (single instance, guarded by `s.polling`).
    /// Every tick it refreshes the open channel's messages; every ~6s it also
    /// refreshes the lists. Started on shell mount via [`start_sync`] so the
    /// lists stay live even on the Friends pane. SEAM: replace with SSE.
    /// Backfill older history when the user scrolls near the top: fetch the
    /// page immediately before `oldest`, prepend it, and ask the channel pane
    /// to re-anchor so the viewport stays put. Guarded against overlap and
    /// against running past the start of history.
    pub fn load_older(s: Shell) {
        if s.loading_older.get_untracked() || !s.more_history.get_untracked() {
            return;
        }
        let Some(oldest) = s.oldest.get_untracked() else {
            return;
        };
        let Some(ch) = s.sel_channel.get_untracked() else {
            return;
        };
        s.loading_older.set(true);
        spawn_local(async move {
            if let Ok(l) = api::list_messages_before(&ch.id, &oldest).await {
                if l.messages.len() < MESSAGES_PAGE_LIMIT {
                    s.more_history.set(false);
                }
                let fresh: Vec<_> = l
                    .messages
                    .into_iter()
                    .filter(|m| !s.seen.with_untracked(|h| h.contains(&m.id)))
                    .collect();
                if !fresh.is_empty() {
                    s.oldest
                        .set(fresh.first().map(|m| (m.sent_at.clone(), m.id.clone())));
                    s.seen.update(|h| {
                        for m in &fresh {
                            h.insert(m.id.clone());
                        }
                    });
                    // Anchor to the row currently at the top before everything
                    // shifts down, so the viewport doesn't jump.
                    let anchor = s
                        .messages
                        .with_untracked(|v| v.first().map(|m| m.id.clone()));
                    s.anchor_to.set(anchor);
                    s.messages.update(|v| {
                        let mut nw = fresh;
                        nw.append(v);
                        *v = nw;
                    });
                }
            }
            s.loading_older.set(false);
        });
    }

    fn start_poll(s: Shell) {
        if s.polling.get_untracked() {
            return;
        }
        s.polling.set(true);
        spawn_local(async move {
            let mut tick: u32 = 0;
            loop {
                gloo_timers::future::TimeoutFuture::new(1500).await;
                tick = tick.wrapping_add(1);
                if tick.is_multiple_of(4) {
                    refresh_lists(s);
                    refresh_unread(s);
                }
                if s.pane.get_untracked() != Pane::Channel {
                    continue;
                }
                let Some(ch) = s.sel_channel.get_untracked() else {
                    continue;
                };
                match api::list_messages(&ch.id, None).await {
                    Ok(l) if l.messages.len() < MESSAGES_PAGE_LIMIT => {
                        let fresh = unseen(s, &l.messages);
                        sync_messages(s, l.messages);
                        notify_messages(s, &ch, &fresh);
                    }
                    Ok(_) => {
                        // Long history: page 1 isn't the whole channel, so only
                        // append new messages past the cursor.
                        let cur = s.cursor.get_untracked();
                        if let Ok(l) = api::list_messages(&ch.id, cur.as_ref()).await {
                            let fresh = unseen(s, &l.messages);
                            ingest(s, l.messages);
                            notify_messages(s, &ch, &fresh);
                        }
                    }
                    Err(_) => {}
                }
            }
        });
    }

    /// Start the background sync loop (idempotent). Called on shell mount so the
    /// rail/sidebar/friends stay live before any channel is opened.
    pub fn start_sync(s: Shell) {
        start_poll(s);
    }
}

#[cfg(not(feature = "hydrate"))]
mod act {
    use super::{PendingDelete, Shell};
    use crate::protocol::ChannelSummary;
    use crate::ui::AuthCtx;
    use leptos::prelude::RwSignal;

    pub fn logout(_auth: AuthCtx) {}
    pub fn refresh_guilds(_s: Shell) {}
    pub fn start_sync(_s: Shell) {}
    pub fn load_muted(_s: Shell) {}
    pub fn load_last_seen(_s: Shell) {}
    pub fn toggle_mute(_s: Shell, _cid: String) {}
    pub fn change_password(_s: Shell, _current: String, _new: String) {}
    pub fn restore_session(_s: Shell) -> bool {
        false
    }
    pub fn open_server(_s: Shell, _gid: String) {}
    pub fn open_channel(_s: Shell, _ch: ChannelSummary) {}
    pub fn create_server(_s: Shell, _name: String) {}
    pub fn create_channel(_s: Shell, _name: String) {}
    pub fn invite_member(_s: Shell, _gid: String, _username: String) {}
    pub fn rename_server(_s: Shell, _gid: String, _name: String) {}
    pub fn rename_channel(_s: Shell, _gid: String, _cid: String, _name: String) {}
    pub fn send_message(_s: Shell) {}
    pub fn add_compose_attachment(_s: Shell) {}
    pub fn remove_compose_attachment(_s: Shell, _id: String) {}
    pub fn edit_message(_s: Shell, _cid: String, _mid: String, _body: String) {}
    pub fn delete_message(_s: Shell, _cid: String, _mid: String) {}
    pub fn confirm_delete_message_enabled() -> bool {
        true
    }
    pub fn set_confirm_delete_message(_on: bool) {}
    pub fn ask_delete(_s: Shell, _prompt: String, _pending: PendingDelete) {}
    pub fn cancel_delete(_s: Shell) {}
    // Mirrors the hydrate dispatch shape so the per-action stubs stay "used".
    pub fn confirm_delete(s: Shell) {
        match None::<PendingDelete> {
            Some(PendingDelete::Message { cid, mid }) => delete_message(s, cid, mid),
            Some(PendingDelete::Channel { gid, cid }) => delete_channel(s, gid, cid),
            Some(PendingDelete::Server { gid }) => delete_server(s, gid),
            Some(PendingDelete::Persona { pid }) => remove_persona(s, pid),
            None => {}
        }
    }
    pub fn delete_channel(_s: Shell, _gid: String, _cid: String) {}
    pub fn delete_server(_s: Shell, _gid: String) {}
    pub fn show_friends(_s: Shell) {}
    pub fn show_wardrobe(_s: Shell) {}
    pub fn create_persona(_s: Shell, _name: String, _desc: String) {}
    pub fn update_persona(
        _s: Shell,
        _pid: String,
        _name: String,
        _description: String,
        _color: String,
        _done: RwSignal<bool>,
    ) {
    }
    pub fn remove_persona(_s: Shell, _pid: String) {}
    pub fn leave_shared_persona(_s: Shell, _pid: String) {}
    pub fn load_persona_sharing(
        _s: Shell,
        _pid: String,
        _friends: RwSignal<Vec<crate::protocol::FriendSummary>>,
        _editors: RwSignal<Vec<crate::protocol::PersonaEditor>>,
    ) {
    }
    pub fn set_persona_share(
        _s: Shell,
        _pid: String,
        _aid: String,
        _share: bool,
        _editors: RwSignal<Vec<crate::protocol::PersonaEditor>>,
    ) {
    }
    pub fn set_persona_avatar(_s: Shell, _pid: String) {}
    pub fn wear_persona(_s: Shell, _pid: String) {}
    pub fn unwear(_s: Shell) {}
    pub fn add_friend(_s: Shell, _username: String) {}
    pub fn accept_friend(_s: Shell, _aid: String) {}
    pub fn remove_friend(_s: Shell, _aid: String) {}
    pub fn create_lore(_s: Shell, _cid: String, _keys: Vec<String>, _content: String) {}
    #[allow(clippy::too_many_arguments)]
    pub fn patch_lore(
        _s: Shell,
        _cid: String,
        _eid: String,
        _title: Option<String>,
        _keys: Option<Vec<String>>,
        _content: Option<String>,
        _enabled: Option<bool>,
        _position: Option<i64>,
    ) {
    }
    pub fn swap_lore(_s: Shell, _cid: String, _eid: String, _position: i64, _up: bool) {}
    pub fn delete_lore(_s: Shell, _cid: String, _eid: String) {}
    pub fn submit_feedback(
        _s: Shell,
        _kind: String,
        _body: String,
        _context: Option<String>,
        _modal_open: leptos::prelude::RwSignal<bool>,
    ) {
    }
    pub fn build_feedback_context(_s: Shell) -> Option<String> {
        None
    }
    // Trash (#22 Phase 2) stubs
    pub fn load_deleted_guilds(_s: Shell) {}
    pub fn restore_deleted_guild(_s: Shell, _gid: String) {}
    pub fn load_deleted_channels(_s: Shell, _gid: String) {}
    pub fn restore_channel(_s: Shell, _gid: String, _cid: String) {}
    pub fn load_deleted_messages(_s: Shell, _cid: String) {}
    pub fn restore_deleted_message(_s: Shell, _cid: String, _mid: String) {}
}
