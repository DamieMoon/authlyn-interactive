//! `POST /channels/{cid}/nova` + `POST /channels/{cid}/novasay` — admin-gated
//! Nova DOT in any channel. ssr-only.
//!
//! - `/novasay <text>`: post `text` verbatim as a `kind='system'` "Nova DOT"
//!   message in THIS channel — the per-channel narrowing of the app-wide
//!   [`crate::server::system_messages::broadcast_system_message`] fan-out.
//! - `/nova <prompt>`: post the admin's prompt as their OWN message, then
//!   generate Nova DOT's reply with the local Qwen model
//!   ([`crate::server::nova_llm`]) and post it as a `kind='system'` "Nova DOT"
//!   message. The reply is produced in a spawned task and lands over the SSE bus
//!   like any other message — the POST returns 202 at once so the composer never
//!   hangs on generation. When Nova's ctx tools are configured
//!   ([`crate::server::ctx`]), the reply runs a model-driven tool-call loop.
//!
//! Both are admin-gated (`is_admin`, fail-closed → 403) and re-derive channel
//! membership per call (privacy-404, text-only). "Nova DOT" is the reserved
//! `account:nova_dot` system bot — NOT the `nova-mcp` "Nova" user account (a
//! different entity). Replies and manual lines reuse `kind='system'`, so no
//! schema migration is needed.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use surrealdb::types::SurrealValue;

use crate::protocol::{
    NovaAskRequest, NovaPromptRequest, NovaPromptResponse, NovaSayRequest, SendMessageResponse,
    SyncEvent,
};
use crate::server::auth::AuthAccount;
use crate::server::errors::{error_response, json_rejection_response};
use crate::server::nova_llm::{ChatMessage, NovaLlm, NovaResult, NovaTurn, ToolCall};
use crate::server::permissions::is_admin;
use crate::server::state::AppState;
use crate::server::system_messages::validate_broadcast_body;

use super::posting::{persist_message, resolve_send_persona};
use super::{channel_access, AccessOutcome};

/// The reserved bot account id (seeded in `schema.surql`) that authors every
/// Nova DOT message — the same account `system_messages` broadcasts as.
const NOVA_DOT_ACCOUNT: &str = "nova_dot";

/// Defensive upper bound on a generated reply, in characters. `max_tokens`
/// already bounds the model; this caps the persisted row regardless.
const NOVA_REPLY_MAX_CHARS: usize = 8_000;

/// Posted as Nova DOT when reply generation fails (timeout / model down), so the
/// admin gets honest feedback instead of silence.
const NOVA_UNAVAILABLE_BODY: &str = "⚠️ Nova is unavailable right now.";

/// Upper bound on a per-channel Nova system-prompt addendum, in characters. It
/// is prepended to every reply's context, so it eats the (8192-token) budget —
/// keep it modest.
const NOVA_PROMPT_MAX_CHARS: usize = 8_000;

// ---------------------------------------------------------------------------
// Shared error → status mapping
// ---------------------------------------------------------------------------

