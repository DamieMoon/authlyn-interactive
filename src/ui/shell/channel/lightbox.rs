//! The attachment lightbox: near-fullscreen media viewer with same-message
//! gallery nav, zoom buttons/keys, and (hydrate-only) a self-contained pointer
//! gesture engine — pinch, pan, double-tap, drag-down-to-dismiss.
//!
//! [`LightboxState`]/[`LbTransform`], the pure transform math, and the
//! gesture state machine ([`Gesture`] + [`pinch_pointer_gone`]) are un-gated
//! (plain data, unit-tested under the ssr graph); the interactive view +
//! gesture engine are hydrate-only, with a minimal ssr twin that the client
//! hydrates into.
//!
//! The overlay stays bespoke (NOT `ui::modal` — restyling through it would
//! change visuals, see modal.rs) but carries the same dialog a11y contract:
//! `role="dialog"`/`aria-modal`, focus moves in on open, Tab/Shift+Tab wrap
//! within the controls, Escape closes, and focus returns to the pre-open
//! element on close (review M-49).

use leptos::prelude::*;

use crate::protocol::Attachment;

/// Lightbox gallery state: the clicked message's IMAGE attachments plus the
/// index currently on screen. Arrow keys / pointer-swipe step `idx` within
/// `images`, clamped at the boundaries (no wrap). A one-image message yields a
/// single-entry list, so nav is a no-op. Cloned cheaply (a handful of small
/// `Attachment`s per message).
///
/// Videos are excluded from the gallery (they keep their own inline controls);
/// clicking a video opens a single-entry gallery holding just that video, so
/// the arrow/swipe handlers find nothing to navigate to.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct LightboxState {
    pub(super) images: Vec<Attachment>,
    pub(super) idx: usize,
}

// ---------------------------------------------------------------------------
// Transform math (pure, un-gated, unit-tested under the ssr graph).
//
// Coordinate model: the <img> is flex-centred in the fixed inset-0 overlay, so
// at identity its centre sits at the viewport centre `c = (vw/2, vh/2)`. With
// CSS `transform: translate3d(tx, ty, 0) scale(s)` (default centre origin),
// the image-local offset `p` from the centre renders at `c + t + s·p`. All
// gesture math below solves for `t`/`s` in that model — never width/height/
// top/left (compositor-only, per the evolution rank-38 discipline).
// ---------------------------------------------------------------------------

/// Smallest allowed scale: 1.0 = fit-to-screen (the laid-out size).
pub(super) const ZOOM_MIN: f64 = 1.0;
/// Largest allowed scale.
pub(super) const ZOOM_MAX: f64 = 4.0;
/// Scale a double-tap toggles up to (from fit), anchored at the tap point.
pub(super) const DOUBLE_TAP_ZOOM: f64 = 2.0;

/// Live transform of the lightbox image, rendered as CSS
/// `translate3d(tx px, ty px, 0) scale(scale)`. Identity = fit-to-screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct LbTransform {
    pub(super) scale: f64,
    pub(super) tx: f64,
    pub(super) ty: f64,
}

impl Default for LbTransform {
    fn default() -> Self {
        Self {
            scale: 1.0,
            tx: 0.0,
            ty: 0.0,
        }
    }
}

/// Clamp one translate axis so the image cannot be lost off-screen: while the
/// scaled extent fits inside the viewport the image stays centred (t = 0);
/// once it overflows, the translate may roam only far enough to pin either
/// image edge to the matching viewport edge.
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
fn clamp_axis(t: f64, scaled: f64, view: f64) -> f64 {
    if scaled <= view {
        0.0
    } else {
        let max = (scaled - view) / 2.0;
        t.clamp(-max, max)
    }
}

/// Clamp a transform's translate to the image extents at its current scale
/// (see [`clamp_axis`]); `img`/`view` are the laid-out image size and the
/// viewport size in px. The scale itself is assumed pre-clamped.
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(super) fn clamp_translate(tf: LbTransform, img: (f64, f64), view: (f64, f64)) -> LbTransform {
    LbTransform {
        scale: tf.scale,
        tx: clamp_axis(tf.tx, img.0 * tf.scale, view.0),
        ty: clamp_axis(tf.ty, img.1 * tf.scale, view.1),
    }
}

/// Re-scale to `new_scale` (clamped to [`ZOOM_MIN`]..[`ZOOM_MAX`]) keeping the
/// image point currently under `anchor` (viewport coords) stationary — the
/// math behind wheel zoom, double-tap zoom, and the +/− buttons (anchored at
/// the viewport centre). Returns an UNCLAMPED translate; compose with
/// [`clamp_translate`].
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(super) fn zoom_about(
    tf: LbTransform,
    anchor: (f64, f64),
    new_scale: f64,
    view: (f64, f64),
) -> LbTransform {
    let s = new_scale.clamp(ZOOM_MIN, ZOOM_MAX);
    let c = (view.0 / 2.0, view.1 / 2.0);
    // Image-local point under the anchor before; keep it under the anchor.
    let px = (anchor.0 - c.0 - tf.tx) / tf.scale;
    let py = (anchor.1 - c.1 - tf.ty) / tf.scale;
    LbTransform {
        scale: s,
        tx: anchor.0 - c.0 - s * px,
        ty: anchor.1 - c.1 - s * py,
    }
}

/// Two-finger pinch update from the gesture-start snapshot: the scale follows
/// the finger-distance ratio (clamped) and the image point that sat under the
/// START midpoint is carried to the CURRENT midpoint — zoom around the gesture
/// midpoint and two-finger pan in one expression. Returns an UNCLAMPED
/// translate; compose with [`clamp_translate`].
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(super) fn pinch_update(
    start: LbTransform,
    start_mid: (f64, f64),
    start_dist: f64,
    mid: (f64, f64),
    dist: f64,
    view: (f64, f64),
) -> LbTransform {
    let s = (start.scale * dist / start_dist.max(1.0)).clamp(ZOOM_MIN, ZOOM_MAX);
    let c = (view.0 / 2.0, view.1 / 2.0);
    // Image-local point under the START midpoint, carried to the CURRENT one.
    let px = (start_mid.0 - c.0 - start.tx) / start.scale;
    let py = (start_mid.1 - c.1 - start.ty) / start.scale;
    LbTransform {
        scale: s,
        tx: mid.0 - c.0 - s * px,
        ty: mid.1 - c.1 - s * py,
    }
}

