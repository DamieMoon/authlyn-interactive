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
/// ASSERT guard as `channel.kind`: 'user'/'system'/'roll' (M4/T6) accepted,
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

/// M4/T6 (Fate Engine): the widened `message.kind` enum (`'roll'` added) must
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

/// M4/T5 (message effects): adding the `option<string>` `effect` field to the
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

/// M6/P2 schema guard: `account.display_name` + `account.avatar` already exist
/// and are NONE-safe, so re-applying the schema over a legacy account row
/// (predating both) must NOT crash-loop; the `display_name` backfill must
/// materialize `''` (so a later account UPDATE doesn't hit the SCHEMAFULL
/// NONE-coercion 500), and `avatar` reads back NONE. Documents that M6/P2 adds
/// no account schema. Mirrors the accent/effect over-populated guards above.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn applying_schema_over_legacy_account_backfills_display_name_and_keeps_avatar_none() {
    let db = common::raw_db().await;

    // A minimal pre-display_name / pre-avatar account schema.
    db.query(
        "DEFINE TABLE account SCHEMAFULL;\
         DEFINE FIELD username ON account TYPE string;\
         DEFINE FIELD username_ci ON account TYPE string;\
         DEFINE FIELD password_hash ON account TYPE string;\
         DEFINE FIELD created_at ON account TYPE datetime DEFAULT time::now();",
    )
    .await
    .expect("old schema transport")
    .check()
    .expect("old schema apply");

    // A populated legacy account holding NONE in the not-yet-defined fields.
    db.query(
        "CREATE account:legacy SET username = 'Legacy', username_ci = 'legacy', password_hash = 'x';",
    )
    .await
    .expect("seed legacy transport")
    .check()
    .expect("seed legacy account");

    // Apply the REAL schema over the populated table: adds display_name (+ its
    // backfill), avatar (option), the security fields, the UNIQUE index, and the
    // nova_dot seed. Must not crash-loop.
    db.query(authlyn_interactive::storage::SCHEMA)
        .await
        .expect("apply real schema transport")
        .check()
        .expect("apply real schema over a legacy account");

    #[derive(SurrealValue)]
    struct Row {
        username: String,
        display_name: String,
        avatar_id: Option<String>,
    }
    let mut resp = db
        .query(
            "SELECT username, display_name,
                (IF avatar != NONE THEN meta::id(avatar) ELSE NONE END) AS avatar_id
                FROM account:legacy;",
        )
        .await
        .expect("query")
        .check()
        .expect("check");
    let row: Row = resp
        .take::<Option<Row>>(0)
        .expect("take")
        .expect("legacy account survives the migration");
    assert_eq!(row.username, "Legacy", "username survives untouched");
    assert_eq!(
        row.display_name, "",
        "display_name backfilled to '' (NONE-coercion guard)"
    );
    assert_eq!(
        row.avatar_id, None,
        "avatar reads back NONE on a legacy row"
    );

    // The guard's point: a subsequent account UPDATE must not 500 on the
    // previously-NONE display_name (this is the change-profile / change-password
    // path on a legacy account).
    db.query(
        "UPDATE account:legacy SET display_name = 'Renamed';\
         SELECT VALUE display_name FROM account:legacy;",
    )
    .await
    .expect("update transport")
    .check()
    .expect("updating display_name on a backfilled legacy account must be accepted");
}

