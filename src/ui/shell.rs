//! The authed app frame. `Home` is the `/` route; it bounces to `/login` when
//! the session resolves unauthenticated, otherwise renders the Discord-style
//! shell: a server rail, a channel sidebar, and the message pane (with live
//! markup rendering and a formatting toolbar).
//!
//! State is signal-driven rather than URL-routed for v1 (selecting a server /
//! channel flips signals; deep-link URLs are a later polish). Data-fetching
//! actions are defined twice — a real hydrate version and a no-op ssr stub —
//! so the view's event handlers call them ungated.

use std::collections::HashSet;

use leptos::prelude::*;

use crate::markup::Color;
use crate::protocol::{ChannelSummary, GuildSummary, MessageEnvelope};
use crate::ui::markup_view::render_body;
use crate::ui::AuthCtx;

#[cfg(feature = "hydrate")]
use crate::client::api;

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

/// All of the shell's reactive state, bundled so the action helpers take one
/// `Copy` handle. `RwSignal` is `Copy`, so this whole struct is.
/// (cursor/seen/polling are read only in the hydrate poll path.)
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
    };
    let new_server = RwSignal::new(String::new());
    let new_channel = RwSignal::new(String::new());

    // Effects only run client-side; on ssr this references the no-op stub.
    Effect::new(move |_| refresh_guilds(s));

    let username = move || auth.user.get().map(|u| u.username).unwrap_or_default();
    let logout = move |_| logout_action(auth);

    view! {
        <div class="app">
            // ---- server rail ----
            <nav class="rail">
                {move || s.guilds.get().into_iter().map(|g| {
                    let gid = g.id.clone();
                    let initial = g.name.chars().next().unwrap_or('#').to_string();
                    view! {
                        <button class="rail-guild" title=g.name
                            on:click=move |_| open_server(s, gid.clone())>
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
                        create_server(s, name);
                    }>"+"</button>
                </div>
            </nav>

            // ---- channel sidebar ----
            <aside class="sidebar">
                <Show when=move || s.sel_server.get().is_some()
                    fallback=|| view! { <p class="muted pad">"Pick or create a server."</p> }>
                    <ul class="channels">
                        {move || s.channels.get().into_iter().map(|c| {
                            let ch = c.clone();
                            let sigil = if c.kind == "lorebook" { "📖 " } else { "# " };
                            let cid = c.id.clone();
                            let active = move || {
                                s.sel_channel.get().map(|sc| sc.id) == Some(cid.clone())
                            };
                            view! {
                                <li>
                                    <button class="channel" class:active=active
                                        on:click=move |_| open_channel(s, ch.clone())>
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
                            create_channel(s, name);
                        }>"+"</button>
                    </div>
                </Show>
            </aside>

            // ---- main content ----
            <section class="content">
                <header class="topbar">
                    <span class="muted">"Signed in as " <strong>{username}</strong></span>
                    <span class="spacer"></span>
                    <button on:click=logout>"Log out"</button>
                </header>

                <Show when=move || s.sel_channel.get().is_some()
                    fallback=|| view! { <p class="muted pad">"Select a channel."</p> }>
                    <ul class="messages">
                        {move || s.messages.get().into_iter().map(|m| {
                            let who = m.persona_name.clone()
                                .unwrap_or_else(|| short_id(&m.author_id));
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
                                        send_message(s);
                                    }
                                }
                                #[cfg(not(feature = "hydrate"))]
                                let _ = &ev;
                            }
                            placeholder="type a message — **bold**, *italic*, [red]color[/red]"
                        ></textarea>
                        <button class="send" on:click=move |_| send_message(s)>"Send"</button>
                    </div>
                </Show>
                <p class="error">{move || s.status.get()}</p>
            </section>
        </div>
    }
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

// ---------------------------------------------------------------------------
// Actions — real on hydrate, no-op stubs on ssr so the view calls them ungated
// ---------------------------------------------------------------------------

#[cfg(feature = "hydrate")]
fn logout_action(auth: AuthCtx) {
    let nav = leptos_router::hooks::use_navigate();
    leptos::task::spawn_local(async move {
        let _ = api::logout().await;
        auth.user.set(None);
        nav("/login", Default::default());
    });
}
#[cfg(not(feature = "hydrate"))]
fn logout_action(_auth: AuthCtx) {}

#[cfg(feature = "hydrate")]
fn refresh_guilds(s: Shell) {
    leptos::task::spawn_local(async move {
        if let Ok(resp) = api::list_guilds().await {
            s.guilds.set(resp.guilds);
        }
    });
}
#[cfg(not(feature = "hydrate"))]
fn refresh_guilds(_s: Shell) {}

#[cfg(feature = "hydrate")]
fn open_server(s: Shell, gid: String) {
    s.sel_server.set(Some(gid.clone()));
    s.channels.set(Vec::new());
    leptos::task::spawn_local(async move {
        if let Ok(detail) = api::get_guild(&gid).await {
            s.channels.set(detail.channels.clone());
            if let Some(first) = detail
                .channels
                .iter()
                .find(|c| c.kind == "text")
                .or_else(|| detail.channels.first())
            {
                open_channel(s, first.clone());
            }
        }
    });
}
#[cfg(not(feature = "hydrate"))]
fn open_server(_s: Shell, _gid: String) {}

#[cfg(feature = "hydrate")]
fn open_channel(s: Shell, ch: ChannelSummary) {
    let cid = ch.id.clone();
    s.sel_channel.set(Some(ch));
    s.messages.set(Vec::new());
    s.cursor.set(None);
    s.seen.update(|h| h.clear());
    start_poll(s);
    leptos::task::spawn_local(async move {
        if let Ok(list) = api::list_messages(&cid, None).await {
            ingest(s, list.messages);
        }
    });
}
#[cfg(not(feature = "hydrate"))]
fn open_channel(_s: Shell, _ch: ChannelSummary) {}

#[cfg(feature = "hydrate")]
fn send_message(s: Shell) {
    let Some(ch) = s.sel_channel.get_untracked() else {
        return;
    };
    let body = s.compose.get_untracked();
    if body.trim().is_empty() {
        return;
    }
    s.compose.set(String::new());
    s.status.set(String::new());
    leptos::task::spawn_local(async move {
        match api::post_message(&ch.id, &body).await {
            Ok(_) => {
                let cur = s.cursor.get_untracked();
                if let Ok(list) = api::list_messages(&ch.id, cur.as_ref()).await {
                    ingest(s, list.messages);
                }
            }
            Err(e) => s.status.set(api::humanize(&e)),
        }
    });
}
#[cfg(not(feature = "hydrate"))]
fn send_message(_s: Shell) {}

#[cfg(feature = "hydrate")]
fn create_server(s: Shell, name: String) {
    if name.trim().is_empty() {
        return;
    }
    leptos::task::spawn_local(async move {
        match api::create_guild(&name).await {
            Ok(g) => {
                refresh_guilds(s);
                open_server(s, g.id);
            }
            Err(e) => s.status.set(api::humanize(&e)),
        }
    });
}
#[cfg(not(feature = "hydrate"))]
fn create_server(_s: Shell, _name: String) {}

#[cfg(feature = "hydrate")]
fn create_channel(s: Shell, name: String) {
    let Some(gid) = s.sel_server.get_untracked() else {
        return;
    };
    if name.trim().is_empty() {
        return;
    }
    leptos::task::spawn_local(async move {
        match api::create_channel(&gid, &name, "text").await {
            Ok(_) => open_server(s, gid),
            Err(e) => s.status.set(api::humanize(&e)),
        }
    });
}
#[cfg(not(feature = "hydrate"))]
fn create_channel(_s: Shell, _name: String) {}

// ---- internal (hydrate-only) ----

/// Append messages not already seen, advancing the cursor.
#[cfg(feature = "hydrate")]
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

/// One poll loop, started on the first channel open; it reads the currently
/// selected channel + cursor each tick. SEAM: replace with an SSE subscription.
#[cfg(feature = "hydrate")]
fn start_poll(s: Shell) {
    if s.polling.get_untracked() {
        return;
    }
    s.polling.set(true);
    leptos::task::spawn_local(async move {
        loop {
            gloo_timers::future::TimeoutFuture::new(1500).await;
            let Some(ch) = s.sel_channel.get_untracked() else {
                continue;
            };
            let cur = s.cursor.get_untracked();
            if let Ok(list) = api::list_messages(&ch.id, cur.as_ref()).await {
                ingest(s, list.messages);
            }
        }
    });
}
