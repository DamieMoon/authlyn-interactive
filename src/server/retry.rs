//! SurrealDB write-conflict retry helper, shared between `server::keys` and
//! `server::keyshare` (and future Megolm-rotation work in step 6).
//!
//! Step 3 (`172c777`) introduced this pattern in `server::keys` with the
//! explicit note that "steps 5 + 6 will copy this pattern for room
//! key-share + Megolm rotation." Step 5 hoists it here so the retry policy
//! is defined in exactly one place and every consumer shares the same
//! backoff schedule, attempt cap, and write-conflict matcher.

/// Cap on how many times we'll re-issue a SurrealDB statement that got
/// rejected with a retryable write conflict. 5 attempts gives us linear
/// backoff up to ~25ms + jitter — well under any meaningful client timeout,
/// but enough headroom for the contention windows we've seen in concurrent
/// claims (a fraction of a millisecond each).
const MAX_WRITE_CONFLICT_ATTEMPTS: u32 = 5;

/// Base unit of the backoff schedule. Attempt `n` waits
/// `BASE_BACKOFF_MS * n + jitter` ms before retrying.
const BASE_BACKOFF_MS: u64 = 5;

/// Run `op`, retrying when SurrealDB tells us the transaction lost an MVCC
/// race. Other errors propagate immediately.
///
/// The closure is invoked up to [`MAX_WRITE_CONFLICT_ATTEMPTS`] times.
/// Backoff is linear in attempt number plus a small jitter so concurrent
/// retriers desynchronise instead of stampeding the next slot together.
pub async fn with_write_conflict_retry<F, Fut, T>(mut op: F) -> surrealdb::Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = surrealdb::Result<T>>,
{
    let mut last_err: Option<surrealdb::Error> = None;
    for attempt in 1..=MAX_WRITE_CONFLICT_ATTEMPTS {
        match op().await {
            Ok(v) => return Ok(v),
            Err(e) if is_write_conflict(&e) && attempt < MAX_WRITE_CONFLICT_ATTEMPTS => {
                // `rand::random::<u64>()` is good enough for jitter; no need
                // for a cryptographic source here.
                let jitter = rand::random::<u64>() % 20;
                let backoff_ms = BASE_BACKOFF_MS * attempt as u64 + jitter;
                tracing::debug!(
                    attempt,
                    backoff_ms,
                    error = %e,
                    "SurrealDB write conflict, retrying"
                );
                tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                last_err = Some(e);
                continue;
            }
            Err(e) => return Err(e),
        }
    }
    Err(last_err.expect("loop body always sets last_err before falling through"))
}

/// Identify SurrealDB write-conflict errors via their Display string. The
/// SDK exposes them as plain `surrealdb::Error` values rather than a typed
/// variant, so substring matching is the cheapest reliable test. Both the
/// "Write conflict" and the "retry the transaction" markers appear in the
/// 3.x error text we've observed (full message:
/// `"Query not executed: Transaction conflict: Write conflict, retry the
/// transaction. This transaction can be retried"`).
///
/// Exposed as `pub` (not `pub(crate)`) so the
/// `is_write_conflict_matches_real_surrealdb_conflict` regression test in
/// `tests/keys.rs` can call it directly. Integration tests are compiled as
/// a separate crate, so `pub(crate)` would not be reachable. That canary
/// synthesises a real MVCC conflict against the dev DB and asserts this
/// predicate still fires; a future SurrealDB rename of either substring
/// would silently disable the retry loop everywhere without any compile-
/// time signal, so the canary is load-bearing.
pub fn is_write_conflict(err: &surrealdb::Error) -> bool {
    let s = err.to_string();
    s.contains("Write conflict") || s.contains("retry the transaction")
}

/// Identify SurrealDB UNIQUE-index violation errors via their Display
/// string. SurrealDB 3.1.0-beta.3 surfaces these as plain
/// [`surrealdb::Error`] values whose message is shaped like
/// `"Database index `<index_name>` already contains <key_tuple>, with
/// record `<table>:<existing_id>`"` — empirically captured against
/// `prekey_otk` (`otk_lookup` index, `(device, kid)` UNIQUE) when issuing
/// two CREATEs with the same key tuple from the same connection. The
/// `"already contains"` substring is the load-bearing marker.
///
/// Step 7's `POST /rooms/{id}/join` handler maps this to `409 "user is
/// already a member"` for the concurrent-inviter race: two inviters racing
/// to add the same target survive their respective pre-checks (the row
/// genuinely doesn't exist yet from either snapshot's point of view), then
/// MVCC arbitrates. Under the canary probe one racer surfaces a
/// [`is_write_conflict`] which retries against a fresh snapshot, observes
/// the winner's row, and surfaces this UNIQUE violation. The two
/// predicates' substrings are disjoint by inspection (`"Write conflict"` /
/// `"retry the transaction"` vs `"already contains"`), so neither matcher
/// fires on the other's error.
///
/// Exposed as `pub` so the `is_unique_violation_matches_real_surrealdb_violation`
/// canary in `tests/keys.rs` can call it. Integration tests compile as a
/// separate crate, so `pub(crate)` would not reach. The canary synthesises
/// a real UNIQUE collision against the dev DB and asserts this predicate
/// still fires; if SurrealDB renames the message in a future release the
/// 409 path silently degrades to a 500 (the retry loop would surface the
/// raw error), so the canary is mandatory.
///
/// Mapping happens *outside* [`with_write_conflict_retry`] — UNIQUE
/// violations are not retryable: re-issuing the same CREATE against the
/// same key tuple just fails the same way. The handler runs the retry,
/// then inspects the residual error.
pub fn is_unique_violation(err: &surrealdb::Error) -> bool {
    // `surrealdb::Error` in 3.1.0-beta.3 does NOT expose a structured
    // `IndexExists` variant on the public client-side enum — `Debug` of a
    // real UNIQUE error showed `Error { code: -32000, message: "...", details:
    // Internal, cause: None }`, no enum discriminator we can match on
    // without going through `error::Db` (which lives behind the embedded
    // engine, not the WS client we use). Substring on `Display` it is.
    err.to_string().contains("already contains")
}
