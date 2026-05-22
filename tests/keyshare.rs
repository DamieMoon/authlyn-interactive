//! Integration tests for `POST /rooms/{id}/keyshare` and
//! `GET /rooms/{id}/keyshare/inbox` (routing-plan step 5).
//!
//! These hit a real SurrealDB. Run `./scripts/dev-db.sh` first.
//! Each test reserves a fresh namespace/database so concurrent runs don't
//! collide.
//!
//! Driving the axum `Router` through `tower::ServiceExt::oneshot` avoids
//! binding a TCP port for each test. Shared harness (`Arena`, `test_db`,
//! `post_json`, `get_json`, `random_id`) lives in `tests/common/` since
//! step 7. Crypto-touching helpers (`build_bundle`, `publish_device`,
//! `identity_curve_hex`, `create_room`, `count_keyshare_envelopes`) stay
//! inline because only this binary and `tests/keys.rs` need them.

#![cfg(feature = "ssr")]

use axum::http::StatusCode;
use axum::Router;
use serde_json::{json, Value};
use surrealdb::engine::remote::ws::Client;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

use authlyn_interactive::crypto::{
    prekey::PreKeyBundleBuilder, DeviceAccount, OlmEnvelope, OlmSession,
};
use authlyn_interactive::protocol::ClaimKeyResponse;

mod common;
use common::{arena, get_json, post_json, random_id};

// ---------------------------------------------------------------------------
// Crypto-touching helpers (kept inline)
// ---------------------------------------------------------------------------

fn build_bundle(device: &mut DeviceAccount, otk_count: usize) -> Value {
    let builder = PreKeyBundleBuilder::new();
    let bundle = builder.build(device, otk_count);
    serde_json::to_value(bundle).expect("bundle -> json")
}

