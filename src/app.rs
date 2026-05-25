use leptos::prelude::*;
use leptos_meta::{provide_meta_context, MetaTags, Stylesheet, Title};
use leptos_router::{
    components::{Route, Router, Routes},
    StaticSegment,
};

pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1"/>
                <AutoReload options=options.clone() />
                <HydrationScripts options/>
                <MetaTags/>
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

    view! {
        <Stylesheet id="leptos" href="/pkg/authlyn-interactive.css"/>
        <Title text="authlyn"/>

        <Router>
            <main>
                <Routes fallback=|| "Page not found.".into_view()>
                    <Route path=StaticSegment("") view=Landing/>
                </Routes>
            </main>
        </Router>
    }
}

/// Placeholder landing page during the phase-1 rebuild. The real Discord-style
/// UI (login, server rail, channels, wardrobe, lorebook) lands in `src/ui/`
/// at build step 7 — see `~/.claude/plans/synthetic-zooming-cookie.md`.
#[component]
fn Landing() -> impl IntoView {
    view! {
        <h1>"authlyn"</h1>
        <p>"Roleplay platform — rebuild in progress."</p>
    }
}
