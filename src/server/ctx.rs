//! Nova DOT's **ctx tool surface** — the model-driven knowledge-store bridge. ssr-only.
//!
//! Gives Nova DOT the SAME ctx tools Claude Code uses (`query` / `search` / `get` /
//! `recent` / `store`) as model-driven tool calls: the Qwen model decides when to call,
//! [`run_nova_reply`](crate::server::messages::run_nova_reply) dispatches the call here,
//! feeds the text result back, and the model continues. Sibling to
//! [`crate::server::nova_llm`] (same reqwest / env-gate idiom); held as
//! `Option<Arc<CtxClient>>` on `AppState`. `None` (env unset) disables tools gracefully —
//! Nova replies on channel context + prompt alone, exactly as committed M14.
//!
//! ## Transport — MCP JSON-RPC over loopback `/mcp`
//! POSTs standard MCP JSON-RPC 2.0 (`tools/call`) to `{CTX_BASE_URL}/mcp`, reusing the
//! `X-Context-Key` header. The request advertises both Accept types the Streamable-HTTP
//! spec mandates (`application/json, text/event-stream`) — the ctx go-sdk handler rejects a
//! POST lacking `text/event-stream` with 400 *before* its JSON branch. ctx runs in
//! `Stateless` + `JSONResponse` mode, so the *response* is plain JSON we parse directly (no
//! SSE, no session, no `initialize`). The base URL is **loopback** on novahome (`reqwest`
//! here is built without a TLS feature → plaintext http only); it is the SAME ctx server/data
//! as the public `https://ctx.damienmoon.sh/mcp`.
//!
//! ## Isolation
//! There is **no app-side tag filter** (parity with Claude Code). What Nova can reach is
//! bounded **server-side** by its non-admin service key (`CTX_NOVA_MEMORY_KEY`, minted
//! `--home nova`): reads see the `nova` + `shared` scopes only, writes pin to `nova`. Only
//! the owner-admin can invoke `/nova`. If stricter per-guild isolation is ever wanted, the
//! correct lever is a per-guild ctx **service key** (server-side scope), NOT re-adding an
//! app-side tag filter. [`call_tool`] defaults `store` sensitivity to `internal`.
//!
//! NB: the real HTTP path is exercised only against the live ctx on the deck; unit tests
//! drive the pure helpers and a [`CtxClient::stub`].

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Default ctx base URL — same-host loopback on novahome. The client appends [`MCP_PATH`].
const DEFAULT_BASE_URL: &str = "http://127.0.0.1:8080";
/// Default request timeout. A tool call rides Nova's (already-async, off-request) reply
/// path, so keep it modest — a slow/absent ctx must degrade to an error string, never stall.
const DEFAULT_TIMEOUT_SECS: u64 = 10;
/// The MCP endpoint path appended to the base URL.
const MCP_PATH: &str = "/mcp";

/// Result of a ctx call. Boxed error: a transport / non-2xx / JSON-RPC `error` / unknown
/// tool surfaces as `Err`; the loop turns that into a model-readable `"error: …"` string.
pub type CtxResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

// ---------------------------------------------------------------------------
// Tool registry — the single source of truth (client + model can't disagree)
// ---------------------------------------------------------------------------

/// One ctx tool exposed to the model. `parameters` is a fn pointer because a
/// `serde_json::Value` JSON-schema isn't const-constructible.
pub struct CtxTool {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: fn() -> Value,
}

/// The five ctx tools, matching the live ctx MCP surface. The ONLY source of truth for
/// both [`openai_tool_specs`] (what crosses to the model) and [`is_known_ctx_tool`] (the
/// dispatch guard). Descriptions stay terse + imperative (a small local model follows
/// short instructions better) and carry soft-isolation steering — guidance, not an app
/// constraint (the hard bound is the service key's scope).
pub const CTX_TOOLS: &[CtxTool] = &[
    CtxTool {
        name: "query",
        description: "Ask the knowledge store a question; returns a synthesized answer with sources. Results may be shown in a shared channel.",
        parameters: query_params,
    },
    CtxTool {
        name: "search",
        description: "Search the knowledge store for blocks (no synthesis); returns matching blocks with a preview. Results may be shown in a shared channel.",
        parameters: search_params,
    },
    CtxTool {
        name: "get",
        description: "Fetch one knowledge block in full by its id (a UUID).",
        parameters: get_params,
    },
    CtxTool {
        name: "recent",
        description: "List recently created or updated knowledge blocks.",
        parameters: recent_params,
    },
    CtxTool {
        name: "store",
        description: "Save a knowledge block (category, title, content). Use category \"nova-memory\" for durable notes; do not persist other members' private content.",
        parameters: store_params,
    },
];

