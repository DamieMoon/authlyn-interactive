//! W5/P2 Omloppsbana (`sk-orbit`) — the spatial gesture-first structural
//! skeleton. Full-viewport channel panes in a horizontal swipe strip, a
//! holographic channel pill opening a zoomable orbit-map picker (pill-tap entry
//! ONLY — the pinch entry was judge-killed), a floating composer orb with a
//! length-charged send ring + effect blossom, and a right-edge HoloPanel
//! slide-over. The shell view (`SkOrbitShell`) reuses every existing pane via
//! `use_context::<Shell>()`; the gesture/transition DECISIONS are pure fns in
//! the submodules below (unit-tested, no DOM — the project has no WASM UI test
//! harness). Built on the Foundation substrate: portals (#54), etched glass
//! (#20), HoloPanel (#49), visual haptics (#19), the transform-free
//! .channel-view warp wrapper, and the .app.sk-orbit root class already wired
//! in `shell/mod.rs`.
//!
//! Shared/always-on math modules (no ssr/hydrate crates); the view code that
//! consumes them is feature-gated where it touches `web_sys`.

pub mod charge;
pub mod orbit_map;
pub mod strip;
pub mod warp;

use leptos::portal::Portal;
use leptos::prelude::*;

use self::orbit_map::{map_geom, node_pos};
use super::{
    act, channel::ChannelPane, emoji_manager::EmojiManagerPane, friends::FriendsPane,
    lorebook::LorebookPane, members::MembersPane, Pane, Shell,
};

/// Live viewport (width, height) in CSS px. Falls back to the POCO C3 floor
/// off-DOM / on ssr so the geometry is sane before hydrate.
#[cfg(feature = "hydrate")]
fn viewport_dims() -> (f64, f64) {
    let win = leptos::web_sys::window();
    let w = win
        .as_ref()
        .and_then(|w| w.inner_width().ok())
        .and_then(|v| v.as_f64())
        .unwrap_or(360.0);
    let h = win
        .and_then(|w| w.inner_height().ok())
        .and_then(|v| v.as_f64())
        .unwrap_or(800.0);
    (w, h)
}
#[cfg(not(feature = "hydrate"))]
fn viewport_dims() -> (f64, f64) {
    (360.0, 800.0)
}

