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

/// `message.kind` (the user/system/roll discriminator) carries the same enum
/// ASSERT guard as `channel.kind`: 'user'/'system'/'roll' (W4/T6) accepted,
/// anything else rejected on the pinned beta.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn message_kind_guard_accepts_known_set_rejects_other() {
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

    for k in ["user", "system", "roll"] {
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

/// W4/T6 (Fate Engine): the widened `message.kind` enum (`'roll'` added) must
/// reach a database whose `kind` field ALREADY EXISTS with the old two-value
/// ASSERT — exactly prod's state at deploy time. A `DEFINE FIELD IF NOT
/// EXISTS` is a no-op there (the field exists, so the old ASSERT silently
/// survives and every `/roll` insert dies on it); the definition must be
/// re-applied (OVERWRITE) for the widening to land. This applies the real
/// `storage::SCHEMA` over an old-ASSERT populated table and proves a
/// `kind='roll'` row is then accepted (and legacy rows survive untouched).
#[cfg(feature = "ssr")]
#[tokio::test]
async fn widened_kind_assert_reaches_a_db_where_kind_already_exists() {
    let db = common::raw_db().await;

    // The pre-T6 message schema: `kind` already defined with the OLD
    // two-value ASSERT, plus the fields whose NONE-coercion matters.
    db.query(
        "DEFINE TABLE message SCHEMAFULL;\
         DEFINE FIELD channel ON message TYPE record<channel>;\
         DEFINE FIELD author ON message TYPE record<account>;\
         DEFINE FIELD body ON message TYPE string;\
         DEFINE FIELD attachments ON message TYPE array<string> DEFAULT [];\
         DEFINE FIELD pinged_users ON message TYPE array<record<account>> DEFAULT [];\
         DEFINE FIELD kind ON message TYPE string DEFAULT 'user' \
             ASSERT $value IN ['user', 'system'];\
         DEFINE FIELD tier ON message TYPE string DEFAULT 'default';\
         DEFINE FIELD sent_at ON message TYPE datetime DEFAULT time::now();",
    )
    .await
    .expect("old schema transport")
    .check()
    .expect("old schema apply");

    // A populated legacy row (record links are not referentially enforced).
    db.query(
        "CREATE message:legacy SET channel = channel:x, author = account:y, \
         body = 'hi', attachments = ['keep-this-blob'];",
    )
    .await
    .expect("seed legacy transport")
    .check()
    .expect("seed legacy row");

    // Apply the REAL schema over the already-defined field.
    db.query(authlyn_interactive::storage::SCHEMA)
        .await
        .expect("apply real schema transport")
        .check()
        .expect("apply real schema");

    // The widened enum must now hold: 'roll' accepted AND persisted.
    let mut resp = db
        .query(
            "CREATE message:fate SET channel = channel:x, author = account:y, \
             body = '1d6 → [4] = 4', kind = 'roll';\
             SELECT VALUE kind FROM message:fate;",
        )
        .await
        .expect("roll insert transport")
        .check()
        .expect("kind='roll' must be accepted after the re-apply");
    let kinds: Vec<String> = resp.take(1).expect("take kind");
    assert_eq!(kinds, vec!["roll".to_string()], "roll kind persists");

    // And the legacy row survived the apply untouched.
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
    let row = row.expect("legacy message row survives the re-apply");
    assert_eq!(row.attachments, vec!["keep-this-blob".to_string()]);
    assert_eq!(row.kind, "user", "legacy kind backfill still lands");
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

/// Task 0: `guild.accent_color` is `option<string>` added to the populated
/// `guild` table. Applying the real schema over a pre-existing guild row must
/// NOT crash-loop (no backfill needed for option<>), the legacy row reads back
/// accent_color = NONE, and a value written after apply persists (proving it's
/// a real defined field, not silently stripped by SCHEMAFULL). Mirrors
/// `applying_effect_over_populated_messages_keeps_legacy_rows_with_effect_none`.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn applying_accent_color_over_populated_guilds_keeps_legacy_rows_with_accent_none() {
    let db = common::raw_db().await;

    // A minimal pre-`accent_color` guild schema: every guild field EXCEPT
    // accent_color, so re-applying the full schema introduces only it.
    db.query(
        "DEFINE TABLE guild SCHEMAFULL;\
         DEFINE FIELD name ON guild TYPE string;\
         DEFINE FIELD owner ON guild TYPE record<account>;\
         DEFINE FIELD icon ON guild TYPE option<record<media_blob>>;\
         DEFINE FIELD created_at ON guild TYPE datetime DEFAULT time::now();\
         DEFINE FIELD deleted_at ON guild TYPE option<datetime>;",
    )
    .await
    .expect("old schema transport")
    .check()
    .expect("old schema apply");

    // A populated legacy guild (record links are not referentially enforced, so
    // the dangling owner id is fine).
    db.query("CREATE guild:legacy SET name = 'G', owner = account:y;")
        .await
        .expect("seed legacy transport")
        .check()
        .expect("seed legacy guild");

    // Apply the REAL schema: adds accent_color (option<string>) over the
    // populated table. Must not crash-loop — no backfill for option<>.
    db.query(authlyn_interactive::storage::SCHEMA)
        .await
        .expect("apply real schema transport")
        .check()
        .expect("apply real schema");

    #[derive(SurrealValue)]
    struct Row {
        name: String,
        accent_color: Option<String>,
    }
    let mut resp = db
        .query("SELECT name, accent_color FROM guild:legacy;")
        .await
        .expect("query")
        .check()
        .expect("check");
    let row: Option<Row> = resp.take(0).expect("take");
    let row = row.expect("legacy guild row survives the migration");
    assert_eq!(row.name, "G", "name must survive the apply untouched");
    assert_eq!(
        row.accent_color, None,
        "legacy guilds read back accent_color = NONE"
    );

    // Existence probe: an accent written AFTER the apply must persist.
    let mut resp = db
        .query(
            "UPDATE guild:legacy SET accent_color = 'purple';\
             SELECT VALUE accent_color FROM guild:legacy;",
        )
        .await
        .expect("update transport")
        .check()
        .expect("updating a legacy guild with an accent must be accepted");
    let accents: Vec<Option<String>> = resp.take(1).expect("take accent");
    assert_eq!(
        accents,
        vec![Some("purple".to_string())],
        "accent_color must be a real defined field that persists, not silently stripped"
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

/// W5 review M-37: the `guild_member_account` index (the account-only lookup
/// `access::visible_channels` runs on every /events connect, ListsChanged
/// visibility reload, and GET /unread) must land on a database whose
/// `guild_member` table is ALREADY POPULATED — exactly prod's state at deploy
/// time. `DEFINE INDEX` builds over existing rows at apply, so the apply must
/// not error, the index must exist afterwards, and the lookup must serve the
/// pre-existing rows through it.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn new_guild_member_account_index_applies_over_populated_rows() {
    let db = common::raw_db().await;

    // The pre-M-37 guild_member schema: every real field, but ONLY the
    // (guild, account) composite index — no account-only index.
    db.query(
        "DEFINE TABLE guild_member SCHEMAFULL;\
         DEFINE FIELD guild ON guild_member TYPE record<guild>;\
         DEFINE FIELD account ON guild_member TYPE record<account>;\
         DEFINE FIELD role ON guild_member TYPE string DEFAULT 'member' ASSERT $value IN ['owner', 'admin', 'member'];\
         DEFINE FIELD active_persona ON guild_member TYPE option<record<persona>>;\
         DEFINE FIELD joined_at ON guild_member TYPE datetime DEFAULT time::now();\
         DEFINE INDEX guild_member_pair ON guild_member FIELDS guild, account UNIQUE;",
    )
    .await
    .expect("old schema transport")
    .check()
    .expect("old schema apply");

    // Populated membership rows (record links are not referentially enforced,
    // so the dangling guild/account ids are fine).
    db.query(
        "CREATE guild_member SET guild = guild:g1, account = account:alpha;\
         CREATE guild_member SET guild = guild:g2, account = account:alpha;\
         CREATE guild_member SET guild = guild:g1, account = account:beta;",
    )
    .await
    .expect("seed transport")
    .check()
    .expect("seed membership rows");

    // Apply the REAL schema: must not error, and must add the new index.
    db.query(authlyn_interactive::storage::SCHEMA)
        .await
        .expect("apply real schema transport")
        .check()
        .expect("apply real schema over populated guild_member");

    let mut resp = db
        .query("LET $i = (INFO FOR TABLE guild_member); RETURN object::keys($i.indexes);")
        .await
        .expect("info query")
        .check()
        .expect("info check");
    let indexes: Vec<String> = resp.take(1).expect("take index names");
    assert!(
        indexes.contains(&"guild_member_account".to_string()),
        "the account-only index must exist after apply, got: {indexes:?}"
    );

    // The visible_channels lookup shape serves the pre-existing rows.
    let mut resp = db
        .query(
            "SELECT VALUE meta::id(guild) FROM guild_member
                WHERE account = type::record('account', $account);",
        )
        .bind(("account", "alpha".to_string()))
        .await
        .expect("lookup query")
        .check()
        .expect("lookup check");
    let mut guilds: Vec<String> = resp.take(0).expect("take guilds");
    guilds.sort();
    assert_eq!(
        guilds,
        vec!["g1".to_string(), "g2".to_string()],
        "pre-existing memberships are served through the new index"
    );
}
