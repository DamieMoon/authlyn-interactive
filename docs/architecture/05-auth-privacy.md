# 05 — Auth & Privacy

Server-trusted username/password identity for `authlyn-interactive`, end to end: account creation, argon2id password hashing, opaque session-token mint/verify (SHA-256-at-rest), the typed cookie jar, the `AuthAccount` extractor that is the single identity chokepoint, env-driven fail-closed admin gating, password change / admin reset, and the two cross-cutting rules that every other subsystem inherits — **the WebKit `Secure`-cookie trap** and the **404-not-403 privacy posture**.

Stack, dependency purposes (argon2 / sha2 / rand / axum-extra cookie), and toolchain are in [`../../Cargo.toml`](../../Cargo.toml) and [`../../CLAUDE.md`](../../CLAUDE.md) — not repeated here. Conventions (handler naming, DTO suffixes, privacy-404) are in [`../reference/conventions.md`](../reference/conventions.md). Wire DTOs live in [`03-data-model.md`](03-data-model.md); the SSE stream that reuses these session primitives is [`04-realtime-sse.md`](04-realtime-sse.md).

## Graph placement

This subsystem spans all three disjoint feature graphs (the no-cross-import rule, [`01-overview.md`](01-overview.md)):

| Layer | Files | Graph |
| --- | --- | --- |
| Wire DTOs (always-on, wasm-safe, serde-only) | `src/protocol.rs` (auth block) | always-on |
| Server handlers + crypto + session | `src/server/auth/*`, `src/server/permissions.rs` | **ssr** (never wasm) |
| Browser REST wrappers + login/register UI | `src/client/api.rs`, `src/ui/auth.rs` | **hydrate** (never server) |

`src/ui/auth.rs` is *compiled* under both ssr and hydrate (it is part of the Leptos component tree), but its network logic is `#[cfg(feature = "hydrate")]` — under ssr the `submit` closures are inert and `auth` is `let _ =`'d to silence the unused warning. The DTOs in `protocol.rs` are the only types shared across the graph boundary; they must keep compiling to `wasm32-unknown-unknown` (serde-only, no axum/surrealdb/tokio).

## Trust model

Identity is a classic **server-side session**. The client never supplies its own account id; it is always re-derived server-side from the session cookie.

1. `POST /auth/register` and `POST /auth/login` mint a random opaque token (`random_token` — 32 bytes of `rand::thread_rng`, hex-encoded), store **only its SHA-256** in the `session` table (`session.token_hash`), and hand the raw token to the browser in an `HttpOnly; Secure; SameSite=Lax; Max-Age=30d` cookie named `authlyn_session`.
2. Every protected handler takes the `AuthAccount` extractor in its signature. The extractor reads the cookie, hashes it, looks up an unexpired session row, and yields the bare account key — or rejects with **401**.
3. The raw token leaves the server exactly once (the `Set-Cookie` on register/login). The DB only ever holds `SHA-256(token)`, so a DB read does not yield a usable credential.

```
register/login → random_token() → sha256_hex → CREATE session{token_hash, expires_at=now+30d}
                                                     │
                                  Set-Cookie: authlyn_session=<raw token>  (HttpOnly+Secure+Lax+30d)

authed request → AuthAccount extractor → CookieJar[authlyn_session]
                                       → session_token_hash(token)      (= sha256_hex)
                                       → account_for_token_hash(hash)    SELECT … WHERE token_hash=$h AND expires_at>now
                                       → Some(account_key) | None→401
```

`AuthAccount` is the single import that ~33 server modules rely on for identity (`cameos`, `dms`, `friends`, `guilds/*`, `lorebook`, `media`, `messages/*`, `personas/*`, `push`, `system_messages`, `feedback`, …). One extractor, one definition of "valid session" — there is no second identity path.

### One definition of "valid session" (extractor ⇆ SSE)

The token→hash transform (`session_token_hash`) and the hash→account lookup (`account_for_token_hash`) are hoisted to `pub(crate)` in `src/server/auth/mod.rs` specifically so the long-lived `GET /events` SSE stream consumes the auth module's *own* primitives rather than mirroring them. `server::events` hashes the cookie once at connect and re-runs `account_for_token_hash` **every frame and at least every 30s**, so a logged-out or password-reset session drops its stream — "is this session valid" can never mean two different things in two places. See [`04-realtime-sse.md`](04-realtime-sse.md).

