//! Integration tests for `POST /rooms/{id}/messages` and
//! `GET /rooms/{id}/messages` (routing-plan step 8).
//!
//! These hit a real SurrealDB. Run `./scripts/dev-db.sh` first. Each test
//! reserves a fresh namespace/database (via `tests/common::arena`) so
//! concurrent runs don't collide.
//!
//! ## Crypto-imports inversion vs step 7
//!
//! **This file deliberately inverts the "no `crypto::*` imports" rule that
//! `tests/rooms.rs` (step 7) called out.** Step 8 ships the cryptographic
//! invariant test the membership state machine deferred — the
//! forward-exclusion three-user rotation
//! (`forward_exclusion_three_user_rotation`) — which exercises Megolm
//! session rotation across a leave event using real
//! `crypto::megolm::{MegolmOutbound, MegolmInbound}` and the Olm key-share
//! envelope from step 4. The two-clients LIVE delivery test
//! (`two_clients_live_select_delivery`) similarly uses
//! `MegolmOutbound::encrypt` + `MegolmInbound::decrypt` to prove the wire
//! ciphertext round-trips. The other 15 tests are pure server-surface and
//! do not touch crypto.
//!
//! ## LIVE subscribe-before-trigger discipline
//!
//! SurrealDB 3.1.0-beta.3 LIVE queries do NOT replay historical rows —
//! every notification corresponds to a CREATE/UPDATE/DELETE that happened
//! *after* the subscription was registered. An ephemeral probe binary
//! (since deleted) confirmed the subscription is synchronous from the
//! caller's POV: by the time `db.query("LIVE SELECT ...").stream::<Notification<R>>(0)`
//! returns, the server has acknowledged the subscription and a subsequent
//! CREATE in the same connection produces a notification reliably (within
//! the 2s timeout this file uses). No `tokio::task::yield_now()` is
//! required between subscribe and trigger.
//!
//! ## DB-state assertions for test 17
//!
//! Test 17 (forward exclusion) explicitly reads M2's ciphertext via the
//! raw `arena.db` handle rather than `GET /rooms/:id/messages`. The
//! reason: a removed user calling the GET handler receives `404 "room not
//! found"` (the privacy-404 already covered by test 12) — so an
//! HTTP-only test would fail at the wrong layer (access-control 404),
//! masking what we actually want to prove (the CRYPTOGRAPHIC claim that
//! Megolm session rotation puts subsequent ciphertexts outside the
//! removed user's reach even with raw ciphertext access).

#![cfg(feature = "ssr")]

use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use futures::StreamExt;
use serde_json::{json, Value};
use surrealdb::engine::remote::ws::Client;
use surrealdb::types::{Action, SurrealValue};
use surrealdb::{Notification, Surreal};
use tokio::time::timeout;
use tower::ServiceExt;

use authlyn_interactive::crypto::{
    prekey::PreKeyBundleBuilder, DeviceAccount, MegolmCiphertext, MegolmInbound, MegolmOutbound,
    OlmEnvelope, OlmSession,
};
use authlyn_interactive::protocol::{ClaimKeyResponse, InboxEnvelope};

mod common;
use common::{arena, get_json, post_json, random_id};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Seed a fresh `(user, device)` pair via raw CREATE statements. Identity
/// keys are zero-bytes hex — most of step 8's tests don't decrypt anything
/// so the real curve/ed25519 material is irrelevant; tests 16/17 use
/// `publish_device` instead because they DO need real crypto.
async fn seed_user_and_device(db: &Surreal<Client>) -> (String, String) {
    let user_id = random_id();
    let device_id = random_id();
    db.query(
        "CREATE type::record('user', $user_id) SET display_name = '';
         CREATE type::record('device', $device_id)
             SET user = type::record('user', $user_id),
                 identity_curve25519 = $hex,
                 identity_ed25519    = $hex;",
    )
    .bind(("user_id", user_id.clone()))
    .bind(("device_id", device_id.clone()))
    .bind(("hex", "00".repeat(32)))
    .await
    .expect("seed user+device")
    .check()
    .expect("seed user+device check");
    (user_id, device_id)
}