/// M6 security fix (be2fb18, account-takeover purge): the self-service
/// recovery fields `security_question` + `security_answer_hash` were dropped.
/// On prod they exist as POPULATED `option<string>` fields, so the schema purge
/// must, over a populated table, (1) `UPDATE … UNSET` the stale values and
/// (2) `REMOVE FIELD` the definitions WITHOUT crash-looping boot — and that
/// full-table account UPDATE must run AFTER the `display_name` backfill, or it
/// 500s the SCHEMAFULL NONE-coercion on a pre-`display_name` row (the load-
/// bearing ordering claim in `schema.surql:33-34`). This seeds the hardest prod
/// shape — a row that BOTH predates `display_name` AND holds security values —
/// so a future reorder of those two statements regresses here, not on prod boot.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn applying_schema_over_account_with_security_fields_purges_them_without_crashing() {
    let db = common::raw_db().await;

    // Pre-fix account schema: NO `display_name` (forces the backfill), WITH the
    // two security fields defined as `option<string>` — exactly what the purge
    // must remove on prod.
    db.query(
        "DEFINE TABLE account SCHEMAFULL;\
         DEFINE FIELD username ON account TYPE string;\
         DEFINE FIELD username_ci ON account TYPE string;\
         DEFINE FIELD password_hash ON account TYPE string;\
         DEFINE FIELD created_at ON account TYPE datetime DEFAULT time::now();\
         DEFINE FIELD security_question ON account TYPE option<string>;\
         DEFINE FIELD security_answer_hash ON account TYPE option<string>;",
    )
    .await
    .expect("old schema transport")
    .check()
    .expect("old schema apply");

    // A populated legacy account that actually HOLDS recovery credentials — the
    // row the purge has to clean (display_name is NONE here: pre-display_name).
    db.query(
        "CREATE account:legacy SET username = 'Legacy', username_ci = 'legacy', \
         password_hash = 'x', security_question = 'first pet', \
         security_answer_hash = 'argon2-of-the-answer';",
    )
    .await
    .expect("seed legacy transport")
    .check()
    .expect("seed legacy account with security fields");

    // Apply the REAL schema: backfills display_name, then UNSETs + REMOVEs the
    // security fields over the populated table. Must not crash-loop (the boot
    // path `.expect()`s this in main.rs — a failure panics the server).
    db.query(authlyn_interactive::storage::SCHEMA)
        .await
        .expect("apply real schema transport")
        .check()
        .expect("apply real schema over an account holding security fields");

    // The field DEFINITIONS are gone (REMOVE FIELD landed).
    let mut resp = db
        .query("LET $i = (INFO FOR TABLE account); RETURN object::keys($i.fields);")
        .await
        .expect("info query")
        .check()
        .expect("info check");
    let fields: Vec<String> = resp.take(1).expect("take field names");
    assert!(
        !fields.contains(&"security_question".to_string())
            && !fields.contains(&"security_answer_hash".to_string()),
        "both security fields must be removed from the schema, got: {fields:?}"
    );

    // The row survived and display_name backfilled to '' — proving the backfill
    // ran BEFORE the security UNSET (else that UPDATE 500s the NONE display_name).
    #[derive(SurrealValue)]
    struct Row {
        username: String,
        display_name: String,
        password_hash: String,
    }
    let mut resp = db
        .query("SELECT username, display_name, password_hash FROM account:legacy;")
        .await
        .expect("query")
        .check()
        .expect("check");
    let row: Row = resp
        .take::<Option<Row>>(0)
        .expect("take")
        .expect("legacy account survives the purge migration");
    assert_eq!(row.username, "Legacy", "username survives untouched");
    assert_eq!(row.password_hash, "x", "password_hash survives untouched");
    assert_eq!(
        row.display_name, "",
        "display_name backfilled to '' (purge ran after the NONE-coercion guard)"
    );

    // The stale recovery values are unreachable: on a SCHEMAFULL table the
    // removed field can no longer be written, so a stray reset attempt errors
    // rather than re-populating a recovery credential.
    let revived = db
        .query("UPDATE account:legacy SET security_answer_hash = 'sneaky';")
        .await
        .expect("transport");
    assert!(
        revived.check().is_err(),
        "the removed security field must reject writes (SCHEMAFULL undefined-field)"
    );

    // The change-password path (a revalidating account UPDATE) still succeeds
    // post-purge — no 500 on the migrated row.
    db.query("UPDATE account:legacy SET password_hash = 'rotated';")
        .await
        .expect("update transport")
        .check()
        .expect("change-password on a purged legacy account must be accepted");
}

/// M4/T5: the `message.effect` enum guard — `option<string>` with
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

