//! Nova DOT's language-model backend (the `/nova` command). ssr-only.
//!
//! A thin OpenAI-compatible chat client to the project's local "llama-chat"
//! model — Qwen3.6 27B served by llama.cpp (`:server-cuda`) on novahome at
//! `:8091`, OpenAI protocol, no auth. Sampling is baked into the model server
//! (`--top-k/--top-p/--presence-penalty/--reasoning off`), so a request carries
//! only the messages plus a `max_tokens` cap — never sampling params.
//!
//! [`NovaLlm`] is built from env at startup ([`NovaLlm::from_env`]) and held as
//! `Option<Arc<NovaLlm>>` on `AppState`. `None` (env unset) disables `/nova`
//! gracefully — the handler returns 503 and `/novasay` is unaffected. Tests
//! inject a [`NovaLlm::stub`] (via `AppState::with_nova_llm`) so the `/nova`
//! flow is provable without a network model.
//!
//! NB: "Nova DOT" here is the reserved `account:nova_dot` system bot the reply
//! is authored as — NOT the `nova-mcp` bridge's "Nova" user account (a separate
//! entity that drives the app over MCP). This module is the app *calling* a
//! model; the bridge is a model *driving* the app.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// One chat turn sent to the model. `role` is `"system"`, `"user"`, or
/// `"assistant"` (OpenAI chat-completions shape).
#[derive(Clone, Debug, Serialize)]
pub struct ChatMessage {
    pub role: &'static str,
    pub content: String,
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
/// Default Nova DOT voice. Overridable via `NOVA_LLM_SYSTEM_PROMPT`.
const DEFAULT_SYSTEM_PROMPT: &str = "You are Nova DOT, the in-house commentator and assistant of this self-hosted roleplay chat platform. You speak as a single, distinct character named \"Nova DOT\". Read the recent channel conversation and reply helpfully and in character to the most recent message, matching the channel's tone. Keep replies concise. Address the channel members directly; never narrate as another participant.";

/// Nova DOT's model backend plus the knobs the `/nova` handler reads.
pub struct NovaLlm {
    /// How many recent channel messages to feed the model as context.
    pub context_messages: usize,
    /// The system prompt establishing Nova DOT's voice.
    pub system_prompt: String,
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
    /// Test backend: returns this canned reply, no network.
    Stub(String),
    /// Test backend: echoes back the assembled SYSTEM message content, so a test
    /// can assert what system prompt was built (e.g. the per-channel addendum).
    Echo,
}

impl NovaLlm {
    /// Build from env, or `None` when `NOVA_LLM_URL` is unset/empty (which
    /// disables `/nova`). Mirrors the `PushSender::from_env` idiom — every other
    /// var falls back to a sane default for the novahome llama.cpp endpoint.
    pub fn from_env() -> Option<Arc<NovaLlm>> {
        let url = env_nonempty("NOVA_LLM_URL")?;
        let model = env_nonempty("NOVA_LLM_MODEL").unwrap_or_else(|| DEFAULT_MODEL.to_string());
        let max_tokens = env_parse("NOVA_LLM_MAX_TOKENS").unwrap_or(DEFAULT_MAX_TOKENS);
        let timeout_secs = env_parse("NOVA_LLM_TIMEOUT_SECS").unwrap_or(DEFAULT_TIMEOUT_SECS);
        let context_messages =
            env_parse("NOVA_CONTEXT_MESSAGES").unwrap_or(DEFAULT_CONTEXT_MESSAGES);
        let system_prompt = env_nonempty("NOVA_LLM_SYSTEM_PROMPT")
            .unwrap_or_else(|| DEFAULT_SYSTEM_PROMPT.to_string());
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .ok()?;
        Some(Arc::new(NovaLlm {
            context_messages,
            system_prompt,
            backend: Backend::Http {
                client,
                url,
                model,
                max_tokens,
            },
        }))
    }

    /// A no-network backend returning `reply` verbatim — integration tests
    /// inject this via `AppState::with_nova_llm`.
    pub fn stub(reply: impl Into<String>) -> Arc<NovaLlm> {
        Arc::new(NovaLlm {
            context_messages: DEFAULT_CONTEXT_MESSAGES,
            system_prompt: DEFAULT_SYSTEM_PROMPT.to_string(),
            backend: Backend::Stub(reply.into()),
        })
    }

    /// A no-network backend that echoes the assembled SYSTEM prompt back as the
    /// "reply" — tests inject this to assert what system prompt was built (e.g.
    /// that a per-channel addendum was appended). `with_nova_llm` in `AppState`.
    pub fn stub_echo() -> Arc<NovaLlm> {
        Arc::new(NovaLlm {
            context_messages: DEFAULT_CONTEXT_MESSAGES,
            system_prompt: DEFAULT_SYSTEM_PROMPT.to_string(),
            backend: Backend::Echo,
        })
    }

    /// Send `messages` to the model and return the assistant's reply text
    /// (trimmed). A non-2xx status, a transport error, or a missing choice all
    /// surface as `Err`.
    pub async fn complete(&self, messages: Vec<ChatMessage>) -> NovaResult<String> {
        match &self.backend {
            Backend::Stub(reply) => Ok(reply.clone()),
            Backend::Echo => Ok(messages
                .iter()
                .find(|m| m.role == "system")
                .map(|m| m.content.clone())
                .unwrap_or_default()),
            Backend::Http {
                client,
                url,
                model,
                max_tokens,
            } => {
                #[derive(Serialize)]
                struct ChatRequest<'a> {
                    model: &'a str,
                    messages: &'a [ChatMessage],
                    max_tokens: u32,
                    stream: bool,
                }
                #[derive(Deserialize)]
                struct ChatResponse {
                    choices: Vec<Choice>,
                }
                #[derive(Deserialize)]
                struct Choice {
                    message: ChoiceMessage,
                }
                #[derive(Deserialize)]
                struct ChoiceMessage {
                    content: String,
                }
                let resp = client
                    .post(url)
                    .json(&ChatRequest {
                        model,
                        messages: &messages,
                        max_tokens: *max_tokens,
                        stream: false,
                    })
                    .send()
                    .await?
                    .error_for_status()?;
                let body: ChatResponse = resp.json().await?;
                let content = body
                    .choices
                    .into_iter()
                    .next()
                    .map(|c| c.message.content)
                    .ok_or("nova: model returned no choices")?;
                Ok(content.trim().to_string())
            }
        }
    }
}

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