| Primitive | Defined in | Also consumed by |
| --- | --- | --- |
| `SESSION_COOKIE = "authlyn_session"` | `session.rs:23` | `server/events.rs:148` |
| `session_token_hash(token) -> String` | `session.rs:102` | `server/events.rs:183` |
| `account_for_token_hash(state, hash)` | `session.rs:111` | `server/events.rs:102` |

## Public surface

Routes registered in `src/server/mod.rs::small_body_routes` (lines 70–83), under the 512 KiB JSON body cap. `register`/`login` are public; everything else self-gates via `AuthAccount`.

| Method · Path | Handler | Auth | Success | Notable failures |
| --- | --- | --- | --- | --- |
| `POST /auth/register` | `registration::register` | public | `201` + `Set-Cookie` + `AuthResponse` | `400` invalid creds; `409` username taken; `500` storage |
| `POST /auth/login` | `registration::login` | public | `200` + `Set-Cookie` + `AuthResponse` | `401` (identical body for unknown-user vs wrong-password) |
| `POST /auth/logout` | `registration::logout` | cookie (best-effort) | `204`, cookie cleared | (none — delete failure is logged, still `204`) |
| `GET /auth/me` | `registration::me` | `AuthAccount` | `200` + `MeResponse` | `401` no/expired/garbage cookie; `500` storage |
| `PATCH /account` | `registration::patch_account` | `AuthAccount` | `204` (emits `ListsChanged` on change) | `400` bad display_name; `404` unknown avatar media; `401` |
| `POST /auth/change-password` | `password::change_password` | `AuthAccount` | `204` | `400` new too short; `401` wrong current / unauth |
| `POST /auth/admin/reset-password` | `admin::admin_reset_password` | `AuthAccount` **+ `is_admin`** | `204`, target sessions invalidated | `403` not admin; `404` no such user; `400` new too short |