/// M5 review M-37: the `guild_member_account` index (the account-only lookup
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

/// M7/P1: a DM thread is a `channel` with `guild = NONE` and `kind = 'dm'`. The
/// schema widens `channel.guild` `record<guild>` → `option<record<guild>>` and
/// the `channel.kind` ASSERT `['text','lorebook']` → `+['dm']`, BOTH via
/// `DEFINE FIELD OVERWRITE`. On a populated prod DB the fields already exist with
/// the strict definitions, so `IF NOT EXISTS` would be a no-op that keeps them —
/// and then `CREATE channel SET guild = NONE, kind = 'dm'` (every DM) dies on the
/// strict `record<guild>` type and the narrow enum ASSERT. This applies the real
/// `storage::SCHEMA` over an old-strict populated channel and proves: legacy rows
/// survive with their guild + kind, AND a guild-less `kind='dm'` channel is now
/// accepted (the assertion that fails if either OVERWRITE were `IF NOT EXISTS`).
#[cfg(feature = "ssr")]
#[tokio::test]
async fn widening_channel_guild_to_option_over_populated_channels_admits_guildless_dms() {
    let db = common::raw_db().await;

    // The pre-M7 channel schema: `guild` STRICT (non-option), `kind` with the
    // OLD two-value ASSERT — exactly prod's state at deploy time.
    db.query(
        "DEFINE TABLE channel SCHEMAFULL;\
         DEFINE FIELD guild      ON channel TYPE record<guild>;\
         DEFINE FIELD name       ON channel TYPE string;\
         DEFINE FIELD kind       ON channel TYPE string DEFAULT 'text' ASSERT $value IN ['text', 'lorebook'];\
         DEFINE FIELD position   ON channel TYPE int DEFAULT 0;\
         DEFINE FIELD created_at ON channel TYPE datetime DEFAULT time::now();\
         DEFINE FIELD deleted_at ON channel TYPE option<datetime>;\
         DEFINE INDEX channel_guild ON channel FIELDS guild;",
    )
    .await
    .expect("old schema transport")
    .check()
    .expect("old schema apply");

    // A populated legacy guild channel (record links are not referentially
    // enforced, so the dangling guild id is fine).
    db.query("CREATE channel:legacy SET guild = guild:x, name = 'general', kind = 'text';")
        .await
        .expect("seed legacy transport")
        .check()
        .expect("seed legacy channel");

    // Apply the REAL schema: OVERWRITEs guild → option<> and kind → +'dm' over the
    // populated table. Must not crash-loop (existing rows hold a guild + a valid
    // kind, both still valid; no backfill).
    db.query(authlyn_interactive::storage::SCHEMA)
        .await
        .expect("apply real schema transport")
        .check()
        .expect("apply real schema over a strict-guild populated channel table");

    #[derive(SurrealValue)]
    struct Row {
        guild_id: Option<String>,
        name: String,
        kind: String,
    }
    let mut resp = db
        .query(
            "SELECT (IF guild != NONE THEN meta::id(guild) ELSE NONE END) AS guild_id, name, kind
                FROM channel:legacy;",
        )
        .await
        .expect("query")
        .check()
        .expect("check");
    let row: Row = resp
        .take::<Option<Row>>(0)
        .expect("take")
        .expect("legacy channel survives the widening");
    assert_eq!(
        row.guild_id,
        Some("x".to_string()),
        "legacy channel keeps its guild link"
    );
    assert_eq!(row.name, "general", "name survives untouched");
    assert_eq!(row.kind, "text", "kind survives untouched");

    // THE BITE: a guild-less DM channel must now be accepted. This fails if the
    // strict `record<guild>` survived (NONE rejected) OR the narrow kind ASSERT
    // survived ('dm' rejected) — i.e. if either OVERWRITE had been IF NOT EXISTS.
    let mut resp = db
        .query(
            "CREATE channel:dm SET guild = NONE, kind = 'dm', name = '';\
             SELECT (IF guild != NONE THEN meta::id(guild) ELSE NONE END) AS guild_id, name, kind
                FROM channel:dm;",
        )
        .await
        .expect("dm create transport")
        .check()
        .expect("a guild-less kind='dm' channel must be accepted after the widening");
    let row: Row = resp
        .take::<Option<Row>>(1)
        .expect("take dm")
        .expect("dm channel row");
    assert_eq!(row.guild_id, None, "DM channel has no guild");
    assert_eq!(row.kind, "dm", "DM channel kind persists");

    // And a normal guild channel still validates (the enum widening didn't break
    // the existing values).
    db.query("CREATE channel:legacy2 SET guild = guild:x, name = 'two', kind = 'lorebook';")
        .await
        .expect("transport")
        .check()
        .expect("an existing kind value must stay accepted after the widening");

    // An out-of-set kind is still rejected.
    let bad = db
        .query("CREATE channel:bad SET guild = guild:x, name = 'bad', kind = 'bogus';")
        .await
        .expect("transport");
    assert!(
        bad.check().is_err(),
        "channel.kind ASSERT must still reject an out-of-set value"
    );
}

