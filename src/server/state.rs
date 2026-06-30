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
use std::time::{Duration, Instant};

use axum::extract::FromRef;
use leptos::prelude::LeptosOptions;
use surrealdb::engine::remote::ws::Client;
use surrealdb::Surreal;

use crate::server::nova_llm::NovaLlm;
use crate::server::push::PushSender;

/// The Ghost Quill draft store (M4/T7): `(channel_id, account_id)` →
/// `(draft text, last-ping Instant)`. Named so the `AppState` field stays
/// readable (and clippy's type-complexity lint quiet).
pub type TypingDraftMap = HashMap<(String, String), (String, Instant)>;

/// Production TTL for a stored typing draft (M4/T7 Ghost Quill). Matches the
/// typing indicator's 8s (`server::messages::typing::TYPING_TTL`) on purpose:
/// the same ~2s ping cadence keeps both alive, so the ghost row and the
/// "is typing" line expire together.
pub const DEFAULT_DRAFT_TTL: Duration = Duration::from_secs(8);

/// Production period for the SSE forced session re-check (review M-05
/// follow-up): a `/events` connection re-validates its session at least once
/// per period REGARDLESS of bus traffic — the re-check runs on a deadline
/// that only an actual re-check (deadline lapse, or a delivered frame's
/// per-frame gate) advances, never a mere bus receive. So a revoked session
/// dies within ~one period whether its stream is fully silent OR fed only
/// events the privacy/target filters drop (which complete `recv()` without
/// ever reaching the per-frame gate). 30s ≈ one revalidation per idle
/// connection per half-minute — noise-level next to the per-delivered-frame
/// checks an active stream already issues.
pub const DEFAULT_SSE_RECHECK_PERIOD: Duration = Duration::from_secs(30);

