//! M5/P0 #19 Visual Haptics helper. Adds a vh-* class to an element and
//! removes it on animationend (so it can re-fire). The visual form is the
//! primary feedback language; where navigator.vibrate exists AND the user
//! enabled the vibration enhancement, mirror to a designed pattern.

#[cfg(feature = "hydrate")]
use gloo_storage::{LocalStorage, Storage};

#[cfg(feature = "hydrate")]
const KEY_HAPTIC_VIBRATE: &str = "authlyn.haptic_vibrate";

/// The three feedback kinds in the app's haptic vocabulary. This is the
/// forward-looking CONTRACT every future feature speaks; M5/P0 wires only one
/// live consumer (send-commit → `Thud`), so `Tick`/`Shimmer` are not yet
/// constructed — the allow documents that they are vocabulary, not dead code
/// (Initiative turns, Relay Baton, reactions received will speak them).
/// Ungated on ssr where nothing fires a visual haptic, so allow there too
/// (mirrors `Prefs`'s `cfg_attr` pattern).
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub enum Vh {
    /// Light acknowledge (radial threshold, effect-chip arm, copy). 10ms.
    Tick,
    /// Weighty land (roll result, send commit). 20ms.
    Thud,
    /// Received glint (reaction/resonance received). No vibration.
    Shimmer,
}

#[cfg(feature = "hydrate")]
pub fn haptic_vibrate_enabled() -> bool {
    LocalStorage::get::<String>(KEY_HAPTIC_VIBRATE)
        .map(|v| v == "1")
        .unwrap_or(false)
}

#[cfg(feature = "hydrate")]
pub fn set_haptic_vibrate(on: bool) {
    let _ = LocalStorage::set(KEY_HAPTIC_VIBRATE, if on { "1" } else { "0" });
}

/// Fire the visual haptic on `el`; mirror to navigator.vibrate when enabled.
#[cfg(feature = "hydrate")]
pub fn vh(el: &leptos::web_sys::Element, kind: Vh) {
    use leptos::wasm_bindgen::closure::Closure;
    use leptos::wasm_bindgen::JsCast;
    let class = match kind {
        Vh::Tick => "vh-tick",
        Vh::Thud => "vh-thud",
        Vh::Shimmer => "vh-shimmer",
    };
    let _ = el.class_list().add_1(class);
    // Remove on animationend so a repeat trigger re-fires the animation.
    let el2 = el.clone();
    let class_owned = class.to_string();
    let cb = Closure::<dyn FnMut()>::new(move || {
        let _ = el2.class_list().remove_1(&class_owned);
    });
    // Open Question #6 (resolved against web-sys 0.3.85): the once-option
    // listener binding is `AddEventListenerOptions::new()` (returns a value) +
    // `set_once(true)` (takes &self, returns ()) +
    // `add_event_listener_with_callback_and_add_event_listener_options`.
    // `AddEventListenerOptions` is enabled transitively (tachys); we also add
    // it explicitly to the web-sys feature list so the binding never depends
    // on a transitive crate keeping it.
    let opts = leptos::web_sys::AddEventListenerOptions::new();
    opts.set_once(true);
    let _ = el.add_event_listener_with_callback_and_add_event_listener_options(
        "animationend",
        cb.as_ref().unchecked_ref(),
        &opts,
    );
    cb.forget();
    // Enhancement: mirror to a designed vibration pattern where supported.
    if haptic_vibrate_enabled() {
        if let Some(win) = leptos::web_sys::window() {
            let nav = win.navigator();
            let ms: u32 = match kind {
                Vh::Tick => 10,
                Vh::Thud => 20,
                Vh::Shimmer => return, // shimmer has no vibration
            };
            // Open Question #6 (resolved): `vibrate_with_duration(&self, u32)`
            // is the web-sys 0.3.85 binding (needs the `Navigator` feature,
            // added to the web-sys feature list for this consumer).
            let _ = nav.vibrate_with_duration(ms);
        }
    }
}

// ---- ssr stubs ----
#[cfg(not(feature = "hydrate"))]
pub fn haptic_vibrate_enabled() -> bool {
    false
}
#[cfg(not(feature = "hydrate"))]
pub fn set_haptic_vibrate(_on: bool) {}