/// M7/P1 (mirror review M-37 / `guild_member_account`): the account-only
/// `dm_member_account` index — `access::visible_channels` asks "which DM threads
/// is this account in?" on every /events connect and GET /unread — must land on a
/// database whose `dm_member` table is ALREADY POPULATED. `DEFINE INDEX` builds
/// over existing rows at apply, so the apply must not error, the index must exist
/// afterwards, and the lookup must serve the pre-existing rows through it.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn new_dm_member_account_index_applies_over_populated_rows() {
    let db = common::raw_db().await;

    // The pre-index dm_member schema: every real field, but ONLY the
    // (channel, account) composite — no account-only index.
    db.query(
        "DEFINE TABLE dm_member SCHEMAFULL;\
         DEFINE FIELD channel   ON dm_member TYPE record<channel>;\
         DEFINE FIELD account   ON dm_member TYPE record<account>;\
         DEFINE FIELD joined_at ON dm_member TYPE datetime DEFAULT time::now();\
         DEFINE INDEX dm_member_pair ON dm_member FIELDS channel, account UNIQUE;",
    )
    .await
    .expect("old schema transport")
    .check()
    .expect("old schema apply");

    // Populated DM membership rows (record links are not referentially enforced).
    db.query(
        "CREATE dm_member SET channel = channel:t1, account = account:alpha;\
         CREATE dm_member SET channel = channel:t2, account = account:alpha;\
         CREATE dm_member SET channel = channel:t1, account = account:beta;",
    )
    .await
    .expect("seed transport")
    .check()
    .expect("seed dm_member rows");

    // Apply the REAL schema: must not error, and must add the new index.
    db.query(authlyn_interactive::storage::SCHEMA)
        .await
        .expect("apply real schema transport")
        .check()
        .expect("apply real schema over populated dm_member");

    let mut resp = db
        .query("LET $i = (INFO FOR TABLE dm_member); RETURN object::keys($i.indexes);")
        .await
        .expect("info query")
        .check()
        .expect("info check");
    let indexes: Vec<String> = resp.take(1).expect("take index names");
    assert!(
        indexes.contains(&"dm_member_account".to_string()),
        "the account-only index must exist after apply, got: {indexes:?}"
    );

    // The visible_channels lookup shape serves the pre-existing rows.
    let mut resp = db
        .query(
            "SELECT VALUE meta::id(channel) FROM dm_member
                WHERE account = type::record('account', $account);",
        )
        .bind(("account", "alpha".to_string()))
        .await
        .expect("lookup query")
        .check()
        .expect("lookup check");
    let mut threads: Vec<String> = resp.take(0).expect("take threads");
    threads.sort();
    assert_eq!(
        threads,
        vec!["t1".to_string(), "t2".to_string()],
        "pre-existing DM memberships are served through the new index"
    );
}

