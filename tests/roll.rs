//! W4/T6 Fate Engine integration tests: `POST /channels/{cid}/roll` —
//! server-authoritative dice. The server parses a constrained grammar
//! (`NdM(+|-K)?`, bare `dM` = `1dM`, `coin`, `oracle`), rolls with its own
//! RNG, and persists the formatted result as a `kind='roll'` message — so a
//! client can never forge an outcome. Rolls are persona-aware like a normal
//! send (same `can_edit_persona` double-check) and FULLY IMMUTABLE: the
//! author's own edit and delete are explicit 403s (cheating-proof — without
//! that guard the roller could PATCH the body into a forged result or delete
//! an unfavorable roll).

mod common;

#[cfg(feature = "ssr")]
use axum::http::{Method, StatusCode};
#[cfg(feature = "ssr")]
use serde_json::json;

/// Register an owner, create a guild, and return
/// `(owner_cookie, guild_id, default_text_channel_id)`.
#[cfg(feature = "ssr")]
async fn owner_with_text_channel(router: &axum::Router) -> (String, String, String) {
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
    (owner, gid, cid)
}

/// POST /channels/{cid}/roll with the given expression (no persona).
#[cfg(feature = "ssr")]
async fn roll(
    router: &axum::Router,
    cookie: &str,
    cid: &str,
    expr: &str,
) -> (StatusCode, serde_json::Value) {
    let (status, _, body) = common::send(
        router,
        Method::POST,
        &format!("/channels/{cid}/roll"),
        Some(cookie),
        Some(&json!({ "expr": expr })),
    )
    .await;
    (status, body)
}

