//! SurrealDB write-conflict retry helper, shared by every handler that issues
//! a racy CREATE against a UNIQUE index — registration (`account`), guild
//! membership (`guild_member`), persona editors (`persona_editor`), friendships
//! (`friendship`), custom emoji (`custom_emoji`), per-channel persona wear
//! (`channel_active_persona`), and push subscriptions (`push_subscription`).
//!
//! Centralising the policy here means every consumer shares one backoff
//! schedule, one attempt cap, and the same write-conflict / UNIQUE-violation
//! matchers — so inv13 (a racy CREATE resolves to an idempotent 409, never a
//! 500) is realised in exactly one place.

/// Cap on how many times we'll re-issue a SurrealDB statement that got
/// rejected with a retryable write conflict. With 5 attempts we sleep between
/// the first four — the 5th attempt's error returns without sleeping (the loop
/// guard is `attempt < MAX_WRITE_CONFLICT_ATTEMPTS`) — so the linear
/// `BASE_BACKOFF_MS · n + jitter` schedule is a deterministic floor of
/// 5+10+15+20 = 50ms across those four sleeps, up to ~126ms with maximum jitter
/// (4 × up-to-19ms). Still well under any meaningful client timeout, but enough
/// headroom for the sub-millisecond contention windows we've seen in concurrent
/// claims.
const MAX_WRITE_CONFLICT_ATTEMPTS: u32 = 5;

/// Base unit of the backoff schedule. Attempt `n` waits
/// `BASE_BACKOFF_MS * n + jitter` ms before retrying.
const BASE_BACKOFF_MS: u64 = 5;

/// Run `op`, retrying when SurrealDB tells us the transaction lost an MVCC
/// race. Other errors propagate immediately.
///
/// The closure is invoked up to `MAX_WRITE_CONFLICT_ATTEMPTS` times.
/// Backoff is linear in attempt number plus a small jitter so concurrent
/// retriers desynchronise instead of stampeding the next slot together.
pub async fn with_write_conflict_retry<F, Fut, T>(mut op: F) -> surrealdb::Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = surrealdb::Result<T>>,
{
    let mut last_err: Option<surrealdb::Error> = None;
    let mut first_err: Option<String> = None;
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
                first_err.get_or_insert_with(|| e.to_string());
                last_err = Some(e);
                continue;
            }
            Err(e) => {
                if is_write_conflict(&e) {
                    // Retries exhausted (review M-33). Either genuine
                    // contention outlasted the backoff, or a
                    // permanently-failing transaction matched the 3.1.x
                    // generic abort text (see `is_write_conflict`) and was
                    // replayed — the root cause then sits on the failing
                    // statement's result row, which `Response::check()`
                    // discarded, so it is only visible in the DB's own logs.
                    // Logging the first error alongside the residual at least
                    // surfaces drift between attempts.
                    tracing::warn!(
                        attempts = attempt,
                        first_error = first_err.as_deref().unwrap_or("<first attempt>"),
                        residual = %e,
                        "write-conflict retries exhausted; surfacing the residual error"
                    );
                }
                return Err(e);
            }
        }
    }
    Err(last_err.expect("loop body always sets last_err before falling through"))
}

