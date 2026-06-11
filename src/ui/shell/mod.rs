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

use crate::protocol::{ChannelSummary, ListFriendsResponse};

// Trash DTOs reused from protocol (no new types needed — server returns the
// existing GuildSummary / ChannelSummary / MessageEnvelope shapes for trash too).
use crate::ui::avatar::monogram;
use crate::ui::emoji::EmojiResolver;
use crate::ui::icons::{IconChat, IconFriends, IconPersonas, IconServers};
use crate::ui::inline_rename::InlineRename;
use crate::ui::modal::Modal;
use crate::ui::AuthCtx;

mod account;
mod channel;
mod emoji_manager;
mod friends;
mod lorebook;
mod members;
mod state;
mod wardrobe;

#[cfg(feature = "hydrate")]
pub(crate) use state::COMPOSER_MAX_ATTACHMENTS;
pub(crate) use state::{
    Composer, MessageView, Modals, Notify, Prefs, Selection, Social, SyncState, Trash,
};

use account::AccountModal;
use channel::{ChannelManagerModal, ChannelPane};
use emoji_manager::EmojiManagerPane;
use friends::FriendsPane;
use lorebook::LorebookPane;
use members::MembersPane;
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
pub(crate) enum Pane {
    Friends,
    Channel,
    Lorebook,
    Emoji,
    Members,
}

/// A destructive action awaiting confirmation. Stored in `Shell::pending_delete`
/// (with a human prompt in `confirm_prompt`); the top-level confirm modal in
/// `AppShell` dispatches the matching `act::` fn when the user confirms. Storing
/// a closure in a signal is awkward in Leptos, so we describe the action as data.
#[derive(Clone)]
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(crate) enum PendingDelete {
    Message { cid: String, mid: String },
    Channel { gid: String, cid: String },
    Server { gid: String },
    Persona { pid: String },
}

/// Aggregate of the shell's reactive state, grouped into 9 sub-structs.
///
/// Each sub-struct is also `provide_context`'d in `AppShell` so a deeper
/// component can pull just the slice it needs via `use_context::<Selection>()`
/// (the pane-component migration in W6/C8). `act::*` keeps taking the full
/// aggregate handle so action functions stay short and uncluttered.
#[derive(Clone, Copy)]
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(crate) struct Shell {
    pub(crate) sel: Selection,
    pub(crate) msg: MessageView,
    pub(crate) composer: Composer,
    pub(crate) sync: SyncState,
    pub(crate) social: Social,
    pub(crate) modals: Modals,
    pub(crate) notify: Notify,
    pub(crate) trash: Trash,
    pub(crate) prefs: Prefs,
}

