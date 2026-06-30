//! Nova DOT in any channel — `/nova` (LLM-backed) + `/novasay` (manual). ssr-only.
//!
//! Mirrors `tests/system_messages.rs`: the admin-ALLOWED HTTP path can't be
//! driven under parallel workers (the `is_admin` env read races them), so the
//! CORES (`post_nova_say_core`, `run_nova_reply`) are exercised directly against
//! a real member account, the `/nova` model call is a no-network stub injected
//! via `AppState::with_nova_llm`, and only the fail-closed admin gate (and unauth)
//! is checked through the router.

mod common;

#[cfg(feature = "ssr")]
use authlyn_interactive::protocol::SyncEvent;
#[cfg(feature = "ssr")]
use authlyn_interactive::server::ctx::CtxClient;
#[cfg(feature = "ssr")]
use authlyn_interactive::server::messages::{
    build_chat_messages, effective_system_prompt, get_nova_prompt_core, post_nova_say_core,
    reply_or_unavailable, run_nova_reply, set_nova_prompt_core, NovaContextRow, NovaError,
};
#[cfg(feature = "ssr")]
use authlyn_interactive::server::nova_llm::{FunctionCall, NovaLlm, NovaTurn, ToolCall};
#[cfg(feature = "ssr")]
use axum::http::{Method, StatusCode};
#[cfg(feature = "ssr")]
use serde_json::json;
#[cfg(feature = "ssr")]
use surrealdb::types::SurrealValue;

