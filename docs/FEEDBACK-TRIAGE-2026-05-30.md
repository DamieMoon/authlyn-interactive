# Feedback triage — 2026-05-30

Source: the app's **feedback inbox** = the `feedback` table in SurrealDB **`authlyn/prod`**
(read-only recipe below). Snapshot at triage time: **48 rows — 25 live (`status='new'`),
23 archived (`status='deleted'`)**; 7 filed today by tester `Foxtrot`, 3 still-live bugs filed
2026-05-29 by `damienmoon` (the user/admin).

Re-read the live inbox any time (HTTP `/sql`; the surrealdb MCP WS client can't handshake the
installed 3.0.4 — "Server sent no subprotocol"):

```
curl -sS -X POST http://127.0.0.1:8000/sql -u root:root \
  -H "Accept: application/json" -H "surreal-ns: authlyn" -H "surreal-db: prod" \
  --data "SELECT meta::id(id) AS id, author.username AS author_username, kind, body, context, status, created_at FROM feedback WHERE status!='deleted' ORDER BY created_at DESC;"
```

Effort key: **S** small / **M** medium / **L** large. Triage is grounded in code (file:line
pointers). **No code was changed in this triage session** — it stops at the plan; pick a batch
to start.

---

## Batch 1 — quick wins (S, no/low risk, high confidence)

1. **Feedback inbox refetches after submit** — *one fix closes two reports.*
   `Foxtrot 0sow9yxg…` (today) + `damienmoon ecxro4eq…` (05-29).
   `src/ui/shell/account.rs:41–58,92–105` + `src/ui/shell/act/feedback.rs:17–38`: the modal
   loads the list once on open and never refetches; after a successful POST, re-call
   `list_feedback` and update the `inbox` signal before closing the modal. (App is
   client-polling, not LIVE SELECT — "real-time" here = refetch-after-mutate.) Risk: none.

2. **Batch attachment order preserved** — `damienmoon mnjs2ljw…`.
   `src/ui/shell/act/message.rs:100–112` (`add_compose_attachment` appends in *upload-completion*
   order), `src/ui/shell/state.rs:93–94`, `src/ui/shell/channel/mod.rs:765–797`: tag each
   attachment with its pick index and sort by it before render/send. Risk: none (order is
   cosmetic; `message.attachments` is `array<string>`, sent as-is).

3. **iOS PWA home-screen icon** — `damienmoon adgh3081…`.
   `src/app.rs:27` has a lone `apple-touch-icon` → `icon-192`. iOS ignores the manifest `icons`;
   add a 180×180 (non-transparent) `apple-touch-icon`. Android works because it reads the
   manifest. Risk: none (cosmetic).

4. **Edit field auto-grows + Enter submits** — `Foxtrot tr5mwgzw…` + `7857wrqb…` (today).
   `src/ui/inline_rename.rs:74,85–91`, `src/ui/shell/channel/mod.rs:422–437`: auto-grow the edit
   textarea, and add a `submit_on_enter` prop set true for message edits (the component is shared
   with the main composer, which must keep Enter = newline). Risk: none.
   *Note:* "move the edit box into the composer to reuse the markdown toolbar" is the **L** part
   of `7857wrqb…` — track it separately, not in this quick win.

---

## Batch 2 — correctness + notification polish (M)

5. **Mobile channel-switch shows the previous channel's messages** — `Foxtrot gwiif7xy…` (today).
   *The one true correctness bug in today's batch — it displays wrong data.*
   `src/ui/shell/act/channel.rs:29–96` (`open_channel_at` clears state then kicks the initial
   fetch) + poll loop `src/ui/shell/act/message.rs:800–841`: an in-flight poll for the old
   channel can ingest into the freshly-switched channel before the initial fetch lands. Add a
   switch-generation/epoch guard (or gate the poll until the initial fetch completes). Risk:
   low (display-only, no DB mutation) but high user-visible severity.

6. **Notification "white square" → app icon + sender avatar** — `Foxtrot o44fm3y3…`.
   `src/server/push.rs:331–348` (payload has no `icon`/`badge`/`image`) + `public/sw.js:137–171`:
   include icon/badge and `image` = sender avatar. The avatar is already materialized on the
   message row (`message.persona_avatar`, `schema.surql:146`) — canonicalize the media URL
   (media.rs rules) before embedding. Risk: minor (validate the avatar URL).

7. **Reconcile the two notification paths** — `Foxtrot vkz5t1es…`.
   Web-push (`src/server/push.rs:258–268`) and client-poll (`src/ui/shell/act/notify.rs:356–391`,
   `public/sw.js`) can both fire. Suppress the client-poll notification when a push subscription
   is active (or unify the display options). Risk: none.

