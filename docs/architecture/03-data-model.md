# Data model

The entire SurrealDB data model is one file, [`src/storage/schema.surql`](../../src/storage/schema.surql) — 21 `SCHEMAFULL` tables, 114 `DEFINE FIELD`, 25 `DEFINE INDEX` (13 `UNIQUE`), and a handful of inline backfills. It is embedded into the binary at compile time as a `&str` and applied verbatim once on boot. There is no migration framework, no version table, and no Rust-side schema logic: the `.surql` *is* the schema and the migration engine, and SurrealDB validates it at apply time. The discipline that keeps that single file idempotent over a populated production DB — `option<>`-or-coalesced-backfill, `DEFINE FIELD OVERWRITE` for widened types — is the subject of half this document.

Stack, dependency rationale, and the SurrealDB version pin live in [`Cargo.toml`](../../Cargo.toml) `#`-comments and [`CLAUDE.md`](../../CLAUDE.md); request/auth flow that consumes these tables is in [02-request-lifecycle.md](02-request-lifecycle.md) and [05-auth-privacy.md](05-auth-privacy.md); the realtime layer that emits on every mutation is [04-realtime-sse.md](04-realtime-sse.md).

## Boot-apply contract

`src/storage/mod.rs` is one line:

```rust
pub const SCHEMA: &str = include_str!("schema.surql");
```

`db::apply_schema` ([`src/db.rs:53-56`](../../src/db.rs)) is the sole consumer:

```rust
pub async fn apply_schema(db: &Surreal<Client>) -> surrealdb::Result<()> {
    db.query(storage::SCHEMA).await?.check()?;
    Ok(())
}
```

The whole file is sent as **one multi-statement query**, and `.check()` turns any rejected `DEFINE` or any failed backfill `UPDATE` into an `Err`. `main.rs` `.expect()`s that call, so a schema that does not apply cleanly **panics the server on boot** — over a populated prod DB that is a crash-loop, not a graceful degrade. The gate for "is this schema safe to deploy" is therefore precisely: *does this multi-statement string apply cleanly, and re-apply idempotently, over a prod-shaped populated DB?* That gate is pinned by `tests/schema_apply.rs::applying_full_schema_over_prod_shaped_populated_db_is_crash_free_and_idempotent` (100 legacy messages + 5 channels + 3 guest rows, applied then re-applied).

Two consequences follow from "one query, applied verbatim":