/// M7/P1 review H1: the `dm_pair` 1:1 dedup lock. Its `dm_pair_key` UNIQUE index
/// is the single arbiter that makes concurrent 1:1 creates converge — so apply
/// must (a) create the index, (b) the index must actually reject a duplicate
/// `pair_key` (the property the race-safe create_dm relies on), and (c) re-apply
/// idempotently over an already-populated dm_pair table (it builds over existing
/// rows). Because dm_pair only ever holds distinct non-NONE keys, the index never
/// hits the repeated-NONE collision a channel.pair_key column would.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn dm_pair_unique_index_rejects_duplicate_pair_and_reapplies() {
    let db = common::raw_db().await;

    // First apply creates dm_pair + the UNIQUE index.
    db.query(authlyn_interactive::storage::SCHEMA)
        .await
        .expect("first apply transport")
        .check()
        .expect("first apply");

    let mut resp = db
        .query("LET $i = (INFO FOR TABLE dm_pair); RETURN object::keys($i.indexes);")
        .await
        .expect("info query")
        .check()
        .expect("info check");
    let indexes: Vec<String> = resp.take(1).expect("take index names");
    assert!(
        indexes.contains(&"dm_pair_key".to_string()),
        "the UNIQUE dedup index must exist after apply, got: {indexes:?}"
    );

    // One row per pair is fine.
    db.query("CREATE dm_pair SET pair_key = 'alpha\u{1f}beta', channel = channel:t1;")
        .await
        .expect("first pair transport")
        .check()
        .expect("first pair inserts");

    // The SAME pair_key (a second thread racing the same pair) must be rejected.
    let dup = db
        .query("CREATE dm_pair SET pair_key = 'alpha\u{1f}beta', channel = channel:t2;")
        .await
        .expect("dup transport");
    assert!(
        dup.check().is_err(),
        "dm_pair_key UNIQUE must reject a duplicate pair_key (the dedup arbiter)"
    );

    // Re-apply over the populated table: idempotent, no crash, index intact.
    db.query(authlyn_interactive::storage::SCHEMA)
        .await
        .expect("re-apply transport")
        .check()
        .expect("re-apply over populated dm_pair");

    // A DIFFERENT pair still inserts after re-apply.
    db.query("CREATE dm_pair SET pair_key = 'gamma\u{1f}delta', channel = channel:t3;")
        .await
        .expect("third pair transport")
        .check()
        .expect("a distinct pair still inserts after re-apply");
}

/// M7/P2 (mirror `dm_member_account`): the account-only `channel_guest_account`
/// index — `access::visible_channels` + GET /cameos ask "which channels is this
/// account a guest in?" on every /events connect and GET /unread — must land on a
/// database whose `channel_guest` table is ALREADY POPULATED. `DEFINE INDEX` builds
/// over existing rows at apply, so the apply must not error, the index must exist
/// afterwards, and the lookup must serve the pre-existing rows through it.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn new_channel_guest_account_index_applies_over_populated_rows() {
    let db = common::raw_db().await;

    // The pre-index channel_guest schema: every real field, but ONLY the
    // (channel, account) composite — no account-only index.
    db.query(
        "DEFINE TABLE channel_guest SCHEMAFULL;\
         DEFINE FIELD channel    ON channel_guest TYPE record<channel>;\
         DEFINE FIELD account    ON channel_guest TYPE record<account>;\
         DEFINE FIELD invited_by ON channel_guest TYPE record<account>;\
         DEFINE FIELD created_at ON channel_guest TYPE datetime DEFAULT time::now();\
         DEFINE FIELD expires_at ON channel_guest TYPE option<datetime>;\
         DEFINE INDEX channel_guest_pair ON channel_guest FIELDS channel, account UNIQUE;",
    )
    .await
    .expect("old schema transport")
    .check()
    .expect("old schema apply");

    // Populated guest rows (record links are not referentially enforced).
    db.query(
        "CREATE channel_guest SET channel = channel:c1, account = account:guest1, invited_by = account:host;\
         CREATE channel_guest SET channel = channel:c2, account = account:guest1, invited_by = account:host;\
         CREATE channel_guest SET channel = channel:c1, account = account:guest2, invited_by = account:host;",
    )
    .await
    .expect("seed transport")
    .check()
    .expect("seed channel_guest rows");

    // Apply the REAL schema: must not error, and must add the new index.
    db.query(authlyn_interactive::storage::SCHEMA)
        .await
        .expect("apply real schema transport")
        .check()
        .expect("apply real schema over populated channel_guest");

    let mut resp = db
        .query("LET $i = (INFO FOR TABLE channel_guest); RETURN object::keys($i.indexes);")
        .await
        .expect("info query")
        .check()
        .expect("info check");
    let indexes: Vec<String> = resp.take(1).expect("take index names");
    assert!(
        indexes.contains(&"channel_guest_account".to_string()),
        "the account-only index must exist after apply, got: {indexes:?}"
    );

    // The visible_channels / list_cameos lookup shape serves the pre-existing rows.
    let mut resp = db
        .query(
            "SELECT VALUE meta::id(channel) FROM channel_guest
                WHERE account = type::record('account', $account);",
        )
        .bind(("account", "guest1".to_string()))
        .await
        .expect("lookup query")
        .check()
        .expect("lookup check");
    let mut channels: Vec<String> = resp.take(0).expect("take channels");
    channels.sort();
    assert_eq!(
        channels,
        vec!["c1".to_string(), "c2".to_string()],
        "pre-existing guest invites are served through the new index"
    );
}

