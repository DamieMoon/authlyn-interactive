//! Web Notifications + Web Push, driven almost entirely by reflection
//! (`Reflect::get` + `Function::call`) so no extra web-sys features are needed
//! and every step is fallible-swallowed. The full surface is documented at
//! each function, but the cardinal rules are:
//!
//! - `notifications_available()` is a feature-detect via `Reflect`; iOS Safari
//!   outside an installed PWA has no `Notification` global at all, and *touching*
//!   `Notification::permission()` there traps the WASM. ALL paths gate on this.
//! - `request_notify_permission()` only fires its async `subscribe` from a user
//!   gesture (send / mute click) so iOS' gesture-bound subscribe is satisfied.
//! - `show_notification()` prefers the service-worker `registration.showNotification()`
//!   path for installed-PWA standalone display mode, falling back to the
//!   `new Notification()` constructor for a plain tab.
//! - `notify_messages()` is the poll-loop hook; it never notifies for the
//!   user's own messages (`s.sync.me` filter) or when the tab is foregrounded or
//!   when the channel is muted.

#[cfg(feature = "hydrate")]
use super::super::Shell;
#[cfg(feature = "hydrate")]
use crate::client::api;
#[cfg(feature = "hydrate")]
use crate::protocol::{ChannelSummary, MessageEnvelope};
#[cfg(feature = "hydrate")]
use leptos::prelude::*;
#[cfg(feature = "hydrate")]
use leptos::task::spawn_local;

/// True only when `window.Notification` actually exists. iOS Safari outside
/// an installed PWA has no `Notification` global at all, and *touching*
/// `web_sys::Notification::permission()` there traps the WASM (the binding
/// dereferences an undefined global). Feature-detect via reflection first so
/// the whole notification path can never throw / abort the send-receive flow.
#[cfg(feature = "hydrate")]
fn notifications_available() -> bool {
    let Some(win) = leptos::web_sys::window() else {
        return false;
    };
    match js_sys::Reflect::get(&win, &wasm_bindgen::JsValue::from_str("Notification")) {
        Ok(v) => !v.is_undefined() && !v.is_null(),
        Err(_) => false,
    }
}

/// Ask for Web Notification permission if undecided, and once it is (or
/// already is) granted, register a Web Push subscription so notifications
/// arrive even when the PWA is backgrounded/closed (#30). Must run from a
/// user gesture — `request_permission` is gesture-bound, and on iOS the
/// subscribe that follows it is too, so both ride the same tap. No-ops
/// (never throws) where the API is missing — e.g. iOS Safari outside an
/// installed PWA.
#[cfg(feature = "hydrate")]
pub(super) fn request_notify_permission(s: Shell) {
    use leptos::web_sys::{Notification, NotificationPermission};
    if !notifications_available() {
        return;
    }
    match Notification::permission() {
        NotificationPermission::Default => {
            // Ask; subscribe only after the user actually grants.
            if let Ok(promise) = Notification::request_permission() {
                spawn_local(async move {
                    if let Ok(v) = wasm_bindgen_futures::JsFuture::from(promise).await {
                        if v.as_string().as_deref() == Some("granted") {
                            ensure_push_subscription(s).await;
                        }
                    }
                });
            }
        }
        NotificationPermission::Granted => {
            // Already granted (a prior session, or the first send/mute after
            // push shipped): make sure a subscription exists. Idempotent —
            // getSubscription() short-circuits if we already have one. Runs
            // from this gesture, so iOS is satisfied.
            spawn_local(async move {
                ensure_push_subscription(s).await;
            });
        }
        _ => {}
    }
}

