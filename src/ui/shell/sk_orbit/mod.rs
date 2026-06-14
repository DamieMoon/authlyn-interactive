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

pub mod blossom;
pub mod charge;
pub mod drag;
pub mod orbit_map;
pub mod strip;
pub mod warp;

use leptos::portal::Portal;
use leptos::prelude::*;

use self::orbit_map::{map_geom, node_pos};
use super::holopanel::{Detent, Edge, HoloPanel};
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

/// The orbit-map dialog's focusable children in DOM order, for the Tab trap
/// (mirrors `holopanel::PanelDrag::focusables` / `lightbox::focusables` — the
/// shared selector shape). The map is `aria-modal` but nothing makes the
/// scrimmed shell behind it inert, so wrapping Tab here is the ONLY thing
/// keeping keyboard/AT focus from escaping into the still-focusable pill +
/// composer + topbar behind the portal (design law §13: Modal-parity trap).
#[cfg(feature = "hydrate")]
fn focusables(root: &leptos::web_sys::Element) -> Vec<leptos::web_sys::HtmlElement> {
    use leptos::wasm_bindgen::JsCast as _;
    const SEL: &str = "a[href], button:not([disabled]), input:not([disabled]), \
                       textarea:not([disabled]), select:not([disabled]), \
                       [tabindex]:not([tabindex=\"-1\"])";
    let Ok(list) = root.query_selector_all(SEL) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(list.length() as usize);
    for i in 0..list.length() {
        if let Some(el) = list
            .item(i)
            .and_then(|n| n.dyn_into::<leptos::web_sys::HtmlElement>().ok())
        {
            out.push(el);
        }
    }
    out
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
    // Kind-aware sigil, consistent with the W3 shell (`📖 ` lorebook, `# `
    // otherwise — `shell/mod.rs`); no surface renders the bare name.
    let channel_sigil = move || {
        s.sel
            .sel_channel
            .get()
            .map(|c| if c.kind == "lorebook" { "📖 " } else { "# " })
            .unwrap_or("# ")
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
    // Modal-parity focus (gate item, design law §13): trap (Tab/Shift+Tab wrap
    // within the map — see the on:keydown handler), Esc closes, and restore-to-
    // trigger — the pill is the trigger, so closing the map restores focus to it
    // (WCAG 2.4.3). The overlay div is focused on open.
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
    // Strip geometry: the current channel's index in the sidebar order.
    let cur_idx = move || {
        let chans = s.sel.channels.get();
        s.sel
            .sel_channel
            .get()
            .and_then(|c| chans.iter().position(|x| x.id == c.id))
    };
    let chan_count = move || s.sel.channels.get().len();
    let strip_ref = NodeRef::<leptos::html::Div>::new();
    // StoredValues feed the hydrate StripDrag without re-rendering it.
    #[cfg(feature = "hydrate")]
    let idx_sv = StoredValue::new(0usize);
    #[cfg(feature = "hydrate")]
    let count_sv = StoredValue::new(0usize);
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        idx_sv.set_value(cur_idx().unwrap_or(0));
        count_sv.set_value(chan_count());
    });
    // Commit: a Prev/Next swipe opens the neighbor channel (act handles the
    // switch + warp). The destination index comes from the UNIT-TESTED
    // strip::commit_target (Task 1.3) — edge guards (no prev at first / no next
    // at last / Stay) all collapse to None ⇒ no-op. A committed switch makes the
    // destination the ACTIVE channel, so marking it read on open is correct
    // (peek-never-marks-read concerns only non-current neighbors, which are
    // name-only here and never reach open_channel — see the Task 1.3 intro).
    let on_strip_commit = move |commit: strip::StripCommit| {
        let chans = s.sel.channels.get_untracked();
        let Some(i) = cur_idx() else { return };
        if let Some(j) = strip::commit_target(commit, i, chans.len()) {
            if let Some(ch) = chans.get(j).cloned() {
                act::open_channel(s, ch);
            }
        }
    };
    #[cfg(feature = "hydrate")]
    let strip_drag =
        self::drag::StripDrag::new(idx_sv, count_sv, Callback::new(on_strip_commit), strip_ref);
    #[cfg(not(feature = "hydrate"))]
    let _ = (strip_ref, on_strip_commit);
    // Composer-orb charge: the SVG ring fills with message LENGTH via the
    // log curve (#33 — a one-liner is a sliver, a paragraph ~60%, only a saga
    // pegs). The orb is the SOLE send surface under orbit (the in-pane
    // ChannelPane `.send` + its linear ring are hidden in SCSS), so this is the
    // ONLY ring that reflects length.
    let charge = Memo::new(move |_| {
        s.composer
            .compose
            .with(|c| self::charge::charge_fraction(c))
    });
    let armed_glyph = move || match s.composer.effect_mode.get().as_deref() {
        Some("whisper") => "🤫",
        Some("shout") => "📣",
        Some("spell") => "✨",
        _ => "",
    };
    // Effect blossom: a 480ms hold on the orb (BlossomHold, move-slop disarmed so
    // a jittery Send tap never blossoms — #47) opens three glass effect chips;
    // the trailing click is guarded so the hold never also sends.
    let blossom_open = RwSignal::new(false);
    let hold = blossom::BlossomHold::new();
    let orb_ref = NodeRef::<leptos::html::Button>::new();
    // Right-edge HoloPanel slide-over (personas + station). Summoned by an
    // explicit ☰ button (the chat-layer right-edge swipe is DEMOTED — it fights
    // iOS back-swipe). The single OPEN detent means only on_close can dismiss it.
    let station_open = RwSignal::new(false);
    // The summon button — focus restores here when the panel closes (§13).
    let station_btn_ref = NodeRef::<leptos::html::Button>::new();
    // Close the slide-over AND restore focus to the summon button (§13 Modal-
    // parity restore-to-trigger). Used by on_close (Esc + swipe-to-close) and
    // the explicit ← button.
    //
    // DELIBERATE asymmetric motion (matches the orbit-map sibling's `close_map`,
    // the established overlay convention here): close is a hard parent un-mount
    // (flip `station_open` false → the `<Show>`/`.then()` drops the HoloPanel +
    // scrim instantly), so the slide-IN transition plays but there is no
    // slide-OUT. Deferring un-mount behind a transitionend / timeout (set --p=0,
    // then drop) would add a motion-only state machine for symmetry alone; left
    // as a Phase-7 polish carry rather than risking the un-mount path now. The
    // SSE/composer/selection invariants are untouched either way (the panel is a
    // pure overlay; closing it never remounts AppShell).
    let close_station = move || {
        station_open.set(false);
        #[cfg(feature = "hydrate")]
        if let Some(btn) = station_btn_ref.get_untracked() {
            let _ = btn.focus();
        }
    };
    view! {
        <section class="content sk-orbit-content">
            <button class="sk-orbit-pill" type="button"
                node_ref=pill_ref
                aria-haspopup="dialog"
                aria-expanded=move || map_open.get().to_string()
                title="Open the orbit map"
                on:click=move |_| map_open.set(true)>
                <span class="sk-orbit-pill-name">{channel_sigil}{channel_name}</span>
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
                Pane::Channel => {
                    #[cfg(feature = "hydrate")]
                    let d = strip_drag.clone();
                    // Four handles: pointercancel shares the release path with
                    // pointerup (M-35) but needs its OWN clone — both can't move
                    // the same `d_up` into their closures.
                    #[cfg(feature = "hydrate")]
                    let (d_down, d_move, d_up, d_cancel) = (d.clone(), d.clone(), d.clone(), d);
                    view! {
                        <div class="sk-orbit-strip sk-orbit-strip--snap" node_ref=strip_ref
                            on:pointerdown=move |ev| {
                                #[cfg(feature = "hydrate")] d_down.down(&ev);
                                #[cfg(not(feature = "hydrate"))] let _ = &ev;
                            }
                            on:pointermove=move |ev| {
                                #[cfg(feature = "hydrate")] d_move.moved(&ev);
                                #[cfg(not(feature = "hydrate"))] let _ = &ev;
                            }
                            on:pointerup=move |ev| {
                                #[cfg(feature = "hydrate")] d_up.up(&ev);
                                #[cfg(not(feature = "hydrate"))] let _ = &ev;
                            }
                            on:pointercancel=move |ev| {
                                #[cfg(feature = "hydrate")] d_cancel.up(&ev);
                                #[cfg(not(feature = "hydrate"))] let _ = &ev;
                            }>
                            // prev/current/next. The current pane is the real
                            // ChannelPane (owns composer + list). The neighbors
                            // are peek previews (lazy first page, NEVER mark read).
                            <div class="sk-orbit-pane sk-orbit-pane-prev" aria-hidden="true">
                                {move || neighbor_preview(s, cur_idx().and_then(|i| i.checked_sub(1)))}
                            </div>
                            <div class="sk-orbit-pane sk-orbit-pane-cur">
                                <ChannelPane/>
                            </div>
                            <div class="sk-orbit-pane sk-orbit-pane-next" aria-hidden="true">
                                {move || neighbor_preview(s, cur_idx().map(|i| i + 1).filter(|&j| j < chan_count()))}
                            </div>
                        </div>
                    }.into_any()
                }
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
                            match ev.key().as_str() {
                                "Escape" => {
                                    ev.prevent_default();
                                    close_map();
                                }
                                // Modal-parity focus trap (design law §13): wrap
                                // Tab/Shift+Tab within the map's own controls so
                                // keyboard/AT focus can't escape into the
                                // still-focusable scrimmed shell behind the
                                // portal (pill, composer, topbar). Mirrors
                                // `lightbox`/`holopanel`'s trap; this keydown
                                // only fires while focus is inside the dialog,
                                // which is also what keeps Escape working.
                                "Tab" => {
                                    #[cfg(feature = "hydrate")]
                                    {
                                        use leptos::wasm_bindgen::JsCast as _;
                                        let Some(map) = map_ref.get_untracked() else {
                                            return;
                                        };
                                        let root: &leptos::web_sys::Element =
                                            (*map).unchecked_ref();
                                        let els = focusables(root);
                                        if els.is_empty() {
                                            return;
                                        }
                                        let active = leptos::web_sys::window()
                                            .and_then(|w| w.document())
                                            .and_then(|d| d.active_element())
                                            .and_then(|el| {
                                                el.dyn_into::<leptos::web_sys::HtmlElement>().ok()
                                            });
                                        let idx = active
                                            .as_ref()
                                            .and_then(|a| els.iter().position(|el| el == a));
                                        let last = els.len() - 1;
                                        // Wrap at either end; Shift+Tab from the
                                        // dialog root (idx None, the post-open
                                        // state) lands on the last control rather
                                        // than escaping backwards.
                                        let (wrap, target) = if ev.shift_key() {
                                            (idx == Some(0) || idx.is_none(), last)
                                        } else {
                                            (idx == Some(last), 0)
                                        };
                                        if wrap {
                                            ev.prevent_default();
                                            let _ = els[target].focus();
                                        }
                                    }
                                }
                                _ => {}
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
                                let has_unread = unread.contains(&c.id);
                                let ch = c.clone();
                                let sigil = if c.kind == "lorebook" { "📖 " } else { "# " };
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
                                        <span class="sk-orbit-node-label">{sigil}{c.name.clone()}</span>
                                        {has_unread.then(|| view! { <span class="sk-orbit-node-dot" aria-hidden="true"></span> })}
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
            <div class="sk-orbit-orb-wrap">
                <button class="sk-orbit-orb" type="button"
                    node_ref=orb_ref
                    // Braced: a bare `>` would close the <button> tag in rstml
                    // (matching the in-pane `.send` ring, channel/mod.rs).
                    class:charging={move || charge.get() > 0.0}
                    class:armed=move || s.composer.effect_mode.get().is_some()
                    style:--charge=move || format!("{:.3}", charge.get())
                    style:--dash=move || format!("{:.1}", self::charge::dash_offset(charge.get()))
                    title="Send (hold for effects)"
                    on:pointerdown=move |ev| {
                        #[cfg(feature = "hydrate")]
                        if let Some(el) = orb_ref.get_untracked() {
                            use leptos::wasm_bindgen::JsCast as _;
                            let el: leptos::web_sys::Element = (*el).clone().unchecked_into();
                            hold.down(&ev, blossom_open, el);
                        }
                        #[cfg(not(feature = "hydrate"))]
                        let _ = &ev;
                    }
                    on:pointermove=move |ev| hold.moved(&ev)
                    on:pointerup=move |ev| {
                        hold.cancel();
                        let _ = &ev;
                    }
                    on:pointercancel=move |ev| {
                        hold.cancel();
                        let _ = &ev;
                    }
                    on:click=move |_| {
                        // Guard: if the hold fired, swallow the trailing click
                        // (it opened the blossom, it must not also send).
                        if hold.take_fired() {
                            return;
                        }
                        act::send_message(s);
                    }>
                    <svg class="sk-orbit-ring" viewBox="0 0 52 52" aria-hidden="true">
                        <circle class="sk-orbit-ring-track" cx="26" cy="26" r="24"></circle>
                        <circle class="sk-orbit-ring-arc" cx="26" cy="26" r="24"></circle>
                    </svg>
                    <span class="sk-orbit-orb-glyph">{move || {
                        let g = armed_glyph();
                        if g.is_empty() { "➤" } else { g }
                    }}</span>
                </button>
                {move || blossom_open.get().then(|| {
                    let chips = [("whisper", "🤫"), ("shout", "📣"), ("spell", "✨")];
                    view! {
                        <div class="sk-orbit-blossom" role="menu" aria-label="Message effect">
                            {chips.into_iter().enumerate().map(|(i, (name, glyph))| {
                                let name_owned = name.to_string();
                                view! {
                                    <button class="sk-orbit-chip" role="menuitem"
                                        tabindex=if i == 0 { "0" } else { "-1" }
                                        style:--chip-i=i.to_string()
                                        title=name
                                        on:click=move |_| {
                                            // Toggle-arm this effect, then close.
                                            s.composer.effect_mode.update(|e| {
                                                *e = if e.as_deref() == Some(name_owned.as_str()) {
                                                    None
                                                } else {
                                                    Some(name_owned.clone())
                                                };
                                            });
                                            blossom_open.set(false);
                                        }>
                                        {glyph}
                                    </button>
                                }
                            }).collect_view()}
                        </div>
                    }
                })}
                <button class="sk-orbit-station-btn" type="button"
                    node_ref=station_btn_ref
                    aria-haspopup="dialog"
                    aria-expanded=move || station_open.get().to_string()
                    title="Personas & station settings"
                    on:click=move |_| station_open.set(true)>"☰"</button>
            </div>
            {move || station_open.get().then(|| view! {
                <HoloPanel
                    edge=Edge::Right
                    label="Personas & station settings"
                    open=true
                    detents=vec![Detent { at: 1.0, key: "open" }]
                    // Single-detent: a committed OPEN drag just re-asserts open
                    // (no-op). Dismissal flows through on_close (Esc + swipe-to-
                    // close → snap-to-closed), which restores focus to the button.
                    on_commit=move |_key: &'static str| {}
                    on_close=move |_| close_station()
                >
                    <div class="sk-orbit-station">
                        <button class="sk-orbit-station-close" title="Close" aria-label="Close"
                            on:click=move |_| close_station()>"←"</button>
                        <h2>{move || {
                            let cn = s.sel.sel_channel.get().map(|c| c.name).unwrap_or_default();
                            format!("Your persona in #{cn}")
                        }}</h2>
                        <div class="sk-orbit-persona-grid">
                            {move || {
                                let active = s.social.active_persona.get();
                                s.social.personas.get().into_iter().map(|p| {
                                    let pid = p.id.clone();
                                    let is_active = active.as_deref() == Some(p.id.as_str());
                                    view! {
                                        <button class="sk-orbit-persona-card"
                                            class:active=is_active
                                            title=p.name.clone()
                                            on:click=move |_| act::wear_persona(s, pid.clone())>
                                            {p.name.clone()}
                                        </button>
                                    }
                                }).collect_view()
                            }}
                        </div>
                        <h2>"Station"</h2>
                        <label class="sk-orbit-toggle">
                            <input type="checkbox"
                                prop:checked=move || s.prefs.ghost_quill.get()
                                on:change=move |ev| {
                                    let on = event_target_checked(&ev);
                                    s.prefs.ghost_quill.set(on);
                                    act::set_ghost_quill(on);
                                }/>
                            "Ghost Quill (live co-writer)"
                        </label>
                        <label class="sk-orbit-toggle">
                            <input type="checkbox"
                                prop:checked=move || s.prefs.eyecandy.get()
                                on:change=move |ev| {
                                    let on = event_target_checked(&ev);
                                    s.prefs.eyecandy.set(on);
                                    act::set_eyecandy(on);
                                }/>
                            "Aurora-max (eye-candy tier)"
                        </label>
                    </div>
                </HoloPanel>
            })}
        </section>
    }
}

/// A lightweight, read-only preview of a neighbor channel for the swipe strip's
/// prev/next slots. NAME-ONLY for Phase 2 (the lazy first-page neighbor render
/// is the Phase-7 carry 9.4.3-c) — which is exactly why peek-never-marks-read
/// holds STRUCTURALLY: a name-only neighbor is never a mounted `ChannelPane`,
/// never becomes "current", and never reaches `act::open_channel`/last-seen.
/// `idx == None` (no neighbor at the edge) renders an empty peek.
fn neighbor_preview(s: Shell, idx: Option<usize>) -> impl IntoView {
    let label = idx
        .and_then(|i| s.sel.channels.get().get(i).map(|c| c.name.clone()))
        .unwrap_or_default();
    view! {
        <div class="sk-orbit-peek">
            {(!label.is_empty()).then(|| view! {
                <span class="sk-orbit-peek-name">"# "{label}</span>
            })}
        </div>
    }
}
