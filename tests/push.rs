//! Wave-1 SAFETY-NET: Web Push subscription CRUD characterization
//! (`src/server/push.rs`). The full `notify_new_message` send needs a live
//! push service and stays out of scope (audit 019e6c08 / task brief), but the
//! payload's ROW READ (`load_notification_info`) is pure DB and pinned here
//! (review M-42): the `effect` column must survive the SQL projection from a
//! real whisper row through to the masked notification body — an
//! `Option<String>` that silently decodes to `None` when the projection line
//! is dropped would put whisper plaintext on OS lock screens while the
//! in-module formatter unit tests stayed green.
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

// ---------------------------------------------------------------------------
// notification payload row read (review M-42)
// ---------------------------------------------------------------------------

/// Register an owner, create a guild, and return `(cookie, default_text_cid)`.
#[cfg(feature = "ssr")]
async fn owner_with_text_channel(router: &axum::Router) -> (String, String) {
    let owner = common::register_account(router, "Owner", "password123").await;
    let (st, _, guild) = common::send(
        router,
        Method::POST,
        "/guilds",
        Some(&owner),
        Some(&json!({ "name": "Guild" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let gid = guild["id"].as_str().unwrap().to_string();
    let (st, _, detail) = common::send(
        router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let cid = detail["channels"][0]["id"].as_str().unwrap().to_string();
    (owner, cid)
}

/// Review M-42: the effect-column PLUMBING from a real DB row to the push
/// body. The pure formatter (`notification_body`) is unit-tested in-module,
/// but those tests hand it an effect the real path may no longer supply —
/// this one posts a whisper over HTTP and reads the row back through the
/// EXACT SQL projection `notify_inner` uses, pinning that `effect` decodes
/// as `Some("whisper")` and that the composed body is the fixed mask.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn push_payload_row_read_carries_the_effect_column_from_a_real_whisper_row() {
    let a = common::arena().await;
    let (owner, cid) = owner_with_text_channel(&a.router).await;

    // A whispered message lands via the real HTTP send path.
    let (st, _, body) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        Some(&json!({ "body": "the hidden secret", "effect": "whisper" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "whispered post: {body:?}");
    let mid = body["id"].as_str().unwrap().to_string();

    let info = authlyn_interactive::server::push::load_notification_info(&a.state, &mid)
        .await
        .expect("notification row read")
        .expect("the just-posted message must resolve");
    assert_eq!(
        info.effect.as_deref(),
        Some("whisper"),
        "the effect column must survive the SQL projection — None here means \
         the projection line was dropped/misspelled and every whisper rides \
         push payloads in plaintext"
    );
    let pushed = info.notification_body();
    assert!(
        !pushed.contains("hidden secret"),
        "whispered text must never reach the push body, got {pushed:?}"
    );
    assert_eq!(pushed, "(whisper)", "masked with the fixed placeholder");

    // Contrast: a plain message decodes effect=None and passes through.
    let (st, _, body) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        Some(&json!({ "body": "hello there" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let plain_mid = body["id"].as_str().unwrap().to_string();
    let plain = authlyn_interactive::server::push::load_notification_info(&a.state, &plain_mid)
        .await
        .expect("notification row read")
        .expect("plain message resolves");
    assert_eq!(plain.effect, None, "no effect on a plain message");
    assert_eq!(plain.notification_body(), "hello there");

    // A vanished message (deleted between persist and notify) is the
    // documented Ok(None) early-out, never an error.
    let gone = authlyn_interactive::server::push::load_notification_info(&a.state, "nosuchmsg")
        .await
        .expect("unknown id is not a query error");
    assert!(gone.is_none(), "unknown message resolves to None");
}

#[cfg(feature = "ssr")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_subscribe_same_endpoint_converges_to_one_row() {
    // F-D6-3: two simultaneous re-subscribes of the same endpoint (a service
    // worker firing twice) both 204 and converge to one row — never 500 on the
    // push_subscription endpoint UNIQUE index (with_write_conflict_retry).
    let a = common::arena().await;
    let user = common::register_account(&a.router, "User", "password123").await;
    let endpoint = "https://push.example.com/race";
    let body = full_sub(endpoint);
    let req1 =
        common::build_json_request(Method::POST, "/push/subscribe", Some(&user), Some(&body));
    let req2 =
        common::build_json_request(Method::POST, "/push/subscribe", Some(&user), Some(&body));
    let h1 = tokio::spawn(common::status_of(a.router.clone(), req1));
    let h2 = tokio::spawn(common::status_of(a.router.clone(), req2));
    let got = [h1.await.unwrap(), h2.await.unwrap()];
    assert!(
        !got.contains(&StatusCode::INTERNAL_SERVER_ERROR),
        "subscribe race must never 500: {got:?}"
    );
    assert!(
        got.iter().all(|s| *s == StatusCode::NO_CONTENT),
        "both concurrent subscribes should 204: {got:?}"
    );
    assert_eq!(
        subs_for_endpoint(&a.db, endpoint).await,
        1,
        "exactly one subscription row survives the race"
    );
}
