//! Integration tests for `POST /rooms`, `POST /rooms/{id}/join`, and
//! `POST /rooms/{id}/leave` (routing-plan step 7).
//!
//! These hit a real SurrealDB. Run `./scripts/dev-db.sh` first. Each test
//! reserves a fresh namespace/database (via `tests/common::arena`) so
//! concurrent runs don't collide.
//!
//! No `crypto::*` imports anywhere in this file — step 7 is pure server
//! surface. Devices are seeded with throwaway-hex identity keys via the
//! local `create_test_user_and_device` helper, which is enough because no
//! handler decrypts anything.
//!
//! ## Critical for tests 16 and 17
//!
//! The concurrency canaries assert DB state *after* the race, not just
//! response codes. Response-only assertions miss the "HTTP 4xx returned
//! but the CREATE already fired" failure mode (the step-5 lesson per the
//! `validate-before-side-effect` memory). The `count_*` and `select_*`
//! helpers below do that work.

#![cfg(feature = "ssr")]

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use serde_json::{json, Value};
use surrealdb::engine::remote::ws::Client;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;
use tower::ServiceExt;

mod common;
use common::{arena, post_json, random_id};

// ---------------------------------------------------------------------------
// Test fixtures (no crypto)
// ---------------------------------------------------------------------------

