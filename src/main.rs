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
        .expect("SurrealDB connect failed after 10 retries (is `./scripts/dev-db.sh` running locally, or `surrealdb.service` on the Pi?)");
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
    let state = AppState::with_leptos(surreal, leptos_options.clone(), media_dir, push);

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