/// The Omloppsbana shell chrome. Renders as a sibling of the W3 chrome under
/// `.app.sk-orbit`, reusing every pane via `use_context::<Shell>()` (zero new
/// state, no remount on switch). This first cut mounts only the pane switch +
/// account control; the orbit chrome (pill, orbit map, composer orb, slide-
/// over) lands in later tasks. The full-viewport panes + swipe strip layout is
/// driven entirely by `style/_sk_orbit.scss` keyed off `.app.sk-orbit`.
#[component]
pub fn SkOrbitShell() -> impl IntoView {
    let s = use_context::<Shell>().expect("Shell provided by AppShell");
    let auth = use_context::<crate::ui::AuthCtx>().expect("AuthCtx provided at root");
    let username = move || auth.user.get().map(|u| u.username).unwrap_or_default();
    let map_open = RwSignal::new(false);
    let channel_name = move || {
        s.sel
            .sel_channel
            .get()
            .map(|c| c.name)
            .unwrap_or_else(|| "—".to_string())
    };
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
    // Modal-parity focus (gate item): the pill is the trigger; closing the map
    // restores focus to it (WCAG 2.4.3). The overlay div is focused on open.
    let pill_ref = NodeRef::<leptos::html::Button>::new();
    let map_ref = NodeRef::<leptos::html::Div>::new();
    let close_map = move || {
        map_open.set(false);
        #[cfg(feature = "hydrate")]
        if let Some(pill) = pill_ref.get_untracked() {
            let _ = (*pill).focus();
        }
    };
    // Focus the overlay container when it mounts (the dialog announces its own
    // name; the first Tab lands on the first node — never spotlighting one as
    // chosen). Mirrors `ChannelSheet`'s focus-in Effect.
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        if map_open.get() {
            if let Some(map) = map_ref.get() {
                let _ = (*map).focus();
            }
        }
    });
    view! {
        <section class="content sk-orbit-content">
            <button class="sk-orbit-pill" type="button"
                node_ref=pill_ref
                aria-haspopup="dialog"
                aria-expanded=move || map_open.get().to_string()
                title="Open the orbit map"
                on:click=move |_| map_open.set(true)>
                <span class="sk-orbit-pill-name">"# "{channel_name}</span>
                <span class="sk-orbit-pill-server">{server_name}</span>
                <span class="sk-orbit-pill-dots" aria-hidden="true">
                    {move || {
                        let chans = s.sel.channels.get();
                        let cur = s.sel.sel_channel.get().map(|c| c.id);
                        chans.into_iter().map(|c| {
                            let on = Some(&c.id) == cur.as_ref();
                            view! { <span class="sk-orbit-dot" class:on=on></span> }
                        }).collect_view()
                    }}
                </span>
            </button>
            <header class="topbar sk-orbit-topbar">
                <span class="muted">"Signed in as " <strong>{username}</strong></span>
                <span class="spacer"></span>
                <span class="sync-chip" class:live=move || s.sync.sse_live.get()>
                    {move || if s.sync.sse_live.get() { "● LIVE" } else { "● POLLING" }}
                </span>
            </header>
            {move || match s.sync.pane.get() {
                Pane::Friends => view! { <FriendsPane/> }.into_any(),
                Pane::Channel => view! { <ChannelPane/> }.into_any(),
                Pane::Lorebook => view! { <LorebookPane/> }.into_any(),
                Pane::Emoji => view! { <EmojiManagerPane/> }.into_any(),
                Pane::Members => view! { <MembersPane/> }.into_any(),
            }}
            <p class="error">{move || s.composer.status.get()}</p>
            {move || map_open.get().then(|| view! {
                <Portal>
                    <div class="sk-orbit-map" role="dialog" aria-modal="true"
                        node_ref=map_ref
                        aria-label="Orbit map — pick a channel or server" tabindex="-1"
                        on:keydown=move |ev: leptos::ev::KeyboardEvent| {
                            if ev.key() == "Escape" {
                                ev.prevent_default();
                                close_map();
                            }
                        }>
                        <button class="sk-orbit-map-scrim" aria-label="Close orbit map"
                            on:click=move |_| close_map()></button>
                        <div class="sk-orbit-core">{server_name}</div>
                        {move || {
                            // Geometry from the live viewport (UX-equality).
                            let (vw, vh) = viewport_dims();
                            let g = map_geom(vw, vh);
                            let chans = s.sel.channels.get();
                            let n = chans.len();
                            let unread = s.notify.unread.get();
                            chans.into_iter().enumerate().map(|(i, c)| {
                                let p = node_pos(i, n, g.orbit_radius);
                                let cid = c.id.clone();
                                let has_unread = unread.contains(&c.id);
                                let ch = c.clone();
                                view! {
                                    <button class="sk-orbit-node"
                                        class:unread=has_unread
                                        style:transform=format!(
                                            "translate(calc(50vw + {}px), calc(50vh + {}px)) translate(-50%, -50%)",
                                            p.x, p.y
                                        )
                                        title=c.name.clone()
                                        on:click=move |_| {
                                            act::open_channel(s, ch.clone());
                                            close_map();
                                        }>
                                        <span class="sk-orbit-node-label">{c.name.clone()}</span>
                                        {has_unread.then(|| view! { <span class="sk-orbit-node-dot" aria-hidden="true"></span> })}
                                        {let _ = &cid;}
                                    </button>
                                }
                            }).collect_view()
                        }}
                        {move || {
                            // Other servers docked in the top corners.
                            let (vw, vh) = viewport_dims();
                            let g = map_geom(vw, vh);
                            let cur = s.sel.sel_server.get();
                            s.sel.guilds.get().into_iter()
                                .filter(|gd| Some(&gd.id) != cur.as_ref())
                                .enumerate()
                                .map(|(i, gd)| {
                                    let gid = gd.id.clone();
                                    // Alternate left/right docks so multiple far
                                    // servers stay on-screen.
                                    let side = if i % 2 == 0 { 1.0 } else { -1.0 };
                                    view! {
                                        <button class="sk-orbit-far"
                                            style:transform=format!(
                                                "translate(calc(50vw + {}px), calc(50vh + {}px)) translate(-50%, -50%)",
                                                g.far_x * side, g.far_y
                                            )
                                            title=gd.name.clone()
                                            on:click=move |_| {
                                                act::open_server(s, gid.clone());
                                                close_map();
                                            }>
                                            {gd.name.clone()}
                                        </button>
                                    }
                                }).collect_view()
                        }}
                    </div>
                </Portal>
            })}
        </section>
    }
}