#[component]
fn AppShell() -> impl IntoView {
    let auth = use_context::<AuthCtx>().expect("AuthCtx provided at root");

    // Construct each state sub-struct, then make each available via
    // `provide_context` so pane components (W6/C8) can pull just the slice
    // they need without taking the full Shell aggregate as a prop.
    let sel = Selection {
        guilds: RwSignal::new(Vec::new()),
        sel_server: RwSignal::new(None),
        sel_owner: RwSignal::new(None),
        channels: RwSignal::new(Vec::new()),
        guild_channels: RwSignal::new(HashMap::new()),
        guild_emoji: RwSignal::new(Vec::new()),
        sel_channel: RwSignal::new(None),
    };
    provide_context(sel);

    let msg = MessageView {
        messages: RwSignal::new(Vec::new()),
        cursor: RwSignal::new(None),
        oldest: RwSignal::new(None),
        loading_older: RwSignal::new(false),
        more_history: RwSignal::new(true),
        loading_initial: RwSignal::new(false),
        anchor_to: RwSignal::new(None),
        seen: RwSignal::new(HashSet::new()),
        typing: RwSignal::new(Vec::new()),
    };
    provide_context(msg);

    let composer = Composer {
        compose: RwSignal::new(String::new()),
        compose_attachments: RwSignal::new(Vec::new()),
        status: RwSignal::new(String::new()),
        drafts: RwSignal::new(crate::ui::shell::act::channel::load_drafts()),
        last_used_colors: RwSignal::new(act::load_color_history()),
        replying_to: RwSignal::new(None),
        editing: RwSignal::new(None),
    };
    provide_context(composer);

    let sync = SyncState {
        polling: RwSignal::new(false),
        me: RwSignal::new(auth.user.get_untracked().map(|u| u.account_id)),
        pane: RwSignal::new(Pane::Friends),
        sheet_open: RwSignal::new(false),
        wardrobe_open: RwSignal::new(false),
    };
    provide_context(sync);

    let social = Social {
        friends: RwSignal::new(ListFriendsResponse {
            friends: Vec::new(),
            incoming: Vec::new(),
            outgoing: Vec::new(),
        }),
        personas: RwSignal::new(Vec::new()),
        active_persona: RwSignal::new(None),
        lore: RwSignal::new(Vec::new()),
    };
    provide_context(social);

    let modals = Modals {
        pending_delete: RwSignal::new(None),
        confirm_prompt: RwSignal::new(None),
    };
    provide_context(modals);

    let notify = Notify {
        muted: RwSignal::new(HashSet::new()),
        unread: RwSignal::new(HashSet::new()),
        pinged: RwSignal::new(HashSet::new()),
        unread_count: RwSignal::new(HashMap::new()),
        unread_guilds: RwSignal::new(HashSet::new()),
        last_seen: RwSignal::new(HashMap::new()),
        web_push_enabled: RwSignal::new(false),
    };
    provide_context(notify);

    let trash = Trash {
        deleted_guilds: RwSignal::new(Vec::new()),
        deleted_channels: RwSignal::new(Vec::new()),
        deleted_messages: RwSignal::new(Vec::new()),
        show_msg_trash: RwSignal::new(false),
    };
    provide_context(trash);

    let prefs = Prefs {
        dialogue_style: RwSignal::new(act::rp_dialogue_style_enabled()),
        eyecandy: RwSignal::new(act::eyecandy_enabled()),
    };
    provide_context(prefs);

    let s = Shell {
        sel,
        msg,
        composer,
        sync,
        social,
        modals,
        notify,
        trash,
        prefs,
    };
    // Make the aggregate available to pane components (W6/C8) so they can drop
    // their `s: Shell` prop in favour of `use_context::<Shell>()`.
    provide_context(s);

    // Keep `s.sync.me` in sync with the auth context (it resolves async after mount).
    Effect::new(move |_| {
        s.sync.me.set(auth.user.get().map(|u| u.account_id));
    });
    // Provide the emoji resolver to the whole shell subtree so the markup
    // renderer turns `:shortcode:` into a custom-emoji image or a unicode glyph
    // without threading a parameter through every render call site.
    let emoji_map = Memo::new(move |_| {
        s.sel
            .guild_emoji
            .get()
            .into_iter()
            .map(|e| (e.name, e.media_id))
            .collect::<HashMap<String, String>>()
    });
    provide_context(EmojiResolver::new(emoji_map));
    // W7/D1: kick off the lazy `/emoji.json` fetch at shell mount so the
    // picker and `:shortcode:` resolver are warm by the time the first
    // composer renders. No-op if already loaded or in flight.
    crate::ui::emoji::data::warm();
    let new_server = RwSignal::new(String::new());
    let new_channel = RwSignal::new(String::new());
    // Channel-creator dialog: open/closed + the chosen kind (text|lorebook;
    // the extension point where Gallery lands later).
    let new_channel_kind = RwSignal::new("text".to_string());
    let channel_creator_open = RwSignal::new(false);
    let new_invite = RwSignal::new(String::new());
    // Account-management modal visibility (change password, future options).
    let account_open = RwSignal::new(false);
    // Guild-trash panel open/closed (rail trash button toggles it).
    let guild_trash_open = RwSignal::new(false);
    // Deleted-channel list open/closed in the sidebar (owner-only).
    let chan_trash_open = RwSignal::new(false);
    // L-5: the unified channel-management window (create/rename/delete/reorder),
    // opened from the owner-gated "⚙ Manage" button in the server header.
    let channel_manager_open = RwSignal::new(false);
    // Inline-rename edit state for the server title (owner only). The edit
    // buffer lives INSIDE `<InlineRename>` (W6/C7); this signal just gates
    // whether the input is rendered. (The per-channel equivalent moved into
    // `ChannelList` with the W3/T5 extraction, alongside the drag-reorder
    // source indices — each list instance owns its own.)
    let editing_server = RwSignal::new(false);
    // The invite/manage controls show only to the owner of the open server.
    let is_owner = move || {
        let me = auth.user.get().map(|u| u.account_id);
        me.is_some() && me == s.sel.sel_owner.get()
    };
    // The open server's name, derived from the rail list (auto-updates on rename).
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

    // A notification deep-link arrives as `/?server=&channel=&m=` (set by the
    // service worker's notificationclick handler). Read it once at mount; when
    // present it wins over the stored-session restore. Router-driven, so it
    // needs no extra web-sys features. None on ssr (the Effect is client-only).
    #[cfg(feature = "hydrate")]
    let deep_link: Option<(String, String, Option<String>)> = {
        let q = leptos_router::hooks::use_query_map().get_untracked();
        match (q.get("channel"), q.get("server")) {
            (Some(cid), Some(gid)) => Some((gid, cid, q.get("m"))),
            _ => None,
        }
    };
    #[cfg(not(feature = "hydrate"))]
    let deep_link: Option<(String, String, Option<String>)> = None;
    #[cfg(feature = "hydrate")]
    let nav = leptos_router::hooks::use_navigate();

    // On mount: load the guild rail, then either follow a notification
    // deep-link or restore the last session (falling back to the Friends home).
    // A deep-linked/restored channel wins; we don't show Friends over it.
    // (No-ops on ssr; the stub `restore_session` returns false so ssr still
    // lands on Friends.)
    Effect::new(move |_| {
        act::refresh_guilds(s);
        if let Some((gid, cid, message)) = deep_link.clone() {
            act::open_deep_link(s, gid, cid, message);
            // Strip the query so a manual refresh doesn't yank us back here.
            #[cfg(feature = "hydrate")]
            nav(
                "/",
                leptos_router::NavigateOptions {
                    replace: true,
                    scroll: false,
                    ..Default::default()
                },
            );
        } else if !act::restore_session(s) {
            act::show_friends(s);
        }
        // Keep the rail/sidebar/friends + open channel live (idempotent).
        act::start_sync(s);
        act::load_muted(s);
        // Load the offline localStorage marks first, then overlay the
        // server-synced read cursors on top (L-1 cross-device sync): a newer
        // server cursor wins, a failed fetch falls back to localStorage.
        act::load_last_seen(s);
        act::hydrate_last_seen(s);
        // Window-focus listener: when the user returns to the tab with a
        // channel already open, clear any tray notifications that arrived
        // for that channel while we were backgrounded (feedback row
        // kx24k2cwftdppidhmh0e).
        #[cfg(feature = "hydrate")]
        act::wire_focus_clears_notifs(s);
        // SW message listener: a push notification clicked from a backgrounded
        // PWA routes via the SW's `client.navigate()`, which throws in some
        // standalone contexts; its fallback posts a NOTIFICATION_CLICK message
        // to this window. Register the listener so that payload deep-links the
        // app instead of being silently dropped (feedback br3ebxgjj1lh3qfbz3n8).
        #[cfg(feature = "hydrate")]
        act::wire_notification_click(s);
    });

    let username = move || auth.user.get().map(|u| u.username).unwrap_or_default();

    view! {
        <div class="app" class:dialogue-style=move || s.prefs.dialogue_style.get() class:fx-max=move || s.prefs.eyecandy.get()>
            <nav class="rail">
                <button class="rail-home" title="Friends"
                    on:click=move |_| act::show_friends(s)>"@"</button>
                <RailGuilds/>
                // Guild trash button — loads + opens the deleted-guilds panel.
                <button class="rail-trash" title="Trashed servers"
                    class:active=move || guild_trash_open.get()
                    on:click=move |_| {
                        let now_open = !guild_trash_open.get_untracked();
                        guild_trash_open.set(now_open);
                        if now_open {
                            act::load_deleted_guilds(s);
                        }
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
                let guilds = s.trash.deleted_guilds.get();
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
                <Show when=move || s.sel.sel_server.get().is_some()
                    fallback=|| view! {
                        <p class="muted pad">"Pick or create a server, or visit Friends (@)."</p>
                    }>
                    <div class="server-header">
                        {move || if editing_server.get() {
                            view! {
                                <InlineRename
                                    value=server_name()
                                    on_save=move |v| {
                                        if let Some(gid) = s.sel.sel_server.get_untracked() {
                                            act::rename_server(s, gid, v);
                                        }
                                        editing_server.set(false);
                                    }
                                    on_cancel=move || editing_server.set(false)
                                />
                            }.into_any()
                        } else {
                            view! {
                                <span class="server-title">{server_name()}</span>
                                <Show when=is_owner fallback=|| ()>
                                    // L-5: open the unified channel-management
                                    // window (create/rename/delete/reorder).
                                    <button class="row-edit" title="Manage channels"
                                        on:click=move |_| channel_manager_open.set(true)>"⚙"</button>
                                    <button class="row-edit" title="rename server"
                                        on:click=move |_| editing_server.set(true)>"✎"</button>
                                    <button class="row-edit danger" title="delete server"
                                        on:click=move |_| {
                                            if let Some(gid) = s.sel.sel_server.get_untracked() {
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
                        on:click=move |_| act::show_wardrobe(s)>
                        "🎭 Wardrobe"
                    </button>
                    <button class="wardrobe-btn"
                        on:click=move |_| act::show_emoji_manager(s)>
                        "😀 Emoji"
                    </button>
                    <button class="wardrobe-btn"
                        on:click=move |_| act::show_members(s)>
                        "👥 Members"
                    </button>
                    <ChannelList/>
                    <Show when=is_owner fallback=|| ()>
                        <div class="channel-add">
                            <button class="channel-add-btn" title="New channel"
                                on:click=move |_| {
                                    new_channel.set(String::new());
                                    new_channel_kind.set("text".to_string());
                                    channel_creator_open.set(true);
                                }>"＋ Channel"</button>
                        </div>
                    </Show>
                    // Channel-creator dialog (opened only via the owner-gated
                    // button above): choose the channel type + name. The lorebook
                    // kind is fully wired (LorebookPane); this dialog is also where
                    // a Gallery kind will be added later (R2).
                    {move || channel_creator_open.get().then(|| view! {
                        <Modal class="channel-creator"
                            close=move || channel_creator_open.set(false)>
                            <h3>"New channel"</h3>
                            <div class="creator-kind">
                                <label class="pref-row">
                                    <input type="radio" name="ch-kind" value="text"
                                        prop:checked=move || new_channel_kind.get() == "text"
                                        on:change=move |_| new_channel_kind.set("text".to_string())/>
                                    <span>"# Text"</span>
                                </label>
                                <label class="pref-row">
                                    <input type="radio" name="ch-kind" value="lorebook"
                                        prop:checked=move || new_channel_kind.get() == "lorebook"
                                        on:change=move |_| new_channel_kind.set("lorebook".to_string())/>
                                    <span>"📖 Lorebook"</span>
                                </label>
                            </div>
                            <input class="creator-name" prop:value=move || new_channel.get()
                                on:input=move |ev| new_channel.set(event_target_value(&ev))
                                placeholder="channel name"/>
                            <div class="creator-actions">
                                <button on:click=move |_| channel_creator_open.set(false)>
                                    "Cancel"
                                </button>
                                <button class="account-save" on:click=move |_| {
                                    let name = new_channel.get_untracked();
                                    if name.trim().is_empty() {
                                        return;
                                    }
                                    let kind = new_channel_kind.get_untracked();
                                    new_channel.set(String::new());
                                    channel_creator_open.set(false);
                                    act::create_channel(s, name, kind);
                                }>"Create"</button>
                            </div>
                        </Modal>
                    })}
                    // L-5: the unified channel-management window. Owner-gated
                    // open (the server re-checks require_manager on every
                    // mutate, so the gate is defence-in-depth, not the boundary).
                    {move || (channel_manager_open.get() && is_owner()).then(|| view! {
                        <ChannelManagerModal s=s open=channel_manager_open/>
                    })}
                    // Deleted-channels panel (owner only).
                    <Show when=is_owner fallback=|| ()>
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
                                "🗑 Trashed channels"
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
                    </Show>
                    <Show when=is_owner fallback=|| ()>
                        <div class="invite-row">
                            <input prop:value=move || new_invite.get()
                                on:input=move |ev| new_invite.set(event_target_value(&ev))
                                placeholder="invite by username"/>
                            <button on:click=move |_| {
                                let gid = s.sel.sel_server.get_untracked();
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
                    // Mobile fast-switch (W3/T5, spec §2): tapping the channel
                    // name opens the channel sheet; the ▾ is the affordance.
                    // CSS hides this on desktop (the sidebar is the switcher).
                    {move || s.sel.sel_channel.get()
                        .filter(|_| s.sync.pane.get() == Pane::Channel)
                        .map(|c| {
                            let sigil = if c.kind == "lorebook" { "📖 " } else { "# " };
                            view! {
                                <button class="chan-trigger" title="Switch channel"
                                    on:click=move |_| s.sync.sheet_open.set(true)>
                                    <span class="chan-trigger-name">{sigil}{c.name}</span>
                                    <span class="chan-trigger-caret" aria-hidden="true">"▾"</span>
                                </button>
                            }
                        })}
                    <span class="muted">"Signed in as " <strong>{username}</strong></span>
                    // Mute toggle for the open channel (suppresses its
                    // new-message notifications); 🔔 = active, 🔕 = muted.
                    {move || s.sel.sel_channel.get()
                        .filter(|_| s.sync.pane.get() == Pane::Channel)
                        .map(|c| {
                            let cid = c.id.clone();
                            let cid_t = c.id.clone();
                            let cid_b = c.id.clone();
                            let cid_trash = c.id.clone();
                            view! {
                                <button class="row-edit"
                                    title=move || if s.notify.muted.get().contains(&cid_t) { "Unmute channel" } else { "Mute channel" }
                                    on:click=move |_| act::toggle_mute(s, cid.clone())>
                                    {move || if s.notify.muted.get().contains(&cid_b) { "🔕" } else { "🔔" }}
                                </button>
                                // Trash toggle: load and show deleted messages in this channel.
                                <button class="row-edit"
                                    title=move || if s.trash.show_msg_trash.get() { "Hide deleted" } else { "Show deleted" }
                                    on:click=move |_| {
                                        let now_open = !s.trash.show_msg_trash.get_untracked();
                                        s.trash.show_msg_trash.set(now_open);
                                        if now_open {
                                            act::load_deleted_messages(s, cid_trash.clone());
                                        } else {
                                            s.trash.deleted_messages.set(Vec::new());
                                        }
                                    }>
                                    {move || if s.trash.show_msg_trash.get() { "🗑✓" } else { "🗑" }}
                                </button>
                            }
                        })
                    }
                    <span class="spacer"></span>
                    <button title="Account"
                        on:click=move |_| { s.composer.status.set(String::new()); account_open.set(true); }>
                        "⚙"
                    </button>
                    <button on:click=move |_| act::logout(auth)>"Log out"</button>
                </header>
                {move || match s.sync.pane.get() {
                    Pane::Friends => view! { <FriendsPane/> }.into_any(),
                    Pane::Channel => view! { <ChannelPane/> }.into_any(),
                    Pane::Lorebook => view! { <LorebookPane/> }.into_any(),
                    Pane::Emoji => view! { <EmojiManagerPane/> }.into_any(),
                    Pane::Members => view! { <MembersPane/> }.into_any(),
                }}
                <p class="error">{move || s.composer.status.get()}</p>
            </section>

            // Mobile-only bottom tab bar (W3/T5; CSS hides it on desktop).
            // Chat returns to the channel pane, Servers opens the channel-
            // switch sheet (the sheet IS the server+channel switcher),
            // Friends/Personas reuse the existing pane/modal actions. Every
            // non-Servers tab also dismisses the sheet so a tab tap never
            // lands "behind" it.
            <nav class="bottom-tabs">
                <button class="tab"
                    class:active=move || s.sync.pane.get() == Pane::Channel && !s.sync.sheet_open.get()
                    on:click=move |_| {
                        s.sync.sheet_open.set(false);
                        s.sync.pane.set(Pane::Channel);
                    }>
                    <IconChat class="tab-icon"/>
                    <span class="tab-label">"Chat"</span>
                    // Aggregate unread dot: any guild has a channel with
                    // messages past the user's last-seen mark.
                    {move || (!s.notify.unread_guilds.get().is_empty())
                        .then(|| view! { <span class="tab-dot"></span> })}
                </button>
                <button class="tab"
                    class:active=move || s.sync.sheet_open.get()
                    on:click=move |_| s.sync.sheet_open.set(true)>
                    <IconServers class="tab-icon"/>
                    <span class="tab-label">"Servers"</span>
                </button>
                <button class="tab"
                    class:active=move || s.sync.pane.get() == Pane::Friends && !s.sync.sheet_open.get()
                    on:click=move |_| {
                        s.sync.sheet_open.set(false);
                        act::show_friends(s);
                    }>
                    <IconFriends class="tab-icon"/>
                    <span class="tab-label">"Friends"</span>
                </button>
                <button class="tab"
                    class:active=move || s.sync.wardrobe_open.get()
                    on:click=move |_| {
                        s.sync.sheet_open.set(false);
                        act::show_wardrobe(s);
                    }>
                    <IconPersonas class="tab-icon"/>
                    <span class="tab-label">"Personas"</span>
                </button>
            </nav>

            // Channel-switch bottom sheet (W3/T5): a glass sheet over its own
            // scrim, mobile-only via CSS. Reuses the SAME RailGuilds /
            // ChannelList components the desktop columns render. Tapping the
            // backdrop dismisses; tapping a channel row switches AND
            // dismisses (wired in ChannelRow). Tapping a guild keeps the
            // sheet open so a channel can be picked next. The DM "Direct"
            // slot lands in W6; drag-down-to-close is a later polish — the
            // backdrop tap is the W3 dismissal floor.
            {move || s.sync.sheet_open.get().then(|| view! {
                <div class="sheet-backdrop" on:click=move |_| s.sync.sheet_open.set(false)></div>
                <div class="channel-sheet" role="dialog" aria-label="Switch channel">
                    <div class="sheet-handle" aria-hidden="true"></div>
                    <div class="sheet-guilds">
                        <RailGuilds/>
                    </div>
                    <div class="sheet-channels">
                        <Show when=move || s.sel.sel_server.get().is_some()
                            fallback=|| view! {
                                <p class="muted pad">"Pick a server above."</p>
                            }>
                            <ChannelList/>
                        </Show>
                    </div>
                </div>
            })}

            {move || if account_open.get() {
                view! { <AccountModal s=s open=account_open/> }.into_any()
            } else {
                ().into_any()
            }}

            // Wardrobe popup (F-2): a dismissible Modal — backdrop click, Esc, or
            // the X close it. Auto-closes when a channel is opened (act::open_channel
            // clears `wardrobe_open`). The wide variant widens the dialog for the
            // persona grid; nested modals inside `WardrobePane` (detail editor, info
            // popup) keep their own `stop_propagation` so inner backdrop clicks only
            // dismiss the inner modal, not this one.
            {move || s.sync.wardrobe_open.get().then(|| {
                view! {
                    <Modal class="wardrobe-modal" close=move || s.sync.wardrobe_open.set(false)>
                        <button class="modal-x" title="close" aria-label="Close wardrobe"
                            on:click=move |_| s.sync.wardrobe_open.set(false)>"✕"</button>
                        <WardrobePane/>
                    </Modal>
                }
            })}

            // Top-level confirm dialog for destructive actions. Shown whenever a
            // `PendingDelete` is queued; backdrop/Cancel clears it without acting,
            // "Delete" dispatches the queued action (see `act::confirm_delete`).
            {move || s.modals.pending_delete.get().is_some().then(|| {
                let prompt = s.modals.confirm_prompt.get().unwrap_or_default();
                view! {
                    <Modal class="confirm-modal" close=move || act::cancel_delete(s)>
                        <h3>"Confirm delete"</h3>
                        <p>{prompt}</p>
                        <div class="confirm-actions">
                            <button on:click=move |_| act::cancel_delete(s)>"Cancel"</button>
                            <button class="danger"
                                on:click=move |_| act::confirm_delete(s)>"Delete"</button>
                        </div>
                    </Modal>
                }
            })}
        </div>
    }
}

/// The guild-circle list (W3/T5 extraction): active ring, per-guild unread
/// dot, click-to-open, hover ↑/↓/⤒/⤓ reorder controls, and HTML5
/// drag-to-reorder (L-5). Rendered by BOTH the desktop rail (vertical) and
/// the mobile channel-sheet's server strip (horizontal; it hides the reorder
/// chrome in CSS) so the two stay one source. Owns its drag-source index, so
/// concurrent instances can't observe each other's drags. Pulls [`Shell`]
/// from context (the W6/C8 pattern the pane components use).
#[component]
fn RailGuilds() -> impl IntoView {
    let s = use_context::<Shell>().expect("Shell provided by AppShell");
    // L-5: index of the guild currently being dragged (HTML5 DnD), or None
    // between drags. Set on dragstart, read on drop, cleared on dragend/drop.
    let drag_from = RwSignal::new(None::<usize>);
    view! {
        {move || {
            let guilds = s.sel.guilds.get();
            let len = guilds.len();
            // `len`/`idx`/`drag_from` feed the reorder `disabled` closures and
            // drag handlers, which the `view!` macro strips on ssr — silence
            // the unused warning.
            let _ = (len, &drag_from);
            guilds.into_iter().enumerate().map(|(idx, g)| {
                let gid = g.id.clone();
                let initial = monogram(&g.name, '#');
                let gid_active = gid.clone();
                let gid_unread = gid.clone();
                view! {
                    // Drag-to-reorder (HTML5): the wrap is draggable;
                    // dragstart records this index, dragover allows the
                    // drop, drop moves the dragged guild here (L-5).
                    <div class="rail-guild-wrap" draggable="true"
                        on:dragstart=move |_ev| drag_from.set(Some(idx))
                        on:dragover=move |_ev| {
                            #[cfg(feature = "hydrate")] _ev.prevent_default();
                        }
                        on:drop=move |_ev| {
                            #[cfg(feature = "hydrate")] {
                                _ev.prevent_default();
                                if let Some(from) = drag_from.get_untracked() {
                                    act::move_guild(s, from, idx);
                                }
                                drag_from.set(None);
                            }
                        }
                        on:dragend=move |_ev| drag_from.set(None)>
                        <button class="rail-guild" title=g.name
                            class:active=move || s.sel.sel_server.get().as_deref() == Some(gid_active.as_str())
                            class:unread=move || act::guild_has_unread(s, &gid_unread)
                            on:click=move |_| act::open_server(s, gid.clone())>
                            {initial}
                        </button>
                        // Personal rail reorder (#17/FB2 + L-5): ↑/↓ swap
                        // a neighbour, ⤒/⤓ bring to top/bottom. ↑/⤒
                        // disabled on the first guild, ↓/⤓ on the last.
                        <div class="rail-reorder">
                            <button class="rail-reorder-btn" title="Move up"
                                disabled=move || idx == 0
                                on:click=move |_| act::swap_guild(s, idx, true)>"↑"</button>
                            <button class="rail-reorder-btn" title="Move down"
                                disabled=move || idx == len.saturating_sub(1)
                                on:click=move |_| act::swap_guild(s, idx, false)>"↓"</button>
                            <button class="rail-reorder-btn" title="Bring to top"
                                disabled=move || idx == 0
                                on:click=move |_| act::move_guild_to_bounds(s, idx, true)>"⤒"</button>
                            <button class="rail-reorder-btn" title="Bring to bottom"
                                disabled=move || idx == len.saturating_sub(1)
                                on:click=move |_| act::move_guild_to_bounds(s, idx, false)>"⤓"</button>
                        </div>
                    </div>
                }
            }).collect_view()
        }}
    }
}

/// The channel-row list (W3/T5 extraction): the `<ul class="channels">` of
/// [`ChannelRow`]s. Rendered by BOTH the desktop sidebar and the mobile
/// channel-sheet (which hides the owner management chrome in CSS) so the two
/// stay one source. Owns the shared-across-rows inline-rename target and the
/// drag-reorder source index — per instance, so the sidebar's edit state
/// never leaks into the sheet's. Pulls [`Shell`] from context.
#[component]
fn ChannelList() -> impl IntoView {
    let s = use_context::<Shell>().expect("Shell provided by AppShell");
    // Which channel id is being inline-renamed (owner only), if any. The
    // rename draft buffer lives inside `<InlineRename>` itself (W6/C7).
    let editing = RwSignal::new(None::<String>);
    // L-5: index of the channel row currently being dragged (shared across
    // rows so the drop-target row can read which row started the drag).
    let drag_from = RwSignal::new(None::<usize>);
    view! {
        <ul class="channels">
            {move || {
                let chans = s.sel.channels.get();
                let len = chans.len();
                chans.into_iter().enumerate().map(|(idx, c)| {
                    view! { <ChannelRow s=s ch=c idx=idx len=len editing=editing drag_from=drag_from/> }
                }).collect_view()
            }}
        </ul>
    }
}

/// One channel row in the sidebar: the open-channel button, plus an owner-only
/// inline rename (✎ → input + ✓/✕). Edit state is shared across rows via the
/// `editing` (which cid, if any) signal owned by [`ChannelList`]; the rename
/// draft buffer lives inside `<InlineRename>` itself (W6/C7).
#[component]
fn ChannelRow(
    s: Shell,
    ch: ChannelSummary,
    idx: usize,
    len: usize,
    editing: RwSignal<Option<String>>,
    // L-5: the shared drag-source index for HTML5 drag-to-reorder (owned by
    // `AppShell`). `None` between drags.
    drag_from: RwSignal<Option<usize>>,
) -> impl IntoView {
    let auth = use_context::<AuthCtx>().expect("AuthCtx provided at root");
    let is_owner = move || {
        let me = auth.user.get().map(|u| u.account_id);
        me.is_some() && me == s.sel.sel_owner.get()
    };
    // `idx`/`len`/`drag_from` feed the reorder buttons' `disabled` closures and
    // the drag handlers, which the `view!` macro strips on ssr — silence the
    // ssr-side unused warning the same way wardrobe.rs does.
    let _ = (idx, len, drag_from);
    let cid = ch.id.clone();
    let name0 = ch.name.clone();
    let sigil = if ch.kind == "lorebook" { "📖 " } else { "# " };
    view! {
        // Drag-to-reorder (owner only in practice; the server re-checks).
        // dragstart records this row, dragover allows the drop, drop moves the
        // dragged channel to this index (L-5).
        <li draggable="true"
            on:dragstart=move |_ev| drag_from.set(Some(idx))
            on:dragover=move |_ev| {
                #[cfg(feature = "hydrate")] _ev.prevent_default();
            }
            on:drop=move |_ev| {
                #[cfg(feature = "hydrate")] {
                    _ev.prevent_default();
                    if let Some(from) = drag_from.get_untracked() {
                        act::move_channel(s, from, idx);
                    }
                    drag_from.set(None);
                }
            }
            on:dragend=move |_ev| drag_from.set(None)>
            {move || {
                let cid = cid.clone();
                let name0 = name0.clone();
                let ch = ch.clone();
                if editing.get().as_deref() == Some(cid.as_str()) {
                    let save_cid = cid.clone();
                    view! {
                        <InlineRename
                            value=name0.clone()
                            on_save=move |v| {
                                if let Some(gid) = s.sel.sel_server.get_untracked() {
                                    act::rename_channel(s, gid, save_cid.clone(), v);
                                }
                                editing.set(None);
                            }
                            on_cancel=move || editing.set(None)
                        />
                    }.into_any()
                } else {
                    let active_cid = cid.clone();
                    let active = move || s.sel.sel_channel.get().map(|c| c.id) == Some(active_cid.clone());
                    let unread_cid = cid.clone();
                    // White glow for plain unread; orange `pinged` glow WINS when
                    // the channel has a ping for me (L-4). The CSS keys off both
                    // classes — `.pinged` overrides `.unread` styling.
                    let unread = move || s.notify.unread.get().contains(&unread_cid);
                    let pinged_cid = cid.clone();
                    let pinged = move || s.notify.pinged.get().contains(&pinged_cid);
                    // Per-channel unread count badge (L-4): a small pill showing
                    // the number of unread messages, hidden (no badge) at 0. One
                    // reactive closure owning a single cid clone renders either the
                    // badge span or an empty view, so it stays `Fn` (a non-`Copy`
                    // String can't be shared across a separate `when` + children).
                    let badge_cid = cid.clone();
                    let badge = move || {
                        let n = s
                            .notify
                            .unread_count
                            .get()
                            .get(&badge_cid)
                            .copied()
                            .unwrap_or(0);
                        if n == 0 {
                            return ().into_any();
                        }
                        let label = if n > 99 { "99+".to_string() } else { n.to_string() };
                        view! { <span class="channel-badge">{label}</span> }.into_any()
                    };
                    let start_cid = cid.clone();
                    let start_name = name0.clone();
                    view! {
                        <button class="channel" class:active=active class:unread=unread class:pinged=pinged
                            on:click=move |_| {
                                // Picking a channel dismisses the mobile sheet
                                // (W3/T5); a no-op from the desktop sidebar,
                                // where the sheet is never open.
                                s.sync.sheet_open.set(false);
                                act::open_channel(s, ch.clone());
                            }>
                            {sigil}{name0.clone()}
                            {badge}
                        </button>
                        <Show when=is_owner fallback=|| ()>
                            // Reorder (L-5): ↑/↓ swap a neighbour, ⤒/⤓ bring to
                            // top/bottom. ↑/⤒ disabled on the first channel,
                            // ↓/⤓ on the last.
                            <button class="channel-reorder" title="Move up"
                                disabled=move || idx == 0
                                on:click=move |_| act::swap_channel(s, idx, true)>"↑"</button>
                            <button class="channel-reorder" title="Move down"
                                disabled=move || idx == len.saturating_sub(1)
                                on:click=move |_| act::swap_channel(s, idx, false)>"↓"</button>
                            <button class="channel-reorder" title="Bring to top"
                                disabled=move || idx == 0
                                on:click=move |_| act::move_channel_to_bounds(s, idx, true)>"⤒"</button>
                            <button class="channel-reorder" title="Bring to bottom"
                                disabled=move || idx == len.saturating_sub(1)
                                on:click=move |_| act::move_channel_to_bounds(s, idx, false)>"⤓"</button>
                            <button class="row-edit" title="rename channel" on:click={
                                let start_cid = start_cid.clone();
                                move |_| editing.set(Some(start_cid.clone()))
                            }>"✎"</button>
                            <button class="row-edit danger" title="delete channel" on:click={
                                let del_cid = start_cid.clone();
                                let del_name = start_name.clone();
                                move |_| {
                                    if let Some(gid) = s.sel.sel_server.get_untracked() {
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
// ungated and gloo-net never enters the ssr graph). Defined in `act/` so the
// view stays focused on layout and each action cluster lives in its own file.
// ---------------------------------------------------------------------------

mod act;
