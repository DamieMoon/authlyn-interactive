//! The authed Discord-style shell: a server rail, a channel sidebar, and a
//! content pane that switches between channel messages, the lorebook editor,
//! the wardrobe, and the friends list.
//!
//! State is signal-driven (a `Copy` [`Shell`] handle); deep-link URLs are a
//! later polish. All data-fetching lives in the `act` module, defined twice —
//! real on hydrate, no-op stubs on ssr — so the view's handlers call it
//! ungated and the gloo-net client never enters the ssr graph.

use std::collections::HashSet;

use leptos::prelude::*;

use crate::markup::Color;
use crate::protocol::{
    ChannelSummary, GuildSummary, ListFriendsResponse, LorebookEntry, MessageEnvelope,
    PersonaSummary,
};
use crate::ui::markup_view::render_body;
use crate::ui::AuthCtx;

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

/// All of the shell's reactive state, bundled into one `Copy` handle.
#[derive(Clone, Copy)]
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
struct Shell {
    guilds: RwSignal<Vec<GuildSummary>>,
    sel_server: RwSignal<Option<String>>,
    channels: RwSignal<Vec<ChannelSummary>>,
    sel_channel: RwSignal<Option<ChannelSummary>>,
    messages: RwSignal<Vec<MessageEnvelope>>,
    cursor: RwSignal<Option<(String, String)>>,
    seen: RwSignal<HashSet<String>>,
    compose: RwSignal<String>,
    status: RwSignal<String>,
    polling: RwSignal<bool>,
    pane: RwSignal<Pane>,
    friends: RwSignal<ListFriendsResponse>,
    personas: RwSignal<Vec<PersonaSummary>>,
    active_persona: RwSignal<Option<String>>,
    lore: RwSignal<Vec<LorebookEntry>>,
}