fn query_params() -> Value {
    json!({
        "type": "object",
        "properties": {
            "question": { "type": "string", "description": "the question to answer" },
            "limit": { "type": "integer", "description": "max sources (default 5)" }
        },
        "required": ["question"]
    })
}
fn search_params() -> Value {
    json!({
        "type": "object",
        "properties": {
            "query": { "type": "string", "description": "search text" },
            "category": { "type": "string" },
            "tags": { "type": "array", "items": { "type": "string" } },
            "limit": { "type": "integer", "description": "max results (default 10)" }
        }
    })
}
fn get_params() -> Value {
    json!({
        "type": "object",
        "properties": { "id": { "type": "string", "description": "block UUID" } },
        "required": ["id"]
    })
}
fn recent_params() -> Value {
    json!({
        "type": "object",
        "properties": {
            "limit": { "type": "integer", "description": "max blocks (default 10, max 50)" },
            "category": { "type": "string" }
        }
    })
}
fn store_params() -> Value {
    json!({
        "type": "object",
        "properties": {
            "category": { "type": "string" },
            "title": { "type": "string" },
            "content": { "type": "string" },
            "tags": { "type": "array", "items": { "type": "string" } },
            "sensitivity": { "type": "string", "enum": ["credentials", "personal", "internal", "public"] }
        },
        "required": ["category", "title", "content"]
    })
}

/// The OpenAI-shaped `tools` array — the only thing about ctx that crosses into the model.
/// Derived from [`CTX_TOOLS`] so a spec can never drift from the dispatch guard.
pub fn openai_tool_specs() -> Vec<Value> {
    CTX_TOOLS
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": (t.parameters)(),
                }
            })
        })
        .collect()
}

