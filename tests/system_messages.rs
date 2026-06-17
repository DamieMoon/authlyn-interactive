//! App-admin system broadcast ("Nova DOT") — `POST /admin/system-message`.
//!
//! Mirrors the project's admin-endpoint test convention (see `tests/feedback.rs`):
//! the admin-ALLOWED path can't be driven through HTTP (the `is_admin` env read
//! races parallel workers), so the fan-out LOGIC is exercised directly against the
//! DB via the `broadcast_system_message` core fn, while only the fail-closed gate
//! (non-admin → 403, unauth → 401) is checked through the router.

mod common;

#[cfg(feature = "ssr")]
use authlyn_interactive::server::system_messages::{
    broadcast_system_message, validate_broadcast_body,
};
#[cfg(feature = "ssr")]
use authlyn_interactive::server::AppState;
#[cfg(feature = "ssr")]
use axum::http::{Method, StatusCode};
#[cfg(feature = "ssr")]
use serde_json::json;
#[cfg(feature = "ssr")]
use surrealdb::types::SurrealValue;

// ---------------------------------------------------------------------------
// Body validation (pure fn — unit-testable without the admin-gated route)
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
#[test]
fn validate_broadcast_body_enforces_bounds_and_trims() {
    assert!(validate_broadcast_body("").is_err(), "empty → err");
    assert!(
        validate_broadcast_body("   \n\t ").is_err(),
        "whitespace-only → err"
    );
    assert_eq!(
        validate_broadcast_body("  hello  ").expect("ok"),
        "hello",
        "valid body is trimmed"
    );
    assert!(
        validate_broadcast_body(&"x".repeat(4000)).is_ok(),
        "4000 chars is the upper boundary"
    );
    assert!(
        validate_broadcast_body(&"x".repeat(4001)).is_err(),
        "4001 chars → err"
    );
}

