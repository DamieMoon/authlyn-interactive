# 10 — Nova MCP bridge

`nova-mcp` is a standalone binary that lets an external LLM act as the authlyn
user **Nova**. It is the third, fully disjoint feature graph (`nova`): it imports
**zero** ssr/hydrate code, links none of the app's DTOs, and reaches authlyn
**only over HTTP** — as an ordinary logged-in account, via `reqwest` with a
cookie jar — re-exposing a small read/send surface over MCP (`rmcp`,
streamable-HTTP transport).

Entire graph: one file, [`src/bin/nova-mcp.rs`](../../src/bin/nova-mcp.rs) (333
lines). Feature wiring: [`Cargo.toml`](../../Cargo.toml) `[[bin]]` (L16–19) +
`[features] nova` (L218–230).

---

## Two Novas — read this first

The single largest source of confusion in this codebase: there are **two
unrelated "Nova" things**, in two different graphs. They share a brand but
nothing else.

| | **Nova** (this doc) | **Nova DOT** |
|---|---|---|
| Graph | `nova` (`nova-mcp` binary) | `ssr` + `hydrate` (the app) |
| What it is | An external LLM driving a *normal user account* over MCP | A reserved, login-disabled **system bot account** |
| Account handle | `Nova` (`NOVA_USERNAME`, default) | `nova-dot` (record `account:nova_dot`) |
| Display name | "Nova" — renders like any user | "Nova DOT" |
| Can log in? | Yes (it *is* a login) | **No** — login is rejected by design |
| Authors what? | Ordinary `kind='user'` messages | All `kind='system'` broadcast messages |
| Special server support? | **None** — just an account | Seeded by schema; renders the `.nova-orb` brand SVG |
| Pinned by | compile-gated (see [Pinning](#pinning--tests)) | `tests/schema_apply.rs::nova_dot_system_account_is_seeded_and_cannot_log_in`; `tests/system_messages.rs::*` |

**Nova DOT is not part of the `nova` feature** and is not covered here — it is an
ssr/hydrate concern (admin broadcast → fan-out into each guild's first text
channel). See [05-auth-privacy.md](./05-auth-privacy.md) and
[03-data-model.md](./03-data-model.md). Everything below is about `nova-mcp`.

---

## What it is

`nova-mcp` is a bridge, not a feature of the app. The intended topology:

```
external LLM client  ──MCP (streamable-HTTP)──▶  nova-mcp  ──HTTP (reqwest)──▶  authlyn ssr server
(the "stronger brain")        POST /mcp          (holds Nova's        GET/POST /auth/* /guilds
                                                  cookie session)      /channels/* /friends
```

- The **external LLM** connects over MCP and calls Nova's tools. It is the
  decision-maker; `nova-mcp` carries no model.
- `nova-mcp` translates each tool call into an authenticated REST call against a
  *running* authlyn server, using a `reqwest::Client` with `cookie_store(true)`
  so Nova's session cookie persists across calls.
- authlyn sees a normal authenticated request from the `Nova` account. There is
  **no special-casing of Nova on the server** — her messages are `kind='user'`,
  authored by `Nova`, and render as "Nova" like any other member.

A human "adds Nova to a server" by inviting the username `Nova`, exactly as for a
person (tool description, `src/bin/nova-mcp.rs:195`).

### Lurk doctrine

Nova **sees every message** in channels she is in but is **never obligated to
reply**. The behavioral spec is not in Rust control flow — it lives in the tool
*descriptions* and in `NOVA_INSTRUCTIONS` (`src/bin/nova-mcp.rs:264–270`), which
`get_info()` advertises as the MCP server `instructions`. These strings are
effectively prompt code: they tell the external model to lurk / start a topic /
join in at its discretion, to keep posts natural and sparse, and to catch up
incrementally via the keyset cursor (see [`read_messages`](#tool-surface)).
Editing those strings changes Nova's behavior; treat them as code.

---

## Graph isolation (hard invariant)

`nova` is one of the three [disjoint feature graphs](./01-overview.md) and is the
*most* isolated of the three.

- **No app code, in either direction.** `src/bin/nova-mcp.rs` imports only
  `anyhow`, `rmcp`, `serde`, `serde_json`, `reqwest`, `tokio`, `axum::Router`,
  `tracing*` (imports at L23–33). It references **no** `crate::`, `leptos`,
  `surrealdb`, `protocol.rs`, or `markup/`. Conversely, no ssr/hydrate code
  imports the bin. Coupling to authlyn is the wire only.
- **DTO-decoupled by construction.** Responses are passed through as raw
  `serde_json::Value` (the doc comment at `src/bin/nova-mcp.rs:39–40` makes this
  explicit), so a change to `src/protocol.rs` cannot break `nova-mcp` at compile
  time — the only contract is the JSON shape on the wire. The only typed bodies
  *sent* are the local `Creds` and `SendBody` structs (L41–50), not the app's.
- **Never in the default / cargo-leptos build.** `[[bin]] nova-mcp` carries
  `required-features = ["nova"]` (`Cargo.toml:19`). With `nova` off (the
  default), Cargo does not compile the bin at all, and none of `rmcp` / `reqwest`
  / the extra tokio features are pulled. `cargo leptos build` (which builds the
  `authlyn-interactive` bin with `bin-features = ["ssr"]`,
  `Cargo.toml:259,296`) never touches it.
- **Excluded from `/check` and the pre-commit hook.** The quality gate
  (`cargo fmt` + clippy on ssr + clippy on hydrate-wasm32) does **not** build
  `nova` — confirmed in [`.claude/commands/check.md`](../../.claude/commands/check.md)
  ("the nova bridge is a separate manual check") and
  [`.githooks/pre-commit`](../../.githooks/pre-commit) (no nova step). CLAUDE.md
  states the rule directly ("nova is NOT in `/check`", L19). **You must build it
  by hand** when you touch it.

### Build

```
cargo build --release --bin nova-mcp --features nova
```

This exact command is the only allowlisted nova build in
[`.claude/settings.json`](../../.claude/settings.json) (L15) and is the one given
in `CLAUDE.md` (L19) and the bin's `//!` header (L10). There is no
`cargo leptos` path and no CI step for it.

The `nova` feature (`Cargo.toml:218–230`) pulls `rmcp`, `reqwest`, `anyhow`,
`axum`, a full `tokio` (`rt-multi-thread` + `macros` + `net` + `signal`), and
`tracing` / `tracing-subscriber`. Per-dependency rationale is in the
`#`-comments at `Cargo.toml:132–139` (authoritative — do not duplicate here).

---

## Running it

```
cargo build --release --bin nova-mcp --features nova
NOVA_PASSWORD=… ./target/release/nova-mcp
```

Startup sequence (`main`, `src/bin/nova-mcp.rs:292–333`):

1. Init `tracing_subscriber` with `EnvFilter` from env, default `info`.
2. Read the `NOVA_*` env (table below). **`NOVA_PASSWORD` is required** — absent,
   the process aborts immediately with `"NOVA_PASSWORD must be set"`
   (`src/bin/nova-mcp.rs:303`). The other three have defaults.
3. Build the `reqwest` client with `cookie_store(true)` (L306).
4. **`ensure_session()` once, before serving** (L313–315) — Nova must be able to
   authenticate at boot or the process exits (`context("initial login/register
   to authlyn")`).
5. Mount `StreamableHttpService` (with a `LocalSessionManager`) at `/mcp` on an
   `axum::Router`, bind `NOVA_BIND`, and `axum::serve` with a `ctrl_c` graceful
   shutdown (L318–331).

### Configuration

| Env var | Required | Default | Meaning | Source |
|---|---|---|---|---|
| `NOVA_PASSWORD` | **yes** | — (abort) | Nova's account password | `src/bin/nova-mcp.rs:303` |
| `NOVA_AUTHLYN_URL` | no | `http://127.0.0.1:8081` | authlyn API base URL | `src/bin/nova-mcp.rs:300–301` |
| `NOVA_USERNAME` | no | `Nova` | Nova's account handle | `src/bin/nova-mcp.rs:302` |
| `NOVA_BIND` | no | `127.0.0.1:8082` | MCP HTTP bind address | `src/bin/nova-mcp.rs:304` |
| `RUST_LOG` | no | `info` | `tracing` `EnvFilter` | `src/bin/nova-mcp.rs:295–297` |

These vars are documented **only** in the bin's `//!` header and here.
[`.env.example`](../../.env.example) contains **no** `NOVA_*` entry — it is the
*server's* config template (`SURREAL_*` / `VAPID_*`), and `nova-mcp` does not read
`.env`. See [11-build-deploy-pwa.md](./11-build-deploy-pwa.md) for the full
environment-config surface.

### First-run account bootstrap

`ensure_session()` (`src/bin/nova-mcp.rs:63–91`) is a **login-then-register**
bootstrap:

1. `POST /auth/login` with `{username, password}`. On 2xx → done.
2. On any non-success → `POST /auth/register` with the same creds. On 2xx → done
   (the Nova account now exists, created on first run).
3. If register also fails → `bail!` with both status codes.

So the first time `nova-mcp` runs against a fresh authlyn, it **creates the Nova
account** with the supplied password. This is convenient but security-adjacent:
the account's password is whatever `NOVA_PASSWORD` is set to, and a misconfigured
URL pointing at a *different* authlyn would register Nova there. Keep
`NOVA_AUTHLYN_URL` pointed at the intended instance.

### Loopback / no-TLS assumption

`reqwest` is built `default-features = false` with only `json` / `cookies` /
`multipart` — **no TLS** (`Cargo.toml:138`, `#`-comment L135–136). The default
base is plain `http://127.0.0.1:8081`. `nova-mcp` therefore assumes a **localhost
HTTP** authlyn, *not* a public HTTPS endpoint. Pointing `NOVA_AUTHLYN_URL` at an
`https://` URL will fail to connect (no TLS backend compiled in). If a remote
target is ever needed, add a TLS feature to `reqwest` — do not assume one is
present.

### Port-collision caveat (8082)

`NOVA_BIND` defaults to `127.0.0.1:8082`. On **novahome**, the **test-deck app
service `authlyn-test` also listens on `:8082`** (see
[11-build-deploy-pwa.md](./11-build-deploy-pwa.md) and
[`.claude/commands/test-deploy.md`](../../.claude/commands/test-deploy.md), L20).
Running `nova-mcp` with the default bind on that host collides with the test deck.
Set `NOVA_BIND` explicitly when co-locating; the default is only safe on a host
where nothing else holds 8082. (There is no in-repo systemd unit for `nova-mcp` —
production operation of the bridge is not yet documented anywhere; it is only
referenced aspirationally in the design spec.)

---

## Tool surface

Six MCP tools, generated by the `rmcp` `#[tool_router]` / `#[tool]` macros
(`src/bin/nova-mcp.rs:177–262`). Each maps one-to-one onto an authlyn REST call;
the response is returned verbatim as a JSON-string `Content::text` via
`json_result` (`src/bin/nova-mcp.rs:173–175`). Argument structs derive
`schemars::JsonSchema`, which `rmcp` turns into each tool's input schema.

| Tool | Args | authlyn call | Route (server) | Returns |
|---|---|---|---|---|
| `whoami` | — | `GET /auth/me` | `src/server/mod.rs:74` (`auth::me`) | Nova's account id + username |
| `list_guilds` | — | `GET /guilds` | `src/server/mod.rs:90` (`guilds::list_guilds`) | `{guilds:[{id,name}]}` |
| `list_channels` | `guild_id` | `GET /guilds/{guild_id}` | `src/server/mod.rs:99` (`guilds::get_guild`) | guild + channels `[{id,name,kind}]` |
| `read_messages` | `channel_id`, `since?`, `after_id?` | `GET /channels/{channel_id}/messages` | `src/server/mod.rs:151` (`messages::list_messages`) | messages oldest→newest |
| `send_message` | `channel_id`, `body` | `POST /channels/{channel_id}/messages` | `src/server/mod.rs:151` (`messages::post_message`) | the created message |
| `list_friends` | — | `GET /friends` | `src/server/mod.rs:215` (`friends::list_friends`) | accepted friends + pending in/out |

Every consumed route is verified present in
[`src/server/mod.rs`](../../src/server/mod.rs) at the line cited. The canonical
request/response shapes for these endpoints are the app's REST reference, not
this file — see [../reference/rest-api.md](../reference/rest-api.md). `nova-mcp`
deliberately does **not** model them (raw `Value` pass-through), so the tables
above describe only the *fields the tool descriptions name*, which the external
model relies on.

### `read_messages` keyset catch-up

`read_messages` is the one tool with non-trivial semantics. Its `since`
(RFC3339 `sent_at`) and `after_id` (the last message `id`) are a **composite
keyset cursor**: the tool description (`src/bin/nova-mcp.rs:218`) and
`NOVA_INSTRUCTIONS` tell the model to remember the last message it saw per
channel and pass both back to fetch only newer messages, so it stays current
without re-reading history.

Both must be supplied together — `nova-mcp` only forwards them as query params
when **both** are `Some` (`src/bin/nova-mcp.rs:225–228`). On the server side, the
cursor lives in `src/server/messages/reading.rs` (`since`/`after_id` at L32–33;
`Both { since, after_id }` cursor + validation at L252–264, returning
`"since must be RFC3339 datetime"` / `"after_id must not be empty"`). Messages
return **oldest→newest**, and the `after_id` tie-breaker disambiguates rows that
share a `sent_at`.

Two response fields the description (`src/bin/nova-mcp.rs:218`) tells the model
to read: `author_display` (the speaker's *shown* name) and `persona_name`. Per
the message DTO (`src/protocol.rs:341–356`), `author_display` is resolved **live
at read time**, whereas `persona_name` is the persona snapshot frozen at send and
goes `None` once that persona is deleted — pinned by
`tests/messages.rs::account_identity_resolves_live_while_persona_name_stays_frozen`.

### `send_message` markup

`send_message` posts a `{body}` (`SendBody`, `src/bin/nova-mcp.rs:47–50`),
matching the server's `SendMessageRequest { body }` (`src/protocol.rs:264–265`).
The description advertises the app's inline markup — `**bold**`, `*italic*`,
`[red]…[/red]` — which is rendered by the always-on [markup
engine](./06-markup-engine.md); Nova emits the same raw markup any user types.

### Resilience: 401 → re-auth → single retry

Both HTTP helpers re-authenticate transparently on session expiry
(`src/bin/nova-mcp.rs:93–122`):

- `get` / `post`: issue the request; **if `401 Unauthorized`**, call
  `ensure_session()` and **retry exactly once**. Any other non-2xx surfaces via
  `error_for_status()` → `anyhow::Error` → `McpError::internal_error`
  (`internal`, L169–171).
- Empty-body tolerance: a `204 No Content` returns `Value::Null`; a `201`
  (the `send_message` create) is parsed as JSON but falls back to `Null` if the
  body is empty/odd (`.unwrap_or(Value::Null)`, L113–121).

This 401-retry + empty-body handling is the bridge's entire resilience model — a
long-lived `nova-mcp` survives authlyn session-cookie expiry without operator
intervention, re-logging-in (or, if the account vanished, re-registering) on the
next call.

---

## Transport & MCP identity

- **Transport:** `rmcp` `StreamableHttpService` mounted via
  `nest_service("/mcp")` (`src/bin/nova-mcp.rs:319–324`), with a
  `LocalSessionManager`. The service factory clones the `Authlyn` client into a
  fresh `Nova` per MCP session (`move || Ok(Nova::new(factory_api.clone()))`,
  L320) — all sessions therefore share **one** authlyn cookie jar (one Nova
  identity), which is correct: there is a single Nova account.
- **Server info** (`get_info`, `src/bin/nova-mcp.rs:274–285`):
  - `protocol_version = ProtocolVersion::V_2025_03_26` (pinned to the
    `2025-03-26` MCP revision that `rmcp` 0.8 speaks).
  - `capabilities`: tools only (`.enable_tools()`).
  - `server_info`: `name = "nova-mcp"`, `version = env!("CARGO_PKG_VERSION")`
    (the crate's SemVer, currently `27.0.1`), rest from
    `Implementation::from_build_env()`.
  - `instructions = NOVA_INSTRUCTIONS` (the lurk-doctrine prompt).

---

## Relationship to deploy / ops

`nova-mcp` is **not** part of the authlyn deploy pipeline. The GitHub-Actions CD
([`.github/workflows/deploy.yml`](../../.github/workflows/deploy.yml)) and the
runbooks build and ship the **app** (`authlyn-interactive` bin via
`cargo leptos build`), never the bridge.

Host/port facts changed under the v27 cut: prod is no longer the retired Pi
*fenrir* but **novahome** (`name: Deploy to novahome`, `runs-on: [self-hosted,
novahome]`, `/opt/authlyn-prod/deploy.sh`, app `:8083` — `deploy.yml:1,28,55`),
and the old prod freeze is lifted — a push to `main` auto-deploys to prod, each
promotion owner-gated. Treat the workflow file + the
[deploy](../../.claude/commands/deploy.md) /
[test-deploy](../../.claude/commands/test-deploy.md) runbooks as canonical for
host/port facts. Full deploy story: [11-build-deploy-pwa.md](./11-build-deploy-pwa.md).

---

## Pinning & tests

`nova-mcp` has **no integration test** — there is no `tests/nova*.rs`, and no
`tests/*.rs` exercises the bridge (grep of `tests/` finds only `nova_dot`,
i.e. the *other* Nova). Its guarantees are **compile-gated**, not runtime-pinned:

| Claim | How it is pinned |
|---|---|
| `nova` is a disjoint graph (no `crate::`/leptos/surrealdb) | `src/bin/nova-mcp.rs:23–33` (imports); `cargo build --features nova` compiles, the bin holds no `crate::` refs |
| nova never enters the default / cargo-leptos build | `Cargo.toml:16–19` (`required-features = ["nova"]`), `:259,296` (bin = `authlyn-interactive`, ssr) |
| nova excluded from `/check` + pre-commit | `.claude/commands/check.md`; `.githooks/pre-commit` (no nova step); `CLAUDE.md:19` |
| Consumed REST routes exist | `src/server/mod.rs:74,90,99,151,215` |
| `read_messages` keyset (`since`/`after_id`) is real server behavior | `src/server/messages/reading.rs:32–33,252–264`; `tests/messages.rs::cursor_paginates_past_100_in_order`, `::cursor_tie_break_with_equal_sent_at_is_strict_in_both_directions`, `::malformed_cursor_is_400_not_500` |
| `author_display` live / `persona_name` frozen | `tests/messages.rs::account_identity_resolves_live_while_persona_name_stays_frozen` (`src/protocol.rs:341–356`) |
| `send_message` body shape | `src/protocol.rs:264–265` (`SendMessageRequest { body }`) |
| auth bootstrap targets real endpoints | `tests/auth.rs::register_sets_cookie_and_me_resolves_it`, `::login_good_and_bad_credentials` |
| **Nova ≠ Nova DOT** (system bot) | `tests/schema_apply.rs::nova_dot_system_account_is_seeded_and_cannot_log_in`; `tests/system_messages.rs::broadcast_posts_a_nova_dot_system_message_into_each_guilds_first_text_channel` |

When changing `nova-mcp`, "compiles under `--features nova`" is the only
automated signal — there is no behavioral test to catch a broken tool. Verify the
tool surface manually against a running authlyn.

---

## Source map

**Key files**

- [`src/bin/nova-mcp.rs`](../../src/bin/nova-mcp.rs) — the entire `nova` graph:
  `Authlyn` reqwest client (cookie session + 401-retry + bootstrap-register), the
  6 MCP tools, `ServerHandler`/`get_info`, streamable-HTTP `main`.
- [`Cargo.toml`](../../Cargo.toml) — `[[bin]] nova-mcp` `required-features` (L16–19);
  `[features] nova` (L218–230); `rmcp`/`reqwest`(no-TLS)/`anyhow` deps +
  `#`-comments (L132–139). Authoritative for deps/graph constraints.
- [`.claude/settings.json`](../../.claude/settings.json) — the allowlisted nova
  build command (L15).
- [`.env.example`](../../.env.example) — server config template; contains **no**
  `NOVA_*` (nova config is the env table above).
- [`src/server/mod.rs`](../../src/server/mod.rs) — the router; the REST routes
  nova consumes (L74, 90, 99, 151, 215).
- [`src/server/messages/reading.rs`](../../src/server/messages/reading.rs) —
  server side of `read_messages` (the `since`/`after_id` keyset cursor).
- [`src/protocol.rs`](../../src/protocol.rs) — wire DTOs nova decouples from
  (`SendMessageRequest`, the message envelope fields).

**Tests that pin the claims**

- `tests/messages.rs` — `cursor_paginates_past_100_in_order`,
  `cursor_tie_break_with_equal_sent_at_is_strict_in_both_directions`,
  `malformed_cursor_is_400_not_500`,
  `account_identity_resolves_live_while_persona_name_stays_frozen` (the
  `read_messages` cursor + field semantics).
- `tests/auth.rs` — `register_sets_cookie_and_me_resolves_it`,
  `login_good_and_bad_credentials` (the bootstrap endpoints).
- `tests/schema_apply.rs::nova_dot_system_account_is_seeded_and_cannot_log_in`,
  `tests/system_messages.rs::*` — pin **Nova DOT** (the *other* Nova); cited only
  to keep the two disambiguated.
- `nova-mcp` itself: **(unpinned at runtime)** — compile-gated only; no
  `tests/nova*.rs` exists.

**Related docs**

- [01-overview.md](./01-overview.md) — the three disjoint feature graphs.
- [05-auth-privacy.md](./05-auth-privacy.md) — sessions, login/register, Nova DOT.
- [06-markup-engine.md](./06-markup-engine.md) — the inline markup `send_message` emits.
- [11-build-deploy-pwa.md](./11-build-deploy-pwa.md) — full env-config surface, deploy host/port map, the 8082 collision.
- [../reference/rest-api.md](../reference/rest-api.md) — canonical request/response shapes for the consumed routes.
