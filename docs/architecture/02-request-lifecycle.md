# Request Lifecycle

How a single HTTP request to the JSON API travels through the `ssr` server: from the axum router, through the session-cookie identity extractor, through per-mutation authorization re-derivation, to the SurrealDB call, and back out as either a typed [`ErrorBody`](#error-model) or a success response — plus the best-effort SSE nudge a mutation emits on the way out.

Scope is the **JSON API** (`small_body_routes`) and its cross-cutting machinery. The Leptos SSR/hydrate page handlers, the realtime `GET /events` stream, and media upload/download have their own lifecycles — see [04-realtime-sse.md](./04-realtime-sse.md), [07-ui-shell.md](./07-ui-shell.md), and the media notes in [03-data-model.md](./03-data-model.md).

This document is `ssr`-graph only. The graph rules (ssr / hydrate / nova, never cross-import) are in [01-overview.md](./01-overview.md) and `CLAUDE.md`.

---

## End-to-end path

```
HTTP request
  │
  ▼
RequestBodyLimitLayer        ── 512 KiB (JSON group) or 64 MiB (media group); over-cap → 413
  │
  ▼
axum route match             ── static segments outrank {captures}; method-routed
  │
  ▼
extractors run (in order)
  ├─ AuthAccount             ── session cookie → account id, or short-circuit 401/500
  ├─ Path / Query            ── ids from the URL
  └─ Json<…Request>          ── body parse; rejection → json_rejection_response (400)
  │
  ▼
handler body
  ├─ authorization re-derived per call   ── access.rs / permissions.rs (404 for non-membership)
  ├─ SurrealDB query(s)                  ── via state.db, optionally wrapped in
  │                                         with_write_conflict_retry (racy CREATE)
  └─ on success: state.emit / emit_for   ── best-effort id-only SSE nudge
  │
  ▼
Response
  ├─ success: 200/201/204 (+ Set-Cookie on auth)
  └─ failure: error_response(status, ErrorBody{ error })
  │
  ▼
map_response layer (JSON group only)     ── stamps Cache-Control: no-store on EVERY response
```

Every step below cites the source file and, where the behavior is load-bearing, the test that pins it.

---

## 1. The router: two body-limit groups + ranking

`make_router(state)` returns a fully-stated `Router` (so the integration tests can `oneshot` it with no port bind); `api_router()` returns the un-stated `Router<AppState>` that `main.rs` merges the Leptos handlers onto. Both delegate to `api_routes()`, which merges three things:

| Sub-router | Body cap | Purpose |
|---|---|---|
| `serve_service_worker` (`/sw.js`) | — | dynamic SW with `__BUILD_REV__` substituted; `Cache-Control: no-cache` |
| `small_body_routes()` | **512 KiB** (`REQUEST_BODY_LIMIT_BYTES`) | the entire JSON API (auth, guilds, messages, personas, lorebook, friends, DMs, cameos, push, feedback, admin) |
| `media_routes()` | **64 MiB** (`MEDIA_BODY_LIMIT_BYTES`) | `POST /media`, `GET /media/{id}` |

`src/server/mod.rs:68` (`small_body_routes`), `:273` (`media_routes`), `:340` (`api_routes`), `:352` (`make_router`).

### Why two groups, not one layer with two limits

`RequestBodyLimitLayer` composes with **min-limit semantics**: a larger inner cap nested under a smaller outer one still rejects at the smaller one. So a single router with both a 512 KiB and a 64 MiB layer would clamp media uploads to 512 KiB. The two caps must therefore live on **disjoint route groups**, merged only after each carries its own layer (`src/server/mod.rs:5`–`8`, `:257`, `:282`).

The media group additionally raises **axum's own** `DefaultBodyLimit` (`~2 MB` by default) to 64 MiB — axum's default and the tower-http layer both apply and min wins, so leaving axum's default in place silently capped uploads well under the intended ceiling and failed multi-MB phone photos with `"could not read multipart body"` (`src/server/mod.rs:277`–`282`).

### Static-over-dynamic ranking

axum's router prefers a **static** path segment over a `{capture}` segment regardless of declaration order. The route table relies on this so "collection-level" static routes are never shadowed by a sibling `{id}` route:

| Static route | Would otherwise collide with |
|---|---|
| `/guilds/trash` | `/guilds/{id}` |
| `/channels/read-state` | `/channels/{cid}/…` |
| `/channels/{cid}/messages/trash` | `/channels/{cid}/messages/{mid}` |
| `/dms` | (no dynamic sibling — ranks trivially) |
| `/channels/{cid}/guests/me` | `/channels/{cid}/guests/{aid}` |

`src/server/mod.rs:92`–`94`, `:140`–`144`, `:153`–`157`, `:225`–`235`. This is the router-level half of the project convention "static routes rank over dynamic" (see [../reference/conventions.md](../reference/conventions.md)); declaration order in the source is for human readers, not the matcher.

The full route → handler → DTO matrix lives in [../reference/rest-api.md](../reference/rest-api.md); this doc covers only the lifecycle every route shares.

---

## 2. Identity: the `AuthAccount` extractor (session cookie ONLY)

Identity is **server-derived from the session cookie and nothing else** — never a client-supplied user id, header, or body field. Any handler that needs the caller's account id takes `AuthAccount` in its signature; its absence on a handler means the route is public (only `/auth/register`, `/auth/login`, `/push/vapid-key`, and `GET /sw.js` are).

`AuthAccount(pub String)` carries the bare account key (`meta::id` form, e.g. `"abc123"`). Its `FromRequestParts` impl (`src/server/auth/session.rs:35`):

1. Reads the `authlyn_session` cookie from the request headers (`SESSION_COOKIE`, `src/server/auth/session.rs:23`). Missing → **401** `{"error":"authentication required"}`.
2. Resolves the raw token by SHA-256: `account_for_token` → `account_for_token_hash`, a single `SELECT meta::id(account) FROM session WHERE token_hash = $h AND expires_at > time::now()` (`src/server/auth/session.rs:130`). The DB stores **only** the token hash — the raw token never lands in a row.
3. Maps the outcome: live session → `AuthAccount(account)`; no/expired session → **401**; DB error → **500** `{"error":"storage error"}`.

| Cookie state | Result | Pinned by |
|---|---|---|
| valid, unexpired | `AuthAccount(id)`, handler runs | `tests/auth.rs::register_sets_cookie_and_me_resolves_it` |
| absent | 401 `authentication required` | `tests/auth.rs::me_without_cookie_is_401` |
| garbage value | 401 | `tests/auth.rs::me_with_garbage_cookie_is_401` |
| session row deleted (logout/reset) | 401 | `tests/auth.rs::logout_invalidates_the_session` |

### Cookie attributes (issue side)

`session_cookie(token)` builds `authlyn_session` as `Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=30d` (`src/server/auth/session.rs:166`). It is set by `register` and `login` (`src/server/auth/registration.rs:83`, `:145`) and cleared on `logout`.

The `Secure` attribute is the WebKit cookie trap: Safari/WebKit **drops** a `Secure` cookie over `http://localhost` (Chromium accepts it), so a WebKit client "logs in" 200 but every subsequent `AuthAccount` extraction 401s because the cookie was never stored. WebKit/iOS must be tested over HTTPS at the deck's public domain — full rule in `CLAUDE.md` ("WebKit Secure-cookie trap"). Do **not** weaken `secure(true)` to work around it.

The same hash transform and lookup (`session_token_hash`, `account_for_token_hash`) are re-exported `pub(crate)` and reused by the long-lived `GET /events` stream so "is this session valid" can never mean two different things across the per-request and per-frame paths (`src/server/auth/mod.rs:44`); see [04-realtime-sse.md](./04-realtime-sse.md).

---

## 3. Body parsing: typed 400 on malformed JSON

Mutating handlers take the body as `Result<Json<…Request>, JsonRejection>` rather than bare `Json<…>`, so a parse failure is handled in the body instead of producing axum's default plaintext rejection. `json_rejection_response` (`src/server/errors.rs:23`) maps the rejection to a stable **400** with a reason string that is part of the API surface:

| `JsonRejection` variant | 400 reason |
|---|---|
| `JsonDataError` | `invalid JSON body shape` |
| `JsonSyntaxError` | `malformed JSON` |
| `MissingJsonContentType` | `missing Content-Type: application/json` |
| `BytesRejection` | `could not read request body` |
| (other) | `invalid JSON request` |

Example wiring: `src/server/guilds/mod.rs:267`–`270` (`create_guild`). After a successful parse, field-level validation (e.g. `validate_name`) produces its own 400s; see [../reference/conventions.md](../reference/conventions.md) for the DTO/validation conventions.

---

## 4. Authorization: re-derived per mutation (privacy-404)

There is **no** request-scoped permission cache and no trust in anything the client sends. Every handler that touches a guild/channel/persona re-derives the caller's rights from the DB at the moment of the mutation, using the shared helpers in `permissions.rs` and `access.rs`. Two helper families:

### Guild role gates (`src/server/permissions.rs`)

| Helper | Returns | Status mapping |
|---|---|---|
| `caller_role(gid, account)` | `Option<role>` | raw lookup; caller decides the status |
| `require_manager(gid, account)` | `Ok(())` / early `Response` | non-member → **404**, plain member → **403**, owner/admin → Ok; guild must also be **live** (`ensure_guild_live` first) |
| `require_owner(gid, account)` | `Ok(())` / early `Response` | non-member → **404**, non-owner member → **403**, owner → Ok; **liveness-agnostic** (so restore can act on a trashed guild) |

`require_manager` gates the everyday management mutations (create/patch/delete channels, invite/kick/role, rename); admins are deliberately near-peers of the owner. `require_owner` gates only the structural/irreversible ones (`delete_guild`). `require_manager` calls `ensure_guild_live` so a **soft-deleted** guild — invisible to reads — is also immutable; `require_owner` deliberately skips that check so `restore_guild` can operate on a trashed guild (`src/server/permissions.rs:54`–`134`).

### The privacy rule: non-membership is **404, not 403**

A caller who is not a member of a guild gets **`404 {"error":"guild not found"}`**, identical to the response for a guild that does not exist — never a `403`. A `403` would confirm the resource exists, leaking its existence to a non-member. `403` is reserved for the case where the caller **is** a member but lacks the required *role* (plain member attempting a manager/owner action), which leaks nothing they didn't already know.

| Caller | `GET /guilds/{id}` | Manager mutation |
|---|---|---|
| owner / admin | 200 / proceeds | proceeds |
| plain member | 200 | **403** `admin only` |
| non-member | **404** `guild not found` | **404** `guild not found` |
| (no such guild) | **404** | **404** |

Source: `get_guild` non-member arm `src/server/guilds/mod.rs:368`; `require_manager` arms `src/server/permissions.rs:65`–`67`. Pinned by:

- `tests/guilds.rs::nonmember_get_guild_is_404` — outsider reading a guild gets 404.
- `tests/guilds.rs::rename_guild_and_channel_is_manager_gated` — owner 204; **plain member 403** on both guild and channel rename.
- `tests/guilds.rs::channel_create_is_owner_gated` — plain member 403 on channel create; owner 201.
- `tests/guilds.rs::invite_unknown_user_is_404` — inviting a non-existent target is 404.

This is the same server-trusted + privacy-404 invariant that governs auth and personas (`tests/auth.rs`, `tests/personas.rs`); see [05-auth-privacy.md](./05-auth-privacy.md) for the cross-subsystem treatment.

### Channel membership resolution (`src/server/access.rs`)

Message, persona, and lorebook handlers resolve "is the caller a member of the guild that owns this channel, and what kind of channel is it?" through `resolve_membership(state, cid, account, filter_deleted)` (`src/server/access.rs:47`). It resolves `cid` → `(guild, kind)`, then checks membership in the table that matches the channel kind:

- `kind = 'dm'` → a `dm_member` row;
- a guild channel → a `guild_member` row **or** an active (unexpired) `channel_guest` row (Guest Cameos, M7/P2; expiry is `expires_at = NONE OR > now`).

It returns a three-outcome `Membership { Member{kind} | ChannelNotFound | NotMember }`; every current call site collapses both negative outcomes to a privacy-404 / `false`, preserving the rule above. The one behavioral knob is `filter_deleted`: `messages`/`personas` resolve only a live channel in a live guild; `lorebook` resolves regardless of soft-delete state (preserved verbatim — `src/server/access.rs:17`–`22`).

`visible_channels(state, account)` (`src/server/access.rs:164`) is the account-wide form (every live channel the caller may currently see), reused by `GET /events` filtering and `GET /unread`.

### Persona edit-access and the admin guard

`owns_persona` / `is_persona_editor` / `can_edit_persona` (owner OR redeemed editor) gate persona PATCH and "wear" (`src/server/permissions.rs:140`–`189`). `is_admin(state, account_id)` is **fail-closed**: the caller is admin iff their stored `username_ci` is in the env-configured admin set (`AUTHLYN_ADMIN_USERNAMES` ∪ `AUTHLYN_ADMIN_USERNAME`, trimmed + lowercased); an empty set authorizes no one (`src/server/permissions.rs:200`–`239`). It gates `/feedback` listing, `/admin/system-message`, `/admin/dev/reload`, and admin password reset (admin failure → 403).

---

## 5. The DB call + write-conflict retry → idempotent 409

Handlers issue SurrealDB statements through `state.db` (an `Arc<Surreal<Client>>`; see [§7](#7-appstate-what-every-handler-shares)). Any handler that issues a **racy `CREATE` against a UNIQUE index** wraps it in `with_write_conflict_retry` so an MVCC race resolves to an **idempotent 409, never a 500**.

This is realized in exactly one place (`src/server/retry.rs`) so every consumer shares one backoff schedule, one attempt cap, and one set of error matchers. The ~25 call sites include registration (`account`), guild membership (`guild_member`), persona editors (`persona_editor`), friendships (`friendship`), custom emoji (`custom_emoji`), per-channel persona wear (`channel_active_persona`), and push subscriptions (`push_subscription`) — see the `grep` of `with_write_conflict_retry` across `src/server/` for the full list.

### `with_write_conflict_retry` (`src/server/retry.rs:33`)

Runs the closure up to `MAX_WRITE_CONFLICT_ATTEMPTS = 5` times. On a **retryable** error (`is_write_conflict`) it sleeps `BASE_BACKOFF_MS(5) * attempt + jitter(0..20)` ms and retries — a deterministic floor of `5+10+15+20 = 50 ms` across the four inter-attempt sleeps, up to `~126 ms` with maximum jitter. Jitter de-stampedes concurrent retriers. Any **non**-retryable error propagates immediately. Exhausting the budget on a still-conflicting error logs and returns the residual error to the caller.

### The two matchers — kept disjoint (`src/server/retry.rs:135`, `:173`)

The SDK exposes these as plain `surrealdb::Error` with no typed variant, so both predicates substring-match the `Display` text. The wording drifts across SurrealDB releases, so the matchers are intentionally broad and pinned against live error text:

| Predicate | Matches (case-insensitive) | Maps to |
|---|---|---|
| `is_write_conflict` | `"write conflict"` **or** `"can be retried"` **or** the full sentence `"the query was not executed due to a failed transaction"` | retry inside the loop |
| `is_unique_violation` | `"already contains"` | **409** *outside* the loop |

The three `is_write_conflict` markers cover three observed SurrealDB texts (`=3.1.0-beta.3` SDK, `3.0.4` server, `3.1.3` server). The third marker is matched as the **full** sentence, not a loose `"failed transaction"` substring, so a thrown error merely echoing that phrase can't trip the retry loop (review M-33). The two predicates' substrings are disjoint by inspection, so neither fires on the other's error.

UNIQUE violations are mapped to 409 **outside** `with_write_conflict_retry` — re-issuing the same `CREATE` against the same key tuple just fails identically, so it is not retryable. The handler runs the retry (which absorbs the *transient* MVCC race), then inspects the **residual** error: `is_unique_violation` → 409, anything else → 500. Canonical shape (`src/server/guilds/membership.rs:146`–`174`):

```rust
let result = with_write_conflict_retry(|| async {
    state.db.query("CREATE guild_member SET …").await?.check()?;
    Ok(())
}).await;
match result {
    Ok(()) => { state.emit(SyncEvent::ListsChanged); StatusCode::CREATED.into_response() }
    Err(e) if is_unique_violation(&e) => error_response(StatusCode::CONFLICT, "user is already a member"),
    Err(e) => { tracing::error!(error = %e, "invite_member write failed");
                error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error") }
}
```

Many such handlers also keep a cheap **pre-check** (`caller_role` here, `src/server/guilds/membership.rs:137`) that returns the same 409 body on the common non-racing case; the retry+UNIQUE arm is the race backstop, so the two paths return the identical body.

#### Pinned by

| Claim | Test |
|---|---|
| `is_write_conflict` fires on a real MVCC conflict | `tests/retry_canary.rs::is_write_conflict_matches_real_surrealdb_conflict` |
| `is_unique_violation` fires on a real UNIQUE collision, and `is_write_conflict` does **not** | `tests/retry_canary.rs::is_unique_violation_matches_real_surrealdb_violation` |
| the accepted false-positive class (aborted-txn sibling text) stays matched; loose `"failed transaction"` does **not** match | `tests/retry_canary.rs::aborted_transaction_sibling_text_is_indistinguishable_from_a_write_conflict` |
| concurrent same-username register → one 201 + one 409, never 500 | `tests/auth.rs::concurrent_register_same_username_never_500s` |
| duplicate username (sequential) → 409, case-insensitive | `tests/auth.rs::duplicate_username_is_409_case_insensitive` |
| concurrent invite → exactly one 201, one 409, one `guild_member` row | `tests/guilds.rs::concurrent_invite_yields_one_member_row` |

These canaries are **load-bearing**: a SurrealDB error-text rename would silently disable the retry loop (every UNIQUE 409 would degrade to a 500) with no compile-time signal, so the canaries synthesize the real errors against a live DB and assert the matchers still fire. The SurrealDB major is pinned to `3.x` for exactly this reason (`CLAUDE.md`, "SurrealDB pin").

> **Accepted cost (review M-33):** on 3.1.x, any *permanently*-failing multi-statement transaction surfaces the same generic sibling text that `Response::check()` reports first, so it is byte-identical to a genuine commit-time conflict at this layer and gets replayed 4 extra times (~50–126 ms) before its real error surfaces. Bounded by design: every transactional consumer is idempotent (DELETE+CREATE shaped) and `is_unique_violation` is checked on the residual, so no 409 degrades to a 500. Full reasoning in the `is_write_conflict` doc comment, `src/server/retry.rs:103`–`125`.

---

## 6. The response: error model + best-effort SSE + no-store

### Error model

Every 4xx/5xx reply is built through `error_response(status, msg)` (`src/server/errors.rs:16`), which serializes the canonical [`ErrorBody`](../reference/rest-api.md) — `{"error": "<reason>"}` (`src/protocol.rs:19`) — so the failure body shape is identical across every route. `ErrorBody` lives in the always-on `protocol.rs` (wasm-safe, serde-only) so the hydrate client deserializes the same type. Success responses return their own typed `…Response` DTO (or `204 No Content` for mutations with nothing to return).

| Status | When |
|---|---|
| 400 | malformed JSON (`json_rejection_response`) or field validation |
| 401 | no/invalid/expired session (`AuthAccount`) |
| 403 | authenticated member lacks the required role/admin |
| 404 | resource absent **or** caller is a non-member (privacy) |
| 409 | idempotent racy-`CREATE` / UNIQUE collision |
| 413 | request body over the group's `RequestBodyLimitLayer` cap |
| 500 | storage error (logged; body is the opaque `storage error`) |

### Best-effort SSE emit

On a **successful mutation**, the handler nudges the realtime bus *before* returning: `state.emit(event)` (visibility-filtered) or `state.emit_for(accounts, event)` (account-targeted). This is **best-effort** — `send()` errs only when there are no subscribers (the idle case) and the error is discarded, so it never fails the request (`src/server/state.rs:165`–`181`). The bus is **id-only**: frames carry record ids, never content (the typing-draft text is the one piece of mutable content the server surfaces, and it goes over a separate permission-checked GET, never the bus). The mechanism, the per-connection visibility filter, and the per-frame session re-check are in [04-realtime-sse.md](./04-realtime-sse.md).

### `no-store` on every JSON response

The `small_body_routes` group carries a blanket `map_response` layer that stamps **`Cache-Control: no-store`** on *every* response it produces — successes and errors alike (`src/server/mod.rs:261`–`269`). Dynamic JSON must never be cached by the service worker or the browser HTTP cache: a cached message list once flashed ancient messages on cold open before the live fetch landed. Because it is a blanket layer, a 401 is also `no-store` (an auth error must not be cached either).

The **media** group is a separate router *without* this layer; media responses instead carry their own `private, max-age=31536000, immutable` per-response header (session-gated, so `private` not `public`). Keeping the groups split is what lets the two caching policies coexist.

| Claim | Test |
|---|---|
| authed JSON 200 is `no-store` | `tests/cache_control.rs::json_api_responses_are_no_store` |
| a 401 on the JSON group is also `no-store` | `tests/cache_control.rs::no_store_applies_even_to_error_responses` |
| media responses are **not** `no-store` (immutable, private) | `tests/cache_control.rs::media_route_group_is_not_no_store` |

---

## 7. `AppState`: what every handler shares

Every handler receives `State(AppState)` (`src/server/state.rs:60`). It is cheap to `Clone` (one `Arc` refcount bump per field). The lifecycle-relevant fields:

| Field | Type | Role in the lifecycle |
|---|---|---|
| `db` | `Arc<Surreal<Client>>` | the shared SurrealDB handle every query runs on |
| `events` | `broadcast::Sender<BusEvent>` (cap 256) | the SSE bus `emit`/`emit_for` send into |
| `media_dir` | `Arc<PathBuf>` | canonicalized-at-construction media root (path-traversal check is then a free `starts_with`) |
| `push` | `Option<Arc<PushSender>>` | `None` → every push path is a silent no-op (tests, env unset) |
| `typing` / `typing_drafts` | `Arc<Mutex<…>>` | in-memory ephemeral state; the `Mutex` is **never** held across an `.await` |
| `leptos` | `LeptosOptions` | reachable via `FromRef` so `leptos_routes` accepts the combined state |

Constructors: `AppState::new(db, media_dir)` for tests (placeholder `LeptosOptions`, `push: None`); `AppState::with_leptos(…)` for `main.rs`. The `Copy` tunables `draft_ttl` and `sse_recheck_period` have builder overrides (`with_draft_ttl`, `with_sse_recheck_period`) that must be applied **before** the state is cloned into the router (`src/server/state.rs:146`, `:156`). `media_dir` is canonicalized at construction and the constructor **panics** if the dir doesn't exist — `main.rs` and the test harness `create_dir_all` first (`src/server/state.rs:205`).

The integration tests drive this whole pipeline by handing a test `AppState` to `make_router` and calling `tower::ServiceExt::oneshot` (no port bind); `tests/common/mod.rs` gives each worker an isolated namespace + media tempdir (`arena()`, `register_account`, `send` helpers). See [09-testing.md](./09-testing.md).

---

## Source map

**Source**

| Path | Role |
|---|---|
| `src/server/mod.rs` | route table; two body-limit groups + min-limit rationale; static>dynamic ranking; `make_router`/`api_router`/`api_routes`; the `no-store` `map_response` layer; dynamic `/sw.js`; soft-delete purge sweep |
| `src/server/auth/session.rs` | `AuthAccount` extractor (cookie → account id, 401/500); session token issue/resolve/revoke; `authlyn_session` cookie attributes (`HttpOnly; Secure; SameSite=Lax`) |
| `src/server/auth/mod.rs` | auth submodule layout; re-exports of `AuthAccount` + the session primitives the SSE stream reuses |
| `src/server/auth/registration.rs` | `register`/`login` cookie-set + UNIQUE→409 mapping (lifecycle example) |
| `src/server/permissions.rs` | `caller_role`, `require_manager` (404 non-member / 403 member / live-guild check), `require_owner`, persona edit gates, fail-closed `is_admin` |
| `src/server/access.rs` | `resolve_membership` (3-outcome, per-kind membership table, `filter_deleted` knob); `visible_channels` |
| `src/server/retry.rs` | `with_write_conflict_retry` (5 attempts, linear backoff + jitter); `is_write_conflict` / `is_unique_violation` matchers (disjoint) |
| `src/server/errors.rs` | `error_response` (canonical `ErrorBody`); `json_rejection_response` (typed 400) |
| `src/server/state.rs` | `AppState` struct + constructors; `emit`/`emit_for` (best-effort, id-only); `BusEvent`; injectable `Copy` tunables |
| `src/server/guilds/membership.rs` | concrete retry + pre-check + UNIQUE→409 handler (`invite_member`) |
| `src/protocol.rs` | always-on, wasm-safe `ErrorBody` DTO (`{"error":…}`) |

**Tests that pin the claims here**

| Test | Pins |
|---|---|
| `tests/auth.rs::register_sets_cookie_and_me_resolves_it` | cookie set on register; `AuthAccount` resolves it |
| `tests/auth.rs::me_without_cookie_is_401` / `::me_with_garbage_cookie_is_401` | missing/garbage cookie → 401 |
| `tests/auth.rs::logout_invalidates_the_session` | deleted session row → 401 |
| `tests/auth.rs::concurrent_register_same_username_never_500s` | racy register → one 201 + one 409, never 500 |
| `tests/auth.rs::duplicate_username_is_409_case_insensitive` | UNIQUE → 409 (case-insensitive) |
| `tests/guilds.rs::nonmember_get_guild_is_404` | non-member read → privacy 404 |
| `tests/guilds.rs::rename_guild_and_channel_is_manager_gated` | plain member → 403; owner → 204 |
| `tests/guilds.rs::channel_create_is_owner_gated` | plain member → 403 on create |
| `tests/guilds.rs::invite_unknown_user_is_404` | invite missing target → 404 |
| `tests/guilds.rs::concurrent_invite_yields_one_member_row` | racy invite → one 201 + one 409 + one row |
| `tests/retry_canary.rs::is_write_conflict_matches_real_surrealdb_conflict` | `is_write_conflict` fires on real MVCC conflict |
| `tests/retry_canary.rs::is_unique_violation_matches_real_surrealdb_violation` | `is_unique_violation` fires; `is_write_conflict` stays disjoint |
| `tests/retry_canary.rs::aborted_transaction_sibling_text_is_indistinguishable_from_a_write_conflict` | accepted false-positive class; M-33 loose-match guard |
| `tests/cache_control.rs::json_api_responses_are_no_store` | JSON 200 → `no-store` |
| `tests/cache_control.rs::no_store_applies_even_to_error_responses` | JSON 401 → `no-store` |
| `tests/cache_control.rs::media_route_group_is_not_no_store` | media → immutable/private, not `no-store` |
