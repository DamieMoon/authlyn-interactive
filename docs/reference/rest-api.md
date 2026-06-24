# REST API reference — route ↔ DTO ↔ SyncEvent index

The authoritative list of every HTTP route the axum server mounts, the request/response DTO each
carries, its notable status codes, and which realtime `SyncEvent` (if any) it emits. The **route
table itself is `src/server/mod.rs`** — `small_body_routes()` + `media_routes()` + the `/sw.js`
handler, assembled by `api_routes()`. This file is a derived index; when it disagrees with
`server/mod.rs`, the source wins.

DTOs are defined once in `src/protocol.rs` (always-on, serde-only, wasm-safe) and shared verbatim by
the ssr server and the hydrate client (`src/client/api.rs`). See
[../architecture/06-markup-engine.md](../architecture/06-markup-engine.md) for the `body`-embedded
markup grammar, [../architecture/04-realtime-sse.md](../architecture/04-realtime-sse.md) for the SSE
bus, [../architecture/05-auth-privacy.md](../architecture/05-auth-privacy.md) for the session /
privacy-404 model, and [conventions.md](conventions.md) for the naming + DTO-shape rules summarized
at the bottom.

## How to read this index

- **Identity is never in the body.** The caller is the `account` resolved from the
  `authlyn_session` cookie by the `AuthAccount` extractor (`src/server/auth/session.rs`). Absence /
  expiry / garbage cookie → **401** `{"error":"authentication required"}` before the handler runs.
  Public exceptions: `POST /auth/register`, `POST /auth/login`, `GET /push/vapid-key`,
  `GET /sw.js`. Every other route requires a session.
- **Authorization is re-derived per request**, and **non-membership is a 404, not a 403**
  (privacy-404). A 403 means "you are a member/known party but lack the *role*"; a 404 means "you
  may not even know this exists." Both carry a deliberately identical body to a genuinely-missing
  resource. (`tests/guilds.rs::nonmember_get_guild_is_404`,
  `tests/messages.rs::nonmember_post_probes_collapse_to_the_identical_privacy_404`.)
- **Error body is always** `ErrorBody` = `{"error":"<reason>"}` for every 4xx/5xx
  (`src/server/errors.rs::error_response`). Malformed JSON request bodies → **400** via
  `json_rejection_response`.
- **Write-conflict retry → 409, never 500.** Racy `CREATE` against a UNIQUE index is wrapped in
  `with_write_conflict_retry` (`src/server/retry.rs`); a genuine duplicate surfaces as a clean
  **409** (`is_unique_violation`). (`tests/auth.rs::concurrent_register_same_username_never_500s`,
  `tests/guilds.rs::concurrent_invite_yields_one_member_row`.)
- **SyncEvent emission** is `AppState::emit(ev)` (global / visibility-filtered lane) or
  `AppState::emit_for(accounts, ev)` (account-targeted lane) — `src/server/state.rs`. The "Emits"
  column names the variant and the lane. Frames are **id-only**; clients react by refetching the
  permission-checked endpoints. (`tests/sync_events.rs`.)
- **Two body-size groups.** JSON routes cap at **512 KiB** (`REQUEST_BODY_LIMIT_BYTES`); `/media`
  upload/download caps at **64 MiB** (`MEDIA_BODY_LIMIT_BYTES`). The split is required by
  `RequestBodyLimitLayer`'s min-limit composition (`server/mod.rs` module header).
- **Cache-Control.** Every JSON-group response is `no-store` (a `map_response` layer); error
  responses too. (`tests/cache_control.rs`.) `/media` originals are `private, immutable`; `/sw.js`
  is `no-cache`.

---

## Auth / account

`src/server/auth/` (`mod.rs` re-exports; `registration.rs`, `session.rs`, `password.rs`, `admin.rs`).
Pinned by `tests/auth.rs`.

| Method · path | Handler | Request DTO | Response DTO | Status codes | Emits |
|---|---|---|---|---|---|
| `POST /auth/register` | `auth::register` | `RegisterRequest` | `AuthResponse` + `Set-Cookie` | **201**; 400 bad creds; **409** username taken (UNIQUE-race-safe) | — |
| `POST /auth/login` | `auth::login` | `LoginRequest` | `AuthResponse` + `Set-Cookie` | **200**; **401** `invalid username or password` (same body for unknown-user and wrong-password — no enumeration) | — |
| `POST /auth/logout` | `auth::logout` | — | — | **204** (clears cookie + best-effort session delete) | — |
| `POST /auth/change-password` | `auth::change_password` | `ChangePasswordRequest` | — | **204**; 400 weak new pw; **401** wrong current pw / no session | — |
| `GET /auth/me` | `auth::me` | — | `MeResponse` | **200**; 401 | — |
| `PATCH /account` | `auth::patch_account` | `PatchAccountRequest` | — | **204** (empty body = no-op 204); 400 bad display_name; **404** unknown avatar media | `ListsChanged` (global) **only when something changed** — account identity is live-resolved on every message, so a rename/re-avatar alters this account's old messages everywhere |
| `POST /auth/admin/reset-password` | `auth::admin_reset_password` | `AdminResetPasswordRequest` | — | **204**; **403** non-admin; 400 weak pw; **404** no such user; invalidates target's sessions | — |

