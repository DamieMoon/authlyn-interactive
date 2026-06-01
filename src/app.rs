use leptos::prelude::*;
use leptos_meta::{provide_meta_context, MetaTags, Stylesheet, Title};
use leptos_router::{
    components::{Route, Router, Routes},
    StaticSegment,
};

use crate::ui::auth::{LoginPage, RegisterPage, ResetPage};
use crate::ui::shell::Home;
use crate::ui::AuthCtx;

pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8"/>
                // Zoom-lock for the standalone PWA: two-finger pinch-zoom inside the
                // installed app causes erratic ghosting/lag, so disable user scaling
                // (paired with `touch-action` in the SCSS).
                <meta name="viewport" content="width=device-width, initial-scale=1, maximum-scale=1, minimum-scale=1, user-scalable=no"/>
                // PWA: linked manifest + theme/icon hints. Assets live in
                // `public/` (cargo-leptos `assets-dir`) and are copied to the
                // site root, so they serve at these absolute URLs.
                <link rel="manifest" href="/manifest.webmanifest"/>
                <meta name="theme-color" content="#221c16"/>
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
        <Stylesheet id="leptos" href="/pkg/authlyn-interactive.css"/>
        <Title text="authlyn"/>

        <Router>
            <main>
                <Routes fallback=|| "Page not found.".into_view()>
                    <Route path=StaticSegment("login") view=LoginPage/>
                    <Route path=StaticSegment("register") view=RegisterPage/>
                    <Route path=StaticSegment("reset") view=ResetPage/>
                    <Route path=StaticSegment("") view=Home/>
                </Routes>
            </main>
        </Router>
    }
}
