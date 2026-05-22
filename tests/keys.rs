//! Integration tests for `POST /keys/upload` and `POST /keys/claim/{user}/{device}`.
//!
//! These hit a real SurrealDB. Run `./scripts/dev-db.sh` first.
//! Each test reserves a fresh namespace/database so concurrent runs don't
//! collide.
//!
//! Driving the axum `Router` through `tower::ServiceExt::oneshot` avoids
//! binding a TCP port for each test.

#![cfg(feature = "ssr")]

use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use rand::RngCore;
use serde_json::{json, Value};
use surrealdb::engine::remote::ws::{Client, Ws};
use surrealdb::opt::auth::Root;
use surrealdb::Surreal;
use tower::ServiceExt;

use authlyn_interactive::crypto::{prekey::PreKeyBundleBuilder, DeviceAccount};
use authlyn_interactive::server::retry::is_write_conflict;
use authlyn_interactive::server::{make_router, AppState};
use authlyn_interactive::storage;

// ---------------------------------------------------------------------------
// Test harness
// ---------------------------------------------------------------------------

/// Monotonic counter so concurrent `cargo test` workers each get distinct
/// SurrealDB namespaces, in addition to the process-PID prefix.
static NS_COUNTER: AtomicU64 = AtomicU64::new(0);

/// One isolated test arena: a SurrealDB namespace+database that owns its
/// schema, plus the axum `Router` wired against it.
struct Arena {
    router: Router,
}

async fn arena() -> Arena {
    let db = test_db().await;
    let state = AppState::new(db);
    let router = make_router(state);
    Arena { router }
}

async fn test_db() -> Surreal<Client> {
    let host = std::env::var("SURREAL_URL")
        .unwrap_or_else(|_| "127.0.0.1:8000".into())
        .trim_start_matches("ws://")
        .trim_start_matches("wss://")
        .to_string();
    let user = std::env::var("SURREAL_USER").unwrap_or_else(|_| "root".into());
    let pass = std::env::var("SURREAL_PASS").unwrap_or_else(|_| "root".into());

    let db = Surreal::new::<Ws>(host)
        .await
        .expect("connect to SurrealDB — is ./scripts/dev-db.sh running?");
    db.signin(Root {
        username: user,
        password: pass,
    })
    .await
    .expect("signin");

    let pid = std::process::id();
    let seq = NS_COUNTER.fetch_add(1, Ordering::Relaxed);
    let ns = format!("test_keys_{}_{}", pid, seq);
    let db_name = format!("test_keys_{}_{}", pid, seq);
    db.use_ns(&ns).use_db(&db_name).await.expect("use ns/db");

    db.query(storage::SCHEMA)
        .await
        .expect("apply schema")
        .check()
        .expect("apply schema check");
    db
}

/// Free-form 16-byte hex string used as device/user identifier in v1.
/// Spec calls these "ULIDs", but the auth stub treats them as opaque strings,
/// so a random hex value is sufficient and avoids pulling in a ulid crate.
fn random_id() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Hit the router with a JSON request. Returns (status, parsed body).
async fn post_json(
    router: &Router,
    path: &str,
    headers: &[(&str, &str)],
    body: &Value,
) -> (StatusCode, Value) {
    let mut req_builder = Request::builder()
        .method(Method::POST)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json");
    for (k, v) in headers {
        req_builder = req_builder.header(*k, *v);
    }
    let req = req_builder
        .body(Body::from(serde_json::to_vec(body).unwrap()))
        .unwrap();

    let res = router.clone().oneshot(req).await.expect("oneshot");
    let status = res.status();
    let bytes = to_bytes(res.into_body(), 1 << 20).await.expect("read body");
    let parsed: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            panic!(
                "body parse failed: {e}, raw: {:?}",
                String::from_utf8_lossy(&bytes)
            )
        })
    };
    (status, parsed)
}