8. **Open the PWA to the right channel from a push click** — `Foxtrot br3ebxgj…`.
   `public/sw.js:197–245` closes the notification and either `postMessage({type:"NOTIFICATION_CLICK"})`
   or navigates `/?channel=…&server=…&m=…`, but the app has no listener / query-param deep-link.
   Add a client `message` listener + parse the query params on mount → `open_channel_at`
   (`src/ui/shell/act/channel.rs:29`, already supports an anchor message id). Risk: none.

9. **Dismiss popups when messages are read in-channel** — `Foxtrot 7ty2eyao…`.
   `src/ui/shell/act/notify.rs:289–329` (`clear_notifs_for_channel` fires on channel-open/focus,
   not on read) + `message.rs` ingest: also clear when new messages are ingested while that
   channel is open and focused. Risk: none.

---

## Backlog — needs a decision or is a larger feature

- **Send files through chat** — `Foxtrot 42kyaj3w…` (today). **DECISION (2026-05-30): accept ALL
  file types, served safely.** Today the server is image-only (stored-XSS gate: MIME allowlist +
  `nosniff`, `src/server/media.rs:38–82`) and the client filter only allows image/video
  (`channel/mod.rs:614–620`). Plan: drop the image-only allowlist and widen the client filter to any
  file; **serve non-image blobs as downloads** (`Content-Disposition: attachment` + the existing
  `X-Content-Type-Options: nosniff`, never inline) so an uploaded `text/html` / `image/svg+xml`
  can't execute in the app origin; keep a lightweight **security check** (validate/sniff the declared
  MIME, keep the 64 MiB media body cap). Inline-render only the known-safe image types; everything
  else shows a download chip in the message/gallery. **L + security review.** Pairs with R1 (dedup
  covers all files) and R2 (gallery "media" = images/gifs/videos, now also any file).
- **Image viewer: swipe/arrow within a message + zoom** — `Foxtrot nkqqhkd9…` (today). Idea, M.
- **Ping + per-channel unread counts + jump-to-oldest-unread** — `Foxtrot zytq53qg…`. **L.**
  Partial infra exists (`src/ui/shell/state.rs:133–147`; `refresh_unread`/`last_seen`
  `src/ui/shell/act/message.rs:518–626`) but it's boolean-unread only. Needs counts (HashMap),
  mention/ping tracking, UI badges (orange ping vs white normal), and anchor-to-oldest-unread.
- **Cross-device notification + read/unread sync** — `Foxtrot kuim3tu9…` (today). **L** — read
  state is client localStorage today; cross-device needs a server-side per-account last-seen.
- **Hyperlinks in text & embeds** — `Foxtrot yn53fp2x…`. Markup feature (keep the parser
  panic-free invariant, `src/markup/tree.rs`).
- **Channel/server management window + reordering** — `Foxtrot g4fnv2c1…`.
- **App responsiveness: skeleton/empty previews then hydrate** — `Foxtrot 1kz1ihot…`. Idea, M–L.

---

## Roadmap — larger features requested 2026-05-30 (not from the inbox)

These were raised directly by the user during triage; capturing them so they aren't lost.
Both are **L** and end-to-end (storage + server + UI).

### R1. Media/file deduplication (collapse duplicate blobs to one)
**Goal:** identical uploaded bytes should be stored once; all references point at a single blob.
**Scope (decided 2026-05-30):** covers **all file types**, not just images — the content hash/dedup
is MIME-agnostic and rides on the send-files decision (accept any file).

Today every upload mints a fresh id and writes a new file — `src/server/media.rs:84`
(`random_media_id`), `persist_media_row` (`:143`); `media_blob` (`schema.surql:40–45`) has
`uploader/mime/size_bytes/storage_path/created_at` — **no content hash, no refcount**. So the
same image uploaded twice = two blobs + two on-disk files. References to dedup against:
`message.attachments` (`array<string>`, `schema.surql:151`), `persona_image.media`
(`record<media_blob>`, `:85`), `message.persona_avatar` (`option<record<media_blob>>`, `:146`).

**Recommended design (preserves the unguessable-URL invariant):**
1. **Hash on write.** Compute a content hash (SHA-256) of the bytes in `upload_media` before
   persist. Add `content_hash` to `media_blob` — **must be `option<string>` or carry an
   idempotent backfill** before any other UPDATE (SCHEMAFULL NONE-coercion invariant) — plus an
   index on it.
2. **Dedup the bytes, not the public id.** On upload, look up an existing blob by hash; if found,
   reuse its `storage_path` (so multiple `media_blob` rows can share one on-disk file) while still
   minting a **random public id per reference**. This keeps `media.rs` random-id /
   `starts_with(media_dir)` URL-unguessability intact — content-addressing the *public* id would
   make URLs enumerable (existence leak), so don't.
