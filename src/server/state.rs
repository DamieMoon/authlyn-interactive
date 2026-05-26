//! Application-wide axum state, shared by every route handler.
//!
//! [`AppState`] holds the SurrealDB handle, the on-disk media storage
//! root, and the `LeptosOptions` the Leptos route handlers need. The
//! latter is reachable via `FromRef`, which keeps `leptos_routes` happy
//! while our own routes can still extract the full [`AppState`] when
//! they need the DB or `media_dir`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::extract::FromRef;
use leptos::prelude::LeptosOptions;
use surrealdb::engine::remote::ws::Client;
use surrealdb::Surreal;

use crate::server::push::PushSender;

/// The single state object handed to every axum handler.
///
/// `Clone` is cheap: `LeptosOptions` is small, `Arc<Surreal<Client>>` is
/// a refcount bump, and `Arc<PathBuf>` is the same.
#[derive(Clone)]
pub struct AppState {
    /// Owned by main.rs; cloned into the handlers Leptos generates.
    pub leptos: LeptosOptions,
    /// The shared SurrealDB connection. `Surreal<Client>` is `Clone`,
    /// but we wrap it in `Arc` so the cost of cloning `AppState` per
    /// request stays a refcount instead of a full handle clone.
    pub db: Arc<Surreal<Client>>,
    /// Root directory under which `server::media` writes attachment
    /// ciphertext, **canonicalized at construction** (symlinks
    /// resolved, absolute path). Stored canonical so the GET handler's
    /// path-traversal `starts_with` check is a free comparison rather
    /// than a per-request `canonicalize()` stat-chain. The constructor
    /// rejects a non-existent or unreadable dir — main.rs and the test
    /// harness must `create_dir_all` first.
    pub media_dir: Arc<PathBuf>,
    /// Web Push sender (#30), built from VAPID env at startup. `None` = push
    /// disabled (tests, or env unset) — every push path becomes a silent no-op.
    pub push: Option<Arc<PushSender>>,
    /// Ephemeral "is typing" state (#19), keyed channel_id → account_id →
    /// last-ping `Instant`. Deliberately NOT in the DB: it's transient,
    /// high-churn, and surfaced by piggybacking on the message poll. Guarded by
    /// a plain `std::sync::Mutex`; the critical section is only ever a map
    /// insert / read / prune and is NEVER held across an `.await`.
    pub typing: Arc<Mutex<HashMap<String, HashMap<String, Instant>>>>,
}

impl AppState {
    /// Convenience constructor used by tests, which don't actually render
    /// Leptos pages but need *some* `LeptosOptions` so the type system is
    /// happy. The placeholder `output_name` is irrelevant in test runs.
    /// `media_dir` is passed in because the test harness manages its own
    /// per-arena tempdir layout; it is canonicalized here (panicking on
    /// failure — test setup should always be able to canonicalize the
    /// tempdir it just created).
    pub fn new(db: Surreal<Client>, media_dir: PathBuf) -> Self {
        Self {
            leptos: LeptosOptions::builder().output_name("test").build(),
            db: Arc::new(db),
            media_dir: Arc::new(canonicalize_or_panic(media_dir)),
            push: None,
            typing: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Build with all three halves supplied. Used by `main.rs`. Same
    /// canonicalization contract as [`Self::new`].
    pub fn with_leptos(
        db: Surreal<Client>,
        leptos: LeptosOptions,
        media_dir: PathBuf,
        push: Option<Arc<PushSender>>,
    ) -> Self {
        Self {
            leptos,
            db: Arc::new(db),
            media_dir: Arc::new(canonicalize_or_panic(media_dir)),
            push,
            typing: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

fn canonicalize_or_panic(path: PathBuf) -> PathBuf {
    path.canonicalize().unwrap_or_else(|e| {
        panic!(
            "AppState requires an existing, canonicalizable media_dir; got {}: {e}",
            path.display()
        )
    })
}

// Required so axum/leptos_axum's `leptos_routes` (which needs
// `LeptosOptions: FromRef<S>`) accepts our combined state.
impl FromRef<AppState> for LeptosOptions {
    fn from_ref(input: &AppState) -> Self {
        input.leptos.clone()
    }
}
