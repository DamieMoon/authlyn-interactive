# Pre-merge megaaudit — `main...mendicant-bias` (2026-06-12)

Adversarial multi-agent audit of the Mendicant Bias branch ahead of the merge to `main`
(74 commits, 97 files, audited at HEAD ≈ fe5001a/dc0e162). Method: 10 finder dimensions
+ gap critic (which spawned 3 extra dimensions), dedup, 2+1-vote adversarial verification
per finding, JS synthesis. Workflow run `wf_445eb08c-17d`, 118 agents total.

**Stats:** 45 raw findings → 37 unique after dedup → **52 confirmed**
(24 important, 28 minor), 0 refuted by the verify panel.

Finding ids `M-xx` are the commit-reference ids for the fix swarm (convention: `(review M-xx)`).

**Triage (owner, 2026-06-12): fix ALL 52 before the merge to `main`.** Fixes land as
per-package commits referencing their ids; any finding that turns out to be by-design
is escalated back to the owner instead of silently skipped.

## Important findings (24)

### M-01 — Ghost Quill /typing-drafts is a new body-preview surface that bypasses the whisper hidden-until-tapped invariant

- **Severity:** important  
- **Where:** `src/server/messages/typing.rs` : 135-194 (read), 106-116 (store)  
- **Dimension:** authz

**Evidence.** typing_drafts() returns each stored draft verbatim: `.map(|((_, acct), (text, _))| (acct.clone(), text.clone()))` (L165) → TypingDraftEntry{draft} (L185-189), gated only by channel membership. The draft is stored verbatim/truncated in typing_ping with no effect awareness (L106-108). Meanwhile reading.rs MSG_PROJECTION masks a whispered parent's quote (`IF reply_to.effect = 'whisper' THEN '(whisper)'`) and push.rs notification_body() returns `(whisper)` — but the live-draft path masks nothing.

**Why real.** CLAUDE.md W4 invariant: whisper bodies are masked to `(whisper)` in reply-quote snippets and Web Push, and explicitly says 'extend the mask to any NEW body-preview surface.' Ghost Quill's /typing-drafts is exactly such a new surface. A sender composing a message they will send as `effect='whisper'` (the hidden-until-tapped spoiler) has the plaintext streamed live to opt-in channel members — the very audience the whisper is meant to be hidden from — so the final masked message arrives after the secret already leaked. Mitigations are real (double opt-in; it is the sender's own draft; ephemeral 8s TTL) and the server cannot mask because a draft carries no effect field, so this is a genuine design seam, not a trivial omission. It is unguarded by any test.

**Suggested fix.** Client-side: suppress draft attachment on the typing ping whenever the whisper effect is armed in the composer (the receiver-side fetch then has nothing to show). Or carry the pending effect on TypingPingRequest and have typing_drafts mask drafts whose pending effect is 'whisper'. Add a test pinning that a whisper-armed draft is not served in plaintext.

**Verifier notes.**
- important (unchanged) — correct per the project's documented invariant policy, though reviewers should note the bounded practical impact: leak audience equals the set who could tap-to-reveal the sent whisper anyway, behind double opt-in and an 8s TTL. Dimension is better labeled information-flow than authz.
- minor (down from important) — dimension should be invariant-consistency/UX spoiler-leak, not authz; same-audience cosmetic-effect leak behind double opt-in defaults-off prefs with an 8s ephemeral window

### M-02 — Ghost Quill pings fire during message EDIT — a whispered body surfaces unmasked on the new ghost-row preview surface, and edit save/cancel never clears the stored draft

- **Severity:** important  
- **Where:** `src/ui/shell/channel/mod.rs` : 1433-1446  
- **Dimension:** injection (also reported by: leaks, realtime, tests)

**Evidence.** The composer's on:input handler attaches the compose text to the typing ping with NO edit-mode guard: `let draft = s.prefs.ghost_quill.get_untracked().then(|| value.clone());` (src/ui/shell/channel/mod.rs:1437-1441). The sibling draft-persist function DOES have that guard — `if s.composer.editing.get_untracked().is_some() { return; }` with the comment "the compose box holds the edit text, not a draft" (src/ui/shell/act/channel.rs:61-66) — proving the asymmetry is unintentional. `start_edit` loads the persisted message body into compose (src/ui/shell/act/message.rs:418+). Server-side, `clear_draft` is called only from `post_message` (posting.rs:166-169) and `roll_message` (rolling.rs:254); `edit_message` (src/server/messages/editing.rs:40-85) never clears it, and the client edit-save path (act/message.rs:496-512) sends no clearing ping. Ghost rows render the draft as a bare text node with no whisper veil: `<span class="text">{g.draft}</span>` (channel/mod.rs:976). Receiver chain confirmed live: SSE Typing/MessageCreated → `refresh_ghost_drafts` → `api::get_typing_drafts` → rendered rows (act/sync.rs:495, act/message.rs:1378-1400).

**Why real.** CLAUDE.md W4 invariant (ground truth): whisper bodies are masked to the fixed `(whisper)` placeholder on body-preview surfaces — "extend the mask to any NEW body-preview surface." Ghost Quill rows are exactly such a new surface, added in the same wave. Edit is OFFERED on own whispers (`message_actions` gates on kind only, never effect — channel/mod.rs:129-159), so an author editing their own whisper broadcasts the spoilered body in PLAINTEXT to every opted-in member for the whole edit session, bypassing the tap-to-reveal interaction the feature is built on. Pre-empting refutation: (a) yes, members can tap-reveal anyway, so access control is intact — but the same is true of the reply-quote snippet, which the W4 review nevertheless masked server-side (reading.rs MSG_PROJECTION) and pinned with a test; the invariant protects the presentation mask, not the ACL; (b) yes, the sender opted into Ghost Quill — but that consent covers compose DRAFTS ("rendering the composer draft exactly as it'll appear when sent", channel/mod.rs:924-927), not already-persisted spoiler bodies; an edit will never land as a new message, so even the non-whisper case misrepresents state. (c) The lingering server-side draft after save/cancel (up to the 8s TTL, since nothing clears it) shows a stale ghost beside the just-edited row — the precise artifact clear-on-send/clear-on-roll were added to prevent. tests/typing_drafts.rs covers send (`draft_is_gone_after_the_author_sends_the_message`), roll (`draft_is_gone_after_the_author_rolls`), bare ping, empty draft, TTL, truncation, privacy-404 — the edit path is the one mutation with NO pin, so this is unguarded by tests too.