// ---------------------------------------------------------------------------
// Fan-out core (exercised directly; the HTTP admin gate is unreachable in tests)
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
#[tokio::test]
async fn broadcast_posts_a_nova_dot_system_message_into_each_guilds_first_text_channel() {
    let a = common::arena().await;
    let state = AppState::new(a.db.clone(), a.media_dir.clone());

    // Two live guilds. g1 has two text channels — the broadcast must hit the
    // FIRST by position (g1c1, position 1), not g1c2 (position 2).
    a.db.query(
        "CREATE account:owner SET username='Owner', username_ci='owner', password_hash='x';\
         CREATE guild:g1 SET name='G1', owner=account:owner;\
         CREATE channel:g1c2 SET guild=guild:g1, name='second', kind='text', position=2;\
         CREATE channel:g1c1 SET guild=guild:g1, name='first',  kind='text', position=1;\
         CREATE guild:g2 SET name='G2', owner=account:owner;\
         CREATE channel:g2c1 SET guild=guild:g2, name='only',   kind='text', position=0;",
    )
    .await
    .expect("seed transport")
    .check()
    .expect("seed");

    let result = broadcast_system_message(&state, "Scheduled maintenance")
        .await
        .expect("broadcast");
    assert_eq!(result.guilds_targeted, 2);
    assert_eq!(result.messages_sent, 2);
    assert_eq!(result.guilds_skipped, 0);

    #[derive(SurrealValue)]
    struct Row {
        channel_key: String,
        author_key: String,
        kind: String,
        body: String,
    }
    let mut resp =
        a.db.query(
            "SELECT meta::id(channel) AS channel_key, meta::id(author) AS author_key, kind, body \
             FROM message ORDER BY channel_key;",
        )
        .await
        .expect("query")
        .check()
        .expect("check");
    let rows: Vec<Row> = resp.take(0).expect("take");

    let channels: Vec<String> = rows.iter().map(|r| r.channel_key.clone()).collect();
    assert_eq!(
        channels,
        vec!["g1c1".to_string(), "g2c1".to_string()],
        "lands in g1's FIRST channel and g2's only channel"
    );
    assert!(
        rows.iter().all(|r| r.kind == "system"
            && r.author_key == "nova_dot"
            && r.body == "Scheduled maintenance"),
        "every broadcast row is a Nova DOT system message"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn broadcast_skips_guilds_with_no_live_text_channel() {
    let a = common::arena().await;
    let state = AppState::new(a.db.clone(), a.media_dir.clone());

    a.db.query(
        "CREATE account:o SET username='O', username_ci='owner-seed', password_hash='x';\
         CREATE guild:ghas  SET name='Has',  owner=account:o;\
         CREATE channel:ghasc  SET guild=guild:ghas,  name='general', kind='text',     position=0;\
         CREATE guild:glore SET name='Lore', owner=account:o;\
         CREATE channel:glorec SET guild=guild:glore, name='lore',    kind='lorebook', position=0;\
         CREATE guild:gdel  SET name='Del',  owner=account:o;\
         CREATE channel:gdelc  SET guild=guild:gdel,  name='gone',    kind='text',     position=0, deleted_at=time::now();\
         CREATE guild:gdead SET name='Dead', owner=account:o, deleted_at=time::now();\
         CREATE channel:gdeadc SET guild=guild:gdead, name='c',       kind='text',     position=0;",
    )
    .await
    .expect("seed transport")
    .check()
    .expect("seed");

    let result = broadcast_system_message(&state, "hi")
        .await
        .expect("broadcast");
    assert_eq!(
        result.guilds_targeted, 3,
        "the soft-deleted guild is excluded entirely (not even targeted)"
    );
    assert_eq!(
        result.messages_sent, 1,
        "only the guild with a live text channel receives a message"
    );
    assert_eq!(
        result.guilds_skipped, 2,
        "lorebook-only and deleted-channel-only guilds are skipped"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn system_message_round_trips_with_kind_system_over_get_messages() {
    let a = common::arena().await;
    let state = AppState::new(a.db.clone(), a.media_dir.clone());

    // Guild creation auto-makes a default text channel (channels[0]) — the
    // channel a broadcast targets.
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let (st, _, guild) = common::send(
        &a.router,
        Method::POST,
        "/guilds",
        Some(&owner),
        Some(&json!({ "name": "Guild" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let gid = guild["id"].as_str().unwrap().to_string();
    let (st, _, detail) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let cid = detail["channels"][0]["id"].as_str().unwrap().to_string();

    let result = broadcast_system_message(&state, "Nova speaks")
        .await
        .expect("broadcast");
    assert_eq!(result.messages_sent, 1);

    let (st, _, body) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let msgs = body["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["kind"], "system", "envelope carries kind='system'");
    assert_eq!(
        msgs[0]["author_display"], "Nova DOT",
        "renders as the Nova DOT bot"
    );
    assert_eq!(msgs[0]["body"], "Nova speaks");
}

/// Task 6/7 review carry-over: the fan-out must emit `message_created` on the
/// SSE bus per fanned-out message, like every other message write. Driven on
/// `a.state` (the ROUTER's `AppState`) so the emission lands on the same bus
/// the `GET /events` stream subscribes to.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn broadcast_emits_message_created_per_fanned_out_message_over_sse() {
    let a = common::arena().await;

    // Owner + guild via HTTP so guild_member rows (SSE visibility) are real.
    let owner = common::register_account(&a.router, "BusOwner", "password123").await;
    let (st, _, guild) = common::send(
        &a.router,
        Method::POST,
        "/guilds",
        Some(&owner),
        Some(&json!({ "name": "Bus" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let gid = guild["id"].as_str().unwrap().to_string();
    let (st, _, detail) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let cid = detail["channels"][0]["id"].as_str().unwrap().to_string();

    // Subscribe BEFORE broadcasting.
    let (st, _h, mut body) = common::open_sse(&a.router, "/events", Some(&owner)).await;
    assert_eq!(st, StatusCode::OK);

    let result = broadcast_system_message(&a.state, "the bus hears Nova")
        .await
        .expect("broadcast");
    assert_eq!(result.messages_sent, 1);

    let ev = match common::next_sse_data(&mut body, std::time::Duration::from_secs(3)).await {
        common::SseRead::Data(v) => v,
        other => panic!("expected message_created over SSE, got {other:?}"),
    };
    assert_eq!(ev["type"], "message_created");
    assert_eq!(
        ev["channel_id"],
        cid.as_str(),
        "the event names the guild's default channel the broadcast landed in"
    );
}

/// M6/P3: pin the wire contract the Nova DOT orb renders on. `system_message_meta`
/// (shell/channel/meta.rs) swaps the `.nova-orb` brand SVG in for `chat_avatar`
/// keyed PURELY on `kind=='system'`, and falls the author name back to "Nova DOT"
/// because a system message wears NO persona. If a future change ever attached a
/// persona to a broadcast (or dropped `kind='system'`), the orb path would render
/// the wrong avatar/name — this guards that the broadcast envelope stays
/// orb-shaped: system kind, no persona snapshot, Nova DOT display.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn system_message_envelope_carries_the_nova_orb_render_contract() {
    let a = common::arena().await;
    let state = AppState::new(a.db.clone(), a.media_dir.clone());

    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let (st, _, guild) = common::send(
        &a.router,
        Method::POST,
        "/guilds",
        Some(&owner),
        Some(&json!({ "name": "Guild" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let gid = guild["id"].as_str().unwrap().to_string();
    let (st, _, detail) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let cid = detail["channels"][0]["id"].as_str().unwrap().to_string();

    let result = broadcast_system_message(&state, "Nova surveys the channel")
        .await
        .expect("broadcast");
    assert_eq!(result.messages_sent, 1);

    let (st, _, body) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let msg = &body["messages"].as_array().unwrap()[0];

    // The orb branch keys on this exact shape:
    assert_eq!(
        msg["kind"], "system",
        "the orb is rendered iff kind=='system'"
    );
    assert_eq!(
        msg["author_display"], "Nova DOT",
        "no persona ⇒ display_name falls back to the bot account → the orb's label"
    );
    assert!(
        msg.get("persona_name").map_or(true, |v| v.is_null()),
        "a system broadcast wears NO persona (else the row would show a persona avatar, not the orb)"
    );
    assert!(
        msg.get("persona_avatar_id").map_or(true, |v| v.is_null()),
        "no frozen persona snapshot on a system message"
    );
}

// ---------------------------------------------------------------------------
// Admin gate — fail-closed (empty admin set authorizes no one)
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
#[tokio::test]
async fn system_broadcast_is_403_for_non_admin_and_writes_nothing() {
    let a = common::arena().await;
    let user = common::register_account(&a.router, "User", "password123").await;

    // A guild + channel that WOULD receive a broadcast if the gate failed open.
    a.db.query(
        "CREATE account:o SET username='O', username_ci='owner-seed', password_hash='x';\
         CREATE guild:g SET name='G', owner=account:o;\
         CREATE channel:c SET guild=guild:g, name='general', kind='text', position=0;",
    )
    .await
    .expect("seed transport")
    .check()
    .expect("seed");

    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        "/admin/system-message",
        Some(&user),
        Some(&json!({ "body": "hello" })),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::FORBIDDEN,
        "no admins configured → every caller is non-admin → 403"
    );

    #[derive(SurrealValue)]
    struct IdRow {
        id_key: String,
    }
    let mut resp =
        a.db.query("SELECT meta::id(id) AS id_key FROM message WHERE kind = 'system';")
            .await
            .expect("query")
            .check()
            .expect("check");
    let rows: Vec<IdRow> = resp.take(0).expect("take");
    assert!(rows.is_empty(), "403 means the broadcast fan-out never ran");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn system_broadcast_requires_auth() {
    let a = common::arena().await;
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        "/admin/system-message",
        None,
        Some(&json!({ "body": "hi" })),
    )
    .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}
