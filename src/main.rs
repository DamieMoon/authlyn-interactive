#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() {
    use authlyn_interactive::app::*;
    use authlyn_interactive::db;
    use authlyn_interactive::server::{self, AppState};
    use leptos::logging::log;
    use leptos::prelude::*;
    use leptos_axum::{generate_route_list, LeptosRoutes};

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

    // Combine the SurrealDB handle and Leptos config in one application
    // state. `FromRef<AppState> for LeptosOptions` lets us keep using the
    // Leptos-provided routing helpers.
    let state = AppState::with_leptos(surreal, leptos_options.clone());

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