There is **no self-service / forgotten-password reset** route, and no security-question flow. See [Account recovery](#account-recovery-admin-reset-is-the-sole-path).

### DTOs

All in `src/protocol.rs` (always-on). Wire is serde JSON.

| Type | Shape | Used by |
| --- | --- | --- |
| `RegisterRequest` | `{ username, password }` | `POST /auth/register` |
| `LoginRequest` | `{ username, password }` (distinct type from `RegisterRequest` to allow later divergence, e.g. 2FA) | `POST /auth/login` |
| `AuthResponse` | `{ account_id, username }` (`username` = stored display form, not the ci key) | register/login body |
| `MeResponse` | `{ account_id, username, display_name, is_admin, avatar_id }` | `GET /auth/me` |
| `ChangePasswordRequest` | `{ current_password, new_password }` | `POST /auth/change-password` |
| `PatchAccountRequest` | `{ display_name?, avatar? }` (PATCH-shaped: `Default`, all-`Option<>`) | `PATCH /account` |
| `AdminResetPasswordRequest` | `{ username, new_password }` | `POST /auth/admin/reset-password` |
| `ErrorBody` | `{ error }` | every 4xx/5xx |

`MeResponse.is_admin` and `.avatar_id` carry `#[serde(default)]` for post-ship wire-compat: an older/native client that predates these fields still deserializes (`false` / `None`). `is_admin` is purely a UI hint (gates the Nova DOT broadcast composer) — it is **never** the actual authorization; the server re-checks `is_admin` on the admin route regardless.

The hydrate-side wrappers are `gloo-net` calls in `src/client/api.rs` (`current_user`, `register`, `login`, `logout`, `change_password`, `patch_account`, `admin_reset_password`, plus `humanize(ApiError)`). They are same-origin, so the `Secure` session cookie rides automatically — no manual `Authorization`/cookie header is ever set client-side.

## Registration & login internals

`create_account` (`registration.rs:293`) is the one non-obvious spot. The `CREATE account` runs against the `account_username_ci UNIQUE` index and is wrapped in `with_write_conflict_retry`: two concurrent registrations of the same username can make the MVCC loser surface a *retryable* write conflict rather than the UNIQUE violation. Retrying against a fresh snapshot then surfaces the clean UNIQUE violation, which the caller maps via `is_unique_violation` to **409** — never a 500. A plain `CREATE` here would 500 under the race.

- Pinned: `tests/auth.rs::concurrent_register_same_username_never_500s` (multi-thread; asserts exactly one `201` + one `409`, no `500`) and `tests/auth.rs::duplicate_username_is_409_case_insensitive`.
- The matcher strings inside `is_unique_violation` / `is_write_conflict` are load-bearing and pinned against *real* SurrealDB error text by `tests/retry_canary.rs::is_unique_violation_matches_real_surrealdb_violation` and `tests/retry_canary.rs::is_write_conflict_matches_real_surrealdb_conflict`. SurrealDB SDK/CLI must share the **3.x** major or the texts diverge.

`login` looks the account up case-insensitively (`account_by_username_ci`, shared with admin reset), then verifies. `register` trims and lowercases the username into `username_ci` before the create.

### No username enumeration

Both login failure branches — *no such user* and *wrong password* — return the **identical** `401 {"error":"invalid username or password"}` via the shared `invalid_credentials()` helper (`registration.rs:137`). Login also runs the (unknown-user) lookup and short-circuits before the argon2 verify, so the two paths are not perfectly timing-equal, but the *body and status* are indistinguishable. Do not add a distinct "no such user" message.

- Pinned: `tests/auth.rs::login_good_and_bad_credentials` asserts `body_pw == body_unknown`.

Note the asymmetry by design: `POST /auth/admin/reset-password` *does* return `404 "no such user"` for a missing target — that endpoint is already admin-gated, so it is not an anonymous enumeration surface.

## Password policy & argon2id

Validators live in `crypto.rs`. Username: 3–32 **characters**, no whitespace (`validate_credentials`). Password (`validate_password`, shared by register / change / admin-reset):

| Bound | Unit | Constant | Rationale |
| --- | --- | --- | --- |
| Minimum 8 | **CHARACTERS** | `MIN_PASSWORD_CHARS` | matches the user-facing "at least 8 characters" message; a byte count would let a sub-8-char multibyte password (e.g. three lock emoji = 3 chars / 12 bytes) slip past |
| Maximum 4096 | **BYTES** | `MAX_PASSWORD_BYTES` | a DoS / argon2-input bound; bytes is the correct unit for capping work fed to the hasher |

This CHARS-min / BYTES-max split is deliberate and trivially "simplified" wrong.

- Pinned: `tests/auth.rs::register_rejects_password_under_8_characters_even_when_8_bytes` (`"🔒🔒🔒"` → `400`); change-password length: `tests/auth.rs::change_password_rejects_too_short_new_password`.

**Hashing/verification run on the blocking pool.** `hash_on_blocking_pool` / `verify_on_blocking_pool` (`crypto.rs:43`, `:70`) wrap argon2id in `tokio::task::spawn_blocking` so the tens-of-ms CPU cost never stalls the async runtime. Salt is per-hash (`SaltString::generate(&mut OsRng)`); the stored form is the argon2 PHC string. A *join* failure → `500`; a verify of an **unparseable PHC** returns `false`, i.e. **401, not 500** — this is what makes the seeded `nova_dot` account (`password_hash = '!'`, a non-PHC sentinel) login-impossible by construction rather than crashing.

- The `'!'` sentinel and its login-impossible guarantee are documented at `src/storage/schema.surql` (the `nova_dot` UPSERT block); the parse-fail-→-false behavior that enforces it is `crypto.rs:74-78`. (No dedicated test pins the nova_dot-login-impossible path — code-pinned only.)

## Sessions & the cookie

`issue_session` (`session.rs:73`) → `CREATE session SET account, token_hash, expires_at = time::now() + 30d`. The `session` table (`schema.surql`) has `token_hash UNIQUE` and a `session_account` index. Resolution is the three-hop `account_for_token` → `session_token_hash` → `account_for_token_hash`, the last of which is the shared lookup `SELECT meta::id(account) … WHERE token_hash=$h AND expires_at > time::now()`.

Session teardown:

| Trigger | Function | Scope | Pinned |
| --- | --- | --- | --- |
| Logout | inline `DELETE … WHERE token_hash=$th` (`registration.rs:151`) | the one row; best-effort (failure logged, cookie cleared anyway) | `tests/auth.rs::logout_invalidates_the_session` |
| Admin reset | `delete_sessions_for_account` (`session.rs:134`) | **all** rows for the target account | code-pinned `admin.rs:67`; (no test asserts the post-reset session kill directly) |

`session_cookie` (`session.rs:147`) builds the cookie:

```rust
Cookie::build((SESSION_COOKIE, token))
    .path("/").http_only(true).secure(true)
    .same_site(SameSite::Lax)
    .max_age(time::Duration::days(30))
```

### The WebKit `Secure`-cookie trap (NOT test-pinned — owner-deck oracle only)

`secure(true)` is correct for production but is the single highest-surprise line in this subsystem. **Safari/WebKit drops a `Secure` cookie sent over `http://localhost`** (Chromium accepts it). The failure mode is silent and misleading: the browser "logs in" with a `200`, but the cookie is never stored, so the very next `GET /auth/me` (and every authed request) returns **401**. Chromium-green therefore does not mean WebKit-green here.

Rule: **test WebKit/iOS over HTTPS at the deck's public domain `https://authlyndev.damienmoon.sh`** (cloudflared, publicly-trusted cert already in iOS's trust store → the `Secure` cookie is accepted with no per-device cert step). Both retired workarounds — the `http://localhost` + `secure:false` injection and the LAN dev root CA — must not be reintroduced. Full deck rules: [`11-build-deploy-pwa.md`](11-build-deploy-pwa.md) and [`../../CLAUDE.md`](../../CLAUDE.md).