/// Exact, case-sensitive membership check against [`CTX_TOOLS`] — the dispatch guard
/// ([`call_tool`] rejects an unknown/hallucinated tool name before any network).
pub fn is_known_ctx_tool(name: &str) -> bool {
    CTX_TOOLS.iter().any(|t| t.name == name)
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// Recorded calls + canned returns for the test backend.
#[derive(Default)]
struct StubState {
    /// tool name → canned text `call_tool` returns.
    responses: HashMap<String, String>,
    /// `(name, arguments)` passed to `call_tool`, in order — recorded AFTER the
    /// store-sensitivity default, so [`apply_store_defaults`] is assertable.
    calls: Vec<(String, Value)>,
    /// tool names for which `call_tool` returns `Err` (ctx-outage simulation).
    fail: HashSet<String>,
}

enum CtxBackend {
    /// Real ctx over MCP JSON-RPC (`POST {endpoint}` where endpoint = base + `/mcp`).
    Http {
        client: reqwest::Client,
        endpoint: String,
        /// Monotonic JSON-RPC request id.
        id: AtomicU64,
    },
    /// Test backend: canned per-tool responses + recorded calls, no network.
    Stub(Mutex<StubState>),
}

/// Nova DOT's ctx tool client. See module docs.
pub struct CtxClient {
    backend: CtxBackend,
}

impl CtxClient {
    /// Build from env, or `None` when `CTX_NOVA_MEMORY_KEY` is unset/empty (which disables
    /// Nova's tools). The key is sent as the `X-Context-Key` header on every request; the
    /// scope it grants is key-bound and SQL-enforced server-side.
    ///
    /// Env: `CTX_NOVA_MEMORY_KEY` (required — gates), `CTX_BASE_URL`
    /// (default `http://127.0.0.1:8080`), `CTX_TIMEOUT_SECS`.
    pub fn from_env() -> Option<Arc<CtxClient>> {
        let key = env_nonempty("CTX_NOVA_MEMORY_KEY")?;
        let base_url = env_nonempty("CTX_BASE_URL").unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        let timeout_secs = env_parse("CTX_TIMEOUT_SECS").unwrap_or(DEFAULT_TIMEOUT_SECS);

        let mut headers = reqwest::header::HeaderMap::new();
        let mut value = reqwest::header::HeaderValue::from_str(&key).ok()?;
        value.set_sensitive(true);
        headers.insert("X-Context-Key", value);

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .default_headers(headers)
            .build()
            .ok()?;
        let endpoint = format!("{}{}", base_url.trim_end_matches('/'), MCP_PATH);
        Some(Arc::new(CtxClient {
            backend: CtxBackend::Http {
                client,
                endpoint,
                id: AtomicU64::new(1),
            },
        }))
    }

    /// Dispatch one MCP `tools/call` and return the concatenated text content blocks.
    ///
    /// Choke-point for two app-layer concerns (so they can never drift from [`CTX_TOOLS`]):
    /// an unknown/hallucinated `name` is rejected before any network; a `store` without an
    /// explicit `sensitivity` defaults to `internal`. A transport / non-2xx / JSON-RPC
    /// `error` / unknown-name → `Err`; a tool-level `isError` is returned as `Ok(text)` (the
    /// model should READ the error). The caller turns any `Err` into a model-readable string.
    pub async fn call_tool(&self, name: &str, mut arguments: Value) -> CtxResult<String> {
        if !is_known_ctx_tool(name) {
            return Err(format!("unknown ctx tool: {name}").into());
        }
        apply_store_defaults(name, &mut arguments);
        match &self.backend {
            CtxBackend::Http {
                client,
                endpoint,
                id,
            } => {
                let req = JsonRpcRequest {
                    jsonrpc: "2.0",
                    id: id.fetch_add(1, Ordering::Relaxed),
                    method: "tools/call",
                    params: json!({ "name": name, "arguments": arguments }),
                };
                let parsed: JsonRpcResponse = client
                    .post(endpoint)
                    // The Streamable-HTTP spec requires the Accept header to list BOTH
                    // types even when the server replies in JSON: the go-sdk handler ctx
                    // runs rejects a POST lacking `text/event-stream` with HTTP 400 BEFORE
                    // its JSONResponse branch. We still only ever parse the JSON body.
                    .header(
                        reqwest::header::ACCEPT,
                        "application/json, text/event-stream",
                    )
                    .json(&req)
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await?;
                response_to_text(parsed)
            }
            CtxBackend::Stub(state) => {
                let mut st = state.lock().expect("ctx stub mutex poisoned");
                st.calls.push((name.to_string(), arguments.clone()));
                if st.fail.contains(name) {
                    return Err(format!("stub: tool '{name}' configured to fail").into());
                }
                Ok(st.responses.get(name).cloned().unwrap_or_default())
            }
        }
    }

    // -- Test backends (no network) -----------------------------------------

    /// An empty no-network backend — every `call_tool` records and returns `""`.
    pub fn stub() -> Arc<CtxClient> {
        Arc::new(CtxClient {
            backend: CtxBackend::Stub(Mutex::new(StubState::default())),
        })
    }

    /// A no-network backend pre-seeded with `(tool_name, response_text)` pairs that
    /// `call_tool(name, _)` returns verbatim. Tests inject this via `AppState::with_ctx`.
    pub fn stub_with_responses(entries: &[(&str, &str)]) -> Arc<CtxClient> {
        let mut st = StubState::default();
        for (name, resp) in entries {
            st.responses
                .insert((*name).to_string(), (*resp).to_string());
        }
        Arc::new(CtxClient {
            backend: CtxBackend::Stub(Mutex::new(st)),
        })
    }

    /// A no-network backend whose `call_tool` returns `Err` for the named tools (ctx-outage
    /// simulation), proving the reply loop degrades around a failing tool.
    pub fn stub_failing(tools: &[&str]) -> Arc<CtxClient> {
        let mut st = StubState::default();
        for t in tools {
            st.fail.insert((*t).to_string());
        }
        Arc::new(CtxClient {
            backend: CtxBackend::Stub(Mutex::new(st)),
        })
    }

    /// Test accessor: the `(name, arguments)` pairs passed to `call_tool`, in order
    /// (recorded after the store-sensitivity default).
    pub fn recorded_tool_calls(&self) -> Vec<(String, Value)> {
        match &self.backend {
            CtxBackend::Stub(state) => state.lock().expect("ctx stub mutex poisoned").calls.clone(),
            CtxBackend::Http { .. } => Vec::new(),
        }
    }
}

/// Apply the store-sensitivity default (G1): a non-`store` call is untouched; a `store`
/// without an explicit `sensitivity` gets `internal`; an explicit value is preserved (a
/// `public` request is logged, not overridden). Pure + unit-tested.
fn apply_store_defaults(name: &str, arguments: &mut Value) {
    if name != "store" {
        return;
    }
    if let Some(obj) = arguments.as_object_mut() {
        match obj.get("sensitivity") {
            None => {
                obj.insert("sensitivity".into(), Value::String("internal".into()));
            }
            Some(Value::String(s)) if s == "public" => {
                tracing::warn!("nova ctx store requested sensitivity=public");
            }
            _ => {}
        }
    }
}

/// Pure: a parsed JSON-RPC response → the joined text content (or `Err` on a JSON-RPC
/// `error` / a missing `result`). Only `text` (and untyped) content blocks contribute,
/// joined by newlines, order preserved. `isError` is advisory — its text rides through.
fn response_to_text(parsed: JsonRpcResponse) -> CtxResult<String> {
    if let Some(err) = parsed.error {
        return Err(format!("ctx error {}: {}", err.code, err.message).into());
    }
    let result = parsed.result.ok_or("ctx response missing result")?;
    Ok(result
        .content
        .iter()
        .filter(|b| b.kind.is_empty() || b.kind == "text")
        .map(|b| b.text.as_str())
        .collect::<Vec<_>>()
        .join("\n"))
}

// ---------------------------------------------------------------------------
// JSON-RPC wire structs (tolerant: every read field defaults)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    id: u64,
    method: &'static str,
    params: Value,
}

