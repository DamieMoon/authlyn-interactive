//! SurrealDB connection + schema-apply path (ssr graph) — the sole consumer of
//! [`crate::storage::SCHEMA`].
//!
//! [`connect`] opens a WebSocket [`Surreal`] handle (`SURREAL_URL`, default
//! `127.0.0.1:8000`), signs in as Root, and selects the namespace/database
//! (`SURREAL_NS`/`SURREAL_DB`, default `authlyn`/`dev`); [`connect_with_retries`]
//! wraps it in a bounded backoff for boot races against a not-yet-ready DB.
//! [`apply_schema`] runs the entire embedded schema as one multi-statement query
//! and `.check()`s it, so any rejected `DEFINE` or failed backfill aborts boot
//! rather than serving against a half-migrated DB — the migration discipline that
//! keeps this crash-free + idempotent over a populated prod DB is pinned by
//! `tests/schema_apply.rs::applying_full_schema_over_prod_shaped_populated_db_is_crash_free_and_idempotent`.

use std::env;
use std::future::Future;
use std::time::Duration;

use surrealdb::engine::remote::ws::{Client, Ws};
use surrealdb::opt::auth::Root;
use surrealdb::Surreal;

use crate::storage;

/// Open one WebSocket SurrealDB connection: Root signin, then select
/// `SURREAL_NS`/`SURREAL_DB` (default `authlyn`/`dev`). Reads `SURREAL_URL`
/// (default `127.0.0.1:8000`) and `SURREAL_USER`/`SURREAL_PASS` (default
/// `root`/`root`); a `ws://`/`wss://` scheme prefix on the URL is stripped since
/// the `Ws` engine wants a bare host.
pub async fn connect() -> surrealdb::Result<Surreal<Client>> {
    let url = env::var("SURREAL_URL").unwrap_or_else(|_| "127.0.0.1:8000".into());
    let user = env::var("SURREAL_USER").unwrap_or_else(|_| "root".into());
    let pass = env::var("SURREAL_PASS").unwrap_or_else(|_| "root".into());
    let ns = env::var("SURREAL_NS").unwrap_or_else(|_| "authlyn".into());
    let db_name = env::var("SURREAL_DB").unwrap_or_else(|_| "dev".into());

    let host = url.trim_start_matches("ws://").trim_start_matches("wss://");
    let db = Surreal::new::<Ws>(host).await?;
    db.signin(Root {
        username: user,
        password: pass,
    })
    .await?;
    db.use_ns(ns).use_db(db_name).await?;
    Ok(db)
}

/// Apply the entire embedded [`crate::storage::SCHEMA`] as a single
/// multi-statement query and `.check()` it. `.check()` surfaces any rejected
/// `DEFINE` or failed backfill UPDATE as an `Err`, so `main` aborts boot instead
/// of serving against a half-migrated DB. Idempotent: safe to re-run on every
/// boot over an already-populated database (statement order in `schema.surql` is
/// load-bearing — backfills precede row-revalidating UPDATEs).
pub async fn apply_schema(db: &Surreal<Client>) -> surrealdb::Result<()> {
    db.query(storage::SCHEMA).await?.check()?;
    Ok(())
}

async fn retry<F, Fut, T, E>(mut op: F, max_attempts: u32, backoff: Duration) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let mut last_err: Option<E> = None;
    for attempt in 1..=max_attempts {
        match op().await {
            Ok(v) => return Ok(v),
            Err(e) => {
                eprintln!("attempt {attempt}/{max_attempts}: {e}");
                last_err = Some(e);
                if attempt < max_attempts {
                    tokio::time::sleep(backoff).await;
                }
            }
        }
    }
    Err(last_err.expect("retry called with max_attempts >= 1"))
}

/// [`connect`] with bounded backoff (10 attempts, 500 ms apart) to ride out the
/// boot race where the app starts before SurrealDB is accepting connections.
/// Returns the last connect error if every attempt fails.
pub async fn connect_with_retries() -> surrealdb::Result<Surreal<Client>> {
    retry(connect, 10, Duration::from_millis(500)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn retry_succeeds_after_transient_failures() {
        let counter = Arc::new(AtomicU32::new(0));
        let result: Result<i32, &'static str> = retry(
            || {
                let counter = counter.clone();
                async move {
                    let n = counter.fetch_add(1, Ordering::SeqCst);
                    if n < 3 {
                        Err("not yet")
                    } else {
                        Ok(42)
                    }
                }
            },
            5,
            Duration::from_millis(1),
        )
        .await;

        assert_eq!(result, Ok(42));
        assert_eq!(counter.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn retry_returns_last_error_on_exhaustion() {
        let counter = Arc::new(AtomicU32::new(0));
        let result: Result<i32, &'static str> = retry(
            || {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Err("never")
                }
            },
            3,
            Duration::from_millis(1),
        )
        .await;

        assert_eq!(result, Err("never"));
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }
}
