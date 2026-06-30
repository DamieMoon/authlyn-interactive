//! Nova DOT's language-model backend (the `/nova` command). ssr-only.
//!
//! A thin OpenAI-compatible chat client to the project's local "llama-chat"
//! model — Qwen3.6 27B served by llama.cpp (`:server-cuda`) on novahome at
//! `:8091`, OpenAI protocol, no auth. Sampling is baked into the model server
//! (`--top-k/--top-p/--presence-penalty/--reasoning off`), so a request carries
//! only the messages, a `max_tokens` cap, and — when Nova's ctx tools are
//! configured — the OpenAI `tools` array.
//!
//! [`NovaLlm`] is built from env at startup ([`NovaLlm::from_env`]) and held as
//! `Option<Arc<NovaLlm>>` on `AppState`. `None` (env unset) disables `/nova`
//! gracefully — the handler returns 503 and `/novasay` is unaffected. Tests
//! inject [`NovaLlm::stub`] / [`NovaLlm::stub_echo`] / [`NovaLlm::stub_script`]
//! (via `AppState::with_nova_llm`) so the `/nova` flow — including the tool-call
//! loop — is provable without a network model.
//!
//! ## Tool calling
//! [`NovaLlm::chat`] returns a [`NovaTurn`]: either the model's final `Text`, or
//! a batch of `ToolCalls` the caller must dispatch (to [`crate::server::ctx`])
//! and feed back as `tool`-role messages before calling again. The decision is
//! made purely on the presence of `tool_calls` — `finish_reason` is logged, never
//! the gate (the abliterated Q3 quant emits tool calls with `finish_reason:"stop"`).
//! The loop itself (cap, dispatch, degrade) lives in
//! [`crate::server::messages::run_nova_reply`].
//!
//! NB: "Nova DOT" here is the reserved `account:nova_dot` system bot the reply
//! is authored as — NOT the `nova-mcp` bridge's "Nova" user account (a separate
//! entity that drives the app over MCP). This module is the app *calling* a
//! model; the bridge is a model *driving* the app.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One chat turn sent to the model. `role` is `"system"`, `"user"`,
/// `"assistant"`, or `"tool"` (OpenAI chat-completions shape). The tool fields
/// are skipped when empty, so a plain turn serializes byte-identically to the
/// pre-tool `{role, content}` shape.
#[derive(Clone, Debug, Default, Serialize)]
pub struct ChatMessage {
    pub role: &'static str,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub content: String,
    /// Set on an `assistant` turn that requests tool calls (echoed before the
    /// `tool` results so the model sees its own call).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// Set on a `tool` turn — which call (`id`) this result answers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Set on a `tool` turn — the tool name that produced this result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl ChatMessage {
    /// An `assistant` turn that only requests tool calls (no text), echoed into
    /// the transcript before the matching `tool` results. Empty call `kind`s are
    /// normalized to `"function"` so the echoed turn is well-formed for the
    /// model's chat template.
    pub fn assistant_tool_calls(mut tool_calls: Vec<ToolCall>) -> Self {
        for tc in &mut tool_calls {
            if tc.kind.is_empty() {
                tc.kind = "function".to_string();
            }
        }
        ChatMessage {
            role: "assistant",
            tool_calls,
            ..Default::default()
        }
    }

    /// A `tool` result turn answering one tool call (`tool_call_id`) with `content`.
    pub fn tool_result(tool_call_id: String, name: String, content: String) -> Self {
        ChatMessage {
            role: "tool",
            content,
            tool_call_id: Some(tool_call_id),
            name: Some(name),
            ..Default::default()
        }
    }
}

/// One tool call requested by the model (OpenAI shape) — the SAME type both
/// directions: parsed from the model's response AND serialized back in the
/// echoed `assistant` turn.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCall {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    pub function: FunctionCall,
}

/// The function name + JSON-encoded argument STRING of a [`ToolCall`] (OpenAI
/// sends `arguments` as a string, not an object).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    #[serde(default)]
    pub arguments: String,
}

/// The outcome of one [`NovaLlm::chat`] round-trip: the model's final text, or a
/// batch of tool calls to dispatch and feed back.
#[derive(Clone, Debug)]
pub enum NovaTurn {
    Text(String),
    ToolCalls(Vec<ToolCall>),
}