/// Publish a device's pre-key bundle via `/keys/upload`. Returns the
/// `(user_id, device_id)` pair as the server will see them, so a later
/// claim or keyshare can address the same device.
async fn publish_device(
    router: &Router,
    account: &mut DeviceAccount,
    otk_count: usize,
) -> (String, String) {
    let user_id = random_id();
    let device_id = random_id();
    let bundle = build_bundle(account, otk_count);
    let (status, body) = post_json(
        router,
        "/keys/upload",
        &[("X-Device-Id", &device_id)],
        &json!({ "user_id": user_id, "bundle": bundle }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "publish_device failed: {body}");
    (user_id, device_id)
}

/// Insert a fixture room row owned by the named user. `room.created_by` is
/// `record<user> NOT NULL`, so the user row must already exist (e.g. from
/// a prior `/keys/upload` call).
async fn create_room(db: &Surreal<Client>, room_id: &str, owner_user_id: &str) {
    db.query(
        "CREATE type::record('room', $room_id) \
         SET name = 'test', created_by = type::record('user', $user_id);",
    )
    .bind(("room_id", room_id.to_string()))
    .bind(("user_id", owner_user_id.to_string()))
    .await
    .expect("create room")
    .check()
    .expect("create room check");
}

/// Identity Curve25519 hex (32 bytes → 64 hex chars). Recipients of a PreKey
/// envelope need this to bind the inbound session to the expected sender.
fn identity_curve_hex(account: &DeviceAccount) -> String {
    hex::encode(account.identity_keys().curve25519.as_bytes())
}

/// Count rows in `keyshare_envelope` across the entire test namespace. Used
/// by the negative-path tests to assert that a failed deposit did NOT
/// leave a dangling row behind — discriminates the "HTTP 4xx returned but
/// CREATE already fired" failure mode (the reason `persist_envelope` does
/// the FK pre-check in a *separate round-trip* before the CREATE).
async fn count_keyshare_envelopes(db: &Surreal<Client>) -> usize {
    #[derive(SurrealValue)]
    struct IdOnly {
        id_key: String,
    }
    let mut resp = db
        .query("SELECT meta::id(id) AS id_key FROM keyshare_envelope;")
        .await
        .expect("count query")
        .check()
        .expect("count query check");
    let rows: Vec<IdOnly> = resp.take(0).expect("take rows");
    rows.len()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Test 1: Alice deposits an Olm envelope for Bob via
/// `POST /rooms/test/keyshare`; Bob drains his inbox via
/// `GET /rooms/test/keyshare/inbox`; Bob decrypts the envelope using the
/// `inbound_from_prekey` path and the plaintext round-trips.
#[tokio::test]
async fn deposit_and_retrieve_round_trip() {
    let arena = arena().await;

    // Alice + Bob both publish bundles.
    let mut alice_account = DeviceAccount::new();
    let mut bob_account = DeviceAccount::new();
    let (alice_user_id, alice_device_id) =
        publish_device(&arena.router, &mut alice_account, 3).await;
    let (bob_user_id, bob_device_id) = publish_device(&arena.router, &mut bob_account, 3).await;

    // Fixture room owned by Alice.
    create_room(&arena.db, "test", &alice_user_id).await;

    // Alice claims one of Bob's OTKs and builds an outbound Olm session.
    let (status, claim_body) = post_json(
        &arena.router,
        &format!("/keys/claim/{bob_user_id}/{bob_device_id}"),
        &[],
        &json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "claim failed: {claim_body}");
    let claim: ClaimKeyResponse =
        serde_json::from_value(claim_body).expect("parse ClaimKeyResponse");
    let mut alice_session =
        OlmSession::outbound_from_claim(&alice_account, &claim).expect("outbound session");

    // Alice encrypts a fake Megolm session-key payload. The plaintext shape
    // doesn't matter for step 5 — the wire envelope is opaque ciphertext.
    let plaintext = b"fake-megolm-session-key-payload-bytes";
    let envelope = alice_session.encrypt(plaintext);
    let envelope_json = serde_json::to_value(&envelope).expect("envelope -> json");

    // Alice deposits the envelope for Bob.
    let (status, deposit_body) = post_json(
        &arena.router,
        "/rooms/test/keyshare",
        &[("X-Device-Id", &alice_device_id)],
        &json!({ "recipient_device": bob_device_id, "envelope": envelope_json }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "deposit failed: {deposit_body}"
    );
    assert!(
        deposit_body.get("id").and_then(|v| v.as_str()).is_some(),
        "deposit response must carry the new envelope's id, got: {deposit_body}"
    );

    // Bob drains his inbox.
    let (status, inbox_body) = get_json(
        &arena.router,
        "/rooms/test/keyshare/inbox",
        &[("X-Device-Id", &bob_device_id)],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "inbox GET failed: {inbox_body}");

    let envelopes = inbox_body
        .get("envelopes")
        .and_then(|v| v.as_array())
        .expect("envelopes array in response");
    assert_eq!(
        envelopes.len(),
        1,
        "expected exactly one envelope, got: {inbox_body}"
    );
    let record = &envelopes[0];
    assert_eq!(
        record.get("sender_device").and_then(|v| v.as_str()),
        Some(alice_device_id.as_str()),
        "sender_device must echo alice's device id"
    );

    let returned_envelope: OlmEnvelope = serde_json::from_value(
        record
            .get("envelope")
            .cloned()
            .expect("envelope sub-object"),
    )
    .expect("parse returned OlmEnvelope");

    // Bob decrypts via the PreKey path; plaintext must match.
    let alice_curve_hex = identity_curve_hex(&alice_account);
    let (_bob_session, recovered) =
        OlmSession::inbound_from_prekey(&mut bob_account, &alice_curve_hex, &returned_envelope)
            .expect("inbound session");
    assert_eq!(recovered.as_slice(), plaintext);
}

// ---------------------------------------------------------------------------
// Functional surface
// ---------------------------------------------------------------------------

/// Test 2: One sender, two recipients. Alice deposits one envelope for each;
/// each recipient's inbox returns exactly their own envelope.
#[tokio::test]
async fn multi_recipient_fanout() {
    let arena = arena().await;
    let mut alice_account = DeviceAccount::new();
    let mut bob_account = DeviceAccount::new();
    let mut charlie_account = DeviceAccount::new();
    let (alice_user_id, alice_device_id) =
        publish_device(&arena.router, &mut alice_account, 4).await;
    let (_, bob_device_id) = publish_device(&arena.router, &mut bob_account, 4).await;
    let (_, charlie_device_id) = publish_device(&arena.router, &mut charlie_account, 4).await;
    create_room(&arena.db, "test", &alice_user_id).await;

    // Synthetic envelopes — Test 1 already covers the full crypto path, so
    // here we just need distinguishable ciphertexts. The server only
    // base64-validates; the bytes don't have to be real Olm output.
    let env_for_bob = json!({ "message_type": 0, "ciphertext": "Ym9i" });
    let env_for_charlie = json!({ "message_type": 0, "ciphertext": "Y2hhcmxpZQ==" });

    let (status, _) = post_json(
        &arena.router,
        "/rooms/test/keyshare",
        &[("X-Device-Id", &alice_device_id)],
        &json!({ "recipient_device": bob_device_id, "envelope": env_for_bob }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _) = post_json(
        &arena.router,
        "/rooms/test/keyshare",
        &[("X-Device-Id", &alice_device_id)],
        &json!({ "recipient_device": charlie_device_id, "envelope": env_for_charlie }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Bob drains.
    let (status, bob_inbox) = get_json(
        &arena.router,
        "/rooms/test/keyshare/inbox",
        &[("X-Device-Id", &bob_device_id)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let bob_envs = bob_inbox["envelopes"].as_array().expect("envelopes array");
    assert_eq!(bob_envs.len(), 1, "Bob inbox: {bob_inbox}");
    assert_eq!(
        bob_envs[0]["envelope"]["ciphertext"].as_str(),
        Some("Ym9i"),
        "Bob got the wrong envelope: {bob_inbox}"
    );

    // Charlie drains.
    let (status, charlie_inbox) = get_json(
        &arena.router,
        "/rooms/test/keyshare/inbox",
        &[("X-Device-Id", &charlie_device_id)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let charlie_envs = charlie_inbox["envelopes"]
        .as_array()
        .expect("envelopes array");
    assert_eq!(charlie_envs.len(), 1, "Charlie inbox: {charlie_inbox}");
    assert_eq!(
        charlie_envs[0]["envelope"]["ciphertext"].as_str(),
        Some("Y2hhcmxpZQ=="),
        "Charlie got the wrong envelope: {charlie_inbox}"
    );
}

/// Test 3: After the first drain returns the envelope, the second GET on
/// the same (recipient, room) returns an empty `envelopes` array.
#[tokio::test]
async fn second_drain_returns_empty() {
    let arena = arena().await;
    let mut alice_account = DeviceAccount::new();
    let mut bob_account = DeviceAccount::new();
    let (alice_user_id, alice_device_id) =
        publish_device(&arena.router, &mut alice_account, 2).await;
    let (_, bob_device_id) = publish_device(&arena.router, &mut bob_account, 2).await;
    create_room(&arena.db, "test", &alice_user_id).await;

    let env = json!({ "message_type": 0, "ciphertext": "aGVsbG8=" });
    let (status, _) = post_json(
        &arena.router,
        "/rooms/test/keyshare",
        &[("X-Device-Id", &alice_device_id)],
        &json!({ "recipient_device": bob_device_id, "envelope": env }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, first) = get_json(
        &arena.router,
        "/rooms/test/keyshare/inbox",
        &[("X-Device-Id", &bob_device_id)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["envelopes"].as_array().expect("array").len(), 1);

    let (status, second) = get_json(
        &arena.router,
        "/rooms/test/keyshare/inbox",
        &[("X-Device-Id", &bob_device_id)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        second["envelopes"].as_array().expect("array").len(),
        0,
        "second drain must be empty: {second}"
    );
}

/// Test 4: An envelope deposited in room A is invisible to a GET on room B.
#[tokio::test]
async fn cross_room_isolation() {
    let arena = arena().await;
    let mut alice_account = DeviceAccount::new();
    let mut bob_account = DeviceAccount::new();
    let (alice_user_id, alice_device_id) =
        publish_device(&arena.router, &mut alice_account, 2).await;
    let (_, bob_device_id) = publish_device(&arena.router, &mut bob_account, 2).await;
    create_room(&arena.db, "room_a", &alice_user_id).await;
    create_room(&arena.db, "room_b", &alice_user_id).await;

    let env = json!({ "message_type": 0, "ciphertext": "YQ==" });
    let (status, _) = post_json(
        &arena.router,
        "/rooms/room_a/keyshare",
        &[("X-Device-Id", &alice_device_id)],
        &json!({ "recipient_device": bob_device_id, "envelope": env }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Bob's GET on room_b: empty.
    let (status, body) = get_json(
        &arena.router,
        "/rooms/room_b/keyshare/inbox",
        &[("X-Device-Id", &bob_device_id)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["envelopes"].as_array().expect("array").len(),
        0,
        "room_b inbox must not see room_a deposits: {body}"
    );

    // Sanity: room_a still has it.
    let (status, body) = get_json(
        &arena.router,
        "/rooms/room_a/keyshare/inbox",
        &[("X-Device-Id", &bob_device_id)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["envelopes"].as_array().expect("array").len(), 1);
}

// ---------------------------------------------------------------------------
// Auth / validation paths (typed error bodies)
// ---------------------------------------------------------------------------

/// Test 5: POST without `X-Device-Id` → 401 typed body.
#[tokio::test]
async fn deposit_missing_device_id_is_unauthorized() {
    let arena = arena().await;
    let mut alice_account = DeviceAccount::new();
    let mut bob_account = DeviceAccount::new();
    let (alice_user_id, _) = publish_device(&arena.router, &mut alice_account, 1).await;
    let (_, bob_device_id) = publish_device(&arena.router, &mut bob_account, 1).await;
    create_room(&arena.db, "test", &alice_user_id).await;

    let env = json!({ "message_type": 0, "ciphertext": "YQ==" });
    let (status, body) = post_json(
        &arena.router,
        "/rooms/test/keyshare",
        // No X-Device-Id.
        &[],
        &json!({ "recipient_device": bob_device_id, "envelope": env }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(
        body.get("error").and_then(|v| v.as_str()).is_some(),
        "{body}"
    );
}

/// Test 6: GET without `X-Device-Id` → 401 typed body.
#[tokio::test]
async fn drain_missing_device_id_is_unauthorized() {
    let arena = arena().await;
    let mut alice_account = DeviceAccount::new();
    let (alice_user_id, _) = publish_device(&arena.router, &mut alice_account, 1).await;
    create_room(&arena.db, "test", &alice_user_id).await;

    let (status, body) = get_json(&arena.router, "/rooms/test/keyshare/inbox", &[]).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(
        body.get("error").and_then(|v| v.as_str()).is_some(),
        "{body}"
    );
}

/// Test 7: Self-deposit (recipient_device == X-Device-Id) → 400 typed body.
#[tokio::test]
async fn self_deposit_is_rejected() {
    let arena = arena().await;
    let mut alice_account = DeviceAccount::new();
    let (alice_user_id, alice_device_id) =
        publish_device(&arena.router, &mut alice_account, 1).await;
    create_room(&arena.db, "test", &alice_user_id).await;

    let env = json!({ "message_type": 0, "ciphertext": "YQ==" });
    let (status, body) = post_json(
        &arena.router,
        "/rooms/test/keyshare",
        &[("X-Device-Id", &alice_device_id)],
        // recipient == sender.
        &json!({ "recipient_device": alice_device_id, "envelope": env }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("cannot deposit a keyshare to yourself"),
        "self-deposit error must be specific: {body}"
    );
}

/// Test 8: Deposit naming an unknown recipient device → 404 typed body
/// **and no dangling row**. The dangling-row discriminator was added after
/// the code-quality review caught that the original SQL ran the CREATE
/// unconditionally; without the assertion, the test passed even with the
/// bug.
#[tokio::test]
async fn deposit_to_unknown_recipient_is_not_found() {
    let arena = arena().await;
    let mut alice_account = DeviceAccount::new();
    let (alice_user_id, alice_device_id) =
        publish_device(&arena.router, &mut alice_account, 1).await;
    create_room(&arena.db, "test", &alice_user_id).await;

    let env = json!({ "message_type": 0, "ciphertext": "YQ==" });
    let (status, body) = post_json(
        &arena.router,
        "/rooms/test/keyshare",
        &[("X-Device-Id", &alice_device_id)],
        &json!({ "recipient_device": "nonexistent_device", "envelope": env }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("recipient device not found"),
        "{body}"
    );
    assert_eq!(
        count_keyshare_envelopes(&arena.db).await,
        0,
        "failed deposit must not leave a row behind"
    );
}

/// Test 9: Deposit targeting an unknown room → 404 typed body **and no
/// dangling row**.
#[tokio::test]
async fn deposit_to_unknown_room_is_not_found() {
    let arena = arena().await;
    let mut alice_account = DeviceAccount::new();
    let mut bob_account = DeviceAccount::new();
    let (_, alice_device_id) = publish_device(&arena.router, &mut alice_account, 1).await;
    let (_, bob_device_id) = publish_device(&arena.router, &mut bob_account, 1).await;
    // No `create_room` — the room doesn't exist.

    let env = json!({ "message_type": 0, "ciphertext": "YQ==" });
    let (status, body) = post_json(
        &arena.router,
        "/rooms/nonexistent_room/keyshare",
        &[("X-Device-Id", &alice_device_id)],
        &json!({ "recipient_device": bob_device_id, "envelope": env }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("room not found"),
        "{body}"
    );
    assert_eq!(
        count_keyshare_envelopes(&arena.db).await,
        0,
        "failed deposit must not leave a row behind"
    );
}

/// Test 10: GET on an unknown room → 404 typed body.
#[tokio::test]
async fn drain_unknown_room_is_not_found() {
    let arena = arena().await;
    let mut bob_account = DeviceAccount::new();
    let (_, bob_device_id) = publish_device(&arena.router, &mut bob_account, 1).await;

    let (status, body) = get_json(
        &arena.router,
        "/rooms/nonexistent_room/keyshare/inbox",
        &[("X-Device-Id", &bob_device_id)],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("room not found"),
        "{body}"
    );
}

/// Test 11: Envelope ciphertext that's not valid base64 → 400 typed body.
#[tokio::test]
async fn deposit_with_invalid_base64_ciphertext_is_rejected() {
    let arena = arena().await;
    let mut alice_account = DeviceAccount::new();
    let mut bob_account = DeviceAccount::new();
    let (alice_user_id, alice_device_id) =
        publish_device(&arena.router, &mut alice_account, 1).await;
    let (_, bob_device_id) = publish_device(&arena.router, &mut bob_account, 1).await;
    create_room(&arena.db, "test", &alice_user_id).await;

    let bad_env = json!({ "message_type": 0, "ciphertext": "!!! not base64 !!!" });
    let (status, body) = post_json(
        &arena.router,
        "/rooms/test/keyshare",
        &[("X-Device-Id", &alice_device_id)],
        &json!({ "recipient_device": bob_device_id, "envelope": bad_env }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body.get("error")
            .and_then(|v| v.as_str())
            .map(|s| s.contains("base64"))
            .unwrap_or(false),
        "expected base64-related error, got {body}"
    );
}

/// Test 12: Envelope `message_type` not in `{0, 1}` → 400 typed body.
#[tokio::test]
async fn deposit_with_invalid_message_type_is_rejected() {
    let arena = arena().await;
    let mut alice_account = DeviceAccount::new();
    let mut bob_account = DeviceAccount::new();
    let (alice_user_id, alice_device_id) =
        publish_device(&arena.router, &mut alice_account, 1).await;
    let (_, bob_device_id) = publish_device(&arena.router, &mut bob_account, 1).await;
    create_room(&arena.db, "test", &alice_user_id).await;

    let bad_env = json!({ "message_type": 2, "ciphertext": "YQ==" });
    let (status, body) = post_json(
        &arena.router,
        "/rooms/test/keyshare",
        &[("X-Device-Id", &alice_device_id)],
        &json!({ "recipient_device": bob_device_id, "envelope": bad_env }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body.get("error")
            .and_then(|v| v.as_str())
            .map(|s| s.contains("message_type"))
            .unwrap_or(false),
        "expected message_type error, got {body}"
    );
}

/// Test 15 (Delta-added): X-Device-Id pointing to a device row that
/// never published a bundle → 401 typed body. Without this pre-check
/// the CREATE would fall through and surface SurrealDB's FK error as 500.
#[tokio::test]
async fn deposit_from_unknown_sender_is_unauthorized() {
    let arena = arena().await;
    let mut alice_account = DeviceAccount::new();
    let mut bob_account = DeviceAccount::new();
    let (alice_user_id, _) = publish_device(&arena.router, &mut alice_account, 1).await;
    let (_, bob_device_id) = publish_device(&arena.router, &mut bob_account, 1).await;
    create_room(&arena.db, "test", &alice_user_id).await;

    let env = json!({ "message_type": 0, "ciphertext": "YQ==" });
    let (status, body) = post_json(
        &arena.router,
        "/rooms/test/keyshare",
        // Sender id that's never published — no device row exists for it.
        &[("X-Device-Id", &random_id())],
        &json!({ "recipient_device": bob_device_id, "envelope": env }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("unknown sender device"),
        "{body}"
    );
    // Discriminating assertion: without the two-round-trip pre-check in
    // `persist_envelope`, this CREATE would have fired anyway (sender_device
    // would have been a dangling pointer) and Bob's inbox would have a
    // junk row in it on his next legitimate drain.
    assert_eq!(
        count_keyshare_envelopes(&arena.db).await,
        0,
        "failed deposit must not leave a row behind (dangling-FK regression guard)"
    );
}

// ---------------------------------------------------------------------------
// Concurrency canaries
// ---------------------------------------------------------------------------

/// Test 13: Parallel drain by the same recipient must observe the
/// at-most-once delivery invariant. 20 concurrent GETs against a 50-envelope
/// inbox; the union of returned envelopes equals the 50 deposited, with no
/// envelope returned by two different GETs.
///
/// **Why `flavor = "multi_thread"` matters.** The default `#[tokio::test]`
/// uses `current_thread`, which serialises tasks onto a single OS thread.
/// In that mode the SDK's WS round-trips for each drain interleave but
/// can't truly run in parallel — and SurrealDB may finish each transaction
/// before the next one's SELECT even fires, hiding the DELETE-vs-DELETE
/// race entirely. Multi-threaded runtime + 20 racers reproduces real
/// concurrency.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parallel_drain_each_envelope_once() {
    let arena = arena().await;
    let mut alice_account = DeviceAccount::new();
    let mut bob_account = DeviceAccount::new();
    let (alice_user_id, alice_device_id) =
        publish_device(&arena.router, &mut alice_account, 1).await;
    let (_, bob_device_id) = publish_device(&arena.router, &mut bob_account, 1).await;
    create_room(&arena.db, "test", &alice_user_id).await;

    // Deposit 50 distinguishable envelopes for Bob.
    let n: usize = 50;
    let mut expected_ciphertexts: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    for i in 0..n {
        let ct = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            format!("envelope-{i:03}").as_bytes(),
        );
        let env = json!({ "message_type": 0, "ciphertext": ct });
        let (status, _) = post_json(
            &arena.router,
            "/rooms/test/keyshare",
            &[("X-Device-Id", &alice_device_id)],
            &json!({ "recipient_device": bob_device_id, "envelope": env }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        expected_ciphertexts.insert(ct);
    }

    // Fan out 20 concurrent GETs. Drain is BEGIN/COMMIT-wrapped, so
    // concurrent drains contend on the DELETE inside the transaction;
    // losers retry via `with_write_conflict_retry` and see a fresh
    // snapshot with the winning drain's rows gone.
    let parallel: usize = 20;
    let mut handles = Vec::with_capacity(parallel);
    for _ in 0..parallel {
        let router = arena.router.clone();
        let dev = bob_device_id.clone();
        handles.push(tokio::spawn(async move {
            get_json(
                &router,
                "/rooms/test/keyshare/inbox",
                &[("X-Device-Id", &dev)],
            )
            .await
        }));
    }

    let mut returned: Vec<String> = Vec::new();
    for h in handles {
        let (status, body) = h.await.expect("task join");
        assert_eq!(status, StatusCode::OK, "parallel drain failed: {body}");
        for env in body["envelopes"].as_array().expect("envelopes array") {
            let ct = env["envelope"]["ciphertext"]
                .as_str()
                .expect("ciphertext")
                .to_string();
            returned.push(ct);
        }
    }

    // Drain anything left after the parallel race (e.g. a write-conflict
    // retry that exhausted attempts; expected to be empty in practice).
    let (status, leftover) = get_json(
        &arena.router,
        "/rooms/test/keyshare/inbox",
        &[("X-Device-Id", &bob_device_id)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    for env in leftover["envelopes"].as_array().expect("envelopes array") {
        let ct = env["envelope"]["ciphertext"]
            .as_str()
            .expect("ciphertext")
            .to_string();
        returned.push(ct);
    }

    // At-most-once: no envelope returned twice.
    let unique: std::collections::HashSet<&String> = returned.iter().collect();
    assert_eq!(
        unique.len(),
        returned.len(),
        "duplicate envelopes across concurrent drains: {returned:?}"
    );
    // No loss: every deposited envelope returned exactly once.
    let returned_set: std::collections::HashSet<String> = returned.into_iter().collect();
    assert_eq!(
        returned_set, expected_ciphertexts,
        "missing or extra envelopes after parallel drain"
    );
}

/// Test 14: Concurrent POSTs and GETs must not lose or duplicate envelopes.
/// 20 distinct POSTs in parallel with 10 GETs on the multi-thread runtime
/// (same reasoning as test 13); final-drain GET to mop up.
/// Union of returned envelope ciphertexts equals the set of 20 POSTed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parallel_post_and_get_no_loss_no_duplicate() {
    let arena = arena().await;
    let mut alice_account = DeviceAccount::new();
    let mut bob_account = DeviceAccount::new();
    let (alice_user_id, alice_device_id) =
        publish_device(&arena.router, &mut alice_account, 1).await;
    let (_, bob_device_id) = publish_device(&arena.router, &mut bob_account, 1).await;
    create_room(&arena.db, "test", &alice_user_id).await;

    let n: usize = 20;
    let mut expected: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut post_handles = Vec::with_capacity(n);
    for i in 0..n {
        let ct = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            format!("env-{i:03}").as_bytes(),
        );
        expected.insert(ct.clone());
        let router = arena.router.clone();
        let sender = alice_device_id.clone();
        let recipient = bob_device_id.clone();
        let env = json!({ "message_type": 0, "ciphertext": ct });
        post_handles.push(tokio::spawn(async move {
            post_json(
                &router,
                "/rooms/test/keyshare",
                &[("X-Device-Id", &sender)],
                &json!({ "recipient_device": recipient, "envelope": env }),
            )
            .await
        }));
    }

    let mut get_handles = Vec::with_capacity(10);
    for _ in 0..10 {
        let router = arena.router.clone();
        let dev = bob_device_id.clone();
        get_handles.push(tokio::spawn(async move {
            get_json(
                &router,
                "/rooms/test/keyshare/inbox",
                &[("X-Device-Id", &dev)],
            )
            .await
        }));
    }

    // Settle POSTs first to confirm they all returned 201.
    for h in post_handles {
        let (status, body) = h.await.expect("post join");
        assert_eq!(status, StatusCode::CREATED, "parallel POST failed: {body}");
    }

    let mut returned: Vec<String> = Vec::new();
    for h in get_handles {
        let (status, body) = h.await.expect("get join");
        assert_eq!(status, StatusCode::OK, "parallel GET failed: {body}");
        for env in body["envelopes"].as_array().expect("envelopes array") {
            returned.push(
                env["envelope"]["ciphertext"]
                    .as_str()
                    .expect("ciphertext")
                    .to_string(),
            );
        }
    }

    // Final drain mop-up.
    let (status, leftover) = get_json(
        &arena.router,
        "/rooms/test/keyshare/inbox",
        &[("X-Device-Id", &bob_device_id)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    for env in leftover["envelopes"].as_array().expect("envelopes array") {
        returned.push(
            env["envelope"]["ciphertext"]
                .as_str()
                .expect("ciphertext")
                .to_string(),
        );
    }

    let unique: std::collections::HashSet<&String> = returned.iter().collect();
    assert_eq!(
        unique.len(),
        returned.len(),
        "duplicate envelope across concurrent POST+GET: {returned:?}"
    );
    let returned_set: std::collections::HashSet<String> = returned.into_iter().collect();
    assert_eq!(
        returned_set, expected,
        "envelope set mismatch after parallel POST+GET"
    );
}
