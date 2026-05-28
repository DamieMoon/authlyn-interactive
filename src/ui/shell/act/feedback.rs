//! Feedback actions: submit + admin-archive + the small context-builder that
//! attaches channel id / app version / userAgent to a feedback submission.

use super::super::Shell;
use leptos::prelude::RwSignal;

#[cfg(feature = "hydrate")]
use crate::client::api;
#[cfg(feature = "hydrate")]
use leptos::prelude::*;
#[cfg(feature = "hydrate")]
use leptos::task::spawn_local;

/// Submit a feedback item. Closes the modal on success; surfaces the error
/// via `s.composer.status` on failure.
#[cfg(feature = "hydrate")]
pub fn submit_feedback(
    s: Shell,
    kind: String,
    body: String,
    context: Option<String>,
    modal_open: RwSignal<bool>,
) {
    use crate::protocol::SubmitFeedbackRequest;
    s.composer.status.set(String::new());
    spawn_local(async move {
        match api::submit_feedback(&SubmitFeedbackRequest {
            kind,
            body,
            context,
        })
        .await
        {
            Ok(()) => modal_open.set(false),
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

/// Soft-delete (archive) a feedback item — admin only. The server flips the
/// row's `status` to "deleted" (the row is kept); on success drop it from
/// the loaded inbox so the list updates without a re-fetch.
#[cfg(feature = "hydrate")]
pub fn archive_feedback(
    s: Shell,
    inbox: RwSignal<Option<Vec<crate::protocol::FeedbackItem>>>,
    id: String,
) {
    s.composer.status.set(String::new());
    spawn_local(async move {
        match api::delete_feedback(&id).await {
            Ok(()) => inbox.update(|opt| {
                if let Some(items) = opt {
                    items.retain(|it| it.id != id);
                }
            }),
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

/// Build the context JSON string to attach to a feedback submission.
/// Reads the currently-selected channel id (from Shell signals), the app
/// version (compile-time constant), and navigator.userAgent (browser API).
/// All three are wrapped in a small JSON object and returned as a String.
#[cfg(feature = "hydrate")]
pub fn build_feedback_context(s: Shell) -> Option<String> {
    let channel_id = s
        .sel
        .sel_channel
        .get_untracked()
        .map(|c| c.id)
        .unwrap_or_default();
    let version = env!("CARGO_PKG_VERSION");
    // navigator.userAgent via reflection — `navigator()` is behind the
    // `Navigator` web-sys feature which isn't enabled in this build;
    // the same reflection pattern used by `ensure_push_subscription`.
    let user_agent = (|| {
        use wasm_bindgen::JsValue;
        let win = leptos::web_sys::window()?;
        let nav = js_sys::Reflect::get(&win, &JsValue::from_str("navigator")).ok()?;
        let ua = js_sys::Reflect::get(&nav, &JsValue::from_str("userAgent")).ok()?;
        ua.as_string()
    })()
    .unwrap_or_default();
    // Minimal hand-built JSON — no serde dependency needed for a small static shape.
    let ctx = format!(
        r#"{{"channel_id":{:?},"version":{:?},"user_agent":{:?}}}"#,
        channel_id, version, user_agent
    );
    Some(ctx)
}

// ---- ssr stubs ----

#[cfg(not(feature = "hydrate"))]
pub fn submit_feedback(
    _s: Shell,
    _kind: String,
    _body: String,
    _context: Option<String>,
    _modal_open: RwSignal<bool>,
) {
}
#[cfg(not(feature = "hydrate"))]
pub fn archive_feedback(
    _s: Shell,
    _inbox: RwSignal<Option<Vec<crate::protocol::FeedbackItem>>>,
    _id: String,
) {
}
#[cfg(not(feature = "hydrate"))]
pub fn build_feedback_context(_s: Shell) -> Option<String> {
    None
}
