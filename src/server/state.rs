//! Application-wide axum state, shared by every route handler.
//!
//! [`AppState`] holds the SurrealDB handle and the `LeptosOptions` that the
//! Leptos route handlers need. The latter is reachable via `FromRef`, which
//! keeps `leptos_routes` happy while our own routes can still extract the
//! full [`AppState`] when they need the DB.

use std::sync::Arc;

use axum::extract::FromRef;
use leptos::prelude::LeptosOptions;
use surrealdb::engine::remote::ws::Client;
use surrealdb::Surreal;

/// The single state object handed to every axum handler.
///
/// `Clone` is cheap: `LeptosOptions` is small and `Arc<Surreal<Client>>`
/// is just a refcount bump.
#[derive(Clone)]
pub struct AppState {
    /// Owned by main.rs; cloned into the handlers Leptos generates.
    pub leptos: LeptosOptions,
    /// The shared SurrealDB connection. `Surreal<Client>` is `Clone`,
    /// but we wrap it in `Arc` so the cost of cloning `AppState` per
    /// request stays a refcount instead of a full handle clone.
    pub db: Arc<Surreal<Client>>,
}

impl AppState {
    /// Convenience constructor used by tests, which don't actually render
    /// Leptos pages but need *some* `LeptosOptions` so the type system is
    /// happy. The placeholder `output_name` is irrelevant in test runs.
    pub fn new(db: Surreal<Client>) -> Self {
        Self {
            leptos: LeptosOptions::builder().output_name("test").build(),
            db: Arc::new(db),
        }
    }

    /// Build with both halves supplied. Used by `main.rs`.
    pub fn with_leptos(db: Surreal<Client>, leptos: LeptosOptions) -> Self {
        Self {
            leptos,
            db: Arc::new(db),
        }
    }
}

// Required so axum/leptos_axum's `leptos_routes` (which needs
// `LeptosOptions: FromRef<S>`) accepts our combined state.
impl FromRef<AppState> for LeptosOptions {
    fn from_ref(input: &AppState) -> Self {
        input.leptos.clone()
    }
}
