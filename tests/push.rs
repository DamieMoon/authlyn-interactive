//! Wave-1 SAFETY-NET: Web Push subscription CRUD characterization
//! (`src/server/push.rs`). Subscription lifecycle ONLY — `notify_new_message`
//! needs a live push service and is out of scope (audit 019e6c08 / task brief).
//!
//! Locks current behavior:
//!   - subscribe with a complete subscription → 204; re-subscribing the same
//!     endpoint upserts (still 204, one row);
//!   - an incomplete subscription (blank endpoint / p256dh / auth) → 400;
//!   - unsubscribe is endpoint-scoped AND account-scoped: account A's
//!     unsubscribe of account B's endpoint leaves B's row intact;
//!   - `GET /push/vapid-key` → 404 when VAPID is unconfigured (the test env has
//!     no VAPID_* vars, so `PushSender::from_env()` is None → push disabled).

mod common;

#[cfg(feature = "ssr")]
use axum::http::{Method, StatusCode};
#[cfg(feature = "ssr")]
use serde_json::json;
/// Count push_subscription rows for a given endpoint (direct DB inspection).
/// Selects the endpoints and counts in Rust — avoids aggregate-shape ambiguity.
#[cfg(feature = "ssr")]
async fn subs_for_endpoint(
    db: &surrealdb::Surreal<surrealdb::engine::remote::ws::Client>,
    endpoint: &str,
) -> usize {
    let mut resp = db
        .query("SELECT VALUE endpoint FROM push_subscription WHERE endpoint = $e;")
        .bind(("e", endpoint.to_string()))
        .await
        .expect("select query")
        .check()
        .expect("select check");
    let rows: Vec<String> = resp.take(0).expect("take rows");
    rows.len()
}

#[cfg(feature = "ssr")]
fn full_sub(endpoint: &str) -> serde_json::Value {
    json!({
        "endpoint": endpoint,
        "keys": { "p256dh": "BPq256dhkeybase64url", "auth": "authsecretbase64url" }
    })
}

// ---------------------------------------------------------------------------
// vapid-key: 404 when push is unconfigured
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
#[tokio::test]
async fn vapid_key_is_404_when_push_unconfigured() {
    // No VAPID_* env in tests → state.push is None → 404 (so the client skips
    // the whole subscription dance).
    let a = common::arena().await;
    let user = common::register_account(&a.router, "User", "password123").await;
    let (st, _, _) =
        common::send(&a.router, Method::GET, "/push/vapid-key", Some(&user), None).await;
    assert_eq!(
        st,
        StatusCode::NOT_FOUND,
        "vapid-key 404s with push disabled"
    );
}

// ---------------------------------------------------------------------------
// subscribe / unsubscribe lifecycle
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
#[tokio::test]
async fn subscribe_then_unsubscribe() {
    let a = common::arena().await;
    let user = common::register_account(&a.router, "User", "password123").await;
    let endpoint = "https://push.example.com/sub/abc";

    // Subscribe → 204, one row.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        "/push/subscribe",
        Some(&user),
        Some(&full_sub(endpoint)),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert_eq!(subs_for_endpoint(&a.db, endpoint).await, 1);

    // Re-subscribe same endpoint → upsert (still 204, still ONE row).
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        "/push/subscribe",
        Some(&user),
        Some(&full_sub(endpoint)),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert_eq!(
        subs_for_endpoint(&a.db, endpoint).await,
        1,
        "re-subscribe upserts on endpoint (no duplicate)"
    );

    // Unsubscribe → 204, row gone.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        "/push/unsubscribe",
        Some(&user),
        Some(&json!({ "endpoint": endpoint })),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert_eq!(subs_for_endpoint(&a.db, endpoint).await, 0);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn incomplete_subscription_is_400() {
    let a = common::arena().await;
    let user = common::register_account(&a.router, "User", "password123").await;

    // Blank endpoint.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        "/push/subscribe",
        Some(&user),
        Some(&json!({ "endpoint": "  ", "keys": { "p256dh": "x", "auth": "y" } })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "blank endpoint → 400");

    // Blank p256dh.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        "/push/subscribe",
        Some(&user),
        Some(&json!({ "endpoint": "https://e", "keys": { "p256dh": "", "auth": "y" } })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "blank p256dh → 400");

    // Blank auth.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        "/push/subscribe",
        Some(&user),
        Some(&json!({ "endpoint": "https://e", "keys": { "p256dh": "x", "auth": "" } })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "blank auth → 400");

    // A malformed body (missing the keys object) is a JSON-shape rejection → 400.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        "/push/subscribe",
        Some(&user),
        Some(&json!({ "endpoint": "https://e" })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "missing keys object → 400");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn unsubscribe_is_account_scoped() {
    // Account A cannot delete account B's subscription: the DELETE is scoped by
    // BOTH endpoint AND account (push.rs:226-229). We give A and B distinct
    // endpoints, then have A try to unsubscribe B's endpoint — B's row survives.
    let a = common::arena().await;
    let alice = common::register_account(&a.router, "Alice", "password123").await;
    let bob = common::register_account(&a.router, "Bob", "password123").await;
    let bob_endpoint = "https://push.example.com/bob";

    // Bob subscribes.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        "/push/subscribe",
        Some(&bob),
        Some(&full_sub(bob_endpoint)),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert_eq!(subs_for_endpoint(&a.db, bob_endpoint).await, 1);

    // Alice tries to unsubscribe BOB's endpoint. The route still 204s (a DELETE
    // matching nothing is success) but must NOT touch Bob's row.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        "/push/unsubscribe",
        Some(&alice),
        Some(&json!({ "endpoint": bob_endpoint })),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert_eq!(
        subs_for_endpoint(&a.db, bob_endpoint).await,
        1,
        "A's unsubscribe must not delete B's subscription (account-scoped)"
    );

    // Bob can still unsubscribe his own.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        "/push/unsubscribe",
        Some(&bob),
        Some(&json!({ "endpoint": bob_endpoint })),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert_eq!(subs_for_endpoint(&a.db, bob_endpoint).await, 0);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn subscribe_requires_auth() {
    // The subscribe/unsubscribe routes self-gate via AuthAccount.
    let a = common::arena().await;
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        "/push/subscribe",
        None,
        Some(&full_sub("https://e")),
    )
    .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}