/// M7/P2: the new non-option `message.guest_cameo` bool added to the POPULATED
/// `message` table must be materialised in the SINGLE first backfill statement
/// (alongside attachments/pinged_users/kind), NEVER a separate UPDATE — a separate
/// UPDATE revalidates the whole SCHEMAFULL row and trips the still-NONE arrays,
/// crash-looping apply (the 2026-06-01 attachments-wipe lesson). The bite: apply
/// the real schema over a legacy row that predates `guest_cameo`; the legacy
/// attachments + kind must survive untouched and guest_cameo must read back false.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn applying_guest_cameo_over_populated_messages_materialises_without_wiping_attachments() {
    let db = common::raw_db().await;

    // A pre-`guest_cameo` message schema: every NONE-coercion-sensitive field
    // (both non-option arrays + kind) plus the basics, so re-applying the full
    // schema introduces ONLY `guest_cameo` over a populated row.
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

    // A populated legacy row (record links are not referentially enforced).
    db.query(
        "CREATE message:legacy SET channel = channel:x, author = account:y, \
         body = 'hi', attachments = ['keep-this-blob'], kind = 'user';",
    )
    .await
    .expect("seed legacy transport")
    .check()
    .expect("seed legacy row");

    // Apply the REAL schema: introduces `guest_cameo`, runs the modified first
    // backfill. Must not crash-loop on the still-NONE field.
    db.query(authlyn_interactive::storage::SCHEMA)
        .await
        .expect("apply real schema transport")
        .check()
        .expect("apply real schema");

    #[derive(SurrealValue)]
    struct Row {
        attachments: Vec<String>,
        kind: String,
        guest_cameo: bool,
    }
    let mut resp = db
        .query("SELECT attachments, kind, guest_cameo FROM message:legacy;")
        .await
        .expect("query")
        .check()
        .expect("check");
    let row: Option<Row> = resp.take(0).expect("take");
    let row = row.expect("legacy message row survives the guest_cameo migration");
    assert_eq!(
        row.attachments,
        vec!["keep-this-blob".to_string()],
        "attachments must NOT be wiped by the guest_cameo backfill"
    );
    assert_eq!(row.kind, "user", "legacy kind survives untouched");
    assert!(
        !row.guest_cameo,
        "guest_cameo must be materialised to false on legacy rows"
    );
}

