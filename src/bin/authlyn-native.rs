//! authlyn-native — the native desktop/Android client (Freya).
//!
//! Standalone bin behind the `freya` cargo feature; never part of the Leptos app
//! graph (mirrors `nova-mcp`). It renders a Skia window via Freya and talks to
//! the same axum REST backend over `reqwest`, reusing the wire DTOs in
//! `protocol`. Phase 1: an authenticated round-trip (login → /auth/me → /guilds).
//!
//! Build/run:
//!   cargo build --bin authlyn-native --features freya
//!   AUTHLYN_NATIVE_URL=http://127.0.0.1:3000 ./authlyn-native
//!
//! Config (env):
//!   AUTHLYN_NATIVE_URL   backend base url   (default http://127.0.0.1:3000)
//!   AUTHLYN_NATIVE_USER  account name       (default "native-dev"; created on
//!                                            first run if it doesn't exist)
//!   AUTHLYN_NATIVE_PASS  account password   (default "native-dev-password")

use authlyn_interactive::native::ui::app;
use freya::prelude::*;
use tokio::runtime::Builder;

fn main() {
    // Freya runs its own executor; `#[tokio::main]` is discouraged. Build a tokio
    // runtime and enter it BEFORE launching so reqwest's tokio APIs work, then let
    // Freya's `spawn`/`use_future` drive async UI updates. (`_rt` must outlive the app.)
    let rt = Builder::new_multi_thread().enable_all().build().unwrap();
    let _rt = rt.enter();

    // One process-global ApiClient holds the session for the app's life.
    authlyn_interactive::native::api::init_client();

    launch(
        LaunchConfig::new().with_window(
            WindowConfig::new(app)
                .with_title("authlyn-native")
                .with_size(1100.0, 860.0),
        ),
    );
}
