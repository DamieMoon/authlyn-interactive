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

/// W4/T5 (message effects): adding the `option<string>` `effect` field to the
/// POPULATED `message` table must need NO backfill — NONE is a valid value for
/// an `option<>` field, so the existing first backfill stays untouched and a
/// legacy row survives the apply with `effect = NONE` and its other fields
/// intact. The post-apply UPDATE round-trip is the real existence probe: a
/// SCHEMAFULL table silently STRIPS undefined fields (it does not error), so
/// only a persisted read-back proves the field was actually defined.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn applying_effect_over_populated_messages_keeps_legacy_rows_with_effect_none() {
    let db = common::raw_db().await;

    // A minimal pre-`effect` message schema: the fields whose NONE-coercion
    // matters on re-validation (both non-option arrays + kind) plus the basics,
    // so re-applying the full schema introduces `effect` over a populated row.
    db.query(
        "DEFINE TABLE message SCHEMAFULL;\
         DEFINE FIELD channel ON message TYPE record<channel>;\
         DEFINE FIELD author ON message TYPE record<account>;\
         DEFINE FIELD body ON message TYPE string;\
         DEFINE FIELD attachments ON message TYPE array<string> DEFAULT [];\
         DEFINE FIELD pinged_users ON message TYPE array<record<account>> DEFAULT [];\
         DEFINE FIELD kind ON message TYPE string DEFAULT 'user';\
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

    // Apply the REAL schema: adds `effect` (option<string>) over the populated
    // table. Must not crash-loop — no backfill is required for option<>.
    db.query(authlyn_interactive::storage::SCHEMA)
        .await
        .expect("apply real schema transport")
        .check()
        .expect("apply real schema");

    #[derive(SurrealValue)]
    struct Row {
        body: String,
        attachments: Vec<String>,
        kind: String,
        effect: Option<String>,
    }
    let mut resp = db
        .query("SELECT body, attachments, kind, effect FROM message:legacy;")
        .await
        .expect("query")
        .check()
        .expect("check");
    let row: Option<Row> = resp.take(0).expect("take");
    let row = row.expect("legacy message row survives the migration");
    assert_eq!(row.body, "hi", "body must survive the apply untouched");
    assert_eq!(
        row.attachments,
        vec!["keep-this-blob".to_string()],
        "attachments must survive the apply untouched"
    );
    assert_eq!(row.kind, "user", "kind must survive the apply untouched");
    assert_eq!(row.effect, None, "legacy rows read back effect = NONE");

    // Existence probe: a valid effect written AFTER the apply must persist —
    // on a schema without the field, SCHEMAFULL strips it and this reads NONE.
    let mut resp = db
        .query(
            "UPDATE message:legacy SET effect = 'whisper';\
             SELECT VALUE effect FROM message:legacy;",
        )
        .await
        .expect("update transport")
        .check()
        .expect("updating a legacy row with a valid effect must be accepted");
    let effects: Vec<Option<String>> = resp.take(1).expect("take effect");
    assert_eq!(
        effects,
        vec![Some("whisper".to_string())],
        "effect must be a real defined field that persists, not silently stripped"
    );
}

/// W4/T5: the `message.effect` enum guard — `option<string>` with
/// `ASSERT $value = NONE OR $value IN ['whisper','shout','spell']` on the
/// pinned beta. NONE (the everyday effect-less send) and each known effect are
/// accepted AND persist; an out-of-set value is rejected.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn message_effect_guard_accepts_known_set_rejects_other() {
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

    for e in ["whisper", "shout", "spell"] {
        let mut resp = arena
            .db
            .query(format!(
                "CREATE message:{e} SET channel = channel:c, author = account:a, \
                 body = 'hi', effect = '{e}';\
                 SELECT VALUE effect FROM message:{e};"
            ))
            .await
            .expect("transport")
            .check()
            .unwrap_or_else(|err| panic!("message.effect = '{e}' must be accepted: {err}"));
        let got: Vec<Option<String>> = resp.take(1).expect("take effect");
        assert_eq!(
            got,
            vec![Some(e.to_string())],
            "effect '{e}' must persist (not be silently stripped by SCHEMAFULL)"
        );
    }

    arena
        .db
        .query("CREATE message:plain SET channel = channel:c, author = account:a, body = 'hi';")
        .await
        .expect("transport")
        .check()
        .expect("an effect-less message must stay accepted (option<> ⇒ NONE valid)");

    let bad = arena
        .db
        .query(
            "CREATE message:bad SET channel = channel:c, author = account:a, \
             body = 'hi', effect = 'sparkle-bomb';",
        )
        .await
        .expect("transport");
    assert!(
        bad.check().is_err(),
        "message.effect ASSERT must reject an out-of-set value"
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