/// Double-tap target: zoomed in (however slightly) toggles back to fit;
/// at fit it zooms to [`DOUBLE_TAP_ZOOM`] anchored at the tap point, clamped
/// so the result cannot leave the viewport.
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(super) fn double_tap_target(
    tf: LbTransform,
    tap: (f64, f64),
    img: (f64, f64),
    view: (f64, f64),
) -> LbTransform {
    if tf.scale > 1.01 {
        LbTransform::default()
    } else {
        clamp_translate(zoom_about(tf, tap, DOUBLE_TAP_ZOOM, view), img, view)
    }
}

/// Should a released vertical drag dismiss the lightbox? Yes past a quarter of
/// the viewport height, or on a downward flick (fast release velocity in
/// px/ms) that has travelled at least a little real distance.
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(super) fn should_dismiss(dy: f64, view_h: f64, vy: f64) -> bool {
    dy > view_h * 0.25 || (vy > 0.6 && dy > 48.0)
}

// ---------------------------------------------------------------------------
// Gesture engine. The state machine (mode enum + bookkeeping struct + the
// pinch-degrade transition) is plain data, un-gated so the ssr test graph can
// unit-test it; the DOM wiring below stays hydrate-only.
// ---------------------------------------------------------------------------

/// What the active pointers are currently doing. Single-finger intent is
/// decided once, when the press first leaves the slop radius: pan wins while
/// zoomed (judges' arbitration rule — a pan at scale > 1 must beat the gallery
/// swipe), otherwise the dominant axis picks gallery-swipe vs drag-dismiss.
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
#[derive(Clone, Copy, PartialEq, Default, Debug)]
enum GestureMode {
    #[default]
    Idle,
    /// Pressed, intent not yet decided (still inside the slop radius).
    Pending,
    /// One finger panning a zoomed image.
    Pan,
    /// Two fingers: scale about the midpoint + two-finger pan.
    Pinch,
    /// One finger at fit scale: horizontal gallery swipe (follows the finger).
    SwipeH,
    /// One finger at fit scale: vertical drag toward dismissal.
    DragV,
}

/// Pointer bookkeeping for one gesture. Lives in a `StoredValue` (plumbing,
/// not UI — it must never re-render anything per pointermove).
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
#[derive(Clone, Default)]
struct Gesture {
    /// Active tracked pointers (id, last x, last y); capped at two.
    pointers: Vec<(i32, f64, f64)>,
    mode: GestureMode,
    /// Transform when the current gesture phase started.
    start_tf: LbTransform,
    /// Press point of the deciding finger (origin for pan/swipe/drag deltas).
    origin: (f64, f64),
    /// Two-finger midpoint / distance at pinch start.
    start_mid: (f64, f64),
    start_dist: f64,
    /// Previous move sample (time ms, x, y) feeding the release velocity.
    vel_sample: (f64, f64, f64),
    /// Latest pointer velocity in px/ms (flick detection).
    velocity: (f64, f64),
    /// Last completed tap (time ms, x, y) for double-tap pairing.
    last_tap: Option<(f64, f64, f64)>,
}

/// A pinch just lost a pointer (finger lifted OR the browser cancelled it —
/// system edge gesture, palm rejection): transition the gesture for whatever
/// remains. One survivor while zoomed carries on as a pan re-anchored on that
/// finger; one survivor at fit scale ends the gesture and asks the caller to
/// snap the view home (returns `true`); no survivors go idle where they are;
/// two remaining pointers (an untracked extra finger vanished) keep pinching.
///
/// Shared by `on_pointerup` and `on_pointercancel` so the two release paths
/// can never drift apart again — pointercancel used to skip the degrade and
/// froze the gesture in two-pointer Pinch mode with `.gesturing` pinned
/// (review M-35). `cur` is the live transform at the moment of loss.
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
fn pinch_pointer_gone(g: &mut Gesture, cur: LbTransform) -> bool {
    match g.pointers[..] {
        [survivor] => {
            if cur.scale > 1.01 {
                g.mode = GestureMode::Pan;
                g.start_tf = cur;
                g.origin = (survivor.1, survivor.2);
                false
            } else {
                g.mode = GestureMode::Idle;
                true
            }
        }
        [] => {
            g.mode = GestureMode::Idle;
            false
        }
        _ => false, // both pinch fingers still down: carry on
    }
}

