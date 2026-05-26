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
                <meta name="viewport" content="width=device-width, initial-scale=1"/>
                // PWA: linked manifest + theme/icon hints. Assets live in
                // `public/` (cargo-leptos `assets-dir`) and are copied to the
                // site root, so they serve at these absolute URLs.
                <link rel="manifest" href="/manifest.webmanifest"/>
                <meta name="theme-color" content="#1e2127"/>
                <meta name="mobile-web-app-capable" content="yes"/>
                <meta name="apple-mobile-web-app-capable" content="yes"/>
                <meta name="apple-mobile-web-app-status-bar-style" content="black-translucent"/>
                <link rel="apple-touch-icon" href="/icons/icon-192.png"/>
                <link rel="icon" type="image/png" sizes="192x192" href="/icons/icon-192.png"/>
                <AutoReload options=options.clone() />
                <HydrationScripts options/>
                <MetaTags/>
                // Register the service worker after load. No-op where the API
                // is absent (older browsers, insecure contexts).
                <script>
                    "if('serviceWorker' in navigator){window.addEventListener('load',function(){navigator.serviceWorker.register('/sw.js').catch(function(e){console.error('SW registration failed:',e);});});}"
                </script>
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
