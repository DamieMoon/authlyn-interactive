//! M5/P2 Orbit (`sk-orbit`) — the spatial gesture-first structural
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
pub mod pane_swipe;
pub mod strip;
pub mod warp;

use leptos::portal::Portal;
use leptos::prelude::*;

use self::orbit_map::{channel_orbit, map_geom, seed_of};
use super::holopanel::{Detent, Edge, HoloPanel};
use super::{
    act,
    channel::ChannelPane,
    emoji_manager::EmojiManagerPane,
    friends::{CameosPane, DirectMessagesPane, FriendsPane},
    lorebook::LorebookPane,
    members::MembersPane,
    Pane, Shell,
};
use crate::ui::icons::{
    IconBack, IconBook, IconChat, IconEdit, IconEmoji, IconFriends, IconHold, IconMembers, IconOrb,
    IconPersonas, IconPlus, IconSettings, IconShout, IconSpell, IconStar, IconSwipe, IconWhisper,
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

/// Wrap Tab focus within a portaled dialog `root` (the Modal-parity trap §13),
/// shared by the help + founding overlays (Finding 10) so their trap can't drift
/// from the orbit map's. Computes the active element's position among `root`'s
/// `focusables` and, when Tab/Shift+Tab would leave either end, focuses the
/// opposite end and returns `true` so the caller `prevent_default()`s. Shift+Tab
/// from the dialog root itself (idx `None`, the post-open state) wraps to the
/// last control rather than escaping backwards — exactly the map's rule. Returns
/// `false` (let the browser move focus normally) when there are no focusables or
/// the move stays inside.
#[cfg(feature = "hydrate")]
fn trap_tab(root: &leptos::web_sys::Element, shift: bool) -> bool {
    use leptos::wasm_bindgen::JsCast as _;
    let els = focusables(root);
    if els.is_empty() {
        return false;
    }
    let active = leptos::web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.active_element())
        .and_then(|el| el.dyn_into::<leptos::web_sys::HtmlElement>().ok());
    let idx = active
        .as_ref()
        .and_then(|a| els.iter().position(|el| el == a));
    let last = els.len() - 1;
    let (wrap, target) = if shift {
        (idx == Some(0) || idx.is_none(), last)
    } else {
        (idx == Some(last), 0)
    };
    if wrap {
        let _ = els[target].focus();
    }
    wrap
}

