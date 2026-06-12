//! W4/T4 radial long-press action menu (touch). On coarse-pointer devices a
//! ~450ms press on a `.msg` row blossoms a glass arc of action buttons around
//! the touch point, replacing the hover-revealed `.msg-actions` row (hidden
//! under `(pointer: coarse)` in `_content.scss`). Which buttons appear comes
//! from the shared `message_actions` kind predicate in `mod.rs` — the same
//! one the hover row uses, so the two surfaces can never drift. Desktop is
//! untouched: fine-pointer devices keep the hover row and never arm the
//! long-press (`is_touch()` gate); right-click stays native.
//!
//! The pointer listeners are DELEGATED: `mod.rs` binds [`LongPress`]'s
//! handlers ONCE on the `<ul class="messages">`. This build has no tachys
//! event delegation, so per-row `on:` handlers would mean 5 real
//! `addEventListener` calls (with boxed closures) per row, all detached and
//! re-attached by the non-keyed list on every message change — plus a full
//! `MessageEnvelope` retained per row. Instead the pressed row is resolved
//! at fire time via `target.closest("li[id^='msg-']")`, and the envelope is
//! looked up by id from `s.msg.messages` only when a press actually opens
//! the menu; a scroll-flick pointerdown costs a `String`, never a clone.
//!
//! Always-on module: the render fn and the [`LongPress`] facade compile in
//! both graphs (the buttons call `act::` handlers, which carry ssr no-op
//! stubs; the `<ul>` bindings typecheck against empty ssr method bodies);
//! only the timer/DOM plumbing is hydrate-gated.

use leptos::prelude::*;

use super::super::{act, PendingDelete, Shell};
use crate::protocol::MessageEnvelope;
use crate::ui::icons::{IconCopy, IconEdit, IconReply, IconTrash};
use crate::ui::AuthCtx;

/// Open-menu state: the long-pressed message (the full envelope — reply
/// needs it for the banner preview, edit/copy need the body), the channel,
/// whether the viewer owns it (feeds the `message_actions` predicate), and
/// the press point already clamped inside the viewport so the arc never
/// overflows an edge.
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

/// Long-press bookkeeping for the whole message list, created once per
/// `ChannelPane` and bound on the `<ul>` (delegation — see the module docs).
/// Generation-counter pattern instead of a cancellable timer handle: `down`
/// bumps the generation and detaches a 450ms sleep that only opens the menu
/// if the generation is still current when it wakes; `pointermove` (beyond
/// [`MOVE_SLOP_PX`]), `pointerup`, and `pointercancel` bump it again,
/// turning the sleeping future into a no-op. This avoids holding a `!Send`
/// timer handle (gloo's `Timeout`) in arena storage and needs no
/// `wasm_bindgen::Closure`/`forget()` — the future owns its captures and
/// drops them when it completes. The async tail uses the `try_*` accessors
/// so a press that outlives the pane (channel/guild switch mid-hold disposes
/// the owner) degrades to a no-op, never a panic.
///
/// The struct itself is always-on (mod.rs binds its methods on the `<ul>`
/// unconditionally); the fields and real bodies exist only on hydrate.
#[derive(Clone, Copy)]
pub(super) struct LongPress {
    /// Bumped on every down/cancel; the armed future captures the value it
    /// was spawned with and only fires while it still matches.
    #[cfg(feature = "hydrate")]
    gen: StoredValue<u64>,
    /// Press origin while a press is armed (drift measurement); `None` once
    /// cancelled or fired.
    #[cfg(feature = "hydrate")]
    origin: StoredValue<Option<(f64, f64)>>,
}

impl LongPress {
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "hydrate")]
            gen: StoredValue::new(0),
            #[cfg(feature = "hydrate")]
            origin: StoredValue::new(None),
        }
    }
}

