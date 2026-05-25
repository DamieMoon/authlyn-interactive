-- authlyn-interactive — SurrealDB schema (Discord-style roleplay platform)
--
-- Loaded on startup by `db::apply_schema`. All DEFINEs use `IF NOT EXISTS`
-- so re-applying on every boot is a no-op.
--
-- Server-trusted model (2026-05-25 pivot): the server stores plaintext
-- content. The prior E2EE device/key/room/keyshare tables were retired.
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

-- Channels (nested under a guild; `kind` gates text vs lorebook) --------
DEFINE TABLE IF NOT EXISTS channel SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS guild      ON channel TYPE record<guild>;
DEFINE FIELD IF NOT EXISTS name       ON channel TYPE string;
DEFINE FIELD IF NOT EXISTS kind       ON channel TYPE string DEFAULT 'text' ASSERT $value IN ['text', 'lorebook'];
DEFINE FIELD IF NOT EXISTS position   ON channel TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS created_at ON channel TYPE datetime DEFAULT time::now();
-- Soft-delete (#22): NONE = live (purge window: channel 1d). See guild.deleted_at.
DEFINE FIELD IF NOT EXISTS deleted_at ON channel TYPE option<datetime>;
DEFINE INDEX IF NOT EXISTS channel_guild ON channel FIELDS guild;

-- Guild membership (the worn persona lives here, per-guild) -------------
DEFINE TABLE IF NOT EXISTS guild_member SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS guild          ON guild_member TYPE record<guild>;
DEFINE FIELD IF NOT EXISTS account        ON guild_member TYPE record<account>;
DEFINE FIELD IF NOT EXISTS role           ON guild_member TYPE string DEFAULT 'member' ASSERT $value IN ['owner', 'admin', 'member'];
DEFINE FIELD IF NOT EXISTS active_persona ON guild_member TYPE option<record<persona>>;
DEFINE FIELD IF NOT EXISTS joined_at      ON guild_member TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS guild_member_pair ON guild_member FIELDS guild, account UNIQUE;

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
-- Soft-delete (#22): NONE = live (purge window: message 1h). See guild.deleted_at.
DEFINE FIELD IF NOT EXISTS deleted_at ON message TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS sent_at ON message TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS message_channel_sent ON message FIELDS channel, sent_at;

-- One-time backfill: snapshot the persona identity onto messages sent before
-- the snapshot columns existed, so a later persona delete doesn't scramble
-- their name/avatar. Idempotent: only touches persona'd rows still missing the
-- snapshot, and a dangling (already-deleted) persona simply leaves it NONE.
-- Backfill `attachments` to [] on every pre-attachment row FIRST. SCHEMAFULL
-- arrays behave like the string fields: old rows hold NONE, and ANY message
-- UPDATE re-validates ALL fields — so this must run before the persona backfill
-- below, or that UPDATE trips over the still-NONE `attachments` and aborts
-- schema apply (crash-loops the service). Idempotent via the WHERE.
UPDATE message SET attachments = [] WHERE attachments = NONE;

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
