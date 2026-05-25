//! Step-4 integration tests: persona CRUD owner-scoping, media upload +
//! avatar + gallery (with served MIME), and the per-guild active persona
//! stamping a message (exercising the null-safe persona projection both ways).

mod common;

#[cfg(feature = "ssr")]
use axum::body::{to_bytes, Body};
#[cfg(feature = "ssr")]
use axum::http::{header, Method, Request, StatusCode};
#[cfg(feature = "ssr")]
use serde_json::{json, Value};
#[cfg(feature = "ssr")]
use tower::ServiceExt;

/// Upload a blob via multipart `POST /media`, returning the new media id.
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
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    v["id"].as_str().unwrap().to_string()
}

#[cfg(feature = "ssr")]
async fn create_persona(router: &axum::Router, cookie: &str, name: &str) -> String {
    let (status, _, body) = common::send(
        router,
        Method::POST,
        "/personas",
        Some(cookie),
        Some(&json!({ "name": name, "description": "" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    body["id"].as_str().unwrap().to_string()
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn persona_crud_is_owner_scoped() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;

    let (status, _, body) = common::send(
        &a.router,
        Method::POST,
        "/personas",
        Some(&owner),
        Some(&json!({ "name": "Hero", "description": "brave" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let pid = body["id"].as_str().unwrap().to_string();

    let (status, _, list) =
        common::send(&a.router, Method::GET, "/personas", Some(&owner), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["personas"].as_array().unwrap().len(), 1);

    let (status, _, detail) = common::send(
        &a.router,
        Method::GET,
        &format!("/personas/{pid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["name"], "Hero");
    assert_eq!(detail["description"], "brave");
    assert!(detail["avatar_id"].is_null());
    assert_eq!(detail["gallery"].as_array().unwrap().len(), 0);

    // Another account cannot see it.
    let other = common::register_account(&a.router, "Other", "password123").await;
    let (status, _, _) = common::send(
        &a.router,
        Method::GET,
        &format!("/personas/{pid}"),
        Some(&other),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn patch_persona_is_owner_scoped() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;

    let (_, _, body) = common::send(
        &a.router,
        Method::POST,
        "/personas",
        Some(&owner),
        Some(&json!({ "name": "Hero", "description": "brave" })),
    )
    .await;
    let pid = body["id"].as_str().unwrap().to_string();

    // Owner updates name + description → 204 and the change is observable.
    let (status, _, _) = common::send(
        &a.router,
        Method::PATCH,
        &format!("/personas/{pid}"),
        Some(&owner),
        Some(&json!({ "name": "Heroine", "description": "bolder" })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, list) =
        common::send(&a.router, Method::GET, "/personas", Some(&owner), None).await;
    assert_eq!(status, StatusCode::OK);
    let personas = list["personas"].as_array().unwrap();
    assert_eq!(personas.len(), 1);
    assert_eq!(personas[0]["name"], "Heroine");
    assert_eq!(personas[0]["description"], "bolder");

    // Empty name is rejected.
    let (status, _, _) = common::send(
        &a.router,
        Method::PATCH,
        &format!("/personas/{pid}"),
        Some(&owner),
        Some(&json!({ "name": "  " })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // A different account cannot update it (privacy-404).
    let other = common::register_account(&a.router, "Other", "password123").await;
    let (status, _, _) = common::send(
        &a.router,
        Method::PATCH,
        &format!("/personas/{pid}"),
        Some(&other),
        Some(&json!({ "name": "Hijacked" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // And the owner's persona is untouched.
    let (_, _, list) = common::send(&a.router, Method::GET, "/personas", Some(&owner), None).await;
    assert_eq!(list["personas"][0]["name"], "Heroine");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn avatar_and_gallery_with_served_mime() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let pid = create_persona(&a.router, &owner, "Hero").await;

    let avatar = upload_image(&a.router, &owner, "image/png", b"\x89PNG\r\n\x1a\nfake").await;
    let (status, _, _) = common::send(
        &a.router,
        Method::PUT,
        &format!("/personas/{pid}/avatar"),
        Some(&owner),
        Some(&json!({ "media_id": avatar })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let gallery_media = upload_image(&a.router, &owner, "image/jpeg", b"\xff\xd8\xff fake").await;
    let (status, _, g) = common::send(
        &a.router,
        Method::POST,
        &format!("/personas/{pid}/gallery"),
        Some(&owner),
        Some(&json!({ "media_id": gallery_media })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let image_id = g["id"].as_str().unwrap().to_string();

    let (status, _, detail) = common::send(
        &a.router,
        Method::GET,
        &format!("/personas/{pid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["avatar_id"], avatar);
    let gal = detail["gallery"].as_array().unwrap();
    assert_eq!(gal.len(), 1);
    assert_eq!(gal[0]["media_id"], gallery_media);

    // The avatar serves with its stored MIME so <img> works.
    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("/media/{avatar}"))
        .header(header::COOKIE, &owner)
        .body(Body::empty())
        .unwrap();
    let res = a.router.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers().get(header::CONTENT_TYPE).unwrap(),
        "image/png"
    );

    // Remove the gallery image.
    let (status, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/personas/{pid}/gallery/{image_id}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, _, detail) = common::send(
        &a.router,
        Method::GET,
        &format!("/personas/{pid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(detail["gallery"].as_array().unwrap().len(), 0);
}

/// Create a guild and return `(guild_id, default_text_channel_id)`.
#[cfg(feature = "ssr")]
async fn guild_with_channel(router: &axum::Router, cookie: &str) -> (String, String) {
    let (_, _, g) = common::send(
        router,
        Method::POST,
        "/guilds",
        Some(cookie),
        Some(&json!({ "name": "Guild" })),
    )
    .await;
    let gid = g["id"].as_str().unwrap().to_string();
    let (_, _, d) = common::send(
        router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(cookie),
        None,
    )
    .await;
    let cid = d["channels"][0]["id"].as_str().unwrap().to_string();
    (gid, cid)
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn active_persona_stamps_messages_both_ways() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let (gid, cid) = guild_with_channel(&a.router, &owner).await;
    let pid = create_persona(&a.router, &owner, "Hero").await;

    // Wear the persona.
    let (status, _, _) = common::send(
        &a.router,
        Method::PUT,
        &format!("/guilds/{gid}/active-persona"),
        Some(&owner),
        Some(&json!({ "persona_id": pid })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // A message sent while wearing it is stamped with id + name.
    common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        Some(&json!({ "body": "in character" })),
    )
    .await;

    // Take it off.
    let (status, _, _) = common::send(
        &a.router,
        Method::PUT,
        &format!("/guilds/{gid}/active-persona"),
        Some(&owner),
        Some(&json!({ "persona_id": null })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // A message sent bare has no persona.
    common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        Some(&json!({ "body": "out of character" })),
    )
    .await;

    let (status, _, body) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let msgs = body["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 2);
    // First (in character): persona id + resolved name.
    assert_eq!(msgs[0]["persona_id"], pid);
    assert_eq!(msgs[0]["persona_name"], "Hero");
    // Second (out of character): no persona.
    assert!(msgs[1]["persona_id"].is_null());
    assert!(msgs[1]["persona_name"].is_null());
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn cannot_wear_someone_elses_persona() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let (gid, _cid) = guild_with_channel(&a.router, &owner).await;

    let other = common::register_account(&a.router, "Other", "password123").await;
    let foreign = create_persona(&a.router, &other, "NotYours").await;

    let (status, _, _) = common::send(
        &a.router,
        Method::PUT,
        &format!("/guilds/{gid}/active-persona"),
        Some(&owner),
        Some(&json!({ "persona_id": foreign })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// Key-based shared persona editing (redeem → editor access)
// ---------------------------------------------------------------------------

/// Fetch a persona's share key via the owner's detail view.
#[cfg(feature = "ssr")]
async fn share_key_of(router: &axum::Router, owner_cookie: &str, pid: &str) -> String {
    let (status, _, detail) = common::send(
        router,
        Method::GET,
        &format!("/personas/{pid}"),
        Some(owner_cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    detail["share_key"]
        .as_str()
        .expect("owner detail must carry share_key")
        .to_string()
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn redeem_grants_edit_and_wear_then_revoke() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let editor = common::register_account(&a.router, "Editor", "password123").await;

    let pid = create_persona(&a.router, &owner, "Shared").await;
    let key = share_key_of(&a.router, &owner, &pid).await;
    assert!(!key.is_empty(), "owner should see a non-empty share key");

    // An editor (non-owner) does NOT see the key (None) and gets a 404 before
    // redeeming.
    let (status, _, _) = common::send(
        &a.router,
        Method::GET,
        &format!("/personas/{pid}"),
        Some(&editor),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Redeem the key → editor access.
    let (status, _, _) = common::send(
        &a.router,
        Method::POST,
        "/personas/redeem",
        Some(&editor),
        Some(&json!({ "key": key })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Redeeming again → 409 (already an editor).
    let (status, _, _) = common::send(
        &a.router,
        Method::POST,
        "/personas/redeem",
        Some(&editor),
        Some(&json!({ "key": key })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // The persona now appears in the editor's list with owned=false.
    let (status, _, list) =
        common::send(&a.router, Method::GET, "/personas", Some(&editor), None).await;
    assert_eq!(status, StatusCode::OK);
    let personas = list["personas"].as_array().unwrap();
    assert_eq!(personas.len(), 1);
    assert_eq!(personas[0]["id"], pid.as_str());
    assert_eq!(personas[0]["owned"], false);

    // And in the owner's list with owned=true.
    let (_, _, owner_list) =
        common::send(&a.router, Method::GET, "/personas", Some(&owner), None).await;
    assert_eq!(owner_list["personas"][0]["owned"], true);

    // Editor can PATCH (edit) the persona.
    let (status, _, _) = common::send(
        &a.router,
        Method::PATCH,
        &format!("/personas/{pid}"),
        Some(&editor),
        Some(&json!({ "description": "edited by sharer" })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, _, owner_list) =
        common::send(&a.router, Method::GET, "/personas", Some(&owner), None).await;
    assert_eq!(owner_list["personas"][0]["description"], "edited by sharer");

    // Editor CANNOT delete (owner-only) → 404 (privacy).
    let (status, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/personas/{pid}"),
        Some(&editor),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Editor can wear it in a guild they're a member of.
    let (gid, _cid) = guild_with_channel(&a.router, &editor).await;
    let (status, _, _) = common::send(
        &a.router,
        Method::PUT,
        &format!("/guilds/{gid}/active-persona"),
        Some(&editor),
        Some(&json!({ "persona_id": pid })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Owner sees the editor in the roster, then removes them.
    let (status, _, roster) = common::send(
        &a.router,
        Method::GET,
        &format!("/personas/{pid}/editors"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let editors = roster["editors"].as_array().unwrap();
    assert_eq!(editors.len(), 1);
    let editor_aid = editors[0]["account_id"].as_str().unwrap().to_string();
    assert_eq!(editors[0]["username"], "Editor");

    let (status, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/personas/{pid}/editors/{editor_aid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // After revocation the editor can no longer patch it (privacy-404).
    let (status, _, _) = common::send(
        &a.router,
        Method::PATCH,
        &format!("/personas/{pid}"),
        Some(&editor),
        Some(&json!({ "description": "should fail" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // And it's gone from the editor's list.
    let (_, _, list) = common::send(&a.router, Method::GET, "/personas", Some(&editor), None).await;
    assert_eq!(list["personas"].as_array().unwrap().len(), 0);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn redeem_bad_key_is_404() {
    let a = common::arena().await;
    let user = common::register_account(&a.router, "User", "password123").await;
    let (status, _, _) = common::send(
        &a.router,
        Method::POST,
        "/personas/redeem",
        Some(&user),
        Some(&json!({ "key": "this-key-does-not-exist" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn owner_cannot_redeem_own_key() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let pid = create_persona(&a.router, &owner, "Mine").await;
    let key = share_key_of(&a.router, &owner, &pid).await;

    let (status, _, _) = common::send(
        &a.router,
        Method::POST,
        "/personas/redeem",
        Some(&owner),
        Some(&json!({ "key": key })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // Editor roster endpoint is owner-only: a stranger gets 404.
    let stranger = common::register_account(&a.router, "Stranger", "password123").await;
    let (status, _, _) = common::send(
        &a.router,
        Method::GET,
        &format!("/personas/{pid}/editors"),
        Some(&stranger),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
