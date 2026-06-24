-- authlyn-interactive — SurrealDB schema (Discord-style roleplay platform)
--
-- Loaded on startup by `db::apply_schema`. Most DEFINEs use `IF NOT EXISTS`
-- so re-applying on every boot is a no-op — except the fields whose definition
-- was widened after shipping and must re-apply over an existing field:
-- `channel.guild`, `channel.kind`, `message.kind`, and `guild_member.role`,
-- which use OVERWRITE (each documented at its block).
--
-- Server-trusted model: the server stores plaintext content directly.
-- `record<...>` links are type annotations only (SurrealDB does not enforce
-- referential existence), so the mutual account<->media_blob links are fine
-- regardless of define order. Never `<string>`-cast a datetime that drives
-- ORDER BY / a cursor (lex-mis-orders at format-class boundaries); ordering
-- columns that need it use an integer `position` instead.

-- Accounts -------------------------------------------------------------
DEFINE TABLE IF NOT EXISTS account SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS username      ON account TYPE string;
DEFINE FIELD IF NOT EXISTS username_ci   ON account TYPE string;  -- lowercased, for case-insensitive uniqueness
DEFINE FIELD IF NOT EXISTS display_name  ON account TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS password_hash ON account TYPE string;  -- argon2id PHC string
DEFINE FIELD IF NOT EXISTS avatar        ON account TYPE option<record<media_blob>>;
DEFINE FIELD IF NOT EXISTS created_at    ON account TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS account_username_ci ON account FIELDS username_ci UNIQUE;
-- Same NONE-coercion guard as persona: accounts predating `display_name` hold
-- NONE there, which would 500 any account UPDATE (e.g. change-password).
UPDATE account SET display_name = (display_name ?? '') WHERE display_name = NONE;

-- Security question/answer (self-service password recovery) was REMOVED
-- (2026-06-17): obsolete, and the self-service reset was an account-takeover
-- vector — a session could set a recovery credential WITHOUT the password, then
-- reset through it. Admin reset (/auth/admin/reset-password) is the sole
-- recovery path now. Purge any stored values, then drop the fields. Idempotent:
-- UNSET of an absent key and REMOVE FIELD IF EXISTS are both no-ops on reboot.
-- Runs AFTER the display_name guard above so this account UPDATE can't hit the
-- SCHEMAFULL NONE-coercion 500 on pre-display_name rows.
UPDATE account UNSET security_question, security_answer_hash;
REMOVE FIELD IF EXISTS security_question ON account;
REMOVE FIELD IF EXISTS security_answer_hash ON account;

-- Reserved Nova DOT system/bot account — the author of every `kind='system'`
-- message (the app-admin broadcast feature). Seeded here (pure SurrealQL) so it
-- exists on prod boot AND in every test arena. Login as it is IMPOSSIBLE: the
-- `password_hash` is a non-PHC sentinel, so `verify_on_blocking_pool` parses it to
-- a verify-failure (401, never 500), and the `account_username_ci` UNIQUE index
-- reserves the 'nova-dot' handle from registration. Idempotent: UPSERT on the
-- fixed record id; `created_at` gets its DEFAULT on first create, untouched after.
UPSERT account:nova_dot SET
    username = 'nova-dot',
    username_ci = 'nova-dot',
    display_name = 'Nova DOT',
    password_hash = '!';

-- Sessions (cookie carries a random token; we store only its SHA-256) ---
DEFINE TABLE IF NOT EXISTS session SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS account    ON session TYPE record<account>;
DEFINE FIELD IF NOT EXISTS token_hash ON session TYPE string;  -- SHA-256 hex of the cookie token
DEFINE FIELD IF NOT EXISTS created_at ON session TYPE datetime DEFAULT time::now();
DEFINE FIELD IF NOT EXISTS expires_at ON session TYPE datetime;
DEFINE INDEX IF NOT EXISTS session_token   ON session FIELDS token_hash UNIQUE;
DEFINE INDEX IF NOT EXISTS session_account ON session FIELDS account;

-- Media blobs (server-visible images: avatars, persona art, gallery) ----
DEFINE TABLE IF NOT EXISTS media_blob SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS uploader     ON media_blob TYPE record<account>;
DEFINE FIELD IF NOT EXISTS mime         ON media_blob TYPE string DEFAULT 'application/octet-stream';
DEFINE FIELD IF NOT EXISTS size_bytes   ON media_blob TYPE int;
DEFINE FIELD IF NOT EXISTS storage_path ON media_blob TYPE string;
DEFINE FIELD IF NOT EXISTS created_at   ON media_blob TYPE datetime DEFAULT time::now();