/// Ensure this browser has a Web Push subscription registered with the
/// server. Idempotent: reuses an existing subscription, else subscribes
/// using the server's VAPID public key and POSTs the result. Entirely
/// reflection-driven (no extra web-sys features) and all-or-nothing — any
/// missing API (no `serviceWorker`, no `pushManager`, e.g. iOS Safari
/// outside an installed PWA) just makes it a silent no-op. Call only after
/// Notification permission is granted.
#[cfg(feature = "hydrate")]
async fn ensure_push_subscription(s: Shell) {
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;

    let ok = async {
        let win = leptos::web_sys::window()?;
        let nav = js_sys::Reflect::get(&win, &JsValue::from_str("navigator")).ok()?;
        let sw = js_sys::Reflect::get(&nav, &JsValue::from_str("serviceWorker")).ok()?;
        if sw.is_undefined() || sw.is_null() {
            return None;
        }
        // navigator.serviceWorker.ready : Promise<ServiceWorkerRegistration>
        let ready: js_sys::Promise = js_sys::Reflect::get(&sw, &JsValue::from_str("ready"))
            .ok()?
            .dyn_into()
            .ok()?;
        let reg = JsFuture::from(ready).await.ok()?;
        let pm = js_sys::Reflect::get(&reg, &JsValue::from_str("pushManager")).ok()?;
        if pm.is_undefined() || pm.is_null() {
            return None; // no Push API (e.g. iOS Safari outside an installed PWA)
        }

        // Reuse an existing subscription if the browser already has one.
        let get_sub: js_sys::Function =
            js_sys::Reflect::get(&pm, &JsValue::from_str("getSubscription"))
                .ok()?
                .dyn_into()
                .ok()?;
        let existing = JsFuture::from(
            get_sub
                .call0(&pm)
                .ok()?
                .dyn_into::<js_sys::Promise>()
                .ok()?,
        )
        .await
        .ok()?;

        let subscription = if existing.is_null() || existing.is_undefined() {
            // Fresh subscribe: needs the server's VAPID public key as a
            // Uint8Array applicationServerKey (a base64url string fails on iOS).
            let key_b64 = api::push_vapid_key().await.ok()?.key;
            let key_bytes = base64url_to_bytes(&key_b64)?;
            let key_arr = js_sys::Uint8Array::from(key_bytes.as_slice());

            let opts = js_sys::Object::new();
            js_sys::Reflect::set(&opts, &JsValue::from_str("userVisibleOnly"), &JsValue::TRUE)
                .ok()?;
            js_sys::Reflect::set(&opts, &JsValue::from_str("applicationServerKey"), &key_arr)
                .ok()?;

            let subscribe: js_sys::Function =
                js_sys::Reflect::get(&pm, &JsValue::from_str("subscribe"))
                    .ok()?
                    .dyn_into()
                    .ok()?;
            let p: js_sys::Promise = subscribe.call1(&pm, &opts).ok()?.dyn_into().ok()?;
            JsFuture::from(p).await.ok()?
        } else {
            existing
        };

        // subscription.toJSON() -> { endpoint, keys: { p256dh, auth } }
        let to_json: js_sys::Function =
            js_sys::Reflect::get(&subscription, &JsValue::from_str("toJSON"))
                .ok()?
                .dyn_into()
                .ok()?;
        let json = to_json.call0(&subscription).ok()?;
        let endpoint = js_sys::Reflect::get(&json, &JsValue::from_str("endpoint"))
            .ok()?
            .as_string()?;
        let keys = js_sys::Reflect::get(&json, &JsValue::from_str("keys")).ok()?;
        let p256dh = js_sys::Reflect::get(&keys, &JsValue::from_str("p256dh"))
            .ok()?
            .as_string()?;
        let auth = js_sys::Reflect::get(&keys, &JsValue::from_str("auth"))
            .ok()?
            .as_string()?;

        api::push_subscribe(&crate::protocol::PushSubscribeRequest {
            endpoint,
            keys: crate::protocol::PushSubscriptionKeys { p256dh, auth },
        })
        .await
        .ok()?;
        Some(())
    }
    .await;
    // Mark push live so the poll loop suppresses its duplicate client
    // Notification (server web-push now delivers to backgrounded tabs). Only
    // on a confirmed subscribe — a no-op/failure leaves the poll fallback on.
    if ok.is_some() {
        s.notify.web_push_enabled.set(true);
    }
}

/// Decode a base64url-unpadded string (the VAPID public key) to bytes.
#[cfg(feature = "hydrate")]
fn base64url_to_bytes(s: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s)
        .ok()
}

