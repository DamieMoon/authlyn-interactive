//! Step-0 smoke: the phase-1 schema applies cleanly on the pinned SurrealDB
//! (3.1.0-beta.3), and the `ASSERT $value IN [...]` enum guard on
//! `channel.kind` actually rejects out-of-set values on that beta.

mod common;

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