This is **(unpinned)** at the code/integration level: no `tests/*.rs` asserts the cookie's `Secure`/`HttpOnly`/`SameSite`/`Max-Age` attributes (grep over `tests/` finds none). It is guarded only by CLAUDE.md doctrine + the owner's iPhone deck pass. If you add a fidelity gate, this is the highest-value cookie assertion to add.

## Admin model

Admin is **env-driven and fail-closed**, defined in `src/server/permissions.rs` (not in the auth module). `is_admin(state, account_id)` (`permissions.rs:200`) returns true iff the account's stored `username_ci` is in `admin_username_set()` — the union of `AUTHLYN_ADMIN_USERNAMES` (comma/whitespace-separated) and the legacy singular `AUTHLYN_ADMIN_USERNAME`, each trimmed and lowercased. **An empty/unset set authorizes no one** (`is_admin` early-returns `false`). Admin is a configured-username membership, not a stored DB role/flag.

Consumers: `GET /auth/me` (to populate the `is_admin` UI hint), `POST /auth/admin/reset-password` (the actual `403` gate), and the system-message / feedback surfaces.

- Pinned (behavioral): exercised with an env-set admin via `tests/feedback.rs`, `tests/system_messages.rs`, `tests/dev_reload.rs`. The fail-closed/empty-set logic itself is code-pinned at `permissions.rs:200-239`; (no auth-suite test asserts the `403`-for-non-admin branch of the reset endpoint directly).

### Account recovery: admin reset is the sole path

`POST /auth/admin/reset-password` sets a target's password **without** the current one, then calls `delete_sessions_for_account` so any pre-existing cookie (possibly an attacker's) stops authenticating and the user is forced to a fresh login. It is `is_admin`-gated (`403` otherwise).

The earlier **self-service security-question reset was removed (2026-06-17) as an account-takeover vector**: a logged-in session could set a recovery credential *without* knowing the password, then "recover" through it. The `security_question` / `security_answer_hash` fields are purged and dropped in the schema. Admin reset is now the only recovery path. (Several `//!` headers — `src/server/auth/mod.rs:24`, `src/ui/shell/act/account.rs:1`, `src/ui/shell/act/mod.rs:8` — still mention a "security question / public reset flow"; those comments are **stale**, the routes and fields are gone.)

- Pinned: the field purge + drop is applied and `.check()`'d by `tests/schema_apply.rs` (the security-question removal test, `schema_apply.rs:~500`, registers the legacy fields on a populated row then asserts they are absent after apply).

## Profile updates & live identity