#[cfg(feature = "hydrate")]
impl LongPress {
    /// Delegated `pointerdown` for the message list: resolve the pressed
    /// `.msg` row from the event target and arm the long-press timer (touch
    /// only — desktop returns immediately, keeping mouse selection/drag
    /// native). The envelope is NOT touched here; it is looked up by id only
    /// if the press actually fires.
    pub fn down(
        self,
        ev: &leptos::ev::PointerEvent,
        s: Shell,
        auth: AuthCtx,
        radial: RwSignal<Option<RadialState>>,
    ) {
        if !super::is_touch() {
            return;
        }
        let Some(li) = target_msg_li(ev.target()) else {
            return;
        };
        // System rows never arm — they offer no actions (mirroring their
        // absent hover row; `message_actions` returns none for them). The
        // `system` class set at render is the cheap marker; they also keep
        // the native context menu (see `suppress_touch_context_menu`).
        if li.class_list().contains("system") {
            return;
        }
        let Some(mid) = li.id().strip_prefix("msg-").map(str::to_string) else {
            return;
        };
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
            // Only NOW materialise the envelope, by id from the live list.
            // A channel switch mid-press cleared `messages` (and bumped the
            // generation via `disarm`), so a stale id can never resurface
            // the old channel's message over the new pane.
            let Some(m) = s
                .msg
                .messages
                .try_with_untracked(|ms| ms.iter().find(|m| m.id == mid).cloned())
                .flatten()
            else {
                return;
            };
            let mine = auth
                .user
                .try_get_untracked()
                .flatten()
                .is_some_and(|u| u.account_id == m.author_id);
            // Kind gate shared with the hover row: nothing to offer → no
            // menu (covers any future actionless kind; system rows were
            // already filtered at pointerdown).
            if super::message_actions(&m.kind, mine).count() == 0 {
                return;
            }
            let cid = s
                .sel
                .sel_channel
                .try_get_untracked()
                .flatten()
                .map(|c| c.id);
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

    /// Delegated `pointermove`: drift beyond the slop radius means the user
    /// is scrolling/dragging, not pressing — cancel the pending press.
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

/// ssr stubs: never run (events only exist in the browser), but the `<ul>`
/// bindings in mod.rs are always-on and must typecheck on the server.
#[cfg(not(feature = "hydrate"))]
impl LongPress {
    pub fn down(
        self,
        _ev: &leptos::ev::PointerEvent,
        _s: Shell,
        _auth: AuthCtx,
        _radial: RwSignal<Option<RadialState>>,
    ) {
    }
    pub fn moved(self, _ev: &leptos::ev::PointerEvent) {}
    pub fn cancel(self) {}
}

/// Delegated `contextmenu` for the message list: some Android browsers fire
/// the native context menu on long-press; suppress it on touch so the radial
/// owns the gesture. Gated OFF system rows — they never open the radial, so
/// the native long-press menu (with Copy) is their only copy affordance
/// (CSS re-enables selection/callout on them, see `.msg.system` in
/// `_content.scss`). Desktop right-click stays native (`is_touch` gate).
#[cfg(feature = "hydrate")]
pub(super) fn suppress_touch_context_menu(ev: &leptos::ev::MouseEvent) {
    if !super::is_touch() {
        return;
    }
    let Some(li) = target_msg_li(ev.target()) else {
        return;
    };
    if li.class_list().contains("system") {
        return;
    }
    ev.prevent_default();
}

#[cfg(not(feature = "hydrate"))]
pub(super) fn suppress_touch_context_menu(_ev: &leptos::ev::MouseEvent) {}

/// Resolve the `.msg` row an event landed in: the nearest enclosing
/// `li[id^='msg-']`. The selector keys on the REAL message rows' dom ids —
/// the `skeleton-N` rows and the id-less draft/typing furniture never match.
#[cfg(feature = "hydrate")]
fn target_msg_li(target: Option<leptos::web_sys::EventTarget>) -> Option<leptos::web_sys::Element> {
    use leptos::wasm_bindgen::JsCast;
    target?
        .dyn_into::<leptos::web_sys::Element>()
        .ok()?
        .closest("li[id^='msg-']")
        .ok()
        .flatten()
}

#[cfg(feature = "hydrate")]
thread_local! {
    /// Channel-switch disarm hook. The radial signal and the [`LongPress`]
    /// tracker are `ChannelPane`-local, so the pane registers them here on
    /// setup and `act::channel::open_channel_at` (via
    /// `channel::disarm_radial`) reaches them without riding Shell state.
    /// WASM is single-threaded, so a thread-local IS the global.
    static DISARM: std::cell::Cell<Option<(LongPress, RwSignal<Option<RadialState>>)>> =
        const { std::cell::Cell::new(None) };
}

/// Register the mounted pane's radial handles for [`disarm`]. Cleared on
/// pane disposal — guarded on identity so a remount that registered first
/// is not clobbered by the outgoing pane's late cleanup.
#[cfg(feature = "hydrate")]
pub(super) fn register_disarm(lp: LongPress, radial: RwSignal<Option<RadialState>>) {
    DISARM.with(|d| d.set(Some((lp, radial))));
    on_cleanup(move || {
        DISARM.with(|d| {
            if d.get().is_some_and(|(_, r)| r == radial) {
                d.set(None);
            }
        });
    });
}

/// Disarm any pending long-press and close an open menu. Called on every
/// channel switch: a press armed in the OUTGOING channel must not fire over
/// the incoming pane (it would carry the old channel's envelope — a
/// cross-channel reply banner), and an open menu must not survive the
/// switch. No-op while no `ChannelPane` is mounted.
#[cfg(feature = "hydrate")]
pub(super) fn disarm() {
    if let Some((lp, radial)) = DISARM.with(std::cell::Cell::get) {
        lp.cancel();
        let _ = radial.try_set(None);
    }
}

/// Clamp the press point so the whole arc stays on-screen. `margin_x` is the
/// CSS arc reach — the buttons go `64px orbit radius + 22px half-chip = 86px`
/// past the anchor sideways and upward (`translateX(64px)` + the 44px chip in
/// `_content.scss`) — plus slack. The top clamp keeps the anchor below the
/// topbar, whose height is NOT a constant: on a notched-device PWA it grows
/// by `env(safe-area-inset-top)` (~59px) to ~109-115px total, so measure its
/// real bottom edge at open time and keep 120 as the no-topbar fallback —
/// FLOORED at the arc's own vertical reach (the n4 top chips peak
/// `64·sin(113°) + 22 ≈ 81px` above the anchor; a zero-inset topbar bottoms
/// out at ~58-68px, which alone would let them clip off the top edge). The
/// `.max()` guards keep `clamp` panic-free on degenerate (tiny) viewports.
#[cfg(feature = "hydrate")]
fn clamp_to_viewport(x: f64, y: f64) -> (f64, f64) {
    // Vertical arc reach + slack; pairs with margin_x's 86px+slack derivation.
    const ARC_REACH_TOP: f64 = 90.0;
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
    let top = leptos::web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.query_selector(".topbar").ok().flatten())
        .map(|bar| bar.get_bounding_client_rect().bottom() + 8.0)
        .unwrap_or(120.0)
        .max(ARC_REACH_TOP);
    (
        x.clamp(margin_x, (vw - margin_x).max(margin_x)),
        y.clamp(top, (vh - 16.0).max(top)),
    )
}

/// Render the radial menu while `radial` is `Some`: a full-screen scrim
/// backdrop plus a zero-size anchor `<div role="menu">` fixed at the
/// (clamped) press point whose buttons fan out on an upward arc via per-slot
/// `--ang` transforms (`_content.scss`); `fx-blossom` (`_motion.scss`)
/// scales the whole arc out of the anchor on open. Every button dispatches
/// the SAME `act::` handler as its hover-row counterpart in `meta.rs`, then
/// closes; both surfaces draw their buttons from the shared
/// `message_actions` predicate. Escape closes (the lightbox pattern: the
/// container is focused on open, so its own keydown hears it). Icons are
/// `aria-hidden` (see `icons.rs`), so each button carries an explicit
/// `aria-label` (W3 review convention).
///
/// `armed` is the manufactured-click guard, created ONCE at pane scope by
/// the caller (allocating it per render would leak an arena slot per open).
/// On touch, the compat mouse events (mousedown/up/click) fire at the
/// RELEASE point AFTER pointerup — by then this overlay is on top, so a
/// sub-500ms long-press delivers a stray click to whatever sits at the
/// release point. For the backdrop that meant insta-close; for the BUTTONS
/// it is worse: when `clamp_to_viewport` displaces the anchor away from the
/// finger (press near an edge or the topbar) a chip can sit under the
/// release point, and an unguarded click would activate it — worst case the
/// delete chip, instantly if the user opted out of delete confirmations. So
/// backdrop AND buttons share the one flag: only after a `pointerdown` the
/// OPEN menu itself observed (a genuinely NEW tap, bubbling from a chip or
/// landing on the backdrop) does a click act; the opening press never
/// pointer-downs on the overlay, so its manufactured click is ignored.
pub(super) fn radial_menu(
    s: Shell,
    radial: RwSignal<Option<RadialState>>,
    armed: StoredValue<bool>,
) -> impl IntoView {
    // Focus the menu on open so Escape lands on its own keydown without a
    // global listener (mirrors lightbox.rs); focus also hands the
    // role="menu" container to AT.
    let menu_ref = NodeRef::<leptos::html::Div>::new();
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        if radial.with(|o| o.is_some()) {
            if let Some(el) = menu_ref.get() {
                let _ = (*el).focus();
            }
        }
    });

    move || {
        radial.get().map(|st| {
            // Fresh open starts DISARMED (see the fn docs above).
            armed.set_value(false);
            let actions = super::message_actions(&st.m.kind, st.mine);
            // Slot count picks the arc spread; the CSS defines exactly the
            // two layouts that occur (2 = reply/copy, 4 = + edit/delete).
            let arc_class = if actions.count() == 4 {
                "radial-menu n4"
            } else {
                "radial-menu n2"
            };
            let pos = format!("left:{:.0}px;top:{:.0}px", st.x, st.y);

            let reply_btn = actions.reply.then(|| {
                let reply_m = st.m.clone();
                view! {
                    <button class="radial-btn" role="menuitem" aria-label="reply"
                        on:click=move |_| {
                            if !armed.get_value() {
                                return;
                            }
                            act::start_reply(s, reply_m.clone());
                            radial.set(None);
                        }>
                        <IconReply/>
                    </button>
                }
            });
            let copy_btn = actions.copy.then(|| {
                let copy_body = st.m.body.clone();
                view! {
                    <button class="radial-btn" role="menuitem" aria-label="copy message text"
                        on:click=move |_| {
                            if !armed.get_value() {
                                return;
                            }
                            act::copy_message_body(s, copy_body.clone());
                            radial.set(None);
                        }>
                        <IconCopy/>
                    </button>
                }
            });
            let edit_btn = actions.edit.then(|| {
                let edit_cid = st.cid.clone();
                let edit_mid = st.m.id.clone();
                let edit_body = st.m.body.clone();
                view! {
                    <button class="radial-btn" role="menuitem" aria-label="edit message"
                        on:click=move |_| {
                            if !armed.get_value() {
                                return;
                            }
                            if let Some(cid) = edit_cid.clone() {
                                act::start_edit(s, cid, edit_mid.clone(), edit_body.clone());
                            }
                            radial.set(None);
                        }>
                        <IconEdit/>
                    </button>
                }
            });
            let del_btn = actions.delete.then(|| {
                let del_cid = st.cid.clone();
                let del_mid = st.m.id.clone();
                view! {
                    <button class="radial-btn danger" role="menuitem" aria-label="delete message"
                        on:click=move |_| {
                            if !armed.get_value() {
                                return;
                            }
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
                // Dismiss layer: a genuinely-new tap closes the menu; the
                // opening press's manufactured click finds `armed` false and
                // is ignored, while a real dismissal tap is consumed by the
                // still-mounted backdrop, so nothing below ghost-clicks.
                <div class="radial-backdrop"
                    on:pointerdown=move |_| armed.set_value(true)
                    on:click=move |_| {
                        if armed.get_value() {
                            radial.set(None);
                        }
                    }></div>
                <div class=arc_class style=pos node_ref=menu_ref tabindex="-1"
                    role="menu" aria-label="message actions"
                    // Chip pointerdowns bubble here, arming their click.
                    on:pointerdown=move |_| armed.set_value(true)
                    on:keydown=move |_ev| {
                        #[cfg(feature = "hydrate")]
                        if _ev.key() == "Escape" {
                            _ev.prevent_default();
                            radial.set(None);
                        }
                        #[cfg(not(feature = "hydrate"))]
                        let _ = &_ev;
                    }>
                    {reply_btn}
                    {copy_btn}
                    {edit_btn}
                    {del_btn}
                </div>
            }
        })
    }
}