/// Show `title` as a notification, preferring the service-worker
/// `registration.showNotification()` path so it works when the app runs as
/// an installed PWA (standalone display mode), where the `new Notification()`
/// constructor is unavailable / throws. Falls back to the constructor for a
/// plain browser tab. Every step is fallible and swallowed: this function
/// must never throw and never block the caller (a notification failure must
/// not break message send/receive).
#[cfg(feature = "hydrate")]
fn show_notification(title: &str) {
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::{JsCast, JsValue};

    // SW path: navigator.serviceWorker.ready -> reg.showNotification(title).
    // Driven entirely by reflection (`Reflect::get` + `Function::call`) so
    // it needs no extra web-sys features (no Navigator / ServiceWorker*
    // bindings) and any missing member just yields `None` -> silent
    // fallback. The promise is chained with `.then(onFulfilled, onRejected)`
    // so a rejected `ready`/`showNotification` is swallowed too.
    let sw_dispatched = (|| -> Option<()> {
        let win = leptos::web_sys::window()?;
        // `window.navigator` by reflection (the Navigator web-sys feature
        // isn't enabled in this build).
        let nav = js_sys::Reflect::get(&win, &JsValue::from_str("navigator")).ok()?;
        let sw = js_sys::Reflect::get(&nav, &JsValue::from_str("serviceWorker")).ok()?;
        if sw.is_undefined() || sw.is_null() {
            return None;
        }
        let ready = js_sys::Reflect::get(&sw, &JsValue::from_str("ready")).ok()?;
        let ready: js_sys::Promise = ready.dyn_into().ok()?;
        let title = title.to_owned();
        // `reg.showNotification(title)` once the registration resolves.
        let on_ready = Closure::once_into_js(move |reg: JsValue| {
            let _ = (|| -> Option<()> {
                let show =
                    js_sys::Reflect::get(&reg, &JsValue::from_str("showNotification")).ok()?;
                let show: js_sys::Function = show.dyn_into().ok()?;
                // Returns a Promise; swallow a rejection so it never
                // surfaces as an unhandled rejection.
                let p = show.call1(&reg, &JsValue::from_str(&title)).ok()?;
                if let Ok(p) = p.dyn_into::<js_sys::Promise>() {
                    let noop = js_sys::Function::new_no_args("");
                    let _ = then_via_reflect(&p, &on_ready_noop(), &noop);
                }
                Some(())
            })();
        });
        let on_ready: js_sys::Function = on_ready.dyn_into().ok()?;
        let noop = js_sys::Function::new_no_args("");
        // `ready.then(on_ready, noop)` — invoked reflectively so we pass
        // plain `Function`s instead of typed wasm-bindgen `Closure`s.
        then_via_reflect(&ready, &on_ready, &noop)?;
        Some(())
    })();

    if sw_dispatched.is_some() {
        return;
    }

    // Fallback: plain browser tab. Guard so a throwing constructor (some
    // standalone contexts) can't propagate.
    if notifications_available() {
        let _ = leptos::web_sys::Notification::new(title);
    }
}

/// A no-op fulfilment callback for the inner `showNotification` promise.
#[cfg(feature = "hydrate")]
fn on_ready_noop() -> js_sys::Function {
    js_sys::Function::new_no_args("")
}

/// Call `promise.then(on_fulfilled, on_rejected)` via reflection so the
/// callbacks can be plain `js_sys::Function`s. Returns `None` if `then`
/// is missing or the call traps. Never throws.
#[cfg(feature = "hydrate")]
fn then_via_reflect(
    promise: &js_sys::Promise,
    on_fulfilled: &js_sys::Function,
    on_rejected: &js_sys::Function,
) -> Option<()> {
    use wasm_bindgen::JsCast;
    let then = js_sys::Reflect::get(promise, &wasm_bindgen::JsValue::from_str("then")).ok()?;
    let then: js_sys::Function = then.dyn_into().ok()?;
    then.call2(promise, on_fulfilled, on_rejected).ok()?;
    Some(())
}

/// True when the tab/PWA is backgrounded (so the user would miss messages).
#[cfg(feature = "hydrate")]
fn tab_hidden() -> bool {
    leptos::web_sys::window()
        .and_then(|w| w.document())
        .map(|d| d.hidden())
        .unwrap_or(false)
}