`PATCH /account` updates the caller's *own* row only (the `AuthAccount` extractor *is* the proof of ownership), so it has no membership/manager gate and no privacy-404 surface — you can only edit yourself. `display_name` is trimmed + validated 1–32 (`validate_display_name`); `avatar` is existence-checked against `media_blob` (a privacy-404 if missing, mirroring persona-gallery / guild-icon checks). An empty body is a `204` no-op.

On any change it emits `SyncEvent::ListsChanged` (id-only, like every SSE frame — [`04-realtime-sse.md`](04-realtime-sse.md)). Account identity (`display_name` + avatar) is the only **live-resolved** display data: a rename/re-avatar alters this account's *old* messages in every shared channel, so other members must refetch — the frame carries ids, never the new name.

- Pinned: `tests/auth.rs::patch_account_updates_display_name` (trim), `::patch_account_rejects_bad_display_name`, `::patch_account_unknown_avatar_is_404`, `::patch_account_without_cookie_is_401`.

## The 404-not-403 privacy posture (cross-cutting)

This subsystem establishes the platform-wide rule the rest of the API follows: **authorization is re-derived per request from the session, and non-membership / non-existence returns `404`, not `403`** — so an unauthorized caller cannot tell a resource apart from one that does not exist. Within auth proper the surfaces are narrow (`PATCH /account` is self-scoped; admin reset is the one intentional admin-only `404 "no such user"`), but the contract — identity from the cookie only, never client-supplied, re-checked on every mutate — originates here and is the assumption every guild/persona/lorebook handler builds on. The general rule and its pins are in [`05` siblings] [`../reference/conventions.md`](../reference/conventions.md) and the per-resource docs.

The *401-vs-404* distinction: a missing/garbage/expired session is **401** ("authentication required" / "invalid username or password") — that is the auth boundary. A valid session asking for something it may not see is **404**. Do not return `403` for the latter.

- 401 boundary pinned: `tests/auth.rs::me_without_cookie_is_401`, `::me_with_garbage_cookie_is_401`, `::change_password_requires_authentication`.

## Schema & migration hazards

The `account` and `session` tables are `SCHEMAFULL` and applied on every boot (`db::apply_schema`, pinned by `tests/schema_apply.rs`). Two ordering footguns (the class that crash-loops boot):

1. **`option<>` or backfill-before-revalidate.** A new field on the already-populated `account`/`session` table must be `option<>` *or* get an idempotent backfill `UPDATE` **before** any row-revalidating `UPDATE`. Concretely: the `display_name` NONE-coercion guard (`UPDATE account SET display_name = (display_name ?? '') WHERE display_name = NONE`) must run **before** the `security_question`/`security_answer_hash` `UNSET` UPDATE, or that account UPDATE 500s on rows predating `display_name`. The schema comment calls this ordering out explicitly.
2. **Widening an enum ASSERT needs `DEFINE FIELD OVERWRITE`,** not `IF NOT EXISTS` (which silently keeps the old narrower ASSERT). Not an auth field today, but the same `schema.surql` rule.

- Pinned: `tests/schema_apply.rs` applies the whole schema and `.check()`s it (an arena that boots is the gate); the security-question purge specifically by the removal test (`schema_apply.rs:~500`).

## Complexity hotspots (read the code, not memory)

| Location | Why it is subtle |
| --- | --- |
| `registration.rs:293-324` `create_account` | retry-wrapped `CREATE` vs UNIQUE → clean-violation → `409`; non-obvious why it isn't a plain `CREATE` without the retry_canary context |
| `crypto.rs:18-25,105-112` `validate_password` | the deliberate CHARS-min / BYTES-max split; one test guards it |
| `session.rs:91-130` token resolution | three-hop split exists so the extractor and the SSE per-frame re-check share ONE "valid session"; the `pub(crate)` hoist rationale is the tricky part |
| `session.rs:147-155` `session_cookie` | `secure(true)` — correct for prod, the exact WebKit/localhost `200`-but-`401` trap, not test-pinned |
| `registration.rs:100-116,137-139` | the equal-401-body no-enumeration contract spans two login branches + the shared helper; easy to break by adding a "no such user" message |
| `permissions.rs:200-239` `is_admin` | fail-closed empty-set semantics + two-env-var union; a regression silently over/under-grants and handlers only see a `bool` |