**Suggested fix.** In the on:input handler, suppress the draft attachment while `s.composer.editing` is Some (mirror save_draft's guard) — send the bare ping instead, which also clears any stored entry server-side. Optionally add `super::typing::clear_draft(...)` to the edit_message success path in editing.rs for parity with posting/rolling, and pin with a `draft_is_gone_after_the_author_edits` test.

**Verifier notes.**
- important (unchanged)

### M-03 — Channel trash panel shows deleted whisper bodies unmasked to any member

- **Severity:** important  
- **Where:** `src/ui/shell/channel/mod.rs` : 242, 251  
- **Dimension:** leaks

**Evidence.** `deleted_message_row` renders `let body_preview: String = m.body.chars().take(120).collect();` ... `<p class="trash-msg-body">{body_preview}</p>` (channel/mod.rs:242,251) — `m.effect` is available on the envelope and ignored; no blur veil, no `(whisper)` placeholder. The server returns the FULL body for trash rows (`load_deleted_messages` uses MSG_PROJECTION, src/server/messages/editing.rs:209-231) and 'Any member may view the trash' (editing.rs:174-175); the topbar 'Show deleted' toggle is un-gated in the UI (src/ui/shell/mod.rs:659-669).

**Why real.** This branch introduced the whisper effect (287717a) and the review commit 4e180a5 extended the mask to push payloads and the attachment veil — but missed the trash body preview, a third body-preview surface. Result: an author who posts a whisper and then deletes it (now a one-tap action via the branch's new undo-toast flow, b9b9e2f, which COMMITS the soft-delete immediately) has their secret displayed in flat plaintext to every channel member for the 1h pre-purge window — strictly MORE exposed than the live message ever was. Pre-empting refutation: 'any member could tap-to-reveal anyway' — the invariant explicitly treats passive display without the deliberate reveal gesture as the leak (that is the entire rationale for masking reply quotes, whose audience is identical); the deleting author additionally signalled intent to retract. No test pins raw whisper bodies in the message-trash listing (tests/soft_delete.rs covers guild trash; tests/messages.rs pins only the reply-quote mask), so a fix breaks nothing.

**Suggested fix.** In `deleted_message_row`, key on `m.effect.as_deref() == Some("whisper")` and render the fixed `(whisper)` placeholder (consistent with reading.rs MSG_PROJECTION and push.rs notification_body) instead of the body slice — or apply the same tap-to-reveal veil as the live row.

**Verifier notes.**
- important (unchanged — violates the documented whisper-mask invariant on a passive-display surface, but introduces no new wire-level exposure beyond what tap-to-reveal already grants members)
- minor (downgrade from important): same-audience, opt-in panel, 1h window, no privilege boundary crossed — but a genuine documented-invariant violation that should be fixed with the (whisper) placeholder or the tap-to-reveal veil

### M-04 — Hidden/background tab under SSE silently marks incoming messages read, wiping cross-device unread

- **Severity:** important  
- **Where:** `src/ui/shell/act/sync.rs` : 473-488 (plus act/message.rs:1010-1028, 1044-1049)  
- **Dimension:** realtime

**Evidence.** dispatch()'s channel-scoped arm: after `message::refresh_open_channel(s).await` it runs `message::set_last_seen(s, &oc, cur)` (sync.rs:483-487) with NO `document_hidden()` gate — `document_hidden()` (sync.rs:426-430) is consulted only by `start_retry` and `wake`, never by `dispatch`. `refresh_open_channel` ingests the new message and advances `s.msg.cursor` (message.rs:927-938), so `set_last_seen` sees an advanced cursor and fire-and-forgets `api::mark_read` (message.rs:1019-1027). The server then emits account-targeted `ReadStateChanged` (read_state.rs:126-131), and every other device's dispatch reacts with `refresh_unread` (sync.rs:454), clearing the badge.

**Why real.** EventSource message events are NOT throttled in hidden tabs (they are network events, unlike the old poll loop's setTimeout, which Chrome throttles to ~1 tick/min after 5 min and freezes entirely in frozen tabs). So a desktop tab left in the background on channel X now marks EVERY incoming X message read server-side within milliseconds, and the user's phone never shows the unread glow or count — for messages no human saw. This is exactly the class the project itself ruled a bug in W3: commit fadc583 fixed the sheet flow because it fired 'the cross-device mark_read for a channel the user never saw', and reentry.rs:9-11 calls that finding 'the standing warning'. The branch's own comment (sync.rs:480-482, 'while the user sits reading') assumes visibility it never checks. Pre-existing poll behavior was the same in principle but throttling made it rare; the branch's SSE driver makes it the common case and the branch wrote this exact set_last_seen call.

**Suggested fix.** Gate the open-channel `set_last_seen` (and `refresh_unread`'s open-channel prelude when invoked from dispatch) on `!document_hidden()`; rely on the existing `wake()` pass to mark read when the tab actually foregrounds.

**Verifier notes.**
- important (unchanged — confirmed appropriate)
- important (confirmed, keep) — but reframe: it amplifies/entrenches an unguarded mark-read already present in main's poll loop (no document_hidden gate there either; wipe landed within ~6s-4min pre-branch) rather than introducing new behavior, and the phone still receives Web Push toasts (push.rs is not read-state-gated), so do not escalate above important.

### M-05 — GET /events stream survives session revocation — logout/password-reset does not kick a live SSE consumer

- **Severity:** important  
- **Where:** `src/server/events.rs` : 53-113  
- **Dimension:** realtime (also reported by: errors)

**Evidence.** `events()` resolves `AuthAccount` once at connect (events.rs:53-56) and stores the bare account key in `Conn` (events.rs:61-66); the unfold loop (69-111) never re-validates the session. `reload_visible` (32-39) queries by ACCOUNT id, not session, so visibility keeps refreshing after revocation. `KeepAlive::default()` (113) holds the connection open indefinitely. Meanwhile `delete_sessions_for_account` exists precisely to lock a holder out: it is called from admin password reset (auth/admin.rs:67) and password change (auth/password.rs:200), and logout deletes the session (auth/registration.rs:144).

**Why real.** Every JSON route re-derives identity per request (the CLAUDE.md invariant: identity ONLY from the session cookie), but the branch's new long-lived surface checks it exactly once. Concrete attack: a stolen-cookie attacker opens /events; the victim (or an admin, via the reset-password lockout flow) revokes all sessions believing the attacker is out; the attacker's stream keeps delivering the account's realtime metadata — channel ids with activity, message ids, typing cadence, account-targeted ReadStateChanged (which channels the victim reads, when) and FriendsChanged — indefinitely. Pre-empting refutation: yes, the bus is id-only so no CONTENT leaks and follow-up fetches 401 — that is why this is important, not critical — but per-account activity/read-pattern metadata flowing post-revocation is a real authorization-lifetime gap, and no test covers stream behavior after session deletion.

**Suggested fix.** Re-validate the session token inside the unfold loop on a cheap cadence (e.g. piggyback on every reload_visible / keep-alive interval, or every N events) and end the stream (`return None`) when it no longer resolves.

**Verifier notes.**
- important (unchanged — correctly calibrated: metadata-only leak, no content, but defeats an explicit attacker-lockout flow)

### M-06 — after_send_success ingests the post-send catch-up page without the open-channel stale-guard — sent message renders in the wrong channel after a fast switch, and SSE removed the poll-era self-heal

- **Severity:** important  
- **Where:** `src/ui/shell/act/message.rs` : 150-181  
- **Dimension:** realtime (also reported by: client_ui)

**Evidence.** `after_send_success(s, cid)` guards the NEW-divider clear on still-being-in-the-channel (message.rs:156-158, added in review fe5001a for exactly this 'slow POST resolving after a switch' race) but then runs `api::list_messages(cid, cur)` and `ingest(s, l.messages)` (171-180) with NO such guard. `ingest` appends into the shared `s.msg.messages` / advances `s.msg.cursor` unconditionally (927-938). Contrast `refresh_open_channel`, which stale-guards both branches (1337, 1354, 'feedback gwiif7xy'). The roll path shares this function (88).

**Why real.** Send in channel A and immediately tap channel B (a routine mobile flow): open_channel_at clears B's list/seen/cursor, then A's catch-up response lands and `ingest` pushes A's rows (at least your own just-sent message) onto the BOTTOM of B's pane and sets `s.msg.cursor` to A's cursor. The user sees their message apparently posted to the wrong channel. Under the old 1.5s poll this self-healed within one tick (full-page `sync_messages` reconcile); under the branch's SSE driver nothing reconciles the open channel until the next event arrives for B — a quiet channel shows the contamination for minutes. The bug body pre-dates the branch, but the branch refactored it into `after_send_success`, added a stale-guard for the divider half of the same race while leaving ingest unguarded, extended it to /roll, and removed the poll cadence that masked it.

**Suggested fix.** Mirror refresh_open_channel's guard: after the await, drop the page (skip ingest/set_last_seen) when `s.sel.sel_channel` no longer equals `cid`.

**Verifier notes.**
- important (unchanged)

### M-07 — SSE visibility REVOCATION (kick/leave/guild-delete) on an already-open connection is untested, and the reload error path fails OPEN for it

- **Severity:** important  
- **Where:** `tests/events.rs` : (absent test); src/server/events.rs:32-39  
- **Dimension:** realtime

**Evidence.** tests/events.rs pins the GRANT direction — `channel_creation_emits_lists_changed_and_membership_set_refreshes` (events.rs test:486-525) proves a live connection's visible-set WIDENS after lists_changed — and pins static non-membership (outsider tests:79-177). No test kicks/leaves a member (membership.rs:225 emits the ListsChanged the kicked connection's narrowing depends on) and then asserts their still-open stream goes silent for that guild's channels. Additionally events.rs:35-37: on DB error during reload the code 'keep[s] the stale set. Fail-closed enough (no new grants leak in)' — which is fail-OPEN for revocation: a kicked member's connection keeps receiving message/typing ids until the next successful lists_changed reload, which may be hours away on a quiet instance.

**Why real.** The per-connection visibility filter is the security spine of the new bus (CLAUDE.md architecture: 'filtered per-connection by access::visible_channels'). Revocation currently works only because remove_member happens to emit a BROADCAST ListsChanged after the DELETE; if anyone later optimizes that emit to targeted (the branch already did exactly this conversion for rail-order in d5c0d33, with a test asserting NON-broadcast) or reorders it, a kicked member silently keeps receiving activity ids for the guild they were removed from — and no test would fail. Per the stated rubric this is 'an invariant left unguarded by tests' on a security-load-bearing path. The stale-set-on-error comment shows the asymmetry was noticed but mischaracterized as fail-closed.

**Suggested fix.** Add the revocation twin of the channel-creation test: open member SSE, kick them, drain the lists_changed, post in the guild, assert Timeout (with an aliveness proof on another visible channel). Consider treating reload_visible errors as fail-closed (empty set) or retrying.

**Verifier notes.**
- important (confirmed as-is; note the live exposure is id-only metadata — content fetches still 404 — so it should not be escalated above important)
- minor (downgrade from important): confirmed test-gap hardening + comment fix, but no current defect, metadata-only blast radius capped by per-request authorization on all fetch surfaces, leak window bounded by connection lifetime, and the error path is not attacker-triggerable

### M-08 — SSE message_edited/message_deleted carry message_id the client never uses — others' edits/deletes/restores never reconcile in channels with >100 live messages

- **Severity:** important  
- **Where:** `src/ui/shell/act/message.rs` : 1326-1366 (refresh_open_channel), sync.rs dispatch 434-511  
- **Dimension:** message_domain

**Evidence.** src/ui/shell/act/sync.rs:434-511 — dispatch() routes MessageEdited/MessageDeleted into the catch-all `_` arm and only calls message::refresh_open_channel; grep for `MessageEdited|MessageDeleted|message_id` across src/ui/shell/act/ returns zero hits, so the message_id payload added by commit 101394d is dead on the client. src/ui/shell/act/message.rs:1333-1363 — refresh_open_channel fetches page 1; only when `l.messages.len() < MESSAGES_PAGE_LIMIT` (100) does sync_messages() do the full reconcile that "reflects edits/deletes"; otherwise it re-fetches with the forward cursor and `ingest()` (927-938) only APPENDS rows strictly newer than the cursor. A row edited, soft-deleted, or restored behind the cursor can never arrive through that fetch. src/server/messages/editing.rs:160-163 claims "A restored message reappears — notify-and-fetch treats it as new arrival", and act/message.rs:614-617 claims restore is "already SSE-notified as a new arrival for every other client" — both false once the channel holds >100 live messages.

**Why real.** Reproducible: channel with 101+ live messages, two live viewers. Viewer A deletes (or edits) one of their messages — instant undo-toast UX on A's side — but viewer B's pane keeps showing the deleted/old row indefinitely (until B switches channels), because B's refresh takes the long-history branch which is append-only. Same for restore: the restored row never reappears for B. The append-only reconcile predates the branch (the old poll had it), but the branch is what added the message_edited/message_deleted bus events (101394d), the restore MessageCreated emit, and the undo-delete feature whose docs assert cross-client delivery — i.e., the branch built and documented a live-sync contract its own fetch path cannot satisfy. Not refutable as "events are id-only by design": notify-and-fetch is fine, but the fetch must be able to express the change; the carried message_id exists precisely to allow a targeted refetch and is unused. No test pins cross-client edit/delete visibility in a >100-message channel (tests/events.rs only asserts event DELIVERY, not reconcile).

**Suggested fix.** On MessageEdited/MessageDeleted for the open channel, use the carried message_id: surgically remove the row (deleted) or refetch that single message (edited) — or fall back to a full page-1 replace instead of cursor-append when the event names a row currently in the list. Correct the two over-claiming doc comments.

**Verifier notes.**
- important (unchanged — confirmed at the claimed severity)

### M-09 — POST /channels/{cid}/messages/{mid}/restore authorization has zero adversarial tests while the branch promotes it to a primary path (undo toast)

- **Severity:** important  
- **Where:** `tests/soft_delete.rs` : 304-410 (only happy path); endpoint src/server/messages/editing.rs:139-171  
- **Dimension:** message_domain

**Evidence.** tests/messages.rs:396-465 pins `other_member_cannot_edit_or_delete` (403) and `nonmember_edit_is_privacy_404` for PATCH and DELETE — but no test anywhere exercises POST .../restore as another member (expect 403), as a non-member (expect privacy-404), or with a cross-channel cid/mid pair. grep "restore" over tests/ shows only the owner happy path in soft_delete.rs:384. The code IS correct today (require_own_message at editing.rs:145 gates channel membership + authorship, and message_author_and_kind scopes `WHERE channel = type::record('channel', $cid)` at editing.rs:297-299), but the gate is enforced only by un-pinned code.

**Why real.** CLAUDE.md invariants (privacy-404, own-message mutation) are explicitly "security-load-bearing" and the rubric counts "an invariant left unguarded by tests" as important. The branch's review commit 1540323 deliberately re-routed Undo onto this endpoint (act/message.rs:623-633), making restore a hot, user-facing mutation — yet unlike its two siblings on the same gate, a refactor of require_own_message or of restore_message's wiring (e.g. someone "simplifying" the pre-UPDATE check away, since the UPDATE itself is unscoped: `UPDATE type::record('message', $mid) SET deleted_at = NONE` at editing.rs:151 touches the row regardless of channel/author) would regress silently. The unscoped UPDATE means the ownership check is the ONLY thing standing between any authenticated user and un-deleting someone else's message.

**Suggested fix.** Add restore arms to other_member_cannot_edit_or_delete (403) and nonmember_edit_is_privacy_404 (404), plus a cross-channel-mid restore (403) — three cheap asserts on existing fixtures.

**Verifier notes.**
- important (confirmed as-is; lower edge of the band — coverage gap on a security gate, not a live defect, with the shared gate partially pinned via sibling endpoints)

### M-10 — W1/evolution-#2 sync driver leaks permanent closures holding Shell with non-try accessors — logout disposes the signals, next event panics (WASM abort); EventSource never closed on logout

- **Severity:** important  
- **Where:** `src/ui/shell/act/sync.rs` : 366, 406-420, 473  
- **Dimension:** client_ui

**Evidence.** install_wake_listeners (sync.rs:406-420) attaches visibilitychange/online closures capturing `s: Shell` and `forget()`s them ('two closures alive for the whole session'). wake() starts with `if !s.sync.polling.get_untracked()` (sync.rs:366) and dispatch() with `s.sel.sel_channel.get_untracked()` (sync.rs:473) — non-try accessors. The SSE on_message handler only self-terminates when the GENERATION moves on (sync.rs:174-178), and logout (act/account.rs:18-25) bumps no generation and closes nothing: it just clears auth.user and navigates, which unmounts Home/AppShell and disposes every Shell signal (they are RwSignal::new'd in AppShell's scope, mod.rs:129-229). The module doc admits the assumption: 'The driver, its closures, and its timers assume the shell mounts once per page load' (sync.rs:20-21).

**Why real.** This repo's own ground truth states non-try accessors panic on disposed signals — act/toast.rs:8-10 ('try_* so a toast outliving the shell (logout mid-toast) degrades to a no-op, never a panic'), message.rs:165-166, radial.rs:66-70, reentry.rs save_mark all use try_* for exactly this. After logout: (a) any tab-switch on the login page fires visibilitychange → wake(s) → get_untracked on a disposed signal → Rust panic in wasm32 = abort, app dead until reload; (b) any server event still flowing on the un-closed EventSource → dispatch(s,…) → same panic — and the stream keeps delivering channel-visibility-filtered events to a client whose session was just revoked (the client never closes CURRENT_ES on logout). Worse, after logout→re-login the OLD listeners fire before the new shell's, so the panic also bricks the fresh session. Pre-empting 'pre-existing': main's eternal poll loop shares the disposed-Shell hazard, but the wake listeners and SSE handlers are NEW branch surfaces (57d2954, 4db38a2/4db34e2), the login-page visibilitychange trigger is new reachability, and the branch documented the single-mount assumption without enforcing it.

**Suggested fix.** On logout: bump the driver generation, close CURRENT_ES, and clear PROBE_PENDING (a `sync::shutdown()` called from act::logout — it already takes Shell). Convert wake/dispatch/poll-loop reads to try_* accessors so any closure that outlives the shell degrades to a no-op per house rule.

**Verifier notes.**
- important (unchanged)

### M-11 — GET /unread full-scans every visible channel TWICE per call — the tie-break OR defeats the (channel, sent_at) index range

- **Severity:** important  
- **Where:** `src/server/messages/unread.rs` : 105-133  
- **Dimension:** perf (also reported by: tests)

**Evidence.** The per-channel unread and ping statements use `WHERE channel = type::record('channel', $cid_i) AND deleted_at = NONE AND (sent_at > $at_i OR (sent_at = $at_i AND meta::id(id) > $mid_i))` (unread.rs:106-118). EXPLAIN FULL on the project's documented reference binary (SurrealDB 3.1.3, dev instance, disposable namespace): this plans as `IndexScan {access: "[channel:big]"}` with NO range bound — on a 2000-message channel the caught-up case (0 unread, the common case) produced `IndexScan rows=2000, 2.6ms` (every row fetched to evaluate deleted_at + the OR). The algebraically identical rewrite `sent_at >= $at AND (sent_at > $at OR meta::id(id) > $mid)` plans as `IndexScan {access: "[channel:big] MoreThanEqual <at>"} rows=0, 11µs` — ~240x. The cursorless Latest probe (unread.rs:126-131) also walks all rows (SortTopKByKey over IndexScan rows=2000). LIMIT only short-circuits when matches exist; a fully-read channel always scans everything.

**Why real.** This is the branch's own new endpoint, and the branch's own client makes it hot: sync.rs dispatch calls refresh_unread on EVERY MessageCreated in a non-open channel, for EVERY connected client (sync.rs:503-508), plus wake, reconnect, and the 6s poll fallback. Total cost per /unread call = 2 × Σ|messages in each visible channel| row fetches. At a realistic 6 channels × 5k messages that is ~60k row fetches per call; one message sent to a 5-viewer instance triggers ~300k row fetches on a Raspberry Pi 4B. The module doc claims 'three DB round-trips… independent of channel count' — true for round-trips, false for work. The W1 work EXPLAIN-verified the analogous shapes in posting.rs:364-367 and reading.rs:369-376 but never this one; tests/unread.rs pins correctness only, not plans. (A > x) OR (A = x AND B > y) ⇔ A >= x AND (A > x OR B > y), so the fix is semantics-preserving.

**Suggested fix.** Rewrite both per-channel predicates to the AND-prefixed form `sent_at >= $at_i AND (sent_at > $at_i OR meta::id(id) > $mid_i)` (verified to plan as a MoreThanEqual index range on 3.1.3); keep the strict tie-break tests green. Consider an EXPLAIN-canary note in the module doc like reading.rs's.

**Verifier notes.**
- important (unchanged — correct as filed)
- Keep "important" for the perf defect itself. But the finding's remediation is inverted in risk: the suggested rewrite is a correctness regression (wrong unread counts at scale, phantom unreads, invisible to the existing test suite) — strictly worse than the perf bug it fixes. Treat the deliverable as: confirmed perf finding + new blocker on the proposed fix path (SurrealDB 3.1.3 composite-index range scans return incorrect row sets at scale).

### M-12 — list_messages cursor catch-up has the same un-narrowed OR — O(|channel|) row fetches per call, and W1 turned its call rate event-driven

- **Severity:** important  
- **Where:** `src/server/messages/reading.rs` : 431-455  
- **Dimension:** perf

**Evidence.** CursorState::Both (reading.rs:431-441) and Before (443-454) use `(sent_at > type::datetime($since) OR (sent_at = ... AND meta::id(id) > $after_id))`. EXPLAIN FULL (SurrealDB 3.1.3): `IndexScan {access: "[channel:big]"} rows=2000` — the whole channel is index-walked and row-fetched per catch-up call even when ~0 rows match. The cursorless newest-page branch (421-430) likewise feeds all 2000 rows through SortTopKByKey (ORDER BY the computed alias `id_key` prevents index-ordered early exit). Pre-existing shape on main, but the branch changed who calls it: sync.rs dispatch (468-502) runs refresh_open_channel per channel-scoped SSE event per viewer, replacing the fixed 0.67 Hz poll.

**Why real.** Pre-existing bug newly exposed/multiplied by the branch (in scope per the audit brief). Under the old poll, cost was bounded at 0.67 Hz per viewer. Under W1 SSE, every MessageCreated AND every Typing ping (2s cadence per typist, channel/mod.rs:1434) fans out to every open-channel viewer, each running up to two of these full-channel-scan queries (see next finding). 2 typists + 3 viewers in a 5k-message channel ≈ 2 events/2s × 3 viewers × 2 queries × 5k row fetches = 60k row fetches/s sustained on the Pi while people type. The same one-line predicate rewrite as the /unread finding restores the MoreThan range (empirically verified) with identical strict-tie-break semantics.

**Suggested fix.** Same algebraic rewrite in both cursor arms: `sent_at >= type::datetime($since) AND (sent_at > type::datetime($since) OR meta::id(id) > $after_id)` (mirror with <=/< for Before). The newest-page SortTopK scan is a separate, smaller win (ORDER BY raw sent_at + id tie-break in Rust).

**Verifier notes.**
- important (unchanged) — the headline rows/s estimate is ~2x overstated (formula yields ~30k/s, not 60k), but O(|channel|) row fetches per event per viewer on the Pi prod host still warrants 'important'
- important (unchanged) — but the evidence text should correct 60k→30k row fetches/s and acknowledge the old poll already paid ~20k/s sustained; the branch's real delta is unbounded activity-scaling, not a flat-rate increase

### M-13 — SSE dispatch treats Typing like MessageCreated: full open-channel refresh (100-envelope length-probe + cursored fetch + drafts) per typing ping per viewer

- **Severity:** important  
- **Where:** `src/ui/shell/act/sync.rs` : 460-502  
- **Dimension:** perf

**Evidence.** dispatch's `_` arm (sync.rs:473-502) runs `message::refresh_open_channel` for ANY channel-scoped event matching the open channel — including `SyncEvent::Typing`, emitted on every ~2s typing ping (server typing.rs:118). refresh_open_channel (message.rs:1326-1366) ALWAYS fetches the full newest page first (`api::list_messages(&ch.id, None)`, message.rs:1333) purely as a length probe, then for >100-message channels issues a SECOND cursored list_messages (1349-1350); then refresh_ghost_drafts adds a third request (sync.rs:495). A Typing event cannot carry new messages — only the typing-names line and ghost drafts can have changed.

**Why real.** Per typing ping per viewer this serializes, downloads, JSON-parses and discards up to 100 full MessageEnvelopes (MSG_PROJECTION with per-row attachment_mimes subquery + reply_to derefs) on the floor device (POCO C3), and costs the Pi ~8-10 DB statements across 3 HTTP requests — multiplied by T typists × V viewers × 0.5 Hz. With ≥2 typists this exceeds the legacy 1.5s poll the SSE migration was meant to beat, and it compounds with the un-narrowed cursor scans (previous finding). The W1.5 review already killed exactly this storm for NON-open channels (sync.rs:503-507 comment: 'kills the 2s-cadence /unread storm one busy typist used to inflict') — the open-channel arm kept it. No test pins this fetch pattern.

**Suggested fix.** Branch on the event: for `Typing` in the open channel, fetch only typing-drafts (it already carries names) or a single CURSORED list_messages (which returns `typing` too) — never the uncursored length-probe page. Reserve the full reconcile for MessageCreated/Edited/Deleted; cache the 'channel fits in one page' fact instead of re-probing it per event.

**Verifier notes.**
- important (confirmed as claimed)

### M-14 — SSE visibility revocation mid-stream is completely untested — only the GROW direction is pinned

- **Severity:** important  
- **Where:** `tests/events.rs` : 486-525  
- **Dimension:** tests

**Evidence.** The only visibility-shift test is channel_creation_emits_lists_changed_and_membership_set_refreshes (tests/events.rs:486-525), which proves the set GROWS. No test in the suite opens an SSE stream and then kicks the subscriber (DELETE /guilds/{id}/members/{aid}), soft-deletes the guild, or soft-deletes the channel, then asserts subsequent message/typing events go Timeout-silent. The mechanism exists (src/server/guilds/membership.rs:224-227 emits an untargeted ListsChanged; src/server/events.rs:95-100 reloads conn.visible on channel_id()==None events) but is unguarded. grep for "/members/" in tests/events.rs returns nothing.

**Why real.** Revocation is the SECURITY direction of the CLAUDE.md invariant 'events filtered per-connection by access::visible_channels'. A very plausible regression exists on this exact branch: W1.5 converted rail-reorder and read-state to the TARGETED emit_for lane to stop broadcast amplification (events.rs:77-91); applying the same 'optimization' to remove_member (targeting the kicker) would leave the kicked member's open connection with a stale visible set forever — they keep receiving message_created/message_edited/typing ids for the guild they were removed from — and the entire 293-test suite stays green. Note also events.rs:35-38 documents reload failure as 'fail-closed enough', which is fail-OPEN for revocation, so nothing else backstops it.

**Suggested fix.** Add to tests/events.rs: kicked_member_stops_receiving_channel_events_mid_stream (owner invites Alice, Alice opens /events, proves delivery, owner kicks Alice, Alice drains the lists_changed frame, owner posts again → assert SseRead::Timeout, then aliveness-proof on Alice's own guild) and guild_soft_delete_silences_open_member_streams (same shape via DELETE /guilds/{id}).

**Verifier notes.**
- important (unchanged)

### M-15 — Ghost Quill cross-channel scoping filter is untested — one dropped clause leaks every draft platform-wide

- **Severity:** important  
- **Where:** `src/server/messages/typing.rs` : 164  
- **Dimension:** tests

**Evidence.** typing_drafts collects entries with `.filter(|((chan, acct), _)| *chan == cid && *acct != account.0)` (src/server/messages/typing.rs:164). The drafts map is process-global keyed (channel_id, account_id) (src/server/state.rs TypingDraftMap). Every test in tests/typing_drafts.rs operates on a SINGLE channel (owner_and_member returns one cid); the only negative test (typing_drafts_returns_privacy_404_with_identical_body_for_non_members, lines 122-166) uses a caller with no membership anywhere, which 404s at channel_access BEFORE the map is read.

**Why real.** The handler's own doc comment (typing.rs:130) says 'this fetch is the ONLY way draft text leaves the server, so the permission check here carries the whole design' — but the permission check only gates THE REQUESTED cid; the `*chan == cid` filter is what stops a member of guild B from reading guild A's live drafts through their own channel's endpoint. Mutate/delete that one clause and the full suite passes while every draft on the instance (raw pre-send text, the most sensitive data the feature handles) is served to any authenticated member of any channel. The own-draft exclusion (`*acct != account.0`) IS pinned; the channel clause is not — asymmetric mutation coverage on one security-load-bearing line.

**Suggested fix.** Add to tests/typing_drafts.rs: draft_in_one_channel_never_appears_in_another_channels_fetch — Alice drafts in guild-1's channel; Bob, a member of an unrelated guild-2 (and NOT of guild-1), GETs /channels/{guild2_cid}/typing-drafts and must see []. A same-guild second-channel variant pins the in-guild scoping too.

**Verifier notes.**
- none — important is correct

### M-16 — /unread soft-delete exclusions are entirely unpinned — deleted channels, deleted guilds, and deleted messages

- **Severity:** important  
- **Where:** `tests/unread.rs` : 1-253  
- **Dimension:** tests

**Evidence.** Three load-bearing filters have zero coverage: (1) visible_channels' `deleted_at = NONE ... AND guild.deleted_at = NONE` (src/server/access.rs:122-123) — feeds BOTH /unread row seeding (unread.rs:54) and SSE filtering; (2) the per-channel `deleted_at = NONE` in the unread and ping statements (unread.rs:108,114). tests/unread.rs never deletes anything (grep 'deleted' → only the module doc); tests/soft_delete.rs never touches /unread or open_sse (grep returns nothing); tests/events.rs never deletes a channel/guild.

**Why real.** CLAUDE.md invariant: 'Soft-delete hides on read'. /unread is a NEW read surface from this branch, and the undo-toast feature (b9b9e2f/1540323) made message delete/restore a hot everyday path — yet a mutation dropping `deleted_at = NONE` from the unread statement (deleted messages keep inflating badges until purge, then counts mysteriously drop) or from visible_channels (trashed channels resurrect as badge rows in every client AND start receiving SSE events again) passes the full suite. The existing privacy test (unread_lists_only_channels_the_caller_can_see, tests/unread.rs:239-252) only covers non-membership, not liveness.

**Suggested fix.** Add to tests/unread.rs: soft_deleted_message_stops_counting_toward_unread (cursor at m1, post m2, unread==1, DELETE m2 → unread==0, restore → 1 again — also pins the undo-toast round trip) and soft_deleted_channel_disappears_from_unread (delete the channel via /guilds/{gid}/channels/{cid}, assert its row is gone); plus a guild-soft-delete variant.

### M-17 — SW catch-all respondWith() intercepts the /events EventSource stream — zero benefit, engine-dependent severance/keepalive risk on the binding iOS-PWA gate

- **Severity:** important  
- **Where:** `public/sw.js` : 79-81, 126  
- **Dimension:** gap:pwa_service_worker_interplay

**Evidence.** sw.js:126 `event.respondWith(networkOnly(request))` is the catch-all for every same-origin GET; networkOnly (sw.js:79-81) is `fetch(request, {cache:"no-store"})`. The branch's new sync driver opens `web_sys::EventSource::new("/events")` (src/ui/shell/act/sync.rs:148), so the infinite-lifetime SSE stream is now piped through the service worker on every controlled page. The server already stamps the response `Cache-Control: no-store` (src/server/mod.rs:230-242, applied to the whole small_body_routes group incl. /events) and SSE responses are inherently uncacheable — the interception adds literally nothing (no caching, no offline fallback in networkOnly).

**Why real.** The unchanged SW now man-in-the-middles the branch's core new transport. Interposing a SW on a long-lived streamed response makes the stream's survival depend on SW-lifetime semantics: engines either hold the SW alive for the duration (battery/memory cost — iOS PWA SW lifetimes are the most aggressive of any engine) or terminate the idle SW, and engines without pass-through pipe-splicing sever the body mid-flight with no error semantics the page can distinguish from a network drop. WebKit's SW+streaming-response behavior is exactly the class of quirk the owner's binding real-device-iOS gate (docs/specs 3af8882, mobile-first memory) exists to catch, and headless testing missed every prior iOS bug. The standard hardening (Workbox guidance, SSE-library docs) is to exempt `Accept: text/event-stream` from respondWith. Pre-empting refutation: yes, the client recovers — a severed stream fires onerror, EventSource auto-reconnects, the fetch event re-wakes the SW, and onopen resets the 5-strike counter so it does NOT demote — but each severance cycle is a silent event-loss window (see the companion finding: promoted reopen never resyncs), plus ●LIVE chip flapping, for zero upside. Risk>0, benefit=0 is a defect regardless of which engine you test on.

**Suggested fix.** Early-return in the fetch handler before any respondWith — `if ((request.headers.get("accept") || "").includes("text/event-stream")) return;` (or `url.pathname === "/events"`) — letting the browser drive the SSE connection natively, exactly like cross-origin requests already do.

**Verifier notes.**
- important (sustained; would drop to minor if the companion promoted-reconnect-resync fix lands, since the remaining impact would be SW-lifetime battery cost and LIVE-chip flapping)

### M-18 — Promoted SSE stream's auto-reconnect never resyncs — events emitted during the disconnect gap are silently lost until unrelated activity

- **Severity:** important  
- **Where:** `src/ui/shell/act/sync.rs` : 242-286  
- **Dimension:** gap:pwa_service_worker_interplay

**Evidence.** In `on_open` (sync.rs:242-286) the truth resync — `refresh_lists` + `refresh_unread` + `refresh_open_channel` (lines 270-279) — runs ONLY inside `if !promoted.get()` (probe promotion). For the already-promoted driver, a successful reopen after a disconnect does only `errors.set(0); s.sync.sse_live.set(true)` (280-283). The justifying comment at 272-274 ('events from the polling era never rode this stream — refetch the batched state once') applies verbatim to a reconnect: events from the DISCONNECTED era never rode this stream either. The server bus has no replay — `GET /events` subscribes to a live tokio broadcast with no history and ignores Last-Event-ID (src/server/events.rs:60, frames carry no id, sse_frame:42-47). The pre-W1 poll loop had no such gap because it refetched continuously (message.rs:1413-1436, open channel every 1.5s, lists+unread every 6s).

**Why real.** Concrete scenario on a visible desktop tab (wake() fires only on visibilitychange/online, sync.rs:365-396, so it never helps a tab that stayed visible): a short network blip or deploy-restart drops the stream for 1-2 EventSource retries (1-4 onerror, below MAX_CONSECUTIVE_SSE_ERRORS=5 so no demote→poll→resync), a member posts a message in another channel during the gap, the browser reconnects → no unread badge, no list update, no open-channel reconcile — and on this 4-user instance the next healing event in a quiet channel can be hours away. Pre-empting refutation: (a) a ≥5-error outage DOES demote and the poll loop resyncs — but that only covers long outages, the short-blip path is the hole; (b) the next event for the open channel does heal the pane via refresh_open_channel — but only if/when one arrives, and unread/lists for OTHER channels heal only on the next non-typing event. This is an asymmetry between two parallel paths (probe-open vs reconnect-open) of the same handler, the exact class the prior reviews missed. SW-stream severance (companion finding) multiplies the frequency of these gaps. Hydrate code has zero tests (no wasm-bindgen-test anywhere), so nothing pins either behavior.

**Suggested fix.** In on_open, trigger the same truth resync whenever the open follows ≥1 error on this stream (e.g. capture `errors.get() > 0` before the reset and reuse the lines 274-278 block, optionally behind the existing WAKE_REFRESH_THROTTLE_MS throttle).

**Verifier notes.**
- important (confirmed as-is; lower edge of the band — narrow trigger window, but silent open-pane message loss under a ● LIVE chip with unbounded staleness duration justifies keeping it above moderate)

### M-19 — Webfonts fall into the SW's networkOnly no-store catch-all — re-downloaded with FOUT on every PWA launch; the branch's own SCSS comment claims caching that the SW actively prevents

- **Severity:** important  
- **Where:** `public/sw.js` : 79-81, 126  
- **Dimension:** gap:pwa_service_worker_interplay

**Evidence.** /fonts/*.woff2 requests match no special branch in sw.js (not /pkg/, not /media/, not in PRECACHE, not mode==='navigate') and hit the catch-all networkOnly → `fetch(request, {cache:"no-store"})` (sw.js:79-81,126) — request cache-mode no-store bypasses HTTP-cache READ and WRITE regardless of response headers. The static fallback (leptos_axum file_and_error_handler, src/main.rs:78) sets no Cache-Control anyway (grep: only src/server/media.rs and src/server/mod.rs set Cache-Control anywhere). Yet the branch-rewritten header in style/_typography.scss:9-10 (commit f1366a5) claims the fonts are '≈64 KB total, gzip-immutable cached', and all four faces use font-display:swap (lines 16-46). Files: space-grotesk 13,388+13,284 B (branch-added), crimson-pro 18,336+18,776 B = ~63.8 KB.

**Why real.** In the installed PWA (every page is SW-controlled after first load), all four woff2 files are refetched from the network on every cold open, and font-display:swap guarantees a visible re-render flash of the ENTIRE UI — the branch makes Space Grotesk the body-default chrome font and Crimson Pro the message-prose font, so the W2 'Duo typography' headline feature flashes fallback-then-swap on every single launch of the mobile-first product, plus ~64 KB of waste per open on mobile data. The catch-all pattern is pre-existing, but the branch touched this exact surface (swapped families, rewrote the comment with a false caching claim) and its perf review reasoned from that false claim ('fonts net smaller' is moot when nothing is ever cached). Pre-empting refutation: the per-session memory cache hides this within one session only — every cold start (the dominant PWA pattern) pays it; and no server header can fix it while the SW forces request-level no-store.

**Suggested fix.** Route `/fonts/` like `/pkg/`: `if (url.pathname.startsWith("/fonts/")) { event.respondWith(networkFirst(request, {cache:true})); return; }` — or add the four files to PRECACHE (they are versioned by CACHE_VERSION and change only on design waves). Also correct the _typography.scss comment.

**Verifier notes.**
- minor (downgrade from important): the no-cache/FOUT behavior is fully pre-existing on main/prod — same sw.js catch-all, four self-hosted woff2 fonts, and a swap-rendered webfont body font all exist at the merge-base; the branch never touched sw.js and actually shrinks per-launch font bytes ~82 KB → ~64 KB. The only branch-introduced defect is the false "gzip-immutable cached" claim in style/_typography.scss (fix the comment pre-merge; SW font caching is a cheap follow-up via PRECACHE/cache-first — note the suggested networkFirst variant would NOT fix the online FOUT).

### M-20 — 'Network-first' /pkg/ fetch uses default cache mode — heuristic HTTP caching can serve a stale WASM bundle against the new SSR shell, defeating the documented anti-staleness design exactly when this mega-release deploys

- **Severity:** important  
- **Where:** `public/sw.js` : 57-64, 94-97  
- **Dimension:** gap:pwa_service_worker_interplay

**Evidence.** sw.js:94-97 routes /pkg/ through networkFirst, whose 'network' leg is plain `fetch(request)` (sw.js:59) — default cache mode, so the browser HTTP cache may satisfy it WITHOUT revalidation. The /pkg/ files are stable-named: Cargo.toml:261-273 documents that `hash-files=true` was evaluated and BACKED OUT, stating 'the PWA staleness fix instead rides on the service-worker update lifecycle … plus the per-build CACHE_VERSION that busts old caches on activate' — but CACHE_VERSION busts only SW Cache Storage; it never touches the HTTP cache. The static fallback (leptos_axum file_and_error_handler, main.rs:78) sends Last-Modified with no Cache-Control, so heuristic freshness (~10% of Date−Last-Modified) lets the cached bundle be reused silently. The SSR HTML (dynamic, no validators) is always fresh. sw.js:3-5's own header says stale bundle ⇒ 'hydration mismatches and broken app code'.

**Why real.** Pre-existing code, but the branch is the trigger event: a 97-file, +11.5k-line release that rewrites the entire DOM (Void Station reskin, new nav, new bubbles) plus the new SSE driver. Concrete path: prod last deployed ~2 weeks before this merge; a daily user's HTTP cache holds /pkg/*.wasm with Last-Modified two weeks old → heuristic freshness ~1+ day at last reuse; the mega-release deploys; within that window the user's 'network-first' fetch returns the OLD wasm/js from HTTP cache with zero network traffic, against the NEW SSR shell → hydration mismatch / dead app — for exactly the users the user-gated update banner was built to protect, and tapping the banner's Refresh does not bust the HTTP cache either (it only swaps SWs and Cache Storage). Pre-empting refutation: yes, after enough wall-clock time age exceeds heuristic freshness and a 304/200 revalidation heals it — but the failure window is hours-to-days right at release, and the in-repo design comment (Cargo.toml) demonstrably believes the SW lifecycle covers this when it does not.

**Suggested fix.** In networkFirst, fetch with revalidation forced: `fetch(request, {cache:"no-cache"})` (conditional request — still network-first, 304s keep it cheap, offline catch-path unchanged). Apply at least to the /pkg/ branch; navigations are safe already (no validators on SSR responses).

**Verifier notes.**
- important (confirmed as reported — correctness/availability regression window at every release, self-healing, no security impact; pre-existing on main rather than introduced by this branch, which argues against raising it to critical)
- important (unchanged) — uphold, with the caveat that the finder's "hours-to-days" window is the upper bound; at current deploy cadence (main last moved 2 days ago) the realistic window is minutes-to-hours per user, re-arming whenever deploy gaps grow.

### M-21 — Session-gated media accumulates unbounded in SW Cache Storage and survives logout — the branch's lightbox now funnels full originals in, and the already-filed Cache-Control:public fix will NOT cover this store

- **Severity:** important  
- **Where:** `public/sw.js` : 60-63, 101-104  
- **Dimension:** gap:pwa_service_worker_interplay

**Evidence.** sw.js:101-104 caches every successful /media/ GET (`networkFirst(request,{cache:true})`); the put at sw.js:60-63 has no size cap, no eviction, and no .catch (quota-full ⇒ unhandled rejection). Eviction happens ONLY on the next release's activate (sw.js:39-52 deletes other CACHE_VERSIONs) — which is user-gated behind the update banner. Logout clears nothing: act/account.rs:18-22 only calls api::logout(); no `caches` usage exists anywhere in client code (grep). The branch's new lightbox renders FULL originals `<img src="/media/{id}">` / `<video src=...>` (src/ui/shell/channel/lightbox.rs:733,742,813,818 — file did not exist on main), and the branch's media.rs change adds `Cache-Control: public, max-age=31536000, immutable` so each blob is now stored twice (HTTP cache + Cache Storage). Upload cap is 64 MiB (server/mod.rs MEDIA_BODY_LIMIT_BYTES); typical phone photos are 2-8 MB.

**Why real.** Two concrete consequences the already-filed 'Cache-Control: public' finding does not capture: (1) auth residue — `GET /media/{id}` is session-gated server-side, but after logout every viewed avatar/attachment/original remains readable on the device via the Cache API/DevTools AND is actively SERVED by the SW's offline fallback (networkFirst catch → caches.match, sw.js:66-67) to whoever uses the browser next — and fixing the filed finding by flipping the header to `private`/no-store changes nothing here, because `cache:true` in sw.js stores the response copy regardless of its headers; the fix is incomplete without this store. (2) Growth — one active roleplay channel posting images daily puts hundreds of MB into a cache that only empties when the user taps the update banner of a future release; on quota-limited iOS the unhandled put rejection starts firing. Pre-empting refutation: the /media/ caching branch is pre-existing, but the brief's mandate is the NEW consequence — the branch added the originals-fetching lightbox surface, doubled the storage via the immutable header, and made Cache Storage the surviving copy after the filed header fix lands.

**Suggested fix.** Cache thumbnails only (`url.searchParams.has("w")`), add a simple count/size cap with oldest-first eviction, add `.catch(()=>{})` on the put, and purge the media entries on logout (page posts `{type:"CLEAR_MEDIA_CACHE"}`; SW deletes matching entries) — mirroring the existing CLEAR_NOTIFS_TAG message channel (sw.js:197-216).

**Verifier notes.**
- minor (down from important) — pre-existing main behavior mis-attributed as a branch consequence; valid as a hardening note attached to the Cache-Control finding's fix scope

### M-22 — Whisper effect is broken in both directions for AT users: keyboard can never reveal, screen readers hear the 'hidden' body immediately

- **Severity:** important  
- **Where:** `src/ui/shell/channel/mod.rs` : 852-895 (plus style/_content.scss:544-588)  
- **Dimension:** gap:a11y_ux_equality

**Evidence.** The whisper reveal is `<span class="text" title="whispered — tap to reveal" on:click=...>` (mod.rs:854-863) — a plain span, no tabindex, no role, no keydown; the only delegated list handlers are pointer events (radial.rs LongPress). The veil itself is CSS-only: `.msg.effect-whisper .text { filter: blur(4px); user-select: none; }` (_content.scss:544-550). The media veil twin `<div class="atts-veil" on:click=...>` (mod.rs:883-889) is likewise click-only, and its CSS hides media via `filter: blur(6px); pointer-events: none` (_content.scss:575-579) — `pointer-events:none` does not remove a `<video controls>` from the tab order, so a keyboard user can focus and play a still-veiled whisper video but can never reveal it.

**Why real.** Two concrete failures, not style: (a) `click` on a non-focusable span never fires from keyboard, so a keyboard-only user can NEVER reveal a whispered message — the effect-pick button even advertises "blurred until tapped" (mod.rs:93); (b) `filter: blur()` and `user-select:none` are invisible to the accessibility tree, so screen readers read the full body immediately with no indication it was meant hidden — the spoiler contract sighted users get (choose when to see it) does not exist for SR users. The server-side whisper mask invariant (CLAUDE.md: mask every body-preview surface) shows the team treats whisper concealment as load-bearing; this branch-new client surface enforces it with CSS only. Pre-empting refutation: no other code path adds focusability — grep for tabindex in channel/mod.rs returns nothing for `.text`, and the delegated `<ul>` handlers are pointerdown/up/move/cancel only.

**Suggested fix.** Make the veil a real disclosure: render the hidden body inside a `<button class="text" aria-expanded=...>` (or span with tabindex=0 + role=button + Enter/Space keydown), and while hidden keep the actual text out of the a11y tree (e.g. aria-hidden on the body with a visible/announced "whispered — activate to reveal" label), revealing real text on toggle. Apply the same to the atts-veil wrapper.

### M-23 — On coarse-pointer devices ALL message actions (reply/copy/edit/delete) are unreachable without a pointer — hover row display:none, radial long-press-only

- **Severity:** important  
- **Where:** `style/_content.scss` : 182-203 (with src/ui/shell/channel/radial.rs:111-113)  
- **Dimension:** gap:a11y_ux_equality

**Evidence.** `@media (pointer: coarse) { ... .msg-actions { display: none; } }` (_content.scss:200-202) removes the action row from layout AND the tab order on touch-primary devices, while the replacement radial menu arms exclusively from a 450ms `pointerdown` long-press gated by `if !super::is_touch() { return; }` (radial.rs:111-113); `is_touch()` is the same `(pointer: coarse)` match (channel/mod.rs:270-275). There is no keyboard or AT trigger for the radial anywhere (only pointer handlers are bound on the `<ul>`, channel/mod.rs delegation).

**Why real.** Desktop keyboard users are covered by `:focus-within .msg-actions` (_content.scss:167-170), but on an iPad with a hardware keyboard, an Android device with switch access, or any touch-primary device with AT, `(pointer: coarse)` matches and `display:none` excludes the buttons from the accessibility tree — so reply, copy, edit, delete (and the only undoable-delete entry) are pointer-gesture-only. This directly violates the owner's binding UX-equality value ("never auto-gate features by device/input class", memory + spec #54) and the mobile-first ruling. Pre-empting refutation: this is not hypothetical hardware — iPad+keyboard is mainstream, and the branch itself treats coarse-pointer as a first-class tier (W3/W4).

**Suggested fix.** Keep the buttons in the a11y tree on coarse pointers (e.g. visually-hidden-until-focus instead of display:none, reusing the :focus-within reveal), or add a keyboard/AT trigger that opens the radial (e.g. a context-menu key / focusable per-row "actions" affordance).

**Verifier notes.**
- important (unchanged)
- important (unchanged) — correct as filed; not critical because the hard-blocked population is narrow (switch/keyboard-only AT users; SR pass-through long-press technically exists) and the project has a designed W5+ fix catalogued, but not minor because it violates a binding owner design law on the primary platform and covers every message action.

### M-24 — Channel sheet claims role="dialog" but has no focus management, no Escape, no aria-modal, and pointer-only dismissal — asymmetric with the project's own Modal component

- **Severity:** important  
- **Where:** `src/ui/shell/mod.rs` : 768-784  
- **Dimension:** gap:a11y_ux_equality

**Evidence.** `<div class="sheet-backdrop" on:click=...></div><div class="channel-sheet" role="dialog" aria-label="Switch channel">` (mod.rs:769-770). No NodeRef/focus() on open, no keydown handler, no focus trap, no aria-modal, no focus restore; the only dismissal affordances are the backdrop click and picking a channel. Contrast src/ui/modal.rs:84-158, the branch-era shared Modal that implements all of it (initial focus, Esc, Tab wrap, WCAG 2.4.3 focus restore) and is used by every other dialog on this branch.

**Why real.** Announcing `dialog` to AT while focus remains on the trigger behind a scrim is actively disorienting: a screen reader is told a dialog opened but reading position never moves into it, and there is no keyboard close (grep confirms zero Escape handling for sheet_open; the only Escape handlers are modal.rs, radial.rs, lightbox.rs, composer). The sheet renders at ≤768px viewports, which includes narrow desktop windows and iPad+keyboard, both keyboard-reachable via the real `<button>` tabs. The repo's own Modal proves the intended standard — this is invariant erosion at the seam where W3 built a parallel dialog implementation instead of reusing it.

**Suggested fix.** Either render the sheet through the shared Modal behavior (focus-in, Esc-to-close, Tab wrap, focus restore) or replicate it: focus the sheet on open, close on Escape, add aria-modal="true", restore focus to the trigger on close.

**Verifier notes.**
- important (unchanged — correct as filed)
- important (unchanged — correctly tiered)


## Minor findings (28)

### M-25 — Thumbnail responses are sent `immutable` though their bytes change across pipeline versions, defeating the v2 cache-buster for already-cached browsers

- **Severity:** minor  
- **Where:** `src/server/media.rs` : 296-303, 336-341, 252-258  
- **Dimension:** authz

**Evidence.** jpeg_response() stamps `IMMUTABLE_CACHE = "public, max-age=31536000, immutable"` (L296-303, L336-341) on every `?w=N` thumbnail. But the same file documents (L254-258) that the on-disk thumbnail key `{id}.w{w}.v2.jpg` carries a `v2` tag that 'bumps when the thumbnail PIPELINE changes ... so existing soft thumbnails regenerate sharp without a manual cache wipe.' The HTTP URL `/media/{id}?w=N` does NOT encode that version.

**Why real.** `immutable` tells browsers never to revalidate for a year. The v2 disk cache-buster only regenerates the SERVER copy; a browser that already cached `/media/{id}?w=64` keeps the old (soft) thumbnail for up to a year because the URL is unchanged, so the stated benefit ('regenerate sharp without a manual cache wipe') does not hold for clients. The original-blob arm IS genuinely immutable (id→bytes is fixed), but the derived thumbnail arm is not, yet is labeled immutable. Branch-introduced (commit 487f073) and pinned by tests/media.rs:575-585 as intended, so it is a behavior/doc contradiction with a concrete (if low) consequence.

**Suggested fix.** Encode the pipeline version in the thumbnail URL (e.g. `?w=N&v=2`) so a bump produces a new immutable URL, or send thumbnails with a revalidating policy (`max-age` without `immutable`) while keeping the original blob immutable.

**Verifier notes.**
- minor (unchanged — latent, non-security; manifests only at the next thumbnail-pipeline bump as up-to-a-year stale thumbnails for previously-cached browsers)
- minor is correct (low end of minor: latent-only, cosmetic blast radius, zero current-user impact since the v2 bump predates the immutable header; kept above trivial because the same-file doc contradiction plus a test pinning the wrong invariant will mislead the next pipeline bump). Dimension should be cache-correctness, not authz.

### M-26 — post_message validates the reply target (channel-scoped DB lookup) before the membership gate, leaking message/channel existence via 400-vs-404 to non-members

- **Severity:** minor  
- **Where:** `src/server/messages/posting.rs` : 92-124, 305-319  
- **Dimension:** authz

**Evidence.** In the touched post_message handler, reply_target_valid(&state, &cid, rid) runs at L93 — its query (L309-311) checks whether message `rid` lives in channel `cid` and is not soft-deleted — and returns 400 'invalid reply target' on miss (L95). Only afterwards (L104) does channel_access run, collapsing non-member/unknown-channel to 404 (L121-123). So a non-member POSTing a valid body + reply_to_id gets 400 when `rid` is absent from `cid` but 404 when `rid` exists in `cid`.

**Why real.** The privacy-404 invariant (server/access.rs) requires unauthorized access to collapse to an identical 404 that never reveals existence. Here a caller who is NOT a member of the channel's guild can distinguish 'message rid exists in channel cid' (404) from 'it doesn't' (400), an existence oracle gated only by guessing ids. The effect-validation block the branch inserted at L78-87 sits right beside this ordering yet is channel-independent, so the branch refactored this handler without closing the leak. Ordering predates the branch (verified against main: reply check at line 73, channel_access at 84), so it is pre-existing but lives in a handler the branch modified — I flag it rather than claim introduction.

**Suggested fix.** Run channel_access (the membership gate) before reply_target_valid / all_media_exist, so a non-member always gets the 404 before any channel-scoped probe can run.

**Verifier notes.**
- minor (unchanged) — exploitation needs a valid session plus knowledge/guessing of high-entropy record ids and leaks only an existence/liveness bit (realistic attacker: an ex-member confirming a message survives in a channel they lost access to); still a real violation of a security-load-bearing documented invariant.
- Keep "minor", but it sits at the absolute floor of minor and is defensibly "trivial/info". The 400-vs-404 ordering and oracle are real and do breach the documented privacy-404 invariant, so it should not be dismissed. But practical impact is near-zero: both `cid` and `rid` are random high-entropy SurrealDB auto-ids (messages/channels are `CREATE`d without explicit ids → rand()-based record ids), and neither is ever exposed to a non-member (channel_access already collapses unknown/non-member channels to 404, and message listing requires membership). So this is a *relationship* oracle that merely confirms "message rid lives in channel cid and is live" for two ids the caller already possesses — not an enumeration primitive. Realistic abuse is limited to an ex-member with remembered ids probing soft-delete/move state. Suggested fix (run channel_access before reply_target_valid/all_media_exist) is correct and cheap.

### M-27 — Composer reply banner shows a whispered parent's body in plaintext — asymmetric with the server-side `(whisper)` quote mask

- **Severity:** minor  
- **Where:** `src/ui/shell/act/message.rs` : 381-393  
- **Dimension:** injection (also reported by: leaks)

**Evidence.** `start_reply` builds the banner snippet directly from the envelope body with no effect check: `let snippet: String = m.body.chars().take(100).collect();` (act/message.rs:386). The server masks the SAME snippet once persisted: `body_snippet: (IF reply_to.effect = 'whisper' THEN '(whisper)' ELSE string::slice(reply_to.body, 0, 100) END)` in MSG_PROJECTION (src/server/messages/reading.rs:396-401), pinned by `reply_preview_masks_whispered_parent_snippet` (tests/messages.rs:1225). Reply IS offered on whispers — `message_actions` branches on kind only (channel/mod.rs:129-159) — and both the meta-row reply button (meta.rs) and the touch radial route through `start_reply`.

**Why real.** Tapping reply on a still-blurred whisper instantly prints up to 100 chars of the hidden text in the composer banner — the spoiler interaction (tap the .text to reveal) is bypassed by a different button on the same row. It also creates a visible inconsistency: the pre-send banner shows the secret, while the quote that lands after send shows `(whisper)`. Pre-empting refutation: the viewer is a channel member already entitled to reveal, and the banner is local-only — that is why this is minor, not important — but the W4 mask invariant explicitly asks every body-preview surface to use the fixed placeholder, and this is the one preview surface the W4 review's mask sweep (push + quote snippet, commit 4e180a5) missed.

**Suggested fix.** In `start_reply`, when `m.effect == Some("whisper")` set the banner snippet to the fixed "(whisper)" placeholder (matching what the persisted quote will show).

**Verifier notes.**
- minor (unchanged — correct as filed)
- minor (confirmed as claimed — do not raise; it is local-only, member-entitled content, but the documented W4 mask invariant keeps it above cosmetic)

### M-28 — Native client renders whisper bodies fully unveiled — the effect contract silently does not exist there

- **Severity:** minor  
- **Where:** `src/native/ui.rs` : 1177  
- **Dimension:** leaks

**Evidence.** `message_row` renders every non-editing body via `render_body(&m.body)` (native/ui.rs:1177) with no `m.effect` handling anywhere in src/native/ (grep for 'whisper'/'effect' in src/native: only the composer's `effect: None` at api.rs:439). The branch DID update native for roll immutability (778cfdd, ui.rs:1122-1126) and for the new `effect` field on SendMessageRequest, proving native was in scope for W4 — but the whisper veil was never ported.

**Why real.** Parallel-implementation asymmetry: a whisper sent from the web (blur-until-tap on every web surface, masked in push/quotes) displays as ordinary plaintext the moment a native-client member opens the channel — no reveal gesture, no indication it was whispered. Not a server-boundary leak (members receive the body by design; the veil is client-side), and the native client has no CI/tests per CLAUDE.md, hence minor — but it is a concrete, user-visible breach of the effect's contract introduced by this branch, on a client the branch otherwise kept in sync.

**Suggested fix.** In native `message_row`, when `m.effect.as_deref() == Some("whisper")`, render the `(whisper)` placeholder with a tap/click-to-reveal toggle (or at minimum the placeholder), mirroring the web's revealed-set pattern.

### M-29 — Session-gated /media responses are now marked Cache-Control: public — shared caches may serve them with auth bypassed

- **Severity:** minor  
- **Where:** `src/server/media.rs` : 302, 315, 326, 339  
- **Dimension:** leaks (also reported by: errors)

**Evidence.** `const IMMUTABLE_CACHE: &str = "public, max-age=31536000, immutable";` (media.rs:302), stamped on every successful blob, attachment-download, and thumbnail response (media.rs:315,326,339; added by commit 487f073). The same endpoint requires a session (`account: AuthAccount`, media.rs:224) — 'any authenticated account may fetch any blob (phase 1)' (media.rs:226).

**Why real.** `public` is the one directive that explicitly authorizes SHARED caches (reverse proxy/CDN) to store and re-serve a response despite the request being credentialed — so any caching intermediary ever placed in front of fenrir (or a future CDN) silently nullifies the session gate on /media: a cached avatar/attachment is served to fully unauthenticated requesters who know the URL. The 128-bit random id remains a capability, so this is defense-in-depth erosion rather than an open hole — hence minor — but `private, max-age=31536000, immutable` achieves the COMPLETE stated goal (the comment cites killing PWA refetch chatter; browser/SW caches are private caches) with zero downside, making `public` a pure liability. Pre-empting refutation: tests/cache_control.rs:118-126 pins the exact header string, but that test was added in the same commit to characterize the route-group split (no-store vs media), not to adjudicate public-vs-private; updating the pinned string alongside the fix is the intended maintenance of a characterization test.

**Suggested fix.** Change IMMUTABLE_CACHE to "private, max-age=31536000, immutable" and update the pinned string in tests/cache_control.rs::media_route_group_is_not_no_store.

**Verifier notes.**
- minor (unchanged — correct as filed)
- minor is correct (ceiling — info/nit would also be defensible; do not raise)

### M-30 — A probe EventSource killed without a final event wedges PROBE_PENDING forever — SSE resurrection permanently disabled for the session

- **Severity:** minor  
- **Where:** `src/ui/shell/act/sync.rs` : 347-356, 75-81  
- **Dimension:** realtime

**Evidence.** `probe()` sets `PROBE_PENDING = true` (sync.rs:351) and the flag is reset ONLY inside the probe stream's own onerror/onopen handlers (204, 214, 253, 258) or on constructor failure (354). The module's own premise — 'a frozen mobile PWA can kill the connection without ever delivering a final error event' (78-81, 361-363) — is defended for the PROMOTED stream via the `CURRENT_ES` ready_state watchdog in wake() (382-390), but a one-shot probe is never stored in CURRENT_ES and has no watchdog or timeout. If the PWA freezes while a probe is CONNECTING, the flag never clears; every future `probe()` from the backoff task (336-338) and wake (389, 394) no-ops at 348-350.

**Why real.** By the code's own frozen-PWA model (the exact scenario the wake listeners exist for), one freeze during a resurrection window leaves the client on the poll fallback for the rest of the session: the ● LIVE chip never returns, latency degrades to 1.5s polling, and Ghost Quill (deliberately SSE-only, message.rs:1374-1376) silently disappears — on mobile, the project's ruling meta platform. Sync itself survives via polling, hence minor. Pre-empting refutation: browsers do usually fire onerror on a dead connection, but the branch explicitly engineered around the documented case where they don't — and covered only one of the two streams it creates.

**Suggested fix.** Have wake() (or a timeout armed in probe()) clear PROBE_PENDING when no probe outcome arrived within a bound (e.g. 15s), or track the probe ES in a thread_local and check its ready_state like CURRENT_ES.

**Verifier notes.**
- minor (unchanged — sync survives on the 1.5s poll fallback; loss is LIVE-chip/latency and Ghost Quill for the page-load session, recoverable by reload)
- minor (unchanged — confirmed at the claimed severity)

### M-31 — create_guild broadcasts ListsChanged to every connection for an event only the creator can observe

- **Severity:** minor  
- **Where:** `src/server/guilds/mod.rs` : 248-253  
- **Dimension:** realtime

**Evidence.** `create_guild` success path: `state.emit(SyncEvent::ListsChanged)` — the global lane (guilds/mod.rs:251). At creation the caller is the only member, so no other account's lists or visibility can change. Every connected client reacts to a broadcast ListsChanged with a per-connection DB `visible_channels` reload (events.rs:95-99) plus THREE client refetches: lists, unread, and the open channel page (sync.rs:445-451). Contrast the rail-order handler, which the branch converted to `emit_for(vec![account.0], …)` for exactly this reason ('N×M amplification for a change nobody else can even observe', guilds/mod.rs:156-163) with a regression test (tests/events.rs::rail_reorder_no_longer_broadcasts).

**Why real.** Same-class defect as the one the branch already fixed and tested for rail reorder, left unfixed one handler away: every guild creation costs N connections × (1 DB visibility query + 3 HTTP refetches) for zero observable change to anyone but the actor. Concrete (not stylistic) waste, and an invitation for the next per-user mutation to copy the broadcast form. Pre-empting refutation: invite/kick/channel-create correctly broadcast because another party's lists genuinely change; guild CREATE has no other party by construction.

**Suggested fix.** Use `state.emit_for(vec![account.0.clone()], SyncEvent::ListsChanged)` like rail-order; targeted ListsChanged already reloads the recipient's visibility set (events.rs:85-88, review d5c0d33).

### M-32 — Rewritten hot-path MSG_PROJECTION (attachment_mimes correlated subquery) verified only on SurrealDB 3.1.3; prod runs 3.0.4 and no test can ever cover the skew

- **Severity:** minor  
- **Where:** `src/server/messages/reading.rs` : 368-393  
- **Dimension:** schema

**Evidence.** MSG_PROJECTION now embeds `(SELECT meta::id(id) AS id, mime FROM array::map(($parent.attachments ?? []), |$a| type::record('media_blob', $a)) WHERE id IS NOT NONE) AS attachment_mimes` (reading.rs:402-405 in the const, doc comment at 368-375: "Verified via EXPLAIN FULL on the SurrealDB 3.1.3 server binary (the dev binary)"). The replaced post-page batch (`FROM $records` with SDK-bound RecordIds, deleted at the bottom of reading.rs) is the only form that has ever executed on prod's 3.0.4. The whisper-mask inline `IF reply_to.effect = 'whisper' THEN '(whisper)' ELSE string::slice(...) END` (reading.rs:403-404) rides the same never-run-on-3.0.4 projection.

**Why real.** This is a verification gap with direct incident precedent in this exact repo, not style: the widened `message.kind` ASSERT bug was 'found live' on prod precisely because dev-binary tests cannot exercise prod's 3.0.4 (CLAUDE.md gotcha names the 3.1.3/3.0.4/3.1.0-beta.3 skew as 'first suspect'; the retry canary caught the 3.1.3 conflict-text rename the same way). The new arm uses a closure-as-FROM-source + correlated `$parent` shape new to this codebase, evaluated per message row on the single hottest endpoint (GET /channels/{cid}/messages) — if 3.0.4 errors on or mis-evaluates it, every message page read 500s on prod immediately after the merge deploy, and deploy.sh's health check may not catch it. I attempted to confirm against a real 3.0.4 binary (downloaded the official darwin-arm64 release) and was sandbox-blocked from executing it, so neither I nor CI can adjudicate this pre-merge — which is itself the finding. Pre-empting refutation: yes, SurrealQL closures/array::map exist since 2.0 and $parent since 1.x, so the construct PROBABLY parses on 3.0.4 — but 'probably parses' was also true of the kind ASSERT path, and plan/NONE-row semantics (`FROM <computed array>`, dangling-pointer rows) are exactly the kind of cross-version unevenness the codebase has already documented (the old comment cited 3.1.0-beta.3 'unevenness' as the reason to avoid correlated sub-SELECTs).

**Suggested fix.** Before merging (or in the deploy runbook for this release): run the exact MSG_PROJECTION SELECT once against fenrir's 3.0.4 over HTTP /sql in a throwaway namespace with a seeded message+media_blob, confirming attachment_mimes and the whisper-masked reply snippet evaluate; alternatively upgrade prod to the dev-verified 3.1.3 first. Also extend the deploy health check to probe a message-list read, not just liveness.

**Verifier notes.**
- minor (unchanged) — verification/release-engineering gap with low-probability but prod-outage-shaped worst case; should gate the merge as a runbook step, not a code change

### M-33 — is_write_conflict's new "failed transaction" marker matches ANY aborted multi-statement transaction on 3.1.x, re-executing permanently-failing transactions 5x

- **Severity:** minor  
- **Where:** `src/server/retry.rs` : 87-90  
- **Dimension:** schema

**Evidence.** retry.rs:89: `s.contains("write conflict") || s.contains("can be retried") || s.contains("failed transaction")`. On 3.1.x, "The query was not executed due to a failed transaction" is the generic sibling-statement text for ANY transaction abort, not only MVCC conflicts. Consumers wrapping full BEGIN/COMMIT blocks: read_state.rs:89 (mark-read DELETE+CREATE), personas/wear.rs:154, push.rs:196, personas/gallery.rs:307.

**Why real.** A non-conflict failure inside any of those transactions (e.g. a future schema ASSERT rejection, or any statement-level error that aborts the tx) now matches the predicate and re-runs the whole DELETE+CREATE transaction 4 extra times (~50-126 ms, 5x write attempts) before surfacing 500 — and the surfaced error is the last attempt's generic text, masking the root cause in logs. Pre-empting refutation: the F-D6 adjudication + tests/retry_canary.rs pin only the POSITIVE direction (real conflicts match; UNIQUE violations stay disjoint via "already contains") — nothing pins the false-positive class. I verified the consequence is bounded and correct-in-outcome: the DELETE-then-CREATE idiom is idempotent, mark_read's IF-guard converges on retry, all single-statement UNIQUE creates (invite_member at membership.rs:140, registration, friendship, persona_editor, custom_emoji) surface the plain "already contains" text untouched, and is_unique_violation is checked on the residual error after retries — so no 409→500 flip. Prod's 3.0.4 never emits the new text, so this is dev-box-only until prod upgrades. Concrete (duplicate writes + log masking), not stylistic; but bounded.

**Suggested fix.** Optional hardening: log the FIRST error (root cause) alongside the residual when retries exhaust, and/or note the accepted false-positive class in the retry.rs doc comment so a future 3.1.x prod upgrade doesn't surprise anyone with 5x-replayed failing transactions.

**Verifier notes.**
- minor (unchanged) — confirmed; consequences are bounded latency, redundant re-execution of an already-aborted transaction, and root-cause masking in logs; no data corruption, no 409→500 flip, no security impact. The "dev-box-only" framing in the finding should be dropped: the class also applies to prod 3.0.4.
- minor (confirmed as-is; low end of minor — no correctness/security/availability impact, fix is doc + logging hardening)

### M-34 — Native client renders kind='roll' and effect='whisper' messages as plain user text — server-authoritative rolls are visually forgeable and whisper spoiler-guard is absent on native

- **Severity:** minor  
- **Where:** `src/native/ui.rs` : 1103-1160 (message_row)  
- **Dimension:** message_domain

**Evidence.** src/native/ui.rs message_row (1103-1160) renders every message identically (name + time + body); the only kind awareness is the edit/delete button gate at 1129 (`mine && !editing && m.kind == "user"`, review 778cfdd). grep "effect" over src/native/ hits only api.rs:440 (`effect: None` on send). So on native: (a) a real kind='roll' result ("2d20+3 → [14,8]+3 = 25") and a user-typed message with the identical body are pixel-identical — the web distinguishes them via the roll glass chip + 🎲 glyph (ui/shell/channel/mod.rs:840-851), which is the only thing making the Fate Engine's forge-proofing legible to viewers; (b) a whisper body renders fully readable with no blur/reveal (web blurs `.text` until tapped, mod.rs:852-865).

**Why real.** Branch-introduced surface gap: W4 added both kinds/effects this branch, and the W4 review explicitly extended the kind PREDICATE to native (778cfdd) but stopped at affordances — the trust-display half of roll immutability (a viewer must be able to tell a server-rolled result from typed text) and the whisper hidden-until-tapped semantic never reached the parallel native implementation. Concrete consequence: a player on the native client can be shown a fake "natural 20" typed by another user and has no way to distinguish it from a genuine roll; whispered spoilers are exposed on open. The native graph has no tests/CI (CLAUDE.md gotcha), so nothing will catch this. Minor because native is a secondary client and the whisper effect is a presentation spoiler-guard (members can reveal on web anyway), not a secrecy boundary.

**Suggested fix.** In native message_row, branch on m.kind == "roll" to render a distinct prefixed/badged row (even text-only: "🎲 ROLL — ..."), and on effect == Some("whisper") render a tap-to-reveal placeholder mirroring the web's revealed-set pattern.

### M-35 — Lightbox pinch: losing one finger to pointercancel leaves the gesture frozen in Pinch mode with .gesturing (will-change) pinned — asymmetric with the pointerup path that degrades to Pan

- **Severity:** minor  
- **Where:** `src/ui/shell/channel/lightbox.rs` : 675-690  
- **Dimension:** client_ui

**Evidence.** on_pointercancel (lightbox.rs:675-690) removes the cancelled pointer but only resets mode/gesturing when `g.pointers.is_empty()`. A two-finger pinch losing ONE finger to pointercancel (iOS system edge-gesture, palm rejection — the exact cases the comment names) leaves pointers=[one], mode=Pinch, gesturing=true. on_pointermove's Pinch arm requires the two-element slice pattern `[a, b]` (lightbox.rs:497) so every subsequent move of the remaining finger is dropped. Contrast on_pointerup's Pinch arm (lightbox.rs:563-575), which explicitly degrades to Pan with the surviving finger.

**Why real.** User-visible stuck state on the platform the project treats as primary (mobile-first ruling): mid-pinch, an iOS edge swipe cancels one touch and the image stops responding to the remaining finger entirely until it is lifted and re-pressed; meanwhile `.gesturing` stays set, holding `will-change: transform` (a compositor layer the SCSS comment says is deliberately scoped to gestures to protect floor-device memory) and keeping the snap transition disabled. Recovery only via full lift or a fresh second finger, so it is a transient freeze rather than a permanent brick — hence minor, but it is a concrete state-machine hole the 7-auditor W4 audit and the lightbox review missed, and no unit test covers the cancel path (tests cover only the pure transform math).

**Suggested fix.** In on_pointercancel, mirror on_pointerup's Pinch arm: if one pointer remains and mode==Pinch, degrade to Pan (or Idle at fit scale) re-anchoring start_tf/origin on the survivor, and drop `gesturing` only when pointers empty (as now).

**Verifier notes.**
- minor (unchanged — correct as filed)
- minor (unchanged — confirmed at the claimed severity)

### M-36 — Re-entry scroll memory is never captured when leaving a channel via the bottom tabs — Chat→Friends→Chat silently yanks the reader to the tail

- **Severity:** minor  
- **Where:** `src/ui/shell/act/channel.rs` : 37-45, 85-98  
- **Dimension:** client_ui

**Evidence.** capture_scroll_mark's ONLY caller is open_channel_at (channel.rs:98); the tab-bar paths — show_friends (message.rs:722), show_wardrobe, and the return path show_current_channel (channel.rs:37-45, 'WITHOUT reloading anything') — never capture. On return, ChannelPane remounts with fresh component-local state: prev_count=None and last_dist=0.0 (channel/mod.rs:570,588), so the append effect's follow branch (`last_dist <= threshold`, channel/mod.rs:651-661) unconditionally scrolls to the bottom over the retained message list. No mark exists, so take_restore_anchor has nothing to restore.

**Why real.** The shipped feature (419e280, 'per-channel scroll memory') holds for channel-to-channel switches but silently fails on the single most common mobile loop the same branch built (W3 bottom tabs): scroll up mid-history → tap Friends to answer a request → tap Chat → you are dumped at the tail with your reading position lost, while the identical action via the channel sheet preserves it. That asymmetry is branch-introduced (both the tab bar and the scroll memory are this branch) and contradicts the feature's stated intent ('record where the user stands… before any state below is touched'). Minor because it loses a scroll position, not data — but it is exactly the seam between two of this branch's features.

**Suggested fix.** Call act::reentry::capture_scroll_mark(s) at the top of show_friends/show_wardrobe/show_current_channel's sheet branch (any transition that unmounts ChannelPane while sel_channel is set) — the function already no-ops safely when no list is mounted; and/or have show_current_channel route the retained position through take_restore_anchor on remount.

**Verifier notes.**
- minor (as filed) — lost reading position, no data loss; scope is slightly narrower than claimed (Friends/pane-switch paths only, not Personas/Servers tabs which are overlays)
- minor (unchanged) — correct as filed: user-visible UX regression on the primary mobile flow, but no data loss, no read-state corruption, no security relevance

### M-37 — visible_channels' guild_member lookup is a TableScan — the new W1 query shape has no usable index

- **Severity:** minor  
- **Where:** `src/server/access.rs` : 119-120  
- **Dimension:** perf

**Evidence.** `SELECT VALUE guild FROM guild_member WHERE account = type::record('account', $account)` (access.rs:119-120). The only guild_member index is the UNIQUE composite `guild_member_pair ON guild_member FIELDS guild, account` (schema.surql:133); an account-only predicate cannot use a non-prefix composite field. EXPLAIN on SurrealDB 3.1.3 confirms: `TableScan {table: "guild_member", predicate: "account = account:a1"}`. Runs on every /events connect, on every ListsChanged per connection (events.rs:32-39, N×M amplification documented at events.rs:28-31), and inside every GET /unread (unread.rs:54).

**Why real.** guild_member rows = members × guilds, so this scan grows with exactly the dimension /unread's call rate also grows with (more members → more events → more /unread calls × bigger scans). Harmless at today's ~dozens of rows, but it is the one W1 query shape with no index, on the same hot path as the /unread finding, in a codebase whose stated discipline is EXPLAIN-verified point reads (reading.rs:369-376). The late-May prod incident (74 junk accounts × 74 guilds) shows this table can balloon unexpectedly.

**Suggested fix.** `DEFINE INDEX IF NOT EXISTS guild_member_account ON guild_member FIELDS account;` (plain, non-unique). One line in schema.surql; no backfill hazard (indexes rebuild on define).

**Verifier notes.**
- minor (unchanged — confirmed at the claimed severity)

### M-38 — resolve_display_names keeps the documented `meta::id(id) IN $array` TableScan anti-pattern, now on the W4 Ghost Quill hot path

- **Severity:** minor  
- **Where:** `src/server/messages/reading.rs` : 163-181  
- **Dimension:** perf

**Evidence.** `FROM account WHERE meta::id(id) IN $accts` (reading.rs:170) and `FROM channel_active_persona WHERE channel = ... AND meta::id(account) IN $accts` (reading.rs:175-177). The same branch documents this exact shape as a verified TableScan and fixes it with record-pointer reads elsewhere: posting.rs:364-367 ('instead of a full TableScan — which was the actual plan for WHERE meta::id(id) IN $ids, verified via EXPLAIN') and the MSG_PROJECTION attachment_mimes arm (reading.rs:369-376). channel_active_persona's only index is (account, channel) (schema.surql:142), which the channel-first predicate can't use either.

**Why real.** Pre-existing query (main's resolve_typing_names had it verbatim), but the branch added a second, hotter caller: typing_drafts (typing.rs:170) runs it on every Ghost Quill fetch — i.e., per Typing/MessageCreated SSE event per opted-in viewer — and every list_messages with active typists runs it too. Both tables scanned are small today (accounts ≈ users, channel_active_persona ≈ wears), so cost is currently negligible — but it is a known, already-solved-in-this-branch anti-pattern left on a per-keystroke-cadence path, and the prod account table has ballooned before.

**Suggested fix.** Bind the account ids as RecordIds and read `FROM $records WHERE id IS NOT NONE` (the all_media_exist pattern); for channel_active_persona, either add an index on channel or key the lookup by the (account, channel) pairs as record pointers.

**Verifier notes.**
- none — minor is correct

### M-39 — Roll-result chip puts backdrop-filter INSIDE the scrolling message list, violating the branch's own glass-is-for-chrome battery rule

- **Severity:** minor  
- **Where:** `style/_content.scss` : 497-510  
- **Dimension:** perf

**Evidence.** .roll-chip (rendered per kind='roll' message row in the scrolling <ul.messages>) carries `backdrop-filter: blur(8px); -webkit-backdrop-filter: blur(8px)` plus a glow box-shadow (_content.scss:505-510). The branch's own design contract says the opposite: '_foundation.scss:1-3: Glass is for CHROME (topbar, tabbar, sheets, modals) — prose cards stay opaque (--card) for legibility and battery; see spec §1.' Every other backdrop-filter the branch added sits on fixed chrome with a static backdrop; this is the only one on content that MOVES during scroll.

**Why real.** A backdrop-filter region that moves relative to its backdrop cannot be cached — the compositor re-samples and re-blurs it every scroll frame. A dice-heavy RP session puts several roll chips in the viewport at once, each a live blur region during chat scroll on the floor device (POCO C3, weak GPU — the project's binding mobile-first baseline per spec #54). The reduced-motion kill list (_motion.scss:243-261) covers the chip's keyframes but blur is 'state', so it survives there too; there is no .fx-max gate despite the tier system existing for exactly this stratification.

**Suggested fix.** Drop the blur at the standard tier — the `color-mix(in srgb, var(--card) 55%, transparent)` wash over an opaque fallback already reads as a chip (the mixin's own fallback philosophy) — and move `backdrop-filter` behind `.app.fx-max &`.

**Verifier notes.**
- minor (unchanged)

### M-40 — fx-max aurora-drift animates the full-viewport layer beneath the always-blurred glass chrome — continuous re-blur while idle

- **Severity:** minor  
- **Where:** `style/_foundation.scss` : 56-58  
- **Dimension:** perf

**Evidence.** `.app.fx-max::before { animation: aurora-drift 9s ease-in-out infinite alternate; }` (_foundation.scss:56-58) animates transform+opacity of the fixed, full-viewport z-index:-1 layer (42-52). That layer is the literal backdrop of the glass topbar (_content.scss:70), the fixed .bottom-tabs (_nav.scss:82), and any open sheet — all `backdrop-filter: blur(14px) saturate(1.4)` via the mixin (_foundation.scss:19-23).

**Why real.** With a STATIC backdrop the browser can cache the blurred chrome regions, which is why the standard tier's glass is cheap; the infinite drift invalidates that backdrop every frame, forcing blur(14px) re-evaluation for topbar + tab bar at compositor rate, forever, even when the app is fully idle in the foreground. It is opt-in (eyecandy pref, default OFF, prefs.rs:64-68) and killed under prefers-reduced-motion (_motion.scss:244-245), so the blast radius is self-selected — but a phone-resident PWA (the project's primary form factor) opting into 'eye candy' silently buys a permanent GPU/battery drain disproportionate to a 12px drift nobody watches.

**Suggested fix.** Either drift a layer that no glass samples (animate a child of .content below the bars' bounds), lengthen/step the animation (e.g. steps() at low frequency), or document the cost next to the eyecandy toggle and exclude the bars' rect from the aurora.

**Verifier notes.**
- minor (unchanged — opt-in default-off gating and foreground-only scope cap it, but the permanent battery cost on the mobile-first primary form factor keeps it above informational)

### M-41 — Restore endpoint has no negative-path tests despite becoming the undo-toast hot path

- **Severity:** minor  
- **Where:** `src/server/messages/editing.rs` : 140-171  
- **Dimension:** tests

**Evidence.** POST /channels/{cid}/messages/{mid}/restore (editing.rs:140-171) is tested only on the owner happy path (tests/soft_delete.rs:380-411). No test pins: other-author restore → 403, non-member/unknown-channel restore → privacy-404, restore of a purged id → 403, or that restoring an ALREADY-LIVE message is a 204 no-op that emits a spurious MessageCreated (editing.rs:159-164 emits unconditionally). The branch wired Undo onto this endpoint (commit 1540323).

**Why real.** The gate is the shared require_own_message — pinned via edit/delete (tests/messages.rs:421,431) — but restore's CALL into it is the one line a refactor can drop, and no test would notice; the result would be any guild member resurrecting messages another user deliberately deleted (content-integrity regression on the brand-new undo flow). Severity minor because the shared gate's internals are covered; the uncovered surface is the wiring plus the 403-vs-404 matrix specific to this route.

**Suggested fix.** Add to tests/soft_delete.rs (or a new block in tests/messages.rs): restoring_someone_elses_deleted_message_is_403, restore_is_privacy_404_for_non_members_and_unknown_channels (body-identical assertion like typing_drafts'), restoring_a_purged_message_is_403_not_500.

### M-42 — Whisper push mask: only the pure formatter is tested — the effect-column plumbing from the DB row is unpinned

- **Severity:** minor  
- **Where:** `src/server/push.rs` : 294-312, 364  
- **Dimension:** tests

**Evidence.** notification_body's masking logic is unit-tested in-module (push.rs:441-466), but the integration seam — `effect: Option<String>` in the Info row (push.rs:294), the `effect,` projection line in notify_inner's SQL (push.rs:312), and the `info.effect.as_deref()` thread-through (push.rs:364) — has no test. notify_inner is private, fire-and-forget, and requires a live web-push endpoint, so no integration test exists for ANY push payload content.

**Why real.** An Option<String> field that silently decodes to None when the projection line is dropped or the column is misspelled means every whisper rides push payloads in plaintext onto OS lock screens — the exact W4-review finding (4e180a5) — while the unit tests stay green, because they only exercise the formatter given an effect that the real path may no longer supply. This is the mutation-grade gap CLAUDE.md's 'extend the mask to any NEW body-preview surface' deserves. Minor because the masking LOGIC is pinned and the wiring is currently correct; the fix needs a small testability refactor.

**Suggested fix.** Extract notify_inner's row-read (SQL + Info decode) into a pub(crate) fn returning (title-parts, body, effect) and add an ssr integration test push_payload_row_read_carries_the_effect_column: post a whisper via HTTP, call the helper for that mid, assert effect == Some("whisper") and that notification_body over the pair yields "(whisper)".

### M-43 — Targeted-lane ListsChanged visibility reload (review fix d5c0d33) shipped with no test and is currently dead code

- **Severity:** minor  
- **Where:** `src/server/events.rs` : 85-88  
- **Dimension:** tests

**Evidence.** The trap guard `if matches!(be.event, SyncEvent::ListsChanged) { conn.reload_visible().await; }` inside the targeted lane (events.rs:85-88) was added by review commit d5c0d33, whose message says 'Tests: events suite green (9)' — i.e. zero new tests. No production path currently emits a targeted ListsChanged that changes visibility (rail reorder targets the actor but shifts nothing), and tests/events.rs's rail_reorder_no_longer_broadcasts only asserts frame delivery, not a reload.

**Why real.** This guard exists solely to protect a FUTURE wave (e.g. invite-accept nudging the new member) from a silent privacy-filter stale-out — exactly the kind of latent invariant a later cleanup deletes as 'unreachable'. Since the harness exposes a.state (tests/common/mod.rs:54-60 added this branch precisely for direct-emission tests), the path is cheaply testable today: grant membership via DB/HTTP without the untargeted emit, emit_for([member], ListsChanged) on a.state, and assert the member's pre-existing stream now delivers the new channel's message_created.

**Suggested fix.** Add targeted_lists_changed_reloads_the_connections_visibility_set to tests/events.rs using a.state.emit_for(vec![member_account_id], SyncEvent::ListsChanged) after an invite performed before the stream existed... (subscribe → invite via HTTP → drain the untargeted frame → for the targeted variant, simulate via emit_for and assert the subsequent channel event arrives).

**Verifier notes.**
- minor (unchanged)
- minor (unchanged — correct as filed)

### M-44 — message_actions — the CLAUDE.md-elevated shared kind predicate — has no unit test while every sibling helper does

- **Severity:** minor  
- **Where:** `src/ui/shell/channel/mod.rs` : 129-159  
- **Dimension:** tests

**Evidence.** fn message_actions(kind, mine) (channel/mod.rs:129-159) is the declared single source for per-kind affordances ('Client affordances flow from the shared message_actions(kind, mine) predicate — never re-branch kind checks per surface', CLAUDE.md W4 invariant), consumed by both meta.rs and radial.rs:161/382. It is a pure always-on-graph function — trivially testable under cargo test --features ssr — yet has no #[cfg(test)] block, while the same module tree co-locates unit tests for lightbox transforms (lightbox.rs:834+) and reentry (reentry.rs:219+).

**Why real.** The interesting cells are the non-obvious ones the predicate exists to centralize: roll+mine must NOT offer edit/delete (the server 403s, so a regression yields dead-end buttons — the exact bug commit 778cfdd fixed in the NATIVE client), system offers nothing including reply/copy, and the unknown-kind forward-compat arm must never offer edit/delete. All four arms are one careless match-arm edit away from drifting, the consuming surfaces (hover row, radial arc-count at radial.rs:161) silently follow, and nothing in the suite would fail. Severity minor: server-side enforcement holds regardless.

**Suggested fix.** Add a co-located #[cfg(test)] mod in channel/mod.rs: message_actions_offers_nothing_mutable_outside_kind_user (table-driven over kind × mine pinning all four structs, esp. roll+mine ⇒ edit=false,delete=false and unknown ⇒ reply+copy only) and message_actions_count_drives_the_radial_arms (count: user+mine=4, roll=2, system=0).

**Verifier notes.**
- minor (unchanged — server-side enforcement holds; worst case is dead-end UI affordances)
- minor (unchanged — correct as filed)

### M-45 — Initial visible-set load failure yields a deaf-but-200 SSE stream that suppresses the client's poll fallback

- **Severity:** minor  
- **Where:** `src/server/events.rs` : 60-67  
- **Dimension:** errors

**Evidence.** events() builds `Conn { visible: HashSet::new(), .. }` then calls `conn.reload_visible().await` (events.rs:61-67); reload_visible's Err arm only logs and keeps the current set (events.rs:36-38) — for the INITIAL call that means an EMPTY set — and the handler unconditionally returns 200 + Sse. With an empty set, every channel-scoped event hits `Some(cid) if !conn.visible.contains(cid) => continue` (events.rs:93) and is silently dropped. The hydrate driver promotes SSE and retires polling on `onopen` (src/ui/shell/act/sync.rs:286, 'a promoted SSE driver retires the poll fallback at its next tick', sync.rs:88), so the user sees ● LIVE while receiving nothing until some unrelated global ListsChanged triggers a reload.

**Why real.** Every other handler in this codebase maps a failed DB read to 500 'storage error' so the client can retry; this new endpoint is the only request path that swallows a storage error into a success response. The comment at events.rs:35-37 justifies keep-stale-set for RELOADS (fail-closed), but the initial load failing leaves nothing to be stale — the connection is permanently deaf until a global list mutation occurs, and the W1 fallback design (5-failed-connects trips polling) is defeated because the connect SUCCEEDED. Narrow trigger (AuthAccount's own DB query succeeded ms earlier, so it takes a transient single-query failure), hence minor — but the failure mode is sticky and self-masking, and no test covers it.

**Suggested fix.** Change the handler to return Result<Sse<...>, Response> (nothing has streamed yet at that point): on the initial visible_channels error return error_response(500, "storage error") so EventSource fires onerror and the client's backoff/fallback machinery engages.

**Verifier notes.**
- minor (unchanged — correct as filed)

### M-46 — Precache addAll uses default cache mode — a new SW can precache the PRE-rebrand offline.html/manifest from a heuristically-fresh HTTP cache, shipping the old brown palette into the Void Station release

- **Severity:** minor  
- **Where:** `public/sw.js` : 28-31  
- **Dimension:** gap:pwa_service_worker_interplay

**Evidence.** sw.js:30 `cache.addAll(PRECACHE)` fetches with default request cache mode. PRECACHE entries (manifest.webmanifest, offline.html, icons) are served by the static fallback with Last-Modified and no Cache-Control (main.rs:78; only media.rs/mod.rs set Cache-Control), so heuristic freshness applies. The branch rebrands exactly these files: manifest background/theme #221c16→#0b0e14, offline.html palette (diff main...mendicant-bias). The BUILD_REV stamping that makes the new SW install is intact (src/server/mod.rs:264-291, /sw.js served no-cache) — but the install's addAll can still be satisfied by a stale cached copy of the old-palette files if it is within its heuristic window.

**Why real.** Cross-version staleness with a real (if narrow) window: a user whose browser stored offline.html/manifest well after the previous deploy holds them heuristically fresh for ~10% of their age; if this release lands inside that window, the NEW CACHE_VERSION precaches the OLD files and serves them cache-first (sw.js:108-113) for the entire release — old splash colors and an off-brand offline page directly contradicting the W2/W3 rebrand the branch ships, until the release after this one. Purely cosmetic, hence minor, but it silently undermines the branch's own change and the one-line fix is standard practice. Verified consistency otherwise: app.rs:28 meta theme-color, manifest, and offline.html all agree on #0b0e14 in the working tree.

**Suggested fix.** Bust the HTTP cache at precache time: `cache.addAll(PRECACHE.map((u) => new Request(u, {cache: "reload"})))`.

**Verifier notes.**
- minor (unchanged — purely cosmetic, narrow probabilistic window, self-healing once heuristic freshness lapses)

### M-47 — Radial menu's `armed` click-guard silently swallows keyboard/non-pointer activation of role=menuitem buttons

- **Severity:** minor  
- **Where:** `src/ui/shell/channel/radial.rs` : 395-399, 471, 480  
- **Dimension:** gap:a11y_ux_equality

**Evidence.** Every menu button's on:click begins `if !armed.get_value() { return; }` (e.g. radial.rs:396-399 for reply, same for copy/edit/delete), and `armed` is set true ONLY by `on:pointerdown` on the backdrop (471) or the menu container (480). A keyboard activation (Enter/Space on a focused button) dispatches `click` with NO preceding pointerdown, so it returns silently — on a menu whose container is focused on open (line 370-376) and whose buttons carry explicit role="menuitem" + aria-label, advertising operability to AT.

**Why real.** The guard was added in review commit 810f20f to stop manufactured touch clicks, and the fix's mechanism (pointerdown-before-click) is precisely what keyboard activation lacks — a regression the 7-auditor W4 audit missed because it tested touch. Once the menu is open (Tab reaches the buttons from the focused container), Enter does nothing and gives no feedback; Escape works, so the menu is half-keyboard-operable. Severity minor only because opening the menu currently requires a pointer long-press (see the coarse-pointer finding); fixing that finding promotes this one. Pre-empting refutation: leptos `on:click` is the native click event; Chrome/Safari keyboard clicks carry pointerId -1 and fire no pointerdown, so `armed` is provably false on that path.

**Suggested fix.** Treat a click with `ev.detail() == 0` / `pointer_id == -1` (keyboard-originated) as inherently armed, or also arm on keydown within the menu.

**Verifier notes.**
- none — minor is right while menu open remains long-press-only

### M-48 — Bottom tabs expose active state and unread indicator visually only — no aria-current/aria-pressed, unread dot is an empty span

- **Severity:** minor  
- **Where:** `src/ui/shell/mod.rs` : 716-756  
- **Dimension:** gap:a11y_ux_equality

**Evidence.** All four tabs use `class:active=...` only (e.g. mod.rs:718-719); grep confirms zero `aria-current`/`aria-pressed` in the file. The aggregate unread indicator is `<span class="tab-dot"></span>` (mod.rs:729) — an empty styled span (8px accent dot, _nav.scss:122-131) with no text alternative and not aria-hidden, so AT users get neither the unread fact nor noise; the information exists only as pixels. Both `<nav class="rail">` (372) and `<nav class="bottom-tabs">` (716) are unlabeled landmarks.

**Why real.** A screen reader user tabbing the W3-new bottom bar hears four undifferentiated buttons — which one is current and whether anything is unread is conveyed exclusively by color/glow (also a WCAG 1.4.1 use-of-color issue, since `.active` is color + glow only, no shape change). The branch's own convention (W3 review: explicit aria-labels wherever icons are aria-hidden) shows this is an oversight, not a decision; per-channel unread badges DO have text content (`channel-badge`), making the tab-level dot the one silent surface.

**Suggested fix.** Add `aria-current="page"` (or aria-pressed) bound to the same predicate as class:active, and give the dot an sr-only label like "unread messages" (or aria-hidden plus appending the state to the tab's accessible name).

**Verifier notes.**
- minor (unchanged — correct for a real a11y/state-exposure gap with no security or correctness impact, though the project's UX-equality value makes it worth fixing before merge)
- minor (unchanged)

### M-49 — Rebuilt lightbox has no dialog semantics, no focus trap, and no focus restore on close

- **Severity:** minor  
- **Where:** `src/ui/shell/channel/lightbox.rs` : 692-703, 758-766  
- **Dimension:** gap:a11y_ux_equality

**Evidence.** The branch-new lightbox (commit f81cfed rewrote the file) focuses its container on open (lines 699-701) and handles Escape/arrows/+,-,0 on its keydown (363-389) — but the container `<div class="lightbox" tabindex="-1">` (758-766) carries no role="dialog"/aria-modal, there is no Tab trap (Tab past the zoom "+" button exits into the page behind the full-screen overlay, after which Escape no longer closes since keydown only fires while focus is inside), and no on_cleanup/focus-restore exists anywhere in the file (contrast modal.rs:113-117, which the module docs explicitly opt out of for visual reasons, lines 9-11).

**Why real.** Concrete consequences: AT is never told a modal opened (content behind remains in the reading order), and after close (Escape or drag-dismiss) focus drops to <body> because the focused container is unmounted — a WCAG 2.4.3 regression the shared Modal solves for every other overlay on this branch. Keyboard zoom/nav equivalents themselves are good (arrows, +/-/0 mirror pinch/double-tap/swipe), so this is the remaining gap, not operability. The visual-styling reason for staying bespoke (modal.rs:9-11) does not require dropping the behavioral half.

**Suggested fix.** Add role="dialog" aria-modal="true" aria-label="attachment" to the container, wrap Tab like modal.rs does, and capture/restore the previously-focused element on open/close.

### M-50 — Sync chip 'POLLING' state fails WCAG AA contrast (~3.5:1 at ~11px)

- **Severity:** minor  
- **Where:** `style/_content.scss` : 105-121  
- **Dimension:** gap:a11y_ux_equality

**Evidence.** `.sync-chip { font-size: 0.68rem; color: var(--text-faint); }` (_content.scss:105-110) — `--text-faint: #5d6b80` (_tokens.scss:29) over the glass topbar whose opaque base is `--surface: #0e121a`. Computed contrast ≈ 3.5:1 (and lower over the 55%-alpha glass against bright chat content), versus the 4.5:1 AA requirement for ~10.9px text. The `.live` mint state (#8ee6c8, ≈12.8:1) passes; only the degraded POLLING state fails. Branch-added in W3/T6 (commit 0167793).

**Why real.** This is information, not decoration — the chip is the only surface telling the user real-time sync has degraded to polling, and it is hardest to read in exactly that state. Every other muted-text token spot-checked passes (--text-muted #8a98ad: 6.6:1 on --void, 6.1:1 on --card), so the palette supports a compliant choice; this one token pick is the outlier. Sub-AA fine-print on low-end panels (the project's stated POCO C3 floor, _content.scss:137) compounds it.

**Suggested fix.** Use --text-muted (6:1+) for the POLLING state, keeping --text-faint for the dot glyph only if desired.

### M-51 — New glass chrome has no prefers-reduced-transparency or forced-colors handling — fallback keys only on @supports

- **Severity:** minor  
- **Where:** `style/_foundation.scss` : 9-24  
- **Dimension:** gap:a11y_ux_equality

**Evidence.** The glass mixin's only fallback gate is `@supports (backdrop-filter: blur(1px))...` (_foundation.scss:19-23); a repo-wide grep for `prefers-reduced-transparency` and `forced-colors` returns nothing. The W2 'glass fallback' commit (9db356d) made the opaque `--surface` the default and translucency the @supports upgrade — correct for engine support, but a user who has explicitly set Reduce Transparency (macOS/iOS, exposed to the web since Safari 16.4 / Chromium 118) still gets the full 55%-alpha blurred chrome on topbar, tab bar, and channel sheet.

**Why real.** Reduce Transparency is an accessibility setting people enable for legibility/vestibular reasons; the branch already honors the sibling setting (prefers-reduced-motion has a global freeze in _base.scss:86 plus per-effect kill lists), so this is an asymmetry between two parallel a11y media features rather than a deliberate scope cut. The fix is one media block reusing the already-written opaque fallback. Filed minor: visual legibility degradation for an opt-in user group, no functional lockout.

**Suggested fix.** Inside the glass mixin add `@media (prefers-reduced-transparency: reduce) { background: var(--surface); backdrop-filter: none; -webkit-backdrop-filter: none; }` after the @supports block.

**Verifier notes.**
- minor (unchanged — correctly calibrated)
- minor (unchanged)

### M-52 — Undo-delete toast: focus is destroyed with the deleted row and the 6s window is fixed, making keyboard Undo a blind race

- **Severity:** minor  
- **Where:** `src/ui/shell/act/message.rs` : 541-565 (with src/ui/shell/toast.rs:20-53)  
- **Dimension:** gap:a11y_ux_equality

**Evidence.** delete_message removes the row optimistically (`s.msg.messages.update(|v| v.retain(|m| m.id != mid))`, act/message.rs:552) — when triggered from the keyboard via the row's focus-within 🗑 button, the focused element is unmounted and focus drops to <body>. The toast's Undo button is focusable with a :focus-visible style (_toast.scss:106) and the host is aria-live (toast.rs:22, announces "Message deleted Undo"), but nothing moves or guides focus there; the auto-dismiss timer is a fixed `UNDO_TOAST_MS` with no pause on hover/focus (act/toast.rs push()).

**Why real.** A keyboard user must discover, within 6 seconds, that Shift+Tab from the body lands on the trailing toast-action — undiscoverable and a WCAG 2.2.1 (Timing Adjustable) concern for a timed control. Mitigation exists (the soft-delete stays restorable via the trash pane until the 1h purge, which is why this is minor, not important), but the in-flow undo path the feature was built for (commit b9b9e2f "act instantly, regret for 6 seconds") is effectively pointer-only in practice.

**Suggested fix.** After removing the focused row, move focus to a stable neighbor (next message row or the composer), and pause the dismiss timer while the toast has focus-within or hover (restart on blur).


## Refuted (0)

_The final verify round refuted nothing; earlier rounds’ refutations were absorbed during dedup (raw→unique)._

## Coverage notes per dimension

### authz

Enumerated every new/modified axum route in the diff and checked each against the auth rules. CLEAN: (1) Identity — every new handler (events, unread, typing_ping, typing_drafts, roll_message) derives identity solely from the AuthAccount cookie extractor; none trust request-body/path identity. roll_message re-runs the persona double-check via the shared resolve_send_persona (posting.rs), matching the invariant. (2) Privacy-404 — typing_drafts, roll_message, mark_read, restore_message all route through channel_access/require_own_message and collapse ChannelNotFound+NotMember to an identical 404 body; tests/typing_drafts.rs::typing_drafts_returns_privacy_404_with_identical_body_for_non_members and tests/roll.rs::nonmember_roll_is_privacy_404 pin it. (3) Roll immutability — edit/delete both 403 on kind='roll' (editing.rs:55,102), pinned by tests/roll.rs. (4) SSE bus is genuinely safe against visibility races: it is id-only (notify-and-fetch), so even if a stale MessageCreated frame slips to a just-removed member before their connection processes the broadcast ListsChanged, the client's permission-checked refetch re-authorizes and 404s — confirmed every membership/channel/guild mutation emits ListsChanged (broadcast) and the unfold reloads visible on it; targeted lane (emit_for) only ever carries account-scoped id-only nudges (ReadStateChanged to self, FriendsChanged to both edge parties, rail-order ListsChanged to self), none leaking a channel the recipient can't see. (5) is_admin fail-closed and system-broadcast admin gate unchanged; tests/system_messages.rs pins 403/401. (6) visible_channels filters by guild_member + deleted_at=NONE + kind='text', shared by /events and /unread; tests/unread.rs::unread_lists_only_channels_the_caller_can_see and tests/events.rs::outsider_never_receives_events pin it (harness SseRead distinguishes Timeout from Closed, so negative tests can't pass vacuously). (7) Whisper mask correctly extended to reply-quote (reading.rs) and push (push.rs) with a dedicated unit test; the ONLY missed surface is Ghost Quill drafts (finding 1). The friends-changed emit self-acknowledges a harmless id-only nudge-arbitrary-account side effect (friends.rs:248-260) — already documented/accepted, not reported.

### injection

INJECTION & PARSER SAFETY sweep over git diff main...mendicant-bias (74 commits). CLEAN areas, all read in full: (1) SQL binding discipline — every new/touched query verified parameterized: unread.rs batched multi-statement query (only loop indices `cid_{i}/at_{i}/mid_{i}` and the compile-time `UNREAD_COUNT_CAP` enter query text; stored cursor re-binds as a true surrealdb Datetime, honoring the lex-misorder invariant), access.rs visible_channels, reading.rs resolve_display_names (IN-list via bound $accts) + the new attachment_mimes record-pointer subquery in MSG_PROJECTION (server-minted ids, validated at post via all_media_exist), editing.rs message_author_and_kind, posting.rs persist_message (kind/effect ride as binds; effect_set/persona_set/reply_set are static column fragments), read_state.rs mark_read (RFC3339 pre-parse → 400, type::datetime bind), friends/guilds/membership diffs (emit-only additions), system_messages.rs (emit-only). Pre-existing splice sites re-verified loop-index/static-fragment only (guilds rail order, personas gallery, lorebook PATCH sets). (2) Dice grammar (rolling.rs) — panic-free and overflow-safe on adversarial input: digits-only gate rejects signs/unicode digits, u32 parse catches 30-digit overflow, the single raw index rest.as_bytes()[sides.len()] is provably in-bounds (separator exists), bounds 1..=100 dice / 2..=1000 sides / |K|<=1000 checked post-parse, i64 totals can't overflow; tests/roll.rs pins bounds, invalid-expr 400s, privacy-404, persona snapshot, immutability. (3) Markup — src/markup and src/ui/markup_view.rs untouched by the branch; link tokenizer remains http/https-gated; new render surfaces (draft preview, ghost rows, roll chip) all use Leptos text nodes (auto-escaped); roll bodies rendered verbatim, never markup-parsed; effect class re-whitelisted client-side (whisper|shout|spell) so wire values can't inject class names. (4) inner_html — only src/ui/icons.rs with compile-time const SVG path literals. (5) URL/attribute sinks — /media/{id} src/href built from server-validated random ids; get_element_by_id (not query_selector) used for dynamic msg-{id} lookups; EventSource fixed \"/events\"; SSE frames JSON-serialized (newline-safe) via axum Event. (6) Whisper mask coverage on NEW surfaces — push payload masked (+unit tests), reply-quote masked (+integration test), /unread carries no bodies, undo toast/date separators/hum carry no bodies; the two gaps found are the findings above. (7) Lightbox/radial gesture math — divisions guarded (start_dist.max(1.0), scale clamped >= 1.0), no unwrap/indexing panics; unit-tested under ssr. (8) Ghost Quill store — char-boundary truncation correct at the 2000 cap, mutex never held across await, TTL prune on read+write, membership re-checked on the drafts fetch, drafts endpoint is the only draft-text egress (SSE bus stays id-only). (9) typing_drafts/unread/events test suites confirmed pinning privacy-404, cross-guild non-leak, statement-index bookkeeping. NOT covered (other auditors' dimensions): SSE delivery/visibility race semantics, unread-count semantics (own-message counting), CSS/design changes, native Freya client beyond its 4-file diff (verified effect:None + kind gating only).

### leaks

Dimension: content leaks & media safety, branch main...mendicant-bias (74 commits). CLEAN after inspection: (1) Web Push whisper mask present and unit-tested (push.rs:415-417, notification_body; whisper_effect_masks_push_notification_body_with_fixed_placeholder) — image-only whispers also masked; push payload `image` is the persona avatar id, never an attachment. (2) Server reply-quote mask present in MSG_PROJECTION (reading.rs:400-404), pinned by tests/messages.rs. (3) GET /unread (unread.rs) carries only ids/counts/timestamps, gated by visible_channels; bind-name splice is loop-index-only (sanctioned form). (4) SSE bus (events.rs + protocol.rs SyncEvent:856-885) is strictly id-only — no variant carries text; per-connection visibility filter + targeted lane verified, reload-on-ListsChanged for targeted events present. (5) Ghost Quill server side: typing_drafts endpoint membership-gated (privacy-404), caller's own draft excluded, TTL-pruned on read AND write, drafts cleared on send/roll (posting.rs clear_draft call), draft never rides the bus, mutex never held across await; tests/typing_drafts.rs covers permissions. Draft renders as a Leptos text node (escaped, no XSS). (6) Corridor hum (act/hum.rs) is pure per-channel boolean generations — no content, unit-tested. (7) Re-entry (act/reentry.rs) localStorage stores only channel-id→message-id scroll marks; date labels derived from timestamps. (8) Undo-toast (act/toast.rs, toast.rs) shows fixed text 'Message deleted', no body snippet; restore is server-gated by require_own_message; roll rows reject edit/delete 403 (editing.rs:55-57,102-104). (9) Media path traversal: thumbnail cache path uses the URL id only AFTER the id resolved to an existing media_blob row (ids are server-minted random hex, load_media_row → 404 first); canonicalize + starts_with(media_dir) retained; w clamped 16-512; X-Content-Type-Options: nosniff present on all three response builders; SVG/HTML still rejected at upload and neutralized on serve. (10) Lightbox gallery is same-message-only (lightbox.rs:14-21), so veiled whisper attachments are not reachable from another message's gallery; whisper attachment veil CSS present (_content.scss:575-585, pointer-events gate) and the veil wrapper is insert-only. (11) No tracing/log line echoes body or draft text anywhere in src/server (grepped); error bodies are fixed strings. (12) posting.rs effect validation (400 on unknown, empty=absent) matches schema ASSERT; rolls carry no user text and no effect; system-broadcast push path carries no whisper interplay. NOT covered: actual runtime testing (read-only audit), SCSS visual correctness of the veil blur radius, service-worker JS (no sw changes touching media caching found in the diff).

### realtime

CLEAN after end-to-end reads: (1) Bus payloads are genuinely id-only — every SyncEvent variant (protocol.rs:856-885) carries only channel/message ids; sse_frame serializes the enum directly; Ghost Quill draft TEXT verified to ride only the membership-gated GET /channels/{cid}/typing-drafts (typing.rs:135-195), never the bus. (2) Account-targeted lane: targets matched against AuthAccount.0 (bare meta::id key, session.rs:27-30); read_state and friends emit the same form; tests pin both devices receiving and third parties silent (tests/events.rs:254-433) — no wrong-account delivery found. (3) Targeted-ListsChanged visibility reload trap already fixed (events.rs:85-88, review d5c0d33). (4) Emit ordering is post-commit in ALL mutation handlers (posting.rs:177, rolling.rs:258, editing.rs:72/120/161, membership.rs:154/225/285, channels.rs, deletion.rs) — no emit-before-commit race. (5) typing_drafts mutex discipline holds everywhere: lock→mutate→drop, no .await under lock (typing.rs:88-116, 154-167, 202-208); TTL pruned on read AND write; truncation is char-boundary-safe; clear-on-send AND clear-on-roll pinned by tests/typing_drafts.rs (8 tests incl. privacy-404, author-exclusion, TTL via with_draft_ttl). (6) RecvError::Lagged → reload + ListsChanged resync nudge is correct and the client honors it (dispatch ListsChanged refetches lists+unread+open channel, sync.rs:445-451); subscribe-before-load closes the connect gap. (7) GET /unread: strict composite cursor (sent_at > $at OR (= AND meta::id > $mid)) matches the pagination tie-break, binds datetime as true Datetime, soft-deletes excluded everywhere, loop-index-only splicing, mixed-batch statement alignment + privacy pinned by tests/unread.rs (4 tests); no off-by-one found; client first-sight baseline race already guarded (unread==0 proof, message.rs:1071-1090). (8) Client driver generation machinery: traced all bump/capture sites — no persistent double-EventSource or double-poll-loop; transient overlap is bounded to one event/tick by the stale-gen self-close; demote/promote handover keeps the polling latch correctly; mount-order race (restore_session vs start_sync) ruled out (restore is async, start_sync synchronous in same Effect, shell/mod.rs:328-352); ReadStateChanged cross-device feedback loop ruled out (advanced-gate in set_last_seen prevents ping-pong). (9) Corridor hum: derived only from server-side visibility-filtered events, cosmetic-only (never touches read state or fetches), generation decay unit-tested in act/hum.rs. (10) W3 sheet auto-open unread-wipe fix (fadc583) verified still in place (select_server_for_sheet does not auto-open). (11) tests/common SseRead three-way harness prevents vacuous negative assertions. NOT covered: actual browser-level behavior of the wasm driver (no hydrate test rig exists), nginx/proxy buffering of SSE in prod, and broadcast-capacity (256) lag under load — reasoned about only.

### schema

SCHEMA & DB SAFETY dimension, full branch diff (main...mendicant-bias) audited. CLEAN findings, all verified by reading every feeding path: (1) schema.surql diff is exactly two changes, both correct — `message.kind` widened via DEFINE FIELD OVERWRITE (the documented IF-NOT-EXISTS exception; full TYPE/DEFAULT/ASSERT restated; runs before the backfill; OVERWRITE doesn't re-validate rows) and `message.effect` added as option<string> with `ASSERT $value = NONE OR $value IN [...]` so the backfill UPDATE's whole-row revalidation of legacy effect=NONE rows passes; the kind materialisation stays INSIDE the single first backfill (coalesced ?? 'user'); replayed mentally against populated fenrir state and pinned by 4 prod-shaped tests in tests/schema_apply.rs (populated-table apply, old-ASSERT-already-exists apply, effect-over-populated with persistence probe, NONE round-trip). (2) No new tables/fields/indexes beyond `effect` — unread/read-state/feedback/push tables all predate the branch; no new UNIQUE index over NONE-holding columns. (3) GET /unread (new, unread.rs): strict composite (sent_at, id_key) tie-break identical to reading.rs cursor and mark_read's MAX-cursor compare; stored cursor bound as a native Datetime (stronger than type::datetime cast — never a string); only loop indices and the compile-time UNREAD_COUNT_CAP spliced into SQL (sanctioned form); statement-index bookkeeping pinned by unread_mixed_batch_keeps_statement_indices_aligned; visibility from access::visible_channels (live text channels, live guilds only) — outsider test pins privacy. (4) Datetime wire round-trip: to_rfc3339_fixed (9-digit nanos) → mark_read parse → type::datetime is lossless; no lex-misorder regression. (5) Roll persist path: shares persist_message + resolve_send_persona (persona double-check) with post_message; kind bound as $kind never spliced; roll grammar parser is panic-free (byte-index on split_once separator verified safe, digits-only gate blocks sign smuggling, i64 sums can't overflow at N≤100,M≤1000); edit/delete 403 guards on kind=='roll' with channel-scoped author+kind read; restore needs no guard (rolls can never be soft-deleted — verified delete is the only deleted_at setter and it 403s first). (6) All racy UNIQUE-create paths still route through with_write_conflict_retry; mark_read/wear/push-subscribe DELETE-then-CREATE transactions converge on retry; invite/register/friendship single-statement CREATEs map \"already contains\" → 409. (7) purge_soft_deleted DOES cascade channel_read_state + channel_active_persona + user_guild_order + custom_emoji for purged channels/guilds (server/mod.rs:311-350) — no new orphan class; Ghost Quill drafts are in-memory with TTL pruning on write AND read, cleared on send/roll (test-pinned), mutex never held across await, membership-gated both directions, draft text never on the SSE bus (id-only invariant intact — typing_ping emits only Typing{channel_id}). (8) Whisper mask covers every body-preview surface I could enumerate: reply-quote snippet (MSG_PROJECTION, test-pinned), push payload incl. image-only whisper (unit tests), /unread (id-only), SSE (id-only), corridor-hum (id-only); trash listing returns full bodies by design (whisper is cosmetic per spec, full body is member-visible anyway). (9) SSE bus: targeted lane (ReadStateChanged/FriendsChanged/rail ListsChanged) is strictly self/edge-scoped; targeted-ListsChanged visibility reload trap fixed (review d5c0d33); subscribe-before-load ordering correct; Lagged → resync nudge; adversarial privacy tests use three-way SseRead (non-vacuous). (10) emit ordering is persist-then-emit everywhere (no fetch-before-commit race); nova_dot UPSERT idempotent. (11) Media immutable Cache-Control safe: ids are server-minted random and blobs never replaced in place; error paths uncached; tests/cache_control.rs covers. NOT covered: actual execution of the new MSG_PROJECTION on a 3.0.4 server (finding 1 — sandbox blocked executing a downloaded 3.0.4 binary); client/ui and native-graph code beyond their DB-touching seams (other auditors' dimensions); load behavior of /unread with hundreds of visible channels (2 statements/channel, indexed — scale-bounded by design comment).

### message_domain

MESSAGE-DOMAIN dimension, branch mendicant-bias (74 commits) vs main. Verified CLEAN: (1) Roll immutability — edit AND delete on kind='roll' are explicit 403s in editing.rs:55-57/102-104, guard is kind-based (returned by require_own_message), NOT inherited from system authorship (doc + code confirm); no manager/admin route can mutate arbitrary messages (full route table in server/mod.rs:139-229 — only own-gated PATCH/DELETE/restore exist; purge is internal); pinned by tests/roll.rs editing_own_roll_is_403... and deleting_own_roll_is_403.... Rolls can never be soft-deleted ⇒ restore needs no roll guard (verified: nothing else sets message.deleted_at). Grammar parser checked for panics/bypasses (parse_digits digits-only gate, sides-index access safe via split_once, i64 sums can't overflow at N≤100/M≤1000). (2) Persona send-path — roll_message uses the exact shared resolve_send_persona + persist_message (rolling.rs:217-249); no divergence (post checks kind!='text' same way; only request field name differs); pinned by roll_with_unowned_persona_is_rejected_as_attribution. (3) Snapshot invariant — persist_message snapshots name/desc/color/avatar in the CREATE (posting.rs:235-243); push payload uses snapshot persona_name (push.rs:307); typing-draft names resolve live BY DESIGN (pre-send drafts, shared resolve_display_names); system messages have no persona; native renders envelope snapshots. MSG_PROJECTION's `?? persona.*` legacy fallback is pre-existing documented design. (4) effect validated server-side against MESSAGE_EFFECTS (posting.rs:78-87), matches schema ASSERT (schema.surql effect field admits NONE + the 3 values); pinned by message_effect_round_trips... and unknown_message_effect_is_400...; PATCH cannot touch effect. Whisper mask verified on BOTH documented surfaces (MSG_PROJECTION reply snippet reading.rs:397-405; push notification_body push.rs:415-417, unit-pinned) and checked on every NEW body-preview surface: local Notification is title-only (notify.rs:481-491), unread is counts-only, hum is id-only, ghost drafts carry pre-send text (no effect exists yet), trash list uses MSG_PROJECTION. (5) Shared message_actions predicate — hover row (meta.rs:51), radial (radial.rs:161/382 + system-class pointerdown gate), native button gate (ui.rs:1129) all flow from it; channel/mod.rs:792-804 is_system/is_roll/effect branches are styling-only; the client effect class re-whitelist prevents class injection; no rogue kind re-branches found (repo-wide grep). (6) Soft-delete on read — unread.rs filters deleted_at=NONE in all three statement shapes and visible_channels filters channel+guild deletion; re-entry/hum derive from already-filtered data; drafts are memory-only with membership-checked fetch (typing_drafts.rs privacy-404 pinned). (7) Restore authz — own-gated + channel-scoped via require_own_message (code correct; test gap reported as finding 2); restore-races-purge is benign (SurrealDB 3.x UPDATE on a missing record no-ops → 204, undo window 6s vs 1h purge makes it unreachable from the toast; trash-pane restore reloads server truth). Ghost Quill: id-only bus intact (draft text only via permission-checked GET, typing.rs), sender/receiver双 opt-in verified at composer (channel/mod.rs:1437-1441) and refresh_ghost_drafts pref gate; clear-on-send AND clear-on-roll pinned by tests. Also checked: optimistic-hide vs in-flight reconcile race (self-heals via own MessageDeleted event — flicker only, not reported); edit not re-resolving pinged_users and reply-target validation ordered before membership check (both pre-existing on main, unchanged by branch — not reported); resurface partition_point ordering matches server composite cursor. NOT covered: SSE bus internals/visibility-set lifecycle (other dimension; tests/events.rs pins privacy), media, auth/session, lightbox gesture code, SCSS."

### client_ui

Scope: full read of the branch's hydrate-graph surface — act/toast.rs, shell/toast.rs, act/reentry.rs (+unit tests), act/channel.rs (open_channel_at lifecycle), act/message.rs (delete/undo/resurface, after_send_success, ingest/sync_messages, refresh_unread, start_poll), act/sync.rs (SSE driver, demote/probe/wake), act/hum.rs (+tests), act/prefs.rs, act/guild.rs, act/account.rs, act/mod.rs re-exports, channel/mod.rs (full, incl. divider render, append/anchor effects, composer), channel/lightbox.rs (full gesture engine + tests), channel/radial.rs, channel/meta.rs, channel/attachments.rs, shell/mod.rs, shell/state.rs, client/api.rs + protocol.rs diffs, _toast.scss, _nav.scss, _content.scss (dividers/whisper/floats), _lightbox.scss. Verified-CLEAN per my dimension: (1) undo-toast keyed timers (generation-key dismiss correct, no double-delete — affordances vanish with the row; restore-after-timeout falls back to trash pane by documented design; single-slot replacement documented; aria-live host permanently mounted, pointer-events handled); resurface/seen/cursor interplay with reconciles is consistent and review-hardened. (2) NEW divider is one-shot per open (cleared in open_channel_at and guarded clear on send per fe5001a), recomputes correctly across load_older prepends (self-corrects upward), tie-break matches the server cursor (unit-tested), sentinel ids can't collide with msg- selectors (radial target_msg_li, capture_scroll_mark, anchor lookups all checked); take_restore_anchor consumed unconditionally per review. (3) Lightbox: pointer capture released implicitly; rAF frames disarmed on Close/Snap; double-tap anchor math unit-tested; dismiss-vs-pan arbitration (pan wins zoomed) correct; passive-target filtering covers all chrome. (4) Corridor hum: generation decay pure-tested, suppressed on open row, no fetches added; W3 cross-device unread-wipe NOT re-introduced — select_server_for_sheet is select-only and refresh_unread's first-sight baseline carries the unread>0 guard. (5) ssr stubs exist for every new act fn with sane defaults (restore_session false, prefs false, load_scroll_marks empty); gloo-storage reads all go through LocalStorage::get (no raw localStorage in branch code). (6) Feature-graph disjointness holds: protocol.rs additions are serde-only; reentry/hum/lightbox/sync gate web-sys/gloo behind hydrate. (7) Safe-area: bottom inset owned exactly once — .bottom-tabs owns it on mobile, composer's inset zeroed ≤768px, toast adds (--tabbar-h + inset) on top of measured --composer-h = single count; verified against _nav.scss/_toast.scss/_content.scss. (8) No visualViewport changes on the branch; composer height is ResizeObserver-measured onto <html> with cleanup. Known pre-existing issues NOT reported (unchanged code, not newly exposed): attachment gallery includes non-image file tiles (attachments.rs is_image, diff-empty vs main) so a file-bearing message can step the lightbox onto a broken <img>; refresh_unread's prelude can mark a lorebook channel read with a stale text-channel cursor; stale last_dist when switching channels with scrollTop exactly 0; trash-pane restore leaves oldest/more_history stale; edits/deletes invisible in >100-message channels until reload. Not run: cargo clippy --features hydrate (read-only audit; branch CI presumably covers compile).

### perf

Scope: full `git diff main...mendicant-bias` (74 commits) audited for the PERFORMANCE dimension; server claims verified empirically with EXPLAIN FULL against the local SurrealDB 3.1.3 dev binary (the project's documented EXPLAIN reference) in disposable namespaces (created, measured, REMOVEd; dev in-memory instance only — prod untouched). CHECKED AND CLEAN: (1) SSE server hot loop — per-event per-connection cost is a HashSet lookup only; DB is hit solely on ListsChanged/Lagged per connection (events.rs:32-39), N×M amplification already documented in-code; broadcast capacity 256 with lag→ListsChanged resync is sound notify-and-fetch at this scale. (2) Account-targeted lane (emit_for) — no visibility query, id-only, the remove_friend spare-nudge side effect is documented and rate-bounded. (3) MSG_PROJECTION growth — attachment_mimes uses EXPLAIN-verified record-pointer point-reads (DynamicScan), reply_to/whisper-mask arms are per-row point derefs; no N+1 reintroduced. (4) /unread statement bookkeeping pinned by tests/unread.rs mixed-batch regression; empty-batch edge handled; the 2C-statement structure itself is fine — the defect is the predicate shape (filed). (5) Typing-draft store — O(map) retain prune on write/read, mutex never across await, 2000-char truncation, bounded. (6) Roll grammar bounds (≤100 dice, ≤1000 sides) — no CPU amplification. (7) Client SSE self-heal — wake refresh throttled 3s, probes single-flight with 30s→5min jittered backoff, generation handover prevents double drivers: well engineered. (8) Corridor hum — pure generation map, zero fetches, bounded timers, suppressed-on-open; render closures fine-grained per row. (9) Re-entry — capture_scroll_mark bounded by page size (≤100 getBoundingClientRect, once per switch); divider math O(page). (10) Lightbox — rAF-batched single style write per frame, will-change scoped to gesture, overlay re-renders only on open/close: exemplary. (11) WASM bundle — only web-sys feature flags added (EventSource/MessageEvent/DomRect/ResizeObserver), no new crates in hydrate; fonts net smaller (Space Grotesk replaces EB Garamond). (12) prefers-reduced-motion — explicit kill list covers ALL new W4 keyframe consumers including non-fx-named classes (constellation, hum dot, radial, roll, effects) plus the _base.scss global freeze; exemplary. (13) Body-limit route groups untouched. NOT independently re-verified: actual Pi 4B timings (all absolute numbers measured on the M2 dev machine — the Pi multiplies them); the non-keyed wholesale message-list re-render per append (pre-existing pattern, consciously mitigated by the branch via delegated listeners — noted, not filed); MessageEdited→refresh_unread minor over-fetch (edit can't change unread) judged too small to file.

### tests

Audited the full branch test delta (tests/events.rs, sync_events.rs, unread.rs, typing_drafts.rs, roll.rs, system_messages.rs, schema_apply.rs, messages.rs additions, media.rs/cache_control.rs additions, common/mod.rs SSE harness) against the server code it claims to pin (server/events.rs, access.rs, messages/{unread,typing,rolling,posting,editing,reading}.rs, push.rs, state.rs, guilds/*, system_messages.rs, storage/schema.surql) plus the client seams (ui/shell/channel/mod.rs composer, act/message.rs ghost-draft fetch, state.rs effect_mode). CLEAN areas, verified by reading both test and implementation: (1) roll grammar/bounds/format — exhaustive 400 matrix incl. boundary-exact accepts, sign-smuggling (`+3`, `2d6+-1`), overflow via digits-only parse; author edit AND delete 403 with body-unchanged/survival re-asserts; suggested-persona rejection and persona snapshot on rolls; roll privacy-404 asserts the body-identical contract, not just status. (2) typing drafts — TTL via injected arena_with_draft_ttl (no sleeps), clear-on-send AND clear-on-roll, bare-ping wire compat + clear, empty-string clear, 2000-char char-boundary truncation, own-draft exclusion, and a three-way privacy-404 body-identity check against the messages handler. Receiver/sender opt-in is client-localStorage by documented design — no server enforcement exists to test. (3) schema_apply — prod-shaped guards are COMPLETE for this branch's schema delta: kind backfill-fold regression, kind ASSERT widening over an already-defined field (the OVERWRITE exception), effect-over-populated-rows with a SCHEMAFULL silent-strip existence probe, effect enum ASSERT incl. NONE, nova_dot seed + login sentinel. (4) system broadcast — fan-out core (first-channel-by-position, skip lorebook-only/deleted-channel/deleted-guild), fail-closed 403 + writes-nothing, 401, and per-message SSE emission on the ROUTER's state (a.state added to the harness for exactly this); the admin-ALLOWED HTTP path is untested but that's the documented env-race convention from tests/feedback.rs. (5) SSE test QUALITY is high: negative assertions are Timeout-not-Closed via the three-way SseRead with aliveness proofs on the same stream — not vacuous. (6) whisper reply-quote mask integration-tested end-to-end; effect round-trip incl. empty-string-as-absent; unknown effect 400 + persists-nothing. (7) media immutable Cache-Control on all three arms + 404-stays-uncached; nosniff/MIME-allowlist/path-canonicalization tests pre-exist and the branch added no new media write surface. (8) retry: no new racy UNIQUE-index CREATEs introduced (drafts are in-memory, rolls are plain CREATEs); retry_canary pins the matcher against the live 3.1.3 binary and the branch's matcher update (690b93a). (9) unread mixed-batch statement-index bookkeeping pinned with deliberately differing counts (anti-vacuous). Findings above are the residue: the revocation direction of SSE visibility, ghost-draft channel scoping, the whisper×ghost-quill seam, equal-sent_at tie-break in both cursor implementations, soft-delete exclusions on /unread, restore negatives, push effect plumbing, the targeted-reload trap guard, and the message_actions predicate. Not reported as findings: hydrate/freya graphs having no test harness (documented accepted state in CLAUDE.md), the Lagged→resync path (capacity-256 starvation impractical to synthesize), Cache-Control 'public' on cookie-authed media (security-dimension, ids are unguessable capabilities, no shared cache in prod topology), and kicked members' drafts lingering ≤8s TTL (bounded, in-memory).

### errors

Scope: full `git diff main...mendicant-bias` server (ssr) surface — events.rs, state.rs (AppState/BusEvent/emit/emit_for), messages/{rolling,unread,typing,posting,editing,reading,read_state,mod}.rs, access.rs (visible_channels), push.rs, media.rs, retry.rs, system_messages.rs, friends.rs, guilds/{channels,deletion,membership,mod}.rs, schema.surql, protocol.rs, plus tests/{events,roll,typing_drafts,unread,media,cache_control}.rs and tests/common/mod.rs harness. CLEAN findings per the assigned dimension: (1) Panic vectors — every added unwrap/expect/index judged in context: rolling.rs:112 `rest.as_bytes()[sides.len()]` is provably in-bounds (split_once guarantees a 1-byte ASCII separator at that byte offset); parse_digits rejects sign/overflow/unicode digits; roll totals bounded (100×1000+1000, no i64 overflow); truncate_chars is char-boundary-safe; unread.rs `out[*i]` indexes are constructed from the same `visible` enumeration; sse_frame's expect cannot fail for the current internally-tagged unit/struct-variant enum (commented in code); mutex .expect(poisoned) matches the pre-existing typing discipline and critical sections cannot panic. (2) Status mapping — roll grammar errors are clean 400s validated before any DB touch (no existence-probe added; note the pre-existing-on-main reply_target/attachment probes before the membership gate in posting.rs are NOT branch-introduced, verified against main); privacy-404 discipline holds on roll/typing-drafts/unread (pinned by tests); unknown effect → 400 before the DB ASSERT; roll immutability 403s pinned by tests/roll.rs. (3) Racy CREATEs — the branch adds none on UNIQUE indexes (message CREATE has no unique index; mark_read retry wrapper pre-existing); retry.rs's broadened 'failed transaction' match is review-adjudicated (F-D6) with a live canary pinning predicate disjointness. (4) Locks — typing_drafts mutex never held across await (verified all 3 sites + clear_draft is sync); no new shared server state beyond it; corridor-hum/unread caches are hydrate-side only. (5) Task leaks — no new tokio::spawn in the server diff; SSE connections are unfold streams dropped with the response body; broadcast capacity 256 with Lagged→resync bounds memory; lag/reload amplification is documented in-code as accepted at instance scale. (6) Web-push — branch only added whisper masking (review 4e180a5, unit-pinned); dead-sub pruning unchanged. (7) Image paths untouched except Cache-Control (finding 3). (8) Body limits — all new routes (/events, /unread, /channels/{cid}/roll, /typing-drafts, typing's new JSON body) registered in the 512KiB small-body group; drafts truncated to 2000 chars; bare-ping wire-compat pinned by tests. Whisper-mask invariant checked against every NEW body-preview surface added by the branch (unread = ids only, SSE = id-only, ghost drafts = compose text not message bodies, trash listing uses the masked MSG_PROJECTION): no unmasked surface found.