/// Result of a model call. The error is boxed for logging only — the `/nova`
/// handler degrades a failure to a visible "Nova is unavailable" system line.
pub type NovaResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Default model tag/path. The llama.cpp server is single-model and lenient
/// about this field, so the exact value is mostly informational.
const DEFAULT_MODEL: &str = "/models/Qwen3.6-27B-abliterated-rMAX.i1-Q3_K_M.gguf";
/// Default request timeout — a 27B model on the 4080 can take many seconds.
const DEFAULT_TIMEOUT_SECS: u64 = 120;
/// Default reply length cap (tokens). Bounds latency and keeps a reply inside
/// the model's 8192-token-per-slot context budget.
const DEFAULT_MAX_TOKENS: u32 = 768;
/// Default count of recent channel messages fed to the model as context.
const DEFAULT_CONTEXT_MESSAGES: usize = 24;
/// Default max model↔tool round-trips before a tools-disabled squeeze (bounds
/// the small model's tool loop). `0` ⇒ tools never offered ⇒ plain-chat parity.
const DEFAULT_MAX_TOOL_ITERS: usize = 4;
/// Default per-tool-result char cap (bounds what a tool's text adds to context).
const DEFAULT_TOOL_RESULT_MAX_CHARS: usize = 3_000;
/// Default wall-clock budget for the whole reply (the tool loop + every model
/// call). Generous — a single 27B turn can take many seconds, and a reply may
/// take several turns.
const DEFAULT_REPLY_BUDGET_SECS: u64 = 240;
/// Default Nova DOT voice. Overridable via `NOVA_LLM_SYSTEM_PROMPT`.
const DEFAULT_SYSTEM_PROMPT: &str = "You are Nova DOT, the in-house commentator and assistant of this self-hosted roleplay chat platform. You speak as a single, distinct character named \"Nova DOT\". Read the recent channel conversation and reply helpfully and in character to the most recent message, matching the channel's tone. Keep replies concise. Address the channel members directly; never narrate as another participant.";

/// Nova DOT's model backend plus the knobs the `/nova` handler reads.
pub struct NovaLlm {
    /// How many recent channel messages to feed the model as context.
    pub context_messages: usize,
    /// The system prompt establishing Nova DOT's voice.
    pub system_prompt: String,
    /// Max model↔tool round-trips before the tools-disabled squeeze. `0` = tools
    /// never offered (plain-chat kill-switch).
    pub max_tool_iters: usize,
    /// Per-tool-result char cap fed back into the model context.
    pub tool_result_max_chars: usize,
    /// Wall-clock budget (seconds) for the whole reply (loop + every model call).
    pub reply_budget_secs: u64,
    backend: Backend,
}

enum Backend {
    /// Real model over HTTP (OpenAI `/v1/chat/completions`).
    Http {
        client: reqwest::Client,
        url: String,
        model: String,
        max_tokens: u32,
    },
    /// Test backend: returns this canned reply as `Text`, no network.
    Stub(String),
    /// Test backend: echoes back the assembled SYSTEM message content as `Text`,
    /// so a test can assert what system prompt was built.
    Echo,
    /// Test backend: a scripted sequence of [`NovaTurn`]s, popped per `chat` call
    /// and **sticky on the last** — so a script ending in `Text` terminates the
    /// loop and one ending in `ToolCalls` exercises the iteration cap.
    Script(Mutex<VecDeque<NovaTurn>>),
}

impl NovaLlm {
    /// Build from env, or `None` when `NOVA_LLM_URL` is unset/empty (which
    /// disables `/nova`). Every other var falls back to a sane default for the
    /// novahome llama.cpp endpoint.
    pub fn from_env() -> Option<Arc<NovaLlm>> {
        let url = env_nonempty("NOVA_LLM_URL")?;
        let model = env_nonempty("NOVA_LLM_MODEL").unwrap_or_else(|| DEFAULT_MODEL.to_string());
        let max_tokens = env_parse("NOVA_LLM_MAX_TOKENS").unwrap_or(DEFAULT_MAX_TOKENS);
        let timeout_secs = env_parse("NOVA_LLM_TIMEOUT_SECS").unwrap_or(DEFAULT_TIMEOUT_SECS);
        let context_messages =
            env_parse("NOVA_CONTEXT_MESSAGES").unwrap_or(DEFAULT_CONTEXT_MESSAGES);
        let max_tool_iters = env_parse("NOVA_TOOL_ITERS").unwrap_or(DEFAULT_MAX_TOOL_ITERS);
        let tool_result_max_chars =
            env_parse("NOVA_TOOL_RESULT_MAX_CHARS").unwrap_or(DEFAULT_TOOL_RESULT_MAX_CHARS);
        let reply_budget_secs =
            env_parse("NOVA_REPLY_BUDGET_SECS").unwrap_or(DEFAULT_REPLY_BUDGET_SECS);
        let system_prompt = env_nonempty("NOVA_LLM_SYSTEM_PROMPT")
            .unwrap_or_else(|| DEFAULT_SYSTEM_PROMPT.to_string());
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .ok()?;
        Some(Arc::new(NovaLlm {
            context_messages,
            system_prompt,
            max_tool_iters,
            tool_result_max_chars,
            reply_budget_secs,
            backend: Backend::Http {
                client,
                url,
                model,
                max_tokens,
            },
        }))
    }