/// M7/P2 prod-shape DEPLOY GATE: the two single-table guards above each prove ONE
/// migration in isolation over a tiny seed. This rehearses the actual prod boot —
/// the FULL real schema applied over a DB already populated with prod-shaped legacy
/// rows across the M7/P1+P2 risk surface AT ONCE, then re-applied (boot-restart
/// idempotency). It covers what the isolated guards don't: (1) the
/// `message.guest_cameo` fold over MANY legacy rows (scale, not 1 row), (2) the
/// `channel_guest_account` index build AND the `channel.guild`/`channel.kind`
/// OVERWRITE widenings co-applied in the same pass over populated rows, (3) a
/// SECOND apply over the now-migrated populated DB stays Ok (a non-idempotent
/// DEFINE/backfill would crash-loop the service on restart). A NONE-coercion crash
/// (the 2026-06-01 attachments-wipe hazard) surfaces as a `.check()` Err.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn applying_full_schema_over_prod_shaped_populated_db_is_crash_free_and_idempotent() {
    let db = common::raw_db().await;

    // The on-disk shape a PRE-M7 prod DB carries before the boot-apply migrates it:
    // channel with the OLD strict `guild` (record<guild>, not option) and the OLD
    // narrow `kind` ASSERT (no 'dm'); message WITHOUT `guest_cameo`; channel_guest
    // WITHOUT the account-only index. Record links are not referentially enforced,
    // so the linked account/guild rows need not exist.
    db.query(
        "DEFINE TABLE channel SCHEMAFULL;\
         DEFINE FIELD guild      ON channel TYPE record<guild>;\
         DEFINE FIELD name       ON channel TYPE string;\
         DEFINE FIELD kind       ON channel TYPE string DEFAULT 'text' ASSERT $value IN ['text', 'lorebook'];\
         DEFINE FIELD position   ON channel TYPE int DEFAULT 0;\
         DEFINE FIELD created_at ON channel TYPE datetime DEFAULT time::now();\
         DEFINE INDEX channel_guild ON channel FIELDS guild;\
         DEFINE TABLE message SCHEMAFULL;\
         DEFINE FIELD channel      ON message TYPE record<channel>;\
         DEFINE FIELD author       ON message TYPE record<account>;\
         DEFINE FIELD body         ON message TYPE string;\
         DEFINE FIELD attachments  ON message TYPE array<string> DEFAULT [];\
         DEFINE FIELD pinged_users ON message TYPE array<record<account>> DEFAULT [];\
         DEFINE FIELD kind         ON message TYPE string DEFAULT 'user';\
         DEFINE FIELD tier         ON message TYPE string DEFAULT 'default';\
         DEFINE FIELD sent_at      ON message TYPE datetime DEFAULT time::now();\
         DEFINE INDEX message_channel_sent ON message FIELDS channel, sent_at;\
         DEFINE TABLE channel_guest SCHEMAFULL;\
         DEFINE FIELD channel    ON channel_guest TYPE record<channel>;\
         DEFINE FIELD account    ON channel_guest TYPE record<account>;\
         DEFINE FIELD invited_by ON channel_guest TYPE record<account>;\
         DEFINE FIELD created_at ON channel_guest TYPE datetime DEFAULT time::now();\
         DEFINE FIELD expires_at ON channel_guest TYPE option<datetime>;\
         DEFINE INDEX channel_guest_pair ON channel_guest FIELDS channel, account UNIQUE;",
    )
    .await
    .expect("pre-M7 schema transport")
    .check()
    .expect("pre-M7 schema apply");

    // Prod-shaped seed: 5 populated channels (guild set, kind 'text' — the OVERWRITE
    // targets), 100 legacy messages each carrying an attachment and lacking
    // guest_cameo (the realistic prod state: prior backfills already ran, only
    // guest_cameo is new), and 3 channel_guest rows lacking the account-only index.
    let mut seed = String::new();
    for i in 0..5 {
        seed.push_str(&format!(
            "CREATE channel:c{i} SET guild = guild:g, name = 'chan {i}', kind = 'text';"
        ));
    }
    for i in 0..100 {
        seed.push_str(&format!(
            "CREATE message:m{i} SET channel = channel:c{c}, author = account:y, \
             body = 'legacy {i}', attachments = ['blob-{i}'], kind = 'user';",
            c = i % 5
        ));
    }
    seed.push_str(
        "CREATE channel_guest SET channel = channel:c0, account = account:guest1, invited_by = account:host;\
         CREATE channel_guest SET channel = channel:c1, account = account:guest1, invited_by = account:host;\
         CREATE channel_guest SET channel = channel:c0, account = account:guest2, invited_by = account:host;",
    );
    db.query(seed)
        .await
        .expect("seed transport")
        .check()
        .expect("seed prod-shaped rows");

    // THE GATE — first boot-apply over the populated pre-M7 DB. This is exactly the
    // boot path `db::apply_schema` (db.rs) → `main.rs`'s `.expect()`; a crash here
    // is a boot crash-loop in prod.
    db.query(authlyn_interactive::storage::SCHEMA)
        .await
        .expect("first apply transport")
        .check()
        .expect("FIRST apply of real schema over prod-shaped populated DB");

    // Boot-restart idempotency — the same schema re-runs over the now-migrated DB.
    db.query(authlyn_interactive::storage::SCHEMA)
        .await
        .expect("second apply transport")
        .check()
        .expect("SECOND apply (boot-restart idempotency) over the migrated DB");

    // Message migration over MANY rows: every legacy message kept its attachment and
    // got guest_cameo = false (no wipe, no crash). Counted in Rust to dodge any
    // aggregation-idiom footgun.
    #[derive(SurrealValue)]
    struct Msg {
        attachments: Vec<String>,
        guest_cameo: bool,
    }
    let mut resp = db
        .query("SELECT attachments, guest_cameo FROM message;")
        .await
        .expect("message query")
        .check()
        .expect("message check");
    let msgs: Vec<Msg> = resp.take(0).expect("take messages");
    assert_eq!(
        msgs.len(),
        100,
        "all 100 legacy messages survive the migration"
    );
    assert!(
        msgs.iter().all(|m| !m.attachments.is_empty()),
        "no attachment wiped by the guest_cameo backfill (the 2026-06-01 hazard)"
    );
    assert!(
        msgs.iter().all(|m| !m.guest_cameo),
        "guest_cameo backfilled to false on every legacy row"
    );

    // channel.guild / channel.kind OVERWRITE widenings landed over populated rows.
    #[derive(SurrealValue)]
    struct Chan {
        kind: String,
    }
    let mut resp = db
        .query("SELECT kind FROM channel;")
        .await
        .expect("channel query")
        .check()
        .expect("channel check");
    let chans: Vec<Chan> = resp.take(0).expect("take channels");
    assert_eq!(
        chans.len(),
        5,
        "all channels survive the guild/kind OVERWRITE widening"
    );
    assert!(
        chans.iter().all(|c| c.kind == "text"),
        "kind survives the ASSERT-widening OVERWRITE untouched"
    );

    // The new account-only index exists and serves the pre-existing guest rows.
    let mut resp = db
        .query("LET $i = (INFO FOR TABLE channel_guest); RETURN object::keys($i.indexes);")
        .await
        .expect("info query")
        .check()
        .expect("info check");
    let indexes: Vec<String> = resp.take(1).expect("take index names");
    assert!(
        indexes.contains(&"channel_guest_account".to_string()),
        "the account-only index must exist after apply, got: {indexes:?}"
    );
    let mut resp = db
        .query(
            "SELECT VALUE meta::id(channel) FROM channel_guest
                WHERE account = type::record('account', $account);",
        )
        .bind(("account", "guest1".to_string()))
        .await
        .expect("lookup query")
        .check()
        .expect("lookup check");
    let mut channels: Vec<String> = resp.take(0).expect("take channels");
    channels.sort();
    assert_eq!(
        channels,
        vec!["c0".to_string(), "c1".to_string()],
        "pre-existing guest invites are served through the new index"
    );
}