3. **Refcount or scan-before-delete.** SurrealDB does NOT enforce `record<>` links (cleanup is the
   app's job), so deleting a file is only safe when no `media_blob.storage_path` and no referencing
   row points at it. Either maintain a refcount or have the GC scan the three reference sites above.

**Cadence (you asked — here's the proposal):** do dedup **at upload time** (no dupes ever appear
for new uploads, no sweep needed) **plus** a **one-time backfill** to collapse existing dupes,
**plus** a **weekly orphan-GC** (cron/scheduled task) that deletes blobs/files no longer referenced
anywhere. Weekly is fine for the orphan sweep; the dedup itself shouldn't wait on a sweep.

**Risk:** touches two CLAUDE.md invariants — media random-id/unguessable URL, and "record<> links
aren't referential; cascade is the app's job" — plus the SCHEMAFULL field-add rule. Needs care +
tests, but no auth-model change.

### R2. Gallery channel (media-only browse/upload)
**Goal:** a channel where you can only upload and browse media (images, gifs, videos) — rendered
as a gallery grid, not a chat stream.

**Reality check (verified 2026-05-30):** the only channel type that can be *created* today is
`text` — `prod` has 111 channels, all `kind='text'`, 0 lorebook. The `kind` *plumbing* exists
end-to-end (schema `schema.surql:105` `ASSERT $value IN ['text','lorebook']`; `CreateChannelRequest.kind`
`protocol.rs:161`; server validation `CHANNEL_KINDS=["text","lorebook"]` `channels.rs:21,45`;
`insert_channel(…, kind)` `channels.rs:61`; client `api::create_channel(gid,name,kind)` `api.rs:272`),
and `lorebook` is a *fully-built* second kind (its own `/channels/{cid}/lorebook` routes,
`lorebook_entry` table, `LorebookPane` editor) — **but it has NO create-UI**: `act::create_channel`
hardcodes `"text"` (`src/ui/shell/act/channel.rs:173`) and the create form (`ui/shell/mod.rs:480–486`)
has no kind picker. So lorebook is latent/unreachable, and a gallery kind is NOT free.

Steps to add a gallery kind:
1. **Schema:** widen the ASSERT to include `'gallery'` (`schema.surql:105`). Backward-compatible —
   all existing rows are `text`, so no backfill (widening an ASSERT on an existing field, not adding one).
2. **Server:** add `'gallery'` to `CHANNEL_KINDS` (`channels.rs:21`); in the post path
   (`src/server/messages/posting.rs`) require ≥1 media attachment and reject/ignore text body for
   `kind='gallery'`. All authz invariants stay (guild role + membership + media MIME allowlist).
3. **UI:** ✅ the create-UI gap is **closed (2026-05-30 session)** — the ＋-button channel-creator
   menu (`ui/shell/mod.rs`; `act::create_channel(s, name, kind)`) now lets you pick a kind and
   un-buried lorebook. Adding Gallery = a third radio in that menu + rendering gallery channels as a
   media grid/lightbox instead of the message list (`src/ui/shell/channel/mod.rs`).
   Pairs with **R1** (galleries multiply dupes) and the image-viewer idea (`nkqqhkd9…`).

**Risk:** SCHEMAFULL ASSERT widening (safe here); "media" now includes video/any file per the
**Send-files decision** above. With the create-UI menu already built, the remaining gallery work is
the schema/server `kind` + the grid rendering.

---

## Invariants to respect across all of the above
- Authorization is re-derived per mutate (guild role + channel membership + persona ownership);
  never trust stored state. Unauthorized → privacy-404.
- Media URLs use a server-minted random id + `starts_with(media_dir)` canonicalization + image-only
  MIME allowlist + `nosniff` (stored-XSS gate).
- SCHEMAFULL: a new field on a populated table must be `option<>` or get an idempotent UPDATE
  backfill *before* any other UPDATE, or schema apply crash-loops boot.
- `record<>` links are type annotations only — no referential enforcement; cascade/cleanup is the
  app's job (central to R1).
- Markup parser stays panic-free on arbitrary input (relevant to the hyperlinks idea).

---

## Addendum — 2026-05-31 (backlog)

- **`feedback:qu5ogwxou87v3826wqq3`** — *"em-dashes in client UI"* (kind: bug, 2026-05-31, v2026.5.30, Android Chrome). **Backlogged / deferred.**
  - **Summary:** User reports em-dashes (`—`) appearing in the client UI text. Likely an auto-typography/`--`→`—` conversion (or literal em-dashes in copy/markup rendering) the user finds undesirable.
  - **Suspected area:** markup/typography rendering (`src/markup/`) and/or static UI copy strings. Triage whether the dashes are author-entered, auto-converted on render, or baked into UI labels.
  - **Status:** left `new` in the inbox; recorded here for a later pass. Not blocking.

- **`feedback:kx09a1k1fuh1yz6n1ciw`** — *"Media stored in /data/authlyn/ is accessible by anyone and not even hashed or stored in a database"* — **archived (status=`deleted`)** on 2026-05-31. Triaged as mostly-inaccurate / LOW: media is session-gated, DB-tracked, and served via random 16-byte ids (see security audit + memory `security-findings-backlog`). The only real residual is the deferred per-blob ACL gap, already captured in the security backlog.