/// Why a `/novasay` (or a `/nova` prompt) gate failed, mapped to a status by the
/// handler. Public so integration tests can assert the core's outcome.
#[derive(Debug)]
pub enum NovaError {
    /// 400 with this exact user-facing message.
    BadRequest(&'static str),
    /// 404 privacy-404 (unknown channel OR caller not a member).
    NotFound,
    /// 500 storage error.
    Storage(surrealdb::Error),
}

fn nova_error_response(e: NovaError, ctx: &str) -> Response {
    match e {
        NovaError::BadRequest(m) => error_response(StatusCode::BAD_REQUEST, m),
        NovaError::NotFound => error_response(StatusCode::NOT_FOUND, "channel not found"),
        NovaError::Storage(err) => {
            tracing::error!(error = %err, "{ctx} storage error");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

/// App-admin gate. `None` = authorized; `Some(resp)` is the 403/500 to return.
async fn admin_gate(state: &AppState, account: &str) -> Option<Response> {
    match is_admin(state, account).await {
        Ok(true) => None,
        Ok(false) => Some(error_response(StatusCode::FORBIDDEN, "forbidden")),
        Err(e) => {
            tracing::error!(error = %e, "admin check failed");
            Some(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage error",
            ))
        }
    }
}

/// Membership + kind gate shared by both commands: `account` must be a member of
/// `cid` and `cid` must be a live TEXT channel, else the privacy-404 / 400.
async fn require_text_channel(state: &AppState, cid: &str, account: &str) -> Result<(), NovaError> {
    match channel_access(state, cid, account)
        .await
        .map_err(NovaError::Storage)?
    {
        AccessOutcome::Ok(ctx) => {
            if ctx.kind != "text" {
                return Err(NovaError::BadRequest(
                    "cannot post messages to a non-text channel",
                ));
            }
            Ok(())
        }
        AccessOutcome::ChannelNotFound | AccessOutcome::NotMember => Err(NovaError::NotFound),
    }
}

// ---------------------------------------------------------------------------
// /novasay — manual "Nova DOT says" into one channel
// ---------------------------------------------------------------------------

/// Auth-free core: post `body` as a Nova DOT `kind='system'` message into `cid`,
/// after checking `account` is a member of a TEXT channel (privacy-404). The
/// admin gate lives in [`nova_say`]; this core is exposed so integration tests
/// can drive it directly (the `is_admin` env read races parallel workers — same
/// constraint as `system_messages`). Returns the new message id.
pub async fn post_nova_say_core(
    state: &AppState,
    cid: &str,
    account: &str,
    body: &str,
) -> Result<String, NovaError> {
    let body = validate_broadcast_body(body).map_err(NovaError::BadRequest)?;
    require_text_channel(state, cid, account).await?;
    let id = persist_nova_dot_message(state, cid, &body)
        .await
        .map_err(NovaError::Storage)?;
    Ok(id)
}

/// POST /channels/{cid}/novasay — admin-only. Post `body` verbatim as a Nova DOT
/// `kind='system'` message in this channel. Non-members get the privacy-404.
#[tracing::instrument(skip_all, fields(account = %account.0, channel = %cid))]
pub async fn nova_say(
    State(state): State<AppState>,
    Path(cid): Path<String>,
    account: AuthAccount,
    payload: Result<Json<NovaSayRequest>, JsonRejection>,
) -> Response {
    if let Some(resp) = admin_gate(&state, &account.0).await {
        return resp;
    }
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    match post_nova_say_core(&state, &cid, &account.0, &req.body).await {
        Ok(id) => (StatusCode::CREATED, Json(SendMessageResponse { id })).into_response(),
        Err(e) => nova_error_response(e, "novasay"),
    }
}

// ---------------------------------------------------------------------------
// /nova — LLM-backed Nova DOT reply
// ---------------------------------------------------------------------------

/// POST /channels/{cid}/nova — admin-only. Post the admin's `prompt` as their
/// own message, then generate Nova DOT's reply (Qwen) in a spawned task and post
/// it as a `kind='system'` "Nova DOT" message. Returns 202 with the PROMPT's
/// message id; the reply arrives over SSE when generation completes. A 503 when
/// Nova is unconfigured (no model endpoint) — returned BEFORE the prompt is
/// posted, so an unconfigured box never strands a bare question.
#[tracing::instrument(skip_all, fields(account = %account.0, channel = %cid))]
pub async fn nova_ask(
    State(state): State<AppState>,
    Path(cid): Path<String>,
    account: AuthAccount,
    payload: Result<Json<NovaAskRequest>, JsonRejection>,
) -> Response {
    if let Some(resp) = admin_gate(&state, &account.0).await {
        return resp;
    }
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    let prompt = req.prompt.trim_end().to_string();
    if prompt.trim().is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "message must have text");
    }
    if prompt.chars().count() > super::MAX_BODY_CHARS {
        return error_response(StatusCode::BAD_REQUEST, "message body too long");
    }
    // Nova must be configured (a model endpoint) — fail fast BEFORE posting the
    // prompt, so an unconfigured dev box doesn't strand a bare question.
    if state.nova_llm.is_none() {
        return error_response(StatusCode::SERVICE_UNAVAILABLE, "Nova is not configured");
    }
    // Membership gate (privacy-404 + text-only) + the admin's worn persona —
    // their prompt is a normal persona-aware send.
    let (stored_persona, via_guest) = match channel_access(&state, &cid, &account.0).await {
        Ok(AccessOutcome::Ok(ctx)) => {
            if ctx.kind != "text" {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "cannot post messages to a non-text channel",
                );
            }
            (ctx.active_persona, ctx.via_guest)
        }
        Ok(AccessOutcome::ChannelNotFound) | Ok(AccessOutcome::NotMember) => {
            return error_response(StatusCode::NOT_FOUND, "channel not found");
        }
        Err(e) => {
            tracing::error!(error = %e, "channel_access failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    let persona = match resolve_send_persona(
        &state,
        &account.0,
        req.persona.as_deref(),
        stored_persona.as_deref(),
    )
    .await
    {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "can_edit_persona failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    // Post the admin's prompt as their own (persona-aware) message — visible to
    // the channel, the same persist + emit path as a normal send.
    let prompt_id = match persist_message(
        &state,
        &cid,
        &account.0,
        persona.as_deref(),
        &prompt,
        "user",
        &[],
        None,
        &[],
        None,
        via_guest,
    )
    .await
    {
        Ok(id) => id,
        Err(e) => {
            tracing::error!(error = %e, "persist nova prompt failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    super::typing::clear_draft(&state, &cid, &account.0);
    crate::server::push::notify_new_message(state.clone(), prompt_id.clone(), account.0.clone());
    state.emit(SyncEvent::MessageCreated {
        channel_id: cid.clone(),
    });

    // Generate + post Nova's reply OFF the request path. The reply lands via the
    // SSE bus when ready; a failure posts a visible "unavailable" line.
    let task_state = state.clone();
    let task_cid = cid.clone();
    tokio::spawn(async move {
        if let Err(e) = run_nova_reply(&task_state, &task_cid).await {
            tracing::error!(error = %e, channel = %task_cid, "nova reply generation failed");
            post_unavailable(&task_state, &task_cid).await;
        }
    });

    (
        StatusCode::ACCEPTED,
        Json(SendMessageResponse { id: prompt_id }),
    )
        .into_response()
}

/// Generate Nova DOT's reply to the latest channel context and post it as a
/// `kind='system'` message. Auth-free core — exposed for integration tests
/// (inject a stub `NovaLlm` via `AppState::with_nova_llm`, and optionally a stub
/// `CtxClient` via `AppState::with_ctx`). Returns the reply message id, or `None`
/// when Nova is unconfigured or the model returns empty; a blown reply budget
/// surfaces as `Err` so the caller posts the "unavailable" line (M14 parity).
///
/// When ctx tools are configured ([`AppState::ctx`] is `Some`), the reply runs a
/// model-driven tool-call loop ([`run_tool_loop`]); otherwise it is a single
/// tools-less model call — byte-identical to committed M14.
pub async fn run_nova_reply(state: &AppState, cid: &str) -> NovaResult<Option<String>> {
    let Some(nova) = state.nova_llm.clone() else {
        return Ok(None);
    };
    let context = recent_context(state, cid, nova.context_messages).await?;
    // Effective system prompt = global base + this channel's admin-set addendum.
    let channel_prompt = channel_nova_prompt(state, cid).await?;
    let system_prompt = effective_system_prompt(&nova.system_prompt, channel_prompt.as_deref());
    let mut messages = build_chat_messages(&system_prompt, &context);
    // ctx tools are offered ONLY when configured — the no-ctx path is a single
    // tools-less model call (committed-M14 behaviour).
    let tools = if state.ctx.is_some() {
        crate::server::ctx::openai_tool_specs()
    } else {
        Vec::new()
    };
    // Bound the WHOLE reply (the tool loop + every model call) by a wall-clock
    // budget; a blown budget surfaces as `Err` so the caller posts the visible
    // "unavailable" line (like any generation failure, M14 parity), never a hang.
    let reply = match tokio::time::timeout(
        std::time::Duration::from_secs(nova.reply_budget_secs),
        run_tool_loop(state, &nova, &mut messages, &tools),
    )
    .await
    {
        Ok(result) => result?,
        Err(_elapsed) => {
            tracing::warn!(channel = %cid, "nova reply budget elapsed");
            return Err("nova reply budget elapsed".into());
        }
    };
    let reply: String = reply.trim().chars().take(NOVA_REPLY_MAX_CHARS).collect();
    if reply.is_empty() {
        return Ok(None);
    }
    let id = persist_nova_dot_message(state, cid, &reply).await?;
    Ok(Some(id))
}

/// Drive the model↔ctx-tool loop: call the model; if it answers with text, that
/// is the reply; if it requests tools, echo the call, dispatch each to ctx, append
/// the results as `tool` turns, and call again — up to `max_tool_iters` rounds,
/// then one tools-disabled "squeeze" so a capped-out model still produces a plain
/// reply. Each dispatch is TOTAL (always a model-readable string), so a bad tool
/// call or a ctx outage degrades the answer without failing the reply. When tools
/// is empty (ctx unconfigured) or `max_tool_iters == 0`, this is a single model call.
async fn run_tool_loop(
    state: &AppState,
    nova: &NovaLlm,
    messages: &mut Vec<ChatMessage>,
    tools: &[serde_json::Value],
) -> NovaResult<String> {
    for _ in 0..nova.max_tool_iters {
        match nova.chat(messages, tools).await? {
            NovaTurn::Text(text) => return Ok(text),
            NovaTurn::ToolCalls(calls) => {
                // Echo the assistant's tool-call turn FIRST, then exactly one `tool`
                // message per call id (the chat template requires the pairing).
                messages.push(ChatMessage::assistant_tool_calls(calls.clone()));
                for call in &calls {
                    let out = dispatch_ctx_tool(state, nova, call).await;
                    messages.push(ChatMessage::tool_result(
                        call.id.clone(),
                        call.function.name.clone(),
                        out,
                    ));
                }
            }
        }
    }
    // Iteration cap hit while the model is still requesting tools: one final
    // tools-disabled call so we never post a dangling tool-call turn.
    match nova.chat(messages, &[]).await? {
        NovaTurn::Text(text) => Ok(text),
        NovaTurn::ToolCalls(_) => Ok(String::new()),
    }
}

/// Dispatch ONE model-requested tool call to ctx, ALWAYS returning a model-readable
/// string (never an `Err` into the loop): a non-configured store, malformed /
/// non-object arguments, an unknown tool name, or a ctx failure all become an
/// `"error: …"` line the model can read and recover from. The result is truncated
/// to bound the small model's context.
async fn dispatch_ctx_tool(state: &AppState, nova: &NovaLlm, call: &ToolCall) -> String {
    let Some(ctx) = state.ctx.as_ref() else {
        return "error: knowledge store not configured".to_string();
    };
    let raw = call.function.arguments.trim();
    let args: serde_json::Value = if raw.is_empty() {
        serde_json::json!({})
    } else {
        match serde_json::from_str(raw) {
            Ok(v @ serde_json::Value::Object(_)) => v,
            Ok(_) => return "error: tool arguments must be a JSON object".to_string(),
            Err(e) => {
                tracing::warn!(error = %e, tool = %call.function.name, "nova tool args parse failed");
                return format!("error: arguments were not valid JSON: {e}");
            }
        }
    };
    // Audit every model-driven ctx call (G3).
    tracing::info!(tool = %call.function.name, args = %args, "nova ctx tool call");
    match ctx.call_tool(&call.function.name, args).await {
        Ok(text) if text.trim().is_empty() => "(no results)".to_string(),
        Ok(text) => text.chars().take(nova.tool_result_max_chars).collect(),
        Err(e) => {
            tracing::warn!(error = %e, tool = %call.function.name, "nova ctx tool failed");
            format!("error: tool '{}' failed: {e}", call.function.name)
        }
    }
}

/// Best-effort "Nova is unavailable" system line, posted when generation fails.
async fn post_unavailable(state: &AppState, cid: &str) {
    if let Err(e) = persist_nova_dot_message(state, cid, NOVA_UNAVAILABLE_BODY).await {
        tracing::error!(error = %e, "posting nova-unavailable line failed");
    }
}

/// Persist `body` as a Nova DOT `kind='system'` message in `cid`, then notify +
/// emit exactly like every other message write. Shared by `/novasay`, the `/nova`
/// reply, and the unavailable-fallback line.
async fn persist_nova_dot_message(
    state: &AppState,
    cid: &str,
    body: &str,
) -> surrealdb::Result<String> {
    let id = persist_message(
        state,
        cid,
        NOVA_DOT_ACCOUNT,
        None,
        body,
        "system",
        &[],
        None,
        &[],
        None,
        false,
    )
    .await?;
    crate::server::push::notify_new_message(
        state.clone(),
        id.clone(),
        NOVA_DOT_ACCOUNT.to_string(),
    );
    state.emit(SyncEvent::MessageCreated {
        channel_id: cid.to_string(),
    });
    Ok(id)
}

// ---------------------------------------------------------------------------
// Model context
// ---------------------------------------------------------------------------

/// One channel message reduced to what Nova needs for context: who said it
/// (author key + display) and the text. `author_key == "nova_dot"` marks Nova's
/// own prior turns (mapped to the `assistant` role). Public so the pure
/// [`build_chat_messages`] mapping can be unit-tested.
#[derive(SurrealValue, Clone, Debug)]
pub struct NovaContextRow {
    pub author_key: String,
    pub author_display: String,
    pub body: String,
}

/// The newest `n` live messages in `cid`, oldest-first (chronological), for the
/// model context. Lightweight projection (no attachments/personas/replies) —
/// Nova only needs speaker + text. The admin's just-posted prompt is the newest
/// row, so it is the final user turn.
async fn recent_context(
    state: &AppState,
    cid: &str,
    n: usize,
) -> surrealdb::Result<Vec<NovaContextRow>> {
    // SurrealDB 3.1 requires the ORDER BY idiom (`sent_at`) to appear in the
    // projection (same trap as `system_messages`'s `position`), so it rides
    // along on this internal row and is dropped when we map to NovaContextRow.
    #[derive(SurrealValue)]
    struct Row {
        author_key: String,
        author_display: String,
        body: String,
        #[allow(dead_code)] // present only to satisfy the ORDER BY idiom rule
        sent_at: surrealdb::types::Datetime,
    }
    let mut resp = state
        .db
        .query(
            "SELECT meta::id(author) AS author_key,
                    (author.display_name ?: author.username) AS author_display,
                    body,
                    sent_at
             FROM message
             WHERE channel = type::record('channel', $cid) AND deleted_at = NONE
             ORDER BY sent_at DESC LIMIT $n;",
        )
        .bind(("cid", cid.to_string()))
        .bind(("n", n as i64))
        .await?
        .check()?;
    let mut rows: Vec<Row> = resp.take(0)?;
    rows.reverse(); // DESC fetch → chronological transcript for the model
    Ok(rows
        .into_iter()
        .map(|r| NovaContextRow {
            author_key: r.author_key,
            author_display: r.author_display,
            body: r.body,
        })
        .collect())
}

/// Map channel context + the system prompt into an OpenAI chat-message list.
/// Nova's own prior messages (`author_key == "nova_dot"`) become `assistant`
/// turns; everyone else becomes a `user` turn prefixed with their display name
/// so the model can follow a multi-speaker channel. Pure + unit-tested.
pub fn build_chat_messages(system_prompt: &str, context: &[NovaContextRow]) -> Vec<ChatMessage> {
    let mut out = Vec::with_capacity(context.len() + 1);
    out.push(ChatMessage {
        role: "system",
        content: system_prompt.to_string(),
        ..Default::default()
    });
    for row in context {
        if row.author_key == NOVA_DOT_ACCOUNT {
            out.push(ChatMessage {
                role: "assistant",
                content: row.body.clone(),
                ..Default::default()
            });
        } else {
            out.push(ChatMessage {
                role: "user",
                content: format!("{}: {}", row.author_display, row.body),
                ..Default::default()
            });
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Per-channel Nova system prompt (admin-set; appended to the global base)
// ---------------------------------------------------------------------------

/// Combine the global Nova DOT base prompt with a channel-specific ADDENDUM
/// (append, not replace — keeps Nova's identity, adds channel flavor). An empty
/// or absent channel addendum → the global base alone. Pure + unit-tested.
pub fn effective_system_prompt(global: &str, channel: Option<&str>) -> String {
    match channel.map(str::trim).filter(|c| !c.is_empty()) {
        Some(addendum) => format!("{}\n\n{}", global.trim_end(), addendum),
        None => global.to_string(),
    }
}

/// The channel's stored per-channel Nova system prompt (`channel.nova_prompt`),
/// or `None` when unset (or the channel is missing).
async fn channel_nova_prompt(state: &AppState, cid: &str) -> surrealdb::Result<Option<String>> {
    let mut resp = state
        .db
        .query("SELECT VALUE nova_prompt FROM type::record('channel', $cid);")
        .bind(("cid", cid.to_string()))
        .await?
        .check()?;
    // SELECT VALUE → an array of 0/1 field values; a NONE field surfaces as null.
    let vals: Vec<Option<String>> = resp.take(0)?;
    Ok(vals.into_iter().flatten().next())
}

/// Auth-free core: set (`Some`) or clear (`None`/empty) the channel's Nova
/// system-prompt addendum, after the membership + text-channel gate. Exposed for
/// integration tests (the `is_admin` gate lives in [`set_nova_prompt`]).
pub async fn set_nova_prompt_core(
    state: &AppState,
    cid: &str,
    account: &str,
    prompt: Option<String>,
) -> Result<(), NovaError> {
    require_text_channel(state, cid, account).await?;
    let prompt = prompt
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty());
    if let Some(ref p) = prompt {
        if p.chars().count() > NOVA_PROMPT_MAX_CHARS {
            return Err(NovaError::BadRequest("nova prompt too long"));
        }
    }
    // Explicit NONE clear vs SET — avoids any null-vs-NONE ambiguity in binding.
    let q = match prompt {
        Some(p) => state
            .db
            .query("UPDATE type::record('channel', $cid) SET nova_prompt = $prompt;")
            .bind(("cid", cid.to_string()))
            .bind(("prompt", p)),
        None => state
            .db
            .query("UPDATE type::record('channel', $cid) SET nova_prompt = NONE;")
            .bind(("cid", cid.to_string())),
    };
    q.await
        .map_err(NovaError::Storage)?
        .check()
        .map_err(NovaError::Storage)?;
    Ok(())
}

/// Auth-free core: read the channel's current Nova system-prompt addendum after
/// the membership + text gate. Exposed for tests.
pub async fn get_nova_prompt_core(
    state: &AppState,
    cid: &str,
    account: &str,
) -> Result<Option<String>, NovaError> {
    require_text_channel(state, cid, account).await?;
    channel_nova_prompt(state, cid)
        .await
        .map_err(NovaError::Storage)
}

/// PUT /channels/{cid}/nova-prompt — admin-only. Set or clear (empty body) this
/// channel's Nova DOT system-prompt addendum (appended to the global base).
#[tracing::instrument(skip_all, fields(account = %account.0, channel = %cid))]
pub async fn set_nova_prompt(
    State(state): State<AppState>,
    Path(cid): Path<String>,
    account: AuthAccount,
    payload: Result<Json<NovaPromptRequest>, JsonRejection>,
) -> Response {
    if let Some(resp) = admin_gate(&state, &account.0).await {
        return resp;
    }
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    match set_nova_prompt_core(&state, &cid, &account.0, req.prompt).await {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => nova_error_response(e, "nova-prompt set"),
    }
}

/// GET /channels/{cid}/nova-prompt — admin-only. The channel's stored Nova
/// system-prompt addendum (None when unset), for the settings field to display.
#[tracing::instrument(skip_all, fields(account = %account.0, channel = %cid))]
pub async fn get_nova_prompt(
    State(state): State<AppState>,
    Path(cid): Path<String>,
    account: AuthAccount,
) -> Response {
    if let Some(resp) = admin_gate(&state, &account.0).await {
        return resp;
    }
    match get_nova_prompt_core(&state, &cid, &account.0).await {
        Ok(prompt) => (StatusCode::OK, Json(NovaPromptResponse { prompt })).into_response(),
        Err(e) => nova_error_response(e, "nova-prompt get"),
    }
}
