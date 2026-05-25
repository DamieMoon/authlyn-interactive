//! Nova MCP — a streamable-HTTP MCP server that lets a local AI act as the
//! special user **Nova** inside authlyn.
//!
//! It is a standalone bridge (behind the `nova` cargo feature; never part of the
//! Leptos app graph): it holds Nova's authlyn session and exposes a small,
//! read/send tool surface over MCP. Nova is just an account whose username is
//! "Nova", so her messages show up as "Nova" with no special server support.
//!
//! Build/run:
//!   cargo build --release --bin nova-mcp --features nova
//!   NOVA_PASSWORD=… ./nova-mcp
//!
//! Config (env):
//!   NOVA_AUTHLYN_URL  authlyn API base url   (default http://127.0.0.1:8081)
//!   NOVA_USERNAME     Nova's account name    (default "Nova")
//!   NOVA_PASSWORD     Nova's password        (required; the account is created
//!                                             on first run if it doesn't exist)
//!   NOVA_BIND         MCP HTTP bind addr     (default 127.0.0.1:8082)
//!
//! Trigger model: Nova SEES every message in her channels but is never obligated
//! to reply — the model decides (lurk / start / join). The tool docs say so.

use anyhow::Context as _;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars, tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpService,
    },
    ErrorData as McpError, ServerHandler,
};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// authlyn HTTP client (as the Nova account)
// ---------------------------------------------------------------------------

/// Request bodies we send to authlyn. Responses are passed through as raw JSON
/// (`serde_json::Value`) so the bridge stays decoupled from the app's DTOs.
#[derive(Serialize)]
struct Creds<'a> {
    username: &'a str,
    password: &'a str,
}

#[derive(Serialize)]
struct SendBody<'a> {
    body: &'a str,
}

/// Holds Nova's cookie session (reqwest cookie jar) and re-authenticates on 401.
#[derive(Clone)]
struct Authlyn {
    http: reqwest::Client,
    base: String,
    username: String,
    password: String,
}

impl Authlyn {
    /// Log in; bootstrap-register the account on first run if login fails.
    async fn ensure_session(&self) -> anyhow::Result<()> {
        let creds = Creds {
            username: &self.username,
            password: &self.password,
        };
        let login = self
            .http
            .post(format!("{}/auth/login", self.base))
            .json(&creds)
            .send()
            .await?;
        if login.status().is_success() {
            return Ok(());
        }
        let register = self
            .http
            .post(format!("{}/auth/register", self.base))
            .json(&creds)
            .send()
            .await?;
        if register.status().is_success() {
            return Ok(());
        }
        anyhow::bail!(
            "authlyn auth failed (login {} / register {})",
            login.status(),
            register.status()
        );
    }

    async fn get(&self, path: &str, query: &[(&str, &str)]) -> anyhow::Result<serde_json::Value> {
        let url = format!("{}{}", self.base, path);
        let mut resp = self.http.get(&url).query(query).send().await?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.ensure_session().await?;
            resp = self.http.get(&url).query(query).send().await?;
        }
        let resp = resp.error_for_status()?;
        Ok(resp.json::<serde_json::Value>().await?)
    }

    async fn post<B: Serialize>(&self, path: &str, body: &B) -> anyhow::Result<serde_json::Value> {
        let url = format!("{}{}", self.base, path);
        let send = |b: &B| self.http.post(&url).json(b).send();
        let mut resp = send(body).await?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.ensure_session().await?;
            resp = send(body).await?;
        }
        let resp = resp.error_for_status()?;
        if resp.status() == reqwest::StatusCode::NO_CONTENT {
            Ok(serde_json::Value::Null)
        } else {
            // 201 (send) carries a JSON body; tolerate an empty/odd body.
            Ok(resp
                .json::<serde_json::Value>()
                .await
                .unwrap_or(serde_json::Value::Null))
        }
    }
}

// ---------------------------------------------------------------------------
// MCP tool arguments
// ---------------------------------------------------------------------------

#[derive(Deserialize, schemars::JsonSchema)]
struct GuildArg {
    /// The server (guild) id, e.g. an `id` from `list_guilds`.
    guild_id: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct ReadArg {
    /// The channel id to read.
    channel_id: String,
    /// Optional: return only messages strictly after this RFC3339 `sent_at`
    /// (pair with `after_id`). Use the values from the last message you saw to
    /// fetch only what's new.
    #[serde(default)]
    since: Option<String>,
    /// Optional: the `id` of the last message you saw (disambiguates messages
    /// sharing a `sent_at`).
    #[serde(default)]
    after_id: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct SendArg {
    /// The channel id to post in.
    channel_id: String,
    /// The message text. Supports the app's inline markup: **bold**, *italic*,
    /// [red]colored[/red].
    body: String,
}

// ---------------------------------------------------------------------------
// MCP server
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Nova {
    api: Authlyn,
    tool_router: ToolRouter<Nova>,
}

fn internal(e: anyhow::Error) -> McpError {
    McpError::internal_error(e.to_string(), None)
}

fn json_result(v: &serde_json::Value) -> CallToolResult {
    CallToolResult::success(vec![Content::text(v.to_string())])
}

#[tool_router]
impl Nova {
    fn new(api: Authlyn) -> Self {
        Self {
            api,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Nova's own identity in authlyn (account id + username). You appear to others as 'Nova'."
    )]
    async fn whoami(&self) -> Result<CallToolResult, McpError> {
        let v = self.api.get("/auth/me", &[]).await.map_err(internal)?;
        Ok(json_result(&v))
    }

    #[tool(
        description = "List the servers (guilds) Nova belongs to. Returns {guilds:[{id,name}]}. A human adds Nova to a server by inviting the username 'Nova'."
    )]
    async fn list_guilds(&self) -> Result<CallToolResult, McpError> {
        let v = self.api.get("/guilds", &[]).await.map_err(internal)?;
        Ok(json_result(&v))
    }

