//! M4/T7 Ghost Quill integration tests: the ephemeral typing-draft store
//! behind `POST /channels/{cid}/typing` (optional `draft` body field) and
//! `GET /channels/{cid}/typing-drafts`.
//!
//! The SSE bus stays id-only — draft TEXT never rides a `SyncEvent`; it is
//! only ever readable through the permission-checked GET under test here.
//! Covered: cross-member readability + own-draft exclusion, the privacy-404
//! (body-identical) for non-members, TTL pruning via an INJECTED short TTL
//! (no 8s sleeps), clear-on-send (message, roll, and edit — review M-02),
//! the bare ping staying wire-compatible while clearing any stored draft,
//! the empty-string clear, the 2000-char truncation cap (char-boundary
//! safe), the whisper-armed draft mask (review M-01 — the M4 "extend the
//! mask to any NEW body-preview surface" invariant), and the per-channel
//! scoping clause that keeps one channel's drafts out of every other
//! channel's fetch (review M-15 — cross-guild AND same-guild).

mod common;

#[cfg(feature = "ssr")]
use axum::http::{Method, StatusCode};
#[cfg(feature = "ssr")]
use serde_json::json;

/// Register an owner, create a guild, add `Alice` as a member, and return
/// `(owner_cookie, alice_cookie, default_text_channel_id)`.
#[cfg(feature = "ssr")]
async fn owner_and_member(router: &axum::Router) -> (String, String, String) {
    let owner = common::register_account(router, "Owner", "password123").await;
    let (status, _, guild) = common::send(
        router,
        Method::POST,
        "/guilds",
        Some(&owner),
        Some(&json!({ "name": "Guild" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let gid = guild["id"].as_str().unwrap().to_string();

    let (status, _, detail) = common::send(
        router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cid = detail["channels"][0]["id"].as_str().unwrap().to_string();

    let alice = common::register_account(router, "Alice", "password123").await;
    let (status, _, _) = common::send(
        router,
        Method::POST,
        &format!("/guilds/{gid}/members"),
        Some(&owner),
        Some(&json!({ "username": "Alice" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    (owner, alice, cid)
}

/// Ping typing in `cid` carrying `draft`, asserting the 204.
#[cfg(feature = "ssr")]
async fn ping_with_draft(router: &axum::Router, cookie: &str, cid: &str, draft: &str) {
    let (status, _, body) = common::send(
        router,
        Method::POST,
        &format!("/channels/{cid}/typing"),
        Some(cookie),
        Some(&json!({ "draft": draft })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "typing ping with draft must 204: {body:?}"
    );
}

/// GET the channel's typing drafts as `cookie`, asserting 200 and returning
/// the parsed array.
#[cfg(feature = "ssr")]
async fn get_drafts(router: &axum::Router, cookie: &str, cid: &str) -> Vec<serde_json::Value> {
    let (status, _, body) = common::send(
        router,
        Method::GET,
        &format!("/channels/{cid}/typing-drafts"),
        Some(cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "typing-drafts must 200: {body:?}");
    body.as_array()
        .unwrap_or_else(|| panic!("typing-drafts returns a JSON array, got {body:?}"))
        .clone()
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn posted_draft_is_readable_by_another_member_and_never_by_its_author() {
    let a = common::arena().await;
    let (owner, alice, cid) = owner_and_member(&a.router).await;

    ping_with_draft(&a.router, &alice, &cid, "the dragon wakes…").await;

    // Another member sees Alice's draft with her resolved display name.
    let drafts = get_drafts(&a.router, &owner, &cid).await;
    assert_eq!(drafts.len(), 1, "exactly one live draft: {drafts:?}");
    assert_eq!(drafts[0]["display_name"], "Alice");
    assert_eq!(drafts[0]["draft"], "the dragon wakes…");
    assert!(
        drafts[0]["account_id"].is_string(),
        "entry carries the author's account id"
    );

    // The author NEVER sees their own ghost.
    let own = get_drafts(&a.router, &alice, &cid).await;
    assert!(own.is_empty(), "own draft must be excluded: {own:?}");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn typing_drafts_returns_privacy_404_with_identical_body_for_non_members() {
    let a = common::arena().await;
    let (_owner, _alice, cid) = owner_and_member(&a.router).await;
    let mallory = common::register_account(&a.router, "Mallory", "password123").await;

    // Non-member on a real channel.
    let (status_real, _, body_real) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/typing-drafts"),
        Some(&mallory),
        None,
    )
    .await;
    // Any caller on an unknown channel.
    let (status_unknown, _, body_unknown) = common::send(
        &a.router,
        Method::GET,
        "/channels/nosuchchannel/typing-drafts",
        Some(&mallory),
        None,
    )
    .await;
    // The canonical privacy-404 every other channel handler emits.
    let (status_msgs, _, body_msgs) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages"),
        Some(&mallory),
        None,
    )
    .await;

    assert_eq!(status_real, StatusCode::NOT_FOUND);
    assert_eq!(status_unknown, StatusCode::NOT_FOUND);
    assert_eq!(status_msgs, StatusCode::NOT_FOUND);
    assert_eq!(
        body_real, body_unknown,
        "non-member and unknown-channel bodies must be indistinguishable"
    );
    assert_eq!(
        body_real, body_msgs,
        "typing-drafts privacy-404 body must be byte-identical to the messages handler's"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn expired_draft_is_pruned_after_the_injected_ttl() {
    // 50ms TTL injected at AppState construction — no 8s sleeps in tests.
    let a = common::arena_with_draft_ttl(std::time::Duration::from_millis(50)).await;
    let (owner, alice, cid) = owner_and_member(&a.router).await;

    ping_with_draft(&a.router, &alice, &cid, "fading whisper").await;
    let drafts = get_drafts(&a.router, &owner, &cid).await;
    assert_eq!(drafts.len(), 1, "draft live inside the TTL window");

    tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    let drafts = get_drafts(&a.router, &owner, &cid).await;
    assert!(
        drafts.is_empty(),
        "draft must prune after the TTL: {drafts:?}"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn draft_is_gone_after_the_author_sends_the_message() {
    let a = common::arena().await;
    let (owner, alice, cid) = owner_and_member(&a.router).await;

    ping_with_draft(&a.router, &alice, &cid, "almost done typing this").await;
    assert_eq!(get_drafts(&a.router, &owner, &cid).await.len(), 1);

    let (status, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&alice),
        Some(&json!({ "body": "almost done typing this" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let drafts = get_drafts(&a.router, &owner, &cid).await;
    assert!(
        drafts.is_empty(),
        "send must clear the author's draft so no ghost lingers beside the real message: {drafts:?}"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn draft_is_gone_after_the_author_rolls() {
    let a = common::arena().await;
    let (owner, alice, cid) = owner_and_member(&a.router).await;

    ping_with_draft(&a.router, &alice, &cid, "/roll coin").await;
    assert_eq!(get_drafts(&a.router, &owner, &cid).await.len(), 1);

    let (status, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/roll"),
        Some(&alice),
        Some(&json!({ "expr": "coin" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let drafts = get_drafts(&a.router, &owner, &cid).await;
    assert!(
        drafts.is_empty(),
        "a roll replaces the composed text — it must clear the draft too: {drafts:?}"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn bare_typing_ping_still_succeeds_and_clears_any_stored_draft() {
    let a = common::arena().await;
    let (owner, alice, cid) = owner_and_member(&a.router).await;

    ping_with_draft(&a.router, &alice, &cid, "now you see me").await;
    assert_eq!(get_drafts(&a.router, &owner, &cid).await.len(), 1);

    // The pre-Ghost-Quill bare ping (no body at all) must keep working
    // unchanged — and, per the documented absent-draft semantics, it clears
    // the stored draft (a sender toggling the pref OFF stops ghosting at the
    // very next ping).
    let (status, _, body) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/typing"),
        Some(&alice),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "bare ping must keep working: {body:?}"
    );

    let drafts = get_drafts(&a.router, &owner, &cid).await;
    assert!(
        drafts.is_empty(),
        "a bare ping clears the stored draft: {drafts:?}"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn empty_string_draft_on_ping_clears_the_stored_entry() {
    let a = common::arena().await;
    let (owner, alice, cid) = owner_and_member(&a.router).await;

    ping_with_draft(&a.router, &alice, &cid, "soon to vanish").await;
    assert_eq!(get_drafts(&a.router, &owner, &cid).await.len(), 1);

    // `draft: ""` (sender deleted everything / pref just toggled off mid-type)
    // clears the entry — same semantics as an absent field.
    ping_with_draft(&a.router, &alice, &cid, "").await;
    let drafts = get_drafts(&a.router, &owner, &cid).await;
    assert!(
        drafts.is_empty(),
        "an empty draft clears the stored entry: {drafts:?}"
    );
}

/// Review M-01 (M4 whisper invariant): a draft composed with the whisper
/// effect ARMED is the exact spoiler the hidden-until-tapped feature
/// protects — it must never be served in plaintext to the very audience the
/// sent whisper will be veiled from. The server masks it to the SAME fixed
/// `(whisper)` placeholder as the reply-quote snippet (`reading.rs`
/// MSG_PROJECTION) and the push payload (`push.rs` notification_body).
#[cfg(feature = "ssr")]
#[tokio::test]
async fn whisper_armed_draft_is_masked_to_the_fixed_placeholder_for_other_members() {
    let a = common::arena().await;
    let (owner, alice, cid) = owner_and_member(&a.router).await;

    let (status, _, body) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/typing"),
        Some(&alice),
        Some(&json!({ "draft": "the hidden secret", "effect": "whisper" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "whisper-armed ping must still 204: {body:?}"
    );

    let drafts = get_drafts(&a.router, &owner, &cid).await;
    assert_eq!(drafts.len(), 1, "the ghost row itself survives: {drafts:?}");
    let served = drafts[0]["draft"].as_str().unwrap();
    assert!(
        !served.contains("hidden secret"),
        "a whisper-armed draft must never leave the server in plaintext, got {served:?}"
    );
    assert_eq!(
        served, "(whisper)",
        "masked with the fixed placeholder every other surface uses"
    );
}

/// Backwards-compat half of M-01: the `effect` field is OPTIONAL and only
/// `whisper` masks — an absent effect (today's client) and the non-secret
/// effects (`shout`/`spell`) keep the documented plaintext behavior.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn non_whisper_and_absent_effects_keep_the_draft_plaintext() {
    let a = common::arena().await;
    let (owner, alice, cid) = owner_and_member(&a.router).await;

    // Shout-armed: not a spoiler, served verbatim.
    let (status, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/typing"),
        Some(&alice),
        Some(&json!({ "draft": "SHOUTED SOON", "effect": "shout" })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let drafts = get_drafts(&a.router, &owner, &cid).await;
    assert_eq!(drafts.len(), 1);
    assert_eq!(
        drafts[0]["draft"], "SHOUTED SOON",
        "a shout-armed draft is not a secret and passes through"
    );

    // Absent effect (the pre-M-01 wire shape): unchanged plaintext behavior.
    ping_with_draft(&a.router, &alice, &cid, "no effect armed").await;
    let drafts = get_drafts(&a.router, &owner, &cid).await;
    assert_eq!(drafts.len(), 1);
    assert_eq!(
        drafts[0]["draft"], "no effect armed",
        "an effect-less ping keeps today's behavior exactly"
    );
}

/// Review M-02: EDIT is the one message mutation that replaces the composed
/// text — it must clear the author's stored draft on success, in parity with
/// clear-on-send (`posting.rs`) and clear-on-roll (`rolling.rs`), so no stale
/// ghost row lingers beside the just-edited message for up to the TTL.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn draft_is_gone_after_the_author_edits_a_message() {
    let a = common::arena().await;
    let (owner, alice, cid) = owner_and_member(&a.router).await;

    // Alice posts a message she will then edit.
    let (status, _, body) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&alice),
        Some(&json!({ "body": "the original wording" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let mid = body["id"].as_str().unwrap().to_string();

    // Mid-edit the composer pings its content as a draft (today's client does
    // exactly this — the server must clear on the edit landing regardless of
    // any future client-side guard).
    ping_with_draft(&a.router, &alice, &cid, "the original wording, but better").await;
    assert_eq!(get_drafts(&a.router, &owner, &cid).await.len(), 1);

    let (status, _, body) = common::send(
        &a.router,
        Method::PATCH,
        &format!("/channels/{cid}/messages/{mid}"),
        Some(&alice),
        Some(&json!({ "body": "the edited wording" })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "edit must land: {body:?}");

    let drafts = get_drafts(&a.router, &owner, &cid).await;
    assert!(
        drafts.is_empty(),
        "a landed edit must clear the author's draft so no stale ghost lingers \
         beside the edited message: {drafts:?}"
    );
}

/// Review M-15: the `*chan == cid` clause in `typing_drafts` is the ONLY
/// thing standing between a member of an unrelated guild and every draft on
/// the instance — the membership gate only covers the REQUESTED channel, and
/// the drafts map is process-global. Bob passes channel_access on HIS OWN
/// channel, so this read reaches the map; the scoping clause must hide
/// Alice's guild-1 draft from him.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn draft_in_one_channel_never_appears_in_another_channels_fetch() {
    let a = common::arena().await;
    let (owner, alice, cid) = owner_and_member(&a.router).await;

    // Bob owns an UNRELATED guild with its own default channel — he is a
    // member nowhere near guild 1.
    let bob = common::register_account(&a.router, "Bob", "password123").await;
    let (status, _, guild2) = common::send(
        &a.router,
        Method::POST,
        "/guilds",
        Some(&bob),
        Some(&json!({ "name": "Elsewhere" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let gid2 = guild2["id"].as_str().unwrap().to_string();
    let (status, _, detail2) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid2}"),
        Some(&bob),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cid2 = detail2["channels"][0]["id"].as_str().unwrap().to_string();

    ping_with_draft(&a.router, &alice, &cid, "secret plan for guild one").await;

    // Aliveness proof: the draft IS in the map (a guild-1 member sees it)…
    assert_eq!(
        get_drafts(&a.router, &owner, &cid).await.len(),
        1,
        "the draft must be live for guild-1 members"
    );
    // …and the scoping clause alone keeps it out of Bob's fetch.
    let foreign = get_drafts(&a.router, &bob, &cid2).await;
    assert!(
        foreign.is_empty(),
        "a guild-1 draft must never surface through an unrelated guild's channel: {foreign:?}"
    );
}

/// Same-guild half of M-15: the scoping is per-CHANNEL, not per-guild — a
/// draft composed in channel 1 must not surface in channel 2's fetch even
/// when the caller is a legitimate member of both.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn draft_is_scoped_to_its_channel_even_within_the_same_guild() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let (status, _, guild) = common::send(
        &a.router,
        Method::POST,
        "/guilds",
        Some(&owner),
        Some(&json!({ "name": "Guild" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let gid = guild["id"].as_str().unwrap().to_string();
    let (status, _, detail) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cid1 = detail["channels"][0]["id"].as_str().unwrap().to_string();

    // A second text channel in the SAME guild.
    let (status, _, second) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/channels"),
        Some(&owner),
        Some(&json!({ "name": "two", "kind": "text" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let cid2 = second["id"].as_str().unwrap().to_string();

    let alice = common::register_account(&a.router, "Alice", "password123").await;
    let (status, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/members"),
        Some(&owner),
        Some(&json!({ "username": "Alice" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    ping_with_draft(&a.router, &alice, &cid1, "channel-one thoughts").await;

    assert_eq!(
        get_drafts(&a.router, &owner, &cid1).await.len(),
        1,
        "the draft is live in its own channel"
    );
    let other = get_drafts(&a.router, &owner, &cid2).await;
    assert!(
        other.is_empty(),
        "a channel-1 draft must not surface in channel 2 of the same guild: {other:?}"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn overlong_draft_is_truncated_to_the_cap_on_a_char_boundary() {
    let a = common::arena().await;
    let (owner, alice, cid) = owner_and_member(&a.router).await;

    // 2100 two-byte chars: a byte-indexed truncation would panic or split a
    // char; the documented behavior is TRUNCATE to 2000 CHARS (never reject —
    // a mid-typing ping must not start failing because the composer grew).
    let long = "ä".repeat(2100);
    ping_with_draft(&a.router, &alice, &cid, &long).await;

    let drafts = get_drafts(&a.router, &owner, &cid).await;
    assert_eq!(drafts.len(), 1);
    let stored = drafts[0]["draft"].as_str().unwrap();
    assert_eq!(
        stored.chars().count(),
        2000,
        "draft truncates to the 2000-char cap"
    );
    assert_eq!(
        stored,
        &"ä".repeat(2000),
        "truncation lands on a char boundary"
    );
}