-- Personas (account-global; the "worn" one is per-guild, see guild_member)
DEFINE TABLE IF NOT EXISTS persona SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS owner       ON persona TYPE record<account>;
DEFINE FIELD IF NOT EXISTS name        ON persona TYPE string;
DEFINE FIELD IF NOT EXISTS description ON persona TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS color       ON persona TYPE string DEFAULT '';  -- markup palette name (red…gray) or '' for none
DEFINE FIELD IF NOT EXISTS avatar      ON persona TYPE option<record<media_blob>>;  -- primary image
DEFINE FIELD IF NOT EXISTS share_key   ON persona TYPE string DEFAULT '';  -- redeem token granting edit access
DEFINE FIELD IF NOT EXISTS created_at  ON persona TYPE datetime DEFAULT time::now();
-- Wardrobe display order. Added after rows existed, so it must be `option<int>`:
-- old rows hold NONE (valid, no backfill) and a later UPDATE re-validating all
-- fields won't crash. NONE sorts last in `list_personas` (coalesced).
DEFINE FIELD IF NOT EXISTS position    ON persona TYPE option<int>;
DEFINE INDEX IF NOT EXISTS persona_owner ON persona FIELDS owner;
-- Non-unique: every fresh persona gets a random 22-char token (collision-free
-- in practice); a plain index keeps the redeem lookup fast without rejecting
-- the empty-default sentinel on any pre-existing rows.
DEFINE INDEX IF NOT EXISTS persona_share_key ON persona FIELDS share_key;
-- Backfill string fields added after rows already existed (share_key, color).
-- SCHEMAFULL DEFAULT only applies on write, so old personas hold NONE there;
-- any UPDATE then fails coercion ("Expected string but found NONE"), breaking
-- persona edits. Materialise them so edits work. Idempotent via the WHERE.
UPDATE persona SET
    share_key   = (share_key ?? ''),
    color       = (color ?? ''),
    description = (description ?? '')
    WHERE share_key = NONE OR color = NONE OR description = NONE;

-- Persona editors (key-redeem grants another account edit+wear access) -----
DEFINE TABLE IF NOT EXISTS persona_editor SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS persona    ON persona_editor TYPE record<persona>;
DEFINE FIELD IF NOT EXISTS account    ON persona_editor TYPE record<account>;
DEFINE FIELD IF NOT EXISTS created_at ON persona_editor TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS persona_editor_pair ON persona_editor FIELDS persona, account UNIQUE;

-- Persona gallery (a persona owns N images, ordered by `position`) -------
DEFINE TABLE IF NOT EXISTS persona_image SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS persona    ON persona_image TYPE record<persona>;
DEFINE FIELD IF NOT EXISTS media      ON persona_image TYPE record<media_blob>;
DEFINE FIELD IF NOT EXISTS position   ON persona_image TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS created_at ON persona_image TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS persona_image_persona ON persona_image FIELDS persona;