/// Identify SurrealDB write-conflict errors via their Display string. The
/// SDK exposes them as plain `surrealdb::Error` values rather than a typed
/// variant, so substring matching is the cheapest reliable test. The exact
/// wording drifts between SurrealDB releases, so we match CASE-INSENSITIVELY on
/// markers from EVERY text we've observed: the `=3.1.0-beta.3` SDK
/// emits `"...Transaction conflict: Write conflict, retry the transaction. This
/// transaction can be retried"`, the `3.0.4` server (prod/fenrir) emits
/// `"...Transaction conflict: Transaction write conflict. This transaction
/// can be retried"`, and the `3.1.3` server (dev box since 2026-06) emits
/// `"The query was not executed due to a failed transaction"`. The first two
/// contain `"write conflict"` and `"can be retried"`; the third contains
/// neither, so the full generic sentence is matched as the third marker (the
/// FULL sentence — not a loose `"failed transaction"` substring — so an error
/// that merely mentions the phrase, e.g. a THROWN message echoing user data,
/// can never trip the retry loop; review M-33). All three markers are absent
/// from the UNIQUE-violation text (`"already contains"`), so the two
/// predicates stay disjoint (the `is_unique_violation` canary asserts this
/// against the live server).
///
/// **Accepted false-positive class (review M-33).** On 3.1.x the third
/// marker's sentence is the generic SIBLING-statement text for ANY aborted
/// multi-statement transaction, not only MVCC conflicts: when a statement
/// inside BEGIN/COMMIT fails permanently (e.g. a future ASSERT rejection),
/// the server rewrites every non-failing statement's result row to that
/// sentence, parks the root cause on the failing row, and puts the only
/// distinguishing text on the COMMIT row (`"Cannot COMMIT: Transaction
/// conflict: … can be retried"` for a genuine conflict vs `"Cannot COMMIT:
/// the transaction was aborted due to a prior error"`). `Response::check()`
/// surfaces the FIRST error by statement index — the generic sibling — so at
/// this layer a permanently-failing transaction is byte-identical to a
/// genuine commit-time conflict and gets replayed 4 extra times (~50–126 ms)
/// with the root cause masked in OUR logs (it stays visible in SurrealDB's).
/// This is accepted because no text-level narrowing can keep the genuine
/// 3.1.x conflict matched while excluding the impostor; a real fix would
/// have consumers inspect the full per-statement `Response` instead of
/// `check()`. Bounded by design: every transactional consumer (read_state,
/// personas/wear, personas/gallery, push) is idempotent DELETE+CREATE
/// shaped, and `is_unique_violation` is checked on the residual error, so no
/// 409 degrades to a 500. Pinned by
/// `aborted_transaction_sibling_text_is_indistinguishable_from_a_write_conflict`
/// in `tests/retry_canary.rs` — if that canary's first assertion ever fails,
/// the texts became distinguishable and this marker should be narrowed.
///
/// Exposed as `pub` (not `pub(crate)`) so the
/// `is_write_conflict_matches_real_surrealdb_conflict` regression test in
/// `tests/retry_canary.rs` can call it directly. Integration tests are compiled
/// as a separate crate, so `pub(crate)` would not be reachable. That canary
/// synthesises a real MVCC conflict against the dev DB and asserts this
/// predicate still fires; a future SurrealDB rename of BOTH markers would
/// silently disable the retry loop everywhere without any compile-time signal,
/// so the canary is load-bearing.
pub fn is_write_conflict(err: &surrealdb::Error) -> bool {
    let s = err.to_string().to_ascii_lowercase();
    s.contains("write conflict")
        || s.contains("can be retried")
        || s.contains("the query was not executed due to a failed transaction")
}

/// Identify SurrealDB UNIQUE-index violation errors via their Display
/// string. SurrealDB surfaces these as plain [`surrealdb::Error`] values
/// whose message is shaped like `Database index <index_name> already
/// contains <key_tuple>, with record <table>:<existing_id>` — captured
/// against `guild_member_pair` (`(guild, account)` UNIQUE) when two CREATEs
/// race the same key tuple. The `"already contains"` substring is the
/// load-bearing marker.
///
/// `invite_member` (`guilds/membership.rs`) maps this to `409 "user is
/// already a member"` for the concurrent-inviter race: two inviters racing
/// to add the same target both pass their pre-check (the row genuinely
/// doesn't exist yet from either snapshot), then MVCC arbitrates. One racer
/// surfaces an [`is_write_conflict`], retries against a fresh snapshot,
/// observes the winner's row, and surfaces this UNIQUE violation. The same
/// idempotent-409 shape covers registration, persona editors, friendships,
/// custom emoji, and push subscriptions. The two predicates' substrings are
/// disjoint by inspection (`"write conflict"` / `"can be retried"` vs
/// `"already contains"`), so neither matcher fires on the other's error.
///
/// Exposed as `pub` so the `is_unique_violation_matches_real_surrealdb_violation`
/// canary in `tests/retry_canary.rs` can call it. Integration tests compile
/// as a separate crate, so `pub(crate)` would not reach. The canary
/// synthesises a real UNIQUE collision against the dev DB and asserts this
/// predicate still fires; if SurrealDB renames the message in a future
/// release the 409 path silently degrades to a 500 (the retry loop would
/// surface the raw error), so the canary is mandatory.
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
