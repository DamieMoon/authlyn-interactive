//! Per-message avatar + send-time formatter. Both are pure helpers shared by
//! the message-row map and the click-the-name info popup.

/// Format an RFC3339 timestamp for display beside the author name.
///
/// On hydrate (browser) we hand the string to JavaScript's `Date`, which
/// parses RFC3339 and renders in the viewer's local timezone + locale.
/// On ssr (native) there is no browser timezone, so we fall back to the
/// raw timestamp — the value is replaced by the localized one as soon as
/// the client hydrates.
#[cfg(feature = "hydrate")]
pub(super) fn format_local_time(sent_at: &str) -> String {
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_str(sent_at));
    // NaN time => unparseable input; keep the raw string rather than show
    // "Invalid Date".
    if date.get_time().is_nan() {
        return sent_at.to_string();
    }
    let undef = wasm_bindgen::JsValue::UNDEFINED;
    let day = String::from(date.to_locale_date_string("default", &undef));
    let time = String::from(date.to_locale_time_string("default"));
    format!("{day} {time}")
}

#[cfg(not(feature = "hydrate"))]
pub(super) fn format_local_time(sent_at: &str) -> String {
    sent_at.to_string()
}

use leptos::prelude::*;

/// A circular persona avatar for chat: the send-time snapshot image (served at
/// `/media/{id}`) when present, else the name's first letter as a monogram.
/// `fill` true makes it fill its parent slot (the info popup's `.info-portrait`);
/// false renders a fixed small inline circle (the per-message meta row). Styled
/// inline because `main.scss` is owned by a parallel work stream.
pub(super) fn chat_avatar(avatar_id: &Option<String>, name: &str, fill: bool) -> impl IntoView {
    let frame = if fill {
        "width:100%;height:100%;border-radius:inherit;overflow:hidden;display:flex;\
         align-items:center;justify-content:center"
            .to_string()
    } else {
        "width:2.5rem;height:2.5rem;border-radius:50%;overflow:hidden;flex:0 0 auto;\
         display:inline-flex;align-items:center;justify-content:center;\
         background:#3a3550;color:#cdb8f0;font-weight:600;font-size:1.05rem;\
         vertical-align:middle;margin-right:0.5rem"
            .to_string()
    };
    match avatar_id {
        Some(id) => {
            // Request a downscaled JPEG thumbnail instead of the full upload so
            // avatars load fast: the small row circle needs ~128px, the popup ~256.
            let tw = if fill { 256 } else { 128 };
            let src = format!("/media/{id}?w={tw}");
            view! {
                <span class="chat-avatar" style=frame>
                    <img src=src alt="" style="width:100%;height:100%;object-fit:cover"/>
                </span>
            }
            .into_any()
        }
        None => {
            let monogram = name
                .chars()
                .next()
                .unwrap_or('?')
                .to_uppercase()
                .to_string();
            view! { <span class="chat-avatar" style=frame>{monogram}</span> }.into_any()
        }
    }
}
