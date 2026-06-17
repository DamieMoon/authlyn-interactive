//! Step-2 integration tests: guild create/list/detail, the membership
//! privacy-404, owner-gated channel creation, and the concurrent-invite race
//! against the `guild_member_pair` UNIQUE index.

mod common;

#[cfg(feature = "ssr")]
use axum::body::{to_bytes, Body};
#[cfg(feature = "ssr")]
use axum::http::{header, Method, Request, StatusCode};
#[cfg(feature = "ssr")]
use serde_json::json;
#[cfg(feature = "ssr")]
use tower::ServiceExt;

#[cfg(feature = "ssr")]
async fn create_guild(router: &axum::Router, cookie: &str, name: &str) -> String {
    let (status, _, body) = common::send(
        router,
        Method::POST,
        "/guilds",
        Some(cookie),
        Some(&json!({ "name": name })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create guild: {body:?}");
    body["id"].as_str().expect("guild id").to_string()
}

/// A valid 2x1 RGB PNG (correct chunk CRCs) so the media store accepts it as an
/// inline image — same fixture as `tests/media.rs`.
#[cfg(feature = "ssr")]
const TINY_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // signature
    0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, // IHDR length + type
    0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x01, // 2x1
    0x08, 0x02, 0x00, 0x00, 0x00, 0x7B, 0x40, 0xE8, // 8-bit RGB + CRC
    0xDD, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, // IDAT length + type
    0x54, 0x78, 0xDA, 0x63, 0xF8, 0xCF, 0xC0, 0x00, // zlib stream
    0x44, 0x00, 0x08, 0xFE, 0x01, 0xFF, 0x19, 0xC0, // ...
    0x6B, 0xE7, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, // IEND
    0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

/// Upload a blob via multipart `POST /media`, returning the new media id.
/// Mirrors `tests/media.rs::upload_image`.
#[cfg(feature = "ssr")]
async fn upload_image(router: &axum::Router, cookie: &str, mime: &str, data: &[u8]) -> String {
    let boundary = "Xbnd";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"img\"\r\n\
         Content-Type: {mime}\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(data);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let req = Request::builder()
        .method(Method::POST)
        .uri("/media")
        .header(header::COOKIE, cookie)
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();
    let res = router.clone().oneshot(req).await.expect("oneshot");
    assert_eq!(res.status(), StatusCode::CREATED, "media upload should 201");
    let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    v["id"].as_str().unwrap().to_string()
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn guild_icon_set_round_trips_and_is_manager_gated() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let gid = create_guild(&a.router, &owner, "Iconic").await;
    let mid = upload_image(&a.router, &owner, "image/png", TINY_PNG).await;

    // Owner sets the icon → 204; summary + detail then carry icon_id == mid.
    let (status, _, _) = common::send(
        &a.router,
        Method::PUT,
        &format!("/guilds/{gid}/icon"),
        Some(&owner),
        Some(&json!({ "media_id": mid })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "owner sets icon");

    let (_, _, detail) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(
        detail["icon_id"].as_str(),
        Some(mid.as_str()),
        "detail icon_id"
    );

    let (_, _, list) = common::send(&a.router, Method::GET, "/guilds", Some(&owner), None).await;
    assert_eq!(
        list["guilds"][0]["icon_id"].as_str(),
        Some(mid.as_str()),
        "summary icon_id"
    );

    // Unknown media id → 404 (media-not-found; owner passes the manager gate).
    let (status, _, _) = common::send(
        &a.router,
        Method::PUT,
        &format!("/guilds/{gid}/icon"),
        Some(&owner),
        Some(&json!({ "media_id": "deadbeefdeadbeefdeadbeefdeadbeef" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "unknown media id 404s");

    // A plain member cannot set the icon (403, before the media check).
    let member = common::register_account(&a.router, "Member", "password123").await;
    let (status, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/members"),
        Some(&owner),
        Some(&json!({ "username": "Member" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let m_mid = upload_image(&a.router, &member, "image/png", TINY_PNG).await;
    let (status, _, _) = common::send(
        &a.router,
        Method::PUT,
        &format!("/guilds/{gid}/icon"),
        Some(&member),
        Some(&json!({ "media_id": m_mid })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "plain member cannot set icon"
    );

    // A non-member gets the privacy 404.
    let outsider = common::register_account(&a.router, "Outsider", "password123").await;
    let o_mid = upload_image(&a.router, &outsider, "image/png", TINY_PNG).await;
    let (status, _, _) = common::send(
        &a.router,
        Method::PUT,
        &format!("/guilds/{gid}/icon"),
        Some(&outsider),
        Some(&json!({ "media_id": o_mid })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "non-member gets privacy 404");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn create_lists_and_details_with_default_channel() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let gid = create_guild(&a.router, &owner, "My Guild").await;

    let (status, _, body) =
        common::send(&a.router, Method::GET, "/guilds", Some(&owner), None).await;
    assert_eq!(status, StatusCode::OK);
    let guilds = body["guilds"].as_array().unwrap();
    assert_eq!(guilds.len(), 1);
    assert_eq!(guilds[0]["name"], "My Guild");

    let (status, _, body) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "My Guild");
    let channels = body["channels"].as_array().unwrap();
    assert_eq!(channels.len(), 1, "a fresh guild has one default channel");
    assert_eq!(channels[0]["name"], "general");
    assert_eq!(channels[0]["kind"], "text");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn fresh_guild_has_null_icon_id_in_summary_and_detail() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let gid = create_guild(&a.router, &owner, "Iconless").await;

    // Summary (GET /guilds): a guild with no icon projects icon_id = null
    // (the NONE arm of the meta::id(guild.icon) projection).
    let (status, _, body) =
        common::send(&a.router, Method::GET, "/guilds", Some(&owner), None).await;
    assert_eq!(status, StatusCode::OK);
    let g = &body["guilds"][0];
    assert!(
        g["icon_id"].is_null(),
        "fresh guild summary icon_id is null: {g:?}"
    );

    // Detail (GET /guilds/{id}): same NONE projection.
    let (status, _, body) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body["icon_id"].is_null(),
        "fresh guild detail icon_id is null: {body:?}"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn nonmember_get_guild_is_404() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let gid = create_guild(&a.router, &owner, "Secret").await;

    let outsider = common::register_account(&a.router, "Outsider", "password123").await;
    let (status, _, _) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&outsider),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "non-members get a privacy 404"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn channel_create_is_owner_gated() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let gid = create_guild(&a.router, &owner, "Guild").await;

    let member = common::register_account(&a.router, "Member", "password123").await;
    let (status, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/members"),
        Some(&owner),
        Some(&json!({ "username": "Member" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // A plain member cannot create channels.
    let (status, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/channels"),
        Some(&member),
        Some(&json!({ "name": "lore", "kind": "lorebook" })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // The owner can.
    let (status, _, body) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/channels"),
        Some(&owner),
        Some(&json!({ "name": "lore", "kind": "lorebook" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["kind"], "lorebook");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn invite_unknown_user_is_404() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let gid = create_guild(&a.router, &owner, "Guild").await;

    let (status, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/members"),
        Some(&owner),
        Some(&json!({ "username": "ghost" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn invite_grants_invitee_access() {
    // The contract the invite UI relies on: once invited, the user sees the
    // guild in their list and can open it (membership == access).
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let gid = create_guild(&a.router, &owner, "My Guild").await;
    let alice = common::register_account(&a.router, "Alice", "password123").await;

    // Before the invite, Alice can't see the guild.
    let (status, _, body) =
        common::send(&a.router, Method::GET, "/guilds", Some(&alice), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["guilds"].as_array().unwrap().len(), 0);

    let (status, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/members"),
        Some(&owner),
        Some(&json!({ "username": "Alice" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // After the invite, the guild appears in Alice's list and she can open it.
    let (status, _, body) =
        common::send(&a.router, Method::GET, "/guilds", Some(&alice), None).await;
    assert_eq!(status, StatusCode::OK);
    let guilds = body["guilds"].as_array().unwrap();
    assert_eq!(guilds.len(), 1);
    assert_eq!(guilds[0]["name"], "My Guild");

    let (status, _, _) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&alice),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "invited member can open the guild");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn rename_guild_and_channel_is_manager_gated() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let gid = create_guild(&a.router, &owner, "Old Name").await;

    // Grab the default channel id.
    let (_, _, body) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&owner),
        None,
    )
    .await;
    let cid = body["channels"][0]["id"].as_str().unwrap().to_string();

    // Owner renames the guild, then the channel.
    let (st, _, _) = common::send(
        &a.router,
        Method::PATCH,
        &format!("/guilds/{gid}"),
        Some(&owner),
        Some(&json!({ "name": "New Name" })),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    let (st, _, _) = common::send(
        &a.router,
        Method::PATCH,
        &format!("/guilds/{gid}/channels/{cid}"),
        Some(&owner),
        Some(&json!({ "name": "renamed" })),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    // Both renames are reflected.
    let (_, _, body) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(body["name"], "New Name");
    assert_eq!(body["channels"][0]["name"], "renamed");

    // A plain member cannot rename the guild or its channels.
    let member = common::register_account(&a.router, "Member", "password123").await;
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/members"),
        Some(&owner),
        Some(&json!({ "username": "Member" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    let (st, _, _) = common::send(
        &a.router,
        Method::PATCH,
        &format!("/guilds/{gid}"),
        Some(&member),
        Some(&json!({ "name": "hax" })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    let (st, _, _) = common::send(
        &a.router,
        Method::PATCH,
        &format!("/guilds/{gid}/channels/{cid}"),
        Some(&member),
        Some(&json!({ "name": "hax" })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn concurrent_invite_yields_one_member_row() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let gid = create_guild(&a.router, &owner, "Guild").await;

    // Register the target directly so we can grab its account id for the
    // post-condition DB assertion.
    let (status, _, body) = common::send(
        &a.router,
        Method::POST,
        "/auth/register",
        None,
        Some(&json!({ "username": "Target", "password": "password123" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let target_id = body["account_id"].as_str().unwrap().to_string();

    let url = format!("/guilds/{gid}/members");
    let invite = json!({ "username": "Target" });
    let (r1, r2) = tokio::join!(
        common::send(&a.router, Method::POST, &url, Some(&owner), Some(&invite)),
        common::send(&a.router, Method::POST, &url, Some(&owner), Some(&invite)),
    );
    let statuses = [r1.0, r2.0];
    assert!(
        statuses.contains(&StatusCode::CREATED),
        "exactly one invite should 201: {statuses:?}"
    );
    assert!(
        statuses.contains(&StatusCode::CONFLICT),
        "the racing invite should 409: {statuses:?}"
    );

    // The UNIQUE index must leave exactly one membership row.
    let mut resp =
        a.db.query(
            "SELECT VALUE meta::id(id) FROM guild_member
                WHERE guild = type::record('guild', $gid)
                  AND account = type::record('account', $tid);",
        )
        .bind(("gid", gid.clone()))
        .bind(("tid", target_id.clone()))
        .await
        .unwrap()
        .check()
        .unwrap();
    let ids: Vec<String> = resp.take(0).unwrap();
    assert_eq!(ids.len(), 1, "exactly one guild_member row for the target");
}

/// Register an account, returning `(session_cookie, account_id)`.
#[cfg(feature = "ssr")]
async fn register_with_id(router: &axum::Router, name: &str) -> (String, String) {
    let (status, cookie, body) = common::send(
        router,
        Method::POST,
        "/auth/register",
        None,
        Some(&json!({ "username": name, "password": "password123" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    (
        cookie.unwrap(),
        body["account_id"].as_str().unwrap().to_string(),
    )
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn promoting_a_member_to_admin_lets_them_manage() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let gid = create_guild(&a.router, &owner, "Guild").await;
    let (member, member_id) = register_with_id(&a.router, "Member").await;

    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/members"),
        Some(&owner),
        Some(&json!({ "username": "Member" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    // A plain member can't create channels.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/channels"),
        Some(&member),
        Some(&json!({ "name": "x", "kind": "text" })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // Owner grants admin — the easy path to share control.
    let (st, _, _) = common::send(
        &a.router,
        Method::PUT,
        &format!("/guilds/{gid}/members/{member_id}/role"),
        Some(&owner),
        Some(&json!({ "role": "admin" })),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    // Now the (promoted) admin can manage.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/channels"),
        Some(&member),
        Some(&json!({ "name": "x", "kind": "text" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn plain_member_cannot_grant_admin() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let gid = create_guild(&a.router, &owner, "Guild").await;
    let (member, _) = register_with_id(&a.router, "Member").await;
    let (_, third_id) = register_with_id(&a.router, "Third").await;

    for name in ["Member", "Third"] {
        let (st, _, _) = common::send(
            &a.router,
            Method::POST,
            &format!("/guilds/{gid}/members"),
            Some(&owner),
            Some(&json!({ "username": name })),
        )
        .await;
        assert_eq!(st, StatusCode::CREATED);
    }

    let (st, _, _) = common::send(
        &a.router,
        Method::PUT,
        &format!("/guilds/{gid}/members/{third_id}/role"),
        Some(&member),
        Some(&json!({ "role": "admin" })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn owner_role_cannot_be_changed() {
    let a = common::arena().await;
    let (owner, owner_id) = register_with_id(&a.router, "Owner").await;
    let gid = create_guild(&a.router, &owner, "Guild").await;

    let (st, _, _) = common::send(
        &a.router,
        Method::PUT,
        &format!("/guilds/{gid}/members/{owner_id}/role"),
        Some(&owner),
        Some(&json!({ "role": "member" })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
}

/// L-5: a channel-`position` PATCH persists and reorders the sidebar list.
/// This is the same endpoint the manager's reorder (swap / move-to-bounds)
/// drives — renumbering each moved channel to a fresh index. We create three
/// channels, PATCH their positions into reverse order, and assert the
/// `GET /guilds/{id}` list (ORDER BY position) reflects the new order.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn channel_reorder_persists() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let gid = create_guild(&a.router, &owner, "Guild").await;

    // The default "general" channel is position 0; add two more.
    let mut cids = vec![channel_id_at(&a.router, &owner, &gid, 0).await];
    for name in ["beta", "gamma"] {
        let (st, _, body) = common::send(
            &a.router,
            Method::POST,
            &format!("/guilds/{gid}/channels"),
            Some(&owner),
            Some(&json!({ "name": name, "kind": "text" })),
        )
        .await;
        assert_eq!(st, StatusCode::CREATED);
        cids.push(body["id"].as_str().unwrap().to_string());
    }

    // Reverse the order via position PATCHes (index = new slot).
    for (slot, cid) in cids.iter().rev().enumerate() {
        let (st, _, _) = common::send(
            &a.router,
            Method::PATCH,
            &format!("/guilds/{gid}/channels/{cid}"),
            Some(&owner),
            Some(&json!({ "position": slot as i64 })),
        )
        .await;
        assert_eq!(st, StatusCode::NO_CONTENT);
    }

    let (_, _, body) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&owner),
        None,
    )
    .await;
    let listed: Vec<String> = body["channels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_string())
        .collect();
    let mut expected = cids.clone();
    expected.reverse();
    assert_eq!(listed, expected, "channels are listed in the patched order");
}

/// L-5: the manager's "bring to top" / "bring to bottom" path renumbers the
/// full list so the moved channel lands at index 0 or the last index. We
/// emulate the helper by computing the new full order client-side, PATCHing
/// each changed slot, and asserting the persisted list.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn move_channel_to_top_and_bottom() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let gid = create_guild(&a.router, &owner, "Guild").await;

    let mut cids = vec![channel_id_at(&a.router, &owner, &gid, 0).await];
    for name in ["beta", "gamma"] {
        let (st, _, body) = common::send(
            &a.router,
            Method::POST,
            &format!("/guilds/{gid}/channels"),
            Some(&owner),
            Some(&json!({ "name": name, "kind": "text" })),
        )
        .await;
        assert_eq!(st, StatusCode::CREATED);
        cids.push(body["id"].as_str().unwrap().to_string());
    }
    // cids == [general, beta, gamma] at positions 0,1,2.

    // Bring the last (gamma) to the top → [gamma, general, beta].
    let mut order = cids.clone();
    let moved = order.remove(2);
    order.insert(0, moved);
    persist_channel_order(&a.router, &owner, &gid, &order).await;
    assert_eq!(channel_order(&a.router, &owner, &gid).await, order);

    // Bring the first (gamma) to the bottom → [general, beta, gamma].
    let moved = order.remove(0);
    order.push(moved);
    persist_channel_order(&a.router, &owner, &gid, &order).await;
    assert_eq!(channel_order(&a.router, &owner, &gid).await, order);
    assert_eq!(
        order,
        vec![cids[0].clone(), cids[1].clone(), cids[2].clone()]
    );
}

/// L-5: `PUT /rail/order` persists the caller's personal guild-rail order —
/// the path the manager's server-reorder (drag / move-to-bounds) drives.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn rail_order_persists() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let g1 = create_guild(&a.router, &owner, "One").await;
    let g2 = create_guild(&a.router, &owner, "Two").await;
    let g3 = create_guild(&a.router, &owner, "Three").await;

    // Put the rail in a deliberate, non-creation order.
    let desired = vec![g3.clone(), g1.clone(), g2.clone()];
    let (st, _, _) = common::send(
        &a.router,
        Method::PUT,
        "/rail/order",
        Some(&owner),
        Some(&json!({ "guild_ids": desired })),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    let (_, _, body) = common::send(&a.router, Method::GET, "/guilds", Some(&owner), None).await;
    let listed: Vec<String> = body["guilds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(listed, desired, "the rail is listed in the persisted order");
}

/// L-5 / review F-D1b-3: a soft-deleted guild must reject channel creation —
/// `require_manager` calls `ensure_guild_live`, so a trashed guild collapses
/// to a privacy-404 even for its owner. Guards the soft-delete-immutability
/// invariant for the channel-create path the manager exposes.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn create_channel_in_soft_deleted_guild_is_rejected() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let gid = create_guild(&a.router, &owner, "Doomed").await;

    // Soft-delete the guild (owner-gated).
    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/guilds/{gid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    // Creating a channel in a trashed guild is a privacy-404.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/channels"),
        Some(&owner),
        Some(&json!({ "name": "ghost", "kind": "text" })),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::NOT_FOUND,
        "channel create on a soft-deleted guild must 404"
    );
}

/// The channel id at `pos` in the guild's position-sorted list.
#[cfg(feature = "ssr")]
async fn channel_id_at(router: &axum::Router, cookie: &str, gid: &str, pos: usize) -> String {
    channel_order(router, cookie, gid).await[pos].clone()
}

/// The guild's channel ids in server-listed (position) order.
#[cfg(feature = "ssr")]
async fn channel_order(router: &axum::Router, cookie: &str, gid: &str) -> Vec<String> {
    let (_, _, body) = common::send(
        router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(cookie),
        None,
    )
    .await;
    body["channels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_string())
        .collect()
}

/// PATCH every channel whose desired slot differs from its current index —
/// the move-to-bounds / drag renumber the manager performs client-side.
#[cfg(feature = "ssr")]
async fn persist_channel_order(router: &axum::Router, cookie: &str, gid: &str, order: &[String]) {
    for (slot, cid) in order.iter().enumerate() {
        let (st, _, _) = common::send(
            router,
            Method::PATCH,
            &format!("/guilds/{gid}/channels/{cid}"),
            Some(cookie),
            Some(&json!({ "position": slot as i64 })),
        )
        .await;
        assert_eq!(st, StatusCode::NO_CONTENT);
    }
}