/// Register an owner, create a guild (auto-makes a default text channel), and
/// return `(owner_cookie, owner_account_id, text_channel_id)`. The owner is a
/// real `guild_member`, so `channel_access` resolves them as a member.
#[cfg(feature = "ssr")]
async fn owner_guild_channel(router: &axum::Router) -> (String, String, String) {
    let (st, cookie, body) = common::send(
        router,
        Method::POST,
        "/auth/register",
        None,
        Some(&json!({ "username": "NovaOwner", "password": "password123" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "register: {body:?}");
    let owner = cookie.expect("session cookie");
    let owner_id = body["account_id"].as_str().expect("account_id").to_string();

    let (st, _, guild) = common::send(
        router,
        Method::POST,
        "/guilds",
        Some(&owner),
        Some(&json!({ "name": "NovaGuild" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let gid = guild["id"].as_str().unwrap().to_string();

    let (st, _, detail) = common::send(
        router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let cid = detail["channels"][0]["id"].as_str().unwrap().to_string();
    (owner, owner_id, cid)
}

// ---------------------------------------------------------------------------
// build_chat_messages — pure role mapping (no DB)
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
#[test]
fn build_chat_messages_maps_roles_and_prefixes_speakers() {
    let ctx = vec![
        NovaContextRow {
            author_key: "alice".into(),
            author_display: "Alice".into(),
            body: "hej".into(),
        },
        NovaContextRow {
            author_key: "nova_dot".into(),
            author_display: "Nova DOT".into(),
            body: "hello!".into(),
        },
        NovaContextRow {
            author_key: "bob".into(),
            author_display: "Bob".into(),
            body: "what's up".into(),
        },
    ];
    let msgs = build_chat_messages("SYS", &ctx);
    assert_eq!(msgs.len(), 4, "system + 3 context turns");
    assert_eq!(msgs[0].role, "system");
    assert_eq!(msgs[0].content, "SYS");
    assert_eq!(msgs[1].role, "user");
    assert_eq!(
        msgs[1].content, "Alice: hej",
        "non-Nova speakers are prefixed with their display name"
    );
    assert_eq!(msgs[2].role, "assistant");
    assert_eq!(
        msgs[2].content, "hello!",
        "Nova DOT's own turns map to assistant, no prefix"
    );
    assert_eq!(msgs[3].role, "user");
    assert_eq!(msgs[3].content, "Bob: what's up");
}

// ---------------------------------------------------------------------------
// /novasay core — manual Nova DOT line into one channel
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
#[tokio::test]
async fn novasay_posts_a_nova_dot_system_message_into_the_channel() {
    let a = common::arena().await;
    let (_owner, owner_id, cid) = owner_guild_channel(&a.router).await;

    let id = post_nova_say_core(&a.state, &cid, &owner_id, "Nova says hi")
        .await
        .expect("novasay core");
    assert!(!id.is_empty());

    #[derive(SurrealValue)]
    struct Row {
        channel_key: String,
        author_key: String,
        kind: String,
        body: String,
    }
    let mut resp =
        a.db.query(
            "SELECT meta::id(channel) AS channel_key, meta::id(author) AS author_key, kind, body \
             FROM message;",
        )
        .await
        .expect("query")
        .check()
        .expect("check");
    let rows: Vec<Row> = resp.take(0).expect("take");
    assert_eq!(rows.len(), 1, "exactly the one Nova DOT line");
    assert_eq!(rows[0].channel_key, cid, "lands in the targeted channel");
    assert_eq!(rows[0].author_key, "nova_dot");
    assert_eq!(rows[0].kind, "system");
    assert_eq!(rows[0].body, "Nova says hi");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn novasay_is_privacy_404_for_a_non_member_and_writes_nothing() {
    let a = common::arena().await;
    let (_owner, _owner_id, cid) = owner_guild_channel(&a.router).await;

    // A second account that is NOT a member of the channel's guild.
    let (st, _c, body) = common::send(
        &a.router,
        Method::POST,
        "/auth/register",
        None,
        Some(&json!({ "username": "Outsider", "password": "password123" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let outsider_id = body["account_id"].as_str().unwrap().to_string();

    let r = post_nova_say_core(&a.state, &cid, &outsider_id, "sneaky").await;
    assert!(
        matches!(r, Err(NovaError::NotFound)),
        "a non-member gets the privacy-404, got {r:?}"
    );

    #[derive(SurrealValue)]
    struct IdRow {
        id_key: String,
    }
    let mut resp =
        a.db.query("SELECT meta::id(id) AS id_key FROM message;")
            .await
            .expect("query")
            .check()
            .expect("check");
    let rows: Vec<IdRow> = resp.take(0).expect("take");
    assert!(rows.is_empty(), "the 404 means nothing was written");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn novasay_into_a_non_text_channel_is_400() {
    let a = common::arena().await;
    let (st, cookie, body) = common::send(
        &a.router,
        Method::POST,
        "/auth/register",
        None,
        Some(&json!({ "username": "LoreOwner", "password": "password123" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let owner = cookie.unwrap();
    let owner_id = body["account_id"].as_str().unwrap().to_string();

    let (st, _, guild) = common::send(
        &a.router,
        Method::POST,
        "/guilds",
        Some(&owner),
        Some(&json!({ "name": "LoreGuild" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let gid = guild["id"].as_str().unwrap().to_string();

    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/channels"),
        Some(&owner),
        Some(&json!({ "name": "world", "kind": "lorebook" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    // Resolve the lorebook channel id from the guild detail (kind-agnostic to the
    // create response shape).
    let (_st, _, detail) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&owner),
        None,
    )
    .await;
    let lore_cid = detail["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "lorebook")
        .expect("a lorebook channel")["id"]
        .as_str()
        .unwrap()
        .to_string();

    let r = post_nova_say_core(&a.state, &lore_cid, &owner_id, "lore noise").await;
    assert!(
        matches!(r, Err(NovaError::BadRequest(_))),
        "a lorebook channel rejects a Nova line, got {r:?}"
    );
}

// ---------------------------------------------------------------------------
// /nova reply — generation via a stubbed model
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
#[tokio::test]
async fn nova_reply_posts_as_nova_dot_system_and_emits_message_created() {
    let a = common::arena().await;
    let (owner, _owner_id, cid) = owner_guild_channel(&a.router).await;

    // The admin's prompt (a normal message) so Nova has channel context.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        Some(&json!({ "body": "Nova, what's happening?" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    // Nova-enabled state sharing the arena's DB + SSE bus, with a stubbed model.
    let nova_state = a
        .state
        .clone()
        .with_nova_llm(NovaLlm::stub("Nova's canned reply"));
    // Subscribe AFTER the prompt was posted, so only the reply's emit arrives.
    let mut rx = nova_state.events.subscribe();

    let reply_id = run_nova_reply(&nova_state, &cid)
        .await
        .expect("run_nova_reply")
        .expect("a reply id (the stub is non-empty)");
    assert!(!reply_id.is_empty());

    // The reply landed as a Nova DOT system message carrying the stub body.
    #[derive(SurrealValue)]
    struct Row {
        author_key: String,
        kind: String,
        body: String,
    }
    let mut resp =
        a.db.query(
            "SELECT meta::id(author) AS author_key, kind, body \
             FROM message WHERE author = account:nova_dot;",
        )
        .await
        .expect("query")
        .check()
        .expect("check");
    let rows: Vec<Row> = resp.take(0).expect("take");
    assert_eq!(rows.len(), 1, "exactly one Nova DOT reply");
    assert_eq!(rows[0].author_key, "nova_dot");
    assert_eq!(rows[0].kind, "system");
    assert_eq!(rows[0].body, "Nova's canned reply");

    // And it emitted MessageCreated on the bus, like every other message write.
    let ev = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("an emit within 2s")
        .expect("bus recv");
    assert!(
        matches!(ev.event, SyncEvent::MessageCreated { ref channel_id } if channel_id == &cid),
        "the reply emits MessageCreated for the channel, got {:?}",
        ev.event
    );
}

// ---------------------------------------------------------------------------
// Admin gate — fail-closed (empty admin set authorizes no one) + unauth
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
#[tokio::test]
async fn nova_commands_are_403_for_a_non_admin_and_write_nothing() {
    let a = common::arena().await;
    let (owner, _owner_id, cid) = owner_guild_channel(&a.router).await;

    // No admins configured → the owner (a normal user) is non-admin → 403 on both.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/novasay"),
        Some(&owner),
        Some(&json!({ "body": "hi" })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "/novasay is admin-gated");

    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/nova"),
        Some(&owner),
        Some(&json!({ "prompt": "hi" })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "/nova is admin-gated");

    #[derive(SurrealValue)]
    struct IdRow {
        id_key: String,
    }
    let mut resp =
        a.db.query("SELECT meta::id(id) AS id_key FROM message WHERE kind = 'system';")
            .await
            .expect("query")
            .check()
            .expect("check");
    let rows: Vec<IdRow> = resp.take(0).expect("take");
    assert!(rows.is_empty(), "403 wrote no Nova DOT message");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn nova_requires_auth() {
    let a = common::arena().await;
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        "/channels/whatever/nova",
        None,
        Some(&json!({ "prompt": "hi" })),
    )
    .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------------
// Per-channel Nova system prompt (admin-set addendum, appended to global base)
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
#[test]
fn effective_system_prompt_appends_addendum_or_falls_back_to_base() {
    assert_eq!(effective_system_prompt("BASE", None), "BASE");
    assert_eq!(
        effective_system_prompt("BASE", Some("   ")),
        "BASE",
        "a blank addendum → the base alone"
    );
    assert_eq!(
        effective_system_prompt("BASE", Some("flavor")),
        "BASE\n\nflavor",
        "the addendum is appended after the base, not a replacement"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn nova_prompt_set_get_and_clear_round_trips() {
    let a = common::arena().await;
    let (_owner, owner_id, cid) = owner_guild_channel(&a.router).await;

    assert_eq!(
        get_nova_prompt_core(&a.state, &cid, &owner_id)
            .await
            .expect("get"),
        None,
        "unset initially"
    );

    set_nova_prompt_core(&a.state, &cid, &owner_id, Some("Be terse.".into()))
        .await
        .expect("set");
    assert_eq!(
        get_nova_prompt_core(&a.state, &cid, &owner_id)
            .await
            .expect("get"),
        Some("Be terse.".to_string())
    );

    // None clears.
    set_nova_prompt_core(&a.state, &cid, &owner_id, None)
        .await
        .expect("clear");
    assert_eq!(
        get_nova_prompt_core(&a.state, &cid, &owner_id)
            .await
            .expect("get"),
        None
    );

    // A blank/whitespace value also clears.
    set_nova_prompt_core(&a.state, &cid, &owner_id, Some("x".into()))
        .await
        .expect("set again");
    set_nova_prompt_core(&a.state, &cid, &owner_id, Some("   ".into()))
        .await
        .expect("clear via blank");
    assert_eq!(
        get_nova_prompt_core(&a.state, &cid, &owner_id)
            .await
            .expect("get"),
        None
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn nova_prompt_core_is_privacy_404_for_a_non_member() {
    let a = common::arena().await;
    let (_owner, _owner_id, cid) = owner_guild_channel(&a.router).await;

    let (st, _c, body) = common::send(
        &a.router,
        Method::POST,
        "/auth/register",
        None,
        Some(&json!({ "username": "PromptOutsider", "password": "password123" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let outsider_id = body["account_id"].as_str().unwrap().to_string();

    assert!(matches!(
        set_nova_prompt_core(&a.state, &cid, &outsider_id, Some("x".into())).await,
        Err(NovaError::NotFound)
    ));
    assert!(matches!(
        get_nova_prompt_core(&a.state, &cid, &outsider_id).await,
        Err(NovaError::NotFound)
    ));
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn nova_reply_appends_the_channel_prompt_into_the_system_prompt() {
    let a = common::arena().await;
    let (owner, owner_id, cid) = owner_guild_channel(&a.router).await;

    set_nova_prompt_core(
        &a.state,
        &cid,
        &owner_id,
        Some("Speak like a pirate.".into()),
    )
    .await
    .expect("set channel prompt");

    // Seed a message so Nova has context.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        Some(&json!({ "body": "ahoy" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    // The echo stub returns the assembled SYSTEM prompt AS its reply, so we can
    // assert the channel addendum flowed into it.
    let nova_state = a.state.clone().with_nova_llm(NovaLlm::stub_echo());
    run_nova_reply(&nova_state, &cid)
        .await
        .expect("run_nova_reply")
        .expect("a reply id");

    #[derive(SurrealValue)]
    struct Row {
        body: String,
    }
    let mut resp =
        a.db.query("SELECT body FROM message WHERE author = account:nova_dot;")
            .await
            .expect("query")
            .check()
            .expect("check");
    let rows: Vec<Row> = resp.take(0).expect("take");
    assert_eq!(rows.len(), 1);
    assert!(
        rows[0].body.contains("Speak like a pirate."),
        "the channel addendum is in the system prompt; got: {}",
        rows[0].body
    );
    assert!(
        rows[0].body.trim_end().ends_with("Speak like a pirate."),
        "the addendum is appended AFTER the global base"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn nova_prompt_endpoints_are_403_for_a_non_admin() {
    let a = common::arena().await;
    let (owner, _owner_id, cid) = owner_guild_channel(&a.router).await;

    let (st, _, _) = common::send(
        &a.router,
        Method::PUT,
        &format!("/channels/{cid}/nova-prompt"),
        Some(&owner),
        Some(&json!({ "prompt": "x" })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "PUT nova-prompt is admin-gated");

    let (st, _, _) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/nova-prompt"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "GET nova-prompt is admin-gated");
}

// ---------------------------------------------------------------------------
// /nova reply — model-driven ctx tool-calling loop (stubbed model + ctx)
// ---------------------------------------------------------------------------

/// Build one model-requested tool call.
#[cfg(feature = "ssr")]
fn tool_call(id: &str, name: &str, args: &str) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        kind: "function".to_string(),
        function: FunctionCall {
            name: name.to_string(),
            arguments: args.to_string(),
        },
    }
}

/// The model requests one ctx tool, the server dispatches it, and the model's
/// FINAL text (not the tool output) is posted as Nova DOT.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn nova_reply_dispatches_a_ctx_tool_then_posts_the_models_final_text() {
    let a = common::arena().await;
    let (owner, _owner_id, cid) = owner_guild_channel(&a.router).await;
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        Some(&json!({ "body": "Nova, what do you remember?" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    let ctx = CtxClient::stub_with_responses(&[("query", "CANNED ANSWER")]);
    let nova_state = a
        .state
        .clone()
        .with_nova_llm(NovaLlm::stub_script(vec![
            NovaTurn::ToolCalls(vec![tool_call(
                "c1",
                "query",
                r#"{"question":"what do you remember?"}"#,
            )]),
            NovaTurn::Text("Here is what I found.".into()),
        ]))
        .with_ctx(ctx.clone());

    run_nova_reply(&nova_state, &cid)
        .await
        .expect("run_nova_reply")
        .expect("a reply id");

    let calls = ctx.recorded_tool_calls();
    assert_eq!(calls.len(), 1, "the model's one tool call reached ctx");
    assert_eq!(calls[0].0, "query");
    assert_eq!(calls[0].1["question"], "what do you remember?");

    #[derive(SurrealValue)]
    struct Row {
        body: String,
    }
    let mut resp =
        a.db.query("SELECT body FROM message WHERE author = account:nova_dot;")
            .await
            .expect("query")
            .check()
            .expect("check");
    let rows: Vec<Row> = resp.take(0).expect("take");
    assert_eq!(rows.len(), 1, "exactly one Nova DOT reply");
    assert_eq!(
        rows[0].body, "Here is what I found.",
        "the model's FINAL text is posted, not the tool output"
    );
}

/// Malformed tool-call arguments are caught BEFORE ctx is touched; the reply
/// still completes from the model's next turn.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn nova_reply_handles_a_malformed_tool_call_without_failing() {
    let a = common::arena().await;
    let (_owner, _owner_id, cid) = owner_guild_channel(&a.router).await;

    let ctx = CtxClient::stub();
    let nova_state = a
        .state
        .clone()
        .with_nova_llm(NovaLlm::stub_script(vec![
            NovaTurn::ToolCalls(vec![tool_call("c1", "query", "{ not json")]),
            NovaTurn::Text("Falling back to a plain answer.".into()),
        ]))
        .with_ctx(ctx.clone());

    run_nova_reply(&nova_state, &cid)
        .await
        .expect("run_nova_reply")
        .expect("a reply id");

    assert!(
        ctx.recorded_tool_calls().is_empty(),
        "malformed args never reach ctx"
    );

    #[derive(SurrealValue)]
    struct Row {
        body: String,
    }
    let mut resp =
        a.db.query("SELECT body FROM message WHERE author = account:nova_dot;")
            .await
            .expect("query")
            .check()
            .expect("check");
    let rows: Vec<Row> = resp.take(0).expect("take");
    assert_eq!(rows[0].body, "Falling back to a plain answer.");
}

/// An unknown/hallucinated tool name is rejected by the `call_tool` guard (never
/// recorded), and the reply degrades to the model's next turn.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn nova_reply_with_an_unknown_tool_name_degrades() {
    let a = common::arena().await;
    let (_owner, _owner_id, cid) = owner_guild_channel(&a.router).await;

    let ctx = CtxClient::stub();
    let nova_state = a
        .state
        .clone()
        .with_nova_llm(NovaLlm::stub_script(vec![
            NovaTurn::ToolCalls(vec![tool_call("c1", "delete", "{}")]),
            NovaTurn::Text("I can't do that, but here's a reply.".into()),
        ]))
        .with_ctx(ctx.clone());

    let id = run_nova_reply(&nova_state, &cid)
        .await
        .expect("run_nova_reply");
    assert!(id.is_some());
    assert!(
        ctx.recorded_tool_calls().is_empty(),
        "an unknown tool name is rejected before any backend call"
    );
}

/// A failing ctx tool becomes a model-readable error string; the reply continues
/// to the model's next turn.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn nova_reply_continues_when_a_ctx_tool_errors() {
    let a = common::arena().await;
    let (_owner, _owner_id, cid) = owner_guild_channel(&a.router).await;

    let ctx = CtxClient::stub_failing(&["query"]);
    let nova_state = a
        .state
        .clone()
        .with_nova_llm(NovaLlm::stub_script(vec![
            NovaTurn::ToolCalls(vec![tool_call("c1", "query", r#"{"question":"x"}"#)]),
            NovaTurn::Text("Answering despite the tool error.".into()),
        ]))
        .with_ctx(ctx.clone());

    run_nova_reply(&nova_state, &cid)
        .await
        .expect("run_nova_reply")
        .expect("a reply id");

    assert_eq!(
        ctx.recorded_tool_calls().len(),
        1,
        "the failing tool WAS attempted"
    );

    #[derive(SurrealValue)]
    struct Row {
        body: String,
    }
    let mut resp =
        a.db.query("SELECT body FROM message WHERE author = account:nova_dot;")
            .await
            .expect("query")
            .check()
            .expect("check");
    let rows: Vec<Row> = resp.take(0).expect("take");
    assert_eq!(rows[0].body, "Answering despite the tool error.");
}

/// A model that ONLY ever requests tools (sticky script) must still terminate:
/// the iteration cap bounds the dispatches and the reply ends (here, with nothing
/// posted because the model never produced final text).
#[cfg(feature = "ssr")]
#[tokio::test]
async fn nova_reply_terminates_when_the_model_only_ever_requests_tools() {
    let a = common::arena().await;
    let (_owner, _owner_id, cid) = owner_guild_channel(&a.router).await;

    let llm = NovaLlm::stub_script(vec![NovaTurn::ToolCalls(vec![tool_call(
        "c1", "query", "{}",
    )])]);
    let max = llm.max_tool_iters;
    let ctx = CtxClient::stub();
    let nova_state = a.state.clone().with_nova_llm(llm).with_ctx(ctx.clone());

    let result = run_nova_reply(&nova_state, &cid)
        .await
        .expect("run_nova_reply");
    assert!(
        result.is_none(),
        "a model that only calls tools yields no final text → nothing posted"
    );
    let n = ctx.recorded_tool_calls().len();
    assert!(n >= 1, "tools were actually dispatched");
    assert!(
        n <= max,
        "dispatch is bounded by max_tool_iters ({max}), got {n}"
    );
}

/// With ctx unconfigured no tools are offered; even if the (stub) model still
/// emits a tool call, dispatch degrades to a "not configured" result and the
/// reply completes.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn nova_reply_degrades_when_model_requests_tools_but_ctx_is_unconfigured() {
    let a = common::arena().await;
    let (_owner, _owner_id, cid) = owner_guild_channel(&a.router).await;

    let nova_state = a.state.clone().with_nova_llm(NovaLlm::stub_script(vec![
        NovaTurn::ToolCalls(vec![tool_call("c1", "query", "{}")]),
        NovaTurn::Text("Plain reply, no tools.".into()),
    ]));

    run_nova_reply(&nova_state, &cid)
        .await
        .expect("run_nova_reply")
        .expect("a reply id");

    #[derive(SurrealValue)]
    struct Row {
        body: String,
    }
    let mut resp =
        a.db.query("SELECT body FROM message WHERE author = account:nova_dot;")
            .await
            .expect("query")
            .check()
            .expect("check");
    let rows: Vec<Row> = resp.take(0).expect("take");
    assert_eq!(rows[0].body, "Plain reply, no tools.");
}

// ---------------------------------------------------------------------------
// /nova reply — failure-degradation + write paths (the branches a green happy
// path leaves unproven: blown budget, the squeeze's success branch, the
// kill-switch, the store write, and the persisted-row cap)
// ---------------------------------------------------------------------------

/// A blown wall-clock reply budget surfaces as `Err` (M14 parity) and writes nothing —
/// turning that `Err` into a visible "unavailable" line is the caller's job (next test).
#[cfg(feature = "ssr")]
#[tokio::test]
async fn nova_reply_errors_when_the_reply_budget_is_blown() {
    let a = common::arena().await;
    let (_owner, _owner_id, cid) = owner_guild_channel(&a.router).await;

    // A backend that pends (sleeps) so a zero-second budget deterministically elapses —
    // every other stub returns synchronously and would beat the timeout.
    let mut llm = NovaLlm::stub_slow(std::time::Duration::from_secs(30), "too late");
    std::sync::Arc::get_mut(&mut llm)
        .expect("freshly built, unique Arc")
        .reply_budget_secs = 0;
    let nova_state = a.state.clone().with_nova_llm(llm);

    let r = run_nova_reply(&nova_state, &cid).await;
    assert!(
        r.is_err(),
        "a blown reply budget surfaces as Err, got {r:?}"
    );

    #[derive(SurrealValue)]
    struct IdRow {
        id_key: String,
    }
    let mut resp =
        a.db.query("SELECT meta::id(id) AS id_key FROM message WHERE author = account:nova_dot;")
            .await
            .expect("query")
            .check()
            .expect("check");
    let rows: Vec<IdRow> = resp.take(0).expect("take");
    assert!(rows.is_empty(), "a blown budget posts no Nova DOT reply");
}

/// `reply_or_unavailable` turns a generation failure (here a blown budget) into a
/// visible Nova DOT "unavailable" line — the admin never gets silence. This is the
/// exact body `nova_ask` spawns.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn reply_or_unavailable_posts_the_unavailable_line_on_failure() {
    let a = common::arena().await;
    let (_owner, _owner_id, cid) = owner_guild_channel(&a.router).await;

    let mut llm = NovaLlm::stub_slow(std::time::Duration::from_secs(30), "too late");
    std::sync::Arc::get_mut(&mut llm)
        .expect("freshly built, unique Arc")
        .reply_budget_secs = 0;
    let nova_state = a.state.clone().with_nova_llm(llm);

    reply_or_unavailable(nova_state.clone(), cid.clone()).await;

    #[derive(SurrealValue)]
    struct Row {
        kind: String,
        body: String,
    }
    let mut resp =
        a.db.query("SELECT kind, body FROM message WHERE author = account:nova_dot;")
            .await
            .expect("query")
            .check()
            .expect("check");
    let rows: Vec<Row> = resp.take(0).expect("take");
    assert_eq!(rows.len(), 1, "exactly the one unavailable line");
    assert_eq!(rows[0].kind, "system");
    assert_eq!(
        rows[0].body, "⚠️ Nova is unavailable right now.",
        "the exact user-facing unavailable copy"
    );
}

/// After the iteration cap, the tools-disabled "squeeze" can still produce final TEXT
/// (a capped-out model that answers once tools are stripped) — that text is posted.
/// Complements `…only_ever_requests_tools`, which drives the squeeze's EMPTY branch.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn nova_reply_squeeze_posts_final_text_after_the_iteration_cap() {
    let a = common::arena().await;
    let (_owner, _owner_id, cid) = owner_guild_channel(&a.router).await;

    let max = NovaLlm::stub("x").max_tool_iters;
    // `max` tool-only rounds drive the loop to its cap; then the squeeze answers.
    let mut turns: Vec<NovaTurn> = (0..max)
        .map(|_| NovaTurn::ToolCalls(vec![tool_call("c1", "query", "{}")]))
        .collect();
    turns.push(NovaTurn::Text("Squeezed reply.".into()));

    let ctx = CtxClient::stub();
    let nova_state = a
        .state
        .clone()
        .with_nova_llm(NovaLlm::stub_script(turns))
        .with_ctx(ctx.clone());

    run_nova_reply(&nova_state, &cid)
        .await
        .expect("run_nova_reply")
        .expect("a reply id");

    assert_eq!(
        ctx.recorded_tool_calls().len(),
        max,
        "every capped round dispatched its tool before the squeeze"
    );

    #[derive(SurrealValue)]
    struct Row {
        body: String,
    }
    let mut resp =
        a.db.query("SELECT body FROM message WHERE author = account:nova_dot;")
            .await
            .expect("query")
            .check()
            .expect("check");
    let rows: Vec<Row> = resp.take(0).expect("take");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].body, "Squeezed reply.");
}

/// `max_tool_iters == 0` is the plain-chat kill-switch: even with ctx configured the
/// loop is skipped entirely (no tool dispatch) and the squeeze produces the reply.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn nova_reply_with_zero_tool_iters_never_dispatches_a_tool() {
    let a = common::arena().await;
    let (_owner, _owner_id, cid) = owner_guild_channel(&a.router).await;

    let mut llm = NovaLlm::stub_script(vec![NovaTurn::Text("Plain, no tools.".into())]);
    std::sync::Arc::get_mut(&mut llm)
        .expect("freshly built, unique Arc")
        .max_tool_iters = 0;

    let ctx = CtxClient::stub();
    let nova_state = a.state.clone().with_nova_llm(llm).with_ctx(ctx.clone());

    run_nova_reply(&nova_state, &cid)
        .await
        .expect("run_nova_reply")
        .expect("a reply id");

    assert!(
        ctx.recorded_tool_calls().is_empty(),
        "with 0 iters the tool loop is skipped — no ctx dispatch"
    );

    #[derive(SurrealValue)]
    struct Row {
        body: String,
    }
    let mut resp =
        a.db.query("SELECT body FROM message WHERE author = account:nova_dot;")
            .await
            .expect("query")
            .check()
            .expect("check");
    let rows: Vec<Row> = resp.take(0).expect("take");
    assert_eq!(rows[0].body, "Plain, no tools.");
}

/// The model's `store` call flows through the loop to ctx WITH the sensitivity default
/// applied (`internal`) — Nova's only write path, proven end-to-end (not just the unit).
#[cfg(feature = "ssr")]
#[tokio::test]
async fn nova_reply_dispatches_a_store_with_the_internal_sensitivity_default() {
    let a = common::arena().await;
    let (_owner, _owner_id, cid) = owner_guild_channel(&a.router).await;

    let ctx = CtxClient::stub();
    let nova_state = a
        .state
        .clone()
        .with_nova_llm(NovaLlm::stub_script(vec![
            NovaTurn::ToolCalls(vec![tool_call(
                "c1",
                "store",
                r#"{"category":"nova-memory","title":"t","content":"c"}"#,
            )]),
            NovaTurn::Text("Saved.".into()),
        ]))
        .with_ctx(ctx.clone());

    run_nova_reply(&nova_state, &cid)
        .await
        .expect("run_nova_reply")
        .expect("a reply id");

    let calls = ctx.recorded_tool_calls();
    assert_eq!(calls.len(), 1, "the store call reached ctx");
    assert_eq!(calls[0].0, "store");
    assert_eq!(
        calls[0].1["sensitivity"], "internal",
        "a tagless store defaults to sensitivity=internal through the loop"
    );
    assert_eq!(calls[0].1["category"], "nova-memory");

    #[derive(SurrealValue)]
    struct Row {
        body: String,
    }
    let mut resp =
        a.db.query("SELECT body FROM message WHERE author = account:nova_dot;")
            .await
            .expect("query")
            .check()
            .expect("check");
    let rows: Vec<Row> = resp.take(0).expect("take");
    assert_eq!(rows[0].body, "Saved.");
}

/// A reply longer than the persisted-row cap is truncated to `NOVA_REPLY_MAX_CHARS`
/// (8000) regardless of the model's token cap.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn nova_reply_truncates_an_over_long_reply_to_the_char_cap() {
    let a = common::arena().await;
    let (_owner, _owner_id, cid) = owner_guild_channel(&a.router).await;

    let nova_state = a
        .state
        .clone()
        .with_nova_llm(NovaLlm::stub("x".repeat(9_000)));

    run_nova_reply(&nova_state, &cid)
        .await
        .expect("run_nova_reply")
        .expect("a reply id");

    #[derive(SurrealValue)]
    struct Row {
        body: String,
    }
    let mut resp =
        a.db.query("SELECT body FROM message WHERE author = account:nova_dot;")
            .await
            .expect("query")
            .check()
            .expect("check");
    let rows: Vec<Row> = resp.take(0).expect("take");
    assert_eq!(
        rows[0].body.chars().count(),
        8_000,
        "the persisted reply is capped at NOVA_REPLY_MAX_CHARS"
    );
}

/// A single model turn requesting MULTIPLE tool calls dispatches each, in order.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn nova_reply_dispatches_every_call_in_a_multi_tool_turn() {
    let a = common::arena().await;
    let (_owner, _owner_id, cid) = owner_guild_channel(&a.router).await;

    let ctx = CtxClient::stub();
    let nova_state = a
        .state
        .clone()
        .with_nova_llm(NovaLlm::stub_script(vec![
            NovaTurn::ToolCalls(vec![
                tool_call("c1", "query", r#"{"question":"a"}"#),
                tool_call("c2", "search", r#"{"query":"b"}"#),
            ]),
            NovaTurn::Text("Done.".into()),
        ]))
        .with_ctx(ctx.clone());

    run_nova_reply(&nova_state, &cid)
        .await
        .expect("run_nova_reply")
        .expect("a reply id");

    let calls = ctx.recorded_tool_calls();
    assert_eq!(calls.len(), 2, "both calls in the batch were dispatched");
    assert_eq!(calls[0].0, "query");
    assert_eq!(calls[1].0, "search");
}

/// An empty argument string is normalized to `{}` and still reaches ctx (not an error).
#[cfg(feature = "ssr")]
#[tokio::test]
async fn nova_reply_normalizes_empty_tool_args_to_an_empty_object() {
    let a = common::arena().await;
    let (_owner, _owner_id, cid) = owner_guild_channel(&a.router).await;

    let ctx = CtxClient::stub();
    let nova_state = a
        .state
        .clone()
        .with_nova_llm(NovaLlm::stub_script(vec![
            NovaTurn::ToolCalls(vec![tool_call("c1", "recent", "")]),
            NovaTurn::Text("ok".into()),
        ]))
        .with_ctx(ctx.clone());

    run_nova_reply(&nova_state, &cid)
        .await
        .expect("run_nova_reply")
        .expect("a reply id");

    let calls = ctx.recorded_tool_calls();
    assert_eq!(calls.len(), 1, "empty args still dispatch");
    assert_eq!(calls[0].0, "recent");
    assert_eq!(
        calls[0].1,
        json!({}),
        "an empty arg string becomes an empty object"
    );
}

/// A tool-args value that is valid JSON but NOT an object is rejected before ctx; the
/// reply degrades to the model's next turn.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn nova_reply_rejects_non_object_tool_args_before_ctx() {
    let a = common::arena().await;
    let (_owner, _owner_id, cid) = owner_guild_channel(&a.router).await;

    let ctx = CtxClient::stub();
    let nova_state = a
        .state
        .clone()
        .with_nova_llm(NovaLlm::stub_script(vec![
            NovaTurn::ToolCalls(vec![tool_call("c1", "query", "42")]),
            NovaTurn::Text("Plain answer.".into()),
        ]))
        .with_ctx(ctx.clone());

    run_nova_reply(&nova_state, &cid)
        .await
        .expect("run_nova_reply")
        .expect("a reply id");

    assert!(
        ctx.recorded_tool_calls().is_empty(),
        "a non-object args value never reaches ctx"
    );

    #[derive(SurrealValue)]
    struct Row {
        body: String,
    }
    let mut resp =
        a.db.query("SELECT body FROM message WHERE author = account:nova_dot;")
            .await
            .expect("query")
            .check()
            .expect("check");
    let rows: Vec<Row> = resp.take(0).expect("take");
    assert_eq!(rows[0].body, "Plain answer.");
}
