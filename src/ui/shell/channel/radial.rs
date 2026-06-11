//! W4/T4 radial long-press action menu (touch). On coarse-pointer devices a
//! ~450ms press on a `.msg` row blossoms a glass arc of action buttons —
//! reply/copy for every message, plus edit/delete for the viewer's own —
//! around the touch point, replacing the hover-revealed `.msg-actions` row
//! (hidden under `(pointer: coarse)` in `_content.scss`). Desktop is
//! untouched: fine-pointer devices keep the hover row and never arm the
//! long-press (`is_touch()` gate); right-click stays native.
//!
//! Always-on module: the render fn compiles in both graphs (the buttons call
//! `act::` handlers, which carry ssr no-op stubs); only the [`LongPress`]
//! timer plumbing is hydrate-gated.

use leptos::prelude::*;

use super::super::{act, PendingDelete, Shell};
use crate::protocol::MessageEnvelope;
use crate::ui::icons::{IconCopy, IconEdit, IconReply, IconTrash};

/// Open-menu state: the long-pressed message (the full envelope — reply
/// needs it for the banner preview, edit/copy need the body), the channel,
/// whether the viewer owns it (gates edit/delete), and the press point
/// already clamped inside the viewport so the arc never overflows an edge.
#[derive(Clone, Debug)]
pub(super) struct RadialState {
    pub m: MessageEnvelope,
    pub cid: Option<String>,
    pub mine: bool,
    pub x: f64,
    pub y: f64,
}

/// Press-to-open delay. Below iOS Safari's ~500ms native long-press so the
/// radial wins the race (the `contextmenu` suppression + the coarse-pointer
/// `user-select: none` in CSS cover the browsers that fire earlier).
#[cfg(feature = "hydrate")]
const LONG_PRESS_MS: u32 = 450;

/// Pointer drift (px) past which a pending press is treated as a scroll /
/// drag and cancelled. Generous enough for finger jitter; the browser's own
/// `pointercancel` (fired when it claims the gesture for panning) is the
/// backstop.
#[cfg(feature = "hydrate")]
const MOVE_SLOP_PX: f64 = 10.0;

/// Long-press bookkeeping shared by every `.msg` row, created once per
/// `ChannelPane`. Generation-counter pattern instead of a cancellable timer
/// handle: `down` bumps the generation and detaches a 450ms sleep that only
/// opens the menu if the generation is still current when it wakes;
/// `pointermove` (beyond [`MOVE_SLOP_PX`]), `pointerup`, and `pointercancel`
/// bump it again, turning the sleeping future into a no-op. This avoids
/// holding a `!Send` timer handle (gloo's `Timeout`) in arena storage and
/// needs no `wasm_bindgen::Closure`/`forget()` — the future owns its
/// captures and drops them when it completes. The async tail uses the
/// `try_*` accessors so a press that outlives the pane (channel/guild
/// switch mid-hold disposes the owner) degrades to a no-op, never a panic.
#[cfg(feature = "hydrate")]
#[derive(Clone, Copy)]
pub(super) struct LongPress {
    /// Bumped on every down/cancel; the armed future captures the value it
    /// was spawned with and only fires while it still matches.
    gen: StoredValue<u64>,
    /// Press origin while a press is armed (drift measurement); `None` once
    /// cancelled or fired.
    origin: StoredValue<Option<(f64, f64)>>,
}

#[cfg(feature = "hydrate")]
impl LongPress {
    pub fn new() -> Self {
        Self {
            gen: StoredValue::new(0),
            origin: StoredValue::new(None),
        }
    }

    /// `pointerdown` on a `.msg`: arm the long-press timer (touch only —
    /// desktop returns immediately, keeping mouse selection/drag native).
    pub fn down(
        self,
        ev: &leptos::ev::PointerEvent,
        radial: RwSignal<Option<RadialState>>,
        m: MessageEnvelope,
        cid: Option<String>,
        mine: bool,
    ) {
        if !super::is_touch() {
            return;
        }
        let gen = self.gen.get_value().wrapping_add(1);
        self.gen.set_value(gen);
        let (x, y) = (ev.client_x() as f64, ev.client_y() as f64);
        self.origin.set_value(Some((x, y)));
        leptos::task::spawn_local(async move {
            gloo_timers::future::TimeoutFuture::new(LONG_PRESS_MS).await;
            // Still the same un-cancelled press? (try_*: the pane may have
            // been disposed while we slept.)
            if self.gen.try_get_value() != Some(gen) {
                return;
            }
            // Disarm so the finger lifting off the now-open menu doesn't
            // read as a drift-cancel of a press that already fired.
            self.origin.try_set_value(None);
            let (cx, cy) = clamp_to_viewport(x, y);
            let _ = radial.try_set(Some(RadialState {
                m,
                cid,
                mine,
                x: cx,
                y: cy,
            }));
        });
    }

    /// `pointermove` on a `.msg`: drift beyond the slop radius means the
    /// user is scrolling/dragging, not pressing — cancel the pending press.
    /// (Cheap no-op on desktop hover: `origin` is only ever set on touch.)
    pub fn moved(self, ev: &leptos::ev::PointerEvent) {
        if let Some((ox, oy)) = self.origin.get_value() {
            let dx = ev.client_x() as f64 - ox;
            let dy = ev.client_y() as f64 - oy;
            if dx * dx + dy * dy > MOVE_SLOP_PX * MOVE_SLOP_PX {
                self.cancel();
            }
        }
    }