/// The Orbit shell chrome. Renders as a sibling of the M3 chrome under
/// `.app.sk-orbit`, reusing every pane via `use_context::<Shell>()` (zero new
/// state, no remount on switch). The orbit chrome — channel pill, zoomable
/// orbit map, swipe strip, composer orb (charge ring + effect blossom), and the
/// right-edge station slide-over — is driven by `style/_sk_orbit.scss` keyed off
/// `.app.sk-orbit`.
///
/// `account_open` is the shell-wide AccountModal visibility signal (owned by
/// `AppShell`, mod.rs); the station slide-over's "Account & preferences" / "Log
/// out" affordances flip it. The orbit chrome has NO topbar gear, so this is the
/// ONLY path to the (skeleton-independent) account modal — without it the orbit
/// user is trapped with no logout and no way back to the skeleton chooser (F2).
/// `server_open` is its owner-gated sibling — the shell-wide ServerModal
/// (accent / invitations / channels) opened by the station's "Server settings"
/// button (M5 parity: consolidates the M3 sidebar's scattered server controls).
#[component]
pub fn SkOrbitShell(account_open: RwSignal<bool>, server_open: RwSignal<bool>) -> impl IntoView {
    let s = use_context::<Shell>().expect("Shell provided by AppShell");
    // Owner gate for the station's server-management affordances (mirrors
    // AppShell's is_owner; `s.sync.me` is the viewer's account id, set at init).
    let is_owner = move || {
        let me = s.sync.me.get();
        me.is_some() && me == s.sel.sel_owner.get()
    };
    // Promoted to shared state (state.rs) so the root-mounted Account/Server
    // modals can return here on dismiss (`act::show_orbit_map`). Copy semantics
    // keep every in-shell use-site below unchanged.
    let map_open = s.sync.map_open;
    // Node-dive exit (a-orbit.html enterChannel): tapping a channel zooms the map
    // INTO that node (scale 3.4 + fade) before it un-mounts. `diving` flips the
    // `.diving` class; `dive_origin` re-points the map's transform-origin to the
    // tapped node's screen centre so the zoom flies into the chosen channel.
    let diving = RwSignal::new(false);
    let dive_origin = RwSignal::new(String::from("center"));
    // Composer choreography (M5/P2, the prototype's `body.composing`): the orb
    // becomes a COMPOSE trigger (no longer send) — a tap reveals the composer and
    // hides the orb; the in-composer send button commits; a tap-away scrim
    // collapses it back (a-orbit.html expandComposer/collapseComposer).
    let composing = RwSignal::new(false);
    // Collapse the composer back to the orb once a send/roll actually LANDS
    // (the prototype's `body.composing` clears on send; owner found the composer
    // staying open over his own message on his iPhone). `composing` is shell-
    // local but the send path lives in ChannelPane/`act::send_message`, so bridge
    // the collapse through the shared one-shot pulse `after_send_success` fires on
    // every committed send + roll (`s.composer.sent`). A FAILED send never pulses
    // it, so the composer stays open to retry — the right behaviour.
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        if s.composer.sent.get() {
            composing.set(false);
        }
    });
    // The INVERSE bridge (M6 bug-fix): editing or replying to a message must
    // REVEAL the composer, exactly as tapping the orb does. `act::start_edit` /
    // `act::start_reply` live in the shared act layer and set `editing` /
    // `replying_to`, but `composing` is shell-local — so open it here when either
    // becomes Some. Their own `query_selector(".composer textarea").focus()` runs
    // during the user gesture but targets the still-hidden (visibility:hidden)
    // at-rest composer on orbit, a no-op; this reveal is what makes it appear, and
    // the focus below (after the reveal) is the one that lands. We do NOT read
    // `composing` here, so writing it cannot re-trigger this effect (no cycle).
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        let opening = s.composer.editing.get().is_some() || s.composer.replying_to.get().is_some();
        if opening {
            composing.set(true);
            use leptos::wasm_bindgen::JsCast as _;
            if let Some(el) = leptos::web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| {
                    d.query_selector(".app.sk-orbit .composer textarea")
                        .ok()
                        .flatten()
                })
            {
                if let Ok(t) = el.dyn_into::<leptos::web_sys::HtmlElement>() {
                    let _ = t.focus();
                }
            }
        }
    });
    let channel_name = move || {
        s.sel
            .sel_channel
            .get()
            .map(|c| c.name)
            .unwrap_or_else(|| "—".to_string())
    };
    // Kind-aware sigil, consistent with the M3 shell (`📖 ` lorebook, `# `
    // otherwise — `shell/mod.rs`); no surface renders the bare name.
    let channel_sigil = move || {
        let is_lore = s
            .sel
            .sel_channel
            .get()
            .map(|c| c.kind == "lorebook")
            .unwrap_or(false);
        if is_lore {
            view! { <IconBook/> }.into_any()
        } else {
            view! { "#" }.into_any()
        }
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
    // Scene-light (#B, Eye-candy tier): the ambient pane wash takes the tint of the
    // currently-active speaker — the most-recent message's persona palette color
    // (`persona_color` is the markup palette NAME, protocol.rs), mapped to its
    // `--tint-{name}` token (style/_tokens.scss). Empty/None ⇒ no tint. Bound
    // always but only VISIBLE under `.fx-max` (the SCSS gates it), so this is a
    // no-op at Standard tier.
    let scene_tint = move || {
        s.msg.messages.with(|ms| {
            ms.last()
                .and_then(|m| m.persona_color.clone())
                // Reuse the validated palette→token mapper (the same one the M3
                // accent binds at this .app root, shell/mod.rs): it returns "" for
                // empty/unknown so a non-palette name can never reach the CSS, and
                // the `--tint-*` tokens stay the single palette source. NEVER
                // splice the raw server-supplied `persona_color` into the token
                // string — `persona.color` has no schema ASSERT (schema.surql), so
                // the app-layer validation is the only thing keeping it safe.
                .map(|c| crate::ui::accent::accent_var_css(&c))
                .unwrap_or_default()
        })
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
    // Boot lands in the orbit view (owner ruling 2026-06-17): open the map on
    // first client mount so the app opens to the channels orbiting the restored
    // last server, not a flat pane. Client-only Effect — runs once after
    // hydration (Leptos effects never run on ssr, so the map is rendered closed
    // server-side → no hydration mismatch; its geometry needs the real viewport
    // anyway). Writes-only (reads no signal) ⇒ fires exactly once, and never
    // re-opens after the user dives into a channel.
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        map_open.set(true);
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
    // Finding 9 (M7): the six non-channel dispatch panes (Friends/Members/Emoji/
    // Lorebook/DMs/Cameos) mount full-viewport but were dismissed ONLY by the
    // top-left back disc — diverging from the product's swipe-right-to-close
    // paradigm (the wardrobe slide-over). Wrap them in a swipe surface bound to
    // `pane_swipe::PaneSwipe` (axis-lock reused from `strip::axis_lock`, the SAME
    // 28% commit the wardrobe Modal uses), and on a completed rightward swipe pop
    // back to the orbit MAP — the panes' correct dismiss target, exactly where
    // the back disc lands (`act::show_orbit_map`). The back disc stays the
    // keyboard/a11y fallback. The Channel pane is NOT wrapped: it owns its own
    // horizontal swipe (the channel strip), and back-swiping a channel is a
    // neighbor switch, not a dismiss.
    let panes_ref = NodeRef::<leptos::html::Div>::new();
    #[cfg(feature = "hydrate")]
    let pane_swipe = self::pane_swipe::PaneSwipe::new(panes_ref);
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
    // Effect blossom: a 480ms hold on the orb (BlossomHold, move-slop disarmed so
    // a jittery Send tap never blossoms — #47) opens three glass effect chips;
    // the trailing click is guarded so the hold never also sends. KEYBOARD parity
    // (a11y): the blossom is a `role="menu"` with full roving-tabindex arrow
    // navigation (`blossom_active` indexes the focused chip), an Escape that
    // closes + refocuses the orb, focus-on-open onto the first chip, AND a
    // keyboard OPEN path off the orb (ArrowUp/Down — Enter/Space stay SEND, like
    // the pointer tap) — the orb is the SOLE effect surface, so a
    // pointer-hold-only open would lock keyboard and AT users out of effects
    // entirely.
    let blossom_open = RwSignal::new(false);
    // Roving tabindex cursor: which of the 3 chips currently owns tabindex=0
    // (the rest are -1). DOM order is whisper(0)/shout(1)/spell(2); the SCSS
    // fans them UP via flex column-reverse, so ArrowUp walks toward higher
    // indices and ArrowDown toward lower — matching the visual stack.
    let blossom_active = RwSignal::new(0usize);
    let hold = blossom::BlossomHold::new();
    let orb_ref = NodeRef::<leptos::html::Button>::new();
    let blossom_ref = NodeRef::<leptos::html::Div>::new();
    // Refocus the orb after the blossom closes via keyboard (Escape / pick), so
    // focus is never stranded on an unmounted chip (WCAG 2.4.3, mirroring
    // `close_map`/`close_station`'s restore-to-trigger).
    let focus_orb = move || {
        #[cfg(feature = "hydrate")]
        if let Some(orb) = orb_ref.get_untracked() {
            let _ = (*orb).focus();
        }
    };
    // Open the blossom from the keyboard. The focus-on-open Effect below resets
    // the roving cursor and lands focus on the first chip once the chips mount.
    let open_blossom_kbd = move || blossom_open.set(true);
    // Focus the first chip when the blossom opens (the Effect re-runs on every
    // open since it reads `blossom_open`), and reset the roving cursor to 0 so
    // the tabindex=0 chip and the actually-focused chip agree even when the
    // pointer-HOLD path (`blossom::BlossomHold`, which can't touch this signal)
    // opened it after a prior keyboard session left the cursor elsewhere.
    // Mirrors `radial_menu`'s focus-on-open.
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        if blossom_open.get() {
            blossom_active.set(0);
            if let Some(menu) = blossom_ref.get() {
                use leptos::wasm_bindgen::JsCast as _;
                if let Some(first) = (*menu)
                    .query_selector(".sk-orbit-chip")
                    .ok()
                    .flatten()
                    .and_then(|el| el.dyn_into::<leptos::web_sys::HtmlElement>().ok())
                {
                    let _ = first.focus();
                }
            }
        }
    });
    // Right-edge HoloPanel slide-over (personas + station). Summoned from the
    // orbit-map DOCK (✦ Personas / ⚙ Station, a-orbit.html #mapDock) — the
    // floating ☰ was removed (1:1: the prototype has no such button; station
    // lives in the map). The single OPEN detent means only on_close dismisses it.
    let station_open = RwSignal::new(false);
    // Gesture-help overlay (a-orbit.html #helpBtn / #hints) — the bottom-left "?"
    // opens a one-card legend of the orbit's gestures.
    let help_open = RwSignal::new(false);
    // Founding (create-server) dialog: open flag + the name buffer. Orbit's
    // ONLY make-a-new-server entry — the M3 rail-add is retired. Opens from the
    // orbit-map dock, where servers (the far-docks) spatially live.
    let founding = RwSignal::new(false);
    let new_server_name = RwSignal::new(String::new());
    // Finding 10 (M7): the help + founding overlays are `aria-modal` dialogs but
    // dismissed ONLY by scrim-tap / button — no Esc, no focus-trap, no
    // restore-to-trigger. Give them the SAME Modal-parity handling the orbit map
    // has (Esc closes, Tab/Shift+Tab wrap within the card via the shared
    // `focusables`, focus lands in the card on open, focus returns to the trigger
    // on close — WCAG 2.4.3 §13). They stay portaled compact cards; the scrim-tap
    // is kept. NodeRefs: the card containers (focus-on-open + Tab-trap root) and
    // the help "?" button (its restore target — the founding dialog's trigger is
    // the in-map dock button, which lives in the map portal, so its restore is
    // best-effort to the document and the trap is what matters there).
    let help_ref = NodeRef::<leptos::html::Div>::new();
    let help_btn_ref = NodeRef::<leptos::html::Button>::new();
    let founding_ref = NodeRef::<leptos::html::Div>::new();
    // Close help + restore focus to the "?" trigger (mirrors `close_map`).
    let close_help = move || {
        help_open.set(false);
        #[cfg(feature = "hydrate")]
        if let Some(btn) = help_btn_ref.get_untracked() {
            let _ = (*btn).focus();
        }
    };
    // Focus the help card when it opens (its `tabindex=-1` makes it focusable; the
    // first Tab then lands on the first control). Mirrors the map's focus-in Effect.
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        if help_open.get() {
            if let Some(card) = help_ref.get() {
                let _ = (*card).focus();
            }
        }
    });
    // Focus the founding card when it opens (so keystrokes — and the name input's
    // first Tab — land in scope; the input is the first focusable).
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        if founding.get() {
            if let Some(card) = founding_ref.get() {
                let _ = (*card).focus();
            }
        }
    });
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
        // Focus returns to the pill (the persistent map entry) — the old ☰
        // summon button is gone; station now opens from the map dock.
        #[cfg(feature = "hydrate")]
        if let Some(pill) = pill_ref.get_untracked() {
            let _ = (*pill).focus();
        }
    };
    // B-fix (owner deck-finding 2026-06-20): a pure LEAVE of the station (back
    // disc / swipe / Esc) returns to the orbit MAP (home), not whatever channel
    // is mounted underneath. The "Go to X" buttons keep bare `close_station()`
    // — they navigate to their own pane/modal and must NOT pop the map.
    let dismiss_station = move || {
        close_station();
        act::show_orbit_map(s);
    };
    view! {
        <section class="content sk-orbit-content"
            // Composer choreography state (M5/P2): `.composing` lives on THIS
            // section (the signal is owned here in SkOrbitShell, not the parent
            // `.app`) — the SCSS keys the composer slide + orb hide + send reveal
            // off `.sk-orbit-content.composing`.
            class:composing=move || composing.get()
            style:--scene-tint=move || scene_tint()
            // Prototype collapse-on-tap-outside (a-orbit.html:845): while composing,
            // a tap on the VOID (not the composer, not a message) slides the
            // composer back down. Message taps fall through to the ChannelPane
            // (whisper reveal / shout reshake); composer + pill taps keep working.
            on:click=move |ev: leptos::ev::MouseEvent| {
                if !composing.get_untracked() {
                    return;
                }
                #[cfg(feature = "hydrate")]
                {
                    use leptos::wasm_bindgen::JsCast as _;
                    let outside = ev
                        .target()
                        .and_then(|t| t.dyn_into::<leptos::web_sys::Element>().ok())
                        .map(|el| {
                            // Exclude the orb: a tap on it is what STARTED composing
                            // (its handler runs first, sets composing=true), and THIS
                            // same click bubbles here — without the orb exclusion we'd
                            // immediately re-collapse it (target is "outside" the
                            // composer). Messages fall through to the ChannelPane.
                            el.closest(".composer").ok().flatten().is_none()
                                && el.closest(".msg").ok().flatten().is_none()
                                && el.closest(".sk-orbit-orb-wrap").ok().flatten().is_none()
                        })
                        .unwrap_or(true);
                    if outside {
                        composing.set(false);
                    }
                }
                #[cfg(not(feature = "hydrate"))]
                let _ = &ev;
            }>
            // F3 cosmic starfield backdrop (Standard-tier; position:fixed,
            // z-index:-1 so it sits behind all content). Static box-shadow dot
            // field + opacity-only twinkle on the `fx-`-classed layers (global
            // reduced-motion kill auto-covers them). Pure decoration.
            <div class="sk-orbit-stars" aria-hidden="true">
                <div class="fx-sk-orbit-stars-a"></div>
                <div class="fx-sk-orbit-stars-b"></div>
            </div>
            // C2 (context-bleed fix 2026-06-22): the channel pill (its "# channel /
            // guild" identity + carousel dots + the map-entry tap) is meaningful
            // ONLY on the channel surface; off-channel it described a channel you'd
            // left AND its tap fell through to re-open the orbit map from every
            // dispatch pane. Gate it to the channel pane — the INVERSE of the
            // back-disc below (which shows off-channel), mirroring that convention.
            // Its map-entry semantics stay intact on the channel.
            {move || (s.sync.pane.get() == Pane::Channel).then(|| view! {
                <button class="sk-orbit-pill" type="button"
                    node_ref=pill_ref
                    aria-haspopup="dialog"
                    aria-expanded=move || map_open.get().to_string()
                    title="Open the orbit map"
                    on:click=move |_| {
                        // Reset any prior dive so the enter-warp scales from centre.
                        dive_origin.set("center".to_string());
                        diving.set(false);
                        map_open.set(true);
                    }>
                    <span class="sk-orbit-pill-hash" aria-hidden="true">{channel_sigil}</span>
                    <span class="sk-orbit-pill-name">{channel_name}</span>
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
                        // Re-homed sync signal (M5/P2 fidelity): the LIVE/POLLING state
                        // lost its topbar chip when the bar was removed to match the
                        // prototype's clean cosmic top; a mint dot on the pill carries
                        // it — and unlike the old `.sync-chip` it stays visible on mobile.
                        <span class="sk-orbit-pill-live"
                            class:live=move || s.sync.sse_live.get()
                            aria-hidden="true"></span>
                    </span>
                </button>
            })}
            // The dispatch panes' back affordance now lives IN the pane header (the
            // `.account-head` bar below) — owner deck-finding 2026-06-22; the
            // floating `.sk-orbit-pane-back` disc is retired.
            // Channel pane: its OWN horizontal swipe strip (neighbor switch). Kept
            // a separate reactive block from the non-channel panes below so the
            // back-swipe surface (Finding 9) never wraps the channel — back-swiping
            // a channel is a neighbor switch, not a dismiss. Mutually exclusive
            // with the non-channel block (both gate on `pane`), so only one mounts.
            {move || (s.sync.pane.get() == Pane::Channel).then(|| {
                    #[cfg(feature = "hydrate")]
                    let d = strip_drag.clone();
                    // Four handles: pointercancel shares the release path with
                    // pointerup (M-35) but needs its OWN clone — both can't move
                    // the same `d_up` into their closures.
                    #[cfg(feature = "hydrate")]
                    let (d_down, d_move, d_up, d_cancel) = (d.clone(), d.clone(), d.clone(), d);
                    view! {
                        // M5/P2 #d single-channel = NO swipe: a 1-channel guild
                        // has nowhere to swipe, so it renders ONLY the current
                        // pane — no prev/next peeks, no "orbit's edge" boundary
                        // (the edge affordance stays for MULTI-channel list edges,
                        // where a real neighbor exists the other way). The
                        // `--single` modifier collapses the 3-slot geometry: the
                        // strip drops to 100vw with no `--strip-x` offset so the
                        // lone cur pane fills the viewport (the resting -100vw
                        // assumes a 3-pane strip with cur in the MIDDLE; with the
                        // prev pane gone, cur becomes the first slot and -100vw
                        // would push it off-screen). The drag engine stays bound
                        // but is inert at count≤1 (both edges true ⇒ no commit).
                        <div class="sk-orbit-strip sk-orbit-strip--snap"
                            class:sk-orbit-strip--single=move || strip::collapses_to_single(chan_count())
                            node_ref=strip_ref
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
                            // are peek previews (lazy first page, NEVER mark read)
                            // — rendered ONLY for multi-channel guilds (a single
                            // channel has no neighbor either way, so no peek panes
                            // mount: #d "no swipe / no orbit's edge").
                            {move || (!strip::collapses_to_single(chan_count())).then(|| view! {
                                <div class="sk-orbit-pane sk-orbit-pane-prev" aria-hidden="true">
                                    {move || neighbor_preview(s, cur_idx().and_then(|i| i.checked_sub(1)))}
                                </div>
                            })}
                            <div class="sk-orbit-pane sk-orbit-pane-cur">
                                <ChannelPane/>
                            </div>
                            {move || (!strip::collapses_to_single(chan_count())).then(|| view! {
                                <div class="sk-orbit-pane sk-orbit-pane-next" aria-hidden="true">
                                    {move || neighbor_preview(s, cur_idx().map(|i| i + 1).filter(|&j| j < chan_count()))}
                                </div>
                            })}
                        </div>
                    }
            })}
            // Finding 9 (M7): the six non-channel dispatch panes, wrapped in the
            // shared swipe-right-to-back surface. One wrapper (`panes_ref` +
            // `pane_swipe`) hosts whichever pane is active; an axis-locked
            // rightward drag past 28% of the viewport (the SAME commit the
            // wardrobe slide-over uses) pops to the orbit MAP — the back disc's
            // target — so leaving these surfaces matches the wardrobe dismiss
            // paradigm instead of the back-button-only divergence. The back disc
            // (above) stays the keyboard/a11y fallback. Inner `match` re-keys the
            // pane off `s.sync.pane`; the wrapper is bound ONCE. Absent on the
            // channel (the channel's own strip block above owns the gesture).
            {move || (s.sync.pane.get() != Pane::Channel).then(|| {
                // Per-build clones (the `.then` closure re-runs on every `pane`
                // change while off-channel, so the engine is cloned each run and
                // the clones move into the handlers — mirrors the Channel strip's
                // `strip_drag.clone()`). pointercancel needs its OWN clone: it and
                // pointerup share the release path (`up`) but can't move the same
                // value into two closures.
                #[cfg(feature = "hydrate")]
                let (ps_down, ps_move, ps_up, ps_cancel) = (
                    pane_swipe.clone(),
                    pane_swipe.clone(),
                    pane_swipe.clone(),
                    pane_swipe.clone(),
                );
                view! {
                <div class="sk-orbit-panes"
                    node_ref=panes_ref
                    on:pointerdown=move |ev| {
                        #[cfg(feature = "hydrate")] ps_down.down(&ev);
                        #[cfg(not(feature = "hydrate"))] let _ = &ev;
                    }
                    on:pointermove=move |ev| {
                        #[cfg(feature = "hydrate")] ps_move.moved(&ev);
                        #[cfg(not(feature = "hydrate"))] let _ = &ev;
                    }
                    on:pointerup=move |ev| {
                        #[cfg(feature = "hydrate")] { if ps_up.up(&ev) { act::show_orbit_map(s); } }
                        #[cfg(not(feature = "hydrate"))] let _ = &ev;
                    }
                    on:pointercancel=move |ev| {
                        #[cfg(feature = "hydrate")] { if ps_cancel.up(&ev) { act::show_orbit_map(s); } }
                        #[cfg(not(feature = "hydrate"))] let _ = &ev;
                    }>
                    // Wardrobe-paradigm pane head (owner deck-finding 2026-06-22):
                    // the dispatch panes get the same sticky `.account-head` bar the
                    // wardrobe/account/server slide-overs use — it OWNS the top
                    // safe-area inset (the gated-off pill used to), and carries the
                    // title + a back-arrow (`.row-edit` -> "<-" via _modal.scss) +
                    // the "swipe -> close" hint. DMs/Cameos pop to Friends (their
                    // parent); the rest pop to the orbit map.
                    <header class="account-head">
                        <h2>{move || match s.sync.pane.get() {
                            Pane::Friends => "Friends",
                            Pane::DirectMessages => "Direct messages",
                            Pane::Cameos => "Cameos",
                            Pane::Members => "Members",
                            Pane::Emoji => "Custom emoji",
                            Pane::Lorebook => "Lorebook",
                            Pane::Channel => "",
                        }}</h2>
                        <button class="row-edit" type="button" aria-label="Back"
                            on:click=move |_| match s.sync.pane.get_untracked() {
                                Pane::DirectMessages | Pane::Cameos => { s.sync.pane.set(Pane::Friends); }
                                _ => { act::show_orbit_map(s); }
                            }><IconBack/></button>
                    </header>
                    {move || match s.sync.pane.get() {
                        Pane::Friends => view! { <FriendsPane/> }.into_any(),
                        Pane::Lorebook => view! { <LorebookPane/> }.into_any(),
                        Pane::Emoji => view! { <EmojiManagerPane/> }.into_any(),
                        Pane::Members => view! { <MembersPane/> }.into_any(),
                        Pane::DirectMessages => view! { <DirectMessagesPane/> }.into_any(),
                        Pane::Cameos => view! { <CameosPane/> }.into_any(),
                        // Unreachable: the outer `.then` gates Channel out. Render
                        // nothing rather than the strip (which lives in its own
                        // block above) — the strip must never mount twice.
                        Pane::Channel => ().into_any(),
                    }}
                </div>
                }
            })}
            <p class="error">{move || s.composer.status.get()}</p>
            {move || map_open.get().then(|| view! {
                <Portal>
                    <div class="sk-orbit-map" role="dialog" aria-modal="true"
                        node_ref=map_ref
                        class:diving=move || diving.get()
                        style:transform-origin=move || dive_origin.get()
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
                        // Cosmic ambience INSIDE the portal: the map is a body-portal
                        // sibling of `.app`, so the fixed app starfield (z:-1 under
                        // `.app`) can't reach it — clone the two twinkle layers here so
                        // the map floats over the same star field as the chat (the SCSS
                        // selectors are `:is(.app.sk-orbit, .sk-orbit-map)`-scoped).
                        <div class="sk-orbit-stars" aria-hidden="true">
                            <div class="fx-sk-orbit-stars-a"></div>
                            <div class="fx-sk-orbit-stars-b"></div>
                        </div>
                        // Map core (the central "star"): the server emblem, its
                        // name, and a live channel count. Split into three spans
                        // so the SCSS can size/stack them independently (NAMING
                        // CONTRACT: glyph/name/sub).
                        <div class="sk-orbit-core">
                            // The nucleus is a luminous STAR (the prototype's per-server
                            // emblem `glyph:"✦"`, a-orbit.html:510/752) — the "sun" of this
                            // solar system, NOT a letter-monogram. Server identity rides the
                            // `.sk-orbit-core-name` label below; authlyn has no per-guild
                            // glyph field yet, so a fixed ✦ is the faithful nucleus.
                            {move || {
                                // Per-server icon as the nucleus when one is set; the ✦
                                // star stays the faithful no-icon fallback (M6/P1).
                                let sid = s.sel.sel_server.get();
                                let icon = s
                                    .sel
                                    .guilds
                                    .get()
                                    .into_iter()
                                    .find(|g| Some(&g.id) == sid.as_ref())
                                    .and_then(|g| g.icon_id);
                                match icon {
                                    Some(id) => view! {
                                        <img class="sk-orbit-core-icon"
                                            src=format!("/media/{id}?w=128") alt=""/>
                                    }
                                    .into_any(),
                                    None => view! {
                                        <span class="sk-orbit-core-glyph" aria-hidden="true"><IconStar/></span>
                                    }
                                    .into_any(),
                                }
                            }}
                            <span class="sk-orbit-core-name">{server_name}</span>
                            <span class="sk-orbit-core-sub">
                                {move || {
                                    let n = chan_count();
                                    format!("{n} channel{}", if n == 1 { "" } else { "s" })
                                }}
                            </span>
                        </div>
                        {move || {
                            // Geometry from the live viewport (UX-equality).
                            let (vw, vh) = viewport_dims();
                            let g = map_geom(vw, vh);
                            let chans = s.sel.channels.get();
                            let n = chans.len();
                            let unread = s.notify.unread.get();
                            let r = g.orbit_radius;
                            // Per-guild seed → every channel's orbit is LOCKED across
                            // refreshes (radius/period/retrograde from the guild seed ^
                            // channel id; orbit_map::channel_orbit, owner ruling
                            // 2026-06-16). The active server id IS the seed.
                            let guild_seed = seed_of(
                                s.sel.sel_server.get().as_deref().unwrap_or_default(),
                            );
                            // Halo rings — the visible orbital TRACKS the nodes ride
                            // (a-orbit.html `.haloRing.r1` solid + `.outer` dashed).
                            // Static, centred on the star; sized to the live radius.
                            let inner_d = format!("{}px", r * 2.0);
                            let outer_d = format!("{}px", (r + 38.0) * 2.0);
                            let nodes = chans.into_iter().enumerate().map(|(i, c)| {
                                // Seeded Kepler orbit: inner channels revolve faster
                                // (period ∝ r^1.5), ~17% retrograde, locked per guild.
                                let orbit = channel_orbit(guild_seed, &c.id, i, n, r);
                                // Start angle for the STATIC placement (so reduced-motion
                                // rests each node at its own angle, not stacked at 0°).
                                let a0 = orbit.y.atan2(orbit.x).to_degrees();
                                let has_unread = unread.contains(&c.id);
                                let ch = c.clone();
                                let is_lore = c.kind == "lorebook";
                                view! {
                                    // Per-node revolving frame (a-orbit.html `.orbit`, but
                                    // each node spins at its OWN --orbit-period instead of
                                    // sharing one 90s ring); `.retro` flips the retrograde
                                    // ones. `.sk-orbit-nodepos` STATICALLY places it at
                                    // (a0, radius); `.sk-orbit-node-in` counter-spins at the
                                    // same period to keep the glyph upright.
                                    <div class="sk-orbit-orbit" class:retro=orbit.retrograde
                                        style:--orbit-period=format!("{:.1}s", orbit.period_s)
                                        style:--orbit-r=format!("{:.1}px", orbit.radius)
                                        style:--orbit-a=format!("{:.1}deg", a0)>
                                    <div class="sk-orbit-nodepos">
                                    <button class="sk-orbit-node"
                                        class:unread=has_unread
                                        title=c.name.clone()
                                        on:click=move |ev: leptos::ev::MouseEvent| {
                                            #[cfg(feature = "hydrate")]
                                            {
                                                use leptos::wasm_bindgen::JsCast as _;
                                                // Zoom INTO the tapped node: transform-origin
                                                // = its screen centre (a-orbit.html enterChannel).
                                                if let Some(el) = ev.current_target().and_then(|t| {
                                                    t.dyn_into::<leptos::web_sys::Element>().ok()
                                                }) {
                                                    let r = el.get_bounding_client_rect();
                                                    dive_origin.set(format!(
                                                        "{}px {}px",
                                                        r.x() + r.width() / 2.0,
                                                        r.y() + r.height() / 2.0
                                                    ));
                                                }
                                                diving.set(true);
                                                act::open_channel(s, ch.clone());
                                                // Defer the un-mount so the dive (scale 3.4 +
                                                // fade, .55s) plays before the portal drops
                                                // (a-orbit.html defers 580ms), then restore
                                                // focus to the pill (now the entered channel).
                                                leptos::task::spawn_local(async move {
                                                    gloo_timers::future::TimeoutFuture::new(560)
                                                        .await;
                                                    map_open.set(false);
                                                    diving.set(false);
                                                    if let Some(pill) = pill_ref.get_untracked() {
                                                        let _ = (*pill).focus();
                                                    }
                                                });
                                            }
                                            #[cfg(not(feature = "hydrate"))]
                                            {
                                                let _ = &ev;
                                                act::open_channel(s, ch.clone());
                                                map_open.set(false);
                                            }
                                        }>
                                        <span class="sk-orbit-node-in">
                                            <span class="sk-orbit-node-hash" aria-hidden="true">
                                                {if is_lore {
                                                    view! { <IconBook/> }.into_any()
                                                } else {
                                                    view! { "#" }.into_any()
                                                }}
                                            </span>
                                            <span class="sk-orbit-node-name">{c.name.clone()}</span>
                                            {has_unread.then(|| view! { <span class="sk-orbit-node-dot" aria-hidden="true"></span> })}
                                        </span>
                                    </button>
                                    </div>
                                    </div>
                                }
                            }).collect_view();
                            view! {
                                <div class="sk-orbit-halo sk-orbit-halo-inner" aria-hidden="true"
                                    style:width=inner_d.clone() style:height=inner_d></div>
                                <div class="sk-orbit-halo sk-orbit-halo-outer" aria-hidden="true"
                                    style:width=outer_d.clone() style:height=outer_d></div>
                                <div class="sk-orbit-ring">{nodes}</div>
                            }
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
                                                // Switch worlds but STAY in the orbit
                                                // view (owner ruling): select + load
                                                // the server's channels, no dive, map
                                                // stays open so its channels orbit.
                                                // `select_server_for_sheet` is the
                                                // select-without-auto-open primitive
                                                // (tap a node to enter).
                                                act::select_server_for_sheet(s, gid.clone());
                                            }>
                                            // Mini-nucleus: the monogram emblem
                                            // distinguishes each far server (authlyn has
                                            // no per-guild glyph; the active core keeps
                                            // the ✦ star), name labelled below the disc.
                                            {match gd.icon_id.clone() {
                                                Some(id) => view! {
                                                    <img class="sk-orbit-far-icon"
                                                        src=format!("/media/{id}?w=64") alt=""/>
                                                }
                                                .into_any(),
                                                None => view! {
                                                    <span class="sk-orbit-far-glyph" aria-hidden="true">
                                                        {crate::ui::avatar::monogram(&gd.name, '#')}
                                                    </span>
                                                }
                                                .into_any(),
                                            }}
                                            <span class="sk-orbit-far-name">{gd.name.clone()}</span>
                                        </button>
                                    }
                                }).collect_view()
                        }}
                        // Map dock (a-orbit.html #mapDock): the in-map entry to
                        // personas + station — the prototype's home for it,
                        // replacing the removed floating ☰. The two buttons lead to
                        // DISTINCT destinations (owner deck-finding 2026-06-20: they
                        // formerly both opened the same slide-over): "Personas" goes
                        // straight into the wardrobe (persona management → map on
                        // dismiss), "Station" opens the station slide-over (per-channel
                        // wear-grid + Go to + Server + Account).
                        <div class="sk-orbit-map-dock">
                            <button class="sk-orbit-sat" type="button"
                                on:click=move |_| { close_map(); act::show_wardrobe(s); }>
                                <IconPersonas/>" Personas"
                            </button>
                            <button class="sk-orbit-sat" type="button"
                                on:click=move |_| { close_map(); station_open.set(true); }>
                                <IconSettings/>" Station"
                            </button>
                            // Found a new server (orbit's only create-server
                            // entry; M3's rail-add is retired). Opens the
                            // founding dialog over the shell.
                            <button class="sk-orbit-sat" type="button"
                                on:click=move |_| { new_server_name.set(String::new()); founding.set(true); }>
                                <IconPlus/>" New server"
                            </button>
                        </div>
                        <p class="sk-orbit-map-hint" aria-hidden="true">"tap a node to enter"</p>
                    </div>
                </Portal>
            })}
            // C2 (context-bleed fix 2026-06-22): the compose orb (and the effect
            // blossom + tap-away scrim nested in this wrap) is a CHANNEL affordance
            // — off-channel it bled the same way the pill did and its tap revealed a
            // composer for a surface (Friends/Members/Emoji/…) that has no message
            // input. Gate the whole wrap to the channel pane (same convention as the
            // pill above); one gate on the wrapper removes the orb, blossom AND its
            // scrim off-channel. The bottom-left help "?" FAB lives OUTSIDE this wrap
            // and stays on every pane.
            {move || (s.sync.pane.get() == Pane::Channel).then(|| view! {
            <div class="sk-orbit-orb-wrap">
                <button type="button"
                    node_ref=orb_ref
                    // Leptos `Attribute`/`RenderHtml` are impl'd for attribute
                    // tuples only up to a fixed arity (≤26); ONE element over the
                    // ceiling drops the whole tuple's trait impl and rustc reports
                    // it as a cascade (`IntoClass not satisfied` on a `class:`
                    // slot, `RenderHtml not satisfied` on the <button>, and
                    // `cannot find value ev` in the `on:` closures). This button
                    // carries ~17 attrs/handlers, so the three class slots are
                    // collapsed into ONE reactive `class=move || String` and the
                    // two `style:` vars into ONE `style=move || String` (the
                    // documented `class=move || …` / `style=move || …` form) to
                    // stay under the limit. SCSS still keys off `.sk-orbit-orb` /
                    // `.charging` (charge.get()>0) / `.full` (prototype
                    // `#sendBtn.full`, a-orbit.html:266 — brightened arc+glow at
                    // the top of the length curve, distinct from the `data-armed`
                    // tint below) and the `--charge`/`--dash` custom props.
                    class=move || {
                        let c = charge.get();
                        let mut s = String::from("sk-orbit-orb");
                        if c > 0.0 {
                            s.push_str(" charging");
                        }
                        if c >= 1.0 {
                            s.push_str(" full");
                        }
                        s
                    }
                    style=move || {
                        let c = charge.get();
                        format!("--charge:{:.3};--dash:{:.1}", c, self::charge::dash_offset(c))
                    }
                    // Effect tint (prototype `#orb[data-armed="…"]`,
                    // a-orbit.html:222-224): emit the ARMED effect NAME
                    // (whisper|shout|spell) so the SCSS can border/glow/recolor
                    // the orb per effect; empty string when unarmed clears it
                    // (an absent value would leave the last tint stuck). This is
                    // the orb↔SCSS naming contract — `class:armed` was a boolean
                    // that couldn't carry WHICH effect. Custom `data-*` attrs need
                    // Leptos 0.8's `attr:` prefix (only TYPED attrs like `aria-*`
                    // go bare — a bare `data-armed=closure` fails to typecheck and
                    // poisons the whole attribute chain); an empty value renders
                    // `data-armed=""`, which matches no
                    // `[data-armed="whisper|shout|spell"]` rule, so the tint clears.
                    attr:data-armed=move || s.composer.effect_mode.get().unwrap_or_default()
                    // a11y: announce the orb as the trigger for the effect
                    // `role="menu"` blossom (WCAG 4.1.2) and expose its state.
                    aria-haspopup="menu"
                    aria-expanded=move || blossom_open.get().to_string()
                    title="Send (hold or press ↑ for effects)"
                    // Keyboard OPEN path (a11y): the orb is the SOLE effect
                    // surface and the pointer-HOLD open is unreachable by
                    // keyboard, so ArrowUp/ArrowDown open the blossom (the chips
                    // fan UP from the orb, so ↑ is the natural reveal). Enter /
                    // Space stay the SEND activation (the button's default click),
                    // matching the pointer tap — never hijacked to open effects.
                    on:keydown=move |ev: leptos::ev::KeyboardEvent| {
                        match ev.key().as_str() {
                            "ArrowUp" | "ArrowDown" => {
                                ev.prevent_default();
                                open_blossom_kbd();
                            }
                            _ => {}
                        }
                    }
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
                        // (it opened the effect blossom, it must not also compose).
                        if hold.take_fired() {
                            return;
                        }
                        // Prototype choreography (a-orbit.html:859 expandComposer):
                        // the orb REVEALS the composer (it no longer sends) and
                        // hides itself (SCSS `.composing`); the in-composer send
                        // button commits. Focus the textarea so the keyboard rises.
                        composing.set(true);
                        #[cfg(feature = "hydrate")]
                        {
                            use leptos::wasm_bindgen::JsCast as _;
                            if let Some(el) = leptos::web_sys::window()
                                .and_then(|w| w.document())
                                .and_then(|d| {
                                    d.query_selector(".app.sk-orbit .composer textarea")
                                        .ok()
                                        .flatten()
                                })
                            {
                                if let Ok(t) = el.dyn_into::<leptos::web_sys::HtmlElement>() {
                                    let _ = t.focus();
                                }
                            }
                        }
                    }>
                    <svg class="sk-orbit-ring" viewBox="0 0 52 52" aria-hidden="true">
                        <circle class="sk-orbit-ring-track" cx="26" cy="26" r="24"></circle>
                        <circle class="sk-orbit-ring-arc" cx="26" cy="26" r="24"></circle>
                    </svg>
                    <span class="sk-orbit-orb-glyph">{move || {
                        // IconEdit compose pen when unarmed (the prototype's
                        // #orbGlyph, a-orbit.html:416) — the orb opens the composer;
                        // the armed-effect icon (whisper/shout/spell) still wins when
                        // an effect is loaded, so you see what the next message carries.
                        match s.composer.effect_mode.get().as_deref() {
                            Some("whisper") => view! { <IconWhisper/> }.into_any(),
                            Some("shout") => view! { <IconShout/> }.into_any(),
                            Some("spell") => view! { <IconSpell/> }.into_any(),
                            _ => view! { <IconEdit/> }.into_any(),
                        }
                    }}</span>
                </button>
                {move || blossom_open.get().then(|| {
                    // (effect name, English label). The per-effect ICON is derived
                    // from the name below (IconWhisper / IconShout / IconSpell,
                    // matching the orb badge); labels are ENGLISH (UI-copy rule) —
                    // the SCSS uppercases them.
                    let chips = [
                        ("whisper", "whisper"),
                        ("shout", "shout"),
                        ("spell", "spell"),
                    ];
                    view! {
                        // Tap-outside dismiss layer (mirrors `.sk-orbit-map-scrim`):
                        // a transparent full-viewport button that closes the
                        // blossom on click. Without it a pointer user who opened
                        // the blossom (480ms hold) had no tap-away dismiss — only
                        // picking a chip closed it. DOM-first + lower z-index than
                        // the chips (SCSS), so the chips stay clickable on top.
                        <button class="sk-orbit-blossom-scrim" aria-label="Dismiss effects"
                            tabindex="-1"
                            on:click=move |_| blossom_open.set(false)></button>
                        <div class="sk-orbit-blossom" role="menu" aria-label="Message effect"
                            node_ref=blossom_ref
                            // Roving-tabindex menu keyboard model (WAI-ARIA menu):
                            // ArrowUp/Down move the roving cursor (`blossom_active`)
                            // and focus that chip; Escape closes + refocuses the
                            // orb. The chips fan UP via flex column-reverse, so
                            // ArrowUp walks toward higher DOM indices (spell) and
                            // ArrowDown toward lower (whisper) — matching the eye.
                            on:keydown=move |ev: leptos::ev::KeyboardEvent| {
                                match ev.key().as_str() {
                                    "Escape" => {
                                        ev.prevent_default();
                                        blossom_open.set(false);
                                        focus_orb();
                                    }
                                    "ArrowUp" | "ArrowDown" => {
                                        ev.prevent_default();
                                        let up = ev.key() == "ArrowUp";
                                        // 3 chips; wrap at both ends.
                                        let cur = blossom_active.get();
                                        let next = if up {
                                            (cur + 1) % 3
                                        } else {
                                            (cur + 3 - 1) % 3
                                        };
                                        blossom_active.set(next);
                                        #[cfg(feature = "hydrate")]
                                        if let Some(menu) = blossom_ref.get_untracked() {
                                            use leptos::wasm_bindgen::JsCast as _;
                                            if let Ok(list) =
                                                (*menu).query_selector_all(".sk-orbit-chip")
                                            {
                                                if let Some(el) = list.item(next as u32).and_then(
                                                    |n| {
                                                        n.dyn_into::<leptos::web_sys::HtmlElement>()
                                                            .ok()
                                                    },
                                                ) {
                                                    let _ = el.focus();
                                                }
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }>
                            {chips.into_iter().enumerate().map(|(i, (name, label))| {
                                let name_owned = name.to_string();
                                let glyph = match name {
                                    "shout" => view! { <IconShout/> }.into_any(),
                                    "spell" => view! { <IconSpell/> }.into_any(),
                                    _ => view! { <IconWhisper/> }.into_any(),
                                };
                                view! {
                                    <button class="sk-orbit-chip" role="menuitem"
                                        // Per-effect class so the SCSS can tint
                                        // each chip its effect color (prototype
                                        // `.fxBtn.whisper/.shout/.spell`,
                                        // a-orbit.html:238-240). Exactly one of the
                                        // three is true per chip — the contract the
                                        // SCSS keys off.
                                        class:whisper=name == "whisper"
                                        class:shout=name == "shout"
                                        class:spell=name == "spell"
                                        // Roving tabindex: exactly one chip is in
                                        // the Tab order (the active cursor); arrows
                                        // move within. Reactive so it tracks the
                                        // cursor as ArrowUp/Down rove.
                                        tabindex=move || if blossom_active.get() == i { "0" } else { "-1" }
                                        style:--chip-i=i.to_string()
                                        title=name
                                        on:click=move |_| {
                                            // Arm this effect AND carry straight into
                                            // compose (spec §1: the effect blossom arms
                                            // the next message). A deliberate pick SETS
                                            // the mode (not toggle — a toggle-off would
                                            // open the composer with no effect, the
                                            // round-4 "redundant" complaint), closes the
                                            // blossom, then reveals the composer + focuses
                                            // the textarea — the SAME open recipe the orb
                                            // tap uses above — so the user writes with the
                                            // effect armed in one gesture instead of being
                                            // dropped back on the resting orb.
                                            s.composer.effect_mode.set(Some(name_owned.clone()));
                                            blossom_open.set(false);
                                            composing.set(true);
                                            #[cfg(feature = "hydrate")]
                                            {
                                                use leptos::wasm_bindgen::JsCast as _;
                                                if let Some(el) = leptos::web_sys::window()
                                                    .and_then(|w| w.document())
                                                    .and_then(|d| {
                                                        d.query_selector(".app.sk-orbit .composer textarea")
                                                            .ok()
                                                            .flatten()
                                                    })
                                                {
                                                    if let Ok(t) = el.dyn_into::<leptos::web_sys::HtmlElement>() {
                                                        let _ = t.focus();
                                                    }
                                                }
                                            }
                                        }>
                                        // Split glyph + label (prototype's
                                        // `.fxBtn > .ic + .lb`, a-orbit.html:420)
                                        // so the SCSS stacks them; ENGLISH label.
                                        <span class="sk-orbit-chip-ic" aria-hidden="true">{glyph}</span>
                                        <span class="sk-orbit-chip-lb">{label}</span>
                                    </button>
                                }
                            }).collect_view()}
                        </div>
                    }
                })}
            </div>
            })}
            // Help button (a-orbit.html #helpBtn) — bottom-left, balances the
            // bottom-right orb; opens the gesture-hints overlay. Hidden while
            // composing (SCSS) so it never crowds the keyboard.
            <button class="sk-orbit-help" type="button"
                node_ref=help_btn_ref
                aria-haspopup="dialog"
                aria-expanded=move || help_open.get().to_string()
                title="Gesture help"
                on:click=move |_| help_open.set(true)>"?"</button>
            {move || help_open.get().then(|| view! {
                <Portal>
                    // Scrim-tap kept; routed through `close_help` so the dismiss
                    // restores focus to the "?" trigger (§13), same as Esc.
                    <div class="sk-orbit-hints" role="dialog" aria-modal="true"
                        aria-label="Gesture help"
                        on:click=move |_| close_help()>
                        // The card is the focus-trap root + focus-on-open target
                        // (Finding 10): `tabindex=-1` makes it focusable, the Esc
                        // closes (restoring the trigger), and Tab/Shift+Tab wrap
                        // within via the shared `trap_tab` — mirroring the map.
                        <div class="sk-orbit-hints-card"
                            node_ref=help_ref tabindex="-1"
                            on:click=move |ev: leptos::ev::MouseEvent| ev.stop_propagation()
                            on:keydown=move |ev: leptos::ev::KeyboardEvent| {
                                match ev.key().as_str() {
                                    "Escape" => {
                                        ev.prevent_default();
                                        close_help();
                                    }
                                    "Tab" => {
                                        #[cfg(feature = "hydrate")]
                                        if let Some(card) = help_ref.get_untracked() {
                                            use leptos::wasm_bindgen::JsCast as _;
                                            let root: &leptos::web_sys::Element =
                                                (*card).unchecked_ref();
                                            if trap_tab(root, ev.shift_key()) {
                                                ev.prevent_default();
                                            }
                                        }
                                        #[cfg(not(feature = "hydrate"))]
                                        let _ = &ev;
                                    }
                                    _ => {}
                                }
                            }>
                            <h2>"Orbit"</h2>
                            <p class="sk-orbit-hints-sub">"You navigate space, not menus"</p>
                            <div class="sk-orbit-hints-rows">
                                {[
                                    ("swipe", "Swipe sideways", "Switch between channels"),
                                    ("orb", "The orb", "Tap to write — hold for whisper / shout / spell"),
                                    ("hold", "Hold a message", "Reply, react, edit or delete"),
                                    ("star", "Tap the pill", "Open the orbit map — pick a channel or world"),
                                    ("whisper", "Whispers", "Touch a whisper to listen"),
                                ].into_iter().map(|(g, t, d)| {
                                    let glyph = match g {
                                        "swipe" => view! { <IconSwipe/> }.into_any(),
                                        "orb" => view! { <IconOrb/> }.into_any(),
                                        "hold" => view! { <IconHold/> }.into_any(),
                                        "star" => view! { <IconStar/> }.into_any(),
                                        _ => view! { <IconWhisper/> }.into_any(),
                                    };
                                    view! {
                                    <div class="sk-orbit-hint-row">
                                        <span class="sk-orbit-hint-g" aria-hidden="true">{glyph}</span>
                                        <span class="sk-orbit-hint-t">
                                            <b>{t}</b>
                                            <span>{d}</span>
                                        </span>
                                    </div>
                                    }
                                }).collect_view()}
                            </div>
                            <button class="sk-orbit-hints-ok" type="button"
                                on:click=move |_| close_help()>"Got it "<IconStar/></button>
                        </div>
                    </div>
                </Portal>
            })}
            // Founding dialog (create-server) — orbit's only "make a new server"
            // entry, opened from the map dock. Mirrors the help overlay's
            // portal/card; on submit calls act::create_server, which refreshes
            // the guild list so the new world appears in the map.
            {move || founding.get().then(|| view! {
                <Portal>
                    <div class="sk-orbit-hints" role="dialog" aria-modal="true"
                        aria-label="Found a new server"
                        on:click=move |_| founding.set(false)>
                        // Esc + Tab-trap + focus-on-open (Finding 10), mirroring
                        // the help card and the orbit map. The card is the trap
                        // root; the name input is its first focusable.
                        <div class="sk-orbit-hints-card"
                            node_ref=founding_ref tabindex="-1"
                            on:click=move |ev: leptos::ev::MouseEvent| ev.stop_propagation()
                            on:keydown=move |ev: leptos::ev::KeyboardEvent| {
                                match ev.key().as_str() {
                                    "Escape" => {
                                        ev.prevent_default();
                                        founding.set(false);
                                    }
                                    "Tab" => {
                                        #[cfg(feature = "hydrate")]
                                        if let Some(card) = founding_ref.get_untracked() {
                                            use leptos::wasm_bindgen::JsCast as _;
                                            let root: &leptos::web_sys::Element =
                                                (*card).unchecked_ref();
                                            if trap_tab(root, ev.shift_key()) {
                                                ev.prevent_default();
                                            }
                                        }
                                        #[cfg(not(feature = "hydrate"))]
                                        let _ = &ev;
                                    }
                                    _ => {}
                                }
                            }>
                            <h2>"Found a world"</h2>
                            <p class="sk-orbit-hints-sub">"Name your new server"</p>
                            <input class="sk-orbit-found-input" type="text"
                                prop:value=move || new_server_name.get()
                                placeholder="server name"
                                on:input=move |ev| new_server_name.set(event_target_value(&ev))/>
                            <button class="sk-orbit-hints-ok" type="button"
                                on:click=move |_| {
                                    let name = new_server_name.get_untracked();
                                    if name.trim().is_empty() {
                                        return;
                                    }
                                    new_server_name.set(String::new());
                                    founding.set(false);
                                    act::create_server(s, name);
                                }>"Found "<IconStar/></button>
                        </div>
                    </div>
                </Portal>
            })}
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
                    on_close=move |_| dismiss_station()
                >
                    <div class="sk-orbit-station">
                        <button class="sk-orbit-station-close" title="Close" aria-label="Close"
                            on:click=move |_| dismiss_station()><IconBack/></button>
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
                        // Parity wiring (M5): the full wardrobe editor (create/edit/
                        // delete personas) opens the shared wardrobe Modal
                        // (shell/mod.rs); the station grid above only wears/switches.
                        <button class="sk-orbit-account-btn" type="button"
                            on:click=move |_| { close_station(); act::show_wardrobe(s); }>
                            <IconPersonas/>" Manage personas"
                        </button>
                        // Parity wiring (M5): the Friends / Members / Emoji panes
                        // mount in the orbit pane-dispatch but had NO orbit entry
                        // point (dead arms) — these surface them via the shared act::
                        // helpers (same `s.sync.pane` set the M3 chrome triggers,
                        // shell/mod.rs:471/584/588). Members + Custom emoji are
                        // guild-scoped (read the active server); Friends is account-
                        // global. Close the station first so the pane shows unscrimmed.
                        <h2>"Go to"</h2>
                        <button class="sk-orbit-account-btn" type="button"
                            on:click=move |_| { close_station(); act::show_friends(s); }>
                            <IconFriends/>" Friends"
                        </button>
                        // B3 (owner deck-finding 2026-06-20): DMs were reachable
                        // ONLY via Friends → "Direct messages →" (a buried 2nd hop).
                        // A direct station entry mirrors the Friends/Members/Emoji
                        // pattern (`act::show_dms` = the show_friends sibling).
                        <button class="sk-orbit-account-btn" type="button"
                            on:click=move |_| { close_station(); act::show_dms(s); }>
                            <IconChat/>" Direct messages"
                        </button>
                        <button class="sk-orbit-account-btn" type="button"
                            on:click=move |_| { close_station(); act::show_members(s); }>
                            <IconMembers/>" Members"
                        </button>
                        <button class="sk-orbit-account-btn" type="button"
                            on:click=move |_| { close_station(); act::show_emoji_manager(s); }>
                            <IconEmoji/>" Custom emoji"
                        </button>
                        // Parity wiring (M5): owner-only server management lives in
                        // the ServerModal (accent / invitations / channels) — the
                        // guild-owner sibling of "Account & preferences", opened via
                        // the shared `server_open` signal. Hidden for non-owners; each
                        // server route the modal calls re-checks require_manager.
                        {move || is_owner().then(|| view! {
                            <h2>"Server"</h2>
                            <button class="sk-orbit-account-btn" type="button"
                                on:click=move |_| {
                                    close_station();
                                    // Clear the per-action `composer.status` so a
                                    // transient from another surface (e.g. a cameo
                                    // invite error) doesn't bleed into the Server
                                    // modal's status line — mirrors the Account
                                    // open path below (Finding 19).
                                    s.composer.status.set(String::new());
                                    server_open.set(true);
                                }>
                                <IconSettings/>" Server settings"
                            </button>
                        })}
                        // F2 account-trap fix: the orbit chrome has NO topbar
                        // gear, so WITHOUT this the user is trapped — no way to
                        // reach Account & preferences (and therefore no Log out
                        // and no skeleton switch back to the chooser). This opens
                        // the shell-wide AccountModal (skeleton-independent,
                        // mounted in AppShell), which is the canonical home for
                        // BOTH "Log out" (mobile finding #50a) and the skeleton
                        // picker. Close the station first so focus/scrim don't
                        // stack, then flip the shared signal.
                        <h2>"Account"</h2>
                        <button class="sk-orbit-account-btn" type="button"
                            on:click=move |_| {
                                close_station();
                                s.composer.status.set(String::new());
                                account_open.set(true);
                            }>
                            <IconSettings/>" Account & preferences"
                        </button>
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
/// `idx == None` (no neighbor at a true list edge) renders a deliberate
/// "orbit's edge" affordance — NEVER an empty pane, so the edge rubber-band
/// reveals a designed boundary instead of the black void (M5/P2 void-fix).
fn neighbor_preview(s: Shell, idx: Option<usize>) -> impl IntoView {
    let label = idx
        .and_then(|i| s.sel.channels.get().get(i).map(|c| c.name.clone()))
        .unwrap_or_default();
    view! {
        <div class="sk-orbit-peek">
            {if label.is_empty() {
                // No neighbor this way (first / last / only channel): a designed
                // orbital boundary, never a blank pane for the rubber-band to
                // expose. aria-hidden rides the parent `.sk-orbit-pane-*`.
                view! {
                    <div class="sk-orbit-peek-edge">
                        <span class="sk-orbit-peek-edge-ring"></span>
                        <span class="sk-orbit-peek-edge-text">"orbit's edge"</span>
                    </div>
                }
                .into_any()
            } else {
                view! { <span class="sk-orbit-peek-name">"# "{label}</span> }.into_any()
            }}
        </div>
    }
}
