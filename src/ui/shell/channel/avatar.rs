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

/// Bare 24-hour HH:MM clock for the author's send time — the Orbit orbit's
/// terse timestamp (the prototype's '21:06', a-orbit.html:81), as opposed to the
/// verbose date+time `format_local_time` keeps for deck/hud. Locale-independent
/// like the prototype (zero-padded local getHours/getMinutes); on ssr it falls
/// back to the raw string, replaced on hydrate like its sibling.
#[cfg(feature = "hydrate")]
pub(super) fn format_clock_time(sent_at: &str) -> String {
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_str(sent_at));
    if date.get_time().is_nan() {
        return sent_at.to_string();
    }
    format!("{:02}:{:02}", date.get_hours(), date.get_minutes())
}

#[cfg(not(feature = "hydrate"))]
pub(super) fn format_clock_time(sent_at: &str) -> String {
    sent_at.to_string()
}

use leptos::prelude::*;

use crate::ui::avatar::monogram;

/// A circular persona avatar for chat: the send-time snapshot image (served at
/// `/media/{id}`) when present, else the name's first letter as a monogram.
/// `fill` true makes it fill its parent slot (the info popup's `.info-portrait`);
/// false renders a fixed small inline circle (the per-message meta row).
/// Styling lives on the `.chat-avatar` + `.chat-avatar.fill` rules in
/// style/_content.scss (image dimensions / monogram tile / token colours).
pub(super) fn chat_avatar(avatar_id: &Option<String>, name: &str, fill: bool) -> impl IntoView {
    match avatar_id {
        Some(id) => {
            // The small row circle uses a downscaled JPEG thumbnail so avatars
            // load fast (~128px is ample for a ~40px circle, even at 2x DPR). The
            // `fill` portrait in the persona-info popup is shown large, so serve
            // the FULL-RES original — a fixed-width thumbnail looked blurry scaled
            // up on hi-DPR displays.
            let src = if fill {
                format!("/media/{id}")
            } else {
                format!("/media/{id}?w=128")
            };
            view! {
                <span class="chat-avatar" class:fill=fill>
                    <img src=src alt=""/>
                </span>
            }
            .into_any()
        }
        None => view! { <span class="chat-avatar" class:fill=fill>{monogram(name, '?')}</span> }
            .into_any(),
    }
}
