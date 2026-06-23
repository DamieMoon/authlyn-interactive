# 07 — UI shell (the hydrate browser architecture)

The browser-side app: a Leptos client shell that renders the authed product
(guilds → channels, personas, lorebooks, friends, DMs, cameos) and drives every
mutation through a same-origin REST client and an `EventSource` realtime
consumer. This document is the entry-point + data-flow map; per-function detail
is the rustdoc on each item.

Scope: `src/ui/mod.rs`, `src/ui/shell/` (the `Shell` state aggregate, the
`AppShell` mount, the `act/` dispatch layer, the channel view, the `sk_orbit`
spatial shell), `src/ui/modal.rs` (the shared dialog primitive), and
`src/client/api.rs` (the gloo-net REST client). The always-on AST→view
renderer (`src/ui/markup_view.rs`) and the rest of the shared primitives are in
[06 — markup engine](./06-markup-engine.md); the wire DTOs are in
[03 — data model](./03-data-model.md); the SSE *server* is in
[04 — realtime SSE](./04-realtime-sse.md); CSS/chrome is in
[08 — styling & chrome](./08-styling-chrome.md).

For the stack, the cargo-leptos config, and every dependency's per-graph
purpose, read `Cargo.toml`'s `#`-comments and `README.md` — not this file.

---

## 1. Graph discipline (why everything is doubled)

The UI compiles into **two** of the three disjoint feature graphs
([01 — overview](./01-overview.md)): **ssr** (server render, never wasm) and
**hydrate** (browser/WASM, never the server runtime). `src/ui/mod.rs:1-3` states
the rule the whole subtree obeys:

> all data-fetching lives in `#[cfg(feature = "hydrate")]` blocks so the
> gloo-net client never enters the ssr graph (the bodies are empty closures
> under ssr).

The mechanism, applied everywhere:

- **`act/` action functions** (`src/ui/shell/act/`) are defined **twice** —
  a `#[cfg(feature = "hydrate")]` real impl and a `#[cfg(not(feature =
  "hydrate"))]` no-op twin with the same signature. The `view!` calls them
  ungated; ssr gets a typechecking no-op, so `src/client/api.rs` (gloo-net,
  hydrate-only) is never referenced from the server build. `start_sync`'s twin
  pair at `src/ui/shell/act/sync.rs:137` (real) / `:755` (stub) is the model.
- **Pointer/gesture engines** pair a hydrate-real struct (touches `web_sys`)
  with an ssr stub whose methods are no-ops, so the always-on `view!` can bind
  `on:pointerdown`/etc. ungated. `modal::SwipeClose` (`src/ui/modal.rs:109`
  real / `:260` stub) and every `sk_orbit` drag engine follow this.
- **Pure decision functions** (no DOM) are **un-gated** and compile in all
  three graphs, which is what lets them carry the unit tests — the project has
  no WASM UI-test harness, so gesture/geometry correctness lives in
  `#[cfg(test)]` modules run under the ssr `test` cfg. Examples:
  `modal::swipe_close_lock`, every `sk_orbit::{strip,charge,orbit_map,
  pane_swipe,warp}` math fn, `channel::lightbox` transform math,
  `channel::message_actions`.

