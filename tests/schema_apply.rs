//! Step-0 smoke: the phase-1 schema applies cleanly on the pinned SurrealDB
//! (3.1.0-beta.3), and the `ASSERT $value IN [...]` enum guard on
//! `channel.kind` actually rejects out-of-set values on that beta.

mod common;

#[cfg(feature = "ssr")]
use surrealdb::types::SurrealValue;

#[cfg(feature = "ssr")]
#[tokio::test]
async fn schema_applies_and_kind_guard_holds() {
    // arena() applies storage::SCHEMA and `.check()`s it — a rejected DEFINE
    // (e.g. an ASSERT syntax the beta won't parse) panics here.
    let arena = common::arena().await;

    // Seed an account + guild so the channel's record link points at a real row.
    arena
        .db
        .query(
            "CREATE account:a SET username = 'A', username_ci = 'a', password_hash = 'x';\
             CREATE guild:g SET name = 'G', owner = account:a;",
        )
        .await
        .expect("seed transport")
        .check()
        .expect("account + guild insert");

    // A valid channel kind is accepted.
    arena
        .db
        .query("CREATE channel:c1 SET guild = guild:g, name = 'general', kind = 'text';")
        .await
        .expect("transport")
        .check()
        .expect("kind='text' must be accepted");

    // An out-of-set kind must be rejected by the ASSERT guard.
    let bad = arena
        .db
        .query("CREATE channel:c2 SET guild = guild:g, name = 'bad', kind = 'bogus';")
        .await
        .expect("transport");
    assert!(
        bad.check().is_err(),
        "channel.kind ASSERT must reject an out-of-set value"
    );
}

/// `message.kind` (the system-vs-user discriminator) carries the same enum
/// ASSERT guard as `channel.kind`: 'user'/'system' accepted, anything else
/// rejected on the pinned beta.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn message_kind_guard_accepts_user_and_system_rejects_other() {
    let arena = common::arena().await;
    arena
        .db
        .query(
            "CREATE account:a SET username = 'A', username_ci = 'a', password_hash = 'x';\
             CREATE guild:g SET name = 'G', owner = account:a;\
             CREATE channel:c SET guild = guild:g, name = 'general', kind = 'text';",
        )
        .await
        .expect("seed transport")
        .check()
        .expect("account + guild + channel insert");

    for k in ["user", "system"] {
        arena
            .db
            .query(format!(
                "CREATE message SET channel = channel:c, author = account:a, body = 'hi', kind = '{k}';"
            ))
            .await
            .expect("transport")
            .check()
            .unwrap_or_else(|e| panic!("message.kind = '{k}' must be accepted: {e}"));
    }

    let bad = arena
        .db
        .query("CREATE message SET channel = channel:c, author = account:a, body = 'hi', kind = 'bogus';")
        .await
        .expect("transport");
    assert!(
        bad.check().is_err(),
        "message.kind ASSERT must reject an out-of-set value"
    );
}

/// REGRESSION GUARD (2026-06-01 attachments-wipe shape): adding the SCHEMAFULL
/// `kind` field to the populated `message` table must materialise it to 'user'
/// on legacy rows WITHOUT wiping their `attachments`. The fix folds `kind` into
/// the single existing first backfill with a `?? ` coalesce; this test applies
/// the real `storage::SCHEMA` over a row created under a pre-`kind` schema and
/// asserts attachments survive + kind is materialised.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn applying_kind_over_populated_messages_materialises_without_wiping_attachments() {
    let db = common::raw_db().await;

    // A minimal pre-`kind` message schema: every real message field EXCEPT
    // `kind`, so re-applying the full schema introduces only `kind`.
    db.query(
        "DEFINE TABLE message SCHEMAFULL;\
         DEFINE FIELD channel ON message TYPE record<channel>;\
         DEFINE FIELD author ON message TYPE record<account>;\
         DEFINE FIELD body ON message TYPE string;\
         DEFINE FIELD attachments ON message TYPE array<string> DEFAULT [];\
         DEFINE FIELD pinged_users ON message TYPE array<record<account>> DEFAULT [];\
         DEFINE FIELD tier ON message TYPE string DEFAULT 'default';\
         DEFINE FIELD sent_at ON message TYPE datetime DEFAULT time::now();",
    )
    .await
    .expect("old schema transport")
    .check()
    .expect("old schema apply");

    // A populated legacy row (record links are not referentially enforced, so
    // the dangling channel/author ids are fine).
    db.query(
        "CREATE message:legacy SET channel = channel:x, author = account:y, \
         body = 'hi', attachments = ['keep-this-blob'];",
    )
    .await
    .expect("seed legacy transport")
    .check()
    .expect("seed legacy row");

    // Apply the REAL schema: adds `kind`, runs the modified first backfill, etc.
    db.query(authlyn_interactive::storage::SCHEMA)
        .await
        .expect("apply real schema transport")
        .check()
        .expect("apply real schema");

    #[derive(SurrealValue)]
    struct Row {
        attachments: Vec<String>,
        kind: String,
    }
    let mut resp = db
        .query("SELECT attachments, kind FROM message:legacy;")
        .await
        .expect("query")
        .check()
        .expect("check");
    let row: Option<Row> = resp.take(0).expect("take");
    let row = row.expect("legacy message row survives the migration");
    assert_eq!(
        row.attachments,
        vec!["keep-this-blob".to_string()],
        "attachments must NOT be wiped by the kind backfill"
    );
    assert_eq!(
        row.kind, "user",
        "kind must be materialised to 'user' on legacy rows"
    );
}

/// The reserved Nova DOT bot account (author of all `kind='system'` messages) is
/// seeded by `storage::SCHEMA`, and login as it is impossible (sentinel password
/// hash → 401, never 500 — see `crypto::verify_on_blocking_pool`).
#[cfg(feature = "ssr")]
#[tokio::test]
async fn nova_dot_system_account_is_seeded_and_cannot_log_in() {
    let arena = common::arena().await;

    #[derive(SurrealValue)]
    struct Row {
        display_name: String,
        username_ci: String,
    }
    let mut resp = arena
        .db
        .query("SELECT display_name, username_ci FROM account:nova_dot;")
        .await
        .expect("query")
        .check()
        .expect("check");
    let row: Option<Row> = resp.take(0).expect("take");
    let row = row.expect("account:nova_dot must be seeded by storage::SCHEMA");
    assert_eq!(row.display_name, "Nova DOT", "bot renders as 'Nova DOT'");
    assert_eq!(
        row.username_ci, "nova-dot",
        "bot reserves the 'nova-dot' handle"
    );

    let (st, _, _) = common::send(
        &arena.router,
        axum::http::Method::POST,
        "/auth/login",
        None,
        Some(&serde_json::json!({ "username": "nova-dot", "password": "anything-at-all" })),
    )
    .await;
    assert_eq!(
        st,
        axum::http::StatusCode::UNAUTHORIZED,
        "the Nova DOT bot account cannot be logged into"
    );
}
