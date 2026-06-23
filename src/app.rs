//! Application root: the HTML document shell rendered by the server and the
//! top-level [`App`] component shared by both graphs.
//!
//! [`shell`] (ssr) emits the `<!DOCTYPE html>` … `<body>` envelope — viewport
//! zoom-lock + safe-area opt-in for the standalone PWA, the manifest/apple-touch
//! icon hints iOS reads for Add-to-Home-Screen, the hydration scripts, and the
//! service-worker registration. [`App`] provides the [`crate::ui::AuthCtx`]
//! session context (resolved once on mount via `/auth/me`, hydrate-only) and the
//! router with the three top-level routes (`/login`, `/register`, `/`).

use leptos::prelude::*;
use leptos_meta::{provide_meta_context, MetaTags, Stylesheet, Title};
use leptos_router::{
    components::{Route, Router, Routes},
    StaticSegment,
};

use crate::ui::auth::{LoginPage, RegisterPage};
use crate::ui::shell::Home;
use crate::ui::AuthCtx;

/// SSR document shell: the full `<html>` envelope wrapping [`App`]. Carries the
/// PWA viewport/safe-area meta, manifest + apple-touch-icon links, the Leptos
/// hydration scripts, and the service-worker registration. Mounted by `main.rs`
/// as both the `leptos_routes` shell and the file/error-handler fallback.
pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8"/>
                // Zoom-lock for the standalone PWA: two-finger pinch-zoom inside the
                // installed app causes erratic ghosting/lag, so disable user scaling
                // (paired with `touch-action` in the SCSS). `viewport-fit=cover` lets
                // the layout extend under the notch/Dynamic Island/home indicator so
                // the `env(safe-area-inset-*)` paddings in the SCSS can reserve space.
                <meta name="viewport" content="width=device-width, initial-scale=1, maximum-scale=1, minimum-scale=1, user-scalable=no, viewport-fit=cover"/>
                // PWA: linked manifest + theme/icon hints. Assets live in
                // `public/` (cargo-leptos `assets-dir`) and are copied to the
                // site root, so they serve at these absolute URLs.
                <link rel="manifest" href="/manifest.webmanifest"/>
                <meta name="theme-color" content="#0b0e14"/>
                <meta name="mobile-web-app-capable" content="yes"/>
                <meta name="apple-mobile-web-app-capable" content="yes"/>
                <meta name="apple-mobile-web-app-status-bar-style" content="black-translucent"/>
                // iOS reads apple-touch-icon (NOT the manifest) for Add-to-Home-
                // Screen; 180×180 is its canonical size. The precomposed variant
                // covers older iOS. Best-effort for adgh3081… (no iPhone to verify).
                <link rel="apple-touch-icon" sizes="180x180" href="/icons/icon-180.png"/>
                <link rel="apple-touch-icon-precomposed" sizes="180x180" href="/icons/icon-180.png"/>
                <link rel="apple-touch-icon" href="/icons/icon-192.png"/>
                <link rel="icon" type="image/png" sizes="192x192" href="/icons/icon-192.png"/>
                <AutoReload options=options.clone() />
                <HydrationScripts options/>
                <MetaTags/>
                // Register the service worker + show the "new version available"
                // refresh banner on update. External file (served from public/) so
                // the logic stays readable and isn't subject to view! escaping.
                // No-op where the serviceWorker API is absent.
                <script src="/register-sw.js"></script>
            </head>
            <body>
                <App/>
            </body>
        </html>
    }
}

/// Top-level component (both graphs): provides [`crate::ui::AuthCtx`], resolves
/// the current session once on mount via `/auth/me` (hydrate-only effect), and
/// mounts the router with the `/login`, `/register`, and `/` (`Home`) routes.
#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    // Session state, resolved once on mount via `/auth/me`.
    let auth = AuthCtx {
        user: RwSignal::new(None),
        loading: RwSignal::new(true),
    };
    provide_context(auth);

    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        leptos::task::spawn_local(async move {
            match crate::client::api::current_user().await {
                Ok(me) => auth.user.set(Some(me)),
                Err(_) => auth.user.set(None),
            }
            auth.loading.set(false);
        });
    });

    view! {
        // Per-build version query (`BUILD_REV` from build.rs, the same rev stamped
        // into the service worker): the file is stable-named, so a fronting CDN
        // (Cloudflare) edge-caches it; a new rev makes the freshly-rendered HTML
        // reference a URL the CDN has never cached → immediate fresh fetch, and
        // never a stale stylesheet on any future deploy. Pairs with the
        // `/pkg` `no-cache` header (server::pkg_cache_control); the static handler
        // ignores the query so the file still resolves.
        <Stylesheet id="leptos" href=concat!("/pkg/authlyn-interactive.css?v=", env!("BUILD_REV"))/>
        <Title text="authlyn"/>

        <Router>
            <main>
                <Routes fallback=|| "Page not found.".into_view()>
                    <Route path=StaticSegment("login") view=LoginPage/>
                    <Route path=StaticSegment("register") view=RegisterPage/>
                    <Route path=StaticSegment("") view=Home/>
                </Routes>
            </main>
        </Router>
    }
}