/// Ask the active service worker to close any tray notifications tagged
/// with `cid` (server-side push payload uses the channel id as `tag` — see
/// `src/server/push.rs::send_for_message`). Called from `open_channel_at`
/// (so opening a channel clears its notifs) and from the window-focus
/// handler (so re-focusing the window with a channel already open clears
/// anything that arrived while away). Feedback row kx24k2cwftdppidhmh0e.
///
/// Reflection-driven (same `Reflect::get + Function::call` pattern as the
/// push subscribe path) so we don't pull a new `Navigator`/`ServiceWorker`
/// web-sys feature. Silent no-op when any link in the chain is missing —
/// e.g. browsers without service workers or before the SW has activated.
#[cfg(feature = "hydrate")]
pub fn clear_notifs_for_channel(cid: &str) {
    use wasm_bindgen::{JsCast, JsValue};
    let _ = (|| -> Option<()> {
        let win = leptos::web_sys::window()?;
        let nav = js_sys::Reflect::get(&win, &JsValue::from_str("navigator")).ok()?;
        let sw = js_sys::Reflect::get(&nav, &JsValue::from_str("serviceWorker")).ok()?;
        if sw.is_undefined() || sw.is_null() {
            return None;
        }
        let ctrl = js_sys::Reflect::get(&sw, &JsValue::from_str("controller")).ok()?;
        if ctrl.is_undefined() || ctrl.is_null() {
            // SW not yet controlling the page (first load before claim()).
            return None;
        }
        let post: js_sys::Function = js_sys::Reflect::get(&ctrl, &JsValue::from_str("postMessage"))
            .ok()?
            .dyn_into()
            .ok()?;
        let msg = js_sys::Object::new();
        js_sys::Reflect::set(
            &msg,
            &JsValue::from_str("type"),
            &JsValue::from_str("CLEAR_NOTIFS_TAG"),
        )
        .ok()?;
        js_sys::Reflect::set(&msg, &JsValue::from_str("tag"), &JsValue::from_str(cid)).ok()?;
        post.call1(&ctrl, &msg).ok()?;
        Some(())
    })();
}

/// Auto-dismiss tray notifications for the channel the user is ACTIVELY
/// reading: when fresh messages just landed in the currently-open channel
/// AND the tab is foregrounded, clear that channel's tagged notifications so
/// a push that arrived moments before doesn't linger until the channel is
/// reopened. Feedback row 7ty2eyaoboca2q5lyw37.
///
/// Strictly scoped to `ch` (the open channel passed by the poll loop), so it
/// never touches notifications for other/background channels. No-op when no
/// new messages landed this tick or when the tab is hidden (in which case the
/// user genuinely missed them and should keep the notification). The caller
/// already drops the tick on a stale channel switch, so `ch` is the live open
/// channel here.
#[cfg(feature = "hydrate")]
pub(super) fn dismiss_open_channel_notifs(ch: &ChannelSummary, fresh: &[MessageEnvelope]) {
    if fresh.is_empty() || tab_hidden() {
        return;
    }
    clear_notifs_for_channel(&ch.id);
}

/// Add a `focus` listener to `window` that asks the SW to close any tray
/// notifications tagged with the currently-open channel. Runs once at
/// AppShell mount. The closure stays alive for the page lifetime via
/// `forget()` — we don't want to remove the listener when the AppShell
/// unmounts (the page is gone at that point).
#[cfg(feature = "hydrate")]
pub fn wire_focus_clears_notifs(s: Shell) {
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::JsCast;
    let Some(win) = leptos::web_sys::window() else {
        return;
    };
    let on_focus = Closure::<dyn FnMut()>::new(move || {
        if let Some(ch) = s.sel.sel_channel.get_untracked() {
            clear_notifs_for_channel(&ch.id);
        }
    });
    let _ = win.add_event_listener_with_callback("focus", on_focus.as_ref().unchecked_ref());
    on_focus.forget();
}