    #[tool(
        description = "List the channels in a server. Returns the guild plus its channels [{id,name,kind}]; kind 'text' is a chat channel, 'lorebook' is reference material (don't chat there)."
    )]
    async fn list_channels(
        &self,
        Parameters(a): Parameters<GuildArg>,
    ) -> Result<CallToolResult, McpError> {
        let v = self
            .api
            .get(&format!("/guilds/{}", a.guild_id), &[])
            .await
            .map_err(internal)?;
        Ok(json_result(&v))
    }

    #[tool(
        description = "Read a channel's messages, oldest→newest. To catch up on only what's new since you last looked, pass `since` (sent_at) and `after_id` from the last message you saw. Each message has author_display (the speaker's shown name), persona_name, body, sent_at, id. You SEE every message but are never required to reply — you may lurk, start a topic, or join in. Your call."
    )]
    async fn read_messages(
        &self,
        Parameters(a): Parameters<ReadArg>,
    ) -> Result<CallToolResult, McpError> {
        let mut query: Vec<(&str, &str)> = Vec::new();
        if let (Some(since), Some(after_id)) = (a.since.as_deref(), a.after_id.as_deref()) {
            query.push(("since", since));
            query.push(("after_id", after_id));
        }
        let v = self
            .api
            .get(&format!("/channels/{}/messages", a.channel_id), &query)
            .await
            .map_err(internal)?;
        Ok(json_result(&v))
    }

    #[tool(
        description = "Post a message to a channel as Nova. Use it naturally and sparingly — only when you have something worth saying. Supports inline markup (**bold**, *italic*, [red]…[/red])."
    )]
    async fn send_message(
        &self,
        Parameters(a): Parameters<SendArg>,
    ) -> Result<CallToolResult, McpError> {
        let v = self
            .api
            .post(
                &format!("/channels/{}/messages", a.channel_id),
                &SendBody { body: &a.body },
            )
            .await
            .map_err(internal)?;
        Ok(json_result(&v))
    }

    #[tool(
        description = "List Nova's accepted friends. Returns {friends:[{account_id,username}]} plus pending incoming/outgoing requests."
    )]
    async fn list_friends(&self) -> Result<CallToolResult, McpError> {
        let v = self.api.get("/friends", &[]).await.map_err(internal)?;
        Ok(json_result(&v))
    }
}

const NOVA_INSTRUCTIONS: &str = "You are Nova, a member of this chat community, acting through the authlyn app. \
These tools let you see and participate like any other user; to everyone else you appear as 'Nova'. \
You SEE every message in the channels you are in, but you are NOT required to reply to anything. \
Behave like a real person: lurk and just read when you have nothing to add, jump into a conversation \
when you can contribute, or start a new one when it fits. To stay current without re-reading history, \
remember the last message you saw per channel and call read_messages with its `since` and `after_id` \
to fetch only newer messages. Keep what you post natural and in-context; don't spam.";

#[tool_handler]
impl ServerHandler for Nova {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2025_03_26,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "nova-mcp".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                ..Implementation::from_build_env()
            },
            instructions: Some(NOVA_INSTRUCTIONS.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// Entrypoint
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let base =
        std::env::var("NOVA_AUTHLYN_URL").unwrap_or_else(|_| "http://127.0.0.1:8081".to_string());
    let username = std::env::var("NOVA_USERNAME").unwrap_or_else(|_| "Nova".to_string());
    let password = std::env::var("NOVA_PASSWORD").context("NOVA_PASSWORD must be set")?;
    let bind = std::env::var("NOVA_BIND").unwrap_or_else(|_| "127.0.0.1:8082".to_string());

    let http = reqwest::Client::builder().cookie_store(true).build()?;
    let api = Authlyn {
        http,
        base: base.clone(),
        username,
        password,
    };
    api.ensure_session()
        .await
        .context("initial login/register to authlyn")?;
    tracing::info!("Nova authenticated to authlyn at {base}");

    let factory_api = api.clone();
    let service = StreamableHttpService::new(
        move || Ok(Nova::new(factory_api.clone())),
        LocalSessionManager::default().into(),
        Default::default(),
    );
    let app = axum::Router::new().nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!("Nova MCP listening on http://{bind}/mcp");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