/// Build a real pre-key bundle for `account` and publish it via
/// `/keys/upload`. Returns `(user_id, device_id)` exactly as the server
/// will see them. Used by tests 16 + 17 because those exercise the real
/// Olm + Megolm crypto, not just routing.
async fn publish_device(
    router: &axum::Router,
    account: &mut DeviceAccount,
    otk_count: usize,
) -> (String, String) {
    let user_id = random_id();
    let device_id = random_id();
    let bundle = PreKeyBundleBuilder::new().build(account, otk_count);
    let bundle_json = serde_json::to_value(&bundle).expect("bundle -> json");
    let (status, body) = post_json(
        router,
        "/keys/upload",
        &[("X-Device-Id", &device_id)],
        &json!({ "user_id": user_id, "bundle": bundle_json }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "publish_device failed: {body}");
    (user_id, device_id)
}

/// `Curve25519` identity hex of an account — needed when calling
/// `OlmSession::inbound_from_prekey` (which binds the inbound session to
/// the expected sender's identity key).
fn identity_curve_hex(account: &DeviceAccount) -> String {
    hex::encode(account.identity_keys().curve25519.as_bytes())
}

// ---------------------------------------------------------------------------
// DB-state helpers
// ---------------------------------------------------------------------------

#[derive(SurrealValue)]
struct CountRow {
    n: i64,
}

#[derive(SurrealValue)]
struct MessageStateRow {
    sender_device_key: String,
    megolm_session_id: String,
    message_index: i64,
    ciphertext: String,
    tier: String,
}

/// Number of `message` rows in this arena.
async fn count_messages(db: &Surreal<Client>) -> i64 {
    let mut resp = db
        .query("SELECT count() AS n FROM message GROUP ALL;")
        .await
        .expect("count_messages query")
        .check()
        .expect("count_messages check");
    let row: Option<CountRow> = resp.take(0).expect("count_messages take");
    row.map(|r| r.n).unwrap_or(0)
}

/// Sole message row by id — used by test 13's row-state assertion.
async fn get_message_state(db: &Surreal<Client>, message_id: &str) -> MessageStateRow {
    let mut resp = db
        .query(
            "SELECT
                meta::id(sender_device)  AS sender_device_key,
                megolm_session_id,
                message_index,
                ciphertext,
                tier
             FROM type::record('message', $message_id);",
        )
        .bind(("message_id", message_id.to_string()))
        .await
        .expect("get_message_state query")
        .check()
        .expect("get_message_state check");
    let row: Option<MessageStateRow> = resp.take(0).expect("get_message_state take");
    row.expect("message row must exist")
}

/// Helper: create a room owned by `device_id` via the real HTTP path.
/// Returns the new room's id.
async fn create_room_via_http(router: &axum::Router, device_id: &str, name: &str) -> String {
    let (status, body) = post_json(
        router,
        "/rooms",
        &[("X-Device-Id", device_id)],
        &json!({ "name": name }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create room failed: {body}");
    body["id"].as_str().expect("response.id").to_string()
}

/// Insert a `message` row directly via the DB (bypassing the HTTP path).
/// Used by test 15 to force two messages onto the same `sent_at`
/// timestamp — the composite cursor's tie-break branch is otherwise
/// unreachable because microsecond-resolution `time::now()` would assign
/// distinct timestamps to back-to-back POSTs.
#[allow(clippy::too_many_arguments)] // test seed fixture; one param per column is clearer than a struct
async fn seed_message_with_sent_at(
    db: &Surreal<Client>,
    message_id: &str,
    room_id: &str,
    device_id: &str,
    megolm_session_id: &str,
    message_index: i64,
    ciphertext: &str,
    sent_at_rfc3339: &str,
) {
    db.query(
        "CREATE type::record('message', $message_id) SET
            room              = type::record('room', $room_id),
            sender_device     = type::record('device', $device_id),
            megolm_session_id = $session_id,
            message_index     = $message_index,
            ciphertext        = $ciphertext,
            sent_at           = type::datetime($sent_at);",
    )
    .bind(("message_id", message_id.to_string()))
    .bind(("room_id", room_id.to_string()))
    .bind(("device_id", device_id.to_string()))
    .bind(("session_id", megolm_session_id.to_string()))
    .bind(("message_index", message_index))
    .bind(("ciphertext", ciphertext.to_string()))
    .bind(("sent_at", sent_at_rfc3339.to_string()))
    .await
    .expect("seed message")
    .check()
    .expect("seed message check");
}

// ---------------------------------------------------------------------------
// Validation / auth (tests 1-12)
// ---------------------------------------------------------------------------

/// Test 1: POST without `X-Device-Id` → 401 typed body.
#[tokio::test]
async fn post_missing_device_id_returns_401() {
    let arena = arena().await;
    let (status, body) = post_json(
        &arena.router,
        "/rooms/anything/messages",
        &[],
        &json!({ "megolm_session_id": "s", "message_index": 0, "ciphertext": "YQ==" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("missing X-Device-Id header"),
        "{body}"
    );
}

/// Test 2: POST with a well-formed but unknown `X-Device-Id` → 401
/// `"unknown caller device"`. Distinct from test 1 (`extract_device_id`
/// returns `None` for missing header); this exercises the
/// `load_caller_user` `Ok(None)` branch. Discriminating assertion: no
/// `message` row created.
#[tokio::test]
async fn post_unknown_caller_device_returns_401() {
    let arena = arena().await;
    let bogus_device = random_id();
    let (status, body) = post_json(
        &arena.router,
        "/rooms/anything/messages",
        &[("X-Device-Id", &bogus_device)],
        &json!({ "megolm_session_id": "s", "message_index": 0, "ciphertext": "YQ==" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("unknown caller device"),
        "{body}"
    );
    assert_eq!(
        count_messages(&arena.db).await,
        0,
        "unknown-caller 401 must NOT create a message row"
    );
}

/// Test 3: malformed JSON body → 400 typed body `"malformed JSON"`.
#[tokio::test]
async fn post_malformed_json_returns_typed_400() {
    let arena = arena().await;
    let (_user, device) = seed_user_and_device(&arena.db).await;

    let req = Request::builder()
        .method(Method::POST)
        .uri("/rooms/anything/messages")
        .header(header::CONTENT_TYPE, "application/json")
        .header("X-Device-Id", &device)
        .body(Body::from("not json at all"))
        .unwrap();
    let res = arena.router.clone().oneshot(req).await.expect("oneshot");
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let bytes = to_bytes(res.into_body(), 1 << 20).await.expect("read body");
    let body: Value = serde_json::from_slice(&bytes).expect("typed body");
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("malformed JSON"),
        "{body}"
    );
}

/// Test 4: empty `megolm_session_id` (after trim) → 400. Both `""` and
/// `"   "` must be rejected with the same body.
#[tokio::test]
async fn post_empty_megolm_session_id_returns_400() {
    let arena = arena().await;
    let (_user, device) = seed_user_and_device(&arena.db).await;
    let room_id = create_room_via_http(&arena.router, &device, "r").await;

    for empty in ["", "   ", "\t\n"] {
        let (status, body) = post_json(
            &arena.router,
            &format!("/rooms/{room_id}/messages"),
            &[("X-Device-Id", &device)],
            &json!({
                "megolm_session_id": empty,
                "message_index": 0,
                "ciphertext": "YQ=="
            }),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "session_id {empty:?} should be 400: {body}"
        );
        assert_eq!(
            body.get("error").and_then(|v| v.as_str()),
            Some("megolm_session_id must not be empty"),
            "session_id {empty:?} body: {body}"
        );
    }
    assert_eq!(
        count_messages(&arena.db).await,
        0,
        "empty session_id must not create a message row"
    );
}

/// Test 5: empty `ciphertext` (after trim) → 400.
#[tokio::test]
async fn post_empty_ciphertext_returns_400() {
    let arena = arena().await;
    let (_user, device) = seed_user_and_device(&arena.db).await;
    let room_id = create_room_via_http(&arena.router, &device, "r").await;

    for empty in ["", "   ", "\t"] {
        let (status, body) = post_json(
            &arena.router,
            &format!("/rooms/{room_id}/messages"),
            &[("X-Device-Id", &device)],
            &json!({
                "megolm_session_id": "s",
                "message_index": 0,
                "ciphertext": empty
            }),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "ciphertext {empty:?} should be 400: {body}"
        );
        assert_eq!(
            body.get("error").and_then(|v| v.as_str()),
            Some("ciphertext must not be empty"),
            "ciphertext {empty:?} body: {body}"
        );
    }
    assert_eq!(
        count_messages(&arena.db).await,
        0,
        "empty ciphertext must not create a message row"
    );
}

/// Test 6: non-base64 `ciphertext` → 400. The body is non-empty so the
/// emptiness gate falls through; the base64 well-formedness gate fires.
#[tokio::test]
async fn post_non_base64_ciphertext_returns_400() {
    let arena = arena().await;
    let (_user, device) = seed_user_and_device(&arena.db).await;
    let room_id = create_room_via_http(&arena.router, &device, "r").await;

    let (status, body) = post_json(
        &arena.router,
        &format!("/rooms/{room_id}/messages"),
        &[("X-Device-Id", &device)],
        // Garbage that survives serde but isn't valid base64.
        &json!({
            "megolm_session_id": "s",
            "message_index": 0,
            "ciphertext": "!!!not base 64!!!"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("ciphertext must be base64"),
        "{body}"
    );
    assert_eq!(
        count_messages(&arena.db).await,
        0,
        "non-base64 ciphertext must not create a message row"
    );
}

/// Test 7: POST against an unknown room id → 404 "room not found"
/// (privacy). Asserts no message row created.
#[tokio::test]
async fn post_unknown_room_returns_404() {
    let arena = arena().await;
    let (_user, device) = seed_user_and_device(&arena.db).await;

    let (status, body) = post_json(
        &arena.router,
        "/rooms/nonexistent/messages",
        &[("X-Device-Id", &device)],
        &json!({
            "megolm_session_id": "s",
            "message_index": 0,
            "ciphertext": "YQ=="
        }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("room not found"),
        "{body}"
    );
    assert_eq!(
        count_messages(&arena.db).await,
        0,
        "unknown-room 404 must NOT create a message row"
    );
}

/// Test 8: privacy rule — non-member caller (caller exists, room
/// exists, caller is not a member of THAT room) → 404 "room not
/// found", NOT 403. The DB-state assertion is the discriminator: a
/// leaky implementation that returns 404 but writes the row would pass
/// response-only assertions.
#[tokio::test]
async fn post_non_member_caller_returns_404() {
    let arena = arena().await;
    let (_alice_user, alice_device) = seed_user_and_device(&arena.db).await;
    let (_charlie_user, charlie_device) = seed_user_and_device(&arena.db).await;

    let room_id = create_room_via_http(&arena.router, &alice_device, "r").await;

    let (status, body) = post_json(
        &arena.router,
        &format!("/rooms/{room_id}/messages"),
        &[("X-Device-Id", &charlie_device)],
        &json!({
            "megolm_session_id": "s",
            "message_index": 0,
            "ciphertext": "YQ=="
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "non-member POST must be 404 (privacy), not 403: {body}"
    );
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("room not found"),
        "non-member POST body must match the same room-not-found body: {body}"
    );
    assert_eq!(
        count_messages(&arena.db).await,
        0,
        "non-member POST must NOT create a message row"
    );
}

/// Test 9: GET without `X-Device-Id` → 401 typed body.
#[tokio::test]
async fn get_missing_device_id_returns_401() {
    let arena = arena().await;
    let (status, body) = get_json(&arena.router, "/rooms/anything/messages", &[]).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("missing X-Device-Id header"),
        "{body}"
    );
}

/// Test 10: malformed `?since=` (not RFC3339) → 400. The shape probe
/// rejects "not-a-date" before any DB round-trip.
#[tokio::test]
async fn get_malformed_since_returns_400() {
    let arena = arena().await;
    let (_user, device) = seed_user_and_device(&arena.db).await;
    let room_id = create_room_via_http(&arena.router, &device, "r").await;

    let (status, body) = get_json(
        &arena.router,
        &format!("/rooms/{room_id}/messages?since=not-a-date&after_id=x"),
        &[("X-Device-Id", &device)],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("since must be RFC3339 datetime"),
        "{body}"
    );
}

/// Test 11: partial cursor (only `?since=`, only `?after_id=`) → 400.
/// Either alone is meaningless because the cursor's tie-break branch
/// needs both halves to be discriminating.
#[tokio::test]
async fn get_partial_cursor_returns_400() {
    let arena = arena().await;
    let (_user, device) = seed_user_and_device(&arena.db).await;
    let room_id = create_room_via_http(&arena.router, &device, "r").await;

    // `?since=` alone (well-formed RFC3339).
    let (status, body) = get_json(
        &arena.router,
        &format!("/rooms/{room_id}/messages?since=2026-05-22T12:00:00Z"),
        &[("X-Device-Id", &device)],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "since alone: {body}");
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("since and after_id must be provided together"),
        "since alone: {body}"
    );

    // `?after_id=` alone.
    let (status, body) = get_json(
        &arena.router,
        &format!("/rooms/{room_id}/messages?after_id=abc"),
        &[("X-Device-Id", &device)],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "after_id alone: {body}");
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("since and after_id must be provided together"),
        "after_id alone: {body}"
    );
}

/// Test 11b: empty `?after_id=` (after trim) → 400 "after_id must not be
/// empty". Reaches the `(Some, Some)` arm of `parse_cursor` with a valid
/// `since`, then trips the explicit empty-after-trim guard. Without that
/// guard the cursor reaches SurrealDB with `meta::id(id) > ""` which is
/// degenerate (matches every id at the boundary). Mirrors the
/// `megolm_session_id must not be empty` / `ciphertext must not be empty`
/// trim-and-reject family in `post_message`.
///
/// Cases:
/// - `after_id=` (literal empty) — `parse_cursor` sees `Some("")`.
/// - `after_id=%20%20%20` (whitespace) — trims to `""` and trips the guard.
#[tokio::test]
async fn get_empty_after_id_returns_400() {
    let arena = arena().await;
    let (_user, device) = seed_user_and_device(&arena.db).await;
    let room_id = create_room_via_http(&arena.router, &device, "r").await;

    // (label, percent-encoded form passed in the URL)
    for (label, encoded) in [("empty", ""), ("spaces", "%20%20%20"), ("tab", "%09")] {
        let (status, body) = get_json(
            &arena.router,
            &format!("/rooms/{room_id}/messages?since=2026-05-22T12:00:00Z&after_id={encoded}"),
            &[("X-Device-Id", &device)],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "after_id {label}: {body}");
        assert_eq!(
            body.get("error").and_then(|v| v.as_str()),
            Some("after_id must not be empty"),
            "after_id {label}: {body}"
        );
    }
}

/// Test 12: GET privacy — non-member caller → 404 "room not found".
/// Mirrors test 8's POST shape so the privacy ordering is consistent
/// across verbs. (This is also the orthogonal HTTP claim test 17
/// references — test 17's cryptographic assertion bypasses HTTP via
/// `arena.db` precisely so this 404 doesn't mask the decrypt-failure.)
#[tokio::test]
async fn get_non_member_caller_returns_404() {
    let arena = arena().await;
    let (_alice_user, alice_device) = seed_user_and_device(&arena.db).await;
    let (_charlie_user, charlie_device) = seed_user_and_device(&arena.db).await;
    let room_id = create_room_via_http(&arena.router, &alice_device, "r").await;

    let (status, body) = get_json(
        &arena.router,
        &format!("/rooms/{room_id}/messages"),
        &[("X-Device-Id", &charlie_device)],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("room not found"),
        "{body}"
    );
}

// ---------------------------------------------------------------------------
// Happy paths (tests 13-15)
// ---------------------------------------------------------------------------

/// Test 13: POST → 201 with the new id; the underlying row has all
/// caller-controlled fields exactly as posted AND the server-set
/// `tier = 'default'`. This is the schema-default canary: a future
/// migration that drops the `DEFAULT 'default'` clause on `message.tier`
/// would silently flip the field to an empty string.
#[tokio::test]
async fn post_returns_201_with_row_state() {
    let arena = arena().await;
    let (_user, device) = seed_user_and_device(&arena.db).await;
    let room_id = create_room_via_http(&arena.router, &device, "r").await;

    let (status, body) = post_json(
        &arena.router,
        &format!("/rooms/{room_id}/messages"),
        &[("X-Device-Id", &device)],
        &json!({
            "megolm_session_id": "session-xyz",
            "message_index": 7,
            "ciphertext": "Y2lwaA=="
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let message_id = body
        .get("id")
        .and_then(|v| v.as_str())
        .expect("response.id")
        .to_string();

    let row = get_message_state(&arena.db, &message_id).await;
    assert_eq!(row.sender_device_key, device);
    assert_eq!(row.megolm_session_id, "session-xyz");
    assert_eq!(row.message_index, 7);
    assert_eq!(row.ciphertext, "Y2lwaA==");
    assert_eq!(row.tier, "default", "tier must default to 'default' for v1");
}

/// Test 14: insert five messages via POST; GET without cursor returns
/// them in ASC `(sent_at, id)` order. The wire `message_index` is also
/// preserved 0..=4 since the server doesn't reassign it.
#[tokio::test]
async fn get_no_cursor_returns_latest_messages_asc() {
    let arena = arena().await;
    let (_user, device) = seed_user_and_device(&arena.db).await;
    let room_id = create_room_via_http(&arena.router, &device, "r").await;

    for i in 0..5u32 {
        let (status, _) = post_json(
            &arena.router,
            &format!("/rooms/{room_id}/messages"),
            &[("X-Device-Id", &device)],
            &json!({
                "megolm_session_id": "s",
                "message_index": i,
                "ciphertext": "YQ=="
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }

    let (status, body) = get_json(
        &arena.router,
        &format!("/rooms/{room_id}/messages"),
        &[("X-Device-Id", &device)],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let messages = body
        .get("messages")
        .and_then(|v| v.as_array())
        .expect("messages array");
    assert_eq!(messages.len(), 5, "expected 5 envelopes: {body}");

    // ASC by sent_at: lex compare matches chronological compare because
    // the server emits sent_at as a fixed 9-digit RFC 3339 string (see
    // `server::datetime::to_rfc3339_fixed`). `time::now()` resolution is
    // also high enough that back-to-back POSTs nearly always have
    // distinct sent_at values, so the `<=` rather than `<` is just for
    // the (theoretical) clock-tie case.
    let mut prev: Option<&str> = None;
    for (idx, m) in messages.iter().enumerate() {
        let sent_at = m.get("sent_at").and_then(|v| v.as_str()).expect("sent_at");
        if let Some(p) = prev {
            assert!(
                p <= sent_at,
                "envelopes not in ASC sent_at order: prev={p} curr={sent_at}"
            );
        }
        prev = Some(sent_at);
        assert_eq!(
            m.get("message_index").and_then(|v| v.as_u64()),
            Some(idx as u64),
            "message_index must round-trip"
        );
        assert_eq!(
            m.get("tier").and_then(|v| v.as_str()),
            Some("default"),
            "tier must round-trip as 'default'"
        );
    }
}

/// Test 15: composite cursor with a tie-break.
///
/// **Discriminating-ness.** This test crafts two messages sharing the
/// EXACT same `sent_at` timestamp by inserting them directly via
/// `arena.db` with `type::datetime(...)` — `time::now()` would never
/// produce a real collision at microsecond resolution, so the only way
/// to exercise the `OR (sent_at = $since AND meta::id(id) > $after_id)`
/// branch is to construct the collision manually. Without that OR
/// branch the test MUST fail: a strict `sent_at > $since` filter omits
/// the row at the exact boundary.
///
/// The test seeds three messages:
///   - `m_alpha` and `m_beta` share `sent_at = 12:00:00Z`. Ids force
///     `m_alpha` < `m_beta` lexicographically (`a*` < `b*`).
///   - `m_gamma` has `sent_at = 13:00:00Z`.
///
/// Resuming from `(since=12:00:00Z, after_id=m_alpha-...)` MUST return
/// exactly `[m_beta, m_gamma]` — `m_beta` because the tie-break OR
/// branch picks it up, `m_gamma` because the strict-greater branch
/// picks it up.
#[tokio::test]
async fn get_composite_cursor_resumes_at_boundary() {
    let arena = arena().await;
    let (_user, device) = seed_user_and_device(&arena.db).await;
    let room_id = create_room_via_http(&arena.router, &device, "r").await;

    // Deterministic ids: prefix with 'a-' / 'b-' / 'c-' so alphabetical
    // order is `m_alpha` < `m_beta` < `m_gamma`. The random suffix keeps
    // namespaces isolated even if multiple workers happen to share a
    // SurrealDB instance.
    let m_alpha = format!("a-{}", random_id());
    let m_beta = format!("b-{}", random_id());
    let m_gamma = format!("c-{}", random_id());
    let boundary = "2026-05-22T12:00:00Z";

    seed_message_with_sent_at(
        &arena.db, &m_alpha, &room_id, &device, "s", 0, "YQ==", boundary,
    )
    .await;
    seed_message_with_sent_at(
        &arena.db, &m_beta, &room_id, &device, "s", 1, "Yg==", boundary,
    )
    .await;
    seed_message_with_sent_at(
        &arena.db,
        &m_gamma,
        &room_id,
        &device,
        "s",
        2,
        "Yw==",
        "2026-05-22T13:00:00Z",
    )
    .await;

    // No-cursor sanity: returns all three in (sent_at, id) ASC.
    let (status, body) = get_json(
        &arena.router,
        &format!("/rooms/{room_id}/messages"),
        &[("X-Device-Id", &device)],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let messages = body
        .get("messages")
        .and_then(|v| v.as_array())
        .expect("messages array");
    let ids: Vec<&str> = messages
        .iter()
        .map(|m| m.get("id").and_then(|v| v.as_str()).expect("id"))
        .collect();
    assert_eq!(
        ids,
        vec![m_alpha.as_str(), m_beta.as_str(), m_gamma.as_str()]
    );

    // Resume from after `m_alpha` at the boundary timestamp. The tie-break
    // branch must surface `m_beta`; the strict-greater branch must surface
    // `m_gamma`.
    let url = format!(
        "/rooms/{room_id}/messages?since={boundary}&after_id={alpha}",
        alpha = m_alpha
    );
    let (status, body) = get_json(&arena.router, &url, &[("X-Device-Id", &device)]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let messages = body
        .get("messages")
        .and_then(|v| v.as_array())
        .expect("messages array");
    let ids: Vec<&str> = messages
        .iter()
        .map(|m| m.get("id").and_then(|v| v.as_str()).expect("id"))
        .collect();
    assert_eq!(
        ids,
        vec![m_beta.as_str(), m_gamma.as_str()],
        "composite cursor must tie-break on id at equal sent_at"
    );

    // Sanity: cursor that includes m_beta as the after_id must yield
    // ONLY m_gamma — the OR branch must NOT re-deliver m_beta to its own
    // cursor.
    let url = format!(
        "/rooms/{room_id}/messages?since={boundary}&after_id={beta}",
        beta = m_beta
    );
    let (status, body) = get_json(&arena.router, &url, &[("X-Device-Id", &device)]).await;
    assert_eq!(status, StatusCode::OK);
    let messages = body
        .get("messages")
        .and_then(|v| v.as_array())
        .expect("messages array");
    let ids: Vec<&str> = messages
        .iter()
        .map(|m| m.get("id").and_then(|v| v.as_str()).expect("id"))
        .collect();
    assert_eq!(ids, vec![m_gamma.as_str()]);
}

/// Test 15b: the `LIMIT 100` cap.
///
/// **Discriminating-ness.** This test must fail under either of two
/// regressions: (a) the `LIMIT $page_limit` clause being dropped, OR
/// (b) `MESSAGES_PAGE_LIMIT` being changed to anything other than 100.
/// Seeding exactly 101 rows discriminates against (a) (would return 101)
/// and against `LIMIT = 1000` (would also return 101); fewer than 101
/// would not catch (a), and seeding 1000+ would let the `LIMIT = 1000`
/// mutation slip through. The seeded `arena.db` insert path is
/// load-bearing because `time::now()` at microsecond resolution would
/// not guarantee distinct timestamps across 101 back-to-back POSTs.
///
/// The ASC `(sent_at, id)` ordering is asserted exactly: the first 100
/// rows are returned in the seeded order, with id prefixes `m-000..=m-099`.
///
/// **Timestamp scheme.** Whole-second spacing (`12:00:00Z` ... `12:01:40Z`).
/// The server now ORDERs by raw `datetime` semantics so any seeding
/// scheme would suffice for ordering correctness; whole-second values
/// remain easy to read in failure messages.
#[tokio::test]
async fn get_limit_caps_response_at_100_rows() {
    let arena = arena().await;
    let (_user, device) = seed_user_and_device(&arena.db).await;
    let room_id = create_room_via_http(&arena.router, &device, "r").await;

    // Seed 101 messages with monotonically-increasing sent_at and
    // deterministically-orderable ids (`m-000-...` < `m-001-...` < ...).
    let suffix = random_id();
    let mut seeded_ids: Vec<String> = Vec::with_capacity(101);
    for i in 0..101u32 {
        let id = format!("m-{i:03}-{suffix}");
        // Whole-second spacing: 12:00:00Z, 12:00:01Z, ..., 12:01:40Z.
        let minute = i / 60;
        let second = i % 60;
        let sent_at = format!("2026-05-22T12:{minute:02}:{second:02}Z");
        seed_message_with_sent_at(
            &arena.db,
            &id,
            &room_id,
            &device,
            "s",
            i64::from(i),
            "YQ==",
            &sent_at,
        )
        .await;
        seeded_ids.push(id);
    }

    let (status, body) = get_json(
        &arena.router,
        &format!("/rooms/{room_id}/messages"),
        &[("X-Device-Id", &device)],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let messages = body
        .get("messages")
        .and_then(|v| v.as_array())
        .expect("messages array");
    assert_eq!(
        messages.len(),
        100,
        "GET without cursor must cap at exactly 100 envelopes (seeded 101)"
    );
    let returned_ids: Vec<&str> = messages
        .iter()
        .map(|m| m.get("id").and_then(|v| v.as_str()).expect("id"))
        .collect();
    let expected_ids: Vec<&str> = seeded_ids.iter().take(100).map(String::as_str).collect();
    assert_eq!(
        returned_ids, expected_ids,
        "first 100 envelopes must be returned in ASC (sent_at, id) order"
    );
}

/// Test 15c: cursor positioned past the last row → 200 with an empty
/// `messages` array (not an error, not omitted from the response). Seeds
/// a handful of messages, then asks for messages strictly after a future
/// timestamp; the body must contain a present-but-empty `messages` array.
#[tokio::test]
async fn get_cursor_past_last_row_returns_empty_messages() {
    let arena = arena().await;
    let (_user, device) = seed_user_and_device(&arena.db).await;
    let room_id = create_room_via_http(&arena.router, &device, "r").await;

    // Seed three messages well in the past relative to the cursor.
    for i in 0..3u32 {
        let id = format!("seed-{i}-{}", random_id());
        let sent_at = format!("2026-05-22T12:00:0{i}Z");
        seed_message_with_sent_at(
            &arena.db,
            &id,
            &room_id,
            &device,
            "s",
            i64::from(i),
            "YQ==",
            &sent_at,
        )
        .await;
    }

    // Cursor in 2030 with a maximal-looking after_id. The `since` shape is
    // RFC3339; `after_id` is non-empty so it survives the post-step-8b
    // trim-and-reject in `parse_cursor`.
    let (status, body) = get_json(
        &arena.router,
        &format!(
            "/rooms/{room_id}/messages\
             ?since=2030-01-01T00:00:00Z\
             &after_id=ffffffffffffffffffffffffffffffff"
        ),
        &[("X-Device-Id", &device)],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let messages_value = body
        .get("messages")
        .expect("messages key must be present (not omitted)");
    let messages = messages_value
        .as_array()
        .expect("messages must be a JSON array (not null, not an object)");
    assert_eq!(
        messages.len(),
        0,
        "cursor past last row must return an empty messages array"
    );
}

/// Test 15d: GET orders messages chronologically across timestamps that
/// straddle chrono's `SecondsFormat::AutoSi` format-class boundaries.
///
/// **Why this exists.** SurrealDB's `Datetime::Display` (the implementation
/// the `<string>sent_at` cast invokes per-row) uses
/// `to_rfc3339_opts(SecondsFormat::AutoSi, true)`. `AutoSi` emits
/// VARIABLE-LENGTH sub-second suffixes per row: `Z` (zero nanos), `.NNNZ`
/// (millis-aligned), `.NNNNNNZ` (micros-aligned), `.NNNNNNNNNZ` (otherwise).
/// ASCII ordering: `.` (46) < digit (48-57) < `Z` (90). So chronologically
/// `12:00:00Z < 12:00:00.123Z`, but lex `"12:00:00.123Z" < "12:00:00Z"`.
/// Ordering on the projected string flips at format-class boundaries.
///
/// **Discriminating-ness.** Each seeded row's `message_index` is its
/// chronological rank. If `ORDER BY <projected-string>` is in effect, the
/// returned `message_index` sequence is permuted (not strictly increasing).
/// The assertion is `returned_indices == [0, 1, 2, 3, 4]` — fails iff the
/// server emitted rows in the lexicographic-mis-ordered sequence.
///
/// Sabotage-verification: re-introducing `<string>sent_at AS sent_at`
/// (along with corresponding `MessageRow.sent_at: String` and removing the
/// envelope conversion) MUST flip this assertion to fail.
///
/// The verification is on `message_index`, not on the `sent_at` string —
/// the wire `sent_at` shape itself is allowed to evolve (e.g. fixed
/// 9-digit) without re-flipping the test.
#[tokio::test]
async fn get_orders_format_class_spanning_timestamps_chronologically() {
    let arena = arena().await;
    let (_user, device) = seed_user_and_device(&arena.db).await;
    let room_id = create_room_via_http(&arena.router, &device, "r").await;

    // Five chronologically-increasing timestamps, one per AutoSi format
    // class. Order index 0..4 == chronological order. ALL five have the
    // same wall-clock second so the format-class suffix is the ONLY
    // discriminator — without that constraint a "seconds" boundary would
    // dominate and mask the AutoSi bug.
    let seeds = [
        (0u32, "2026-05-22T12:00:00Z"),           // zero nanos
        (1u32, "2026-05-22T12:00:00.123Z"),       // millis
        (2u32, "2026-05-22T12:00:00.123456Z"),    // micros
        (3u32, "2026-05-22T12:00:00.123456789Z"), // 9-digit nanos
        // A partial-9-digit value that AutoSi normalises into the
        // 9-digit class (trailing zeros forced because the next non-zero
        // sub-second sits past the SI break). Chronologically after #3.
        (4u32, "2026-05-22T12:00:00.999999999Z"),
    ];
    let suffix = random_id();
    for (idx, sent_at) in seeds.iter() {
        // Ids sorted lexically by chronological rank so that any ORDER
        // BY id_key tie-break wouldn't accidentally rescue the test.
        let id = format!("m-{idx}-{suffix}");
        seed_message_with_sent_at(
            &arena.db,
            &id,
            &room_id,
            &device,
            "s",
            i64::from(*idx),
            "YQ==",
            sent_at,
        )
        .await;
    }

    let (status, body) = get_json(
        &arena.router,
        &format!("/rooms/{room_id}/messages"),
        &[("X-Device-Id", &device)],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let messages = body
        .get("messages")
        .and_then(|v| v.as_array())
        .expect("messages array");
    assert_eq!(messages.len(), 5, "expected 5 envelopes: {body}");

    // The chronological rank (== seeded message_index) is the
    // bug-discriminator. Lex-on-AutoSi-projection permutes this.
    let returned_indices: Vec<u64> = messages
        .iter()
        .map(|m| {
            m.get("message_index")
                .and_then(|v| v.as_u64())
                .expect("message_index")
        })
        .collect();
    assert_eq!(
        returned_indices,
        vec![0, 1, 2, 3, 4],
        "GET must order by datetime semantics, not lex-on-string-projection: {body}"
    );
}

// ---------------------------------------------------------------------------
// End-to-end (tests 16-17)
// ---------------------------------------------------------------------------

/// `#[derive(SurrealValue)]` empirically deserializes correctly when
/// nested inside `Notification<R>::data` on a LIVE stream — test 16's
/// passing assertion that `notif.data.sender_device_key == alice_device`
/// is the standing verification of that round-trip.
#[derive(SurrealValue, Debug)]
struct MessageNotificationRow {
    id_key: String,
    sender_device_key: String,
    megolm_session_id: String,
    message_index: i64,
    ciphertext: String,
}

/// Test 16: two clients sharing a Megolm session, with the recipient on a
/// SurrealDB `LIVE SELECT` subscription. Alice POSTs M1 via the HTTP path;
/// Bob receives the notification on his stream, deserializes the row, and
/// decrypts with his `MegolmInbound`. The plaintext must round-trip.
///
/// **Subscribe-before-trigger** is the load-bearing fixture rule:
/// LIVE queries do NOT replay history. The stream is subscribed BEFORE
/// Alice POSTs.
///
/// The 2-second timeout on `stream.next()` keeps a regression from
/// hanging the suite. The empirical probe confirmed the subscription is
/// synchronous from the caller's POV (no yield required between subscribe
/// and trigger).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_clients_live_select_delivery() {
    let arena = arena().await;
    let mut alice_account = DeviceAccount::new();
    let mut bob_account = DeviceAccount::new();
    let (_, alice_device) = publish_device(&arena.router, &mut alice_account, 3).await;
    let (bob_user, bob_device) = publish_device(&arena.router, &mut bob_account, 3).await;

    let room_id = create_room_via_http(&arena.router, &alice_device, "live").await;
    let (status, _) = post_json(
        &arena.router,
        &format!("/rooms/{room_id}/join"),
        &[("X-Device-Id", &alice_device)],
        &json!({ "user": bob_user }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "join must succeed");

    // Bootstrap a real Megolm session: Alice mints `MegolmOutbound`, claims
    // one of Bob's OTKs, Olm-encrypts the session key to Bob over the
    // `/keyshare` channel. Bob drains his inbox, decrypts, instantiates
    // `MegolmInbound`. Same shape as `tests/keyshare.rs`'s deposit round-trip.
    let mut alice_megolm = MegolmOutbound::new();
    let bob_claim_resp = claim_one_otk(&arena.router, &bob_user, &bob_device).await;
    let mut alice_olm_to_bob =
        OlmSession::outbound_from_claim(&alice_account, &bob_claim_resp).expect("olm outbound");
    let key_envelope = alice_olm_to_bob.encrypt(alice_megolm.session_key_base64().as_bytes());
    let (status, _) = post_json(
        &arena.router,
        &format!("/rooms/{room_id}/keyshare"),
        &[("X-Device-Id", &alice_device)],
        &json!({
            "recipient_device": bob_device,
            "envelope": serde_json::to_value(&key_envelope).unwrap()
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "keyshare deposit");

    let (_, inbox_body) = get_json(
        &arena.router,
        &format!("/rooms/{room_id}/keyshare/inbox"),
        &[("X-Device-Id", &bob_device)],
    )
    .await;
    let inbox: Vec<InboxEnvelope> =
        serde_json::from_value(inbox_body["envelopes"].clone()).expect("inbox envelopes");
    assert_eq!(inbox.len(), 1, "Bob's inbox should hold one envelope");
    let alice_curve_hex = identity_curve_hex(&alice_account);
    let (_bob_olm_to_alice, recovered_key_bytes) =
        OlmSession::inbound_from_prekey(&mut bob_account, &alice_curve_hex, &inbox[0].envelope)
            .expect("olm inbound");
    let session_key_str =
        std::str::from_utf8(&recovered_key_bytes).expect("session key is ascii base64");
    let mut bob_megolm =
        MegolmInbound::from_session_key_base64(session_key_str).expect("megolm inbound");

    // SUBSCRIBE BEFORE TRIGGER. The LIVE filter targets messages in this
    // room only — without WHERE we'd also see messages from concurrent
    // test runs, but the per-test namespace already isolates that. LIVE
    // subscription is synchronous on .await — see the "LIVE
    // subscribe-before-trigger discipline" section in the module doc
    // (lines 23-33). The ordering below is load-bearing.
    let bind_room_id = room_id.clone();
    let mut stream = arena
        .db
        .query(
            r#"
            LIVE SELECT
                meta::id(id)            AS id_key,
                meta::id(sender_device) AS sender_device_key,
                megolm_session_id,
                message_index,
                ciphertext
            FROM message
            WHERE room = type::record("room", $room_id);
            "#,
        )
        .bind(("room_id", bind_room_id))
        .await
        .expect("LIVE subscribe")
        .stream::<Notification<MessageNotificationRow>>(0)
        .expect("LIVE stream");

    // Alice POSTs the ciphertext.
    let plaintext = b"hello live";
    let wire = alice_megolm.encrypt(plaintext);
    let (status, _) = post_json(
        &arena.router,
        &format!("/rooms/{room_id}/messages"),
        &[("X-Device-Id", &alice_device)],
        &json!({
            "megolm_session_id": alice_megolm.session_id(),
            "message_index": wire.message_index,
            "ciphertext": wire.ciphertext
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "POST must succeed");

    // Bob's stream must produce exactly one Create notification.
    let notif = timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("LIVE notification did not arrive within 2s")
        .expect("stream ended unexpectedly")
        .expect("notification carried error");
    assert_eq!(notif.action, Action::Create, "expected CREATE: {notif:?}");
    assert_eq!(notif.data.sender_device_key, alice_device);
    assert_eq!(notif.data.megolm_session_id, alice_megolm.session_id());
    assert_eq!(notif.data.message_index, i64::from(wire.message_index));
    let live_wire = MegolmCiphertext {
        message_index: u32::try_from(notif.data.message_index).expect("non-negative"),
        ciphertext: notif.data.ciphertext.clone(),
    };
    let decrypted = bob_megolm.decrypt(&live_wire).expect("decrypt LIVE wire");
    assert_eq!(
        decrypted.plaintext, plaintext,
        "LIVE ciphertext must decrypt back to the original plaintext"
    );
}

/// Test 17 — forward-exclusion three-user rotation: the deferred
/// cryptographic invariant from step 7.
///
/// **Framing.** This is a CRYPTOGRAPHIC claim, not an access-control
/// claim. The orthogonal HTTP privacy-404 is covered by test 12
/// (`get_non_member_caller_returns_404`). The decisive bypass here is
/// that M2's ciphertext is read DIRECTLY via `arena.db` — never via
/// `GET /rooms/.../messages` — precisely so the 404 doesn't mask the
/// decrypt-failure we want to prove.
///
/// **What this proves.** After Charlie leaves the room, Alice rotates
/// to a new Megolm outbound session and shares the new session key with
/// Bob only. Charlie still has the OLD `MegolmInbound` (call it
/// `inbound1`). Even given the raw ciphertext bytes of Alice's
/// post-rotation message M2, Charlie's `inbound1.decrypt(&m2_wire)`
/// MUST fail — because each Megolm ciphertext carries an Ed25519
/// signature under the *outbound's* per-session keypair, and a
/// `MegolmInbound` is hard-bound at bootstrap to one signing key (see
/// `crypto::megolm::tests::wrong_session_key_inbound_cannot_decrypt`
/// for the same shape at the unit level).
///
/// **Past-message invariant.** The rotation must NOT retroactively
/// invalidate Charlie's prior decryptions — Charlie's `inbound1` on
/// the pre-rotation M1 still succeeds.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn forward_exclusion_three_user_rotation() {
    let arena = arena().await;
    let mut alice_account = DeviceAccount::new();
    let mut bob_account = DeviceAccount::new();
    let mut charlie_account = DeviceAccount::new();
    let (_, alice_device) = publish_device(&arena.router, &mut alice_account, 6).await;
    let (bob_user, bob_device) = publish_device(&arena.router, &mut bob_account, 6).await;
    let (charlie_user, charlie_device) =
        publish_device(&arena.router, &mut charlie_account, 6).await;

    // Alice creates the room, invites Bob, invites Charlie.
    let room_id = create_room_via_http(&arena.router, &alice_device, "fe").await;
    let (status, _) = post_json(
        &arena.router,
        &format!("/rooms/{room_id}/join"),
        &[("X-Device-Id", &alice_device)],
        &json!({ "user": bob_user }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = post_json(
        &arena.router,
        &format!("/rooms/{room_id}/join"),
        &[("X-Device-Id", &alice_device)],
        &json!({ "user": charlie_user }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Alice mints outbound session #1 and shares the session key with
    // Bob and Charlie via Olm-wrapped /keyshare envelopes.
    let mut alice_megolm_1 = MegolmOutbound::new();
    let mut bob_inbound_1 = bootstrap_megolm_inbound(
        &arena.router,
        &arena.db,
        &alice_account,
        &alice_device,
        &mut alice_megolm_1,
        &mut bob_account,
        &bob_user,
        &bob_device,
        &room_id,
    )
    .await;
    let mut charlie_inbound_1 = bootstrap_megolm_inbound(
        &arena.router,
        &arena.db,
        &alice_account,
        &alice_device,
        &mut alice_megolm_1,
        &mut charlie_account,
        &charlie_user,
        &charlie_device,
        &room_id,
    )
    .await;

    // M1: pre-removal. Both Bob and Charlie can decrypt.
    let m1_plaintext = b"hello pre-removal";
    let m1_wire = alice_megolm_1.encrypt(m1_plaintext);
    let (status, body) = post_json(
        &arena.router,
        &format!("/rooms/{room_id}/messages"),
        &[("X-Device-Id", &alice_device)],
        &json!({
            "megolm_session_id": alice_megolm_1.session_id(),
            "message_index": m1_wire.message_index,
            "ciphertext": m1_wire.ciphertext
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "M1 POST: {body}");
    let m1_id = body["id"].as_str().expect("M1 id").to_string();

    // Bob GETs and decrypts M1.
    let bob_msgs_before = fetch_messages_via_http(&arena.router, &bob_device, &room_id).await;
    assert!(bob_msgs_before.iter().any(|m| m["id"] == m1_id));
    let bob_m1 = first_envelope(&bob_msgs_before, &m1_id);
    let d_bob_m1 = bob_inbound_1
        .decrypt(&megolm_wire_from_envelope(bob_m1))
        .expect("Bob decrypts M1 pre-removal");
    assert_eq!(d_bob_m1.plaintext, m1_plaintext);

    // Charlie GETs and decrypts M1.
    let charlie_msgs_before =
        fetch_messages_via_http(&arena.router, &charlie_device, &room_id).await;
    assert!(charlie_msgs_before.iter().any(|m| m["id"] == m1_id));
    let charlie_m1 = first_envelope(&charlie_msgs_before, &m1_id);
    let d_charlie_m1 = charlie_inbound_1
        .decrypt(&megolm_wire_from_envelope(charlie_m1))
        .expect("Charlie decrypts M1 pre-removal");
    assert_eq!(d_charlie_m1.plaintext, m1_plaintext);

    // Charlie leaves the room.
    let (status, _) = post_json(
        &arena.router,
        &format!("/rooms/{room_id}/leave"),
        &[("X-Device-Id", &charlie_device)],
        &json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "Charlie /leave");

    // Alice rotates: new outbound session #2. Share with Bob only.
    let mut alice_megolm_2 = MegolmOutbound::new();
    assert_ne!(
        alice_megolm_1.session_id(),
        alice_megolm_2.session_id(),
        "fresh outbound must have a new session_id"
    );
    let mut bob_inbound_2 = bootstrap_megolm_inbound(
        &arena.router,
        &arena.db,
        &alice_account,
        &alice_device,
        &mut alice_megolm_2,
        &mut bob_account,
        &bob_user,
        &bob_device,
        &room_id,
    )
    .await;

    // M2: post-removal. Alice POSTs.
    let m2_plaintext = b"hello post-removal";
    let m2_wire = alice_megolm_2.encrypt(m2_plaintext);
    let (status, body) = post_json(
        &arena.router,
        &format!("/rooms/{room_id}/messages"),
        &[("X-Device-Id", &alice_device)],
        &json!({
            "megolm_session_id": alice_megolm_2.session_id(),
            "message_index": m2_wire.message_index,
            "ciphertext": m2_wire.ciphertext
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "M2 POST: {body}");
    let m2_id = body["id"].as_str().expect("M2 id").to_string();

    // Bob GETs M2 and decrypts via inbound_2.
    let bob_msgs_after = fetch_messages_via_http(&arena.router, &bob_device, &room_id).await;
    let bob_m2 = first_envelope(&bob_msgs_after, &m2_id);
    let d_bob_m2 = bob_inbound_2
        .decrypt(&megolm_wire_from_envelope(bob_m2))
        .expect("Bob decrypts M2 post-removal");
    assert_eq!(d_bob_m2.plaintext, m2_plaintext);

    // FORWARD-EXCLUSION ASSERTION. Read M2 directly via arena.db (NOT
    // via Charlie's /messages, which would 404 by privacy). Then attempt
    // to decrypt with Charlie's inbound_1 — MUST fail because inbound_1
    // is bound to alice_megolm_1's signing key and M2 is signed under
    // alice_megolm_2's signing key.
    let raw_m2 = fetch_message_ciphertext_directly(&arena.db, &m2_id).await;
    let decrypt_attempt = charlie_inbound_1.decrypt(&raw_m2);
    assert!(
        decrypt_attempt.is_err(),
        "Charlie's pre-rotation inbound MUST NOT decrypt M2 (forward exclusion); got: {:?}",
        decrypt_attempt.map(|d| String::from_utf8_lossy(&d.plaintext).into_owned())
    );

    // PAST-MESSAGE-NOT-INVALIDATED ASSERTION. Charlie's `inbound_1`
    // already decrypted M1 once via the HTTP path above; Megolm's
    // skip-ahead cache should let it decrypt the *same* index again as a
    // no-op. Re-read M1 directly via `arena.db` (symmetric with the M2
    // path: both raw-ciphertext fetches come from the same column) and
    // run the decrypt one more time — the symmetric "raw access doesn't
    // change the outcome on `inbound_1`'s legitimate session" check that
    // discriminates the cryptographic claim from the access-control claim.
    let raw_m1 = fetch_message_ciphertext_directly(&arena.db, &m1_id).await;
    let d_charlie_m1_replay = charlie_inbound_1
        .decrypt(&raw_m1)
        .expect("Charlie's inbound_1 must still decrypt M1 (past not invalidated)");
    assert_eq!(d_charlie_m1_replay.plaintext, m1_plaintext);
}

// ---------------------------------------------------------------------------
// Test 16 / 17 plumbing
// ---------------------------------------------------------------------------

/// `POST /keys/claim/{user}/{device}` against the in-process router.
/// Returns the parsed `ClaimKeyResponse` ready for `OlmSession::outbound_from_claim`.
async fn claim_one_otk(router: &axum::Router, user_id: &str, device_id: &str) -> ClaimKeyResponse {
    let (status, body) = post_json(
        router,
        &format!("/keys/claim/{user_id}/{device_id}"),
        &[],
        &json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "/keys/claim: {body}");
    serde_json::from_value(body).expect("ClaimKeyResponse")
}

/// Olm-wrap the current `MegolmOutbound`'s session key, POST it to
/// `recipient_device` via `/keyshare`, drain the recipient's inbox, and
/// instantiate a matching `MegolmInbound`. Returns the inbound session.
///
/// Test 17 uses this twice for Alice→Bob and Alice→Charlie pre-removal,
/// then again post-removal for Alice→Bob only — and the third invocation
/// is what the forward-exclusion assertion hangs on (Charlie is NOT
/// re-invoked for the second outbound session).
#[allow(clippy::too_many_arguments)]
async fn bootstrap_megolm_inbound(
    router: &axum::Router,
    _db: &Surreal<Client>,
    sender_account: &DeviceAccount,
    sender_device_id: &str,
    sender_megolm: &mut MegolmOutbound,
    recipient_account: &mut DeviceAccount,
    recipient_user_id: &str,
    recipient_device_id: &str,
    room_id: &str,
) -> MegolmInbound {
    let claim = claim_one_otk(router, recipient_user_id, recipient_device_id).await;
    let mut olm_to_recipient =
        OlmSession::outbound_from_claim(sender_account, &claim).expect("olm outbound");
    let key_envelope = olm_to_recipient.encrypt(sender_megolm.session_key_base64().as_bytes());
    assert_eq!(key_envelope.message_type, OlmEnvelope::TYPE_PREKEY);

    let (status, _) = post_json(
        router,
        &format!("/rooms/{room_id}/keyshare"),
        &[("X-Device-Id", sender_device_id)],
        &json!({
            "recipient_device": recipient_device_id,
            "envelope": serde_json::to_value(&key_envelope).unwrap()
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "keyshare deposit");

    let (_, inbox_body) = get_json(
        router,
        &format!("/rooms/{room_id}/keyshare/inbox"),
        &[("X-Device-Id", recipient_device_id)],
    )
    .await;
    let inbox: Vec<InboxEnvelope> =
        serde_json::from_value(inbox_body["envelopes"].clone()).expect("inbox envelopes");
    // The recipient drain is delete-on-read; the inbox may already have
    // accumulated multiple envelopes if `bootstrap_megolm_inbound` has
    // been called more than once for the same recipient (e.g. Bob in
    // test 17 receives BOTH session #1 and session #2). Pick the
    // envelope from THIS sender.
    let envelope = inbox
        .into_iter()
        .find(|e| e.sender_device == sender_device_id)
        .expect("envelope from sender must be in drain");

    let sender_curve_hex = identity_curve_hex(sender_account);
    let (_olm_inbound, recovered_key_bytes) =
        OlmSession::inbound_from_prekey(recipient_account, &sender_curve_hex, &envelope.envelope)
            .expect("olm inbound");
    let key_str = std::str::from_utf8(&recovered_key_bytes).expect("base64 ascii session key");
    MegolmInbound::from_session_key_base64(key_str).expect("megolm inbound")
}

/// `GET /rooms/{room_id}/messages` via the router; returns the
/// `messages` array as raw JSON. Panics on non-200.
async fn fetch_messages_via_http(
    router: &axum::Router,
    device_id: &str,
    room_id: &str,
) -> Vec<Value> {
    let (status, body) = get_json(
        router,
        &format!("/rooms/{room_id}/messages"),
        &[("X-Device-Id", device_id)],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "GET /messages: {body}");
    body.get("messages")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
}

fn first_envelope<'a>(messages: &'a [Value], id: &str) -> &'a Value {
    messages
        .iter()
        .find(|m| m.get("id").and_then(|v| v.as_str()) == Some(id))
        .expect("expected message id in /messages response")
}

fn megolm_wire_from_envelope(envelope: &Value) -> MegolmCiphertext {
    MegolmCiphertext {
        message_index: u32::try_from(
            envelope
                .get("message_index")
                .and_then(|v| v.as_u64())
                .expect("message_index"),
        )
        .expect("message_index fits in u32"),
        ciphertext: envelope
            .get("ciphertext")
            .and_then(|v| v.as_str())
            .expect("ciphertext")
            .to_string(),
    }
}

/// Read a message row's ciphertext directly from the DB, bypassing HTTP.
/// Test 17's forward-exclusion assertion is the only caller — it needs to
/// read M2's ciphertext after Charlie has /leave'd, and Charlie's
/// `GET /rooms/.../messages` would 404 by privacy.
async fn fetch_message_ciphertext_directly(
    db: &Surreal<Client>,
    message_id: &str,
) -> MegolmCiphertext {
    #[derive(SurrealValue)]
    struct CipherRow {
        message_index: i64,
        ciphertext: String,
    }
    let mut resp = db
        .query("SELECT message_index, ciphertext FROM type::record('message', $message_id);")
        .bind(("message_id", message_id.to_string()))
        .await
        .expect("fetch ciphertext")
        .check()
        .expect("fetch ciphertext check");
    let row: Option<CipherRow> = resp.take(0).expect("take ciphertext");
    let row = row.expect("message row must exist");
    MegolmCiphertext {
        message_index: u32::try_from(row.message_index).expect("message_index fits in u32"),
        ciphertext: row.ciphertext,
    }
}
