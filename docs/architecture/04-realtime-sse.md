# 04 — Realtime: the id-only SSE bus, Web Push, dev hot-reload

The realtime layer is **notify-and-fetch**, not push-the-payload. A mutation broadcasts a content-free *signal* over an in-process bus; a long-lived `GET /events` SSE stream relays the signal to the clients that may see it; each client reacts by **refetching** through the existing permission-checked JSON endpoints. No message body, draft text, or other content ever rides the bus.

This is a hard design constraint, not an optimization: **the bus must never become an authorization surface.** Per-connection privacy filtering exists, but it is the *second* line of defense — the first is that the frames carry only ids, so even a filter bug leaks an id, never content. The one place draft text *could* leak is Ghost Quill; it is structurally walled off (see [Ghost Quill](#ghost-quill-draft-text-never-touches-the-bus)).

The vocabulary ([`SyncEvent`](#the-syncevent-vocabulary)), the bus envelope ([`BusEvent`](#the-bus-emit-vs-emit_for)), and `GET /events` are **ssr**-graph. `SyncEvent` itself is always-on (it is a wire DTO in `src/protocol.rs`, also consumed by the **hydrate** `EventSource` client). Web Push is **ssr**. See [01-overview](01-overview.md) for the graph rules and [05-auth-privacy](05-auth-privacy.md) for the privacy-404 and session model this layer re-uses.

---

## The `SyncEvent` vocabulary

`src/protocol.rs` (`SyncEvent`). Internally tagged serde enum (`#[serde(tag = "type", rename_all = "snake_case")]`), shared ssr↔hydrate, deliberately content-free.

| Variant | Wire `type` | Payload fields | `channel_id()` | Delivery lane | Emitted by |
|---|---|---|---|---|---|
| `MessageCreated` | `message_created` | `channel_id` | `Some` | visibility-filtered (`emit`) | post / roll / real restore |
| `MessageEdited` | `message_edited` | `channel_id`, `message_id` | `Some` | visibility-filtered | edit |
| `MessageDeleted` | `message_deleted` | `channel_id`, `message_id` | `Some` | visibility-filtered | soft-delete |
| `Typing` | `typing` | `channel_id` | `Some` | visibility-filtered | typing ping |
| `ListsChanged` | `lists_changed` | — | `None` | **either** lane | channel/guild/membership/profile mutations; also the lag/resync nudge |
| `ReadStateChanged` | `read_state_changed` | `channel_id` | `None` (see note) | **targeted** (`emit_for`) | `mark-read` |
| `FriendsChanged` | `friends_changed` | — | `None` | **targeted** | friend request/accept/remove |
| `Reload` | `reload` | — | `None` | **global, filter-bypassing** | `POST /admin/dev/reload` |
| `Unknown` | *(any unrecognized)* | — | `None` | n/a (never constructed) | — |

Wire shapes are pinned by `tests/sync_events.rs::sync_event_serializes_with_snake_case_type_tags` (message/lists/typing/edited + the `Unknown` fallback), `tests/sync_events.rs::targeted_sync_events_pin_their_wire_shape` (`read_state_changed`, `friends_changed`), and `tests/sync_events.rs::reload_sync_event_is_a_bare_global_tag`.

### `channel_id()` is the visibility key, not a content field

`SyncEvent::channel_id() -> Option<&str>` (`src/protocol.rs:1082`) answers one question: *which channel's visibility gate decides whether this connection gets the frame?* The visibility-filtered lane consults it; `Some(cid)` events are delivered iff `cid` is in the connection's visible set, `None` events are delivered to everyone (and trigger a visibility reload, since "lists changed" may have shifted what is visible).

**`ReadStateChanged` carries a `channel_id` field but `channel_id()` returns `None`.** This is deliberate and load-bearing. It is delivered on the *targeted* lane (to the actor's own other devices), which bypasses visibility filtering entirely — so the method is never consulted for it. Returning the id would be actively wrong: the targeted recipient's visible-set snapshot can be momentarily stale, and visibility filtering would then silently drop a self-nudge the recipient is unconditionally entitled to. Pinned by `tests/sync_events.rs::targeted_sync_events_pin_their_wire_shape` (asserts `ReadStateChanged{..}.channel_id() == None`).

### Forward-compat: `Unknown`

`#[serde(other)] Unknown` is the version-skew catch-all. A newer server that emits a `SyncEvent` an older deployed client doesn't recognize must not make that client's `EventSource` throw — it decodes to `Unknown`, which the hydrate consumer ignores. The server **never constructs** `Unknown`. Pinned by the `warp_initiated` case in `tests/sync_events.rs::sync_event_serializes_with_snake_case_type_tags` (`{"type":"warp_initiated"}` → `Unknown`, `channel_id() == None`).

---

## The bus: `emit` vs `emit_for`

`src/server/state.rs`. The bus is a single process-wide `tokio::sync::broadcast::Sender<BusEvent>` held on `AppState.events` (capacity **256**). Every mutation handler best-effort `send`s; every `GET /events` connection `subscribe`s.

```rust
pub struct BusEvent {
    pub event: SyncEvent,
    pub targets: Option<Vec<String>>, // None = visibility-filtered/global; Some = account-targeted
}
```

The envelope's `targets` field selects the delivery lane, via two `AppState` methods:

| Method | `targets` | Lane | Visibility filter? | Use for |
|---|---|---|---|---|
| `emit(ev)` | `None` | visibility-filtered (or global for `None`-scoped events) | yes, by `channel_id()` | anything observable by channel membership |
| `emit_for(accounts, ev)` | `Some(accounts)` | account-targeted | **no** — delivered iff the connection's account is in the list | per-account nudges about the target's *own* state |

Both are **best-effort and infallible**: `broadcast::Sender::send` errs only when there are zero subscribers (the idle case), and that error is discarded. **A bus emit must never fail a request.** Events are droppable by design: a slow consumer that overruns the 256-capacity ring gets `RecvError::Lagged` and is nudged to a full resync rather than blocking producers (see [lag](#lag-the-resync-fallback)).

**`emit_for` bypasses visibility filtering**, so it carries a sharper rule than `emit`: only ever pass an id-only nudge whose mere *arrival* reveals nothing the target couldn't already fetch. The current `emit_for` callers all send `ReadStateChanged` / `FriendsChanged` / `ListsChanged` to accounts that own the changed state — `src/server/messages/read_state.rs:128` (`mark-read` → actor's own devices), `src/server/friends.rs:310` (both edge accounts), `src/server/dms.rs` and `src/server/cameos.rs` (participants/affected), and the targeted `ListsChanged` arms in `src/server/guilds/mod.rs` (guild create — the creator is the sole member at birth).

Why `lists_changed` rides *both* lanes: when a metadata change is observable by many (e.g. a profile rename alters the author display on messages others can see) it goes out `emit`-global so every client refetches (`tests/events.rs::account_profile_change_broadcasts_lists_changed_to_other_members`); when it is observable only by the actor (guild create, rail reorder) it goes out `emit_for` to the actor's own devices and **stays off everyone else's stream** — avoiding the N-connections × M-mutations broadcast amplification (`tests/events.rs::create_guild_no_longer_broadcasts`, `tests/events.rs::rail_reorder_no_longer_broadcasts`).

---

## `GET /events` — the SSE stream

`src/server/events.rs`. A long-lived `text/event-stream` of id-only `SyncEvent`s, filtered per-connection to what the caller may see. Each delivered `SyncEvent` is one unnamed SSE `data:` frame (`{"type":"...","channel_id":"..."}`); the dev-reload nudge is the single exception (a distinct **named** frame — see [dev hot-reload](#dev-hot-reload-the-reload-frame)). The handler attaches axum's `KeepAlive` layer, which owns the wire-level heartbeat independently of the delivery loop.

### Connection setup (order matters)

1. **Auth** via the session cookie through the `AuthAccount` extractor — identical to every JSON route. The handler then also reads the raw cookie value; its SHA-256 hash gates every subsequent frame (see [revocation](#mid-stream-revocation-identity-is-re-checked-for-the-streams-lifetime)). If the cookie is somehow absent post-extractor, it fails closed with 401 (`tests/events.rs::events_requires_a_session`; the belt-and-braces no-cookie path is `src/server/events.rs` `tests::missing_session_cookie_is_rejected_before_any_stream_exists`).
2. **Subscribe to the bus** *before* loading visibility — an event for a channel created in the gap between subscribe and the visible-set load is recovered by the `lists_changed → reload` path, never missed.
3. **Initial visible-set load** via `visible_channels(&state, account)` (`src/server/access.rs`). **A failed initial load returns 500, never a deaf-but-200 stream.** This is subtle: the hydrate driver promotes SSE and retires its poll fallback on a successful open, so a 200 carrying an empty set would leave the client *live but deaf* until some unrelated global `lists_changed` happened by. A 500 instead makes `EventSource` fire `onerror` and the client's backoff/poll-fallback engage. Pinned by `src/server/events.rs` `tests::initial_visible_set_load_failure_returns_500_instead_of_a_deaf_stream`.

The handler subscribes **eagerly in the body** (before the response future resolves) because the integration contract posts a message immediately after the response resolves and must not miss its event.

### The per-connection delivery loop

The stream is a `futures_util::stream::unfold` over a `Conn` holding: the broadcast `Receiver`, the `visible: HashSet<String>` channel set, a `visible_stale` flag, the `session_token_hash`, and a `next_recheck: Instant` deadline. Each iteration:

1. **Deadline gate (top of loop):** if `now >= next_recheck`, run the session re-check; revoked → end the stream, still valid → advance the deadline. This runs *however* the loop got here, so neither a silent bus nor an all-filtered bus can starve the periodic re-check (see [revocation](#mid-stream-revocation-identity-is-re-checked-for-the-streams-lifetime)).
2. **Park on `recv()` until the deadline:** `tokio::time::timeout_at(next_recheck, rx.recv())`. Using a fixed-instant `timeout_at` (not a per-receive `timeout(period, …)`) is load-bearing — a per-receive timer would re-arm on every filtered `continue` and could be starved forever under sustained invisible traffic.
3. **Classify the received `BusEvent`:**
   - **`Reload`** (any envelope): short-circuit *both* the visibility filter and the targeted lane, re-check the session, emit the named `reload` frame. (See [dev hot-reload](#dev-hot-reload-the-reload-frame).)
   - **`targets: Some`** (targeted lane): deliver iff `conn.account` is named, with **no** visibility check. Special-case: a targeted `ListsChanged` reloads `conn.visible` first, because the membership shift it signals changes what *this* connection may subsequently see.
   - **`targets: None`** (visibility lane): if `channel_id()` is `Some(cid)`, deliver iff `cid ∈ visible` (retrying a stale reload first if `visible_stale`); if `None` (e.g. `ListsChanged`), reload `visible` and deliver.
   - **`Lagged(_)`**: reload `visible`, then synthesize a `ListsChanged` to nudge a full resync (see [lag](#lag-the-resync-fallback)).
   - **`Closed`**: end the stream.
   - **timeout elapsed**: `continue` (the top-of-loop gate handles the re-check).
4. **Re-check identity before delivering** (mirrors the per-request rule on JSON routes), then advance `next_recheck` and yield the frame.

A filtered event is a `continue`: it completes `recv()` *without* reaching the per-frame gate — which is exactly why the top-of-loop deadline gate exists.

### Per-connection visibility filtering (privacy)

The visible-channel set is the privacy boundary. It is computed by `visible_channels` (`src/server/access.rs`) — the union of: text channels in guilds the account is a member of (with the guild not soft-deleted), DM threads the account is a `dm_member` of, and unexpired cameo/guest channels (`channel_guest`). The same query backs `GET /unread` and re-runs on every `ListsChanged`/`Lagged`.

The filter is enforced in **both directions** mid-stream, by reloading `visible` whenever a `None`-scoped or targeted `ListsChanged` (or a `Lagged`) arrives:

- **Grow:** a channel created *after* subscribe becomes visible — `tests/events.rs::channel_creation_emits_lists_changed_and_membership_set_refreshes` proves a message in the new channel reaches the already-open stream. The targeted-lane reload guard is pinned by `tests/events.rs::targeted_lists_changed_reloads_the_connections_visibility_set`.
- **Shrink (revocation of *visibility*):** kick / self-leave / guild soft-delete must stop an already-open stream from receiving the guild's channel events — `tests/events.rs::kicked_member_stops_receiving_channel_events_mid_stream`, `::member_who_leaves_a_guild_stops_receiving_its_events_mid_stream`, `::guild_soft_delete_silences_open_member_streams`.

Cross-connection isolation is pinned by `tests/events.rs::outsider_never_receives_events_for_a_channel_they_cannot_see` and `::typing_events_do_not_leak_across_guilds`. Targeted-lane isolation (a nudge reaches *only* the named accounts) is pinned by `tests/events.rs::read_state_changes_reach_only_the_same_account` and `::friend_mutations_reach_both_parties_over_sse`. Each of these asserts the bystander stream **times out (not closes)** and then proves the same stream is still alive — silence must be a filter, never a dead connection.

#### Fail-closed reloads

If a mid-stream `reload_visible` hits a DB error, it **clears** `visible` and sets `visible_stale` (fail closed): a reload is *how a revocation reaches the connection*, so keeping the stale set would keep delivering ids the caller may no longer see. `visible_stale` schedules a retry on the next channel-scoped event, so a transient error self-heals. Pinned by `src/server/events.rs` `tests::reload_visible_failure_clears_the_set_instead_of_keeping_stale_grants`. (The integration suite cannot reach the DB-error arms — they are unit-tested in-module against an uninitialized `Surreal` handle that errors every query without I/O.)

### Mid-stream revocation: identity is re-checked for the stream's lifetime

Unlike a JSON route — which authenticates once per request — `/events` re-derives identity **for the entire lifetime of the stream**. A session that is logged out, password-reset-locked, or expired must *end* the stream, not leave an unkillable metadata feed running on a revoked credential. Two mechanisms enforce it:

1. **Per-frame:** the session is re-validated before every *delivered* frame, via `account_for_token_hash(state, session_token_hash)` — the auth module's own hash-keyed lookup, so the re-check can never drift from the per-request rule. Mismatch or `None` → end the stream. A DB error → **fail closed** (count as revoked; the client's reconnect re-authenticates and 401s). Pinned by `tests/events.rs::logging_out_a_session_ends_its_live_events_stream` and `src/server/events.rs` `tests::session_recheck_db_failure_counts_as_revoked`.
2. **Forced periodic (`sse_recheck_period`, default 30s):** the per-frame gate only fires on *delivery* — so a stream that delivers nothing (silent bus, or a bus whose every event is filtered out for this connection) would never re-check. The deadline gate at the top of the loop closes that hole. The deadline advances **only when a re-check actually runs** (deadline lapse or a delivered frame), never on a mere bus receive — so it measures re-check silence, not bus activity. A revoked session therefore dies within ~one period regardless of traffic shape. Period is injectable via `AppState::with_sse_recheck_period` (`DEFAULT_SSE_RECHECK_PERIOD`, `src/server/state.rs:42`) so the tests run in ms. Pinned by `tests/events.rs::a_quiet_stream_dies_after_revocation_without_any_event` (silent bus) and `::a_revoked_stream_dies_even_while_invisible_bus_traffic_keeps_arriving` (busy-but-filtered bus — the narrower disguise of the same leak).

> **Extend this to any new realtime surface.** The contract — id-only frames, per-frame + periodic session re-check, fail-closed on DB error — is the project invariant, not a one-off for this handler (CLAUDE.md *SSE bus is id-only*).

### Lag: the resync fallback

The broadcast ring is capacity 256. A consumer that falls behind gets `RecvError::Lagged(n)` instead of the dropped events. The loop responds by reloading `visible` (the dropped events may have included a `ListsChanged`) and synthesizing a `ListsChanged` frame — telling the client to refetch everything. This is the same "I lost track, resync" nudge `ListsChanged` serves after any ambiguous gap, and it is *why* events are allowed to be droppable: correctness never depends on delivering every frame, only on eventually nudging a refetch.

---

## Ghost Quill: draft text never touches the bus

`src/server/messages/typing.rs`, store on `AppState.typing_drafts`. Ghost Quill surfaces a co-writer's *live, in-progress* draft text to other members of a channel — the one feature whose payload is genuinely sensitive content. It is built so that **draft text never rides a `SyncEvent`.**

The store is `TypingDraftMap = HashMap<(channel_id, account_id), (draft_text, last_ping_instant)>` — **in-memory only, never the DB**, guarded by a `std::sync::Mutex` whose critical section is *never* held across an `.await` (lock → mutate/collect → drop). Same discipline and TTL as the ephemeral `typing` indicator (`DEFAULT_DRAFT_TTL` = 8s, `src/server/state.rs:30`; injectable via `with_draft_ttl`); pruned opportunistically on every write and read.

### The wall

- **Write** — `POST /channels/{cid}/typing` (`typing_ping`). Membership-gated (privacy-404). Optional JSON body; `draft` (the sender's compose text) is stored, and the handler emits **`SyncEvent::Typing { channel_id }`** on the bus — an id-only *nudge*, carrying no text. The draft itself stays in `typing_drafts`.
- **Read** — `GET /channels/{cid}/typing-drafts` (`typing_drafts`). **This permission-checked fetch is the *only* way draft text leaves the server.** It re-checks channel membership on every call (privacy-404), excludes the caller's own draft, resolves persona-first display names, and returns a bare JSON array of `TypingDraftEntry`.

So a `Typing` frame on the bus says only *"someone pinged typing in channel X, go fetch if you care"*; the fetch re-authorizes per call. The bus stays id-only even for the one feature with content to leak. Pinned by `tests/typing_drafts.rs::posted_draft_is_readable_by_another_member_and_never_by_its_author`.

### Endpoint behavior (the membership check carries the whole design)

The `*chan == cid` scoping clause in `typing_drafts` is, alongside the membership gate, the only thing between a member of an unrelated guild and *every draft on the instance* — the gate only covers the *requested* channel, and the map is process-global. Pinned cross-guild by `tests/typing_drafts.rs::draft_in_one_channel_never_appears_in_another_channels_fetch` and same-guild by `::draft_is_scoped_to_its_channel_even_within_the_same_guild`. Non-member access is the byte-identical privacy-404 the messages handler emits — `tests/typing_drafts.rs::typing_drafts_returns_privacy_404_with_identical_body_for_non_members`.

| Behavior | Rule | Pinned by (`tests/typing_drafts.rs::`) |
|---|---|---|
| Whisper-armed mask | `effect: "whisper"` → stored as fixed `(whisper)` placeholder (spoiler never enters the map) | `whisper_armed_draft_is_masked_to_the_fixed_placeholder_for_other_members` |
| Non-whisper / absent effect | `shout`/`spell`/absent kept plaintext; unknown ignored, never rejected | `non_whisper_and_absent_effects_keep_the_draft_plaintext` |
| Absent / empty `draft` clears | bare ping or `draft: ""` removes the entry (toggling pref off stops ghosting at the next ping) | `bare_typing_ping_still_succeeds_and_clears_any_stored_draft`, `empty_string_draft_on_ping_clears_the_stored_entry` |
| Over-cap truncation | `> 2000` chars truncated on a char boundary, never rejected (a mid-typing ping must not start failing) | `overlong_draft_is_truncated_to_the_cap_on_a_char_boundary` |
| Clear-on-send | send / roll / **edit** drop the author's draft (`clear_draft`) so no ghost lingers beside the landed message | `draft_is_gone_after_the_author_sends_the_message`, `::_rolls`, `::draft_is_gone_after_the_author_edits_a_message` (review M-02) |
| TTL prune | expired entries pruned on read/write | `expired_draft_is_pruned_after_the_injected_ttl` |

The `(whisper)` placeholder is **byte-identical** across the three body-preview surfaces — the typing-draft mask here, the reply-quote mask in `reading.rs` `MSG_PROJECTION`, and the push-payload mask (next section). The whisper-armed draft is masked **at store time**, so the spoiler text never even enters the in-memory map (review M-01).

---

## Web Push: background notifications (aes128gcm + VAPID)

`src/server/push.rs`. Polling could never fire a notification while a mobile PWA was backgrounded — the page's timers freeze, so the only state where a notification was *allowed* to show (`document.hidden`) was exactly the state where the code couldn't run. Web Push fixes this: the OS wakes the service worker via a `push` event even when the page is dead.

This is a parallel delivery channel to the SSE bus, not part of it — but it inherits the same content-masking discipline (whisper bodies are masked before they ride a payload onto a lock screen).

### Configuration & the disabled path

`PushSender` (one isahc client + VAPID private/public/subject) is built once at startup by `PushSender::from_env()` from `VAPID_PRIVATE_KEY` / `VAPID_PUBLIC_KEY` / `VAPID_SUBJECT`. With keys unset (tests, or env unconfigured) it returns `None` → `AppState.push` is `None` → **every push path is a silent no-op** and `GET /push/vapid-key` 404s so the client skips subscribing entirely. The subject must be a `mailto:`/`https:` URL (Apple's endpoint 403s otherwise); it defaults to a deployment mailto. Pinned by `tests/push.rs::vapid_key_is_404_when_push_unconfigured`.

### Endpoints

| Method · Path | DTO | Behavior |
|---|---|---|
| `GET /push/vapid-key` | → `VapidKeyResponse { key }` | public key, or 404 when push is unconfigured |
| `POST /push/subscribe` | `PushSubscribeRequest { endpoint, keys{p256dh, auth} }` | auth; validate non-empty fields (else 400); **upsert by unique `endpoint`** (DELETE-then-CREATE in one write-conflict-retried txn) → 204 |
| `POST /push/unsubscribe` | `PushUnsubscribeRequest { endpoint }` | auth; DELETE scoped by **both endpoint AND account** → 204 |

`subscribe` upserts on `endpoint` so a browser re-subscribing replaces its row rather than duplicating; the DELETE-then-CREATE is wrapped in `with_write_conflict_retry` (see [02-request-lifecycle](02-request-lifecycle.md)) so two concurrent re-subscribes (a service worker firing twice) converge on one row and both return the idempotent 204 instead of a 500 on the MVCC loser. Pinned by `tests/push.rs::subscribe_then_unsubscribe` (upsert keeps one row), `::incomplete_subscription_is_400`, `::subscribe_requires_auth`, and `::concurrent_subscribe_same_endpoint_converges_to_one_row`. `unsubscribe`'s account-scoping (account A can't delete account B's row) is pinned by `tests/push.rs::unsubscribe_is_account_scoped`.

### Send: `notify_new_message`

Fire-and-forget. `post_message` calls `notify_new_message(state, message_id, author)` and returns immediately; it early-outs when push is off, else spawns `notify_inner`, which never blocks or fails the send. `notify_inner`:

1. **Row read** — `load_notification_info(state, mid)` resolves the channel (id + name), optional guild, sender display (persona snapshot ?? live), body, **`effect`**, sender avatar id, and `pinged_keys` from the fresh message row. A vanished message (deleted between persist and notify) is the documented `Ok(None)` early-out, never an error.
2. **Recipients** — every `push_subscription` owned by a channel member who isn't the author. The membership table is picked by `guild_key` presence (single-table per query): `guild_member` for a guild channel, `dm_member` for a DM thread. A guild channel *also* unions in its active (unexpired) `channel_guest` cameos. (Mutes are client-side only — the server can't honour them; the payload carries the channel id so the client/SW could filter later.)
3. **Per-recipient send** — `send_one` encrypts (aes128gcm), VAPID-signs, and POSTs. The JSON payload carries `title` / `body` / `channel` / `guild` / `message` / per-channel `tag` (so a burst in one channel collapses into one notification window) / per-recipient `pinged` (true iff the message `@`-mentions that subscription's owner) / optional `image` (the persona avatar → `/media/{id}`). Body is capped at `MAX_BODY_CHARS` (120) to stay well under the push services' 4 KiB ciphertext cap.
4. **Prune dead** — endpoints the push service reports gone (HTTP 404/410, surfaced as `Ok(false)` from `send_one`) are batch-deleted.

`notify_inner` behind a live push service has no end-to-end test (it would need a real push endpoint), but its **load-bearing seam — the `effect` column surviving the SQL projection through to the masked body — is pinned from a real DB row** by `tests/push.rs::push_payload_row_read_carries_the_effect_column_from_a_real_whisper_row` (review M-42): an `Option<String>` that silently decoded to `None` because the projection line was dropped would put whisper plaintext on every lock screen while the in-module formatter unit tests stayed green. The masking itself (`notification_body`: `whisper` → `(whisper)`; empty → "sent an image"; else trimmed/truncated snippet) is unit-tested in-module (`src/server/push.rs` `tests::whisper_effect_masks_push_notification_body_with_fixed_placeholder`, `::non_whisper_effects_keep_the_normal_snippet_behavior`).

---

## Dev hot-reload: the `reload` frame

`src/server/dev_reload.rs` + `src/server/events.rs`. The test deck runs the **compiled** binary, so it has no cargo-leptos live-reload. When a new build is deployed there, an admin nudge tells every connected client to `location.reload()` onto the new version — over the *same* SSE bus.

- **`POST /admin/dev/reload`** (`dev_reload`) — `is_admin`-gated, **fail-closed**: an empty admin set authorizes no one (403); unauth is 401. Pinned by `tests/dev_reload.rs::dev_reload_is_403_for_non_admin` and `::dev_reload_requires_auth`. The admin-*allowed* path can't be driven through HTTP under parallel test workers (the `is_admin` env read races), so the broadcast logic is exercised directly through the auth-free `broadcast_reload(&state)` core, which simply `emit`s `SyncEvent::Reload`.

`Reload` is unique on three counts, all handled in the `/events` loop:

1. **Global & filter-bypassing** — it short-circuits *both* the per-connection visibility filter *and* the targeted lane, so it reaches every live connection regardless of visible-set or any `targets` list. It still passes the per-frame session re-check (a revoked session *ends*, never reloads). Pinned reaching a zero-guild (empty visible-set) connection by `tests/dev_reload.rs::reload_reaches_a_connection_with_no_visible_channels_as_a_named_frame`.
2. **A distinct named frame** — delivered as `event: reload` (constant `RELOAD_EVENT_NAME`), so the hydrate client listens for it *separately* from the generic `data:`-only message frames. The name lives in one place so server emit and client listener can't drift.
3. **Payload-free** — the frame's `data:` is a content-free `{}` sentinel (axum needs a non-empty data line to dispatch a named event). **The signal is the frame itself; nothing rides it** — the id-only invariant holds even here. Pinned by `tests/dev_reload.rs::reload_frame_is_payload_free`.

---

## Source map

Key files:
- `src/protocol.rs` — `SyncEvent` (always-on wire vocabulary) + `channel_id()`; push DTOs (`VapidKeyResponse`, `PushSubscribeRequest`, `PushSubscriptionKeys`, `PushUnsubscribeRequest`); `TypingPingRequest`, `TypingDraftEntry`.
- `src/server/state.rs` — `AppState`, `BusEvent`, `emit`/`emit_for`, the `events` broadcast sender (cap 256), `typing_drafts` store, `DEFAULT_DRAFT_TTL` / `DEFAULT_SSE_RECHECK_PERIOD` + their `with_*` injectors.
- `src/server/events.rs` — `GET /events`: the `unfold` delivery loop, per-connection visibility filter, per-frame + deadline session re-check, targeted lane, the named `reload` frame; in-module DB-error fault-injection unit tests.
- `src/server/access.rs` — `visible_channels` / `VisibleChannel`: the per-connection privacy boundary (guild member ∪ DM member ∪ unexpired guest).
- `src/server/messages/typing.rs` — Ghost Quill: `typing_ping` (write + id-only `Typing` emit), `typing_drafts` (the only draft-text egress), `clear_draft`, whisper mask, char-boundary truncation.
- `src/server/push.rs` — Web Push: `PushSender`/`from_env`, subscribe/unsubscribe/vapid-key, `notify_new_message` → `notify_inner`, `load_notification_info`, `notification_body` mask.
- `src/server/dev_reload.rs` — `POST /admin/dev/reload` (admin-gated) + the auth-free `broadcast_reload` core.
- `src/server/mod.rs` — route registration (`/events`, `/channels/{cid}/typing`, `/channels/{cid}/typing-drafts`, `/push/*`, `/admin/dev/reload`).

Tests that pin its claims:
- `tests/sync_events.rs` — `SyncEvent` wire shapes, `channel_id()` semantics, the `Unknown` forward-compat fallback.
- `tests/events.rs` — `/events` delivery, per-connection privacy filtering (grow + shrink/revocation), targeted-lane isolation, the broadcast-vs-targeted `lists_changed` distinction, mid-stream session revocation (per-frame + quiet + invisible-traffic deadline), no-op-restore non-emit.
- `src/server/events.rs` (in-module `#[cfg(test)]`) — fail-closed visibility reload, fail-closed session re-check, initial-load-500, pre-stream 401.
- `tests/typing_drafts.rs` — Ghost Quill readability/own-exclusion, privacy-404, channel scoping (cross- + same-guild), whisper mask, clear semantics, truncation, TTL prune, clear-on-send/roll/edit.
- `tests/push.rs` — subscribe/unsubscribe lifecycle + upsert, validation 400, account-scoped unsubscribe, concurrent-subscribe convergence, vapid-key 404, the effect-column row-read → masked body (review M-42).
- `tests/dev_reload.rs` — reload reaches an empty-visible-set connection as a named frame, payload-free frame, admin/auth fail-closed gate.

See also: [02-request-lifecycle](02-request-lifecycle.md) (write-conflict retry, extractors), [05-auth-privacy](05-auth-privacy.md) (session model, privacy-404, `is_admin`), [03-data-model](03-data-model.md) (`push_subscription`, membership tables), [reference/rest-api](../reference/rest-api.md).