#[derive(Deserialize)]
struct JsonRpcResponse {
    #[serde(default)]
    result: Option<ToolResult>,
    #[serde(default)]
    error: Option<JsonRpcErr>,
}

#[derive(Deserialize)]
struct JsonRpcErr {
    code: i64,
    message: String,
}

#[derive(Deserialize)]
struct ToolResult {
    #[serde(default)]
    content: Vec<ContentBlock>,
}

#[derive(Deserialize)]
struct ContentBlock {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    text: String,
}

// ---------------------------------------------------------------------------
// Env helpers (same idiom as nova_llm)
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

    #[test]
    fn request_serializes_to_tools_call_shape() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0",
            id: 7,
            method: "tools/call",
            params: json!({ "name": "query", "arguments": { "question": "hi" } }),
        };
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["jsonrpc"], "2.0");
        assert_eq!(v["id"], 7);
        assert_eq!(v["method"], "tools/call");
        assert_eq!(v["params"]["name"], "query");
        assert_eq!(v["params"]["arguments"]["question"], "hi");
    }

    fn parse(body: &str) -> CtxResult<String> {
        response_to_text(serde_json::from_str(body).unwrap())
    }

    #[test]
    fn joins_text_blocks_in_order_dropping_non_text() {
        let out = parse(
            r#"{"result":{"content":[{"type":"text","text":"a"},{"type":"image","text":"NOPE"},{"type":"text","text":"b"}]}}"#,
        )
        .unwrap();
        assert_eq!(out, "a\nb");
    }

    #[test]
    fn untyped_block_counts_as_text() {
        let out = parse(r#"{"result":{"content":[{"text":"x"}]}}"#).unwrap();
        assert_eq!(out, "x");
    }

    #[test]
    fn tool_level_is_error_still_returns_the_text() {
        // isError is advisory — the model should READ the error text, so it's Ok(text).
        let out =
            parse(r#"{"result":{"content":[{"type":"text","text":"not found"}],"isError":true}}"#)
                .unwrap();
        assert_eq!(out, "not found");
    }

    #[test]
    fn jsonrpc_error_maps_to_err() {
        assert!(parse(r#"{"error":{"code":-32602,"message":"bad params"}}"#).is_err());
    }

    #[test]
    fn missing_result_and_error_is_err() {
        assert!(parse(r#"{"jsonrpc":"2.0","id":1}"#).is_err());
    }

    #[test]
    fn tool_specs_cover_exactly_the_five_tools() {
        let specs = openai_tool_specs();
        assert_eq!(specs.len(), 5);
        for s in &specs {
            assert_eq!(s["type"], "function");
            let name = s["function"]["name"].as_str().unwrap();
            assert!(is_known_ctx_tool(name), "{name} not in registry");
            assert_eq!(s["function"]["parameters"]["type"], "object");
        }
        let names: Vec<&str> = specs
            .iter()
            .map(|s| s["function"]["name"].as_str().unwrap())
            .collect();
        for want in ["query", "search", "get", "recent", "store"] {
            assert!(names.contains(&want), "missing {want}");
        }
    }

    #[test]
    fn store_spec_requires_category_title_content() {
        let store = openai_tool_specs()
            .into_iter()
            .find(|s| s["function"]["name"] == "store")
            .unwrap();
        let req: Vec<&str> = store["function"]["parameters"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(req, vec!["category", "title", "content"]);
    }

    #[test]
    fn is_known_ctx_tool_is_exact_and_case_sensitive() {
        assert!(is_known_ctx_tool("query"));
        assert!(!is_known_ctx_tool("Query"));
        assert!(!is_known_ctx_tool("delete"));
        assert!(!is_known_ctx_tool(""));
    }

    #[test]
    fn store_defaults_sensitivity_to_internal_unless_set() {
        let mut a = json!({ "category": "nova-memory", "title": "t", "content": "c" });
        apply_store_defaults("store", &mut a);
        assert_eq!(a["sensitivity"], "internal");

        let mut b =
            json!({ "category": "x", "title": "t", "content": "c", "sensitivity": "personal" });
        apply_store_defaults("store", &mut b);
        assert_eq!(b["sensitivity"], "personal", "explicit value preserved");

        let mut q = json!({ "question": "hi" });
        apply_store_defaults("query", &mut q);
        assert!(q.get("sensitivity").is_none(), "non-store untouched");
    }
}
