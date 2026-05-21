-- authlyn-interactive — SurrealDB schema
--
-- Loaded on startup by `db::apply_schema`. All DEFINEs use `IF NOT EXISTS`
-- so re-applying on every boot is a no-op.
--
-- Trust model recap (see /Users/damien/.claude/plans/i-turned-on-plan-purring-wozniak.md):
-- the server stores ciphertext + routing metadata only. Olm/Megolm session
-- state lives client-side; the server never holds pickled session secrets.
-- A `tier` column is present on `message` so the AI-visible/private split
-- can be added without a migration.

-- Users ----------------------------------------------------------------
DEFINE TABLE IF NOT EXISTS user SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS display_name ON user TYPE string;
DEFINE FIELD IF NOT EXISTS created_at   ON user TYPE datetime DEFAULT time::now();

-- Devices --------------------------------------------------------------
DEFINE TABLE IF NOT EXISTS device SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS user                ON device TYPE record<user>;
DEFINE FIELD IF NOT EXISTS identity_curve25519 ON device TYPE string;  -- hex
DEFINE FIELD IF NOT EXISTS identity_ed25519    ON device TYPE string;  -- hex
DEFINE FIELD IF NOT EXISTS created_at          ON device TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS device_user ON device FIELDS user;

-- One-time pre-keys ---------------------------------------------------
DEFINE TABLE IF NOT EXISTS prekey_otk SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS device     ON prekey_otk TYPE record<device>;
DEFINE FIELD IF NOT EXISTS kid        ON prekey_otk TYPE string;  -- vodozemac KeyId
DEFINE FIELD IF NOT EXISTS public_key ON prekey_otk TYPE string;  -- hex
DEFINE FIELD IF NOT EXISTS signature  ON prekey_otk TYPE string;  -- Ed25519 sig (hex)
DEFINE INDEX IF NOT EXISTS otk_lookup ON prekey_otk FIELDS device, kid UNIQUE;

-- Fallback pre-key (rotated by client) --------------------------------
DEFINE TABLE IF NOT EXISTS prekey_fallback SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS device     ON prekey_fallback TYPE record<device>;
DEFINE FIELD IF NOT EXISTS kid        ON prekey_fallback TYPE string;
DEFINE FIELD IF NOT EXISTS public_key ON prekey_fallback TYPE string;
DEFINE FIELD IF NOT EXISTS signature  ON prekey_fallback TYPE string;
DEFINE INDEX IF NOT EXISTS fallback_unique ON prekey_fallback FIELDS device UNIQUE;

-- Rooms ----------------------------------------------------------------
DEFINE TABLE IF NOT EXISTS room SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS name       ON room TYPE string;
DEFINE FIELD IF NOT EXISTS created_by ON room TYPE record<user>;
DEFINE FIELD IF NOT EXISTS created_at ON room TYPE datetime DEFAULT time::now();

DEFINE TABLE IF NOT EXISTS room_member SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS room      ON room_member TYPE record<room>;
DEFINE FIELD IF NOT EXISTS user      ON room_member TYPE record<user>;
DEFINE FIELD IF NOT EXISTS joined_at ON room_member TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS room_member_pair ON room_member FIELDS room, user UNIQUE;

DEFINE TABLE IF NOT EXISTS room_event SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS room       ON room_event TYPE record<room>;
DEFINE FIELD IF NOT EXISTS event_type ON room_event TYPE string;  -- 'create' | 'join' | 'leave'
DEFINE FIELD IF NOT EXISTS actor      ON room_event TYPE record<user>;
DEFINE FIELD IF NOT EXISTS target     ON room_event TYPE option<record<user>>;
DEFINE FIELD IF NOT EXISTS at         ON room_event TYPE datetime DEFAULT time::now();

-- In-transit Olm key-share envelopes (Megolm session keys, wrapped in Olm)
DEFINE TABLE IF NOT EXISTS keyshare_envelope SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS recipient_device ON keyshare_envelope TYPE record<device>;
DEFINE FIELD IF NOT EXISTS sender_device    ON keyshare_envelope TYPE record<device>;
DEFINE FIELD IF NOT EXISTS room             ON keyshare_envelope TYPE record<room>;
DEFINE FIELD IF NOT EXISTS olm_message      ON keyshare_envelope TYPE string;  -- base64 OlmMessage
DEFINE FIELD IF NOT EXISTS olm_message_type ON keyshare_envelope TYPE int;     -- 0 PreKey, 1 Message
DEFINE FIELD IF NOT EXISTS created_at       ON keyshare_envelope TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS keyshare_recipient ON keyshare_envelope FIELDS recipient_device;

-- Messages (ciphertext only) ------------------------------------------
DEFINE TABLE IF NOT EXISTS message SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS room              ON message TYPE record<room>;
DEFINE FIELD IF NOT EXISTS sender_device     ON message TYPE record<device>;
DEFINE FIELD IF NOT EXISTS megolm_session_id ON message TYPE string;
DEFINE FIELD IF NOT EXISTS message_index     ON message TYPE int;
DEFINE FIELD IF NOT EXISTS ciphertext        ON message TYPE string;     -- base64
DEFINE FIELD IF NOT EXISTS tier              ON message TYPE string DEFAULT 'default';
DEFINE FIELD IF NOT EXISTS sent_at           ON message TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS message_room_sent ON message FIELDS room, sent_at;

-- Encrypted media blobs (Matrix m.encrypted) --------------------------
DEFINE TABLE IF NOT EXISTS media_blob SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS uploader     ON media_blob TYPE record<user>;
DEFINE FIELD IF NOT EXISTS size_bytes   ON media_blob TYPE int;
DEFINE FIELD IF NOT EXISTS storage_path ON media_blob TYPE string;
DEFINE FIELD IF NOT EXISTS created_at   ON media_blob TYPE datetime DEFAULT time::now();