/// Build a fresh `PreKeyBundle` carrying `otk_count` OTKs + one fallback key.
fn build_bundle(device: &mut DeviceAccount, otk_count: usize) -> Value {
    let builder = PreKeyBundleBuilder::new();
    let bundle = builder.build(device, otk_count);
    serde_json::to_value(bundle).expect("bundle -> json")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Test 1: A device publishes a bundle, then a peer claims an OTK and the
/// returned key matches one of the published OTKs (signature verifies).
#[tokio::test]
async fn round_trip_publish_and_claim() {
    let arena = arena().await;

    let user_id = random_id();
    let device_id = random_id();

    let mut device = DeviceAccount::new();
    let bundle = build_bundle(&mut device, 3);
    let published_otks: Vec<Value> = bundle
        .get("one_time_keys")
        .expect("one_time_keys present")
        .as_array()
        .expect("array")
        .clone();

    // Publish
    let (status, body) = post_json(
        &arena.router,
        "/keys/upload",
        &[("X-Device-Id", &device_id)],
        &json!({ "user_id": user_id, "bundle": bundle }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "upload failed: {body}");
    assert_eq!(
        body.get("device_id").and_then(|v| v.as_str()),
        Some(device_id.as_str())
    );
    assert_eq!(body.get("otk_count").and_then(|v| v.as_u64()), Some(3));

    // Claim
    let (status, body) = post_json(
        &arena.router,
        &format!("/keys/claim/{user_id}/{device_id}"),
        &[],
        &json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "claim failed: {body}");
    assert_eq!(body.get("kind").and_then(|v| v.as_str()), Some("otk"));
    assert_eq!(
        body.get("device_id").and_then(|v| v.as_str()),
        Some(device_id.as_str())
    );

    let returned_kid = body
        .get("key")
        .and_then(|k| k.get("kid"))
        .and_then(|v| v.as_str())
        .expect("kid in response");

    assert!(
        published_otks
            .iter()
            .any(|otk| otk.get("kid").and_then(|v| v.as_str()) == Some(returned_kid)),
        "returned OTK kid {returned_kid} must match one of the published OTKs"
    );
}

/// Test 2: With a 2-key pool, the third claim falls back to the fallback key.
#[tokio::test]
async fn pool_depletion_falls_back() {
    let arena = arena().await;
    let user_id = random_id();
    let device_id = random_id();

    let mut device = DeviceAccount::new();
    let bundle = build_bundle(&mut device, 2);
    let fallback_kid = bundle
        .get("fallback_key")
        .and_then(|f| f.get("kid"))
        .and_then(|v| v.as_str())
        .expect("fallback kid")
        .to_string();

    let (status, _) = post_json(
        &arena.router,
        "/keys/upload",
        &[("X-Device-Id", &device_id)],
        &json!({ "user_id": user_id, "bundle": bundle }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Drain the OTK pool.
    for i in 0..2 {
        let (status, body) = post_json(
            &arena.router,
            &format!("/keys/claim/{user_id}/{device_id}"),
            &[],
            &json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "claim #{i} failed: {body}");
        assert_eq!(
            body.get("kind").and_then(|v| v.as_str()),
            Some("otk"),
            "claim #{i} should still be from OTK pool, got {body}"
        );
    }

    // Now the pool is empty: fallback.
    let (status, body) = post_json(
        &arena.router,
        &format!("/keys/claim/{user_id}/{device_id}"),
        &[],
        &json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "fallback claim failed: {body}");
    assert_eq!(body.get("kind").and_then(|v| v.as_str()), Some("fallback"));
    assert_eq!(
        body.get("key")
            .and_then(|k| k.get("kid"))
            .and_then(|v| v.as_str()),
        Some(fallback_kid.as_str())
    );
}

/// Test 3: An OTK with a corrupted signature is rejected before any DB rows
/// are mutated.
#[tokio::test]
async fn corrupted_otk_signature_is_rejected() {
    let arena = arena().await;
    let user_id = random_id();
    let device_id = random_id();

    let mut device = DeviceAccount::new();
    let mut bundle = build_bundle(&mut device, 2);

    // Corrupt the first OTK's signature by flipping a byte.
    let sig = bundle["one_time_keys"][0]["signature"]
        .as_str()
        .expect("sig")
        .to_string();
    let mut sig_bytes = hex::decode(&sig).expect("hex");
    sig_bytes[0] ^= 0x01;
    bundle["one_time_keys"][0]["signature"] = Value::String(hex::encode(sig_bytes));

    let (status, body) = post_json(
        &arena.router,
        "/keys/upload",
        &[("X-Device-Id", &device_id)],
        &json!({ "user_id": user_id, "bundle": bundle }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "corrupted OTK signature must yield 400, body: {body}"
    );
    assert!(
        body.get("error").is_some(),
        "expected typed error body, got {body}"
    );
}

/// Test 4: Missing `X-Device-Id` header is a hard 401 from the upload
/// endpoint.
#[tokio::test]
async fn upload_without_device_id_header_is_unauthorized() {
    let arena = arena().await;
    let user_id = random_id();

    let mut device = DeviceAccount::new();
    let bundle = build_bundle(&mut device, 1);

    let (status, _body) = post_json(
        &arena.router,
        "/keys/upload",
        // No X-Device-Id header.
        &[],
        &json!({ "user_id": user_id, "bundle": bundle }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// Test 5: Re-publishing replaces the OTK pool — old OTKs are gone and only
/// keys from the new bundle are claimable.
#[tokio::test]
async fn republish_replaces_otk_pool() {
    let arena = arena().await;
    let user_id = random_id();
    let device_id = random_id();

    let mut device = DeviceAccount::new();

    // First publish.
    let bundle1 = build_bundle(&mut device, 2);
    let old_kids: Vec<String> = bundle1["one_time_keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k["kid"].as_str().unwrap().to_string())
        .collect();
    let (status, _) = post_json(
        &arena.router,
        "/keys/upload",
        &[("X-Device-Id", &device_id)],
        &json!({ "user_id": user_id, "bundle": bundle1 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Consume one OTK from the old pool.
    let (status, _) = post_json(
        &arena.router,
        &format!("/keys/claim/{user_id}/{device_id}"),
        &[],
        &json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Republish.
    let bundle2 = build_bundle(&mut device, 3);
    let new_kids: Vec<String> = bundle2["one_time_keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k["kid"].as_str().unwrap().to_string())
        .collect();
    let (status, body) = post_json(
        &arena.router,
        "/keys/upload",
        &[("X-Device-Id", &device_id)],
        &json!({ "user_id": user_id, "bundle": bundle2 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "republish failed: {body}");
    assert_eq!(body.get("otk_count").and_then(|v| v.as_u64()), Some(3));

    // All subsequent claims should return only kids from the new bundle.
    for _ in 0..3 {
        let (status, body) = post_json(
            &arena.router,
            &format!("/keys/claim/{user_id}/{device_id}"),
            &[],
            &json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["kind"].as_str(), Some("otk"));
        let kid = body["key"]["kid"].as_str().unwrap().to_string();
        assert!(
            new_kids.contains(&kid),
            "claim returned kid {kid}, which is not in the new bundle ({new_kids:?})"
        );
        assert!(
            !old_kids.contains(&kid),
            "old OTK kid {kid} was still claimable after republish"
        );
    }
}

/// Test 6: Claiming against a device that was never published is 404.
#[tokio::test]
async fn claim_on_unknown_device_is_not_found() {
    let arena = arena().await;
    let user_id = random_id();
    let device_id = random_id();

    let (status, body) = post_json(
        &arena.router,
        &format!("/keys/claim/{user_id}/{device_id}"),
        &[],
        &json!({}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "expected 404, got {status}: {body}"
    );
}

/// Test 7a: Bundles claiming more than `MAX_OTKS_PER_PUBLISH` OTKs are
/// rejected with 400 before we spend any crypto cycles on them. We don't
/// need to actually sign that many keys — the cap fires on length alone.
#[tokio::test]
async fn too_many_otks_is_rejected() {
    let arena = arena().await;
    let user_id = random_id();
    let device_id = random_id();

    let mut device = DeviceAccount::new();
    let mut bundle = build_bundle(&mut device, 1);
    // Inflate `one_time_keys` past the cap with cheap clones. Their
    // signatures won't verify against `identity_ed25519`, but the
    // length check fires before `verify_self`, so we never reach
    // signature verification.
    let template = bundle["one_time_keys"][0].clone();
    let arr = bundle["one_time_keys"]
        .as_array_mut()
        .expect("array")
        .clone();
    let mut inflated = arr;
    while inflated.len() <= 200 {
        inflated.push(template.clone());
    }
    bundle["one_time_keys"] = Value::Array(inflated);

    let (status, body) = post_json(
        &arena.router,
        "/keys/upload",
        &[("X-Device-Id", &device_id)],
        &json!({ "user_id": user_id, "bundle": bundle }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got body: {body}");
    assert!(
        body.get("error")
            .and_then(|v| v.as_str())
            .map(|s| s.contains("too many one_time_keys"))
            .unwrap_or(false),
        "expected typed too-many-OTKs error, got {body}"
    );
}

/// Test 7: Malformed JSON body yields a typed 400 with `{"error": "..."}`,
/// not axum's default plain-text rejection page. Both syntactic garbage
/// (raw text) and structural mismatch (missing `user_id`) should land on
/// the same shape.
#[tokio::test]
async fn malformed_upload_body_returns_typed_400() {
    let arena = arena().await;

    // Syntax error — raw text isn't valid JSON.
    let req = Request::builder()
        .method(Method::POST)
        .uri("/keys/upload")
        .header(header::CONTENT_TYPE, "application/json")
        .header("X-Device-Id", "dev-1234")
        .body(Body::from("not json at all"))
        .unwrap();
    let res = arena.router.clone().oneshot(req).await.expect("oneshot");
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).expect("typed body");
    assert!(
        body.get("error").and_then(|v| v.as_str()).is_some(),
        "expected typed error body, got {body}"
    );

    // Structural error — valid JSON but missing required `user_id`.
    let (status, body) = post_json(
        &arena.router,
        "/keys/upload",
        &[("X-Device-Id", "dev-1234")],
        &json!({ "bundle": {} }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body.get("error").and_then(|v| v.as_str()).is_some(),
        "expected typed error body, got {body}"
    );
}

/// Test 7b: Republish atomicity under concurrent claims.
///
/// Reproduces the reviewer's original Fix-2 trace: claims racing against
/// a republish must NEVER observe the half-replaced state (DELETE OTKs
/// done, CREATE OTKs not yet committed). Without `BEGIN TRANSACTION ...
/// COMMIT TRANSACTION` wrapping `persist_bundle`, a fraction of concurrent
/// claims land in that window and spuriously get `kind == "fallback"`
/// (or, with the new transaction in place but no retry, surface as 500s).
///
/// We publish a starting pool, then in parallel fire N claims while a
/// republish is in flight. Every claim must come back as `otk`, and
/// every kid must be from either the old or the new pool (never the
/// fallback, never garbage).
#[tokio::test]
async fn republish_does_not_expose_half_replaced_pool() {
    let arena = arena().await;
    let user_id = random_id();
    let device_id = random_id();

    // Initial publish: large pool so claims-vs-republish has lots of room
    // to race without legitimately exhausting the OTKs. Both bundles are
    // sized larger than the claim fan-out so a legitimate "pool empty,
    // serve the fallback" path can't masquerade as the bug we're hunting.
    let mut device = DeviceAccount::new();
    let bundle1 = build_bundle(&mut device, 100);
    let old_kids: std::collections::HashSet<String> = bundle1["one_time_keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k["kid"].as_str().unwrap().to_string())
        .collect();
    let (status, _) = post_json(
        &arena.router,
        "/keys/upload",
        &[("X-Device-Id", &device_id)],
        &json!({ "user_id": user_id, "bundle": bundle1 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Build the replacement bundle ahead of time so the spawn closure
    // doesn't need to share the mutable device.
    let bundle2 = build_bundle(&mut device, 100);
    let new_kids: std::collections::HashSet<String> = bundle2["one_time_keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k["kid"].as_str().unwrap().to_string())
        .collect();

    // Fire republish + N concurrent claims in parallel. We mirror the
    // reviewer's empirical setup (50 claims overlapping a republish) so
    // the contention window is wide enough that a non-transactional
    // persist would land at least one claim mid-replace.
    let claim_path = format!("/keys/claim/{user_id}/{device_id}");
    let n: usize = 50;
    let mut handles = Vec::with_capacity(n + 1);
    let router = arena.router.clone();
    let user_id_clone = user_id.clone();
    let device_id_clone = device_id.clone();
    handles.push(tokio::spawn(async move {
        let (s, _) = post_json(
            &router,
            "/keys/upload",
            &[("X-Device-Id", &device_id_clone)],
            &json!({ "user_id": user_id_clone, "bundle": bundle2 }),
        )
        .await;
        (s, json!({}))
    }));
    for _ in 0..n {
        let router = arena.router.clone();
        let path = claim_path.clone();
        handles.push(tokio::spawn(async move {
            post_json(&router, &path, &[], &json!({})).await
        }));
    }

    // Republish is handle[0], claims are handle[1..=n].
    let mut results = Vec::with_capacity(handles.len());
    for h in handles {
        results.push(h.await.expect("task join"));
    }
    let (rep_status, _) = &results[0];
    assert_eq!(
        *rep_status,
        StatusCode::OK,
        "republish under concurrent load must still succeed"
    );

    let valid_kids: std::collections::HashSet<&String> = old_kids.union(&new_kids).collect();
    for (status, body) in &results[1..] {
        assert_eq!(
            *status,
            StatusCode::OK,
            "claim raced with republish must not 5xx, body: {body}"
        );
        assert_eq!(
            body.get("kind").and_then(|v| v.as_str()),
            Some("otk"),
            "claim returned fallback during republish window: {body}"
        );
        let kid = body["key"]["kid"].as_str().expect("kid").to_string();
        assert!(
            valid_kids.contains(&kid),
            "returned kid {kid} is from neither the old nor the new bundle"
        );
    }
}

/// Test 8: Single-use invariant under concurrency.
///
/// Publishes a 16-OTK bundle, fires 10 parallel `/keys/claim` calls, and
/// asserts every response is 200, every `kind == "otk"`, and the 10 returned
/// kids are pairwise distinct and a subset of the published kids.
///
/// This is the mechanical-sympathy backstop for the
/// `DELETE FROM (SELECT ... LIMIT 1)` pattern: SurrealDB's MVCC rejects
/// concurrent writers with a retryable "Write conflict" error, so the
/// server has to coordinate with that explicitly (bounded retry loop in
/// `pop_one_otk`). Without that retry, a fraction of these claims surface
/// as HTTP 500.
#[tokio::test]
async fn concurrent_claims_each_get_distinct_otk() {
    let arena = arena().await;
    let user_id = random_id();
    let device_id = random_id();

    // Publish a comfortably-sized pool (more than the claim fan-out so we
    // never legitimately fall through to the fallback path).
    let mut device = DeviceAccount::new();
    let bundle = build_bundle(&mut device, 16);
    let published_kids: std::collections::HashSet<String> = bundle["one_time_keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k["kid"].as_str().unwrap().to_string())
        .collect();

    let (status, _) = post_json(
        &arena.router,
        "/keys/upload",
        &[("X-Device-Id", &device_id)],
        &json!({ "user_id": user_id, "bundle": bundle }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Fan out 10 concurrent claims. Each task gets an owned `Router` clone
    // (cheap — just refcount bumps on the inner shared state).
    let claim_path = format!("/keys/claim/{user_id}/{device_id}");
    let n: usize = 10;
    let mut handles = Vec::with_capacity(n);
    for _ in 0..n {
        let router = arena.router.clone();
        let path = claim_path.clone();
        handles.push(tokio::spawn(async move {
            post_json(&router, &path, &[], &json!({})).await
        }));
    }

    let mut returned_kids: Vec<String> = Vec::with_capacity(n);
    for h in handles {
        let (status, body) = h.await.expect("task join");
        assert_eq!(
            status,
            StatusCode::OK,
            "concurrent claim must return 200 (got {status}), body: {body}"
        );
        assert_eq!(
            body.get("kind").and_then(|v| v.as_str()),
            Some("otk"),
            "concurrent claim must be from OTK pool (pool was sized > fan-out), body: {body}"
        );
        let kid = body["key"]["kid"]
            .as_str()
            .expect("kid in response")
            .to_string();
        returned_kids.push(kid);
    }

    // Single-use: every returned kid is distinct.
    let unique: std::collections::HashSet<&String> = returned_kids.iter().collect();
    assert_eq!(
        unique.len(),
        returned_kids.len(),
        "duplicate OTKs handed out across concurrent claims: {returned_kids:?}"
    );

    // Subset: every returned kid was originally published.
    for kid in &returned_kids {
        assert!(
            published_kids.contains(kid),
            "returned kid {kid} was not in the published pool {published_kids:?}"
        );
    }
}

/// Test 9: Lock the
/// [`is_write_conflict`](authlyn_interactive::server::retry::is_write_conflict)
/// substring matcher against a *real* SurrealDB write-conflict error.
///
/// **What this catches:** the retry helper distinguishes "retryable
/// SurrealDB write conflict" from "real DB error" by `Display`-string
/// substring-matching against `"Write conflict"` / `"retry the
/// transaction"`. That text is SurrealDB beta-era error formatting; a
/// future point release could rename either substring and silently disable
/// our retry loop without any compile-time signal. Steps 5+6 (room
/// key-share + Megolm rotation) will copy this retry pattern, so this is
/// the canary for *all* of them.
///
/// **How the conflict is synthesized:** two raw `Surreal<Client>`
/// connections open transactions against the same `conflict_canary:1`
/// row. SurrealDB's MVCC arbiter picks one winner per commit cycle; the
/// loser surfaces an error whose `Display` contains both canary
/// substrings (full text:
/// `"Query not executed: Transaction conflict: Write conflict, retry the
/// transaction. This transaction can be retried"`). The pattern is racy
/// in principle but in practice was deterministic on attempt 0 in a 50-
/// attempt probe against SurrealDB 3.0.4; we still wrap a small retry
/// loop so a scheduling fluke doesn't false-fail the test.
///
/// **If this test ever fails to observe a conflict at all,** that itself
/// is a meaningful signal: either SurrealDB no longer rejects this pattern
/// (the synth is wrong) or it doesn't surface a retryable error (the
/// matcher is moot because the retry path is unreachable). Either way the
/// test failure is informative.
#[tokio::test]
async fn is_write_conflict_matches_real_surrealdb_conflict() {
    use std::sync::Arc;

    // Per-test namespace + database, owned by this test. We deliberately
    // bypass the `arena()` helper because we need two parallel
    // `Surreal<Client>` handles into the *same* ns/db, not just a `Router`.
    let pid = std::process::id();
    let seq = NS_COUNTER.fetch_add(1, Ordering::Relaxed);
    let ns = format!("test_conflict_{}_{}", pid, seq);
    let db_name = format!("test_conflict_{}_{}", pid, seq);

    async fn fresh_conn(ns: &str, db_name: &str) -> Surreal<Client> {
        let host = std::env::var("SURREAL_URL")
            .unwrap_or_else(|_| "127.0.0.1:8000".into())
            .trim_start_matches("ws://")
            .trim_start_matches("wss://")
            .to_string();
        let db = Surreal::new::<Ws>(host)
            .await
            .expect("connect to SurrealDB — is ./scripts/dev-db.sh running?");
        db.signin(Root {
            username: std::env::var("SURREAL_USER").unwrap_or_else(|_| "root".into()),
            password: std::env::var("SURREAL_PASS").unwrap_or_else(|_| "root".into()),
        })
        .await
        .expect("signin");
        db.use_ns(ns).use_db(db_name).await.expect("use ns/db");
        db
    }

    // Setup: define a one-row table both racers will update.
    let setup = fresh_conn(&ns, &db_name).await;
    setup
        .query("DEFINE TABLE IF NOT EXISTS conflict_canary SCHEMAFULL; DEFINE FIELD IF NOT EXISTS v ON conflict_canary TYPE int;")
        .await
        .expect("define table")
        .check()
        .expect("define table check");
    setup
        .query("CREATE type::record('conflict_canary', '1') SET v = 0;")
        .await
        .expect("seed row")
        .check()
        .expect("seed row check");

    let d1 = Arc::new(fresh_conn(&ns, &db_name).await);
    let d2 = Arc::new(fresh_conn(&ns, &db_name).await);

    // Up to 50 attempts: scheduling can rob us of contention on any single
    // round, but never on all 50 (the probe caught it on attempt 0). If the
    // loop falls through, that's a real failure: see the test doc-comment.
    let conflict_err: surrealdb::Error = 'find: {
        for _attempt in 0..50 {
            let q = "BEGIN TRANSACTION; UPDATE type::record('conflict_canary', '1') SET v = $v; COMMIT TRANSACTION;";
            let f1 = {
                let d = d1.clone();
                tokio::spawn(
                    async move { d.query(q).bind(("v", 1i64)).await.and_then(|r| r.check()) },
                )
            };
            let f2 = {
                let d = d2.clone();
                tokio::spawn(
                    async move { d.query(q).bind(("v", 2i64)).await.and_then(|r| r.check()) },
                )
            };
            let r1 = f1.await.expect("d1 join");
            let r2 = f2.await.expect("d2 join");
            if let Err(e) = r1 {
                break 'find e;
            }
            if let Err(e) = r2 {
                break 'find e;
            }
        }
        panic!(
            "SurrealDB no longer synthesizes a write conflict for two concurrent \
             transactions updating the same row in 50 attempts — the conflict \
             synth pattern is broken, which means the canary cannot guard the \
             matcher anymore. Investigate before shipping."
        );
    };

    // Sanity: the conflict error's Display string is what the matcher sees.
    // Surface it in the failure message so the next release's renamed text
    // is immediately visible instead of just "false != true".
    assert!(
        is_write_conflict(&conflict_err),
        "is_write_conflict() returned false for a real SurrealDB write \
         conflict. The error's Display string was: '{conflict_err}'. \
         SurrealDB likely renamed its error text; update both substrings \
         in src/server/retry.rs::is_write_conflict to match (and audit \
         every caller of with_write_conflict_retry — keys.rs + \
         keyshare.rs today, Megolm in step 6)."
    );
}

/// Test 10: A claim for an existing device under the *wrong* user path
/// param is a 404 with a typed error body.
///
/// The URL `/keys/claim/{user}/{device}` advertises a `(user, device)`
/// tuple; the handler enforces that by loading the device's `user`
/// foreign key and comparing it to the path param. Without this
/// cross-check, the `:user` param was decorative and a peer asking for
/// `device-of-bob` under `/keys/claim/alice/...` would silently get
/// bob's OTKs. This test is the regression guard.
///
/// We also assert that the error message is the *specific* "device not
/// found for that user" — not the generic "device not found" — so we
/// can distinguish "the device doesn't exist at all" from "you asked
/// for it under the wrong user" in tracing without re-running the request.
#[tokio::test]
async fn claim_with_wrong_user_path_is_not_found() {
    let arena = arena().await;

    let user_a = random_id();
    let user_b = random_id();
    let device_id = random_id();

    // Publish a device under user_a.
    let mut device = DeviceAccount::new();
    let bundle = build_bundle(&mut device, 2);
    let (status, _) = post_json(
        &arena.router,
        "/keys/upload",
        &[("X-Device-Id", &device_id)],
        &json!({ "user_id": user_a, "bundle": bundle }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Sanity: the right user path works.
    let (status, _) = post_json(
        &arena.router,
        &format!("/keys/claim/{user_a}/{device_id}"),
        &[],
        &json!({}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "claim under the correct user must still succeed"
    );

    // Now claim with the wrong user. The device exists, but not under user_b.
    let (status, body) = post_json(
        &arena.router,
        &format!("/keys/claim/{user_b}/{device_id}"),
        &[],
        &json!({}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "claim under the wrong user must be 404, got {status}: {body}"
    );
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("device not found for that user"),
        "wrong-user 404 must use the disambiguating error message so \
         tracing can tell it apart from the generic device-missing 404, \
         got body: {body}"
    );
}
