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

use std::collections::HashSet;

use leptos::prelude::*;

use crate::protocol::{
    ChannelSummary, GuildSummary, ListFriendsResponse, LorebookEntry, MessageEnvelope,
    PersonaSummary,
};
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
    seen: RwSignal<HashSet<String>>,
    compose: RwSignal<String>,
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
        seen: RwSignal::new(HashSet::new()),
        compose: RwSignal::new(String::new()),
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
    };
    let new_server = RwSignal::new(String::new());
    let new_channel = RwSignal::new(String::new());
    let new_invite = RwSignal::new(String::new());
    // Account-management modal visibility (change password, future options).
    let account_open = RwSignal::new(false);
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
                            view! {
                                <button class="row-edit"
                                    title=move || if s.muted.get().contains(&cid_t) { "Unmute channel" } else { "Mute channel" }
                                    on:click=move |_| act::toggle_mute(s, cid.clone())>
                                    {move || if s.muted.get().contains(&cid_b) { "🔕" } else { "🔔" }}
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
                    let start_cid = cid.clone();
                    let start_name = name0.clone();
                    view! {
                        <button class="channel" class:active=active
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
            s.seen.update(|h| h.clear());
            start_poll(s);
            spawn_local(async move {
                if let Ok(l) = api::list_messages(&cid, None).await {
                    ingest(s, l.messages);
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
        if body.trim().is_empty() {
            return;
        }
        s.compose.set(String::new());
        s.status.set(String::new());
        // Sending is a user gesture — a reliable point to request notification
        // permission so background channels can notify later.
        request_notify_permission();
        spawn_local(async move {
            match api::post_message(&ch.id, &body).await {
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

    pub fn delete_lore(s: Shell, cid: String, eid: String) {
        spawn_local(async move {
            let _ = api::delete_lore(&cid, &eid).await;
            load_lore(s, cid);
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

    /// Ask for Web Notification permission if undecided. Must run from a user
    /// gesture or the browser may ignore it.
    fn request_notify_permission() {
        use leptos::web_sys::{Notification, NotificationPermission};
        if Notification::permission() == NotificationPermission::Default {
            let _ = Notification::request_permission();
        }
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
        let _ = Notification::new(&title);
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
    pub fn delete_lore(_s: Shell, _cid: String, _eid: String) {}
}