/// Bus envelope: an event plus its delivery scope. `targets: None` = the
/// existing visibility-filtered/global path; `Some(accounts)` = deliver ONLY
/// to those accounts' connections (bypasses channel-visibility filtering —
/// targeted events must therefore never carry data the target may not see;
/// they are id-only nudges like everything else on this bus).
#[derive(Clone, Debug)]
pub struct BusEvent {
    pub event: crate::protocol::SyncEvent,
    pub targets: Option<Vec<String>>,
}

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
    /// Nova DOT's LLM backend (`/nova`, `server::nova_llm`), built from env at
    /// startup. `None` = `/nova` disabled (the handler 503s; `/novasay` is
    /// unaffected — it needs no model). Tests inject a stub via
    /// [`AppState::with_nova_llm`].
    pub nova_llm: Option<Arc<NovaLlm>>,
    /// Ephemeral "is typing" state (#19), keyed channel_id → account_id →
    /// last-ping `Instant`. Deliberately NOT in the DB: it's transient,
    /// high-churn, and surfaced by piggybacking on the message poll. Guarded by
    /// a plain `std::sync::Mutex`; the critical section is only ever a map
    /// insert / read / prune and is NEVER held across an `.await`.
    pub typing: Arc<Mutex<HashMap<String, HashMap<String, Instant>>>>,
    /// Ghost Quill (M4/T7): ephemeral live-draft store, keyed
    /// `(channel_id, account_id)` → `(draft text, last-ping Instant)`.
    /// Mirrors `typing`'s discipline exactly: in-memory only (never the DB),
    /// TTL-pruned opportunistically on write and read, and the `Mutex` is
    /// NEVER held across an `.await`. Draft TEXT lives ONLY here and is
    /// surfaced ONLY through the permission-checked
    /// `GET /channels/{cid}/typing-drafts` — it never rides the SSE bus,
    /// which stays id-only by design.
    pub typing_drafts: Arc<Mutex<TypingDraftMap>>,
    /// How long a stored typing draft stays live without a refreshing ping.
    /// [`DEFAULT_DRAFT_TTL`] (8s) in production; injectable via
    /// [`AppState::with_draft_ttl`] so the prune tests don't sleep 8s. Plain
    /// `Copy` data — set it BEFORE the state is cloned into the router.
    pub draft_ttl: Duration,
    /// Longest a `/events` stream may go WITHOUT a session re-check (review
    /// M-05 follow-up — the per-frame gate only fires on DELIVERY, so
    /// without this a revoked session on a stream that delivers nothing,
    /// whether the bus is silent or all its events are filtered out, would
    /// hold its connection open indefinitely). Enforced as a deadline that
    /// only an actual re-check advances — see [`DEFAULT_SSE_RECHECK_PERIOD`].
    /// That default (30s) in production; injectable via
    /// [`AppState::with_sse_recheck_period`] so the revocation test doesn't
    /// sleep 30s. Plain `Copy` data — set it BEFORE the state is cloned into
    /// the router.
    pub sse_recheck_period: Duration,
    /// M1 realtime: the process-wide SSE event bus. Every mutation handler
    /// best-effort `send()`s a [`BusEvent`]; every `GET /events` connection
    /// subscribes. Capacity 256: laggards get `RecvError::Lagged` and are
    /// nudged to resync — events are droppable by design (notify-and-fetch).
    /// The envelope's `targets` field (M1.5) selects the visibility-filtered
    /// path (`None`, via [`AppState::emit`]) or the account-targeted lane
    /// (`Some`, via [`AppState::emit_for`]).
    pub events: tokio::sync::broadcast::Sender<BusEvent>,
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
            nova_llm: None,
            typing: Arc::new(Mutex::new(HashMap::new())),
            typing_drafts: Arc::new(Mutex::new(HashMap::new())),
            draft_ttl: DEFAULT_DRAFT_TTL,
            sse_recheck_period: DEFAULT_SSE_RECHECK_PERIOD,
            events: tokio::sync::broadcast::channel(256).0,
        }
    }

    /// Override the typing-draft TTL (M4/T7) — test injectability so the
    /// prune behavior is provable without sleeping out the production 8s.
    /// Builder-style: apply BEFORE handing the state to `make_router`
    /// (`draft_ttl` is `Copy`, so a later mutation never reaches the
    /// router's clone).
    pub fn with_draft_ttl(mut self, ttl: Duration) -> Self {
        self.draft_ttl = ttl;
        self
    }

    /// Override the SSE forced session re-check period (review M-05
    /// follow-up) — test injectability so the revocation-without-delivery
    /// tests run in milliseconds instead of sleeping out the production 30s. Same
    /// builder-before-`make_router` contract as [`Self::with_draft_ttl`]
    /// (`sse_recheck_period` is `Copy`).
    pub fn with_sse_recheck_period(mut self, period: Duration) -> Self {
        self.sse_recheck_period = period;
        self
    }

    /// Inject Nova DOT's LLM backend (`/nova`) — test injectability so the flow
    /// is provable with a [`crate::server::nova_llm::NovaLlm::stub`] instead of a
    /// network model. Same builder-before-`make_router` contract as the others.
    pub fn with_nova_llm(mut self, nova: Arc<NovaLlm>) -> Self {
        self.nova_llm = Some(nova);
        self
    }

    /// Best-effort bus emission: never fails the request. `send()` errs only
    /// when no subscriber exists (the idle case) — see the `events` field doc
    /// for the capacity/lag rationale. `targets: None` = the visibility-filtered
    /// path every pre-M1.5 call site means.
    pub fn emit(&self, ev: crate::protocol::SyncEvent) {
        let _ = self.events.send(BusEvent {
            event: ev,
            targets: None,
        });
    }

    /// Targeted best-effort emission: delivered only to `accounts`' connections.
    /// Bypasses channel-visibility filtering (see [`BusEvent`]) — only ever pass
    /// id-only nudges whose mere arrival reveals nothing beyond what the target
    /// may already fetch.
    pub fn emit_for(&self, accounts: Vec<String>, ev: crate::protocol::SyncEvent) {
        let _ = self.events.send(BusEvent {
            event: ev,
            targets: Some(accounts),
        });
    }

    /// Build with all three halves supplied. Used by `main.rs`. Same
    /// canonicalization contract as [`Self::new`].
    pub fn with_leptos(
        db: Surreal<Client>,
        leptos: LeptosOptions,
        media_dir: PathBuf,
        push: Option<Arc<PushSender>>,
        nova_llm: Option<Arc<NovaLlm>>,
    ) -> Self {
        Self {
            leptos,
            db: Arc::new(db),
            media_dir: Arc::new(canonicalize_or_panic(media_dir)),
            push,
            nova_llm,
            typing: Arc::new(Mutex::new(HashMap::new())),
            typing_drafts: Arc::new(Mutex::new(HashMap::new())),
            draft_ttl: DEFAULT_DRAFT_TTL,
            sse_recheck_period: DEFAULT_SSE_RECHECK_PERIOD,
            events: tokio::sync::broadcast::channel(256).0,
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