    /// A no-network backend returning `reply` verbatim as `Text` — integration
    /// tests inject this via `AppState::with_nova_llm`.
    pub fn stub(reply: impl Into<String>) -> Arc<NovaLlm> {
        Arc::new(NovaLlm::test(Backend::Stub(reply.into())))
    }

    /// A no-network backend that echoes the assembled SYSTEM prompt back as the
    /// "reply" — tests inject this to assert what system prompt was built.
    pub fn stub_echo() -> Arc<NovaLlm> {
        Arc::new(NovaLlm::test(Backend::Echo))
    }

    /// A no-network backend driven by a scripted sequence of [`NovaTurn`]s
    /// (sticky on the last) — drives the tool-call loop deterministically.
    pub fn stub_script(turns: Vec<NovaTurn>) -> Arc<NovaLlm> {
        Arc::new(NovaLlm::test(Backend::Script(Mutex::new(VecDeque::from(
            turns,
        )))))
    }

    /// Shared test-backend constructor (defaults for every knob).
    fn test(backend: Backend) -> NovaLlm {
        NovaLlm {
            context_messages: DEFAULT_CONTEXT_MESSAGES,
            system_prompt: DEFAULT_SYSTEM_PROMPT.to_string(),
            max_tool_iters: DEFAULT_MAX_TOOL_ITERS,
            tool_result_max_chars: DEFAULT_TOOL_RESULT_MAX_CHARS,
            reply_budget_secs: DEFAULT_REPLY_BUDGET_SECS,
            backend,
        }
    }

    /// Send `messages` (and, when non-empty, the `tools` array) to the model and
    /// return the [`NovaTurn`]: the final assistant text, or the tool calls the
    /// model requested. A non-2xx status, a transport error, or a missing choice
    /// all surface as `Err`. Tools and `tool_choice` are omitted from the request
    /// entirely when `tools` is empty — so the no-tools path is byte-identical to
    /// the pre-tool request.
    pub async fn chat(&self, messages: &[ChatMessage], tools: &[Value]) -> NovaResult<NovaTurn> {
        match &self.backend {
            Backend::Stub(reply) => Ok(NovaTurn::Text(reply.clone())),
            Backend::Echo => Ok(NovaTurn::Text(
                messages
                    .iter()
                    .find(|m| m.role == "system")
                    .map(|m| m.content.clone())
                    .unwrap_or_default(),
            )),
            Backend::Script(turns) => Ok(pop_sticky(
                &mut turns.lock().expect("nova script mutex poisoned"),
            )),
            Backend::Http {
                client,
                url,
                model,
                max_tokens,
            } => {
                let req = ChatRequest {
                    model,
                    messages,
                    max_tokens: *max_tokens,
                    stream: false,
                    tools,
                    tool_choice: if tools.is_empty() { None } else { Some("auto") },
                };
                let body: ChatResponse = client
                    .post(url)
                    .json(&req)
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await?;
                let choice = body
                    .choices
                    .into_iter()
                    .next()
                    .ok_or("nova: model returned no choices")?;
                if let Some(fr) = &choice.finish_reason {
                    tracing::debug!(finish_reason = %fr, "nova model finish_reason");
                }
                Ok(classify(choice.message))
            }
        }
    }
}

/// Pure decision rule: a response message with any `tool_calls` is a tool turn
/// (REGARDLESS of `finish_reason`); otherwise it is the final text (trimmed).
fn classify(message: ChoiceMessage) -> NovaTurn {
    if message.tool_calls.is_empty() {
        NovaTurn::Text(message.content.unwrap_or_default().trim().to_string())
    } else {
        NovaTurn::ToolCalls(message.tool_calls)
    }
}

/// Pop the next scripted turn, sticking on the last (so the loop is driven to a
/// deterministic end). An empty queue yields an empty `Text`.
fn pop_sticky(q: &mut VecDeque<NovaTurn>) -> NovaTurn {
    if q.len() > 1 {
        q.pop_front().expect("len > 1")
    } else {
        q.front().cloned().unwrap_or(NovaTurn::Text(String::new()))
    }
}

/// serde `skip_serializing_if` for the borrowed `tools` slice (a free fn — the
/// `<[_]>::is_empty` path can't take the `&&[Value]` serde hands the predicate).
fn slice_is_empty(tools: &&[Value]) -> bool {
    tools.is_empty()
}

// ---------------------------------------------------------------------------
// Wire structs (OpenAI chat-completions)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    max_tokens: u32,
    stream: bool,
    #[serde(skip_serializing_if = "slice_is_empty")]
    tools: &'a [Value],
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'static str>,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    #[serde(default)]
    message: ChoiceMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize, Default)]