/// Render the lightbox overlay when open. Split out of `ChannelPane` so the
/// hydrate-only nav/zoom/gesture wiring stays one tidy block; the ssr build
/// gets a minimal no-interaction version (the page hydrates into this one).
///
/// `tf` is the image's transform (scale + pan), reset to identity on every
/// open and gallery step. The outer view reacts only to open/closed via an
/// `is_open` memo, so stepping the index or zoom-panning re-renders the inner
/// media only — the focusable container keeps focus, which is what scopes the
/// arrow keys to the lightbox.
///
/// Gestures (the viewport itself is `user-scalable=no`, so this overlay must
/// own them): two-finger pinch zooms about the gesture midpoint; one finger
/// pans while zoomed (clamped so the image can never be lost off-screen);
/// double-tap toggles fit ↔ 2x at the tap point; at fit scale a horizontal
/// drag follows the finger and steps the gallery on release/flick, and a
/// vertical drag shrinks the image + thins the backdrop 1:1 with the finger,
/// dismissing past a quarter of the viewport or on a downward flick. Desktop
/// gets the same physics via mouse-drag and (ctrl+)wheel zoom about the
/// cursor. Everything is transform/opacity-only: pointer math is rAF-batched
/// into one `translate3d(..) scale(..)` write per frame, and `will-change` is
/// scoped to the gesture via the `.gesturing` class (see `_lightbox.scss`).
#[cfg(feature = "hydrate")]
pub(super) fn lightbox_view(
    lightbox: RwSignal<Option<LightboxState>>,
    tf: RwSignal<LbTransform>,
) -> impl IntoView {
    use leptos::ev::{KeyboardEvent, PointerEvent, WheelEvent};
    use leptos::wasm_bindgen::JsCast;
    use leptos::web_sys;
    use send_wrapper::SendWrapper;

    // Scale step for the +/− buttons and keys.
    const ZOOM_STEP: f64 = 0.5;
    // Horizontal travel (px) past which a release steps the gallery.
    const SWIPE_PX: f64 = 50.0;
    // Movement (px) before a press stops being a tap and picks an intent.
    const SLOP_PX: f64 = 8.0;
    // Gallery flick: |vx| (px/ms) and minimum travel that step on release
    // before the full SWIPE_PX distance is reached.
    const FLICK_VX: f64 = 0.5;
    const FLICK_MIN_PX: f64 = 30.0;
    // Two taps pair into a double-tap within this window/radius.
    const DOUBLE_TAP_MS: f64 = 300.0;
    const DOUBLE_TAP_NEAR_PX: f64 = 40.0;

    /// Presses the engine must leave alone: the control buttons (close/nav/
    /// the whole zoom cluster, gaps included) and the `<video>` element
    /// (native controls own their pointers).
    fn is_passive_target(ev: &PointerEvent) -> bool {
        ev.target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
            .is_some_and(|el| {
                el.closest("button, video, .lightbox-zoom")
                    .ok()
                    .flatten()
                    .is_some()
            })
    }

    /// The lightbox's focusable controls in DOM order (close/nav/zoom buttons
    /// plus a video entry's `<video controls>`), for the dialog Tab trap. A
    /// local twin of `ui::modal`'s private helper — the lightbox is
    /// deliberately bespoke (review M-49) — extended with `video[controls]`,
    /// which a modal never hosts: the natively tab-focusable video renders
    /// LAST in the overlay, so without it forward-Tab from zoom "+" wrapped
    /// straight past the video (its controls keyboard-unreachable) and a
    /// pointer-focused video resolved no idx, letting the next Tab escape
    /// into the page behind. Focus inside the closed UA shadow controls
    /// retargets `activeElement` to the `<video>` host, so the host is one
    /// trap stop in either direction.
    fn focusables(root: &web_sys::Element) -> Vec<web_sys::HtmlElement> {
        const SEL: &str = "a[href], button:not([disabled]), input:not([disabled]), \
                           textarea:not([disabled]), select:not([disabled]), \
                           video[controls], [tabindex]:not([tabindex=\"-1\"])";
        let Ok(list) = root.query_selector_all(SEL) else {
            return Vec::new();
        };
        let mut out = Vec::with_capacity(list.length() as usize);
        for i in 0..list.length() {
            if let Some(el) = list
                .item(i)
                .and_then(|n| n.dyn_into::<web_sys::HtmlElement>().ok())
            {
                out.push(el);
            }
        }
        out
    }

    // Backdrop opacity (1 = the usual dim): thinned 1:1 by drag-dismiss.
    let backdrop = RwSignal::new(1.0_f64);
    // True while fingers own the image: disables the snap transition and
    // scopes `will-change: transform` to the gesture (cleared on settle — a
    // permanent compositor layer wastes floor-device memory).
    let gesturing = RwSignal::new(false);
    let gest = StoredValue::new(Gesture::default());
    // rAF batching: pointermove math lands in `raf_pending`; one scheduled
    // frame flushes the LATEST value into the signals, so however fast the
    // pointer stream runs there is a single style write per painted frame.
    let raf_pending = StoredValue::new(None::<(LbTransform, f64)>);
    let raf_armed = StoredValue::new(false);

    // Container ref: autofocus on open (the arrow keys ride the container's
    // on:keydown, which only receives events while it has focus — that is
    // what keeps the handler off the global/textarea key path) + pointer
    // capture + viewport size. Image ref: laid-out size for the clamp math.
    let lb_ref = NodeRef::<leptos::html::Div>::new();
    let img_ref = NodeRef::<leptos::html::Img>::new();

    // Laid-out image size + viewport size (px). None while no <img> is live
    // (video entry, or an image that has not laid out yet) — every transform
    // mutation needs these, so None simply disables zoom/pan/dismiss.
    let metrics = move || -> Option<((f64, f64), (f64, f64))> {
        let img = img_ref.get_untracked()?;
        let (iw, ih) = (
            f64::from(img.offset_width()),
            f64::from(img.offset_height()),
        );
        let lb = lb_ref.get_untracked()?;
        let (vw, vh) = (f64::from(lb.client_width()), f64::from(lb.client_height()));
        (iw > 0.0 && ih > 0.0 && vw > 0.0 && vh > 0.0).then_some(((iw, ih), (vw, vh)))
    };

    // The freshest transform: the not-yet-flushed rAF value when one exists.
    let live_tf = move || {
        raf_pending
            .get_value()
            .map(|(t, _)| t)
            .unwrap_or_else(|| tf.get_untracked())
    };

    // Queue a gesture frame: remember the latest value, arm one rAF flush.
    let schedule = move |t: LbTransform, b: f64| {
        raf_pending.set_value(Some((t, b)));
        if !raf_armed.get_value() {
            raf_armed.set_value(true);
            request_animation_frame(move || {
                raf_armed.set_value(false);
                if let Some((t, b)) = raf_pending.get_value() {
                    raf_pending.set_value(None);
                    tf.set(t);
                    backdrop.set(b);
                }
            });
        }
    };

    // Back to fit-to-screen (also drops any queued gesture frame).
    let reset_view = move || {
        raf_pending.set_value(None);
        tf.set(LbTransform::default());
        backdrop.set(1.0);
    };

    // Step the gallery index, stopping at the boundaries (no wrap), and reset
    // the view transform for the freshly-shown image.
    let step = move |delta: i32| {
        lightbox.update(|opt| {
            if let Some(state) = opt {
                let last = state.images.len().saturating_sub(1);
                let next = (state.idx as i32 + delta).clamp(0, last as i32) as usize;
                state.idx = next;
            }
        });
        reset_view();
    };

    // The +/− buttons and keys: re-scale about the viewport centre (whatever
    // is mid-screen stays mid-screen), clamped to the pan bounds.
    let nudge_zoom = move |delta: f64| {
        let Some((img, view)) = metrics() else {
            return;
        };
        let cur = live_tf();
        let target = zoom_about(cur, (view.0 / 2.0, view.1 / 2.0), cur.scale + delta, view);
        raf_pending.set_value(None);
        tf.set(clamp_translate(target, img, view));
    };

    let on_keydown = move |ev: KeyboardEvent| match ev.key().as_str() {
        "ArrowLeft" => {
            ev.prevent_default();
            step(-1);
        }
        "ArrowRight" => {
            ev.prevent_default();
            step(1);
        }
        "Escape" => {
            ev.prevent_default();
            lightbox.set(None);
        }
        "+" | "=" => {
            ev.prevent_default();
            nudge_zoom(ZOOM_STEP);
        }
        "-" | "_" => {
            ev.prevent_default();
            nudge_zoom(-ZOOM_STEP);
        }
        "0" => {
            ev.prevent_default();
            reset_view();
        }
        "Tab" => {
            // Dialog focus trap, mirroring `ui::modal`: Tab/Shift+Tab wrap
            // within the lightbox's own controls so keyboard focus can't
            // escape into the page behind the full-screen overlay — after
            // which Escape would stop closing, since this keydown only fires
            // while focus is inside (review M-49).
            let Some(lb) = lb_ref.get_untracked() else {
                return;
            };
            let els = focusables(lb.as_ref());
            if els.is_empty() {
                return;
            }
            let active = web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.active_element())
                .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok());
            let idx = active
                .as_ref()
                .and_then(|a| els.iter().position(|el| el == a));
            let last = els.len() - 1;
            // Wrap when leaving either end; Shift+Tab from the container
            // itself (idx None, the post-open state) lands on the last
            // control instead of escaping backwards.
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
        _ => {}
    };

    // Desktop twin of the pinch: (ctrl+)wheel re-scales about the cursor.
    // preventDefault keeps the browser's own ctrl+wheel page zoom off; the
    // small per-tick delta rides the snap transition, which smooths notched
    // mouse wheels into a glide.
    let on_wheel = move |ev: WheelEvent| {
        ev.prevent_default();
        let Some((img, view)) = metrics() else {
            return;
        };
        let cur = live_tf();
        // Exponential feel: equal wheel steps multiply the scale equally.
        let factor = (-ev.delta_y() * 0.002).exp();
        let target = zoom_about(
            cur,
            (f64::from(ev.client_x()), f64::from(ev.client_y())),
            cur.scale * factor,
            view,
        );
        raf_pending.set_value(None);
        tf.set(clamp_translate(target, img, view));
    };

    let on_pointerdown = move |ev: PointerEvent| {
        if is_passive_target(&ev) {
            return;
        }
        // preventDefault keeps the browser's back-swipe / native image drag /
        // selection from firing (paired with `touch-action: none` in SCSS).
        ev.prevent_default();
        // Capture so moves outside the (fullscreen) overlay — possible in
        // desktop windows — keep feeding the gesture.
        if let Some(lb) = lb_ref.get_untracked() {
            let _ = lb.set_pointer_capture(ev.pointer_id());
        }
        let (x, y) = (f64::from(ev.client_x()), f64::from(ev.client_y()));
        let cur = live_tf();
        let has_img = metrics().is_some();
        gest.update_value(|g| {
            if g.pointers.iter().any(|p| p.0 == ev.pointer_id()) || g.pointers.len() >= 2 {
                return; // duplicate or third finger: ignored
            }
            g.pointers.push((ev.pointer_id(), x, y));
            if g.pointers.len() == 1 {
                g.mode = GestureMode::Pending;
                g.origin = (x, y);
                g.start_tf = cur;
                g.vel_sample = (ev.time_stamp(), x, y);
                g.velocity = (0.0, 0.0);
            } else if has_img {
                // Second finger: pinch (images only — a video entry keeps
                // its native behaviour and has no transform to drive).
                g.mode = GestureMode::Pinch;
                g.start_tf = cur;
                let (a, b) = (g.pointers[0], g.pointers[1]);
                g.start_mid = ((a.1 + b.1) / 2.0, (a.2 + b.2) / 2.0);
                g.start_dist = (a.1 - b.1).hypot(a.2 - b.2);
            } else {
                g.mode = GestureMode::Idle;
            }
        });
        gesturing.set(true);
    };

    let on_pointermove = move |ev: PointerEvent| {
        let (x, y) = (f64::from(ev.client_x()), f64::from(ev.client_y()));
        let m = metrics();
        let mut frame = None;
        gest.update_value(|g| {
            let Some(p) = g.pointers.iter_mut().find(|p| p.0 == ev.pointer_id()) else {
                return; // hover / untracked pointer
            };
            p.1 = x;
            p.2 = y;
            let ts = ev.time_stamp();
            let (t0, x0, y0) = g.vel_sample;
            if ts > t0 {
                g.velocity = ((x - x0) / (ts - t0), (y - y0) / (ts - t0));
            }
            g.vel_sample = (ts, x, y);
            // First escape from the slop radius decides the one-finger intent.
            if g.mode == GestureMode::Pending {
                let (dx, dy) = (x - g.origin.0, y - g.origin.1);
                if dx.hypot(dy) > SLOP_PX {
                    g.mode = if g.start_tf.scale > 1.01 {
                        GestureMode::Pan
                    } else if dx.abs() > dy.abs() {
                        GestureMode::SwipeH
                    } else if m.is_some() {
                        GestureMode::DragV
                    } else {
                        GestureMode::Idle // video: no vertical gesture
                    };
                }
            }
            match g.mode {
                GestureMode::Pan => {
                    if let Some((img, view)) = m {
                        let t = LbTransform {
                            scale: g.start_tf.scale,
                            tx: g.start_tf.tx + x - g.origin.0,
                            ty: g.start_tf.ty + y - g.origin.1,
                        };
                        frame = Some((clamp_translate(t, img, view), 1.0));
                    }
                }
                GestureMode::Pinch => {
                    if let (Some((img, view)), [a, b]) = (m, &g.pointers[..]) {
                        let mid = ((a.1 + b.1) / 2.0, (a.2 + b.2) / 2.0);
                        let dist = (a.1 - b.1).hypot(a.2 - b.2);
                        let t =
                            pinch_update(g.start_tf, g.start_mid, g.start_dist, mid, dist, view);
                        frame = Some((clamp_translate(t, img, view), 1.0));
                    }
                }
                GestureMode::SwipeH => {
                    // Follow the finger between gallery images (fit scale,
                    // free X — deliberately unclamped; release decides).
                    frame = Some((
                        LbTransform {
                            scale: 1.0,
                            tx: x - g.origin.0,
                            ty: 0.0,
                        },
                        1.0,
                    ));
                }
                GestureMode::DragV => {
                    if let Some((_, view)) = m {
                        let (dx, dy) = (x - g.origin.0, y - g.origin.1);
                        // Downward travel shrinks the image and thins the
                        // backdrop 1:1 with the finger (the universal photo-
                        // viewer dismissal); upward follows and snaps back.
                        let down = (dy / view.1).clamp(0.0, 1.0);
                        frame = Some((
                            LbTransform {
                                scale: 1.0 - 0.3 * down,
                                tx: dx,
                                ty: dy,
                            },
                            1.0 - 0.85 * (dy / (0.6 * view.1)).clamp(0.0, 1.0),
                        ));
                    }
                }
                _ => {}
            }
        });
        if let Some((t, b)) = frame {
            schedule(t, b);
        }
    };

    // What a finished gesture wants done — computed inside the state update,
    // applied to the signals afterwards (so the settle ordering below holds).
    #[derive(Clone, Copy)]
    enum After {
        Nothing,
        Step(i32),
        Close,
        Snap(LbTransform, f64),
    }

    let on_pointerup = move |ev: PointerEvent| {
        let (x, y) = (f64::from(ev.client_x()), f64::from(ev.client_y()));
        let ts = ev.time_stamp();
        let m = metrics();
        let mut after = After::Nothing;
        gest.update_value(|g| {
            let Some(pos) = g.pointers.iter().position(|p| p.0 == ev.pointer_id()) else {
                return;
            };
            g.pointers.remove(pos);
            match g.mode {
                GestureMode::Pinch => {
                    // One finger lifted: carry on as a pan from here while
                    // still zoomed, otherwise settle back to dead centre
                    // (shared with on_pointercancel — see pinch_pointer_gone).
                    if pinch_pointer_gone(g, live_tf()) {
                        after = After::Snap(LbTransform::default(), 1.0);
                    }
                }
                GestureMode::Pending => {
                    // A tap (never left the slop radius): double-tap toggles
                    // fit ↔ 2x at the tap point; a single tap on the BACKDROP
                    // (outside the rendered image) closes. A single tap ON the
                    // image is deliberately inert — it may be half of a
                    // double-tap, and closing under it would make double-tap
                    // unusable.
                    g.mode = GestureMode::Idle;
                    let prev = g.last_tap.take();
                    let paired = prev.is_some_and(|(t0, x0, y0)| {
                        ts - t0 < DOUBLE_TAP_MS && (x - x0).hypot(y - y0) < DOUBLE_TAP_NEAR_PX
                    });
                    if paired {
                        if let Some((img, view)) = m {
                            after =
                                After::Snap(double_tap_target(live_tf(), (x, y), img, view), 1.0);
                        }
                    } else {
                        g.last_tap = Some((ts, x, y));
                        let inside = m.is_some_and(|(img, view)| {
                            let t = live_tf();
                            (x - (view.0 / 2.0 + t.tx)).abs() <= img.0 * t.scale / 2.0
                                && (y - (view.1 / 2.0 + t.ty)).abs() <= img.1 * t.scale / 2.0
                        });
                        if !inside {
                            after = After::Close;
                        }
                    }
                }
                GestureMode::SwipeH => {
                    g.mode = GestureMode::Idle;
                    let dx = x - g.origin.0;
                    let flick = g.velocity.0.abs() > FLICK_VX && dx.abs() > FLICK_MIN_PX;
                    if dx <= -SWIPE_PX || (flick && dx < 0.0) {
                        after = After::Step(1);
                    } else if dx >= SWIPE_PX || (flick && dx > 0.0) {
                        after = After::Step(-1);
                    } else {
                        after = After::Snap(LbTransform::default(), 1.0);
                    }
                }
                GestureMode::DragV => {
                    g.mode = GestureMode::Idle;
                    let dy = y - g.origin.1;
                    // DragV only engages with live metrics; the INFINITY
                    // fallback still allows the flick arm of the decision.
                    let vh = m.map_or(f64::INFINITY, |(_, view)| view.1);
                    if should_dismiss(dy, vh, g.velocity.1) {
                        after = After::Close;
                    } else {
                        after = After::Snap(LbTransform::default(), 1.0);
                    }
                }
                _ => {
                    if g.pointers.is_empty() {
                        g.mode = GestureMode::Idle;
                    }
                }
            }
        });
        // Settle: drop the gesture class FIRST (re-enabling the snap
        // transition and releasing will-change) so the final write animates
        // once no finger remains (a pinch survivor keeps the class).
        if gest.with_value(|g| g.pointers.is_empty()) {
            gesturing.set(false);
        }
        match after {
            After::Nothing => {
                // e.g. a finished pan: flush whatever the last frame computed
                // so the queued rAF can't land after a later reset.
                if gest.with_value(|g| g.pointers.is_empty()) {
                    if let Some((t, b)) = raf_pending.get_value() {
                        raf_pending.set_value(None);
                        tf.set(t);
                        backdrop.set(b);
                    }
                }
            }
            After::Step(d) => {
                step(d);
            }
            After::Close => {
                // Disarm any queued gesture frame so it can't land post-close.
                raf_pending.set_value(None);
                lightbox.set(None);
            }
            After::Snap(t, b) => {
                raf_pending.set_value(None);
                tf.set(t);
                backdrop.set(b);
            }
        }
    };

    // The browser took the pointer back (system gesture, palm rejection…):
    // forget it, and snap a half-done swipe/drag home. A pinch losing ONE
    // finger degrades exactly like on_pointerup's release — pan on the
    // survivor while zoomed, snap home at fit; before this mirrored path the
    // gesture froze in two-pointer Pinch mode with `.gesturing` (will-change)
    // pinned until a full lift (review M-35).
    let on_pointercancel = move |ev: PointerEvent| {
        let mut snap_home = false;
        gest.update_value(|g| {
            g.pointers.retain(|p| p.0 != ev.pointer_id());
            if g.mode == GestureMode::Pinch {
                snap_home = pinch_pointer_gone(g, live_tf());
            } else if g.pointers.is_empty() && g.mode != GestureMode::Idle {
                snap_home = matches!(g.mode, GestureMode::SwipeH | GestureMode::DragV);
                g.mode = GestureMode::Idle;
            }
        });
        // Settle ordering as in on_pointerup: drop `.gesturing` BEFORE the
        // snap write so the final write animates when this was the last
        // finger. A pinch survivor at fit scale keeps the class until it
        // lifts — harmless, as the transform is already ~identity there, so
        // the un-animated snap-home write has nothing visible to move.
        if gest.with_value(|g| g.pointers.is_empty()) {
            gesturing.set(false);
        }
        if snap_home {
            reset_view();
        }
    };

    // The element that held focus when the lightbox stole it — restored on
    // close (WCAG 2.4.3, review M-49). `SendWrapper` carries the non-`Send`
    // wasm handle across the `StoredValue` boundary, as in `ui::modal`.
    let trigger: StoredValue<Option<SendWrapper<web_sys::HtmlElement>>> = StoredValue::new(None);

    Effect::new(move |_| {
        if lightbox.with(|o| o.is_some()) {
            // Fresh view state on every open (this also runs on gallery steps
            // — the same reset `step` already performs, so it is idempotent).
            gest.set_value(Gesture::default());
            gesturing.set(false);
            reset_view();
            if let Some(el) = lb_ref.get() {
                // Snapshot the focus we are about to steal so close can hand
                // it back. Gallery steps re-run this with focus already
                // inside the lightbox; the `contains` filter drops those so
                // the original trigger is never clobbered.
                let lb_el: &web_sys::Element = (*el).as_ref();
                let prev = web_sys::window()
                    .and_then(|w| w.document())
                    .and_then(|d| d.active_element())
                    .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok())
                    .filter(|el| !lb_el.contains(Some(el.as_ref())));
                if let Some(prev) = prev {
                    trigger.set_value(Some(SendWrapper::new(prev)));
                }
                let _ = (*el).focus();
            }
        } else {
            // Closed (Esc, ✕, backdrop tap, drag-dismiss): hand focus back to
            // the element it was lifted from. Take-then-focus so a later
            // re-open starts from a clean slate.
            let prev = trigger.try_get_value().flatten();
            trigger.set_value(None);
            if let Some(prev) = prev {
                let _ = prev.focus();
            }
        }
    });

    // If the whole pane unmounts while the lightbox is open (channel switch),
    // the close branch above never runs — restore from cleanup instead, like
    // `ui::modal`. A trigger that unmounted with the pane no-ops silently;
    // a normal close already took the value, so this can't double-fire.
    on_cleanup(move || {
        if let Some(prev) = trigger.try_get_value().flatten() {
            let _ = prev.focus();
        }
    });

    // Re-render the overlay only on open/close, not on idx/transform changes.
    let is_open = Memo::new(move |_| lightbox.with(|o| o.is_some()));

    move || {
        {
        is_open.get().then(|| {
            // Current gallery entry (reactive over idx). Falls back to a no-op
            // when the list is somehow empty (never expected).
            let current = move || {
                lightbox.with(|o| {
                    o.as_ref()
                        .and_then(|s| s.images.get(s.idx).cloned())
                })
            };
            // Whether to show the nav arrows / whether each edge is reachable.
            let multi = move || lightbox.with(|o| o.as_ref().is_some_and(|s| s.images.len() > 1));
            let at_start = move || lightbox.with(|o| o.as_ref().is_none_or(|s| s.idx == 0));
            let at_end = move || {
                lightbox.with(|o| o.as_ref().is_none_or(|s| s.idx + 1 >= s.images.len()))
            };

            let media = move || {
                current().map(|att| {
                    let id = att.id.clone();
                    if att.mime.starts_with("video/") {
                        // Video keeps its own controls; no zoom transform.
                        view! {
                            <video class="lightbox-img" controls autoplay
                                src=format!("/media/{id}")></video>
                        }.into_any()
                    } else {
                        // The one element the whole engine drives: a single
                        // translate3d+scale (compositor-only; no layout/filter
                        // churn). draggable=false keeps the desktop native
                        // image-drag ghost out of the pan gesture.
                        view! {
                            <img class="lightbox-img" node_ref=img_ref
                                src=format!("/media/{id}") alt="attachment"
                                draggable="false"
                                // Braced: a bare `>` would close the tag in rstml.
                                class:zoomed={move || tf.get().scale > 1.01}
                                style=move || {
                                    let t = tf.get();
                                    format!(
                                        "transform: translate3d({:.2}px, {:.2}px, 0) scale({:.4})",
                                        t.tx, t.ty, t.scale,
                                    )
                                }/>
                        }.into_any()
                    }
                })
            };

            view! {
                <div class="lightbox" node_ref=lb_ref tabindex="-1"
                    role="dialog" aria-modal="true" aria-label="attachment viewer"
                    class:gesturing=move || gesturing.get()
                    on:keydown=on_keydown
                    on:pointerdown=on_pointerdown
                    on:pointermove=on_pointermove
                    on:pointerup=on_pointerup
                    on:pointercancel=on_pointercancel
                    on:wheel=on_wheel>
                    <div class="lightbox-backdrop"
                        style=move || format!("opacity:{:.3}", backdrop.get())></div>
                    <button class="lightbox-close" title="close"
                        on:click=move |ev| { ev.stop_propagation(); lightbox.set(None); }>"✕"</button>
                    {move || multi().then(|| view! {
                        <button class="lightbox-nav lightbox-prev" title="previous"
                            prop:disabled=at_start
                            on:click=move |ev: leptos::ev::MouseEvent| { ev.stop_propagation(); step(-1); }>"‹"</button>
                        <button class="lightbox-nav lightbox-next" title="next"
                            prop:disabled=at_end
                            on:click=move |ev: leptos::ev::MouseEvent| { ev.stop_propagation(); step(1); }>"›"</button>
                    })}
                    <div class="lightbox-zoom" on:click=move |ev: leptos::ev::MouseEvent| ev.stop_propagation()>
                        <button title="zoom out"
                            on:click=move |ev: leptos::ev::MouseEvent| { ev.stop_propagation(); nudge_zoom(-ZOOM_STEP); }>"−"</button>
                        <button title="reset zoom"
                            on:click=move |ev: leptos::ev::MouseEvent| { ev.stop_propagation(); reset_view(); }>"⤢"</button>
                        <button title="zoom in"
                            on:click=move |ev: leptos::ev::MouseEvent| { ev.stop_propagation(); nudge_zoom(ZOOM_STEP); }>"+"</button>
                    </div>
                    {media}
                </div>
            }
        })
    }
    .into_any()
    }
}