-- Guilds (servers) -----------------------------------------------------
DEFINE TABLE IF NOT EXISTS guild SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS name       ON guild TYPE string;
DEFINE FIELD IF NOT EXISTS owner      ON guild TYPE record<account>;
DEFINE FIELD IF NOT EXISTS icon       ON guild TYPE option<record<media_blob>>;
DEFINE FIELD IF NOT EXISTS created_at ON guild TYPE datetime DEFAULT time::now();
-- Soft-delete (#22): NONE = live; a datetime = when it was trashed. A purge
-- sweep hard-deletes past the rollback window (guild 30d). option<> ⇒ NONE is
-- valid, no backfill needed. EVERY guild read filters `deleted_at = NONE`.
DEFINE FIELD IF NOT EXISTS deleted_at ON guild TYPE option<datetime>;
-- Per-server accent (Open Question #5 / _foundation.scss): a markup-palette
-- name (red…gray, the SAME 8-name vocabulary as persona.color / --tint-*) or
-- NONE for the default electric-blue accent. option<> ⇒ NONE is valid on every
-- pre-existing guild row, so NO backfill is needed and adding it can't trip the
-- SCHEMAFULL NONE-coercion crash (same reasoning as account.avatar
-- and message.effect). Purely cosmetic; gates nothing. NO format ASSERT: the
-- palette is validated server-side (server/accent.rs), mirroring persona.color
-- (a free TYPE string with no ASSERT). If a format ASSERT is ever added it MUST
-- use DEFINE FIELD OVERWRITE, not IF NOT EXISTS (enum-OVERWRITE invariant).
DEFINE FIELD IF NOT EXISTS accent_color ON guild TYPE option<string>;

-- Channels (guild text/lorebook channels, or guild-less DM threads) -----
DEFINE TABLE IF NOT EXISTS channel SCHEMAFULL;
-- M7/P1: a DM thread is a channel with NO guild, so `guild` is option<>.
-- OVERWRITE (not IF NOT EXISTS): on a populated DB the field already exists as
-- the strict `record<guild>`, and IF NOT EXISTS is a no-op there — it would keep
-- the strict type and reject `guild = NONE` on every DM create. Existing rows all
-- hold a guild (valid for option<record>), so no backfill / no NONE-coercion.
-- (The file-wide IF NOT EXISTS exception, here for a record-link widening.)
DEFINE FIELD OVERWRITE     guild      ON channel TYPE option<record<guild>>;
DEFINE FIELD IF NOT EXISTS name       ON channel TYPE string;
-- M7/P1: `'dm'` added. OVERWRITE (not IF NOT EXISTS) — widening an enum ASSERT
-- needs OVERWRITE or the old narrow ASSERT silently survives and rejects 'dm'
-- (the same trap as message.kind below). DEFAULT 'text' unchanged.
DEFINE FIELD OVERWRITE     kind       ON channel TYPE string DEFAULT 'text' ASSERT $value IN ['text', 'lorebook', 'dm'];
DEFINE FIELD IF NOT EXISTS position   ON channel TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS created_at ON channel TYPE datetime DEFAULT time::now();
-- Soft-delete (#22): NONE = live (purge window: channel 1d). See guild.deleted_at.
DEFINE FIELD IF NOT EXISTS deleted_at ON channel TYPE option<datetime>;
-- Read-only lock (M7/P1, review M2): NONE = live; a timestamp = locked. Set on a
-- 1:1 DM when the two friends unfriend (owner ruling: history is preserved but
-- posting is blocked), cleared on re-friend. New optional field → no backfill.
-- Only ever set on kind='dm' channels today; guild channels stay NONE.
DEFINE FIELD IF NOT EXISTS locked_at ON channel TYPE option<datetime>;
DEFINE INDEX IF NOT EXISTS channel_guild ON channel FIELDS guild;

-- Guild membership (the worn persona lives here, per-guild) -------------
DEFINE TABLE IF NOT EXISTS guild_member SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS guild          ON guild_member TYPE record<guild>;
DEFINE FIELD IF NOT EXISTS account        ON guild_member TYPE record<account>;
-- M7: `'admin'` added (review C1, bug hunt 019ef87b). OVERWRITE (not IF NOT
-- EXISTS) — widening an enum ASSERT needs OVERWRITE or the old narrow ASSERT
-- (`['owner','member']`) silently survives on a migrated DB and rejects 'admin'
-- (a 500 in set_member_role; the same trap as channel.kind/message.kind).
-- Re-defining is idempotent and does NOT re-validate existing rows. DEFAULT
-- 'member' unchanged; old 'owner'/'member' rows are a subset of the wider set.
DEFINE FIELD OVERWRITE     role           ON guild_member TYPE string DEFAULT 'member' ASSERT $value IN ['owner', 'admin', 'member'];
DEFINE FIELD IF NOT EXISTS active_persona ON guild_member TYPE option<record<persona>>;
DEFINE FIELD IF NOT EXISTS joined_at      ON guild_member TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS guild_member_pair ON guild_member FIELDS guild, account UNIQUE;
-- M1 hot path (review M-37): `access::visible_channels` asks "which guilds is
-- this account in?" on every /events connect, every per-connection ListsChanged
-- visibility reload, and every GET /unread. An account-only predicate cannot
-- use the (guild, account) composite above (account is not a prefix field), so
-- it planned as a guild_member TableScan (EXPLAIN, SurrealDB 3.1.3). Plain
-- non-unique index; DEFINE INDEX builds over existing rows at apply time, so
-- there is no backfill hazard (guarded by tests/schema_apply.rs).
DEFINE INDEX IF NOT EXISTS guild_member_account ON guild_member FIELDS account;

-- Per-channel worn persona (supersedes per-guild guild_member.active_persona).
-- One row per (account, channel); UPSERT on wear, DELETE on unwear. Brand-new
-- table → no NONE-backfill hazard.
DEFINE TABLE IF NOT EXISTS channel_active_persona SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS account ON channel_active_persona TYPE record<account>;
DEFINE FIELD IF NOT EXISTS channel ON channel_active_persona TYPE record<channel>;
DEFINE FIELD IF NOT EXISTS persona ON channel_active_persona TYPE record<persona>;
DEFINE INDEX IF NOT EXISTS channel_active_persona_pair ON channel_active_persona FIELDS account, channel UNIQUE;

-- DM membership (M7/P1): one row per (account, dm-thread). A DM thread is a
-- channel with kind='dm' and guild=NONE; this is the dm-kind analog of
-- guild_member that `access::resolve_membership` / `access::visible_channels`
-- branch to on kind. Brand-new table → no NONE-backfill hazard. No active_persona
-- here: DM persona wear rides channel_active_persona like every channel. The
-- optional group title lives in channel.name.
DEFINE TABLE IF NOT EXISTS dm_member SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS channel   ON dm_member TYPE record<channel>;
DEFINE FIELD IF NOT EXISTS account   ON dm_member TYPE record<account>;
DEFINE FIELD IF NOT EXISTS joined_at ON dm_member TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS dm_member_pair ON dm_member FIELDS channel, account UNIQUE;
-- Account-only lookup (mirror guild_member_account, review M-37): visible_channels
-- asks "which DM threads is this account in?" on every /events connect + /unread.
-- DEFINE INDEX builds over existing rows at apply time — no backfill hazard
-- (guarded by tests/schema_apply.rs).
DEFINE INDEX IF NOT EXISTS dm_member_account ON dm_member FIELDS account;

-- DM 1:1 dedup lock (M7/P1, review H1): one row per LIVE 1:1 DM, keyed by the
-- sorted account-id pair so concurrent "message X" double-taps (or both parties
-- initiating at once) converge on ONE thread. A check-then-create read can't do
-- this — the two racing creates touch disjoint records, so MVCC never arbitrates
-- without a shared UNIQUE key. Groups never get a row (a pair key is meaningless
-- for 3+), so this table only ever holds distinct non-NONE keys → the UNIQUE
-- index never sees a NONE collision. THAT is why this is a side table, not a
-- channel.pair_key column: a UNIQUE index on such a column would reject the many
-- guild/group/soft-deleted channels sharing pair_key=NONE and crash schema-apply
-- on the populated channel table. Lifecycle: created inside the 1:1 thread's
-- create transaction; deleted when the thread is soft-deleted (leave_dm last
-- member) or hard-purged — so a dm_pair row exists iff a live 1:1 thread does.
DEFINE TABLE IF NOT EXISTS dm_pair SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS pair_key ON dm_pair TYPE string;
DEFINE FIELD IF NOT EXISTS channel  ON dm_pair TYPE record<channel>;
DEFINE INDEX IF NOT EXISTS dm_pair_key ON dm_pair FIELDS pair_key UNIQUE;

-- Guest cameos (M7/P2): the THIRD channel-membership model. A row grants an
-- accepted friend scoped, ephemeral read+post access to ONE guild text channel
-- (spec §9.7 "Guest Cameos") — the guest never gets a guild_member row, so every
-- guild-management gate (require_manager/caller_role read guild_member only)
-- auto-404s them, and visible_channels surfaces only this one channel. This is
-- the kind='text' analog of dm_member that access::resolve_membership /
-- access::channel_access / access::visible_channels branch to alongside
-- guild_member. Brand-new table → no NONE-backfill hazard.
-- Ephemerality (owner ruling, 2026-06-19): manual revoke/leave DELETEs the row;
-- `expires_at` (NONE = no expiry) is enforced as a LAZY-CHECK predicate
-- (`expires_at = NONE OR expires_at > time::now()`) at every membership query —
-- an expired row resolves to non-member without a sweep. `invited_by` keys both
-- the revoke-authz (inviter may revoke) and the unfriend-revoke hook in friends.
DEFINE TABLE IF NOT EXISTS channel_guest SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS channel    ON channel_guest TYPE record<channel>;
DEFINE FIELD IF NOT EXISTS account    ON channel_guest TYPE record<account>;
DEFINE FIELD IF NOT EXISTS invited_by ON channel_guest TYPE record<account>;
DEFINE FIELD IF NOT EXISTS created_at ON channel_guest TYPE datetime DEFAULT time::now();
DEFINE FIELD IF NOT EXISTS expires_at ON channel_guest TYPE option<datetime>;
DEFINE INDEX IF NOT EXISTS channel_guest_pair    ON channel_guest FIELDS channel, account UNIQUE;
-- Account-only lookup (mirror dm_member_account / guild_member_account, review
-- M-37): visible_channels + /cameos ask "which channels is this account a guest
-- in?" on every /events connect + /unread. DEFINE INDEX builds over existing rows
-- at apply time — no backfill hazard (guarded by tests/schema_apply.rs).
DEFINE INDEX IF NOT EXISTS channel_guest_account ON channel_guest FIELDS account;

-- Messages (channel-scoped, plaintext, persona-aware) ------------------
DEFINE TABLE IF NOT EXISTS message SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS channel ON message TYPE record<channel>;
DEFINE FIELD IF NOT EXISTS author  ON message TYPE record<account>;
DEFINE FIELD IF NOT EXISTS persona ON message TYPE option<record<persona>>;  -- "speaking as" (live link)
-- Persona identity snapshotted at send time. A message is an immutable
-- historical utterance: renaming or deleting the persona must not scramble
-- the name/avatar shown on past messages, so we freeze them here instead of
-- resolving `persona.name` live at render time.
DEFINE FIELD IF NOT EXISTS persona_name        ON message TYPE option<string>;
DEFINE FIELD IF NOT EXISTS persona_description ON message TYPE option<string>;
DEFINE FIELD IF NOT EXISTS persona_color       ON message TYPE option<string>;
-- Avatar snapshot: the media_blob the persona wore at send time. A record link,
-- not a copy — the file isn't duplicated, and changing the persona's avatar
-- later leaves past messages pointing at the image they were sent with. NONE
-- is a valid value (option<record>), so no string-coercion backfill hazard.
DEFINE FIELD IF NOT EXISTS persona_avatar      ON message TYPE option<record<media_blob>>;
DEFINE FIELD IF NOT EXISTS body    ON message TYPE string;                   -- plaintext
-- Inline image attachments: media_blob ids (strings — record links aren't
-- referentially enforced here, and id strings serve directly as /media/{id}).
-- Discord-style: rendered as a compacting grid, click opens a lightbox.
DEFINE FIELD IF NOT EXISTS attachments ON message TYPE array<string> DEFAULT [];
DEFINE FIELD IF NOT EXISTS tier    ON message TYPE string DEFAULT 'default'; -- forward-compat AI-visibility
-- Message kind discriminator. `kind='system'` is authored by the reserved
-- `account:nova_dot` bot (app-admin broadcast) and rendered distinctly (no persona,
-- immutable to edits). `kind='roll'` (M4/T6 Fate Engine) is a server-rolled dice
-- result: authored by the roller, persona-aware, and FULLY immutable (the edit and
-- delete handlers 403 on it — cheating-proof). DEFAULT 'user' keeps every normal
-- send unchanged. This is a non-option field added to the POPULATED `message`
-- table, so legacy rows hold NONE and would crash the row-revalidating backfills
-- below — it is materialised in the SAME first backfill statement (see the
-- DATA-SAFETY block), NOT a separate one.
-- OVERWRITE (the file-wide IF NOT EXISTS exception): the enum was widened after
-- the field shipped, and IF NOT EXISTS never re-applies over an existing field —
-- the old two-value ASSERT would silently survive on prod and reject every roll.
-- Re-defining is idempotent and does NOT re-validate existing rows (only writes
-- validate), so the backfill hazard below is untouched by it.
DEFINE FIELD OVERWRITE kind    ON message TYPE string DEFAULT 'user'
    ASSERT $value IN ['user', 'system', 'roll'];
-- Message effect (M4/T5): an optional roleplay delivery flourish picked at
-- send time — 'whisper' (blurred until tapped), 'shout' (shake + warm tint),
-- 'spell' (glow + sparks) — or NONE for an ordinary message. Purely cosmetic
-- (rendering only; does not gate any action). `option<>` ⇒ NONE is a valid
-- value on legacy rows, so NO backfill is needed and the first-backfill block
-- below stays untouched (the SCHEMAFULL NONE-coercion hazard does not apply).
-- The ASSERT admits NONE explicitly, then restricts set members.
DEFINE FIELD IF NOT EXISTS effect  ON message TYPE option<string>
    ASSERT $value = NONE OR $value IN ['whisper', 'shout', 'spell'];
-- Reply-to (L-3): the message this one is replying to (Discord-style quote),
-- or NONE for a non-reply. A live record link — the reply preview is resolved
-- by a null-safe join at read time (MSG_PROJECTION), NOT snapshotted, and a
-- soft-deleted / since-deleted parent simply joins to NONE. option<> ⇒ NONE is
-- a valid value on old rows, but a field added to a populated table still trips
-- the SCHEMAFULL NONE-coercion crash on the next message UPDATE unless an
-- idempotent backfill materialises it FIRST (see the backfill block below).
DEFINE FIELD IF NOT EXISTS reply_to ON message TYPE option<record<message>>;
-- Ping-mentions (L-4): the set of guild-member accounts `@`-mentioned in `body`,
-- resolved server-side at send time (only members of the channel's guild
-- qualify; case-insensitive). Empty when the message pings nobody. An
-- `array<>` DEFAULT [] field added to a POPULATED table: old rows hold NONE,
-- which the persona-snapshot UPDATE below would re-validate and crash on —
-- exactly the `attachments`/`reply_to` hazard — so an idempotent backfill
-- materialises it FIRST (see the backfill block below). A `record<account>`
-- link is a type annotation only (not referentially enforced); a since-deleted
-- pinged account simply dangles, and the read-path `is_pinged` check tolerates it.
DEFINE FIELD IF NOT EXISTS pinged_users ON message TYPE array<record<account>> DEFAULT [];
-- Guest-cameo badge (M7/P2): true iff the author posted as a GUEST (a channel_guest,
-- not a guild_member) in this channel — snapshotted at send time (spec §10: "guest
-- badges at send-time"), so the badge survives the cameo being revoked/expired and
-- the channel_guest row deleted. A non-option bool added to the POPULATED `message`
-- table → legacy rows hold NONE and would crash the row-revalidating backfill below;
-- it is materialised in the SAME first backfill statement (see DATA-SAFETY), NOT a
-- separate one — exactly the `kind`/`attachments`/`pinged_users` discipline.
DEFINE FIELD IF NOT EXISTS guest_cameo ON message TYPE bool DEFAULT false;
-- Soft-delete (#22): NONE = live (purge window: message 1h). See guild.deleted_at.
DEFINE FIELD IF NOT EXISTS deleted_at ON message TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS sent_at ON message TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS message_channel_sent ON message FIELDS channel, sent_at;

-- One-time backfill: snapshot the persona identity onto messages sent before
-- the snapshot columns existed, so a later persona delete doesn't scramble
-- their name/avatar. Idempotent: only touches persona'd rows still missing the
-- snapshot, and a dangling (already-deleted) persona simply leaves it NONE.
-- Backfill the non-option array fields (`attachments`, `pinged_users`) on every
-- legacy row FIRST, in a SINGLE statement. SCHEMAFULL re-validates the WHOLE row
-- on ANY UPDATE write, so both non-option arrays MUST be materialised together:
-- fixing one in isolation (e.g. `SET attachments`) still re-validates the row
-- and trips over the OTHER still-NONE array (`pinged_users`), aborting schema
-- apply and crash-looping the service. (`reply_to` is `option<>`, so NONE is a
-- valid value — no backfill, left alone.) This MUST precede the persona snapshot
-- UPDATE below, which also re-validates every field.
--
-- DATA-SAFETY (critical): each field is COALESCED (`field ?? <default>`), NEVER a
-- bare assignment. A bare `= []` WIPES an existing array on any matched row — and
-- since every legacy row has `pinged_users = NONE`, the WHERE matches ALL of them,
-- so `attachments = []` destroyed every message's attachments on the first apply
-- over a populated DB (prod gallery images vanished, 2026-06-01). `field ?? []`
-- preserves a populated array and only materialises a NONE to []. Idempotent.
-- `kind` (added later, also non-option) is folded into THIS SAME statement on
-- purpose: it must be materialised together with the arrays, because the first
-- write to any legacy row revalidates the whole row and would trip over whichever
-- of the three is still NONE. A separate preceding `kind` backfill can't work — it
-- would hit the still-NONE arrays. `kind ?? 'user'` only fills a NONE (legacy) row
-- and preserves an already-set 'user'/'system'.
-- `guest_cameo` (M7/P2, non-option bool) folds in for the identical reason: a
-- separate UPDATE would revalidate the row and trip the still-NONE arrays/kind.
-- `?? false` fills only a NONE (legacy) row and preserves an already-set value.
UPDATE message
    SET attachments = attachments ?? [],
        pinged_users = pinged_users ?? [],
        kind = kind ?? 'user',
        guest_cameo = guest_cameo ?? false
    WHERE attachments = NONE OR pinged_users = NONE OR kind = NONE OR guest_cameo = NONE;

UPDATE message SET
    persona_name = persona.name,
    persona_description = persona.description,
    persona_color = persona.color,
    persona_avatar = persona.avatar
    WHERE persona != NONE AND persona_name = NONE;

-- Lorebook entries (SillyTavern-style; on a kind='lorebook' channel) ----
DEFINE TABLE IF NOT EXISTS lorebook_entry SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS channel    ON lorebook_entry TYPE record<channel>;
DEFINE FIELD IF NOT EXISTS title      ON lorebook_entry TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS keys       ON lorebook_entry TYPE array<string>;  -- trigger keywords
DEFINE FIELD IF NOT EXISTS content    ON lorebook_entry TYPE string;         -- injected text
DEFINE FIELD IF NOT EXISTS enabled    ON lorebook_entry TYPE bool DEFAULT true;
DEFINE FIELD IF NOT EXISTS position   ON lorebook_entry TYPE int DEFAULT 0;  -- insertion order
DEFINE FIELD IF NOT EXISTS created_at ON lorebook_entry TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS lorebook_entry_channel ON lorebook_entry FIELDS channel;

-- Friendships (one directed row; reverse-pending request auto-accepts) --
DEFINE TABLE IF NOT EXISTS friendship SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS requester  ON friendship TYPE record<account>;
DEFINE FIELD IF NOT EXISTS addressee  ON friendship TYPE record<account>;
DEFINE FIELD IF NOT EXISTS state      ON friendship TYPE string DEFAULT 'pending' ASSERT $value IN ['pending', 'accepted'];
DEFINE FIELD IF NOT EXISTS created_at ON friendship TYPE datetime DEFAULT time::now();
DEFINE FIELD IF NOT EXISTS updated_at ON friendship TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS friendship_pair ON friendship FIELDS requester, addressee UNIQUE;

-- Feedback / bug reports (#31): user-submitted feedback items ---------------
-- `author`: the account that submitted the report.
-- `kind`: bug | idea | other (coerced server-side; DEFAULT guards old rows).
-- `body`: the user's free-text content (1–4000 chars, validated on write).
-- `context`: optional JSON string from the client (channel id, version, UA).
-- `status`: workflow state (new | read | resolved), advanced by the operator.
-- No NONE-coercion backfill needed: this is a new table with no pre-existing rows.
DEFINE TABLE IF NOT EXISTS feedback SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS author     ON feedback TYPE record<account>;
DEFINE FIELD IF NOT EXISTS kind       ON feedback TYPE string DEFAULT 'other';
DEFINE FIELD IF NOT EXISTS body       ON feedback TYPE string;
DEFINE FIELD IF NOT EXISTS context    ON feedback TYPE option<string>;
DEFINE FIELD IF NOT EXISTS status     ON feedback TYPE string DEFAULT 'new';
DEFINE FIELD IF NOT EXISTS created_at ON feedback TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS feedback_created ON feedback FIELDS created_at;

-- Web Push subscriptions (#30): one row per browser push endpoint ----------
-- The client subscribes via the Push API and POSTs the resulting endpoint +
-- receiver keys; the server encrypts (aes128gcm) + VAPID-signs a payload per
-- row when a new message lands in a guild the owner belongs to.
DEFINE TABLE IF NOT EXISTS push_subscription SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS account    ON push_subscription TYPE record<account>;
DEFINE FIELD IF NOT EXISTS endpoint   ON push_subscription TYPE string;
DEFINE FIELD IF NOT EXISTS p256dh     ON push_subscription TYPE string;  -- receiver ECDH pubkey (base64url)
DEFINE FIELD IF NOT EXISTS `auth`     ON push_subscription TYPE string;  -- receiver auth secret (base64url)
DEFINE FIELD IF NOT EXISTS created_at ON push_subscription TYPE datetime DEFAULT time::now();
-- Endpoint is the natural identity (one per browser). UNIQUE so re-subscribing
-- the same browser upserts rather than duplicating. This is a NEW table — no
-- rows predate these fields, so there is no NONE-coercion backfill hazard (the
-- trap that only applies to fields added to tables with existing rows).
DEFINE INDEX IF NOT EXISTS push_subscription_endpoint ON push_subscription FIELDS endpoint UNIQUE;
DEFINE INDEX IF NOT EXISTS push_subscription_account ON push_subscription FIELDS account;

-- Custom emoji (Discord-style shortcodes, scoped to a guild) ---------------
-- `name` is the shortcode (`:name:`), validated server-side to ^[a-z0-9_]{2,32}$.
-- `media` reuses the existing media_blob pipeline — client uploads via POST /media
-- (gets back a media id), then creates the emoji row referencing that id.
-- New table; no rows predate these fields, so no NONE-coercion backfill needed.
DEFINE TABLE IF NOT EXISTS custom_emoji SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS guild      ON custom_emoji TYPE record<guild>;
DEFINE FIELD IF NOT EXISTS name       ON custom_emoji TYPE string;
DEFINE FIELD IF NOT EXISTS media      ON custom_emoji TYPE record<media_blob>;
DEFINE FIELD IF NOT EXISTS creator    ON custom_emoji TYPE record<account>;
DEFINE FIELD IF NOT EXISTS created_at ON custom_emoji TYPE datetime DEFAULT time::now();
-- Per-guild uniqueness: one shortcode per guild (same name OK in different guilds).
DEFINE INDEX IF NOT EXISTS custom_emoji_guild_name ON custom_emoji FIELDS guild, name UNIQUE;

-- Cross-device read state (L-1) --------------------------------------------
-- Persists each account's per-channel "last seen" high-water mark server-side
-- so a second device knows what was already read (today it's client-only
-- localStorage). One row per (account, channel); the POST /channels/{cid}/mark-read
-- handler UPSERTs it, keeping the MAX cursor (an older POST never regresses a
-- newer mark). `last_seen_at` is the message's `sent_at` and `last_seen_id` its
-- id — the same (sent_at, id) composite the client cursor uses. Brand-new table
-- → no NONE-coercion backfill hazard (the trap only hits fields added to tables
-- with pre-existing rows). The (account, channel) UNIQUE index makes the UPSERT
-- idempotent; a racy double-mark resolves via with_write_conflict_retry.
DEFINE TABLE IF NOT EXISTS channel_read_state SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS account      ON channel_read_state TYPE record<account>;
DEFINE FIELD IF NOT EXISTS channel      ON channel_read_state TYPE record<channel>;
DEFINE FIELD IF NOT EXISTS last_seen_at ON channel_read_state TYPE datetime;
DEFINE FIELD IF NOT EXISTS last_seen_id ON channel_read_state TYPE string;
DEFINE FIELD IF NOT EXISTS updated_at   ON channel_read_state TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS channel_read_state_pair ON channel_read_state FIELDS account, channel UNIQUE;

-- Per-user guild rail order (#17/FB2) --------------------------------------
-- Each row records one account's chosen display position for one guild in
-- their personal server rail. New table; no rows predate these fields, so no
-- NONE-coercion backfill hazard. Guilds with no row sort last (stable key) in
-- `list_guilds`. The PUT /rail/order handler replaces an account's rows wholesale.
DEFINE TABLE IF NOT EXISTS user_guild_order SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS account  ON user_guild_order TYPE record<account>;
DEFINE FIELD IF NOT EXISTS guild    ON user_guild_order TYPE record<guild>;
DEFINE FIELD IF NOT EXISTS position ON user_guild_order TYPE int;
-- One ordering row per (account, guild). UNIQUE so a re-order can upsert cleanly.
DEFINE INDEX IF NOT EXISTS user_guild_order_pair ON user_guild_order FIELDS account, guild UNIQUE;