/// Add a `message` listener to `navigator.serviceWorker` that deep-links the
/// app when the service worker posts a `NOTIFICATION_CLICK` payload. The SW's
/// `notificationclick` handler (public/sw.js) normally routes a click via
/// `client.navigate()`, but that throws in some standalone/PWA contexts; its
/// fallback posts `{ type: "NOTIFICATION_CLICK", channel, server, message }`
/// to the focused window instead. Without a listener that payload was silently
/// dropped and the click "bugged out" the backgrounded PWA (feedback row
/// br3ebxgjj1lh3qfbz3n8). Here we reuse the exact deep-link path the
/// `/?server=&channel=&m=` query string uses (`open_deep_link`).
///
/// Reflection-driven (same `Reflect::get + Function::call` pattern as the rest
/// of this module) so no extra `Navigator`/`ServiceWorker` web-sys feature is
/// needed. Runs once at AppShell mount; the closure stays alive for the page
/// lifetime via `forget()` (the page is gone when the AppShell unmounts).
#[cfg(feature = "hydrate")]
pub fn wire_notification_click(s: Shell) {
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::{JsCast, JsValue};
    let _ = (|| -> Option<()> {
        let win = leptos::web_sys::window()?;
        let nav = js_sys::Reflect::get(&win, &JsValue::from_str("navigator")).ok()?;
        let sw = js_sys::Reflect::get(&nav, &JsValue::from_str("serviceWorker")).ok()?;
        if sw.is_undefined() || sw.is_null() {
            return None;
        }
        let add: js_sys::Function =
            js_sys::Reflect::get(&sw, &JsValue::from_str("addEventListener"))
                .ok()?
                .dyn_into()
                .ok()?;
        // The MessageEvent carries `data` = the object the SW posted. Read the
        // fields by reflection and, on a NOTIFICATION_CLICK with a channel,
        // follow the same deep-link the query-param path does.
        let on_message = Closure::<dyn FnMut(JsValue)>::new(move |event: JsValue| {
            let _ = (|| -> Option<()> {
                let data = js_sys::Reflect::get(&event, &JsValue::from_str("data")).ok()?;
                if data.is_undefined() || data.is_null() {
                    return None;
                }
                let ty = js_sys::Reflect::get(&data, &JsValue::from_str("type"))
                    .ok()?
                    .as_string()?;
                if ty != "NOTIFICATION_CLICK" {
                    return None;
                }
                // sw.js posts `server` (the guild id), `channel`, `message`.
                let gid = js_sys::Reflect::get(&data, &JsValue::from_str("server"))
                    .ok()?
                    .as_string()?;
                let cid = js_sys::Reflect::get(&data, &JsValue::from_str("channel"))
                    .ok()?
                    .as_string()?;
                let message = js_sys::Reflect::get(&data, &JsValue::from_str("message"))
                    .ok()
                    .and_then(|v| v.as_string());
                super::open_deep_link(s, gid, cid, message);
                Some(())
            })();
        });
        add.call2(&sw, &JsValue::from_str("message"), on_message.as_ref())
            .ok()?;
        on_message.forget();
        Some(())
    })();
}

/// Fire a Web Notification for new messages in `ch` — but only when the tab
/// is backgrounded (you'd see them otherwise), the channel isn't muted, and
/// permission was granted. Title-only to keep the web-sys surface minimal.
#[cfg(feature = "hydrate")]
pub(super) fn notify_messages(s: Shell, ch: &ChannelSummary, fresh: &[MessageEnvelope]) {
    use leptos::web_sys::{Notification, NotificationPermission};
    // FB10b: never locally notify for the user's OWN messages (server
    // web-push already excludes the author; this is the client `Notification`).
    let me = s.sync.me.get_untracked();
    let fresh: Vec<&MessageEnvelope> = fresh
        .iter()
        .filter(|m| me.as_deref() != Some(m.author_id.as_str()))
        .collect();
    if fresh.is_empty() || !tab_hidden() {
        return;
    }
    if s.notify.muted.with_untracked(|m| m.contains(&ch.id)) {
        return;
    }
    // Duplicate-suppression (feedback vkz5t1esl71p8cuxbfjm): when a Web Push
    // subscription is live, the server already delivers a notification to this
    // backgrounded tab — firing the poll-loop `Notification` too would show the
    // message TWICE. Fire the client notification only as a FALLBACK when push
    // is unavailable/unsubscribed (flag false). The flag flips true only after
    // a confirmed `ensure_push_subscription`.
    if s.notify.web_push_enabled.get_untracked() {
        return;
    }
    // Feature-detect before reading permission: on iOS Safari outside an
    // installed PWA the Notification global is absent and the permission
    // read itself would trap.
    if !notifications_available() {
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
    show_notification(&title);
}