#[component]
fn AppShell() -> impl IntoView {
    let auth = use_context::<AuthCtx>().expect("AuthCtx provided at root");
    let s = Shell {
        guilds: RwSignal::new(Vec::new()),
        sel_server: RwSignal::new(None),
        channels: RwSignal::new(Vec::new()),
        sel_channel: RwSignal::new(None),
        messages: RwSignal::new(Vec::new()),
        cursor: RwSignal::new(None),
        seen: RwSignal::new(HashSet::new()),
        compose: RwSignal::new(String::new()),
        status: RwSignal::new(String::new()),
        polling: RwSignal::new(false),
        pane: RwSignal::new(Pane::Friends),
        friends: RwSignal::new(ListFriendsResponse {
            friends: Vec::new(),
            incoming: Vec::new(),
            outgoing: Vec::new(),
        }),
        personas: RwSignal::new(Vec::new()),
        active_persona: RwSignal::new(None),
        lore: RwSignal::new(Vec::new()),
    };
    let new_server = RwSignal::new(String::new());
    let new_channel = RwSignal::new(String::new());

    // On mount: load the guild rail + the friends home. (No-ops on ssr.)
    Effect::new(move |_| {
        act::refresh_guilds(s);
        act::show_friends(s);
    });

    let username = move || auth.user.get().map(|u| u.username).unwrap_or_default();

    view! {
        <div class="app">
            <nav class="rail">
                <button class="rail-home" title="Friends"
                    on:click=move |_| act::show_friends(s)>"@"</button>
                {move || s.guilds.get().into_iter().map(|g| {
                    let gid = g.id.clone();
                    let initial = g.name.chars().next().unwrap_or('#').to_uppercase().to_string();
                    view! {
                        <button class="rail-guild" title=g.name
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
                    <button class="wardrobe-btn" on:click=move |_| act::show_wardrobe(s)>
                        "🎭 Wardrobe"
                    </button>
                    <ul class="channels">
                        {move || s.channels.get().into_iter().map(|c| {
                            let ch = c.clone();
                            let sigil = if c.kind == "lorebook" { "📖 " } else { "# " };
                            let cid = c.id.clone();
                            let active = move || s.sel_channel.get().map(|sc| sc.id) == Some(cid.clone());
                            view! {
                                <li>
                                    <button class="channel" class:active=active
                                        on:click=move |_| act::open_channel(s, ch.clone())>
                                        {sigil}{c.name}
                                    </button>
                                </li>
                            }
                        }).collect_view()}
                    </ul>
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
            </aside>

            <section class="content">
                <header class="topbar">
                    <span class="muted">"Signed in as " <strong>{username}</strong></span>
                    <span class="spacer"></span>
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
        </div>
    }
}

#[component]
fn ChannelPane(s: Shell) -> impl IntoView {
    view! {
        <div class="channel-view">
            <ul class="messages">
                {move || s.messages.get().into_iter().map(|m| {
                    let who = m.persona_name.clone().unwrap_or_else(|| short_id(&m.author_id));
                    view! {
                        <li class="msg">
                            <span class="who">{who}</span>
                            <span class="text">{render_body(&m.body)}</span>
                        </li>
                    }
                }).collect_view()}
            </ul>
            <div class="composer">
                <div class="toolbar">
                    <button class="fmt" title="bold"
                        on:click=move |_| s.compose.update(|c| c.push_str("**bold**"))>
                        <strong>"B"</strong>
                    </button>
                    <button class="fmt" title="italic"
                        on:click=move |_| s.compose.update(|c| c.push_str("*italic*"))>
                        <em>"i"</em>
                    </button>
                    {Color::ALL.into_iter().map(|col| {
                        let name = col.name();
                        view! {
                            <button class=format!("swatch mk-bg-{name}") title=name
                                on:click=move |_| s.compose.update(|c| {
                                    c.push_str(&format!("[{name}]text[/{name}]"));
                                })>
                            </button>
                        }
                    }).collect_view()}
                </div>
                <textarea
                    prop:value=move || s.compose.get()
                    on:input=move |ev| s.compose.set(event_target_value(&ev))
                    on:keydown=move |ev| {
                        #[cfg(feature = "hydrate")]
                        {
                            if ev.key() == "Enter" && !ev.shift_key() {
                                ev.prevent_default();
                                act::send_message(s);
                            }
                        }
                        #[cfg(not(feature = "hydrate"))]
                        let _ = &ev;
                    }
                    placeholder="type a message — **bold**, *italic*, [red]color[/red]"
                ></textarea>
                <button class="send" on:click=move |_| act::send_message(s)>"Send"</button>
            </div>
        </div>
    }
}

#[component]
fn WardrobePane(s: Shell) -> impl IntoView {
    let name = RwSignal::new(String::new());
    let desc = RwSignal::new(String::new());
    view! {
        <div class="pane">
            <h3>"Wardrobe"</h3>
            <div class="add-row">
                <input prop:value=move || name.get()
                    on:input=move |ev| name.set(event_target_value(&ev))
                    placeholder="persona name"/>
                <input prop:value=move || desc.get()
                    on:input=move |ev| desc.set(event_target_value(&ev))
                    placeholder="description"/>
                <button on:click=move |_| {
                    let (n, d) = (name.get_untracked(), desc.get_untracked());
                    name.set(String::new());
                    desc.set(String::new());
                    act::create_persona(s, n, d);
                }>"Create persona"</button>
            </div>
            <div class="persona-grid">
                {move || s.personas.get().into_iter().map(|p| {
                    let pid = p.id.clone();
                    let pid_worn = pid.clone();
                    let worn = move || s.active_persona.get().as_deref() == Some(pid_worn.as_str());
                    let pid_wear = pid.clone();
                    view! {
                        <div class="persona-card">
                            <span class="pname">{p.name}</span>
                            <Show when=worn
                                fallback=move || {
                                    let pid = pid_wear.clone();
                                    view! {
                                        <button on:click=move |_| act::wear_persona(s, pid.clone())>
                                            "Wear"
                                        </button>
                                    }
                                }>
                                <button class="worn" on:click=move |_| act::unwear(s)>"Worn ✓"</button>
                            </Show>
                        </div>
                    }
                }).collect_view()}
            </div>
        </div>
    }
}

#[component]
fn LorebookPane(s: Shell) -> impl IntoView {
    let keys = RwSignal::new(String::new());
    let content = RwSignal::new(String::new());
    let cid = move || s.sel_channel.get().map(|c| c.id).unwrap_or_default();
    view! {
        <div class="pane">
            <h3>"Lorebook"</h3>
            <div class="lore-list">
                {move || s.lore.get().into_iter().map(|e| {
                    let entry_cid = cid();
                    let eid = e.id.clone();
                    let title = if e.title.is_empty() { e.keys.join(", ") } else { e.title };
                    view! {
                        <div class="lore-entry">
                            <div class="lore-head">
                                <strong>{title}</strong>
                                <button on:click=move |_|
                                    act::delete_lore(s, entry_cid.clone(), eid.clone())>"✕"</button>
                            </div>
                            <div class="lore-content">{e.content}</div>
                        </div>
                    }
                }).collect_view()}
            </div>
            <div class="lore-add">
                <input prop:value=move || keys.get()
                    on:input=move |ev| keys.set(event_target_value(&ev))
                    placeholder="trigger keywords (comma-separated)"/>
                <textarea prop:value=move || content.get()
                    on:input=move |ev| content.set(event_target_value(&ev))
                    placeholder="entry content"></textarea>
                <button on:click=move |_| {
                    let parsed = keys.get_untracked()
                        .split(',')
                        .map(|k| k.trim().to_string())
                        .filter(|k| !k.is_empty())
                        .collect::<Vec<_>>();
                    let body = content.get_untracked();
                    keys.set(String::new());
                    content.set(String::new());
                    act::create_lore(s, cid(), parsed, body);
                }>"Add entry"</button>
            </div>
        </div>
    }
}

#[component]
fn FriendsPane(s: Shell) -> impl IntoView {
    let username = RwSignal::new(String::new());
    view! {
        <div class="pane">
            <h3>"Friends"</h3>
            <div class="add-row">
                <input prop:value=move || username.get()
                    on:input=move |ev| username.set(event_target_value(&ev))
                    placeholder="add by username"/>
                <button on:click=move |_| {
                    let u = username.get_untracked();
                    username.set(String::new());
                    act::add_friend(s, u);
                }>"Add"</button>
            </div>
            {move || {
                let f = s.friends.get();
                view! {
                    <ul class="flist">
                        {f.incoming.into_iter().map(|p| {
                            let aid = p.account_id.clone();
                            view! {
                                <li>
                                    <span class="tag in">"wants to add"</span> {p.username} " "
                                    <button on:click=move |_| act::accept_friend(s, aid.clone())>"Accept"</button>
                                </li>
                            }
                        }).collect_view()}
                        {f.outgoing.into_iter().map(|p| view! {
                            <li><span class="tag out">"pending"</span> {p.username}</li>
                        }).collect_view()}
                        {f.friends.into_iter().map(|p| {
                            let aid = p.account_id.clone();
                            view! {
                                <li>
                                    <span class="tag ok">"friend"</span> {p.username} " "
                                    <button on:click=move |_| act::remove_friend(s, aid.clone())>"Remove"</button>
                                </li>
                            }
                        }).collect_view()}
                    </ul>
                }
            }}
        </div>
    }
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

// ---------------------------------------------------------------------------
// Actions — real on hydrate, no-op stubs on ssr (so the view calls them
// ungated and gloo-net never enters the ssr graph).
// ---------------------------------------------------------------------------

#[cfg(feature = "hydrate")]
mod act {
    use super::{Pane, Shell};
    use crate::client::api;
    use crate::protocol::{ChannelSummary, MessageEnvelope};
    use crate::ui::AuthCtx;
    use leptos::prelude::*;
    use leptos::task::spawn_local;

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

    pub fn open_server(s: Shell, gid: String) {
        s.sel_server.set(Some(gid.clone()));
        s.channels.set(Vec::new());
        spawn_local(async move {
            if let Ok(d) = api::get_guild(&gid).await {
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

    pub fn wear_persona(s: Shell, pid: String) {
        s.active_persona.set(Some(pid.clone()));
        if let Some(gid) = s.sel_server.get_untracked() {
            spawn_local(async move {
                let _ = api::set_active_persona(&gid, Some(pid)).await;
            });
        }
    }

    pub fn unwear(s: Shell) {
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

    /// One poll loop, started on the first text-channel open; reads the current
    /// channel + cursor each tick. SEAM: replace with an SSE subscription.
    fn start_poll(s: Shell) {
        if s.polling.get_untracked() {
            return;
        }
        s.polling.set(true);
        spawn_local(async move {
            loop {
                gloo_timers::future::TimeoutFuture::new(1500).await;
                if s.pane.get_untracked() != Pane::Channel {
                    continue;
                }
                let Some(ch) = s.sel_channel.get_untracked() else {
                    continue;
                };
                let cur = s.cursor.get_untracked();
                if let Ok(l) = api::list_messages(&ch.id, cur.as_ref()).await {
                    ingest(s, l.messages);
                }
            }
        });
    }
}

#[cfg(not(feature = "hydrate"))]
mod act {
    use super::Shell;
    use crate::protocol::ChannelSummary;
    use crate::ui::AuthCtx;

    pub fn logout(_auth: AuthCtx) {}
    pub fn refresh_guilds(_s: Shell) {}
    pub fn open_server(_s: Shell, _gid: String) {}
    pub fn open_channel(_s: Shell, _ch: ChannelSummary) {}
    pub fn create_server(_s: Shell, _name: String) {}
    pub fn create_channel(_s: Shell, _name: String) {}
    pub fn send_message(_s: Shell) {}
    pub fn show_friends(_s: Shell) {}
    pub fn show_wardrobe(_s: Shell) {}
    pub fn create_persona(_s: Shell, _name: String, _desc: String) {}
    pub fn wear_persona(_s: Shell, _pid: String) {}
    pub fn unwear(_s: Shell) {}
    pub fn add_friend(_s: Shell, _username: String) {}
    pub fn accept_friend(_s: Shell, _aid: String) {}
    pub fn remove_friend(_s: Shell, _aid: String) {}
    pub fn create_lore(_s: Shell, _cid: String, _keys: Vec<String>, _content: String) {}
    pub fn delete_lore(_s: Shell, _cid: String, _eid: String) {}
}
