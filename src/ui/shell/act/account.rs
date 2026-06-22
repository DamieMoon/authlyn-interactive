//! Account-management actions: logout, change-password, admin-reset, and
//! profile edits — display name + avatar (M6). The mutators are `Shell`-driven (writing status on error);
//! `logout` keeps the `Shell` parameter for signature symmetry. Beyond
//! clearing the session user, logout tears the sync driver down (review
//! M-10) and asks the service worker to purge the session-gated media cache
//! (review M-21).

use crate::ui::AuthCtx;

#[cfg(feature = "hydrate")]
use super::super::Shell;
#[cfg(feature = "hydrate")]
use crate::client::api;
#[cfg(feature = "hydrate")]
use leptos::prelude::*;
#[cfg(feature = "hydrate")]
use leptos::task::spawn_local;

#[cfg(feature = "hydrate")]
pub fn logout(_s: Shell, auth: AuthCtx) {
    let nav = leptos_router::hooks::use_navigate();
    // Tear the sync driver down BEFORE the session goes (review M-10):
    // generation bump + EventSource close, so no further /events frame can
    // dispatch into the Shell that the navigation below is about to dispose,
    // and the server stops feeding a client whose session is being revoked.
    super::sync::shutdown();
    spawn_local(async move {
        let _ = api::logout().await;
        // Purge session-gated /media/ blobs from the service worker's Cache
        // Storage (review M-21): GET /media/{id} is session-gated
        // server-side, so the SW's side cache must not outlive the session.
        // Local-residue cleanup — it runs whether or not the server
        // round-trip succeeded, because the user is logged out locally
        // either way.
        clear_media_cache();
        auth.user.set(None);
        nav("/login", Default::default());
    });
}

/// Post `{type:"CLEAR_MEDIA_CACHE"}` to the CONTROLLING service worker; its
/// message handler (public/sw.js) deletes the whole media cache. Driven by
/// reflection (`Reflect::get` + `Function::call`) like `act::notify`, so no
/// extra web-sys features are needed — and null-safe at every hop: dev tabs,
/// first visits, and uninstalled contexts have no controller
/// (`navigator.serviceWorker.controller === null`, and iOS Safari may lack
/// `serviceWorker` entirely outside secure contexts), all degrading to a
/// silent no-op.
#[cfg(feature = "hydrate")]
fn clear_media_cache() {
    use wasm_bindgen::{JsCast, JsValue};
    let Some(win) = web_sys::window() else {
        return;
    };
    let Ok(nav) = js_sys::Reflect::get(&win, &JsValue::from_str("navigator")) else {
        return;
    };
    let Ok(sw) = js_sys::Reflect::get(&nav, &JsValue::from_str("serviceWorker")) else {
        return;
    };
    if sw.is_undefined() || sw.is_null() {
        return;
    }
    let Ok(controller) = js_sys::Reflect::get(&sw, &JsValue::from_str("controller")) else {
        return;
    };
    if controller.is_undefined() || controller.is_null() {
        return;
    }
    let msg = js_sys::Object::new();
    let _ = js_sys::Reflect::set(
        &msg,
        &JsValue::from_str("type"),
        &JsValue::from_str("CLEAR_MEDIA_CACHE"),
    );
    let Ok(post) = js_sys::Reflect::get(&controller, &JsValue::from_str("postMessage")) else {
        return;
    };
    let Ok(post) = post.dyn_into::<js_sys::Function>() else {
        return;
    };
    let _ = post.call1(&controller, &msg);
}

/// Change the signed-in account's password. The new==confirm check is the
/// caller's (the modal's) job; this just hits the API and reports.
#[cfg(feature = "hydrate")]
pub fn change_password(s: Shell, current: String, new: String) {
    s.composer.status.set(String::new());
    spawn_local(async move {
        match api::change_password(&current, &new).await {
            Ok(()) => s.composer.status.set("password changed".to_string()),
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

/// Save the account display name (M6), then refetch /auth/me so the local
/// AuthCtx (name + avatar) updates immediately; the patch_account broadcast
/// refreshes OTHER devices via SSE.
#[cfg(feature = "hydrate")]
pub fn save_display_name(s: Shell, auth: AuthCtx, name: String) {
    s.composer.status.set(String::new());
    spawn_local(async move {
        match api::patch_account(Some(&name), None).await {
            Ok(()) => {
                if let Ok(me) = api::current_user().await {
                    auth.user.set(Some(me));
                }
                s.composer.status.set("profile saved".to_string());
            }
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

/// Upload a picture and set it as the account avatar (M6), then refetch /auth/me
/// to refresh the local AuthCtx. Mirrors `persona::set_persona_avatar`.
#[cfg(feature = "hydrate")]
pub fn set_account_avatar(s: Shell, auth: AuthCtx, file: web_sys::File) {
    s.composer.status.set(String::new());
    spawn_local(async move {
        let media_id = match api::upload_media(&file).await {
            Ok(id) => id,
            Err(e) => {
                s.composer.status.set(api::humanize(&e));
                return;
            }
        };
        match api::patch_account(None, Some(&media_id)).await {
            Ok(()) => {
                if let Ok(me) = api::current_user().await {
                    auth.user.set(Some(me));
                }
            }
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

/// Admin-only: reset another account's password by username.
#[cfg(feature = "hydrate")]
pub fn admin_reset_password(s: Shell, username: String, new_password: String) {
    s.composer.status.set(String::new());
    spawn_local(async move {
        match api::admin_reset_password(&username, &new_password).await {
            Ok(()) => s
                .composer
                .status
                .set(format!("password reset for {username}")),
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

// ---- ssr stubs ----

#[cfg(not(feature = "hydrate"))]
use super::super::Shell;

#[cfg(not(feature = "hydrate"))]
pub fn logout(_s: Shell, _auth: AuthCtx) {}
#[cfg(not(feature = "hydrate"))]
pub fn change_password(_s: Shell, _current: String, _new: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn admin_reset_password(_s: Shell, _username: String, _new_password: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn save_display_name(_s: Shell, _auth: AuthCtx, _name: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn set_account_avatar(_s: Shell, _auth: AuthCtx) {}
