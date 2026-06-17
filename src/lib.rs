// The hydrate front's deeply-nested view types (AppShell) overflow the default
// type-layout recursion limit when the release profile computes the async
// hydration layout. Raise it crate-wide; harmless for the ssr build.
#![recursion_limit = "512"]

pub mod app;
pub mod client;
pub mod markup;
pub mod protocol;
pub mod ui;

#[cfg(feature = "ssr")]
pub mod db;

#[cfg(feature = "ssr")]
pub mod server;

#[cfg(feature = "ssr")]
pub mod storage;

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    use crate::app::*;
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_body(App);
}
