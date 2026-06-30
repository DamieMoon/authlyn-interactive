//! Server binary entrypoint (ssr graph).
//!
//! Boot order — each step gates the next: init the tracing subscriber
//! (route-handler diagnostics → stdout → journald), read the Leptos config,
//! connect to SurrealDB with retries and apply the schema (`db::apply_schema`)
//! **before** serving traffic, ensure the encrypted-attachment media dir exists,
//! build the shared [`authlyn_interactive::server::AppState`], spawn the
//! soft-delete purge sweep (#22), then mount `server::api_router` merged with the
//! Leptos SSR routes and serve on the configured address.
//!
//! Under `not(feature = "ssr")` this collapses to a no-op `main` — the client
//! enters through `lib.rs::hydrate` instead, not here.

// The SSR render instantiates the full nested view type; same depth need as lib.rs.
#![recursion_limit = "512"]

#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() {
    use std::path::PathBuf;

    use authlyn_interactive::app::*;
    use authlyn_interactive::db;
    use authlyn_interactive::server::{self, AppState};
    use leptos::logging::log;
    use leptos::prelude::*;
    use leptos_axum::{generate_route_list, LeptosRoutes};

    // Route handler diagnostics (tracing::error!/warn! across the server layer)
    // to stdout → journald. Without an initialized subscriber these events were
    // silently dropped, leaving 500s with no server-side cause to inspect.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,authlyn_interactive=debug".into()),
        )
        .init();

    let conf = get_configuration(None).unwrap();
    let addr = conf.leptos_options.site_addr;
    let leptos_options = conf.leptos_options;

    // Connect to SurrealDB and apply the schema before serving traffic.
    let surreal = db::connect_with_retries()
        .await
        .expect("SurrealDB connect failed after 10 retries — start the local DB with: surreal start --user root --pass root --bind 127.0.0.1:8000 memory");
    db::apply_schema(&surreal)
        .await
        .expect("SurrealDB schema apply failed");
    log!("SurrealDB schema applied");

    // Encrypted-attachment storage root. The Pi's authlyn.service has
    // `ReadWritePaths=/opt/authlyn/media` reserved for this; locally the
    // default `./media` keeps dev runs self-contained inside the repo
    // checkout. Create-if-missing here so a fresh checkout doesn't have
    // to know to mkdir the dir.
    let media_dir =
        PathBuf::from(std::env::var("MEDIA_STORAGE_DIR").unwrap_or_else(|_| "media".into()));
    std::fs::create_dir_all(&media_dir)
        .unwrap_or_else(|e| panic!("create media_dir {}: {e}", media_dir.display()));
    log!("media storage at {}", media_dir.display());

    // Combine the SurrealDB handle, Leptos config, and media dir in one
    // application state. `FromRef<AppState> for LeptosOptions` lets us
    // keep using the Leptos-provided routing helpers.
    // Web Push (#30): build the VAPID sender from env. `None` = push disabled
    // (the app and every push code path run fine; the client just won't subscribe).
    let push = server::push::PushSender::from_env();
    if push.is_some() {
        log!("Web Push enabled (VAPID configured)");
    } else {
        log!("Web Push disabled (set VAPID_PRIVATE_KEY + VAPID_PUBLIC_KEY to enable)");
    }
    // Nova DOT's LLM backend (#nova): build from env. `None` = `/nova` disabled
    // (the handler 503s); `/novasay` works either way.
    let nova_llm = server::nova_llm::NovaLlm::from_env();
    if nova_llm.is_some() {
        log!("Nova LLM enabled (/nova → Qwen)");
    } else {
        log!("Nova LLM disabled (set NOVA_LLM_URL to enable /nova; /novasay still works)");
    }
    // Nova DOT's ctx tool surface (the knowledge-store bridge): build from env.
    // `None` = tools disabled (the reply path is a single tools-less model call).
    let ctx = server::ctx::CtxClient::from_env();
    if ctx.is_some() {
        log!("Nova ctx tools enabled (query/search/get/recent/store via ctx)");
    } else {
        log!("Nova ctx tools disabled (set CTX_NOVA_MEMORY_KEY to enable)");
    }
    let state = AppState::with_leptos(
        surreal,
        leptos_options.clone(),
        media_dir,
        push,
        nova_llm,
        ctx,
    );

    // Background sweep that hard-deletes soft-deleted rows past their rollback
    // window (#22: message 1h, channel 1d, guild 30d). Runs once at boot, hourly after.
    server::spawn_purge_sweep(state.clone());

    // Generate the list of Leptos SSR routes.
    let routes = generate_route_list(App);

    // Build the merged router: the application-specific HTTP API
    // (`server::api_router`) plus the Leptos handlers, all sharing the
    // single `AppState`.
    let app = server::api_router()
        .leptos_routes(&state, routes, {
            let leptos_options = leptos_options.clone();
            move || shell(leptos_options.clone())
        })
        .fallback(leptos_axum::file_and_error_handler::<AppState, _>(shell))
        // Stamp `Cache-Control: no-cache` on the `/pkg/*` bundle so a fronting CDN
        // (Cloudflare) can't serve a stale stable-named JS/WASM/CSS copy for hours
        // (see `server::pkg_cache_control`). Scoped to `/pkg/`; other paths untouched.
        .layer(axum::middleware::from_fn(server::pkg_cache_control))
        .with_state(state);

    log!("listening on http://{}", &addr);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app.into_make_service())
        .await
        .unwrap();
}

#[cfg(not(feature = "ssr"))]
pub fn main() {
    // no client-side main function
    // unless we want this to work with e.g., Trunk for pure client-side testing
    // see lib.rs for hydration function instead
}