/// GET the channel's messages as a JSON array.
#[cfg(feature = "ssr")]
async fn list(router: &axum::Router, cookie: &str, cid: &str) -> Vec<serde_json::Value> {
    let (status, _, body) = common::send(
        router,
        Method::GET,
        &format!("/channels/{cid}/messages"),
        Some(cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    body["messages"].as_array().unwrap().clone()
}

/// Parse a formatted dice body `"<expr> → [r1,r2,…](±K)? = T"` into
/// `(rolls, total)`. Panics on an unexpected shape (the format IS the
/// contract — clients render it verbatim).
#[cfg(feature = "ssr")]
fn parse_dice_body(body: &str, expected_prefix: &str) -> (Vec<i64>, i64) {
    let rest = body
        .strip_prefix(expected_prefix)
        .unwrap_or_else(|| panic!("body {body:?} must start with {expected_prefix:?}"));
    let open = rest.find('[').expect("rolls open bracket");
    let close = rest.find(']').expect("rolls close bracket");
    let rolls: Vec<i64> = rest[open + 1..close]
        .split(',')
        .map(|r| r.trim().parse().expect("die value"))
        .collect();
    let total: i64 = rest
        .rsplit('=')
        .next()
        .expect("total after =")
        .trim()
        .parse()
        .expect("total parses");
    (rolls, total)
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn valid_dice_expr_creates_roll_message_with_result_in_range() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;

    let (status, body) = roll(&a.router, &owner, &cid, "2d20+3").await;
    assert_eq!(status, StatusCode::CREATED, "valid roll must 201: {body:?}");
    assert!(body["id"].is_string(), "response carries the created id");

    let msgs = list(&a.router, &owner, &cid).await;
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["kind"], "roll", "persisted as kind='roll'");
    let roll_body = msgs[0]["body"].as_str().unwrap();
    let (rolls, total) = parse_dice_body(roll_body, "2d20+3 → ");
    assert_eq!(rolls.len(), 2, "two dice rolled: {roll_body}");
    for r in &rolls {
        assert!((1..=20).contains(r), "each die in 1..=20: {roll_body}");
    }
    assert_eq!(
        total,
        rolls.iter().sum::<i64>() + 3,
        "total = sum + modifier: {roll_body}"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn bare_dm_expression_rolls_a_single_die() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;

    let (status, _) = roll(&a.router, &owner, &cid, "d6").await;
    assert_eq!(status, StatusCode::CREATED, "bare dM is accepted as 1dM");

    let msgs = list(&a.router, &owner, &cid).await;
    let roll_body = msgs[0]["body"].as_str().unwrap();
    let (rolls, total) = parse_dice_body(roll_body, "1d6 → ");
    assert_eq!(rolls.len(), 1, "one die: {roll_body}");
    assert!((1..=6).contains(&total), "total in 1..=6: {roll_body}");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn coin_and_oracle_answer_from_their_documented_sets() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;

    let (status, _) = roll(&a.router, &owner, &cid, "coin").await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _) = roll(&a.router, &owner, &cid, "oracle").await;
    assert_eq!(status, StatusCode::CREATED);

    let msgs = list(&a.router, &owner, &cid).await;
    assert_eq!(msgs.len(), 2);
    let coin_body = msgs[0]["body"].as_str().unwrap();
    assert!(
        coin_body == "coin → Heads" || coin_body == "coin → Tails",
        "coin lands Heads or Tails: {coin_body}"
    );
    let oracle_body = msgs[1]["body"].as_str().unwrap();
    let answer = oracle_body
        .strip_prefix("oracle → ")
        .unwrap_or_else(|| panic!("oracle body shape: {oracle_body}"));
    assert!(
        authlyn_interactive::server::messages::ORACLE_ANSWERS.contains(&answer),
        "oracle answers from the documented set: {oracle_body}"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn invalid_expressions_are_400_and_persist_nothing() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;

    for expr in [
        "", "banana", "2x6", "0d6", "2d0", "2d1", "1dd6", "1d6x", "1d6+", "+3", "d", "2d", "-1d6",
        "2d6+-1",
    ] {
        let (status, body) = roll(&a.router, &owner, &cid, expr).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "expr {expr:?} must be rejected, got {status}: {body:?}"
        );
    }
    let msgs = list(&a.router, &owner, &cid).await;
    assert!(msgs.is_empty(), "rejected rolls persist no message");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn dice_bounds_hold_at_and_past_the_documented_limits() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;

    // Past the bounds: rejected.
    for expr in ["101d6", "1d1001", "1d6+1001", "1d6-1001"] {
        let (status, _) = roll(&a.router, &owner, &cid, expr).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "expr {expr:?} exceeds the bounds and must 400"
        );
    }
    // At the bounds: accepted.
    for expr in ["100d6", "1d1000", "1d6+1000", "1d6-1000"] {
        let (status, body) = roll(&a.router, &owner, &cid, expr).await;
        assert_eq!(
            status,
            StatusCode::CREATED,
            "expr {expr:?} sits at the bounds and must be accepted: {body:?}"
        );
    }
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn nonmember_roll_is_privacy_404() {
    let a = common::arena().await;
    let (_owner, _gid, cid) = owner_with_text_channel(&a.router).await;
    let stranger = common::register_account(&a.router, "Stranger", "password123").await;

    let (status, body) = roll(&a.router, &stranger, &cid, "1d6").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "non-member must privacy-404");
    assert_eq!(
        body["error"], "channel not found",
        "identical body to the unknown-channel case — never reveal existence"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn rolling_into_a_lorebook_channel_is_400() {
    let a = common::arena().await;
    let (owner, gid, _cid) = owner_with_text_channel(&a.router).await;
    let (status, _, ch) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/channels"),
        Some(&owner),
        Some(&json!({ "name": "lore", "kind": "lorebook" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let lore_cid = ch["id"].as_str().unwrap().to_string();

    let (status, _) = roll(&a.router, &owner, &lore_cid, "1d6").await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "rolls land in text channels only (mirrors post_message)"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn roll_with_own_persona_snapshots_identity_like_a_normal_send() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;
    let (status, _, p) = common::send(
        &a.router,
        Method::POST,
        "/personas",
        Some(&owner),
        Some(&json!({ "name": "Hero", "description": "d20 main character" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let pid = p["id"].as_str().unwrap().to_string();

    let (status, _, body) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/roll"),
        Some(&owner),
        Some(&json!({ "expr": "1d20", "persona": pid })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body:?}");

    let msgs = list(&a.router, &owner, &cid).await;
    assert_eq!(msgs[0]["persona_id"], pid, "roll authored as the persona");
    assert_eq!(
        msgs[0]["persona_name"], "Hero",
        "persona identity snapshotted onto the roll row"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn roll_with_unowned_persona_is_rejected_as_attribution() {
    // Mirror of personas.rs's revoked-editor send test: a persona the caller
    // cannot edit is REJECTED as attribution (the re-derived can_edit_persona
    // gate), so the roll lands as the bare account — never stamped foreign.
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;
    let other = common::register_account(&a.router, "Other", "password123").await;
    let (status, _, p) = common::send(
        &a.router,
        Method::POST,
        "/personas",
        Some(&other),
        Some(&json!({ "name": "NotYours", "description": "" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let foreign = p["id"].as_str().unwrap().to_string();

    let (status, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/roll"),
        Some(&owner),
        Some(&json!({ "expr": "1d6", "persona": foreign })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let msgs = list(&a.router, &owner, &cid).await;
    assert!(
        msgs[0]["persona_id"].is_null(),
        "a persona the caller cannot edit must not be stamped on a roll"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn editing_own_roll_is_403_and_the_body_is_unchanged() {
    // 6.2b (audit critical): roll immutability is NOT an authorship
    // side-effect — the roller IS the author, so without an explicit kind
    // guard they could PATCH the body into a forged result.
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;
    let (status, _) = roll(&a.router, &owner, &cid, "1d6").await;
    assert_eq!(status, StatusCode::CREATED);
    let msgs = list(&a.router, &owner, &cid).await;
    let mid = msgs[0]["id"].as_str().unwrap().to_string();
    let original_body = msgs[0]["body"].as_str().unwrap().to_string();

    let (status, _, _) = common::send(
        &a.router,
        Method::PATCH,
        &format!("/channels/{cid}/messages/{mid}"),
        Some(&owner),
        Some(&json!({ "body": "1d6 → [6] = 6" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "the author must NOT be able to edit their own roll"
    );

    let msgs = list(&a.router, &owner, &cid).await;
    assert_eq!(
        msgs[0]["body"].as_str().unwrap(),
        original_body,
        "the roll body must be unchanged after the edit attempt"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn deleting_own_roll_is_403_and_the_roll_survives() {
    // 6.2b (audit critical): without the kind guard the roller could delete
    // an unfavorable roll. Rolls are FULLY immutable — no edit, no delete.
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;
    let (status, _) = roll(&a.router, &owner, &cid, "1d6").await;
    assert_eq!(status, StatusCode::CREATED);
    let msgs = list(&a.router, &owner, &cid).await;
    let mid = msgs[0]["id"].as_str().unwrap().to_string();
    let original_body = msgs[0]["body"].as_str().unwrap().to_string();

    let (status, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/channels/{cid}/messages/{mid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "the author must NOT be able to delete their own roll"
    );

    let msgs = list(&a.router, &owner, &cid).await;
    assert_eq!(msgs.len(), 1, "the roll must survive the delete attempt");
    assert_eq!(msgs[0]["body"].as_str().unwrap(), original_body);
}