Consequence for readers: a behavioral claim about the shell is pinned by a unit
test **only** when the logic was extracted into a pure fn. DOM/effect behavior
(focus restore, scroll pinning, the SSE driver's reconnect machine) is
**browser-only** and verified by code + the owner deck-pass, not `tests/*.rs`
([09 — testing](./09-testing.md), and the *UI fidelity* section of
`CLAUDE.md`).

---

## 2. Entry point + auth gate

`Home` (`src/ui/shell/mod.rs:52`) is the authed route's component. It reads the
`AuthCtx` provided once at the app root:

```rust
// src/ui/mod.rs:25
pub struct AuthCtx {
    pub user: RwSignal<Option<MeResponse>>, // None until /auth/me resolves
    pub loading: RwSignal<bool>,            // gates the first paint
}
```

`MeResponse` is the session resolved server-side from the cookie; the client
**never** supplies identity ([05 — auth & privacy](./05-auth-privacy.md)). The
sole resolver is `api::current_user()` (`GET /auth/me`), called from the App
root mount (see [02 — request lifecycle](./02-request-lifecycle.md)).

`Home` does two things:
- a hydrate-only `Effect` redirects to `/login` once `loading` is false and
  `user` is `None` (`src/ui/shell/mod.rs:55-60`);
- a `<Show when=is_authed>` renders `AppShell`, falling back to a `Loading…`
  splash while `loading` is true — the loading gate prevents an unauthed flash
  of the shell before `/auth/me` lands.

---

## 3. Global state: the `Shell` aggregate

`Shell` (`src/ui/shell/mod.rs:112`) is a `Clone + Copy` handle bundling **10**
reactive sub-structs. Every field is an `RwSignal<T>` (itself `Copy`), so the
whole `Shell` is a cluster of pointer-sized signal IDs — cheap to pass by value
into every `act::*` call and every component.

`AppShell` (`src/ui/shell/mod.rs:126`) constructs each sub-struct, calls
`provide_context::<T>(t)` for each, **then** assembles the flat `Shell` and
`provide_context`s that too (`state.rs:1-20`). The dual provision is deliberate
(M6/C8): a deep pane component can pull just the slice it needs
(`use_context::<Selection>()`) while `act::*` keeps taking the full `Shell` so
action signatures stay short.

| Sub-struct | Owns | Key fields (selected) |
|---|---|---|
| `Selection` | server/channel selection + the lists they live in | `guilds`, `sel_server`, `sel_owner`, `channels`, `guild_channels` (per-guild cache, powers cross-guild unread badges), `guild_emoji`, `sel_channel`, `dms`, `cameos` |
| `MessageView` | open channel's messages + pagination + live typists | `messages`, `cursor`/`oldest`/`more_history`/`loading_older` (3-cursor pagination), `loading_initial`, `anchor_to` (post-prepend re-anchor), `seen` (dedupe set), `typing`, `ghost_drafts` (Ghost Quill), `new_divider` (re-entry baseline) |
| `Composer` | compose box state | `compose`, `compose_attachments` (`StagedAttachment` w/ upload lifecycle), `status` (the error `<p>`), `drafts` (per-channel), `replying_to`, `editing`, `sent`+`sent_gen` (send-pulse generation), `effect_mode` (whisper/shout/spell, per-message) |
| `SyncState` | background-sync + pane selection | `polling` (driver latch), `sse_live` (the LIVE/POLLING chip), `me` (account id mirror), `pane`, `wardrobe_open`, `map_open`, `switching` (warp transition) |
| `Social` | friends + wardrobe + worn persona + lore | `friends`, `personas`, `active_persona`, `lore` |
| `Modals` | the destructive-action confirm | `pending_delete` (a `PendingDelete` *as data*, not a closure), `confirm_prompt` |
| `Notify` | mute/unread/last-seen tracking | `muted`, `unread`, `last_seen` (per-channel high-water mark), `web_push_enabled`, `scroll_marks` (re-entry scroll memory) |
| `Trash` | soft-deleted overlays | `deleted_channels`, `deleted_messages`, `show_msg_trash` |
| `Prefs` | per-user prefs (localStorage-mirrored) | `dialogue_style`, `ghost_quill`, `haptic_vibrate`, `skeleton` (see §8) |
| `Toasts` | the one-at-a-time toast capsule | `current` (`Option<Toast>`) |

Definitions and per-field invariants are in `src/ui/shell/state.rs` (every
field carries a `///` rationale). Three patterns recur and matter:

- **Actions described as data, never closures.** `PendingDelete`
  (`mod.rs:98`) and `ToastAction::UndoMessageDelete` (`state.rs:373`) encode a
  deferred action as an enum, because a closure can't ride a signal. The
  confirm modal and the toast host dispatch by `match` (`act::confirm_delete`,
  `act::run_toast_action`).
- **Generation counters for detached timers.** `Composer::sent_gen`
  (`state.rs:191`) and `Toast::key` mint a per-event generation so a detached
  reset timer only clears *its own* state — an earlier send's 400ms pulse timer
  can't truncate a later send's pulse. The same pattern guards the SSE driver
  (§6) and the radial menu.
- **Client-only transient flags never serialize.** `loading_initial`,
  `switching`, `sent`, `UploadStatus`, the whole `Toast` — none are sent or
  persisted; the wire shape the server sees is untouched.

`AppShell`'s mount `Effect` (`src/ui/shell/mod.rs:331-369`) is the boot
sequence; order is load-bearing: `refresh_guilds` → (deep-link **or**
`restore_session` **or** `show_friends`) → `start_sync` → `load_muted` →
`load_last_seen` (localStorage) → `hydrate_last_seen` (server cursors overlay,
L-1 cross-device) → wake/notification-click listeners. The same component also
builds the `EmojiResolver` memo from `guild_emoji` and provides it to the whole
subtree so the markup renderer resolves `:shortcode:` without parameter
threading ([06 — markup engine](./06-markup-engine.md)).

---

## 4. The `act/` dispatch layer

`src/ui/shell/act/mod.rs` is the action layer: ~16 submodules, each
hydrate-real + ssr-stub co-located, re-exporting their public fns so the view
keeps calling `act::xxx` unchanged. The module is `pub` (not `mod`) **only** so
`tests/skeleton_switch.rs` can reach the pref helpers at the stable path
`ui::shell::act::*`; `Shell`/`PendingDelete`/`ToastAction` stay `pub(crate)`,
so no external crate can name them — `#![allow(private_interfaces)]` silences
the resulting lint noise (`act/mod.rs:31-41`).

| Submodule | Responsibility |
|---|---|
| `prefs` | localStorage toggle helpers + the skeleton id surface (`SKELETON_IDS`, `is_valid_skeleton`, `set_skeleton`) |
| `account` / `admin` | logout (calls `sync::shutdown`), password/security, account avatar; system broadcast |
| `guild` / `channel` | rail + sidebar: refresh/swap/open/create/rename/delete/restore; `channel` also owns `open_channel`, deep-link, session restore, `show_orbit_map`, drag reorder |
| `dm` / `cameo` | DM threads (M7/P1) + guest cameos (M7/P2): refresh/open/create/invite/leave |
| `message` | the largest: compose/edit/delete, 3-cursor pagination, the poll fallback + sync/ingest/reconcile primitives, mute/last-seen, friends + lore + member ops, the destructive-confirm dispatcher |
| `sync` | the SSE driver (`start_sync`) + self-healing poll fallback (§6) |
| `persona` | wardrobe: create/update/remove/leave/swap/share/avatar + wear/unwear |
| `reentry` | the NEW-divider baseline, date-separator labels, per-channel scroll memory (pure fns + localStorage/DOM capture) |
| `toast` | push / keyed-dismiss + the action dispatcher (UX evolution #11) |
| `emoji` | guild custom-emoji refresh/create/delete + image upload |
| `notify` | Web Notifications + Web Push (the reflection-heavy subscribe path) |
| `compose_colors` / `haptics` | composer color-swatch history; visual-haptic fire helper |

### Action shape (the contract every fn obeys)

A mutating action is synchronous-looking but spawns the network call:

1. read inputs from signals **untracked** (`get_untracked`) — an action is not
   a reactive computation;
2. optimistically update local signals where it improves perceived latency
   (e.g. `delete_message` hides the row at once);
3. `spawn_local` the `api::*` call;
4. on success, reconcile; on `Err`, surface `api::humanize(&e)` into
   `composer.status` (the red `<p>`) or an error toast.

**Disposal safety (review M-10) is mandatory.** Logout disposes the `Shell`'s
signals, but `spawn_local` futures, `forget()`-ed event closures, and detached
timers outlive it. So **every** access *after* an `await` (and every access
from a forgotten closure) uses `try_set` / `try_get_untracked` / `try_update`:
a disposed shell returns `None` and the action bails — a plain `.set()` on a
disposed signal is a panic, which in WASM is a process abort. `delete_message`
(`src/ui/shell/act/message.rs:627`) shows the discipline end-to-end:
`try_update` on `seen` doubles as the disposal *proof* before the toast push.
This pattern is pinned only by code review + clippy, not a runtime test.

---

## 5. The REST client (`src/client/api.rs`, hydrate-only)

The entire file is gated `#[cfg(feature = "hydrate")]` at `src/client/mod.rs:8`.
It wraps `gloo-net` Fetch; every call shares one envelope:

- **Same-origin → the session cookie rides automatically.** Callers never set
  auth headers; identity is re-derived server-side. This is the client side of
  the server-trusted invariant ([05 — auth & privacy](./05-auth-privacy.md)).
- **Status → typed result.** A thin transport layer at the bottom of the file
  (`get`, `post_json`, `post_empty`, `post_json_empty`, `delete_empty`,
  `put_json`, `put_empty`, `patch_json`) funnels everything through
  `decode`/`decode_empty`, which on 2xx parse the body (or `()`), and otherwise
  lift the server's `{"error": "..."}` into `ApiError::Status(code, msg)`.
- **`ApiError`** has three arms: `Network` (no response — offline/DNS/CORS),
  `Status(u16, String)`, `Codec` (unexpected body shape). `humanize(&e)`
  produces the user-facing string; `e.status()` exposes the code (callers treat
  e.g. `404` push-key as "push unavailable").

One function breaks the gloo-net mold: **`upload_media_with_progress`**
(`src/client/api.rs:674-772`). gloo-net's Fetch transport exposes no
upload-progress event, so this drives a raw `XMLHttpRequest`, subscribes to
`xhr.upload.onprogress`, and bridges XHR's event-driven completion to
`async`/`await` through a `js_sys::Promise` whose `resolve`/`reject` are stashed
in `Rc<RefCell<Option<Function>>>` cells and called from the `load`/`error`/
`abort` handlers. It manually re-implements `decode`'s `{"error":..}` lift
because it bypasses the Fetch path. `upload_media` is the no-progress wrapper.
This is the single most intricate transport in the subsystem.

Functions are grouped by domain (auth, guilds+channels, messages, read-state,
trash, personas/wardrobe, media, lorebook, friends, DMs, cameos, custom emoji,
Web Push, feedback, admin). The full route↔fn↔DTO matrix is in
[reference/rest-api.md](../reference/rest-api.md); the server handlers are in
[02 — request lifecycle](./02-request-lifecycle.md).

---

## 6. Realtime: the SSE consumer + self-healing fallback (`act/sync.rs`)

`start_sync` (`src/ui/shell/act/sync.rs:137`) is the background-sync driver,
idempotent via the `SyncState::polling` latch and called once at shell mount so
the lists stay live before any channel opens.

**Strategy.** Open an `EventSource` on `/events`; react per frame. If the
constructor fails (ancient browser), fall back to `message::start_poll` for the
session (the chip reads `● POLLING`). The `/events` server re-checks the
session every frame and on a keepalive ([04 — realtime SSE](./04-realtime-sse.md)).

**The bus is id-only.** Frames carry a `SyncEvent`
(`src/protocol.rs:1032-1092`) that is **content-free** — variants hold at most a
`channel_id`. `dispatch` (`src/ui/shell/act/sync.rs:572`) reacts *only* by
re-fetching through the existing permission-checked endpoints, so the event
stream is never an authorization surface. In particular, **draft text never
rides `/events`**: Ghost Quill draft content is fetched via the
permission-checked `GET /channels/{cid}/typing-drafts` (`api.rs:387`) and only
when the *receiver's* pref is on; the typing ping pre-masks whisper drafts
before the server ever stores them (`api::post_typing`, `api.rs:363`).

> **Invariant — SSE bus is id-only.** `SyncEvent` has no content fields and
> `dispatch` is refetch-only. Pinned by `src/protocol.rs:1032-1092` (the enum
> shape) + `tests/sync_events.rs::sync_event_serializes_with_snake_case_type_tags`,
> `tests/sync_events.rs::targeted_sync_events_pin_their_wire_shape`,
> `tests/sync_events.rs::reload_sync_event_is_a_bare_global_tag`.
> Server-side delivery + the per-frame session re-check are pinned by
> `tests/events.rs` (server graph). The `dispatch` refetch-only behavior is
> code-pinned at `src/ui/shell/act/sync.rs:572-657` (no WASM runtime test).

**`dispatch` routes to the cheapest sufficient refresh:**

| Event | Reaction |
|---|---|
| `Unknown` (`#[serde(other)]`) | dropped — forward-compat contract |
| `ListsChanged` | `resync_truth` (lists + unread + open channel — it is also the server's post-lag resync nudge) |
| `ReadStateChanged` | `refresh_unread` (a read cursor moved on another device) |
| `FriendsChanged` | `refresh_lists` |
| channel-scoped, **open** channel, `Typing` | `refresh_typing_surface` only (a ping can't carry a new message — review M-13) |
| channel-scoped, **open** channel, other | `refresh_open_channel` + advance last-seen + Ghost-Quill refetch |
| channel-scoped, **non-open** channel, non-`Typing` | `refresh_unread` only (typing in a background channel can't change unread — kills the per-ping `/unread` storm) |

**Self-healing (UX evolution #2).** The poll fallback is **not terminal**:

- A **generation counter** (`SYNC_GEN`, `sync.rs:90`) is bumped on every
  handover (mount connect, probe promotion, demotion). Each driver — an
  EventSource's handlers, the poll loop, the backoff task — captures the
  generation it was installed under and self-terminates the moment the global
  moves on, so a handover can never leave two drivers running.
- After a `demote` (`sync.rs:403`, reached at `MAX_CONSECUTIVE_SSE_ERRORS = 5`
  consecutive errors with no intervening message, or by `wake` finding the
  stream dead), a **capped-backoff** task (`start_retry`, 30 s → 5 min, ±20%
  jitter) probes `/events`. A successful probe `onopen` **promotes** itself to
  the driver via a generation bump and runs a one-shot `resync_truth` (the
  polling era's events never rode this stream).
- **Wake listeners** (`install_wake_listeners`, `sync.rs:527`) on
  `document.visibilitychange`→visible and `window.online` funnel into `wake`
  (`sync.rs:479`): refetch the throttled truth, and — the **frozen-PWA** case —
  if `sse_live` is true but `EventSource.readyState == CLOSED` (a mobile PWA
  killed the connection without ever firing an error), `demote` + `probe`
  immediately so the chip stops lying.
- **`PROBE_PENDING`** is a single-flight slot; a probe wedged past
  `PROBE_TIMEOUT_MS = 15 s` (the frozen-PWA model again, killing a CONNECTING
  probe) is reaped and replaced (review M-30).
- The `● LIVE` chip (`sse_live`) is set **only** on a current-generation
  `onopen` and dropped on every error/demotion/dead-stream, so it never lies
  through a transition.
- **`shutdown`** (`sync.rs:169`) is logout's teardown: bump the generation
  (retiring every outstanding driver/task at its next tick), close the promoted
  + probe streams, release the probe slot. The forgotten closures then no-op
  forever via their `try_`-read of the disposed shell.

The `EventSource` also listens for a **named `event: reload`** frame
(`sync.rs:369-382`) — distinct from `onmessage` — emitted by the test deck on a
new deploy; its arrival (id-only, payload ignored) triggers `location.reload()`
onto the fresh bundle (the deck runs the compiled binary, no cargo-leptos
live-reload).

> **Invariant — disposal-safe driver.** Generation-guarded handover + `try_`
> teardown. Code-pinned at `src/ui/shell/act/sync.rs` (module doc :19-25,
> `shutdown` :169-178, the generation/`try_` guards in `connect` :220-237 and
> `dispatch` :572-579). The reconnect/frozen-PWA failure modes are mobile-only
> and **(unpinned)** by any `tests/*.rs` — verified by code + deck-pass.

`refresh_open_channel`, `start_poll`, `ingest`, and the newest-window reconcile
(`plan_window_reconcile` / `reconcile_newest_window`, `message.rs:1693`/`:1740`)
are the refresh primitives `dispatch` and the poll loop drive; the pure
reconcile core is pinned by
`message.rs::plan_window_reconcile_removes_only_in_window_rows_the_server_dropped`,
`…_patches_an_edited_in_window_row`, and
`…_never_resurrects_an_optimistically_hidden_row` (co-located unit tests).

---

## 7. The channel view (`src/ui/shell/channel/`)

`ChannelPane` (`src/ui/shell/channel/mod.rs:515`) is the message-list + composer
component; it pulls `Shell` and `AuthCtx` from context. Pure-helper carve-outs
live in siblings: `avatar` (`chat_avatar`, `format_local_time`), `attachments`
(`attachment_grid`), `emoji_suggest` (the `:`-autocomplete + picker buttons),
`lightbox` (the near-fullscreen viewer + its gesture engine), `meta`
(message/system meta rows), `radial` (the touch long-press menu), `manager`
(the channel manager), `skeleton` (loading rows).

Key data-flow + behaviors:

- **`message_actions(kind, mine)`** (`mod.rs:137`) is the **single** per-kind
  action predicate, shared by the hover meta row (`meta.rs`) and the touch
  radial (`radial.rs`) so the two surfaces can't drift. Conservative:
  `kind='user'` → reply+copy+(edit/delete if mine); `roll` → reply+copy only
  (the server 403s edit/delete on rolls, cheating-proof); `system` → nothing;
  unknown/future kind → reply+copy, never edit/delete. `count()` picks the
  radial's n2/n4 arc; `0` means never arm.
  *Pinned by* `mod.rs::message_actions_offers_nothing_mutable_outside_kind_user`
  (table-driven) + `mod.rs::message_actions_count_drives_the_radial_arms`.
- **Lightbox transform math** (`lightbox.rs`) — `clamp_translate`, `zoom_about`,
  `pinch_update`, `double_tap_target`, `should_dismiss` — keeps the anchored
  image point stationary under zoom, clamps so a zoomed edge never leaves the
  viewport, and degrades pinch→single-finger identically on `pointerup` and
  `pointercancel` (review M-35). *Pinned by* the 13 co-located tests
  (`clamped_translate_*`, `pinch_keeps_*`, `zoom_about_keeps_*`,
  `double_tap_*`, …).
- **Overlays portal to `document.body`** via `<Portal>` (M5/P0 #54): the
  radial, the lightbox, and the mobile emoji sheet mount to the body so
  `.content` can stay transform-free and never trap them as a CSS containing
  block (`mod.rs:44-47`). The warp-dip class lives on the inner `.channel-view`
  wrapper, not `.content`, for the same reason. `is_mobile_viewport()`
  (`mod.rs:306`) gates the emoji-sheet portal: a `position: fixed` bottom sheet
  belongs on the body, the desktop `position: absolute` popover must stay
  anchored to the composer.
- **Auto-scroll / unread append** (`mod.rs:709-817`) is the trickiest effect:
  thresholds (your own message → follow when within 120px of bottom; others' →
  only at ≤4px), `prev_count` diffing to tell genuine appends from the initial
  load and from in-place edits/deletes, and a **triple re-pin**
  (`TimeoutFuture(0)` + double-rAF + `document.fonts.ready`) so a late
  self-hosted-font reflow can't leave the channel opened mid-history. An
  older-history prepend sets `anchor_to` and is handled by a separate anchor
  effect (`mod.rs:822-847`) that scrolls the previously-top row back into view.
- **Composer mechanics.** `apply_markup` (`mod.rs:332`) splices wrap markers
  around the textarea selection in UTF-16 space (selection ranges are UTF-16)
  and defers the caret-set a tick so it survives Leptos rewriting
  `prop:value`. A `field-sizing: content` feature-detect (`mod.rs:407`)
  short-circuits the JS auto-grow on modern browsers (the Android-shake fix); a
  `ResizeObserver` mirrors the composer band's real height into `--composer-h`
  on `<html>` so the toast host and the channel floats anchor to the composer's
  actual top edge (UX evolution #11 placement contract). Both effects wrap
  their non-`Send` cleanup state in `send_wrapper::SendWrapper`.
- **Whisper veil** (`mod.rs:1021-1219`): the blurred body/media is a real
  disclosure toggle — focusable, `aria-expanded`, Enter/Space operated — that
  while hidden sits outside the a11y tree (`aria-hidden` + `inert`) and on
  reveal *sheds* its `role="button"` (ARIA button is children-presentational
  and forbids interactive descendants). The veiled preview everywhere (reply
  quote, trash row, push body) shows the fixed `(whisper)` mask, never the
  spoiler.

Channel-pane-local signals (emoji/color popover toggles, `revealed` whisper
set, `charge`, lightbox/radial state, scroll-aid signals) deliberately stay
component-local rather than riding `Shell` — they reset on remount and don't
concern other surfaces.

---

## 8. The `sk_orbit` spatial shell (`src/ui/shell/sk_orbit/`)

`SkOrbitShell` (`src/ui/shell/sk_orbit/mod.rs:143`) is the **sole + default**
shell for v27 (owner ruling 2026-06-17): full-viewport channel panes in a
horizontal swipe strip, a holographic channel pill that opens a zoomable
orbit-map picker, a floating composer orb with a length-charged send ring +
effect blossom, and a right-edge HoloPanel station slide-over. It renders
**zero new state** — it reuses every pane via `use_context::<Shell>()`, so a
skeleton switch (when deck/hud ship) is a pure `.app.sk-*` class toggle with no
remount (§ below).

**Pure math vs. view.** The gesture/transition *decisions* are pure fns in the
submodules — `strip` (axis-lock, commit, 3-slot offset, single-channel
collapse), `charge` (length→ring fraction, dash offset), `orbit_map`
(viewport→geometry, seeded Kepler orbits), `pane_swipe` (back-dismiss
threshold), `warp` — each unit-tested under the ssr `test` cfg. The `view!`
that consumes them is feature-gated where it touches `web_sys`.

Load-bearing orbit invariants and their pins:

- **Swipe strip is a fixed 3-slot DOM, live `ChannelPane` always the middle
  slot** → resting offset is `-width` regardless of the channel's list index;
  `idx`/`count` only gate the rubber-band at true edges. `collapses_to_single`
  is `== 1` only (`count == 0` is the transient far-dive load window and must
  keep multi geometry). *Pinned by* `strip.rs::strip_offset_resting_base_is_
  one_slot_regardless_of_list_index`, `…_rubber_bands_only_at_true_edges`,
  `commit_swipe_by_displacement_or_velocity`,
  `strip_collapses_only_for_a_genuinely_single_channel_guild`. A committed swipe
  resolves the neighbor index via `strip::commit_target` and calls
  `act::open_channel`.
- **Per-guild channel orbits are LOCKED across renders**: every orbit parameter
  derives from `guild_seed ^ seed_of(channel_id)` (hand-rolled FNV-1a, never
  std-RNG), inner channels orbit faster (Kepler T ∝ r^1.5), ~17% retrograde.
  *Pinned by* `orbit_map.rs::channel_orbit_is_locked_per_channel`,
  `inner_channels_orbit_faster_than_outer`, `retrograde_is_stable_per_channel`,
  `some_channels_orbit_retrograde`, `orbit_radius_clamps_on_the_poco_c3_floor`.
- **Composer-orb charge fills by message *length***: `charge::charge_fraction`
  is a log curve (one-liner = sliver, paragraph ~60%, only a saga pegs); the orb
  is the sole send surface under orbit (the in-pane `.send` ring is hidden in
  SCSS). *Pinned by* `charge.rs::one_liner_shows_a_sliver_paragraph_mid_saga_
  pegs`, `fraction_is_monotonic_in_word_count`,
  `dash_offset_maps_empty_to_full_circ_and_full_to_zero`.
- **Pane back-dismiss threshold equals the wardrobe modal's.** `PaneSwipe`'s
  28%-of-viewport rightward commit must match `modal::SWIPE_CLOSE_FRACTION` so
  the two gestures feel identical; the modal const is private, so the value is
  duplicated and *pinned by* `pane_swipe.rs::pane_back_fraction_matches_the_
  wardrobe_slide_over` (+ `pane_back_commits_only_rightward_past_threshold`).

**Composer choreography.** Under orbit the orb is a *compose trigger*, not a
send: a tap reveals the composer (`.composing` on the content section) and hides
the orb; the in-composer send commits. Bridges (`mod.rs:173-207`):
`composing` collapses on the shared `s.composer.sent` pulse `after_send_success`
fires (so the composer doesn't stay open over your own just-sent message — a
failed send never pulses, so it stays open to retry); and the **inverse** —
`act::start_edit`/`act::start_reply` set `editing`/`replying_to`, which an
Effect reads to *reveal* the composer and focus it (those actions' own focus
call targets the still-hidden at-rest composer, a no-op).

**Context-bleed gating (2026-06-22).** The pill, the compose orb, and its
blossom/scrim are **channel-only** affordances — gated to `Pane::Channel` so
they don't bleed onto Friends/Members/Emoji/etc. (which have no message input).
The dispatch panes (Friends/Members/Emoji/Lorebook/DMs/Cameos) mount in a
shared swipe-back wrapper (`pane_swipe`) with a wardrobe-paradigm
`.account-head` bar carrying the title + a back-arrow; a rightward swipe pops to
the orbit map (DMs/Cameos pop to Friends first). The Channel pane is **not**
wrapped — it owns the horizontal strip, and back-swiping a channel is a neighbor
switch, not a dismiss.

**Modal-parity focus traps (design law §13).** The orbit map, the help overlay,
and the founding (create-server) dialog are `aria-modal` portals over a still-
focusable scrimmed shell, so each wraps Tab/Shift+Tab within its own focusables
(the shared `focusables`/`trap_tab` helpers, `mod.rs:72`/`:102`), closes on Esc,
focuses the dialog on open, and restores focus to its trigger on close (WCAG
2.4.3). The orbit map's trigger is the pill. The effect blossom is a
`role="menu"` with roving-tabindex arrow nav + an ArrowUp/Down keyboard open
path off the orb (the orb being the sole effect surface, a pointer-hold-only
open would lock keyboard/AT users out).

The orbit shell composes the `holopanel` primitive (`Edge`, `Detent`,
`HoloPanel`, `src/ui/shell/holopanel.rs`) for the station slide-over; that
panel's own pure drag math (`progress_from_delta`, `commits_open`,
`nearest_detent`) is unit-tested there.

### Skeleton is forced, the surface is vestigial-but-pinned

`Prefs::skeleton` is forced to `Some("orbit")` unconditionally at shell init
(`mod.rs:223-228`); the M3 rail/sidebar fallback was retired and there is no
chooser. The `skeleton` signal and the `act::prefs` surface
(`SKELETON_IDS = ["orbit","deck","hud"]`, `SKELETON_FALLBACK = "orbit"`,
`is_valid_skeleton`, `set_skeleton`, `skeleton_pref`) are kept so re-enabling a
chooser when deck/hud land is a known-good surface.

> **Invariant — skeleton id surface stays honest, and a switch never remounts.**
> `set_skeleton` accepts only valid ids and touches *only* a persisted string
> (no SSE/composer/selection state), because the skeleton lives on the same
> stable `Prefs` aggregate / same `.app` root that carries `fx-max` — switching
> it is a pure class toggle. Pinned by
> `tests/skeleton_switch.rs::skeleton_ids_are_exactly_the_three_shells`,
> `…::valid_skeleton_accepts_known_rejects_unknown`,
> `…::fallback_is_a_valid_id`,
> `…::ssr_stubs_signal_no_pref_and_no_storage`,
> `…::set_skeleton_surface_is_pref_only`. The live no-remount guarantee on a
> real device is a documented Phase-7 gate, **(unpinned)** until a second
> skeleton ships (`tests/skeleton_switch.rs:63-69`).

---

## 9. The shared modal (`src/ui/modal.rs`)

`Modal` is the shared `.modal-backdrop` + `.modal` dialog, replacing six
hand-rolled backdrops. It provides the a11y the audit required: `role="dialog"`,
`aria-modal="true"`, Esc-to-close, a Tab/Shift+Tab focus-trap, initial focus on
the first focusable, and **focus restoration** — the element focused at mount is
captured and re-focused on cleanup for *every* dismiss path (Esc, backdrop,
close button, swipe). The captured element is a wasm `HtmlElement`, which is
`!Send`, so it crosses the `StoredValue`/`on_cleanup` boundary wrapped in
`send_wrapper::SendWrapper` (`src/ui/modal.rs:343-384`). WASM is
single-threaded, so the wrapper's same-thread assert always holds.

`PersonaInfo` and `ModalHead` are the two shared sub-components (the read-only
persona profile-peek, and the sticky glass head with the 44px `.row-edit` close
disc — rendered as a back-arrow under the orbit slide-over).

**Opt-in swipe-to-close** (`swipe_close` prop): under orbit a `Modal` is a
full-screen slide-over closed by a rightward drag. The decision logic is two
pure fns:

- `swipe_close_lock(dx, dy)` — locks the gesture to an axis past a 12px slop
  with 1.2× horizontal dominance;
- `swipe_commits_close(dx, viewport_w)` — commits to close only on a rightward
  `dx > viewport_w * 0.28`; leftward/short never closes.

The drag reuses the *same* caller `close`, so focus-restore is unchanged; the
gesture is purely additive presentation, the a11y machinery untouched. The
`SwipeClose` engine bails `set_pointer_capture` when the press starts on an
interactive control (`closest("button, a[href], input, …, [role=button]")`) —
without it, a captured pointer steals the trailing desktop click and the
in-modal buttons go dead (the M6 desktop regression).

> **Invariant — swipe-close geometry.** Pinned by
> `src/ui/modal.rs::swipe_lock_needs_dominant_horizontal_past_slop` and
> `src/ui/modal.rs::swipe_commits_only_rightward_past_threshold` (pure-fn unit
> tests, ssr `test` cfg).

> **Invariant — pointer engines bail capture on controls.** All four
> whole-surface engines (`modal.rs`, `sk_orbit/drag.rs`, `sk_orbit/pane_swipe.rs`,
> `holopanel.rs`) share the bail selector. Pinned by the static scan
> `tests/style_lint.rs::swipe_engines_bail_pointer_capture_on_controls`.

> **Invariant — focus restoration on every dismiss.** Code-pinned at
> `src/ui/modal.rs:343-384`; browser-only, **(unpinned)** by any `tests/*.rs`
> (verified by code + deck-pass).

Two more shell-wide UI rules surface here and are statically pinned:

- **No HTML5 drag-and-drop anywhere** (iOS WebKit has none) — reorder uses the
  pointer-capture grip pattern, and any `draggable=` must be the literal
  `"false"`. Pinned by
  `tests/style_lint.rs::no_html5_drag_and_drop_in_ui`.
- **≥44px touch floor** at every interactive control's shared definition (incl.
  `.row-edit`). Pinned by
  `tests/style_lint.rs::registered_interactive_controls_declare_44px_touch_floor`
  (and the *UI fidelity* rule in `CLAUDE.md`).
- **The three `swipe_close=true` management modals** (Account/Server/Wardrobe)
  must route both `close=` and `ModalHead`'s `on_close=` through `act::modal_back`
  (origin-aware one-step-back, Bug 3) and never enter a channel on dismiss. Pinned
  by `tests/style_lint.rs::management_modal_dismiss_returns_to_origin`.

---

## Source map

Key files (`path` — role):

- `src/ui/mod.rs` — UI module surface + cfg-gating policy; defines `AuthCtx`.
- `src/ui/shell/mod.rs` — `Home` (auth gate), `AppShell` (constructs + provides
  the 10 state sub-structs, the mount boot sequence, the top-level confirm /
  wardrobe / account / server modals + toast host), `Pane`/`PendingDelete`.
- `src/ui/shell/state.rs` — the 10 `RwSignal` sub-structs + `StagedAttachment`/
  `UploadStatus`/`EditingMessage`/`Toast` and the per-field invariants.
- `src/ui/shell/act/mod.rs` — the dispatch-layer surface (~16 submodules,
  hydrate-real + ssr-stub, re-exports).
- `src/ui/shell/act/message.rs` — compose/edit/delete, 3-cursor pagination, the
  poll fallback + reconcile primitives, mute/last-seen, friends/lore/members.
- `src/ui/shell/act/sync.rs` — the `EventSource` driver, `dispatch`, the
  self-healing poll fallback + wake/probe machine, `shutdown`.
- `src/ui/shell/channel/mod.rs` — `ChannelPane` (list + composer), `message_actions`,
  the auto-scroll/whisper-veil/composer effects; siblings `avatar`,
  `attachments`, `emoji_suggest`, `lightbox`, `meta`, `radial`, `manager`,
  `skeleton`.
- `src/ui/shell/sk_orbit/mod.rs` — `SkOrbitShell` (the sole shell); pure-math
  submodules `strip`, `charge`, `orbit_map`, `pane_swipe`, `warp`, `drag`,
  `blossom`.
- `src/ui/shell/holopanel.rs` — the `HoloPanel` slide-over primitive + its pure
  drag math.
- `src/ui/modal.rs` — `Modal`/`PersonaInfo`/`ModalHead`, focus trap + restore,
  the opt-in `SwipeClose` engine + its pure decision fns.
- `src/client/api.rs` — the gloo-net same-origin REST client (`ApiError`,
  `humanize`, the transport layer, `upload_media_with_progress`).
- `src/client/mod.rs` — gates `api` to `#[cfg(feature = "hydrate")]`.

Tests that pin this doc's claims:

- `tests/skeleton_switch.rs` — `skeleton_ids_are_exactly_the_three_shells`,
  `valid_skeleton_accepts_known_rejects_unknown`, `fallback_is_a_valid_id`,
  `ssr_stubs_signal_no_pref_and_no_storage`, `set_skeleton_surface_is_pref_only`
  (the skeleton-pref persistence surface; §8).
- `tests/sync_events.rs` — `sync_event_serializes_with_snake_case_type_tags`,
  `targeted_sync_events_pin_their_wire_shape`,
  `reload_sync_event_is_a_bare_global_tag` (the id-only SSE wire shape; §6).
- `tests/events.rs` — server-side SSE delivery + per-frame session re-check (§6,
  server graph; see [04 — realtime SSE](./04-realtime-sse.md)).
- `src/ui/modal.rs` co-located — `swipe_lock_needs_dominant_horizontal_past_slop`,
  `swipe_commits_only_rightward_past_threshold` (swipe-close geometry; §9).
- `src/ui/shell/channel/mod.rs` co-located —
  `message_actions_offers_nothing_mutable_outside_kind_user`,
  `message_actions_count_drives_the_radial_arms` (§7).
- `src/ui/shell/channel/lightbox.rs` co-located — the 13 transform-math tests
  (`clamped_translate_*`, `pinch_keeps_*`, `zoom_about_keeps_*`, `double_tap_*`; §7).
- `src/ui/shell/sk_orbit/strip.rs` co-located —
  `strip_offset_resting_base_is_one_slot_regardless_of_list_index`,
  `strip_offset_rubber_bands_only_at_true_edges`,
  `commit_swipe_by_displacement_or_velocity`,
  `strip_collapses_only_for_a_genuinely_single_channel_guild` (§8).
- `src/ui/shell/sk_orbit/orbit_map.rs` co-located —
  `channel_orbit_is_locked_per_channel`, `inner_channels_orbit_faster_than_outer`,
  `retrograde_is_stable_per_channel`, `orbit_radius_clamps_on_the_poco_c3_floor` (§8).
- `src/ui/shell/sk_orbit/charge.rs` co-located —
  `one_liner_shows_a_sliver_paragraph_mid_saga_pegs`,
  `fraction_is_monotonic_in_word_count`,
  `dash_offset_maps_empty_to_full_circ_and_full_to_zero` (§8).
- `src/ui/shell/sk_orbit/pane_swipe.rs` co-located —
  `pane_back_fraction_matches_the_wardrobe_slide_over`,
  `pane_back_commits_only_rightward_past_threshold` (§8).
- `src/ui/shell/act/message.rs` co-located —
  `plan_window_reconcile_removes_only_in_window_rows_the_server_dropped`,
  `plan_window_reconcile_patches_an_edited_in_window_row`,
  `plan_window_reconcile_never_resurrects_an_optimistically_hidden_row` (§6).
- `tests/style_lint.rs` — `swipe_engines_bail_pointer_capture_on_controls`,
  `no_html5_drag_and_drop_in_ui`,
  `registered_interactive_controls_declare_44px_touch_floor`,
  `management_modal_dismiss_returns_to_origin` (§8, §9; static scans, see
  [08 — styling & chrome](./08-styling-chrome.md) and [09 — testing](./09-testing.md)).

**Code-pinned only (no `tests/*.rs`)**, verified by clippy across all three
graphs + the owner deck-pass:
- graph discipline / ssr-stub doubling (`src/ui/mod.rs:1-3`, `src/client/mod.rs:8`,
  every `act/` twin pair);
- disposal safety / review-M-10 `try_*` discipline (`src/ui/shell/act/`);
- the SSE self-healing reconnect + frozen-PWA machine (`src/ui/shell/act/sync.rs`);
- `Modal` focus restoration (`src/ui/modal.rs:343-384`);
- the channel auto-scroll re-pin + whisper-veil disclosure
  (`src/ui/shell/channel/mod.rs`).