1. **Statement order is load-bearing.** Within a table block the order is `DEFINE TABLE` → `DEFINE FIELD`s → `DEFINE INDEX`es, then idempotent backfill `UPDATE`s **last**. A backfill that materialises a NONE field must run *before* any later `UPDATE` that re-validates the whole row (see [Migration discipline](#migration-discipline)). The `account` block's security-field purge (`UPDATE … UNSET` + `REMOVE FIELD`) is explicitly ordered *after* the `display_name` backfill for exactly this reason — pinned by `tests/schema_apply.rs::applying_schema_over_account_with_security_fields_purges_them_without_crashing`.
2. **Re-apply is a no-op for `IF NOT EXISTS` DEFINEs and `WHERE`-guarded UPDATEs.** Every `DEFINE` uses `IF NOT EXISTS` except the three `DEFINE FIELD OVERWRITE` sites, which re-define unconditionally (idempotent, and crucially they do **not** re-validate existing rows — only writes validate).

At runtime the schema is never read again. Handlers in `src/server/*` issue typed queries against these tables; `record<...>` links are **type annotations only** — SurrealDB does not enforce referential existence — so a dangling link (a since-deleted persona or account) resolves to `NONE` rather than erroring. This is relied on throughout (snapshot fields, `pinged_users`, `reply_to`), and it is why `tests/schema_apply.rs` can seed rows with dangling FK ids and still exercise the migrations.

The connection itself (`db::connect` / `connect_with_retries`, [`src/db.rs:29-85`](../../src/db.rs)) is a WebSocket to `127.0.0.1:8000` by default, `Root` signin, namespace `authlyn` / database `dev` — all env-overridable (`SURREAL_URL/USER/PASS/NS/DB`).

## The table graph

`record<...>` links are the only relationship mechanism (no graph edges, no `RELATE`). They are not referentially enforced. The graph below is the link topology reconstructed from field types; arrows point from the table holding the link to its target.

```
account ──┬─< session            (account)
          ├─< media_blob          (uploader)         media_blob also <── account.avatar, persona.avatar/gallery, message.persona_avatar, custom_emoji.media
          ├─< persona             (owner)
          ├─< persona_editor      (account)          persona_editor (persona, account)
          ├─< guild               (owner)
          ├─< guild_member        (account)          guild_member (guild, account)
          ├─< dm_member           (account)          dm_member (channel, account)
          ├─< channel_guest       (account, invited_by)
          ├─< channel_active_persona (account)
          ├─< channel_read_state  (account)
          ├─< user_guild_order    (account)
          ├─< friendship          (requester, addressee)
          ├─< feedback            (author)
          ├─< push_subscription   (account)
          └─< message             (author, persona, pinged_users[])

guild ──┬─< channel               (guild, option — NONE for DM threads)
        ├─< guild_member          (guild)
        ├─< custom_emoji          (guild)
        └─< user_guild_order      (guild)

channel ──┬─< message             (channel)
          ├─< lorebook_entry      (channel)
          ├─< dm_member           (channel)          \
          ├─< dm_pair             (channel)           } the three channel-membership models
          ├─< channel_guest       (channel)          /
          ├─< channel_active_persona (channel)
          └─< channel_read_state  (channel)

persona ──< persona_image (persona), persona_editor (persona), guild_member.active_persona, channel_active_persona.persona, message.persona

message ──< message.reply_to      (self-link, option)
```

### Three channel-membership models

A single `channel` table backs guild channels and DM threads (`guild = NONE`, `kind = 'dm'`). Who may see/post in a channel is decided by **three disjoint membership tables**, branched on channel kind in `access::resolve_membership` / `access::visible_channels` (see [05-auth-privacy.md](05-auth-privacy.md)):

| Model | Table | Applies to | Grants |
|---|---|---|---|
| Guild member | `guild_member` | guild channels (`kind ∈ {text, lorebook}`) | full guild membership + per-guild role; the per-guild worn persona historically lived in `active_persona` here |
| DM member | `dm_member` (+ `dm_pair`) | DM threads (`kind = 'dm'`, `guild = NONE`) | membership in one DM/group thread |
| Guest cameo | `channel_guest` | one guild text channel | scoped, ephemeral read+post on **one** channel; no `guild_member` row → every guild-management gate auto-404s the guest |

A guest never gets a `guild_member` row, so `require_manager`/`caller_role` (which read `guild_member` only) auto-reject them; `visible_channels` surfaces exactly the one cameo channel.

## Tables by domain

Every table is `SCHEMAFULL`. `created_at` (`datetime DEFAULT time::now()`) is omitted from the field lists below except where it carries meaning.

### Identity & sessions

**`account`** — the root of the graph.

| Field | Type | Notes |
|---|---|---|
| `username` | `string` | display-cased handle |
| `username_ci` | `string` | lowercased; the uniqueness key |
| `display_name` | `string DEFAULT ''` | added after rows existed → **backfilled** (see below) |
| `password_hash` | `string` | argon2id PHC string |
| `avatar` | `option<record<media_blob>>` | NONE = no avatar |
| `created_at` | `datetime` | |

Indexes: `account_username_ci` (`username_ci`, **UNIQUE**).

The seeded **Nova DOT** system/bot account (`UPSERT account:nova_dot`, [`schema.surql:46-50`](../../src/storage/schema.surql)) authors every `kind='system'` message. Login as it is *impossible*: `password_hash = '!'` is a non-PHC sentinel that `crypto::verify_on_blocking_pool` parses to a verify-failure (**401, never 500**), and the `account_username_ci` UNIQUE index reserves the `nova-dot` handle from registration. Seeding is idempotent (UPSERT on a fixed record id). Pinned by `tests/schema_apply.rs::nova_dot_system_account_is_seeded_and_cannot_log_in`.

The removed `security_question` / `security_answer_hash` fields (self-service recovery, dropped 2026-06-17 as an account-takeover vector) are purged idempotently: `UPDATE … UNSET` then `REMOVE FIELD IF EXISTS`, ordered after the `display_name` backfill. Pinned by `tests/schema_apply.rs::applying_schema_over_account_with_security_fields_purges_them_without_crashing` — which also proves that on a `SCHEMAFULL` table the removed field rejects further writes (a stray reset can't re-populate a recovery credential).

**`session`** — the cookie carries a random token; only its SHA-256 is stored.

| Field | Type | Notes |
|---|---|---|
| `account` | `record<account>` | |
| `token_hash` | `string` | SHA-256 hex of the cookie token |
| `expires_at` | `datetime` | |

Indexes: `session_token` (`token_hash`, **UNIQUE**), `session_account` (`account`).

**`media_blob`** — server-visible images (avatars, persona art, gallery, attachments, custom emoji). Fields: `uploader record<account>`, `mime string DEFAULT 'application/octet-stream'`, `size_bytes int`, `storage_path string`. No index beyond the implicit id. The file is served at `/media/{id}` (the id string doubles as the URL). See [02-request-lifecycle.md](02-request-lifecycle.md) for the media route.

### Personas

**`persona`** — account-global; the *worn* one is per-channel (`channel_active_persona`) or historically per-guild (`guild_member.active_persona`).

| Field | Type | Notes |
|---|---|---|
| `owner` | `record<account>` | |
| `name` | `string` | |
| `description` | `string DEFAULT ''` | backfilled |
| `color` | `string DEFAULT ''` | markup-palette name (`red`…`gray`) or `''`; **no ASSERT** (validated server-side); backfilled |
| `avatar` | `option<record<media_blob>>` | primary image |
| `share_key` | `string DEFAULT ''` | redeem token granting edit access; backfilled |
| `position` | `option<int>` | wardrobe order; added after rows existed → `option<>`, NONE sorts last |

Indexes: `persona_owner` (`owner`), `persona_share_key` (`share_key`, **non-unique** — every fresh persona gets a random 22-char token; a plain index keeps redeem-lookup fast without rejecting the `''` sentinel on legacy rows). `share_key`/`color`/`description` are coalesced in one backfill ([`schema.surql:91-95`](../../src/storage/schema.surql)) so persona edits don't hit the NONE-coercion 500.

**`persona_editor`** — key-redeem grant of edit+wear access to another account. Fields: `persona record<persona>`, `account record<account>`. Index: `persona_editor_pair` (`persona, account`, **UNIQUE**).

**`persona_image`** — a persona owns N gallery images, ordered by `position`. Fields: `persona record<persona>`, `media record<media_blob>`, `position int DEFAULT 0`. Index: `persona_image_persona` (`persona`).

### Guilds & channels

**`guild`**

| Field | Type | Notes |
|---|---|---|
| `name` | `string` | |
| `owner` | `record<account>` | |
| `icon` | `option<record<media_blob>>` | |
| `deleted_at` | `option<datetime>` | soft-delete; NONE = live (purge window 30d) |
| `accent_color` | `option<string>` | per-server accent; markup-palette name or NONE; **no ASSERT** (validated in `server/accent.rs`) |

`accent_color` is `option<>` precisely so it needs no backfill over existing guilds — pinned by `tests/schema_apply.rs::applying_accent_color_over_populated_guilds_keeps_legacy_rows_with_accent_none`. The schema comment notes that if a format ASSERT is ever added it **must** use `DEFINE FIELD OVERWRITE` (the enum-widening invariant).

**`channel`** — guild text/lorebook channels *or* guild-less DM threads.

| Field | Type | Notes |
|---|---|---|
| `guild` | `option<record<guild>>` | **OVERWRITE** — widened from strict `record<guild>` for DM threads (`guild = NONE`) |
| `name` | `string` | for a group DM, the optional title |
| `kind` | `string DEFAULT 'text' ASSERT $value IN ['text','lorebook','dm']` | **OVERWRITE** — `'dm'` added by widening |
| `position` | `int DEFAULT 0` | |
| `deleted_at` | `option<datetime>` | soft-delete (purge window 1d) |
| `locked_at` | `option<datetime>` | read-only lock; set on a 1:1 DM when the two friends unfriend (history preserved, posting blocked), cleared on re-friend; only ever set on `kind='dm'` |

Index: `channel_guild` (`guild`). The two OVERWRITEs are co-tested by `tests/schema_apply.rs::widening_channel_guild_to_option_over_populated_channels_admits_guildless_dms` — which proves a guild-less `kind='dm'` channel is accepted only because both were `OVERWRITE` and not `IF NOT EXISTS`.

**`guild_member`** — the worn persona historically lives here, per-guild.

| Field | Type | Notes |
|---|---|---|
| `guild` | `record<guild>` | |
| `account` | `record<account>` | |
| `role` | `string DEFAULT 'member' ASSERT $value IN ['owner','admin','member']` | |
| `active_persona` | `option<record<persona>>` | per-guild worn persona (superseded by `channel_active_persona`) |
| `joined_at` | `datetime` | |

Indexes: `guild_member_pair` (`guild, account`, **UNIQUE**), `guild_member_account` (`account`, **non-unique**). The account-only index exists because `access::visible_channels` asks "which guilds is this account in?" on every `/events` connect, every ListsChanged visibility reload, and every `GET /unread`; an account-only predicate cannot use the `(guild, account)` composite (account is not its prefix), so without it the query planned a full `TableScan`. Pinned by `tests/schema_apply.rs::new_guild_member_account_index_applies_over_populated_rows`.

### DM threads

**`dm_member`** — one row per (account, DM thread). Fields: `channel record<channel>`, `account record<account>`, `joined_at datetime`. Indexes: `dm_member_pair` (`channel, account`, **UNIQUE**), `dm_member_account` (`account`, non-unique, same `visible_channels` hot-path rationale as `guild_member_account`). Pinned by `tests/schema_apply.rs::new_dm_member_account_index_applies_over_populated_rows`. No `active_persona` here — DM persona wear rides `channel_active_persona` like every channel.

**`dm_pair`** — the 1:1 DM dedup lock. Fields: `pair_key string`, `channel record<channel>`. Index: `dm_pair_key` (`pair_key`, **UNIQUE**).

This is a side table for a non-obvious reason. Concurrent "message X" double-taps (or both parties initiating at once) must converge on **one** thread; a check-then-create read cannot arbitrate that — the two racing `CREATE`s touch disjoint records, so MVCC never sees a conflict without a *shared* UNIQUE key. `dm_pair` provides that shared key (the sorted account-id pair, joined by `\u{1f}`). A `UNIQUE pair_key` column *on `channel`* is forbidden: the many guild/group/soft-deleted channels share `pair_key = NONE` and would collide, crashing schema-apply over a populated `channel` table. Groups never get a `dm_pair` row (a pair key is meaningless for 3+), so the table only ever holds distinct non-NONE keys → the UNIQUE index never sees a NONE collision. Lifecycle: created inside the 1:1 thread's create transaction; deleted when the thread is soft-deleted (last member leaves) or hard-purged. Pinned by `tests/schema_apply.rs::dm_pair_unique_index_rejects_duplicate_pair_and_reapplies`.

**`channel_guest`** — the guest-cameo grant (third membership model).

| Field | Type | Notes |
|---|---|---|
| `channel` | `record<channel>` | the one guild text channel |
| `account` | `record<account>` | the guest |
| `invited_by` | `record<account>` | keys revoke-authz + the unfriend-revoke hook |
| `expires_at` | `option<datetime>` | NONE = no expiry |

Indexes: `channel_guest_pair` (`channel, account`, **UNIQUE**), `channel_guest_account` (`account`, non-unique, `visible_channels` + `GET /cameos` hot path). Ephemerality is **lazy**: `expires_at` is enforced as a predicate (`expires_at = NONE OR expires_at > time::now()`) at every membership query, so an expired row resolves to non-member with no sweep required; the hourly purge only deletes expired rows for index hygiene. Pinned by `tests/schema_apply.rs::new_channel_guest_account_index_applies_over_populated_rows`.

### Messages

**`message`** — channel-scoped, plaintext, persona-aware, immutable historical utterances. The 17 fields mix four distinct kinds of data; the **live-vs-snapshot split is correctness- and security-relevant**.

| Field | Type | Class | Notes |
|---|---|---|---|
| `channel` | `record<channel>` | live link | |
| `author` | `record<account>` | live link | |
| `persona` | `option<record<persona>>` | live link | "speaking as" |
| `persona_name` | `option<string>` | **snapshot** | frozen at send time |
| `persona_description` | `option<string>` | **snapshot** | |
| `persona_color` | `option<string>` | **snapshot** | |
| `persona_avatar` | `option<record<media_blob>>` | **snapshot** | a record *link*, not a file copy |
| `body` | `string` | content | plaintext |
| `attachments` | `array<string> DEFAULT []` | id-string array | media_blob ids (strings — serve directly as `/media/{id}`) |
| `tier` | `string DEFAULT 'default'` | discriminator | forward-compat AI-visibility |
| `kind` | `string DEFAULT 'user' ASSERT $value IN ['user','system','roll']` | discriminator | **OVERWRITE**; `system`=Nova DOT broadcast, `roll`=Fate Engine (both immutable; edit/delete 403) |
| `effect` | `option<string> ASSERT $value = NONE OR $value IN ['whisper','shout','spell']` | discriminator | purely cosmetic delivery flourish |
| `reply_to` | `option<record<message>>` | live self-link | reply preview resolved by null-safe join; a since-deleted parent joins to NONE |
| `pinged_users` | `array<record<account>> DEFAULT []` | link array | `@`-mentioned guild members (resolved server-side at send) |
| `guest_cameo` | `bool DEFAULT false` | discriminator | true iff author posted as a guest; **snapshotted** so the badge survives cameo revocation |
| `deleted_at` | `option<datetime>` | soft-delete | NONE = live (purge window 1h) |
| `sent_at` | `datetime` | cursor | |

Index: `message_channel_sent` (`channel, sent_at`, **non-unique** — the message-list cursor).

**Why snapshot persona identity:** a message is an immutable historical record. Renaming or deleting the persona must not scramble the name/avatar shown on past messages, so `persona_name/description/color/avatar` are frozen at send time rather than resolved from the live `persona.*` at render. The avatar snapshot is a record *link* (the file is not duplicated); changing the persona's avatar later leaves past messages pointing at the image they were sent with. `guest_cameo` is snapshotted for the same reason (the badge must outlive the `channel_guest` row).

The enum guards are real on the pinned beta, not cosmetic: `tests/schema_apply.rs::message_kind_guard_accepts_known_set_rejects_other` and `::message_effect_guard_accepts_known_set_rejects_other` prove the accepted set persists and an out-of-set value is rejected.

### Lorebooks, friends, and ancillary tables

**`lorebook_entry`** (SillyTavern-style, on a `kind='lorebook'` channel): `channel record<channel>`, `title string DEFAULT ''`, `keys array<string>` (trigger keywords), `content string` (injected text), `enabled bool DEFAULT true`, `position int DEFAULT 0`. Index: `lorebook_entry_channel` (`channel`).

**`friendship`** (one directed row; a reverse-pending request auto-accepts): `requester record<account>`, `addressee record<account>`, `state string DEFAULT 'pending' ASSERT $value IN ['pending','accepted']`, `updated_at datetime`. Index: `friendship_pair` (`requester, addressee`, **UNIQUE**).

**`channel_active_persona`** (per-channel worn persona; supersedes `guild_member.active_persona`): one row per (account, channel); UPSERT on wear, DELETE on unwear. Fields: `account`, `channel`, `persona` (all `record<…>`). Index: `channel_active_persona_pair` (`account, channel`, **UNIQUE**).

**`channel_read_state`** (cross-device read high-water mark): `account`, `channel`, `last_seen_at datetime`, `last_seen_id string`, `updated_at`. Index: `channel_read_state_pair` (`account, channel`, **UNIQUE**). The `POST /channels/{cid}/mark-read` handler UPSERTs the MAX cursor (an older POST never regresses a newer mark); a racy double-mark resolves via `with_write_conflict_retry` (see [04-realtime-sse.md](04-realtime-sse.md)).

**`user_guild_order`** (per-user guild-rail order): `account`, `guild`, `position int`. Index: `user_guild_order_pair` (`account, guild`, **UNIQUE**). `PUT /rail/order` replaces an account's rows wholesale; guilds with no row sort last.

**`feedback`** (#31): `author record<account>`, `kind string DEFAULT 'other'` (bug/idea/other — coerced server-side, **no ASSERT**), `body string`, `context option<string>` (client JSON), `status string DEFAULT 'new'` (new/read/resolved). Index: `feedback_created` (`created_at`).

**`push_subscription`** (#30, Web Push): `account`, `endpoint string`, `p256dh string` (receiver ECDH pubkey), `` `auth` string `` (receiver auth secret — backtick-quoted because `auth` is reserved). Indexes: `push_subscription_endpoint` (`endpoint`, **UNIQUE** — re-subscribing the same browser upserts), `push_subscription_account` (`account`).

**`custom_emoji`** (Discord-style shortcodes, guild-scoped): `guild`, `name string` (the `:name:` shortcode, validated `^[a-z0-9_]{2,32}$` server-side), `media record<media_blob>`, `creator record<account>`. Index: `custom_emoji_guild_name` (`guild, name`, **UNIQUE** — one shortcode per guild; the same name is fine in different guilds).

## Index registry

The 13 **UNIQUE** indexes (note the composite/pair indexes — they enforce one-row-per-relationship and drive UPSERT idempotency):

| Index | Table | Fields | Enforces |
|---|---|---|---|
| `account_username_ci` | `account` | `username_ci` | case-insensitive handle uniqueness |
| `session_token` | `session` | `token_hash` | one session per token |
| `persona_editor_pair` | `persona_editor` | `persona, account` | one editor grant per pair |
| `guild_member_pair` | `guild_member` | `guild, account` | one membership per (guild, account) |
| `channel_active_persona_pair` | `channel_active_persona` | `account, channel` | one worn persona per (account, channel) |
| `dm_member_pair` | `dm_member` | `channel, account` | one membership per (thread, account) |
| `dm_pair_key` | `dm_pair` | `pair_key` | **1:1 DM dedup arbiter (MVCC)** |
| `channel_guest_pair` | `channel_guest` | `channel, account` | one cameo per (channel, account) |
| `friendship_pair` | `friendship` | `requester, addressee` | one directed friendship row |
| `push_subscription_endpoint` | `push_subscription` | `endpoint` | one row per browser endpoint |
| `custom_emoji_guild_name` | `custom_emoji` | `guild, name` | one shortcode per guild |
| `channel_read_state_pair` | `channel_read_state` | `account, channel` | one read-state per (account, channel) |
| `user_guild_order_pair` | `user_guild_order` | `account, guild` | one rail-order per (account, guild) |

The 12 **non-unique** indexes are lookup accelerators: `session_account`, `persona_owner`, `persona_share_key`, `persona_image_persona`, `channel_guild`, `guild_member_account`, `dm_member_account`, `channel_guest_account`, `message_channel_sent` (`channel, sent_at` — the message cursor), `lorebook_entry_channel`, `feedback_created`, `push_subscription_account`. The three `*_account` indexes (`guild_member_account`, `dm_member_account`, `channel_guest_account`) are the M-37 hot-path additions for `access::visible_channels`; each is built over already-populated rows at apply time (no backfill hazard for indexes), pinned by the three `new_*_account_index_applies_over_populated_rows` tests.

## Enum / format ASSERT registry

Five field-level enum/format guards. **Widening any of these requires `DEFINE FIELD OVERWRITE`** (see below); the two that were widened post-ship already use it.

| Field | ASSERT | OVERWRITE? | Pin |
|---|---|---|---|
| `channel.kind` | `$value IN ['text','lorebook','dm']` | **yes** (`'dm'` added) | `schema_applies_and_kind_guard_holds`, `widening_channel_guild_to_option_over_populated_channels_admits_guildless_dms` |
| `message.kind` | `$value IN ['user','system','roll']` | **yes** (`'roll'` added) | `message_kind_guard_accepts_known_set_rejects_other`, `widened_kind_assert_reaches_a_db_where_kind_already_exists` |
| `message.effect` | `$value = NONE OR $value IN ['whisper','shout','spell']` | no | `message_effect_guard_accepts_known_set_rejects_other` |
| `guild_member.role` | `$value IN ['owner','admin','member']` | no (shipped with this set) | (code-pinned: [`schema.surql:162`](../../src/storage/schema.surql); exercised by `tests/guilds.rs`) |
| `friendship.state` | `$value IN ['pending','accepted']` | no (shipped with this set) | (code-pinned: [`schema.surql:380`](../../src/storage/schema.surql); exercised by `tests/friends.rs`) |

Fields deliberately **without** an ASSERT, validated server-side instead: `persona.color`, `guild.accent_color` (both markup-palette names → `server/accent.rs`), `feedback.kind`, `feedback.status`, `message.tier`.

## Migration discipline

This is the most expensive-to-relearn part of the codebase. The schema is re-applied on every boot over the existing (populated) prod DB. A `SCHEMAFULL` table **re-validates the entire row on any write**, and a field added to an existing table holds `NONE` on legacy rows until materialised. The two ways that bites, and the rules:

### Adding a field — decision tree

```
Adding a DEFINE FIELD to a table that may already have rows?
│
├─ Is NONE an acceptable value for this field?
│    └─ YES → declare it option<…>. Done. No backfill.
│             (NONE is valid, so no later row-revalidating write trips on it.)
│
└─ NO (non-option: string / array / bool / int with semantics) →
     the field holds NONE on legacy rows, and the NEXT write to such a row
     re-validates the whole row and fails coercion ("Expected X but found NONE"),
     crash-looping boot. You MUST add an idempotent COALESCED backfill that runs
     BEFORE any later row-revalidating UPDATE, and FOLD it into the existing
     first message backfill if the field is on `message` (see below).
```

`option<>` additions pinned by `applying_effect_over_populated_messages_keeps_legacy_rows_with_effect_none` and `applying_accent_color_over_populated_guilds_keeps_legacy_rows_with_accent_none`. The `display_name` backfill (a non-option `string DEFAULT ''` added to `account`) pinned by `applying_schema_over_legacy_account_backfills_display_name_and_keeps_avatar_none`.

### Coalesce, never bare-assign (the 2026-06-01 data-loss rule)

Backfills must use `field ?? <default>`, **never** a bare `field = <default>`. A bare `attachments = []` *wipes* an existing array on every matched row. Because every legacy row had `pinged_users = NONE`, the migration's `WHERE` matched **all** rows, so `attachments = []` destroyed every message's attachments on the first apply over prod (gallery images vanished, 2026-06-01). `field ?? []` preserves a populated array and only materialises a NONE. Pinned by `applying_kind_over_populated_messages_materialises_without_wiping_attachments` (asserts `attachments == ['keep-this-blob']` survives).

### All non-option `message` fields backfill together, in one statement

The single most subtle rule. The `message` backfill ([`schema.surql:351-356`](../../src/storage/schema.surql)) materialises **all four** non-option fields — `attachments`, `pinged_users`, `kind`, `guest_cameo` — in *one* `UPDATE`:

```sql
UPDATE message
    SET attachments  = attachments ?? [],
        pinged_users = pinged_users ?? [],
        kind         = kind ?? 'user',
        guest_cameo  = guest_cameo ?? false
    WHERE attachments = NONE OR pinged_users = NONE OR kind = NONE OR guest_cameo = NONE;
```

A separate per-field backfill cannot work: the first write to a legacy row re-validates the *whole* row and trips over whichever of the other three is still NONE, aborting apply. This statement also **must precede** the persona-snapshot `UPDATE` at [`schema.surql:358-363`](../../src/storage/schema.surql), which likewise re-validates every field. (`reply_to` and `effect` are `option<>`, so NONE is valid and they are left alone.) Pinned by `applying_guest_cameo_over_populated_messages_materialises_without_wiping_attachments` and, at scale (100 rows + re-apply idempotency), by `applying_full_schema_over_prod_shaped_populated_db_is_crash_free_and_idempotent`.

### Widening an enum or a record-link type — use `DEFINE FIELD OVERWRITE`

`DEFINE FIELD IF NOT EXISTS` is a **no-op over a field that already exists** — on a populated prod DB the old, narrower definition silently survives and rejects the new value. Widening therefore requires `DEFINE FIELD OVERWRITE`, which re-defines unconditionally. Re-defining is idempotent and does **not** re-validate existing rows (only writes validate), so it never touches the backfill hazard. The three OVERWRITE sites:

| Site | Widening | Pin |
|---|---|---|
| `channel.guild` ([`schema.surql:141`](../../src/storage/schema.surql)) | `record<guild>` → `option<record<guild>>` (DM threads) | `widening_channel_guild_to_option_over_populated_channels_admits_guildless_dms` |
| `channel.kind` ([`schema.surql:146`](../../src/storage/schema.surql)) | enum `+ 'dm'` | same test |
| `message.kind` ([`schema.surql:281`](../../src/storage/schema.surql)) | enum `['user','system'] → + 'roll'` | `widened_kind_assert_reaches_a_db_where_kind_already_exists` |

(There are **three** `DEFINE FIELD OVERWRITE` statements — `channel.guild` (`schema.surql:141`), `channel.kind` (`:146`), `message.kind` (`:281`); a bare `grep -c "DEFINE FIELD OVERWRITE"` returns 4 only because the `--` comment at `schema.surql:130` also contains the literal phrase.) The widened-enum test seeds a DB whose `kind` field *already exists* with the old two-value ASSERT — exactly prod's state at deploy — and proves a `'roll'`/`'dm'` insert is accepted only because the definition was re-applied.

> One `SCHEMAFULL` subtlety the tests lean on: an undefined field is **silently stripped** on write, not rejected (`SCHEMAFULL` drops unknown fields). So a migration is only proven by a *persisted read-back* — writing a value and reading it back NONE means the field was never actually defined. The over-populated guards (`effect`, `accent_color`) all do this round-trip.

## Soft-delete & the purge sweep

Soft-delete is uniform across the three deletable tables — `guild`, `channel`, `message` — via `deleted_at option<datetime>` (NONE = live, a datetime = when it was trashed). **Every read filters `deleted_at = NONE`**; the matching trash listing filters `deleted_at != NONE`; restore sets it back to NONE. The full soft-delete → trash → restore lifecycle (including the privacy-404 / 403 authorization matrix on restore) is pinned by `tests/soft_delete.rs::{guild,channel,message}_soft_delete_then_restore` and the restore-authz family (`restoring_someone_elses_deleted_message_is_403`, `restore_collapses_to_privacy_404_for_non_members_and_unknown_channels`, `restore_in_a_soft_deleted_channel_or_guild_is_privacy_404`, `restore_of_an_already_live_message_is_an_idempotent_204`, etc.). Restore is idempotent (the undo toast can race its own timeout), and restoring a *purged* message is a 403, never a 500.

### Purge windows

`purge_soft_deleted` ([`src/server/mod.rs:366-442`](../../src/server/mod.rs)) hard-deletes rows past their rollback window:

| Table | Window |
|---|---|
| `message` | 1 hour |
| `channel` | 1 day |
| `guild` | 30 days |

It is spawned by `spawn_purge_sweep` ([`src/server/mod.rs:445-455`](../../src/server/mod.rs)) on a `tokio::time::interval(3600s)` — once shortly after boot, then **hourly**. The window pins live in `tests/soft_delete.rs`, not `schema_apply.rs` (the windows are server logic, not schema): `purge_hard_deletes_message_past_window_only` (backdates one message past 1h, leaves another fresh) verifies the 1h boundary; the cascade tests verify 1d/30d.

### Cascades

A purge cascades to children that were never individually soft-deleted:

- **Channel (1d):** deletes the channel's `message`, `lorebook_entry`, `channel_active_persona`, `channel_read_state`, `dm_member`, `dm_pair`, `channel_guest` before the `channel` row. Pinned by `purge_cascades_channel_to_its_messages` (+ `purge_should_cascade_dm_member_rows` for the DM arm).
- **Guild (30d):** deletes its channels' `message`/`lorebook_entry`/`channel_active_persona`/`channel_read_state`/`channel_guest`, then `channel`, then `guild_member`, `custom_emoji`, `user_guild_order`, then the `guild`. Pinned by `purge_cascades_guild_to_channels_and_messages`, `purge_cascades_guild_to_all_child_tables`, and `purge_should_cascade_guild_member_rows`.
- **Expired cameos:** `channel_guest` rows past `expires_at` are deleted for index hygiene (the lazy-check already excludes them from membership queries).

### The beta.3 composite-index mis-plan dodge

The load-bearing footgun in the purge. SurrealDB **3.1.0-beta.3 mis-plans** a `DELETE` whose `WHERE` filters a **composite-index leading column** with `IN` against a **`LET` variable** — it silently matches **zero rows**, orphaning the children. `guild_member_pair` is `(guild, account)`, so `DELETE guild_member WHERE guild IN $g` (with `$g` a `LET` var) silently no-ops. The fix is to **inline the subquery** instead of binding it to a `LET`:

```sql
-- BROKEN on beta.3 (LET var + composite-index leading column → 0 rows):
LET $g = (SELECT VALUE id FROM guild WHERE deleted_at != NONE AND deleted_at < time::now() - 30d);
DELETE guild_member WHERE guild IN $g;          -- silently matches nothing

-- WORKS (inline subquery):
DELETE guild_member WHERE guild IN (SELECT VALUE id FROM guild
    WHERE deleted_at != NONE AND deleted_at < time::now() - 30d);
```

The same dodge is applied to every other composite-index-leading-column delete: `dm_member` (`channel, account`), `dm_pair` (uses `channel`), `channel_guest` (`channel, account`), `custom_emoji` (`guild, name`), `user_guild_order` (matched the same way for consistency). The non-composite-leading deletes (`message WHERE channel IN …`, `channel WHERE guild IN $g`, `guild WHERE id IN $g`) safely keep the `$g` `LET` var. This is the single most expensive-to-rediscover purge bug; the regression is pinned by `tests/soft_delete.rs::purge_should_cascade_guild_member_rows` (which previously *failed*) and `::purge_should_cascade_dm_member_rows`. See the dense rationale in [`src/server/mod.rs:369-373`](../../src/server/mod.rs).

## SurrealDB version coupling

The schema is SurrealQL *executed by the database*, not Rust — schema correctness (every `ASSERT`, every `DEFINE`, the coalesce dialect, the index planner) is validated by the **live** SurrealDB at apply time, not at compile time. That is why `tests/schema_apply.rs` and `tests/soft_delete.rs` **require a running `surreal` on `ws://127.0.0.1:8000`** (the `/dev-db` task, see [09-testing.md](09-testing.md)). The SDK is pinned `=3.1.0-beta.3` and the on-machine CLI must share the **3.x** major (divergent error texts and — as the purge bug shows — divergent query plans otherwise); see [`Cargo.toml`](../../Cargo.toml) and `tests/retry_canary.rs`.

## Known doc gap

`src/storage/mod.rs` has **no `//!` module header** (it is a one-line `include_str!` const), the lone violation of the "every module a `//!` header" convention in [CLAUDE.md](../../CLAUDE.md). `schema.surql` itself carries a strong top-of-file header and dense per-field rationale comments — those comments are the de-facto source of truth for every migration hazard above.

## Source map

Key files:

- [`src/storage/schema.surql`](../../src/storage/schema.surql) — the entire data model: 21 `SCHEMAFULL` tables, 114 fields, 25 indexes (13 UNIQUE), 5 enum/format ASSERTs, 3 `DEFINE FIELD OVERWRITE`, the coalesced backfills, and the `nova_dot` seed. Dense `--` comments carry the migration-hazard rationale per field; statement order is load-bearing.
- [`src/storage/mod.rs`](../../src/storage/mod.rs) — one line: `pub const SCHEMA: &str = include_str!("schema.surql")`.
- [`src/db.rs`](../../src/db.rs) — `apply_schema` (`:53-56`, the whole `.surql` as one `query().check()`), `connect`/`connect_with_retries` (`:29-85`). The only consumer of `SCHEMA`.
- [`src/server/mod.rs`](../../src/server/mod.rs) — `purge_soft_deleted` (`:366-442`, windowed hard-delete + cascade + the beta.3 inline-subquery dodge), `spawn_purge_sweep` (`:445-455`, the hourly sweep).

Tests that pin the claims:

- `tests/schema_apply.rs` — apply-cleanliness; every enum-ASSERT guard (`schema_applies_and_kind_guard_holds`, `message_kind_guard_accepts_known_set_rejects_other`, `message_effect_guard_accepts_known_set_rejects_other`); the `option<>`-no-backfill guards (`applying_effect_over_populated_messages_keeps_legacy_rows_with_effect_none`, `applying_accent_color_over_populated_guilds_keeps_legacy_rows_with_accent_none`); the coalesced-backfill / no-wipe guards (`applying_kind_over_populated_messages_materialises_without_wiping_attachments`, `applying_guest_cameo_over_populated_messages_materialises_without_wiping_attachments`); the `display_name` + security-field migrations (`applying_schema_over_legacy_account_backfills_display_name_and_keeps_avatar_none`, `applying_schema_over_account_with_security_fields_purges_them_without_crashing`); the OVERWRITE widenings (`widened_kind_assert_reaches_a_db_where_kind_already_exists`, `widening_channel_guild_to_option_over_populated_channels_admits_guildless_dms`); the new-index-over-populated-rows guards (`new_guild_member_account_index_applies_over_populated_rows`, `new_dm_member_account_index_applies_over_populated_rows`, `new_channel_guest_account_index_applies_over_populated_rows`); the `dm_pair` dedup arbiter (`dm_pair_unique_index_rejects_duplicate_pair_and_reapplies`); the `nova_dot` seed (`nova_dot_system_account_is_seeded_and_cannot_log_in`); and the prod-shape deploy gate (`applying_full_schema_over_prod_shaped_populated_db_is_crash_free_and_idempotent`).
- `tests/soft_delete.rs` — soft-delete → trash → restore lifecycle and restore-authz matrix (`{guild,channel,message}_soft_delete_then_restore`, `restoring_someone_elses_deleted_message_is_403`, `restore_collapses_to_privacy_404_for_non_members_and_unknown_channels`, `restore_in_a_soft_deleted_channel_or_guild_is_privacy_404`, `restoring_a_purged_message_is_403_not_500`, `restore_of_an_already_live_message_is_an_idempotent_204`, `restore_with_a_cross_channel_message_id_is_403`); purge windows + cascade (`purge_hard_deletes_message_past_window_only`, `purge_cascades_channel_to_its_messages`, `purge_cascades_guild_to_channels_and_messages`, `purge_cascades_guild_to_all_child_tables`); and the composite-index mis-plan regression (`purge_should_cascade_guild_member_rows`, `purge_should_cascade_dm_member_rows`, `leaving_a_dm_removes_member_rows_so_the_leave_path_never_orphans`).
- `tests/common/mod.rs` — the harness: `test_db`/`arena` apply `storage::SCHEMA` to a fresh isolated namespace; `raw_db` deliberately skips it so migration tests can seed an *old* schema and re-apply the real one over pre-existing rows.