/// Seed a fresh `(user, device)` pair via raw CREATE statements. Identity
/// keys are zero-bytes — step 7 doesn't decrypt anything so the actual
/// curve/ed25519 material is irrelevant. Returns
/// `(user_id, device_id)` exactly as the server will see them.
///
/// Kept inline rather than in `tests/common/` because `tests/keys.rs` and
/// `tests/keyshare.rs` have their own crypto-aware `publish_device` helper
/// and only `tests/rooms.rs` needs the no-crypto shortcut.
async fn create_test_user_and_device(db: &Surreal<Client>) -> (String, String) {
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

// ---------------------------------------------------------------------------
// DB-state helpers
// ---------------------------------------------------------------------------

#[derive(SurrealValue)]
struct CountRow {
    n: i64,
}

#[derive(SurrealValue)]
struct EventRow {
    event_type: String,
    actor_key: String,
    target_key: Option<String>,
}

/// Number of `room_member` rows for `(room_id, user_id)`. Used to assert
/// "exactly one membership row" or "no membership row" after a race.
async fn count_room_members(db: &Surreal<Client>, room_id: &str, user_id: &str) -> i64 {
    let mut resp = db
        .query(
            "SELECT count() AS n FROM room_member
             WHERE room = type::record('room', $room_id)
               AND user = type::record('user', $user_id)
             GROUP ALL;",
        )
        .bind(("room_id", room_id.to_string()))
        .bind(("user_id", user_id.to_string()))
        .await
        .expect("count_room_members query")
        .check()
        .expect("count_room_members check");
    let row: Option<CountRow> = resp.take(0).expect("count_room_members take");
    row.map(|r| r.n).unwrap_or(0)
}

/// Number of `room_event` rows matching `(room_id, event_type, target?)`.
/// `target_key` is `Some(uid)` to filter to a specific target user, or
/// `None` to filter to events with `target IS NONE` (e.g. `'create'`).
async fn count_room_events(
    db: &Surreal<Client>,
    room_id: &str,
    event_type: &str,
    target_key: Option<&str>,
) -> i64 {
    // GROUP ALL forces COUNT() to emit one row even when there are no
    // matches — without it the query returns zero rows and `take` would
    // give `None`, which we'd then have to map to 0 in Rust. With the
    // grouping, zero matches produce `n = 0`.
    let sql = match target_key {
        Some(_) => {
            "SELECT count() AS n FROM room_event
             WHERE room = type::record('room', $room_id)
               AND event_type = $event_type
               AND target = type::record('user', $target_key)
             GROUP ALL;"
        }
        None => {
            "SELECT count() AS n FROM room_event
             WHERE room = type::record('room', $room_id)
               AND event_type = $event_type
               AND target IS NONE
             GROUP ALL;"
        }
    };
    let mut q = db
        .query(sql)
        .bind(("room_id", room_id.to_string()))
        .bind(("event_type", event_type.to_string()));
    if let Some(t) = target_key {
        q = q.bind(("target_key", t.to_string()));
    }
    let mut resp = q
        .await
        .expect("count_room_events query")
        .check()
        .expect("count_room_events check");
    let row: Option<CountRow> = resp.take(0).expect("count_room_events take");
    row.map(|r| r.n).unwrap_or(0)
}

/// Sole `room_event` for `(room_id, event_type)`. Panics if zero or >1
/// match — used by tests that need to inspect the actor/target of a
/// known-single event row.
///
/// `target_key` requires the `IF target = NONE` guard because
/// `meta::id(NONE)` errors with "Argument 1 was the wrong type. Expected
/// `record` but found `NONE`" — `'create'` events have a NONE target by
/// design (no redundant 'join' for the creator).
async fn get_sole_room_event(db: &Surreal<Client>, room_id: &str, event_type: &str) -> EventRow {
    let mut resp = db
        .query(
            "SELECT
                event_type,
                meta::id(actor) AS actor_key,
                IF target = NONE THEN NONE ELSE meta::id(target) END AS target_key
             FROM room_event
             WHERE room = type::record('room', $room_id)
               AND event_type = $event_type;",
        )
        .bind(("room_id", room_id.to_string()))
        .bind(("event_type", event_type.to_string()))
        .await
        .expect("get_sole_room_event query")
        .check()
        .expect("get_sole_room_event check");
    let rows: Vec<EventRow> = resp.take(0).expect("get_sole_room_event take");
    assert_eq!(
        rows.len(),
        1,
        "expected exactly one '{event_type}' event for room {room_id}, found {}",
        rows.len()
    );
    rows.into_iter().next().unwrap()
}

#[derive(SurrealValue)]
struct RoomRow {
    name: String,
    created_by_key: String,
}

async fn get_room(db: &Surreal<Client>, room_id: &str) -> Option<RoomRow> {
    let mut resp = db
        .query(
            "SELECT name, meta::id(created_by) AS created_by_key
             FROM type::record('room', $room_id);",
        )
        .bind(("room_id", room_id.to_string()))
        .await
        .expect("get_room query")
        .check()
        .expect("get_room check");
    let row: Option<RoomRow> = resp.take(0).expect("get_room take");
    row
}

// ---------------------------------------------------------------------------
// Happy paths
// ---------------------------------------------------------------------------

/// Test 1: `POST /rooms { name: "test" }` returns 201 with a typed body
/// AND every implied row is in the DB exactly once: the `room` row with
/// `name` + `created_by`, the creator's `room_member` row, and a single
/// `'create'` `room_event` whose `target IS NONE` (the
/// "actor-of-create-is-the-initial-member" convention — no redundant
/// `'join'` event).
#[tokio::test]
async fn create_room_returns_201_with_full_state() {
    let arena = arena().await;
    let (alice_user, alice_device) = create_test_user_and_device(&arena.db).await;

    let (status, body) = post_json(
        &arena.router,
        "/rooms",
        &[("X-Device-Id", &alice_device)],
        &json!({ "name": "test" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create_room failed: {body}");

    let room_id = body
        .get("id")
        .and_then(|v| v.as_str())
        .expect("response.id")
        .to_string();
    assert!(
        body.get("room_event_id").and_then(|v| v.as_str()).is_some(),
        "response.room_event_id missing: {body}"
    );

    // Room row exists with the right name + creator.
    let room = get_room(&arena.db, &room_id)
        .await
        .expect("room row must exist");
    assert_eq!(room.name, "test");
    assert_eq!(room.created_by_key, alice_user);

    // Membership: Alice is in.
    assert_eq!(
        count_room_members(&arena.db, &room_id, &alice_user).await,
        1
    );

    // Exactly one 'create' event, with actor=alice and target IS NONE.
    assert_eq!(
        count_room_events(&arena.db, &room_id, "create", None).await,
        1,
        "expected exactly one 'create' event with target IS NONE"
    );
    let evt = get_sole_room_event(&arena.db, &room_id, "create").await;
    assert_eq!(evt.actor_key, alice_user);
    assert!(
        evt.target_key.is_none(),
        "'create' event must have target IS NONE (no redundant join), got {:?}",
        evt.target_key
    );

    // No 'join' event for the creator — the convention is that the
    // creator's membership is announced by the 'create' event itself.
    assert_eq!(
        count_room_events(&arena.db, &room_id, "join", Some(&alice_user)).await,
        0,
        "creation must NOT emit a redundant 'join' event for the creator"
    );
}

/// Test 2: Alice creates a room, then invites Bob. Response is 200 with
/// a `room_event_id`; Bob's `room_member` row exists; `'join'` event row
/// has `actor=alice, target=bob`.
#[tokio::test]
async fn join_returns_200_with_event_id() {
    let arena = arena().await;
    let (alice_user, alice_device) = create_test_user_and_device(&arena.db).await;
    let (bob_user, _bob_device) = create_test_user_and_device(&arena.db).await;

    // Alice creates the room.
    let (_, body) = post_json(
        &arena.router,
        "/rooms",
        &[("X-Device-Id", &alice_device)],
        &json!({ "name": "room2" }),
    )
    .await;
    let room_id = body["id"].as_str().expect("room id").to_string();

    // Alice invites Bob.
    let (status, body) = post_json(
        &arena.router,
        &format!("/rooms/{room_id}/join"),
        &[("X-Device-Id", &alice_device)],
        &json!({ "user": bob_user }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "join failed: {body}");
    assert!(
        body.get("room_event_id").and_then(|v| v.as_str()).is_some(),
        "join response must carry room_event_id: {body}"
    );

    assert_eq!(count_room_members(&arena.db, &room_id, &bob_user).await, 1);
    assert_eq!(
        count_room_events(&arena.db, &room_id, "join", Some(&bob_user)).await,
        1
    );
    let evt = get_sole_room_event(&arena.db, &room_id, "join").await;
    assert_eq!(evt.actor_key, alice_user);
    assert_eq!(evt.target_key.as_deref(), Some(bob_user.as_str()));
}

/// Test 3: After Alice invites Bob, Bob's `/leave` returns 200 with a
/// `room_event_id`. Bob's `room_member` row is gone; the `'leave'` event
/// has `actor==target==bob`.
#[tokio::test]
async fn leave_returns_200_with_event_id() {
    let arena = arena().await;
    let (_alice_user, alice_device) = create_test_user_and_device(&arena.db).await;
    let (bob_user, bob_device) = create_test_user_and_device(&arena.db).await;

    let (_, body) = post_json(
        &arena.router,
        "/rooms",
        &[("X-Device-Id", &alice_device)],
        &json!({ "name": "room3" }),
    )
    .await;
    let room_id = body["id"].as_str().expect("room id").to_string();

    let (_, _) = post_json(
        &arena.router,
        &format!("/rooms/{room_id}/join"),
        &[("X-Device-Id", &alice_device)],
        &json!({ "user": bob_user }),
    )
    .await;

    // Bob leaves.
    let (status, body) = post_json(
        &arena.router,
        &format!("/rooms/{room_id}/leave"),
        &[("X-Device-Id", &bob_device)],
        &json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "leave failed: {body}");
    assert!(
        body.get("room_event_id").and_then(|v| v.as_str()).is_some(),
        "leave response must carry room_event_id: {body}"
    );

    assert_eq!(count_room_members(&arena.db, &room_id, &bob_user).await, 0);
    assert_eq!(
        count_room_events(&arena.db, &room_id, "leave", Some(&bob_user)).await,
        1
    );
    let evt = get_sole_room_event(&arena.db, &room_id, "leave").await;
    assert_eq!(evt.actor_key, bob_user, "leave actor must be bob himself");
    assert_eq!(
        evt.target_key.as_deref(),
        Some(bob_user.as_str()),
        "leave target must be bob himself (self-leave only)"
    );
}

// ---------------------------------------------------------------------------
// Auth / validation paths (typed error bodies)
// ---------------------------------------------------------------------------

/// Test 4: POST /rooms without X-Device-Id → 401 typed body.
#[tokio::test]
async fn create_missing_device_id_returns_401() {
    let arena = arena().await;
    let (status, body) = post_json(&arena.router, "/rooms", &[], &json!({ "name": "x" })).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("missing X-Device-Id header"),
        "{body}"
    );
}

/// Test 4b: POST /rooms with a well-formed but unknown X-Device-Id → 401
/// typed body `"unknown caller device"`. Distinct from test 4
/// (`extract_device_id` returns `None` for the *missing header* branch);
/// this exercises the `load_caller_user` `Ok(None)` branch where the
/// device row simply doesn't exist. Mirrors `tests/keyshare.rs`'s
/// `deposit_from_unknown_sender_is_unauthorized`.
///
/// Discriminating assertion: no `room` row was created. This catches a
/// re-ordering mutation that would somehow commit the CREATE before the
/// 401 reply (`load_caller_user` runs strictly before `persist_create_room`,
/// so this is a fail-before-side-effect property — see the
/// `validate-before-side-effect` memory).
#[tokio::test]
async fn create_unknown_caller_device_returns_401() {
    let arena = arena().await;

    // Note: we deliberately do NOT call `create_test_user_and_device` —
    // no device row exists for the id we send.
    let bogus_device = random_id();
    let (status, body) = post_json(
        &arena.router,
        "/rooms",
        &[("X-Device-Id", &bogus_device)],
        &json!({ "name": "test" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("unknown caller device"),
        "{body}"
    );

    // Fail-before-side-effect: zero `room` rows in this arena's DB.
    let mut resp = arena
        .db
        .query("SELECT count() AS n FROM room GROUP ALL;")
        .await
        .expect("count rooms query")
        .check()
        .expect("count rooms check");
    let row: Option<CountRow> = resp.take(0).expect("count rooms take");
    assert_eq!(
        row.map(|r| r.n).unwrap_or(0),
        0,
        "unknown-caller 401 must NOT create a room row"
    );
}

/// Test 4c: POST /rooms with a malformed JSON body (syntactically invalid)
/// → 400 typed body `"malformed JSON"`. Covers the `JsonRejection::JsonSyntaxError`
/// arm of `json_rejection_response`. Mirrors `tests/keys.rs`'s
/// `malformed_upload_body_returns_typed_400` but with a stricter
/// equality check on the body string (the typed-400 reason is the
/// observable contract for the client).
#[tokio::test]
async fn create_malformed_json_returns_typed_400() {
    let arena = arena().await;
    let (_alice_user, alice_device) = create_test_user_and_device(&arena.db).await;

    // Raw text isn't valid JSON — triggers `JsonRejection::JsonSyntaxError`.
    let req = Request::builder()
        .method(Method::POST)
        .uri("/rooms")
        .header(header::CONTENT_TYPE, "application/json")
        .header("X-Device-Id", &alice_device)
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

/// Test 5: POST /rooms/{id}/join without X-Device-Id → 401 typed body.
#[tokio::test]
async fn join_missing_device_id_returns_401() {
    let arena = arena().await;
    let (status, body) = post_json(
        &arena.router,
        "/rooms/anything/join",
        &[],
        &json!({ "user": "u1" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("missing X-Device-Id header"),
        "{body}"
    );
}

/// Test 6: POST /rooms/{id}/leave without X-Device-Id → 401 typed body.
#[tokio::test]
async fn leave_missing_device_id_returns_401() {
    let arena = arena().await;
    let (status, body) = post_json(&arena.router, "/rooms/anything/leave", &[], &json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("missing X-Device-Id header"),
        "{body}"
    );
}

/// Test 7: POST /rooms with empty name (after trim) → 400. Both `""` and
/// `"   "` must be rejected with the same body.
#[tokio::test]
async fn create_empty_name_returns_400() {
    let arena = arena().await;
    let (_alice_user, alice_device) = create_test_user_and_device(&arena.db).await;

    for empty in ["", "   ", "\t\n"] {
        let (status, body) = post_json(
            &arena.router,
            "/rooms",
            &[("X-Device-Id", &alice_device)],
            &json!({ "name": empty }),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "name {empty:?} should be 400: {body}"
        );
        assert_eq!(
            body.get("error").and_then(|v| v.as_str()),
            Some("room name must not be empty"),
            "name {empty:?} body: {body}"
        );
    }

    // Sanity: a 201-char name (one over the 200-char cap) is rejected
    // with the dedicated "too long" message — the cap belongs to this
    // test so the validation table's row is covered alongside the empty
    // case (both are CreateRoomRequest field-level validations).
    let long = "x".repeat(201);
    let (status, body) = post_json(
        &arena.router,
        "/rooms",
        &[("X-Device-Id", &alice_device)],
        &json!({ "name": long }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("room name too long"),
        "{body}"
    );
}

/// Test 8: POST /rooms/{id}/join with an unknown room id (but a valid
/// caller) → 404 "room not found".
#[tokio::test]
async fn join_unknown_room_returns_404() {
    let arena = arena().await;
    let (_alice_user, alice_device) = create_test_user_and_device(&arena.db).await;
    let (bob_user, _) = create_test_user_and_device(&arena.db).await;

    let (status, body) = post_json(
        &arena.router,
        "/rooms/nonexistent_room/join",
        &[("X-Device-Id", &alice_device)],
        &json!({ "user": bob_user }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("room not found"),
        "{body}"
    );
}

/// Test 9: Privacy rule — Charlie (not in Alice's room) tries to invite
/// Bob. Response is 404 "room not found" (NOT 403), AND no
/// `room_member` row is created for Bob.
///
/// The DB-state assertion is the discriminator: a leaky implementation
/// that "returns 404 but writes the row anyway" would pass response-only
/// assertions.
#[tokio::test]
async fn join_non_member_caller_returns_404() {
    let arena = arena().await;
    let (_alice_user, alice_device) = create_test_user_and_device(&arena.db).await;
    let (bob_user, _) = create_test_user_and_device(&arena.db).await;
    let (_charlie_user, charlie_device) = create_test_user_and_device(&arena.db).await;

    let (_, body) = post_json(
        &arena.router,
        "/rooms",
        &[("X-Device-Id", &alice_device)],
        &json!({ "name": "rA" }),
    )
    .await;
    let room_id = body["id"].as_str().expect("room id").to_string();

    let (status, body) = post_json(
        &arena.router,
        &format!("/rooms/{room_id}/join"),
        &[("X-Device-Id", &charlie_device)],
        &json!({ "user": bob_user }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "non-member /join must be 404 (privacy), not 403: {body}"
    );
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("room not found"),
        "non-member /join body must match the same room-not-found body: {body}"
    );

    assert_eq!(
        count_room_members(&arena.db, &room_id, &bob_user).await,
        0,
        "non-member /join must not create a membership row for the target"
    );
    assert_eq!(
        count_room_events(&arena.db, &room_id, "join", Some(&bob_user)).await,
        0,
        "non-member /join must not append a 'join' event"
    );
}

/// Test 10: Alice tries to invite a user id that doesn't have a `user`
/// row → 404 "target user not found", no `room_member` row created.
#[tokio::test]
async fn join_unknown_target_user_returns_404() {
    let arena = arena().await;
    let (_alice_user, alice_device) = create_test_user_and_device(&arena.db).await;

    let (_, body) = post_json(
        &arena.router,
        "/rooms",
        &[("X-Device-Id", &alice_device)],
        &json!({ "name": "rT" }),
    )
    .await;
    let room_id = body["id"].as_str().expect("room id").to_string();

    let bogus_user = random_id();
    let (status, body) = post_json(
        &arena.router,
        &format!("/rooms/{room_id}/join"),
        &[("X-Device-Id", &alice_device)],
        &json!({ "user": bogus_user }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("target user not found"),
        "{body}"
    );
    assert_eq!(
        count_room_members(&arena.db, &room_id, &bogus_user).await,
        0,
        "failed /join must not create a membership row"
    );
}

/// Test 11: Pre-check 409. Alice invites Bob successfully; second
/// invitation from Alice → 409 "user is already a member". DB state:
/// still exactly one `room_member` for Bob.
#[tokio::test]
async fn join_target_already_member_returns_409_via_precheck() {
    let arena = arena().await;
    let (_alice_user, alice_device) = create_test_user_and_device(&arena.db).await;
    let (bob_user, _) = create_test_user_and_device(&arena.db).await;

    let (_, body) = post_json(
        &arena.router,
        "/rooms",
        &[("X-Device-Id", &alice_device)],
        &json!({ "name": "rD" }),
    )
    .await;
    let room_id = body["id"].as_str().expect("room id").to_string();

    let (status, _) = post_json(
        &arena.router,
        &format!("/rooms/{room_id}/join"),
        &[("X-Device-Id", &alice_device)],
        &json!({ "user": bob_user }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = post_json(
        &arena.router,
        &format!("/rooms/{room_id}/join"),
        &[("X-Device-Id", &alice_device)],
        &json!({ "user": bob_user }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("user is already a member"),
        "{body}"
    );

    assert_eq!(
        count_room_members(&arena.db, &room_id, &bob_user).await,
        1,
        "duplicate /join must not create a second membership row"
    );
}

/// Test 12: Self-invite is a 400 (not a 409). DB state: Alice's
/// `room_member` row is still exactly one — no duplicate.
#[tokio::test]
async fn join_self_returns_400() {
    let arena = arena().await;
    let (alice_user, alice_device) = create_test_user_and_device(&arena.db).await;

    let (_, body) = post_json(
        &arena.router,
        "/rooms",
        &[("X-Device-Id", &alice_device)],
        &json!({ "name": "rS" }),
    )
    .await;
    let room_id = body["id"].as_str().expect("room id").to_string();

    let (status, body) = post_json(
        &arena.router,
        &format!("/rooms/{room_id}/join"),
        &[("X-Device-Id", &alice_device)],
        &json!({ "user": alice_user }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("cannot invite yourself"),
        "{body}"
    );

    assert_eq!(
        count_room_members(&arena.db, &room_id, &alice_user).await,
        1,
        "self-invite must not duplicate the creator's membership row"
    );
}

/// Test 13: /leave on an unknown room → 404 "room not found".
#[tokio::test]
async fn leave_unknown_room_returns_404() {
    let arena = arena().await;
    let (_alice_user, alice_device) = create_test_user_and_device(&arena.db).await;

    let (status, body) = post_json(
        &arena.router,
        "/rooms/nonexistent_room/leave",
        &[("X-Device-Id", &alice_device)],
        &json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("room not found"),
        "{body}"
    );
}

/// Test 14: Privacy rule — Charlie (not in Alice's room) tries to /leave
/// it → 404 "room not found", AND no `'leave'` event row is created.
#[tokio::test]
async fn leave_non_member_caller_returns_404() {
    let arena = arena().await;
    let (_alice_user, alice_device) = create_test_user_and_device(&arena.db).await;
    let (charlie_user, charlie_device) = create_test_user_and_device(&arena.db).await;

    let (_, body) = post_json(
        &arena.router,
        "/rooms",
        &[("X-Device-Id", &alice_device)],
        &json!({ "name": "rL" }),
    )
    .await;
    let room_id = body["id"].as_str().expect("room id").to_string();

    let (status, body) = post_json(
        &arena.router,
        &format!("/rooms/{room_id}/leave"),
        &[("X-Device-Id", &charlie_device)],
        &json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("room not found"),
        "{body}"
    );

    assert_eq!(
        count_room_events(&arena.db, &room_id, "leave", Some(&charlie_user)).await,
        0,
        "non-member /leave must not append a 'leave' event"
    );
}

/// Test 15: Cross-room isolation. Alice creates rooms A and B; Bob is
/// invited to A only. Bob /leave on A → 200 (member); Bob /leave on B →
/// 404 (not a member, privacy). Final state across both rooms is what
/// the partial leaves should produce — Bob's membership is gone from A
/// and never existed in B, and the audit log reflects exactly one leave
/// (for A).
#[tokio::test]
async fn leave_cross_room_isolation() {
    let arena = arena().await;
    let (_alice_user, alice_device) = create_test_user_and_device(&arena.db).await;
    let (bob_user, bob_device) = create_test_user_and_device(&arena.db).await;

    let (_, a) = post_json(
        &arena.router,
        "/rooms",
        &[("X-Device-Id", &alice_device)],
        &json!({ "name": "A" }),
    )
    .await;
    let room_a = a["id"].as_str().expect("room A id").to_string();
    let (_, b) = post_json(
        &arena.router,
        "/rooms",
        &[("X-Device-Id", &alice_device)],
        &json!({ "name": "B" }),
    )
    .await;
    let room_b = b["id"].as_str().expect("room B id").to_string();

    // Bob is invited to A only.
    let (status, _) = post_json(
        &arena.router,
        &format!("/rooms/{room_a}/join"),
        &[("X-Device-Id", &alice_device)],
        &json!({ "user": bob_user }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Bob /leave B BEFORE /leave A: should 404 (privacy). This ordering
    // matters — if we did /leave A first, Bob would no longer be a
    // member of *any* room and a buggy pre-check that doesn't filter by
    // `room` (matches "is the caller a member of anything") would
    // *still* surface as 404 because there are no rows to match,
    // creating a false negative. Testing /leave B *while Bob is still
    // in A* forces the pre-check's `room = ...` filter to be load-bearing.
    let (status, body) = post_json(
        &arena.router,
        &format!("/rooms/{room_b}/leave"),
        &[("X-Device-Id", &bob_device)],
        &json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("room not found"),
        "{body}"
    );

    // Bob /leave A: should succeed.
    let (status, body) = post_json(
        &arena.router,
        &format!("/rooms/{room_a}/leave"),
        &[("X-Device-Id", &bob_device)],
        &json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "leave A failed: {body}");

    // Sanity: Bob /leave B *after* /leave A is still 404 — same body.
    let (status, body) = post_json(
        &arena.router,
        &format!("/rooms/{room_b}/leave"),
        &[("X-Device-Id", &bob_device)],
        &json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("room not found"),
        "{body}"
    );

    // Final DB state:
    assert_eq!(
        count_room_members(&arena.db, &room_a, &bob_user).await,
        0,
        "Bob's membership of room A must be gone"
    );
    assert_eq!(
        count_room_members(&arena.db, &room_b, &bob_user).await,
        0,
        "Bob was never in room B"
    );
    assert_eq!(
        count_room_events(&arena.db, &room_a, "leave", Some(&bob_user)).await,
        1,
        "exactly one 'leave' event for Bob on room A"
    );
    assert_eq!(
        count_room_events(&arena.db, &room_b, "leave", Some(&bob_user)).await,
        0,
        "no 'leave' event for Bob on room B (he was never a member)"
    );
}

// ---------------------------------------------------------------------------
// Concurrency canaries
// ---------------------------------------------------------------------------

/// Test 16: Concurrent invites. Alice has invited Charlie; both Alice
/// and Charlie race to invite Bob via `tokio::join!`. Exactly one of
/// the two `/join` calls must return 200 and the other 409 "user is
/// already a member" — the test does NOT pin which inviter wins.
///
/// **DB-state invariant (the real discriminator).** After the race:
/// exactly ONE `room_member` row for Bob, exactly ONE `'join'`
/// `room_event` row with `target = bob`. Any other cardinality means
/// the dual-path 409 mapping (pre-check OR UNIQUE-violation residual)
/// failed: either the row leaked twice (UNIQUE didn't fire and the
/// retry helper happily committed both) or the row never landed
/// (something rolled back when it shouldn't have).
///
/// Runs on the multi-thread runtime so the two POST futures execute on
/// distinct OS threads — current_thread tokio would serialise the
/// spawn futures and turn the race into a sequential pair of
/// pre-check 409s, hiding the UNIQUE path entirely.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_invites_yield_single_join_one_409() {
    let arena = arena().await;
    let (_alice_user, alice_device) = create_test_user_and_device(&arena.db).await;
    let (bob_user, _) = create_test_user_and_device(&arena.db).await;
    let (charlie_user, charlie_device) = create_test_user_and_device(&arena.db).await;

    // Alice creates the room and invites Charlie first, so both Alice
    // and Charlie are members and can each issue /join for Bob.
    let (_, body) = post_json(
        &arena.router,
        "/rooms",
        &[("X-Device-Id", &alice_device)],
        &json!({ "name": "race" }),
    )
    .await;
    let room_id = body["id"].as_str().expect("room id").to_string();
    let (status, _) = post_json(
        &arena.router,
        &format!("/rooms/{room_id}/join"),
        &[("X-Device-Id", &alice_device)],
        &json!({ "user": charlie_user }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Now race Alice and Charlie both inviting Bob.
    let r1 = {
        let router = arena.router.clone();
        let path = format!("/rooms/{room_id}/join");
        let device = alice_device.clone();
        let target = bob_user.clone();
        tokio::spawn(async move {
            post_json(
                &router,
                &path,
                &[("X-Device-Id", &device)],
                &json!({ "user": target }),
            )
            .await
        })
    };
    let r2 = {
        let router = arena.router.clone();
        let path = format!("/rooms/{room_id}/join");
        let device = charlie_device.clone();
        let target = bob_user.clone();
        tokio::spawn(async move {
            post_json(
                &router,
                &path,
                &[("X-Device-Id", &device)],
                &json!({ "user": target }),
            )
            .await
        })
    };
    let (a, c) = tokio::join!(r1, r2);
    let (s_a, b_a) = a.expect("alice join");
    let (s_c, b_c) = c.expect("charlie join");

    // Exactly one 200 + one 409 — order unspecified.
    let mut codes = [s_a, s_c];
    codes.sort_by_key(|c| c.as_u16());
    assert_eq!(
        codes,
        [StatusCode::OK, StatusCode::CONFLICT],
        "expected one 200 and one 409, got alice={s_a} ({b_a}), charlie={s_c} ({b_c})"
    );
    // The 409 body must match the dual-path body verbatim — clients
    // shouldn't have to tell pre-check vs UNIQUE-violation apart.
    let conflict_body = if s_a == StatusCode::CONFLICT {
        &b_a
    } else {
        &b_c
    };
    assert_eq!(
        conflict_body.get("error").and_then(|v| v.as_str()),
        Some("user is already a member"),
        "409 body must match the pre-check body: {conflict_body}"
    );

    // DB invariant: exactly one membership for Bob, exactly one 'join'
    // event with target=bob.
    assert_eq!(
        count_room_members(&arena.db, &room_id, &bob_user).await,
        1,
        "exactly one room_member row for Bob after the race"
    );
    assert_eq!(
        count_room_events(&arena.db, &room_id, "join", Some(&bob_user)).await,
        1,
        "exactly one 'join' event for Bob after the race"
    );
}

/// Test 17: Concurrent /join and /leave. Alice has Bob in the room;
/// concurrently Alice invites Charlie AND Bob leaves. Both
/// transactions commit cleanly; the final state is consistent.
///
/// Disjoint-row concurrent transactions (join target=Charlie writes one
/// `room_member`; leave actor=Bob deletes a different `room_member`)
/// don't actually contend at the MVCC layer — both transactions write
/// to different keys. This test verifies the weaker but still
/// load-bearing invariant: **the handler emits exactly one of each
/// side-effect row, not duplicate or missing rows, when join and leave
/// commit in parallel.** Mutations caught: drop the DELETE in
/// `do_leave_write`, drop the leave-event CREATE, drop the join-event
/// CREATE, or any swallowed-error 200 with no DB write. NOT a
/// contention-arbitration test — that'd require same-row races (out of
/// scope for v1; see "Out of scope — Leave-double-tap row pollution" in
/// the step-7 plan).
///
/// **DB-state invariant.** Final membership = {Alice, Charlie} exactly
/// (Bob is gone). Both `room_event` rows are recorded: one
/// `'join' actor=alice target=charlie`, one `'leave' actor=bob target=bob`.
/// No duplicate or missing rows.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_join_and_leave_no_torn_state() {
    let arena = arena().await;
    let (alice_user, alice_device) = create_test_user_and_device(&arena.db).await;
    let (bob_user, bob_device) = create_test_user_and_device(&arena.db).await;
    let (charlie_user, _) = create_test_user_and_device(&arena.db).await;

    let (_, body) = post_json(
        &arena.router,
        "/rooms",
        &[("X-Device-Id", &alice_device)],
        &json!({ "name": "tear" }),
    )
    .await;
    let room_id = body["id"].as_str().expect("room id").to_string();
    let (status, _) = post_json(
        &arena.router,
        &format!("/rooms/{room_id}/join"),
        &[("X-Device-Id", &alice_device)],
        &json!({ "user": bob_user }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Race Alice-invites-Charlie + Bob-leaves.
    let join_fut = {
        let router = arena.router.clone();
        let path = format!("/rooms/{room_id}/join");
        let device = alice_device.clone();
        let target = charlie_user.clone();
        tokio::spawn(async move {
            post_json(
                &router,
                &path,
                &[("X-Device-Id", &device)],
                &json!({ "user": target }),
            )
            .await
        })
    };
    let leave_fut = {
        let router = arena.router.clone();
        let path = format!("/rooms/{room_id}/leave");
        let device = bob_device.clone();
        tokio::spawn(async move {
            post_json(&router, &path, &[("X-Device-Id", &device)], &json!({})).await
        })
    };
    let (j, l) = tokio::join!(join_fut, leave_fut);
    let (s_j, b_j) = j.expect("join join");
    let (s_l, b_l) = l.expect("leave join");
    assert_eq!(s_j, StatusCode::OK, "concurrent /join must succeed: {b_j}");
    assert_eq!(s_l, StatusCode::OK, "concurrent /leave must succeed: {b_l}");

    // Final state: {Alice, Charlie} are members, Bob is not.
    assert_eq!(
        count_room_members(&arena.db, &room_id, &alice_user).await,
        1
    );
    assert_eq!(
        count_room_members(&arena.db, &room_id, &charlie_user).await,
        1,
        "Charlie's /join must have committed: {b_j}"
    );
    assert_eq!(
        count_room_members(&arena.db, &room_id, &bob_user).await,
        0,
        "Bob's /leave must have committed: {b_l}"
    );

    // Audit log: exactly one 'join' for Charlie, exactly one 'leave'
    // for Bob, no duplicates.
    assert_eq!(
        count_room_events(&arena.db, &room_id, "join", Some(&charlie_user)).await,
        1,
        "expected exactly one 'join' event for Charlie"
    );
    assert_eq!(
        count_room_events(&arena.db, &room_id, "leave", Some(&bob_user)).await,
        1,
        "expected exactly one 'leave' event for Bob"
    );
    // No 'join' was emitted for Bob's leave (sanity — different event_type).
    assert_eq!(
        count_room_events(&arena.db, &room_id, "join", Some(&bob_user)).await,
        1,
        "Bob's original 'join' event (from the pre-race invitation) must still be in the log"
    );
}