/// SSR build: minimal non-interactive lightbox (no nav/zoom/gesture wiring).
/// The client hydrates into the hydrate variant above. `tf` is accepted for a
/// matching signature but unused.
#[cfg(not(feature = "hydrate"))]
pub(super) fn lightbox_view(
    lightbox: RwSignal<Option<LightboxState>>,
    _tf: RwSignal<LbTransform>,
) -> impl IntoView {
    move || {
        lightbox.get().map(|state| {
            let att = state.images.get(state.idx).cloned();
            att.map(|att| {
                let id = att.id.clone();
                let is_video = att.mime.starts_with("video/");
                let media = if is_video {
                    view! {
                        <video class="lightbox-img" controls
                            src=format!("/media/{id}")></video>
                    }
                    .into_any()
                } else {
                    view! {
                        <img class="lightbox-img" src=format!("/media/{id}") alt="attachment"/>
                    }
                    .into_any()
                };
                view! {
                    <div class="lightbox" tabindex="-1"
                        role="dialog" aria-modal="true" aria-label="attachment viewer">
                        <div class="lightbox-backdrop"></div>
                        <button class="lightbox-close" title="close">"✕"</button>
                        {media}
                    </div>
                }
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Render the transform the way the view does and compare image-point
    /// positions: image-local offset `p` from the centre lands at
    /// `viewport_centre + t + scale·p`.
    fn rendered(tf: LbTransform, p: (f64, f64), view: (f64, f64)) -> (f64, f64) {
        (
            view.0 / 2.0 + tf.tx + tf.scale * p.0,
            view.1 / 2.0 + tf.ty + tf.scale * p.1,
        )
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn clamped_translate_recentres_axes_where_the_scaled_image_fits_the_viewport() {
        // 800x600 image at fit inside a 1000x800 viewport: any stray translate
        // must collapse back to dead centre on both axes.
        let tf = LbTransform {
            scale: 1.0,
            tx: 50.0,
            ty: -30.0,
        };
        let got = clamp_translate(tf, (800.0, 600.0), (1000.0, 800.0));
        assert!(close(got.tx, 0.0) && close(got.ty, 0.0), "got {got:?}");
    }

    #[test]
    fn clamped_translate_never_lets_a_zoomed_image_edge_leave_the_viewport_edge() {
        // 800x600 at 2x = 1600x1200 in a 1000x800 viewport: the translate may
        // roam at most ±(scaled−view)/2 = ±300 / ±200 — past that the image
        // edge would detach from the viewport edge and the image could be
        // pushed off-screen and lost.
        let tf = LbTransform {
            scale: 2.0,
            tx: 500.0,
            ty: -1000.0,
        };
        let got = clamp_translate(tf, (800.0, 600.0), (1000.0, 800.0));
        assert!(close(got.tx, 300.0), "tx pinned to +300, got {got:?}");
        assert!(close(got.ty, -200.0), "ty pinned to -200, got {got:?}");
        // And an in-range translate passes through untouched.
        let ok = LbTransform {
            scale: 2.0,
            tx: -120.0,
            ty: 75.0,
        };
        let kept = clamp_translate(ok, (800.0, 600.0), (1000.0, 800.0));
        assert_eq!(kept, ok);
    }

    #[test]
    fn pinch_keeps_the_image_point_under_the_gesture_midpoint_while_scaling_and_panning() {
        let view = (1000.0, 800.0);
        let start = LbTransform::default();
        let start_mid = (600.0, 300.0);
        // Image point that sat under the start midpoint.
        let p = (
            (start_mid.0 - view.0 / 2.0 - start.tx) / start.scale,
            (start_mid.1 - view.1 / 2.0 - start.ty) / start.scale,
        );
        // Fingers spread 100→200 px while the midpoint drifts to (650, 350).
        let mid = (650.0, 350.0);
        let got = pinch_update(start, start_mid, 100.0, mid, 200.0, view);
        assert!(close(got.scale, 2.0), "scale follows distance, got {got:?}");
        let r = rendered(got, p, view);
        assert!(
            close(r.0, mid.0) && close(r.1, mid.1),
            "anchored image point must ride the midpoint: got {r:?}, want {mid:?}"
        );
    }

    #[test]
    fn pinch_scale_is_clamped_to_the_fit_to_max_range() {
        let view = (1000.0, 800.0);
        let start = LbTransform::default();
        let wide = pinch_update(start, (500.0, 400.0), 50.0, (500.0, 400.0), 5000.0, view);
        assert!(close(wide.scale, ZOOM_MAX), "got {wide:?}");
        let tight = pinch_update(start, (500.0, 400.0), 200.0, (500.0, 400.0), 2.0, view);
        assert!(close(tight.scale, ZOOM_MIN), "got {tight:?}");
    }

    #[test]
    fn zoom_about_keeps_the_anchored_viewport_point_stationary() {
        let view = (1000.0, 800.0);
        let cur = LbTransform {
            scale: 1.5,
            tx: 40.0,
            ty: -20.0,
        };
        let anchor = (700.0, 200.0);
        let p = (
            (anchor.0 - view.0 / 2.0 - cur.tx) / cur.scale,
            (anchor.1 - view.1 / 2.0 - cur.ty) / cur.scale,
        );
        let got = zoom_about(cur, anchor, 3.0, view);
        assert!(close(got.scale, 3.0), "got {got:?}");
        let r = rendered(got, p, view);
        assert!(
            close(r.0, anchor.0) && close(r.1, anchor.1),
            "anchored point moved: got {r:?}, want {anchor:?}"
        );
    }

    #[test]
    fn double_tap_zooms_in_at_the_tap_point_and_a_second_double_tap_returns_to_fit() {
        let view = (1000.0, 800.0);
        let img = (800.0, 600.0);
        // From fit: zoom to DOUBLE_TAP_ZOOM keeping the tapped point put
        // (central-ish tap so the clamp is a no-op and the anchor is exact).
        let tap = (550.0, 360.0);
        let p = (tap.0 - view.0 / 2.0, tap.1 - view.1 / 2.0);
        let zoomed = double_tap_target(LbTransform::default(), tap, img, view);
        assert!(close(zoomed.scale, DOUBLE_TAP_ZOOM), "got {zoomed:?}");
        let r = rendered(zoomed, p, view);
        assert!(
            close(r.0, tap.0) && close(r.1, tap.1),
            "tapped point moved: got {r:?}, want {tap:?}"
        );
        // From zoomed (anywhere): toggle back to identity/fit.
        let back = double_tap_target(zoomed, (100.0, 100.0), img, view);
        assert_eq!(back, LbTransform::default());
    }

    #[test]
    fn double_tap_near_an_edge_is_clamped_so_the_image_is_not_lost_off_screen() {
        let view = (1000.0, 800.0);
        let img = (800.0, 600.0);
        // Tap in the far corner: the naive anchor math would over-translate;
        // the result must still respect the clamp bounds (±300/±200 at 2x).
        let got = double_tap_target(LbTransform::default(), (1.0, 1.0), img, view);
        assert!(got.tx.abs() <= 300.0 + 1e-9, "got {got:?}");
        assert!(got.ty.abs() <= 200.0 + 1e-9, "got {got:?}");
    }

    #[test]
    fn losing_one_pinch_finger_while_zoomed_degrades_to_a_pan_anchored_on_the_survivor() {
        // Mid-pinch at 2x, one finger is gone (lifted OR cancelled — the two
        // paths must behave identically, review M-35) and one survives.
        let mut g = Gesture {
            pointers: vec![(7, 320.0, 410.0)],
            mode: GestureMode::Pinch,
            ..Default::default()
        };
        let cur = LbTransform {
            scale: 2.0,
            tx: 12.0,
            ty: -8.0,
        };
        let snap = pinch_pointer_gone(&mut g, cur);
        assert!(!snap, "still zoomed: keep gesturing, no snap home");
        assert_eq!(
            g.mode,
            GestureMode::Pan,
            "the survivor must keep panning, never freeze in two-pointer Pinch"
        );
        assert_eq!(g.start_tf, cur, "pan re-anchors on the live transform");
        assert_eq!(g.origin, (320.0, 410.0), "pan origin is the survivor");
    }

    #[test]
    fn losing_one_pinch_finger_at_fit_scale_ends_the_gesture_and_requests_a_snap_home() {
        let mut g = Gesture {
            pointers: vec![(7, 320.0, 410.0)],
            mode: GestureMode::Pinch,
            ..Default::default()
        };
        let snap = pinch_pointer_gone(&mut g, LbTransform::default());
        assert!(
            snap,
            "at fit there is nothing to pan: settle back to centre"
        );
        assert_eq!(g.mode, GestureMode::Idle);
    }

    #[test]
    fn an_untracked_pointer_vanishing_must_not_break_a_live_two_finger_pinch() {
        // A third (never-tracked) finger cancels: both pinch fingers remain.
        let mut g = Gesture {
            pointers: vec![(1, 100.0, 100.0), (2, 200.0, 200.0)],
            mode: GestureMode::Pinch,
            ..Default::default()
        };
        let snap = pinch_pointer_gone(
            &mut g,
            LbTransform {
                scale: 2.0,
                tx: 0.0,
                ty: 0.0,
            },
        );
        assert!(!snap);
        assert_eq!(g.mode, GestureMode::Pinch, "pinch continues untouched");
    }

    #[test]
    fn losing_both_pinch_fingers_goes_idle_without_a_snap() {
        let mut g = Gesture {
            pointers: vec![],
            mode: GestureMode::Pinch,
            ..Default::default()
        };
        let snap = pinch_pointer_gone(
            &mut g,
            LbTransform {
                scale: 3.0,
                tx: 0.0,
                ty: 0.0,
            },
        );
        assert!(!snap, "no survivor to settle for; the transform stays put");
        assert_eq!(g.mode, GestureMode::Idle);
    }

    #[test]
    fn dismiss_requires_a_quarter_viewport_drag_or_a_genuine_downward_flick() {
        let vh = 800.0;
        // Slow but far: past 25% of the viewport height → dismiss.
        assert!(should_dismiss(vh * 0.3, vh, 0.1));
        // Slow and short → snap back.
        assert!(!should_dismiss(vh * 0.2, vh, 0.1));
        // Short but flicked hard downward → dismiss.
        assert!(should_dismiss(60.0, vh, 1.2));
        // A flick with almost no travel must NOT dismiss (twitch guard).
        assert!(!should_dismiss(20.0, vh, 5.0));
        // An upward drag never dismisses, whatever the speed.
        assert!(!should_dismiss(-300.0, vh, 3.0));
    }
}