Notes: `register`/`login` mint a random opaque token, store only its SHA-256 in `session`, and set
`authlyn_session` as `HttpOnly; Secure; SameSite=Lax` (`session.rs::session_cookie`). The
self-service security-question reset was removed; **admin reset is the sole recovery path**
(`server/mod.rs` comment). `is_admin` is env-driven (`AUTHLYN_ADMIN_USERNAMES`); the admin-*allowed*
path is not HTTP-testable because env races parallel test workers — admin-*denied* (403) is pinned
(`tests/feedback.rs::list_feedback_is_403_for_non_admin`).

---

## Guilds / channels / members / emoji

`src/server/guilds/` (`mod.rs`, `channels.rs`, `membership.rs`, `deletion.rs`, `icon.rs`),
`src/server/emoji.rs`, `src/server/personas/wear.rs` (the per-guild wear route). Pinned by
`tests/guilds.rs`, `tests/emoji.rs`, `tests/soft_delete.rs`.

| Method · path | Handler | Request DTO | Response DTO | Status codes | Emits |
|---|---|---|---|---|---|
| `GET /guilds` | `guilds::list_guilds` | — | `ListGuildsResponse` | 200 | — |
| `POST /guilds` | `guilds::create_guild` | `CreateGuildRequest` | `GuildSummary` | **201**; 400 bad name | `ListsChanged` **targeted to creator** (`emit_for`) — at creation the caller is the only member |
| `GET /guilds/trash` | `guilds::list_deleted_guilds` | — | `ListGuildsResponse` | 200 (owner's soft-deleted guilds) | — |
| `PUT /rail/order` | `guilds::set_rail_order` | `RailOrderRequest` | — | **204** (non-member ids silently dropped) | `ListsChanged` **targeted to caller** (`emit_for`) — per-user preference |
| `GET /guilds/{id}` | `guilds::get_guild` | — | `GuildDetail` | 200; **404** non-member / missing (privacy) | — |
| `PATCH /guilds/{id}` | `guilds::patch_guild` | `PatchGuildRequest` | — | **204**; 400 bad name/accent; **404** non-member; **403** member-not-owner (`require_manager`) | `ListsChanged` (global) per changed field |
| `DELETE /guilds/{id}` | `guilds::delete_guild` | — | — | **204** (soft-delete, 30d window); 404/403 (`require_owner`) | `ListsChanged` (global) |
| `POST /guilds/{id}/restore` | `guilds::restore_guild` | — | — | **204**; 404/403 (owner; `guild_member` survives soft-delete) | `ListsChanged` (global) |
| `PUT /guilds/{id}/icon` | `guilds::set_guild_icon` | `SetGuildIconRequest` | — | **204**; **404** non-member / unknown media; **403** member-not-manager; re-derives `accent_color` from the image (best-effort) | `ListsChanged` (global) |
| `GET /guilds/{id}/trash/channels` | `guilds::list_deleted_channels` | — | `ChannelListResponse` | 200; 404/403 (manager) | — |
| `POST /guilds/{id}/channels` | `guilds::create_channel` | `CreateChannelRequest` | `ChannelSummary` | **201**; 400 bad name / bad `kind`; 404/403 (manager) | `ListsChanged` (global) |
| `PATCH /guilds/{id}/channels/{cid}` | `guilds::patch_channel` | `PatchChannelRequest` | — | **204**; 400 bad name; **404** channel-not-in-guild; 403 (manager) | `ListsChanged` (global) when something changed |
| `DELETE /guilds/{id}/channels/{cid}` | `guilds::delete_channel` | — | — | **204** (soft-delete, 1d window); 404/403 (manager) | `ListsChanged` (global) |
| `POST /guilds/{id}/channels/{cid}/restore` | `guilds::restore_channel` | — | — | **204** (scoped to the guild so a manager can't revive an unrelated id); 403 | `ListsChanged` (global) |
| `GET /guilds/{id}/members` | `guilds::list_members` | — | `ListMembersResponse` | 200; **404** non-member (privacy) | — |
| `POST /guilds/{id}/members` | `guilds::invite_member` | `InviteMemberRequest` | — | **201**; 400 empty username; 404/403 (manager); **404** unknown user; **409** already a member (UNIQUE-race-safe) | `ListsChanged` (global) |
| `DELETE /guilds/{id}/members/{aid}` | `guilds::remove_member` | — | — | **204**; **404** non-member; 400 owner self-leave; **403** kick-without-admin / kicking the owner; 404 member not found | `ListsChanged` (global) |
| `PUT /guilds/{id}/members/{aid}/role` | `guilds::set_member_role` | `SetMemberRoleRequest` | — | **204**; 400 bad role; 404/403 (manager); **403** changing the owner; 404 member not found | `ListsChanged` (global) |
| `GET /guilds/{id}/emoji` | `emoji::list_emoji` | — | `ListEmojiResponse` | 200; **404** non-member (any-role gate) | — |
| `POST /guilds/{id}/emoji` | `emoji::create_emoji` | `CreateEmojiRequest` | `CustomEmoji` | **201**; 404 non-member; 400 bad name (`^[a-z0-9_]{2,32}$`); **409** name taken in guild | — |
| `DELETE /guilds/{id}/emoji/{name}` | `emoji::delete_emoji` | — | — | **204**; 404/403 (manager) | — |
| `PUT /guilds/{id}/active-persona` | `personas::set_active_persona` | `SetActivePersonaRequest` | — | **204** (legacy per-guild wear; `null` removes); **404** non-member of guild / persona not editable | — |

Role model is `owner` \| `admin` \| `member`. `require_owner` gates guild delete/restore;
`require_manager` (owner-or-admin) gates channel CRUD, role changes, icon, emoji delete;
`caller_role`'s `Some(_)` is the any-member gate (read roster, create/list emoji). Static routes
(`/guilds/trash`, `/guilds/{id}/trash/channels`, `/channels/read-state`,
`/channels/{cid}/messages/trash`) outrank their dynamic siblings in axum's router regardless of
declaration order (`server/mod.rs` comments).

---

## Messages / typing / roll / read-state

`src/server/messages/` (`mod.rs` holds `channel_access`; `posting.rs`, `reading.rs`, `editing.rs`,
`rolling.rs`, `typing.rs`, `read_state.rs`, `unread.rs`). Pinned by `tests/messages.rs`,
`tests/roll.rs`, `tests/mentions.rs`, `tests/typing_drafts.rs`, `tests/read_state.rs`,
`tests/unread.rs`. **Channel-scoped** — a DM thread and a cameo channel ride these same routes (a DM
*is* a `kind='dm'` channel; a cameo guest is resolved by `channel_access`).

| Method · path | Handler | Request DTO | Response DTO | Status codes | Emits |
|---|---|---|---|---|---|
| `GET /channels/{cid}/messages` | `messages::list_messages` | query `?since=&after_id=&before=&before_id=` | `ListMessagesResponse` (≤100 `MessageEnvelope`, ASC by `(sent_at,id)`; `typing` + `active_persona` piggyback) | 200; **400** malformed cursor; **404** non-member / missing (privacy) | — |
| `POST /channels/{cid}/messages` | `messages::post_message` | `SendMessageRequest` | `SendMessageResponse` (`{id}`) | **201**; **404** non-member (gate **before** any DB-probing validation, so probes can't leak existence); 400 empty body+no attachments / over-cap / too many attachments / unknown effect / unknown attachment / invalid reply target; 400 non-text/non-dm channel; **403** locked DM | `MessageCreated{channel_id}` (global). Also fires best-effort Web Push + clears the author's Ghost Quill draft |
| `GET /channels/{cid}/messages/trash` | `messages::list_deleted_messages` | — | `ListMessagesResponse` | 200 (any member); 404 (privacy) | — |
| `PATCH /channels/{cid}/messages/{mid}` | `messages::edit_message` | `EditMessageRequest` | — | **204**; 400 empty/over-cap; **404** non-member; **403** not your message (a stranger's *and* a missing message both → 403 so ids can't be probed); **403** `roll` is immutable | `MessageEdited{channel_id,message_id}` (global); clears draft |
| `DELETE /channels/{cid}/messages/{mid}` | `messages::delete_message` | — | — | **204** (soft-delete, 1h window); 404/403 as above; **403** `roll` is immutable | `MessageDeleted{channel_id,message_id}` (global) |
| `POST /channels/{cid}/messages/{mid}/restore` | `messages::restore_message` | — | — | **204** (idempotent; no-op on an already-live row) | `MessageCreated{channel_id}` (global) **only when the row actually transitioned** |
| `POST /channels/{cid}/roll` | `messages::roll_message` | `RollRequest` | `SendMessageResponse` (`{id}`) | **201** (server-rolled, immutable `kind='roll'`); **400** bad expr; **404** non-member; 400 non-text channel | `MessageCreated{channel_id}` (global) + Web Push + clears draft |
| `POST /channels/{cid}/typing` | `messages::typing_ping` | `TypingPingRequest` (optional; bare POST = classic ping) | — | **204**; **404** non-member; 400 malformed JSON (when a body is sent) | `Typing{channel_id}` (global) |
| `GET /channels/{cid}/typing-drafts` | `messages::typing_drafts` | — | bare JSON array of `TypingDraftEntry` | 200 (excludes caller's own draft); **404** non-member | — |
| `POST /channels/{cid}/mark-read` | `messages::mark_read` | `MarkReadRequest` | — | **204** (UPSERT keeps the MAX cursor; older POST never regresses newer); **400** invalid cursor; **404** non-member | `ReadStateChanged{channel_id}` **targeted to caller** (`emit_for`) — other devices refresh |
| `GET /channels/read-state` | `messages::read_state` | — | `ReadStateResponse` | 200 (every channel the caller has a cursor for) | — |
| `GET /unread` | `messages::unread` | — | `UnreadResponse` | 200 (batched per visible text channel; counts capped at 100; DM/cameo rows carry `guild_id=None`) | — |

Server-trusted send invariants: `author` comes from the session; the **speaking-as persona is
re-validated** (`can_edit_persona`) at send time even for the stored per-channel wear
(`posting.rs::resolve_send_persona`), so a revoked editor or deleted persona stops stamping. `@`-
mentions resolve **post-auth** against channel membership (guild members + DM members + active
guests), so a message can only ping people who can see it (`posting.rs::resolve_mentions`;
`tests/mentions.rs`). Whisper-armed drafts and whispered reply-quote/push bodies are masked to the
fixed `(whisper)` placeholder before they leave the server (`tests/messages.rs::reply_preview_masks
_whispered_parent_snippet`, `tests/typing_drafts.rs::whisper_armed_draft_is_masked_to_the_fixed_placeholder_for_other_members`,
`tests/push.rs::push_payload_row_read_carries_the_effect_column_from_a_real_whisper_row`). Roll
immutability: `tests/roll.rs::editing_own_roll_is_403_and_the_body_is_unchanged`,
`deleting_own_roll_is_403_and_the_roll_survives`.

---

## Personas / gallery / editors

`src/server/personas/` (`mod.rs`, `core.rs`, `gallery.rs`, `editors.rs`, `wear.rs`). All
owner-scoped — another account's persona reads/writes as a **privacy-404**. Pinned by
`tests/personas.rs`.

| Method · path | Handler | Request DTO | Response DTO | Status codes | Emits |
|---|---|---|---|---|---|
| `GET /personas` | `personas::list_personas` | — | `ListPersonasResponse` (owned **and** redeemed-editor personas; `owned` flags which) | 200 | — |
| `POST /personas` | `personas::create_persona` | `CreatePersonaRequest` | `PersonaSummary` | **201**; 400 bad name / over-long description / invalid color | — |
| `POST /personas/redeem` | `personas::redeem_persona_key` | `RedeemPersonaKeyRequest` | — | **201**; 400 empty key; **404** no such key; **409** you own it / already an editor | — |
| `GET /personas/{id}` | `personas::get_persona` | — | `PersonaDetail` (owner sees `share_key` + `editors`; editor sees neither) | 200; **404** not owner-or-editor | — |
| `PATCH /personas/{id}` | `personas::patch_persona` | `PatchPersonaRequest` | — | **204** (owner or editor); 400 bad name/color/position; **404** not editable | — |
| `DELETE /personas/{id}` | `personas::delete_persona` | — | — | **204** (owner only; cascades gallery/editors/wear refs); **404** not owner | — |
| `DELETE /personas/{id}/leave` | `personas::leave_persona` | — | — | **204** (editor drops a shared persona; clears their wear); **404** not an editor | — |
| `GET /personas/{id}/editors` | `personas::list_editors` | — | `ListPersonaEditorsResponse` | 200; **404** not owner | — |
| `PUT /personas/{id}/editors/{aid}` | `personas::add_editor` | — | — | **204** (idempotent — re-share is 204); **404** not owner; 400 not an accepted friend | — |
| `DELETE /personas/{id}/editors/{aid}` | `personas::remove_editor` | — | — | **204** (clears the removed editor's wear); **404** not owner | — |
| `PUT /personas/{id}/avatar` | `personas::set_avatar` | `SetAvatarRequest` | — | **204**; **404** not editable / unknown media | — |
| `POST /personas/{id}/gallery` | `personas::add_gallery_image` | `AddGalleryImageRequest` | `AddGalleryImageResponse` (`{id}`) | **201**; **404** not editable / unknown media | — |
| `POST /personas/{id}/gallery/batch` | `personas::add_gallery_images_batch` | `AddGalleryImagesBatchRequest` | `AddGalleryImagesBatchResponse` (`{ids}` in input order) | **201** (atomic, contiguous positions); 400 empty / over-cap (100) / duplicate id; **404** not editable / media not found | — |
| `DELETE /personas/{id}/gallery/{img}` | `personas::remove_gallery_image` | — | — | **204**; **404** not editable | — |
| `PUT /channels/{cid}/active-persona` | `personas::set_channel_active_persona` | `SetActivePersonaRequest` | — | **204** (per-channel wear; `null` removes; idempotent re-wear via UNIQUE-race-safe DELETE-then-CREATE); **404** non-member / persona not editable | — |

Editor access (`can_edit_persona`) = owner **or** a `persona_editor` row redeemed via the share key
or granted to a friend; editors can wear + edit, never delete/re-share. Per-channel wear (`#persona`)
is the current path; the per-guild `PUT /guilds/{id}/active-persona` is the legacy path. Pins:
`persona_crud_is_owner_scoped`, `redeem_grants_edit_and_wear_then_revoke`,
`batch_gallery_atomic_under_partial_failure`, `concurrent_channel_wear_converges_to_one_row`,
`revoked_editor_per_channel_wear_stops_stamping_bare_messages`.

---

## Lorebook

`src/server/lorebook.rs`. Entries live on a `kind='lorebook'` channel and are **collaborative** — any
guild member reads and writes them (no per-user owner). Pinned by `tests/lorebook.rs`.

| Method · path | Handler | Request DTO | Response DTO | Status codes | Emits |
|---|---|---|---|---|---|
| `GET /channels/{cid}/lorebook` | `lorebook::list_entries` | — | `ListLorebookResponse` (ordered by `position`) | 200; **404** non-member; **400** not a lorebook channel | — |
| `POST /channels/{cid}/lorebook` | `lorebook::create_entry` | `CreateLorebookEntryRequest` | `CreateLorebookEntryResponse` (`{id}`) | **201**; 404/400 as above; 400 field bounds (title ≤200, content 1–8000, ≤64 keys ≤100 chars) | — |
| `PATCH /channels/{cid}/lorebook/{eid}` | `lorebook::patch_entry` | `PatchLorebookEntryRequest` | — | **204** (empty body = 204 no-op); **404** non-member / entry-not-in-channel; 400 bounds | — |
| `DELETE /channels/{cid}/lorebook/{eid}` | `lorebook::delete_entry` | — | — | **204** (scoped to the channel); 404 (privacy) | — |

Access uses the shared `access::resolve_membership` with the **soft-delete filter off** (historical
behavior preserved). Pin: `lorebook_ops_on_a_text_channel_are_400`,
`nonmember_cannot_touch_lorebook`.

---

## Friends

`src/server/friends.rs`. Global, guild-independent; one directed `friendship` row per request
(`requester → addressee`), `pending → accepted`. Pinned by `tests/friends.rs`.

| Method · path | Handler | Request DTO | Response DTO | Status codes | Emits |
|---|---|---|---|---|---|
| `GET /friends` | `friends::list_friends` | — | `ListFriendsResponse` (`friends` + `incoming` + `outgoing`) | 200 | — |
| `POST /friends` | `friends::add_friend` | `FriendRequest` (by username) | — | **201** new request, or **200** when it auto-accepts a reverse-pending request; 400 empty / self; **404** unknown user; **409** already friends / duplicate request | `FriendsChanged` **targeted to both edge accounts** (`emit_for`) |
| `POST /friends/{aid}/accept` | `friends::accept_friend` | — | — | **200**; **404** no pending request from that user | `FriendsChanged` (targeted both) |
| `DELETE /friends/{aid}` | `friends::remove_friend` | — | — | **204** (idempotent; either direction); also **locks** the shared 1:1 DM and **revokes** any cameo between the two (best-effort) | `FriendsChanged` (targeted both) |

Pins: `reverse_request_auto_accepts`, `duplicate_request_is_409`,
`unfriend_removes_the_relationship`, `self_and_unknown_user_are_rejected`. The unfriend side-effects
on DMs/cameos are pinned in `tests/dms.rs::unfriending_locks_the_one_to_one_then_refriending_unlocks_it`
and `tests/cameos.rs::unfriending_revokes_only_the_cameo_from_that_inviter`.

---

## Direct messages (DMs)

`src/server/dms.rs` (M7/P1). Thread lifecycle only — messages / read-state / active-persona ride the
`/channels/{id}/…` routes above (a DM thread **is** a channel, `id` = the channel id). Friend-gated.
Pinned by `tests/dms.rs`.

| Method · path | Handler | Request DTO | Response DTO | Status codes | Emits |
|---|---|---|---|---|---|
| `GET /dms` | `dms::list_dms` | — | `ListDmsResponse` (`DmSummary` with live members + `locked`) | 200 | — |
| `POST /dms` | `dms::create_dm` | `CreateDmRequest` | `DmSummary` | **201** new, or **200** when a 1:1 dedups to an existing thread; 400 no members / too many (16); **403** not all are accepted friends | `ListsChanged` **targeted to participants** (`emit_for`) on a real create |
| `POST /dms/{tid}/members` | `dms::invite_to_dm` | `InviteToDmRequest` | `DmSummary` | **200** (idempotent re-add); **404** caller not a member; 400 empty / self / over-cap; **403** invitee not a friend | `ListsChanged` (targeted to current members) |
| `DELETE /dms/{tid}/members/me` | `dms::leave_dm` | — | — | **204** (last member out → thread soft-deleted + dedup lock released); **404** caller not a member | `ListsChanged` (targeted to pre-leave members incl. the leaver) |

1:1 dedup is race-safe via the `dm_pair` UNIQUE lock; a 1:1 whose two friends unfriend goes
**read-only** (`locked=true`) — history readable, posts server-rejected (403 in `post_message`).
Pins: `one_to_one_dm_is_deduped`, `concurrent_one_to_one_creates_converge_on_one_thread`,
`leaving_drops_you_and_last_member_soft_deletes_the_thread`,
`dm_privacy_404_body_is_byte_identical_to_the_guild_channel_404`.

---

## Guest cameos

`src/server/cameos.rs` (M7/P2). Scoped, ephemeral guest access to **one** guild text channel — a
`channel_guest` row (the third membership model after `guild_member` / `dm_member`); the
message/persona/read-state/unread/push stack is inherited via the resolvers. Pinned by
`tests/cameos.rs`.

| Method · path | Handler | Request DTO | Response DTO | Status codes | Emits |
|---|---|---|---|---|---|
| `GET /channels/{cid}/guests` | `cameos::list_guests` | — | `ListGuestsResponse` (active/unexpired) | 200 (any member); **404** non-member | — |
| `POST /channels/{cid}/guests` | `cameos::invite_guest` | `InviteGuestRequest` | `GuestSummary` | **201** (idempotent re-invite); **404** caller not a guild member of a live text channel; 400 empty / self / invalid `expires_at` / already a guild member; **403** invitee not a friend | `ListsChanged` **targeted to {invitee, inviter}** (`emit_for`) |
| `DELETE /channels/{cid}/guests/me` | `cameos::leave_cameo` | — | — | **204** (idempotent; leaks nothing about a channel you can't see) | `ListsChanged` (targeted to caller) |
| `DELETE /channels/{cid}/guests/{aid}` | `cameos::revoke_guest` | — | — | **204**; **404** caller not a guild member (privacy); **403** member who is neither inviter nor manager | `ListsChanged` (targeted to the revoked guest) |
| `GET /cameos` | `cameos::list_cameos` | — | `ListCameosResponse` (guest-side; standalone, no guild rail) | 200 | — |

Only a real guild member (not a guest) may invite; revoke is allowed for the inviter **or** a guild
manager. `expires_at` is enforced as a lazy-check at every membership query. Pins:
`inviting_a_friend_grants_scoped_access_and_badges_the_guest_message`,
`a_guest_is_confined_to_the_one_channel`,
`revoking_a_guest_kills_access_but_keeps_the_badged_history`,
`an_expired_cameo_denies_access_while_a_future_one_grants_it`.

---

## Web Push

`src/server/push.rs` (#30). Pinned by `tests/push.rs`. The whole feature is a no-op when VAPID env is
unset (`PushSender::from_env() → None`).

| Method · path | Handler | Request DTO | Response DTO | Status codes | Emits |
|---|---|---|---|---|---|
| `GET /push/vapid-key` | `push::vapid_key` | — | `VapidKeyResponse` | 200; **404** when push isn't configured (client then skips subscribing) — **public, no session** | — |
| `POST /push/subscribe` | `push::subscribe` | `PushSubscribeRequest` (+ nested `PushSubscriptionKeys`) | — | **204** (UPSERT on `endpoint`, race-safe); 400 incomplete subscription | — |
| `POST /push/unsubscribe` | `push::unsubscribe` | `PushUnsubscribeRequest` | — | **204** (scoped to the caller's own rows) | — |

Sends are fire-and-forget from `post_message` / `roll_message` / system broadcast via
`notify_new_message`; recipients = channel members (guild members + active guests, or DM members)
minus the author; whispered bodies are masked. Pins: `vapid_key_is_404_when_push_unconfigured`,
`subscribe_then_unsubscribe`, `unsubscribe_is_account_scoped`,
`concurrent_subscribe_same_endpoint_converges_to_one_row`.

---

## Feedback

`src/server/feedback.rs` (#31). Pinned by `tests/feedback.rs`.

| Method · path | Handler | Request DTO | Response DTO | Status codes | Emits |
|---|---|---|---|---|---|
| `GET /feedback` | `feedback::list_feedback` | — | `ListFeedbackResponse` (newest-first, `status != 'deleted'`) | 200; **403** non-admin (fail-closed) | — |
| `POST /feedback` | `feedback::submit_feedback` | `SubmitFeedbackRequest` | — | **201** (any authed; `kind` coerced to bug\|idea\|other); 400 empty / >4000 chars | — |
| `DELETE /feedback/{id}` | `feedback::delete_feedback` | — | — | **204** (soft-delete → `status='deleted'`); **403** non-admin | — |

Pins: `submit_feedback_coerces_unknown_kind`, `submit_feedback_body_bounds`,
`list_feedback_is_403_for_non_admin`, `archived_feedback_is_filtered_from_list_query`.

---

## Admin

`src/server/system_messages.rs`, `src/server/dev_reload.rs`. Both admin-gated (`is_admin`,
fail-closed → 403). Pinned by `tests/system_messages.rs`, `tests/sync_events.rs`.

| Method · path | Handler | Request DTO | Response DTO | Status codes | Emits |
|---|---|---|---|---|---|
| `POST /admin/system-message` | `system_messages::send_system_message` | `SendSystemMessageRequest` | `SystemBroadcastResult` | **200** (`guilds_targeted == messages_sent + guilds_skipped`); **403** non-admin; 400 empty / >4000 chars | `MessageCreated{channel_id}` (global) **per fanned-out message** |
| `POST /admin/dev/reload` | `dev_reload::dev_reload` | — | — | **204**; **403** non-admin; no body read | `Reload` (global, bypasses the visibility filter; delivered as a named `event: reload` SSE frame) |

The broadcast authors a `kind='system'` "Nova DOT" message into every live guild's first live text
channel (skipping guilds with none) under the reserved `nova_dot` account; it also fires Web Push.
The admin-*allowed* HTTP path is not test-driven (env races workers) — the auth-free core
(`broadcast_system_message`, `broadcast_reload`) is exercised directly; the 403 path is pinned
(`system_broadcast_is_403_for_non_admin_and_writes_nothing`). Pins:
`broadcast_posts_a_nova_dot_system_message_into_each_guilds_first_text_channel`,
`broadcast_emits_message_created_per_fanned_out_message_over_sse`,
`sync_events.rs::reload_sync_event_is_a_bare_global_tag`.

---

## Realtime SSE

`src/server/events.rs`. The id-only bus consumed by the hydrate `EventSource`. Detailed in
[../architecture/04-realtime-sse.md](../architecture/04-realtime-sse.md).

| Method · path | Handler | Request DTO | Response DTO | Status codes | Emits |
|---|---|---|---|---|---|
| `GET /events` | `events::events` | — | `text/event-stream`; each `data:` frame = one serialized `SyncEvent` (the `Reload` nudge is a named `event: reload` frame) | 200 stream; 401 (and the session is **re-checked every frame and ≥ every 30s** for the stream's lifetime) | — (consumer, not emitter) |

Filtering is per-connection (`visible_channels`); `SyncEvent::channel_id()` decides
visibility-scoping. `Cache-Control: no-store` is correct for SSE. Pinned by `tests/sync_events.rs`
(wire shape) — `GET /events` itself drives no integration suite (long-lived stream); the per-route
emit assertions live in `tests/dms.rs::dm_lifecycle_emits_lists_changed_to_members_over_sse` and
`tests/cameos.rs::cameo_lifecycle_emits_lists_changed_to_the_guest_over_sse`.

---

## Media

`src/server/media.rs`. The **64 MiB** body group. Auth = session; no per-blob ACL in phase 1 (any
authed account may fetch any id). Pinned by `tests/media.rs`.

| Method · path | Handler | Request DTO | Response DTO | Status codes | Emits |
|---|---|---|---|---|---|
| `POST /media` | `media::upload_media` | `multipart/form-data`, field `file` | `MediaUploadResponse` (`{id}`) | **201**; 400 empty / malformed multipart; **415** unsupported media type (script-capable types — `text/html`, `image/svg+xml`, JS — are **rejected**) | — |
| `GET /media/{id}` | `media::download_media` | query `?w=N` (optional thumbnail width, clamped 16–512) | raw bytes (`Content-Type` = stored MIME for inline-safe rasters; everything else forced to a `nosniff` `octet-stream` attachment); thumbnails are JPEG | 200; **304** thumbnail still-current (pipeline-version ETag); **404** unknown id / missing file; **500** path escapes `media_dir` | — |

Inline-renderable allowlist: `image/png|jpeg|gif|webp` (SVG excluded — XML/script). Originals are
`private, max-age=31536000, immutable`; thumbnails are `private, max-age=86400` + a
`THUMB_PIPELINE_VERSION` ETag so a pipeline bump revalidates within a day
(`src/server/media.rs` `THUMB_PIPELINE_VERSION` is pinned by
`thumbnails_revalidate_via_pipeline_version_etag_instead_of_immutable`). Pins:
`upload_rejects_script_capable_mimes`, `url_path_traversal_attempt_does_not_escape`,
`migrated_blob_with_stale_storage_path_falls_back_to_media_dir`,
`original_media_responses_are_privately_immutably_cacheable`.

---

## Service worker

`src/server/mod.rs::serve_service_worker`.

| Method · path | Handler | Request DTO | Response DTO | Status codes | Emits |
|---|---|---|---|---|---|
| `GET /sw.js` | `serve_service_worker` | — | `text/javascript` (embedded `public/sw.js` with `__BUILD_REV__` → compile-time git rev) | 200; `Cache-Control: no-cache`; `Service-Worker-Allowed: /` — **public, no session** | — |

A unique per-build `CACHE_VERSION` makes the browser see a new SW each release, driving the
"new version available" refresh prompt. See
[../architecture/11-build-deploy-pwa.md](../architecture/11-build-deploy-pwa.md).

---

## DTO catalogue (`src/protocol.rs`)

One always-on module (`pub mod protocol;` in `src/lib.rs`, no cfg gate). Imports only `serde` — it
must compile to `wasm32-unknown-unknown` (no axum/surrealdb/tokio). ~84 structs + the `SyncEvent`
enum + `ErrorBody`. The server (ssr) produces them; `src/client/api.rs` (hydrate) consumes them.

### Suffix conventions

| Suffix | Role | Examples |
|---|---|---|
| `Request` | inbound body | `RegisterRequest`, `SendMessageRequest`, `CreateDmRequest` |
| `Response` | top-level outbound envelope wrapping a list/result | `AuthResponse`, `ListGuildsResponse`, `UnreadResponse` |
| `Summary` | one item in a list view | `GuildSummary`, `MemberSummary`, `PersonaSummary`, `DmSummary`, `GuestSummary`, `CameoSummary` |
| `Detail` | one item in a single-resource view (richer than Summary) | `GuildDetail`, `PersonaDetail` |
| `Envelope` | one fully-resolved record on the wire | `MessageEnvelope` |
| `Item` | one row in an admin/list response | `FeedbackItem` |
| `Entry` | one row in a collection fetch | `LorebookEntry`, `TypingDraftEntry` |
| (bare nouns) | nested / embedded shapes | `Attachment`, `ReplyPreview`, `GalleryImage`, `PersonaEditor`, `ChannelSummary`, `ChannelReadCursor`, `ChannelUnread`, `PushSubscriptionKeys`, `CustomEmoji`, `FriendSummary`, `DmMemberSummary` |

### PATCH-shaped DTOs

Every PATCH body derives `Default` and has **all-`Option<>`** fields; an absent field leaves that
column untouched, and an empty body is a 204 no-op. The set:
`PatchAccountRequest`, `PatchGuildRequest`, `PatchChannelRequest`, `PatchPersonaRequest`,
`PatchLorebookEntryRequest`. (`RailOrderRequest`, `CreateDmRequest`, `InviteGuestRequest`,
`TypingPingRequest` also derive `Default` for ergonomic construction, but are not PATCH bodies.)

### `#[serde(default)]` — forward/post-ship wire-compat

Fields **added after a DTO shipped** carry `#[serde(default)]` so a version-skewed producer (a rolling
deploy, a hand-rolled or native client) that omits them still deserializes — without it the whole
response fails to decode on the client. This is a load-bearing rule, pinned by
`src/protocol.rs::tests::message_envelope_deserializes_without_persona_description_or_color` (the
F-D12-3 wire-compat test, the only `#[cfg(test)]` block in the module). Examples:
`MeResponse.is_admin` / `avatar_id`; the whole post-ship tail of `MessageEnvelope`
(`author_avatar_id`, `persona_description`, `persona_color`, `persona_avatar_id`, `attachments`,
`reply_to`, `is_pinged`, `kind` (via `default_message_kind` → `"user"`), `effect`, `guest_cameo`);
`GuildSummary.accent_color` / `icon_id`; `ChannelUnread.guild_id` / `latest_*`.

### `SyncEvent` (the SSE vocabulary)

`#[serde(tag = "type", rename_all = "snake_case")]` enum, deliberately **content-free** (notify-and-
fetch). Variants and their visibility scope (`SyncEvent::channel_id()`):

| Variant | Payload | Visibility | Emitted by (lane) |
|---|---|---|---|
| `MessageCreated` | `{channel_id}` | channel-scoped | post/roll/restore/system-broadcast (`emit`, global) |
| `MessageEdited` | `{channel_id, message_id}` | channel-scoped | `edit_message` (`emit`) |
| `MessageDeleted` | `{channel_id, message_id}` | channel-scoped | `delete_message` (`emit`) |
| `Typing` | `{channel_id}` | channel-scoped | `typing_ping` (`emit`) |
| `ListsChanged` | — | global (refetch lists) | guild/channel/member/icon mutations (`emit`); rail-order/account/DM/cameo (`emit_for`, targeted) |
| `ReadStateChanged` | `{channel_id}` | **`None`** (account-targeted; bypasses visibility filter) | `mark_read` (`emit_for`, caller) |
| `FriendsChanged` | — | **`None`** (account-targeted) | friends mutations (`emit_for`, both edge accounts) |
| `Reload` | — | **`None`** (all connections; named `event: reload` frame) | `dev_reload` (`emit`, global, filter-bypassed) |
| `Unknown` | — | `None` | never constructed (`#[serde(other)]` forward-compat catch-all; consumers MUST ignore) |

`ReadStateChanged` carries a `channel_id` field but reports `None` from `channel_id()` — it is
delivered through the targeted lane, which bypasses channel-visibility filtering entirely
(`src/protocol.rs::SyncEvent::channel_id` doc). Wire shape pinned by
`tests/sync_events.rs::sync_event_serializes_with_snake_case_type_tags`,
`targeted_sync_events_pin_their_wire_shape`, `reload_sync_event_is_a_bare_global_tag`.

---

## Source map

Key files:

- `src/server/mod.rs` — **the route table** (`small_body_routes` + `media_routes` + `/sw.js`); the two body-size groups; the `no-store` JSON layer; `serve_service_worker`; the soft-delete purge sweep.
- `src/protocol.rs` — every wire DTO + `ErrorBody` + the `SyncEvent` enum; the always-on serde-only contract.
- `src/client/api.rs` — the hydrate consumer half of the contract (decode / `ApiError` lifting).
- `src/server/errors.rs` — `error_response` / `json_rejection_response` (the canonical `ErrorBody` wrapper, 400 on malformed JSON).
- `src/server/auth/{session,registration,password,admin}.rs` — `AuthAccount` extractor, session lifecycle, the auth/account handlers.
- `src/server/state.rs` — `AppState::emit` (global lane) / `emit_for` (targeted lane); the bus envelope.
- `src/server/retry.rs` — `with_write_conflict_retry` + `is_unique_violation` (the 409-not-500 path).
- `src/server/{guilds,emoji,messages,personas,lorebook,friends,dms,cameos,push,feedback,media,system_messages,dev_reload,events}.rs` (and submodules) — the domain handlers, one per group above.

Tests that pin the route groups (each cited inline above):

- `tests/auth.rs` — auth/account, register/login/me/change-password/admin-reset, the 401/409 invariants.
- `tests/guilds.rs`, `tests/emoji.rs`, `tests/soft_delete.rs` — guilds/channels/members/emoji, role gates, soft-delete + purge.
- `tests/messages.rs`, `tests/roll.rs`, `tests/mentions.rs`, `tests/typing_drafts.rs`, `tests/read_state.rs`, `tests/unread.rs` — the channel-scoped message/typing/roll/read-state/unread surface.
- `tests/personas.rs` — personas/gallery/editors/wear, owner-scoping + redeem.
- `tests/lorebook.rs`, `tests/friends.rs`, `tests/dms.rs`, `tests/cameos.rs` — lorebook, friends, DMs, cameos.
- `tests/push.rs`, `tests/feedback.rs`, `tests/media.rs`, `tests/system_messages.rs` — push, feedback, media, admin broadcast.
- `tests/sync_events.rs` — `SyncEvent` wire shapes; `tests/cache_control.rs` — the `no-store` / media cache split.
- `src/protocol.rs::tests::message_envelope_deserializes_without_persona_description_or_color` — the `#[serde(default)]` post-ship wire-compat rule.

For stack / dependency / toolchain / convention detail, see `README.md`, `Cargo.toml` (its
`#`-comments), `CLAUDE.md`, and [conventions.md](conventions.md) — not
duplicated here.