## Source map

Key files:

- `src/server/auth/mod.rs` — module facade; re-exports `AuthAccount` and the route handlers, hoists `SESSION_COOKIE` / `session_token_hash` / `account_for_token_hash` to `pub(crate)` for `server::events`. *(Stale `//!` mention of "security question / public reset" — ignore.)*
- `src/server/auth/registration.rs` — `register` / `login` / `logout` / `me` / `patch_account`; `create_account` (write-conflict-retried `CREATE`), `account_profile`, `media_exists`, `invalid_credentials`.
- `src/server/auth/password.rs` — `change_password`; shared DB helpers `account_by_username_ci`, `account_password_hash`, `update_password_hash`. *(No reset/security-question flow despite the `//!` header.)*
- `src/server/auth/crypto.rs` — argon2id `hash_on_blocking_pool` / `verify_on_blocking_pool`, `random_token`, `sha256_hex`, `validate_credentials` / `validate_password` (CHARS-min / BYTES-max).
- `src/server/auth/session.rs` — `AuthAccount` `FromRequestParts` extractor, `issue_session` / `account_for_token{,_hash}` / `delete_sessions_for_account`, `session_token_hash`, `session_cookie` (the `Secure` trap origin).
- `src/server/auth/admin.rs` — `admin_reset_password` (`is_admin`-gated `403`, target by username, then session invalidation).
- `src/server/permissions.rs` — `is_admin` / `admin_username_set` (fail-closed env union); the admin gate, defined outside the auth module.
- `src/ui/auth.rs` — `LoginPage` / `RegisterPage` Leptos components; `#[cfg(hydrate)]` submit handlers call `client::api` then `current_user`.
- `src/client/api.rs` — hydrate `gloo-net` REST wrappers + `humanize`; same-origin so the cookie rides automatically.
- `src/protocol.rs` (auth block) — `RegisterRequest` / `LoginRequest` / `AuthResponse` / `MeResponse` / `ChangePasswordRequest` / `PatchAccountRequest` / `AdminResetPasswordRequest` / `ErrorBody` (always-on DTOs).
- `src/storage/schema.surql` — `account` / `session` tables + `account_username_ci` & `session_token` UNIQUE; `nova_dot` `'!'` sentinel; `display_name` NONE backfill; purged security-question fields.
- `src/server/mod.rs` (lines 70–83) — route registration.

Pinning tests:

- `tests/auth.rs` — the auth suite (16 tests). Cited: `register_sets_cookie_and_me_resolves_it`, `me_without_cookie_is_401`, `me_with_garbage_cookie_is_401`, `login_good_and_bad_credentials` (identical 401 bodies), `duplicate_username_is_409_case_insensitive`, `concurrent_register_same_username_never_500s`, `register_rejects_password_under_8_characters_even_when_8_bytes`, `change_password_rotates_the_login_credential`, `change_password_wrong_current_is_rejected`, `change_password_rejects_too_short_new_password`, `change_password_requires_authentication`, `logout_invalidates_the_session`, `patch_account_updates_display_name`, `patch_account_rejects_bad_display_name`, `patch_account_unknown_avatar_is_404`, `patch_account_without_cookie_is_401`.
- `tests/retry_canary.rs` — `is_unique_violation_matches_real_surrealdb_violation`, `is_write_conflict_matches_real_surrealdb_conflict` (the `409`-not-500 matcher strings against live SurrealDB text).
- `tests/schema_apply.rs` — applies + `.check()`s the whole schema; the security-question removal test pins the field purge/drop.

**Unpinned / code-pinned only (verified absent from `tests/`):**
- Session cookie `Secure`/`HttpOnly`/`SameSite`/`Max-Age` attributes — **(unpinned)**; owner-deck oracle only (the WebKit trap).
- `nova_dot` login-impossible (`'!'` PHC sentinel → verify-false) — code-pinned (`crypto.rs:74-78` + `schema.surql`); no test.
- Admin-reset post-reset session invalidation, and the `403`-for-non-admin branch — code-pinned (`admin.rs:32-39,67`); fail-closed logic at `permissions.rs:200-239`; no auth-suite assertion.
