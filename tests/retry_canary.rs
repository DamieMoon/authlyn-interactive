//! Regression canaries pinning the SurrealDB error-string matchers in
//! `server::retry` against REAL SurrealDB errors.
//!
//! These are load-bearing. `is_write_conflict` / `is_unique_violation`
//! substring-match the SDK's `Display` text (SurrealDB `=3.1.0-beta.3` exposes
//! no typed variant), so a future error-text rename would silently disable the
//! write-conflict retry loop and degrade every UNIQUE-violation 409 to a 500 —
//! with no compile-time signal. Each canary synthesises the real error and
//! asserts the matcher still fires, surfacing the live `Display` string in the
//! failure message so a renamed text is immediately visible.
//!
//! Ported from the retired `tests/keys.rs` (removed in the E2EE pivot,
//! `793b119`). The synthesis is schema-decoupled on purpose: both canaries use
//! their own throwaway tables, not production indexes, so they validate the
//! matchers regardless of how the app schema evolves.
//!
//! These hit a real SurrealDB. Run `./scripts/dev-db.sh` first.

mod common;

#[cfg(feature = "ssr")]
use std::sync::atomic::Ordering;

#[cfg(feature = "ssr")]
use surrealdb::engine::remote::ws::{Client, Ws};
#[cfg(feature = "ssr")]
use surrealdb::opt::auth::Root;
#[cfg(feature = "ssr")]
use surrealdb::Surreal;

#[cfg(feature = "ssr")]
use authlyn_interactive::server::retry::{is_unique_violation, is_write_conflict};

#[cfg(feature = "ssr")]
use common::{arena, NS_COUNTER};

/// Pin [`is_write_conflict`] against a REAL SurrealDB write-conflict error.
///
/// **How the conflict is synthesised:** several raw `Surreal<Client>`
/// connections race `UPDATE`s on the same throwaway row. SurrealDB's MVCC
/// arbiter commits one winner per cycle and rejects the losers with text
/// containing `"Write conflict"` / `"retry the transaction"`. If this ever
/// fails to observe a conflict at all, that itself is a signal (the synth or
/// the SDK's MVCC behaviour changed) — investigate before shipping.
///
/// Multi-thread runtime so the spawned transaction futures run on separate
/// worker threads; `current_thread` would serialise them and shrink the
/// contention window. Fanning out wide (and a 50-attempt cap) re-amortises the
/// residual scheduling risk under load from peer test binaries.
#[cfg(feature = "ssr")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn is_write_conflict_matches_real_surrealdb_conflict() {
    use std::sync::Arc;

    // Per-test ns/db owned by this test. We bypass `arena()` because we need
    // two+ parallel `Surreal<Client>` handles into the SAME ns/db.
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

    // Setup: a one-row table every racer will UPDATE.
    let setup = fresh_conn(&ns, &db_name).await;
    setup
        .query(
            "DEFINE TABLE IF NOT EXISTS conflict_canary SCHEMAFULL; \
             DEFINE FIELD IF NOT EXISTS v ON conflict_canary TYPE int;",
        )
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

    const FANOUT: usize = 10;
    let mut conns: Vec<Arc<Surreal<Client>>> = Vec::with_capacity(FANOUT);
    for _ in 0..FANOUT {
        conns.push(Arc::new(fresh_conn(&ns, &db_name).await));
    }

    let conflict_err: surrealdb::Error = 'find: {
        for _attempt in 0..50 {
            let q = "BEGIN TRANSACTION; \
                     UPDATE type::record('conflict_canary', '1') SET v = $v; \
                     COMMIT TRANSACTION;";
            let mut handles = Vec::with_capacity(FANOUT);
            for (i, conn) in conns.iter().enumerate() {
                let d = conn.clone();
                let v = (i + 1) as i64;
                handles.push(tokio::spawn(async move {
                    d.query(q).bind(("v", v)).await.and_then(|r| r.check())
                }));
            }
            for h in handles {
                let r = h.await.expect("racer join");
                if let Err(e) = r {
                    break 'find e;
                }
            }
        }
        panic!(
            "SurrealDB no longer synthesizes a write conflict across 50 attempts of \
             {FANOUT} parallel transactions updating the same row — the conflict synth \
             pattern is broken, so the canary can no longer guard the matcher. Investigate."
        );
    };

    assert!(
        is_write_conflict(&conflict_err),
        "is_write_conflict() returned false for a real SurrealDB write conflict. The \
         error's Display string was: '{conflict_err}'. SurrealDB likely renamed its \
         error text; update both substrings in src/server/retry.rs::is_write_conflict \
         (and audit every caller of with_write_conflict_retry)."
    );
}

/// Pin [`is_unique_violation`] against a REAL SurrealDB UNIQUE-index violation,
/// and confirm [`is_write_conflict`] does NOT also fire on it (else the retry
/// loop would retry an unretryable error and 500 instead of mapping to 409).
///
/// **How the violation is synthesised:** two `CREATE`s with the same key on a
/// throwaway table bearing a UNIQUE index; the second surfaces an error whose
/// `Display` contains `"already contains"`.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn is_unique_violation_matches_real_surrealdb_violation() {
    let arena = arena().await;

    // Throwaway table + UNIQUE index in this arena's isolated db.
    arena
        .db
        .query(
            "DEFINE TABLE IF NOT EXISTS unique_canary SCHEMAFULL; \
             DEFINE FIELD IF NOT EXISTS k ON unique_canary TYPE string; \
             DEFINE INDEX IF NOT EXISTS unique_canary_k ON unique_canary FIELDS k UNIQUE;",
        )
        .await
        .expect("define unique table")
        .check()
        .expect("define unique table check");

    // First CREATE — succeeds.
    arena
        .db
        .query("CREATE unique_canary SET k = 'dup';")
        .await
        .expect("first insert")
        .check()
        .expect("first insert check");

    // Second CREATE with the same key — must surface a UNIQUE violation.
    let second = arena
        .db
        .query("CREATE unique_canary SET k = 'dup';")
        .await
        .expect("send query")
        .check();
    let err = match second {
        Ok(_) => panic!(
            "second CREATE with a duplicate UNIQUE key succeeded — SurrealDB no longer \
             enforces the UNIQUE index, or DEFINE INDEX semantics changed. The canary \
             can no longer guard the matcher; investigate before shipping."
        ),
        Err(e) => e,
    };

    assert!(
        is_unique_violation(&err),
        "is_unique_violation() returned false for a real SurrealDB UNIQUE-index \
         violation. The error's Display string was: '{err}'. SurrealDB likely renamed \
         its error text; update the substring in src/server/retry.rs::is_unique_violation \
         (and audit the 409/idempotent paths: account, guild_member, persona_editor, \
         friendship, push_subscription, custom_emoji)."
    );

    assert!(
        !is_write_conflict(&err),
        "is_write_conflict() falsely fired on a UNIQUE-index violation: '{err}'. The two \
         predicate substrings are no longer disjoint; rework both predicates in \
         src/server/retry.rs together."
    );
}
