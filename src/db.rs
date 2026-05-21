use std::env;
use std::future::Future;
use std::time::Duration;

use surrealdb::engine::remote::ws::{Client, Ws};
use surrealdb::opt::auth::Root;
use surrealdb::Surreal;

use crate::storage;

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