    /// `pointerup` / `pointercancel`: disarm any pending press. Idempotent —
    /// after the menu has opened this bumps the generation harmlessly.
    pub fn cancel(self) {
        self.gen.update_value(|g| *g = g.wrapping_add(1));
        self.origin.set_value(None);
    }
}

/// Clamp the press point so the whole arc stays on-screen: the buttons
/// reach `64px radius + 22px half-button = 86px` past the anchor sideways
/// and upward (the arc opens upward only), so pad horizontally and keep the
/// anchor below the topbar. The `.max()` guards keep `clamp` panic-free on
/// degenerate (tiny) viewports.
#[cfg(feature = "hydrate")]
fn clamp_to_viewport(x: f64, y: f64) -> (f64, f64) {
    let (vw, vh) = leptos::web_sys::window()
        .map(|w| {
            (
                w.inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(0.0),
                w.inner_height()
                    .ok()
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0),
            )
        })
        .unwrap_or((0.0, 0.0));
    let margin_x = 96.0;
    let top = 120.0;
    (
        x.clamp(margin_x, (vw - margin_x).max(margin_x)),
        y.clamp(top, (vh - 16.0).max(top)),
    )
}

/// Render the radial menu while `radial` is `Some`: a full-screen scrim
/// backdrop plus a zero-size anchor `<div>` fixed at the (clamped) press
/// point whose buttons fan out on an upward arc via per-slot `--ang`
/// transforms (`_content.scss`); `fx-blossom` (`_motion.scss`) scales the
/// whole arc out of the anchor on open. Every button dispatches the SAME
/// `act::` handler as its hover-row counterpart in `meta.rs`, then closes.
/// Icons are `aria-hidden` (see `icons.rs`), so each button carries an
/// explicit `aria-label` (W3 review convention).
pub(super) fn radial_menu(s: Shell, radial: RwSignal<Option<RadialState>>) -> impl IntoView {
    move || {
        radial.get().map(|st| {
            let arc_class = if st.mine {
                "radial-menu n4"
            } else {
                "radial-menu n2"
            };
            let pos = format!("left:{:.0}px;top:{:.0}px", st.x, st.y);

            // Dismissal must ignore the click manufactured by the OPENING
            // press: on touch, the compat mouse events (mousedown/up/click)
            // fire at the release point AFTER pointerup — by then this
            // backdrop is on top, so a sub-500ms long-press would open the
            // menu and instantly close it on finger-lift. Arm the backdrop
            // only once it has seen a `pointerdown` of its own (a genuinely
            // NEW tap); the opening press never pointer-downs on the
            // backdrop, so its stray click is ignored, while a real
            // dismissal tap closes as expected — and is consumed by the
            // still-mounted backdrop, so nothing below ghost-clicks.
            let armed = StoredValue::new(false);

            let reply_m = st.m.clone();
            let copy_body = st.m.body.clone();
            let own_btns = st.mine.then(|| {
                let edit_cid = st.cid.clone();
                let edit_mid = st.m.id.clone();
                let edit_body = st.m.body.clone();
                let del_cid = st.cid.clone();
                let del_mid = st.m.id.clone();
                view! {
                    <button class="radial-btn" aria-label="edit message"
                        on:click=move |_| {
                            if let Some(cid) = edit_cid.clone() {
                                act::start_edit(s, cid, edit_mid.clone(), edit_body.clone());
                            }
                            radial.set(None);
                        }>
                        <IconEdit/>
                    </button>
                    <button class="radial-btn danger" aria-label="delete message"
                        on:click=move |_| {
                            if let Some(cid) = del_cid.clone() {
                                // Same confirm-unless-opted-out branch as the
                                // hover 🗑 in meta.rs.
                                if act::confirm_delete_message_enabled() {
                                    act::ask_delete(
                                        s,
                                        "Delete this message? This cannot be undone."
                                            .to_string(),
                                        PendingDelete::Message {
                                            cid,
                                            mid: del_mid.clone(),
                                        },
                                    );
                                } else {
                                    act::delete_message(s, cid, del_mid.clone());
                                }
                            }
                            radial.set(None);
                        }>
                        <IconTrash/>
                    </button>
                }
            });

            view! {
                <div class="radial-backdrop"
                    on:pointerdown=move |_| armed.set_value(true)
                    on:click=move |_| {
                        if armed.get_value() {
                            radial.set(None);
                        }
                    }></div>
                <div class=arc_class style=pos aria-label="message actions">
                    <button class="radial-btn" aria-label="reply"
                        on:click=move |_| {
                            act::start_reply(s, reply_m.clone());
                            radial.set(None);
                        }>
                        <IconReply/>
                    </button>
                    <button class="radial-btn" aria-label="copy message text"
                        on:click=move |_| {
                            act::copy_message_body(s, copy_body.clone());
                            radial.set(None);
                        }>
                        <IconCopy/>
                    </button>
                    {own_btns}
                </div>
            }
        })
    }
}