struct ChoiceMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ToolCall>,
}

// ---------------------------------------------------------------------------
// Env helpers
// ---------------------------------------------------------------------------

/// Read an env var, trimmed, treating empty/whitespace as absent.
fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Read + parse an env var, treating absent/unparseable as `None`.
fn env_parse<T: std::str::FromStr>(key: &str) -> Option<T> {
    std::env::var(key).ok().and_then(|s| s.trim().parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn classify_prefers_tool_calls_even_with_finish_reason_stop() {
        // The abliterated Q3 emits tool_calls with finish_reason "stop" — the
        // presence of tool_calls, not finish_reason, must decide.
        let body = r#"{"choices":[{"message":{"content":null,"tool_calls":[
            {"id":"c1","type":"function","function":{"name":"query","arguments":"{\"question\":\"hi\"}"}}
        ]},"finish_reason":"stop"}]}"#;
        let resp: ChatResponse = serde_json::from_str(body).unwrap();
        let choice = resp.choices.into_iter().next().unwrap();
        match classify(choice.message) {
            NovaTurn::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].function.name, "query");
                assert_eq!(calls[0].function.arguments, r#"{"question":"hi"}"#);
            }
            NovaTurn::Text(_) => panic!("tool_calls present must classify as ToolCalls"),
        }
    }

    #[test]
    fn classify_content_only_is_text() {
        let body = r#"{"choices":[{"message":{"content":"hello there"}}]}"#;
        let resp: ChatResponse = serde_json::from_str(body).unwrap();
        let choice = resp.choices.into_iter().next().unwrap();
        assert!(matches!(classify(choice.message), NovaTurn::Text(t) if t == "hello there"));
    }

    #[test]
    fn request_includes_tools_only_when_present() {
        let msgs = vec![ChatMessage {
            role: "user",
            content: "hi".into(),
            ..Default::default()
        }];
        let specs = vec![json!({ "type": "function" })];
        let with = ChatRequest {
            model: "m",
            messages: &msgs,
            max_tokens: 10,
            stream: false,
            tools: &specs,
            tool_choice: Some("auto"),
        };
        let v = serde_json::to_value(&with).unwrap();
        assert_eq!(v["tool_choice"], "auto");
        assert!(v["tools"].is_array());

        let without = ChatRequest {
            model: "m",
            messages: &msgs,
            max_tokens: 10,
            stream: false,
            tools: &[],
            tool_choice: None,
        };
        let v = serde_json::to_value(&without).unwrap();
        assert!(v.get("tools").is_none(), "tools omitted when empty");
        assert!(
            v.get("tool_choice").is_none(),
            "tool_choice omitted when none"
        );
    }

    #[test]
    fn tool_and_plain_messages_serialize_correctly() {
        let m = ChatMessage::tool_result("call_1".into(), "query".into(), "RESULT".into());
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["role"], "tool");
        assert_eq!(v["content"], "RESULT");
        assert_eq!(v["tool_call_id"], "call_1");
        assert_eq!(v["name"], "query");

        // A plain user turn omits every tool field — byte-identical to pre-tool.
        let u = ChatMessage {
            role: "user",
            content: "hi".into(),
            ..Default::default()
        };
        let v = serde_json::to_value(&u).unwrap();
        assert_eq!(v["role"], "user");
        assert_eq!(v["content"], "hi");
        assert!(v.get("tool_calls").is_none());
        assert!(v.get("tool_call_id").is_none());
        assert!(v.get("name").is_none());
    }

    #[test]
    fn assistant_tool_calls_normalizes_empty_kind() {
        let calls = vec![ToolCall {
            id: "c1".into(),
            kind: String::new(),
            function: FunctionCall {
                name: "query".into(),
                arguments: "{}".into(),
            },
        }];
        let m = ChatMessage::assistant_tool_calls(calls);
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["role"], "assistant");
        assert_eq!(v["tool_calls"][0]["type"], "function");
        assert_eq!(v["tool_calls"][0]["function"]["name"], "query");
        assert!(
            v.get("content").is_none(),
            "no text on a pure tool-call turn"
        );
    }

    #[test]
    fn pop_sticky_pops_then_sticks_on_last() {
        let mut q = VecDeque::from(vec![NovaTurn::Text("a".into()), NovaTurn::Text("b".into())]);
        assert!(matches!(pop_sticky(&mut q), NovaTurn::Text(t) if t == "a"));
        assert!(matches!(pop_sticky(&mut q), NovaTurn::Text(t) if t == "b"));
        assert!(
            matches!(pop_sticky(&mut q), NovaTurn::Text(t) if t == "b"),
            "sticks on the last turn"
        );
    }
}
