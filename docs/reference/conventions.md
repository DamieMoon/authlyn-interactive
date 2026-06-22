# Coding & contribution conventions

A pointer-map, not a copy. The canonical statements of every rule below live in
[`CLAUDE.md`](../../CLAUDE.md) (the project operating manual) and the dense
`#`-comments in [`Cargo.toml`](../../Cargo.toml). This page indexes them, gives
the one-line *why*, names the files that demonstrate each rule, and is explicit
about which conventions are **test-pinned**, which are **compile-enforced**, and
which are **convention-only** (honored by review, not by any gate).

> **Pinning reality (read this first).** Most conventions here are *not* covered
> by a behavioral test. [`tests/style_lint.rs`](../../tests/style_lint.rs) — the
> only "convention guard" suite — is a pure CSS/UI static scan (motion doctrine,
> WebKit `backdrop-filter` pairing, deck-bug-class regressions, the 44px touch
> floor). It contains **zero** checks for handler naming, DTO suffixes, doc
> headers, or the graph split. Where a rule is enforced, it is enforced by the
> **compiler** (the disjoint-graph rule, via the `/check` clippy passes) or by
> **review** (everything else). This doc marks each rule accordingly; do not read
> "convention" as "tested".

---

## 1. Commits — Conventional Commits + milestone tag

**Canonical:** [`CLAUDE.md` → Conventions](../../CLAUDE.md) and
[`CLAUDE.md` → Namespace: release waves](../../CLAUDE.md).

Format: `type(scope): subject (M#/P#)`.

| Part | Rule |
| --- | --- |
| `type` | one of `feat`, `fix`, `refactor`, `docs`, `chore`, `a11y` |
| `scope` | the touched area, lowercase (`orbit`, `deploy`, `chrome`, …) |
| `subject` | imperative mood, no trailing period |
| trailing tag | the milestone token in parens — `(M5/P2)` |
| body | explains the **invariant or finding** touched (not a changelog of edits) |
| body | add a `Tests:` line naming the suite(s) exercised |
| trailers | `Co-Authored-By:` (and the session trailers the harness appends) |

**Milestone namespace** (the trailing tag's vocabulary):

| Token | Meaning |
| --- | --- |
| `M#` | release wave (Milestone) — e.g. `M7` |
| `/P#` | phase within a wave |
| `/T#` | task within a phase |
| `#N` | a review-finding id (bare, e.g. `#19`, `#43`) — appears in bodies/comments |

Representative real subjects (from `git log`): `fix(orbit): … (M7/P-fix)`,
`docs(deploy): … (M7/P-fix)`. **Pinning:** none — commit shape is review-enforced
(no commit-lint hook in [`.githooks/pre-commit`](../../.githooks/)).

---

## 2. Handler naming

**Canonical:** [`CLAUDE.md` → Conventions](../../CLAUDE.md). Demonstrated
throughout [`src/server/mod.rs`](../../src/server/mod.rs) (the route table).

| Rule | Detail | Example |
| --- | --- | --- |
| `verb_noun`, lowercase | a handler is named for what it does, not `handle_*` | `create_guild`, `list_messages`, `patch_channel`, `set_member_role` |
| **no `handle_` prefix** | the route table reads as verbs | grep `fn handle_` in `src/` → **0 hits** |
| static routes rank over dynamic | a literal segment wins over a `{capture}` **regardless of declaration order** (axum router semantics) — so static and dynamic siblings never shadow | `/guilds/trash` vs `/guilds/{id}`; `/channels/read-state` vs `/channels/{cid}/…`; `/messages/trash` vs `/messages/{mid}` |

The static-over-dynamic property is *why* `mod.rs` can safely declare e.g.
`/guilds/trash` after `/guilds/{id}` — the comments at
[`src/server/mod.rs`](../../src/server/mod.rs) (lines ~92–94, ~140–144, ~153)
call this out at each site. **Pinning:** convention is code-pinned only (the
absence of `handle_` is grep-verifiable; static-vs-dynamic *routing behavior* is
exercised incidentally by the per-feature integration suites, e.g.
[`tests/guilds.rs`](../../tests/guilds.rs), [`tests/messages.rs`](../../tests/messages.rs),
but no test asserts the **naming** rule).

---

## 3. DTO conventions (`src/protocol.rs`)

**Canonical:** [`CLAUDE.md` → Conventions](../../CLAUDE.md). Full DTO surface +
the wire contract live in [`src/protocol.rs`](../../src/protocol.rs) and are
documented in [`../architecture/03-data-model.md`](../architecture/03-data-model.md)
and [`../reference/rest-api.md`](../reference/rest-api.md). This section is the
**naming/shape** convention only.

### 3.1 Suffix taxonomy

Every wire DTO carries a suffix that signals its role. The wire is serde JSON;
the **same Rust struct is both the encode side (server) and decode side (client)** —
no codegen, no schema mirror. Current in-tree counts (verified against
`src/protocol.rs`):

| Suffix | Role | Count | Example |
| --- | --- | --- | --- |
| `Request` | inbound request body | 36 | `SendMessageRequest`, `CreateGuildRequest` |
| `Response` | outbound response body (often a wrapper) | 23 | `AuthResponse`, `ListGuildsResponse` |
| `Summary` | list-row projection (compact) | 9 | `GuildSummary`, `MemberSummary` |
| `Detail` | single-entity full read | 2 | `GuildDetail`, `PersonaDetail` |
| `Envelope` | the rich read row with mixed live/snapshot fields | 1 | `MessageEnvelope` |
| `Item` | one element of a list payload | 1 | `FeedbackItem` |
| `Entry` | one element of a keyed/ordered collection | 2 | `LorebookEntry`, `TypingDraftEntry` |
| `Cursor` | a read-position record | 1 | `ChannelReadCursor` |

Note `Cursor` is **not** in `CLAUDE.md`'s enumerated suffix list
(`Request/Response/Summary/Detail/Envelope/Item/Entry`) but exists in-tree
(`ChannelReadCursor`); treat the `CLAUDE.md` list as the *primary* set and
`Cursor` as an accepted read-position extension. A handful of read DTOs carry no
suffix where the bare name already reads as data (`Attachment`, `ReplyPreview`,
`GalleryImage`, `CustomEmoji`, `CameoSummary`'s peers) — those are intentional,
not violations.

### 3.2 PATCH shape

A PATCH-shaped DTO (partial update where an absent field means "leave untouched"):

- derives `Default`,
- every field is `Option<>`,
- every field carries `#[serde(default)]`.

The six canonical PATCH DTOs: `PatchAccountRequest`, `PatchGuildRequest`,
`PatchChannelRequest`, `PatchPersonaRequest`, `PatchLorebookEntryRequest` (and
the account/guild/channel/persona/lorebook family). See the doc comment + derive
on [`src/protocol.rs` `PatchAccountRequest`](../../src/protocol.rs) (lines
107–119) as the reference shape.

Exactly **10** structs derive `Default` in `protocol.rs`: the 5 `Patch*` types
above **plus** `RailOrderRequest`, `TypingPingRequest`, `ReadStateResponse`,
`CreateDmRequest`, `InviteGuestRequest` (these last five derive `Default` for
construction/empty-body ergonomics, not because they are PATCH-shaped).

> **Pinning:** the suffix rule and the PATCH all-`Option<>`/`Default` shape are
> **convention + compile-time only** — *no* test asserts either. The closest
> coverage is the post-ship wire-compat pin (next item), which only proves
> `#[serde(default)]` works for one field combo, not that PATCH DTOs are shaped
> correctly. A new `Patch*` DTO that forgets `Option<>` would compile and pass
> all suites. (Map note flags this as an UNVERIFIED convention; a `style_lint`
> guard would close it.)

### 3.3 Post-ship wire-compat (`#[serde(default)]`)

Any field **added to an already-shipped DTO** must carry `#[serde(default)]` so a
version-skewed producer that omits it still deserializes (rolling deploy / older
native or PWA client). ~70 `#[serde(default)]` attrs exist for this reason.

**Pinned by** [`src/protocol.rs`](../../src/protocol.rs) inline test
`message_envelope_deserializes_without_persona_description_or_color` (the
`#[cfg(test)]` module at the bottom of the file, ~lines 1144–1164 — the F-D12-3
pin). That pin covers **one** field combo on `MessageEnvelope`; the convention
itself (every new field) is review-enforced.

### 3.4 SSE forward-compat (`SyncEvent`)

The SSE event enum is id-only and forward-compatible: an unknown event tag from a
newer server decodes to `SyncEvent::Unknown` (`#[serde(other)]`) rather than
erroring; the server never constructs `Unknown`.

**Pinned by** [`tests/sync_events.rs`](../../tests/sync_events.rs)
`sync_event_serializes_with_snake_case_type_tags` and
`targeted_sync_events_pin_their_wire_shape`. The full SSE contract is documented
in [`../architecture/04-realtime-sse.md`](../architecture/04-realtime-sse.md).

---

## 4. Doc conventions

**Canonical:** [`CLAUDE.md` → Conventions](../../CLAUDE.md).

| Surface | Rule | Example |
| --- | --- | --- |
| **module** | every module opens with a `//!` header stating its job | [`src/protocol.rs`](../../src/protocol.rs) lines 1–8; [`src/server/mod.rs`](../../src/server/mod.rs) lines 1–8 |
| **public REST fn** | the doc comment **leads** with `/// VERB /path — intent` | `/// POST /dms — start a 1:1 …` ([`src/server/dms.rs`](../../src/server/dms.rs)) |
| **dependency rationale** | lives as `#`-comments in `Cargo.toml`, **not** in `//!` headers | see [`Cargo.toml`](../../Cargo.toml) `[dependencies]` |

There are 25 `/// VERB /path — …` intent lines across `src/server/` today; the
table in [`../reference/rest-api.md`](../reference/rest-api.md) is generated from
these. The `//!` header is near-universal — the only files without one are crate
roots / entrypoints that are self-evident from their name (`src/lib.rs`,
`src/main.rs`, `src/app.rs`, `src/db.rs`, `src/storage/mod.rs`); a new *feature*
module is expected to carry one.

**Pinning:** none — doc presence is review-enforced (no rustdoc-coverage gate in
`/check` or the pre-commit hook).

---

## 5. The disjoint feature graphs (hard rule — never cross-import)

**Canonical:** [`CLAUDE.md` → Disjoint feature graphs](../../CLAUDE.md);
per-graph crate membership + each dep's purpose in
[`Cargo.toml` `[features]`](../../Cargo.toml) and its `#`-comments. Architectural
detail in [`../architecture/01-overview.md`](../architecture/01-overview.md).

Three graphs, mutually exclusive at the binary level:

| Graph | Feature | Binary / target | Imports | Never |
| --- | --- | --- | --- | --- |
| **ssr** | `ssr` | the axum server (`authlyn-interactive`) | axum, tokio, surrealdb, argon2, image, web-push, … | never compiled to wasm |
| **hydrate** | `hydrate` | the browser WASM bundle | gloo-*, web-sys, js-sys, wasm-bindgen, … | never the server runtime |
| **nova** | `nova` | `src/bin/nova-mcp.rs` (`required-features = ["nova"]`) | rmcp, reqwest (MCP bridge) | imports **zero** ssr/hydrate code |

Only **two** modules are always-on (compiled into all three graphs):

- [`src/protocol.rs`](../../src/protocol.rs) — wire DTOs (§3),
- [`src/markup/`](../../src/markup/) — the markup engine
  ([`../architecture/06-markup-engine.md`](../architecture/06-markup-engine.md)).

Both must compile to `wasm32-unknown-unknown`: **serde-only**, no
axum/surrealdb/tokio. `protocol.rs` proves this with a single `use`
(`serde::{Deserialize, Serialize}`, line 10) and an **ungated** `pub mod
protocol;` in [`src/lib.rs`](../../src/lib.rs) (contrast: the server module is
`#[cfg]`-gated).

**Why the rule bites:** a stray `use crate::server::…` from `protocol.rs` or
`markup/`, or a hydrate import of an ssr-only crate, breaks the WASM build with a
late, confusing link/codegen error rather than a clean compile error.

**Enforcement (this one *is* gated — by the compiler):** the `/check` quality
gate compiles all three graphs and fails on any cross-graph leak:

```
cargo clippy --features ssr     --no-deps -- -D warnings
cargo clippy --features hydrate --target wasm32-unknown-unknown --no-deps -- -D warnings
```

(plus the `nova` build by hand: `cargo build --release --bin nova-mcp --features
nova`). The exact `/check` invocations and the toolchain prereq
(`rustup target add wasm32-unknown-unknown`) are in
[`CLAUDE.md` → Build / run / test / check](../../CLAUDE.md). For any change to the
always-on spine, "compiles under all three graphs" is part of *done*.

---

## 6. Tests

**Canonical:** [`CLAUDE.md` → Conventions](../../CLAUDE.md) and
[`CLAUDE.md` → Build / run / test / check](../../CLAUDE.md). Full testing model:
[`../architecture/09-testing.md`](../architecture/09-testing.md).

| Rule | Detail |
| --- | --- |
| location | integration tests in `tests/*.rs`; shared harness **stays** `tests/common/mod.rs` |
| attribute | `#[tokio::test]` |
| naming | full-sentence `snake_case` (`purge_should_cascade_guild_member_rows`, `message_envelope_deserializes_without_persona_description_or_color`) |
| how they run | drive the axum router via `tower::ServiceExt::oneshot` (no port bind); each worker gets an isolated namespace + media tempdir (`tests/common/mod.rs`) |
| prereqs | `cargo test --features ssr` **AND** a live SurrealDB on `ws://127.0.0.1:8000` |
| gate | **0 failed** |

The full pre-commit gate also runs the static
[`tests/style_lint.rs`](../../tests/style_lint.rs) scan (§ UI fidelity in
`CLAUDE.md`); `/check` itself is the fmt+clippy subset only.

---

## 7. Versioning (CalVer → SemVer, pending at v27)

**Canonical:** [`CLAUDE.md` → Conventions: Versioning](../../CLAUDE.md); scheme
owned by [`README.md`](../../README.md) + [`Cargo.toml`](../../Cargo.toml).

| | Scheme | Value |
| --- | --- | --- |
| current | SemVer (from v27) | `version = "27.0.0"` ([`Cargo.toml`](../../Cargo.toml) line 4) |
| codename | manual two-word | `mendicant-bias` ([`Cargo.toml`](../../Cargo.toml) line 9, `[package.metadata.release]`) |
| retired | CalVer `YYYY.M.D` | last CalVer build was `2026.6.1` / `saffron-tide` |

The CalVer→SemVer flip **shipped at the v27 release** (2026-06-22): the `version`
line + codename + `README.md` were flipped as the release commit, then
`mendicant-bias` merged to `main` (tag `v27.0.0`). SemVer from here on.
**Pinning:** none — versioning is a manual release step.

---

## Source map

**Files**

- [`CLAUDE.md`](../../CLAUDE.md) — the operating manual; **canonical** source for every rule on this page (commits, handler naming, DTO suffixes, doc headers, graph split, versioning).
- [`Cargo.toml`](../../Cargo.toml) — `[features]` graph membership + per-dependency `#`-rationale; the `version`/codename for §7.
- [`src/protocol.rs`](../../src/protocol.rs) — the DTO suffix taxonomy (§3), the PATCH shape, the `#[serde(default)]` wire-compat convention, and the only always-on serde-only module besides `markup/`.
- [`src/server/mod.rs`](../../src/server/mod.rs) — the route table demonstrating `verb_noun` handler naming and static-over-dynamic ordering (§2).
- [`src/server/dms.rs`](../../src/server/dms.rs), [`src/server/cameos.rs`](../../src/server/cameos.rs) — representative `/// VERB /path — intent` doc lines (§4).
- [`src/lib.rs`](../../src/lib.rs) — ungated `pub mod protocol;` proving the always-on graph membership (§5).
- [`.githooks/pre-commit`](../../.githooks/) — the full local gate (fmt + clippy ×2 + `style_lint`).

**Tests that pin claims here** (sparse by design — most conventions are review- or compile-enforced, not tested):

- [`tests/style_lint.rs`](../../tests/style_lint.rs) — the **only** convention-guard suite; CSS/UI static scan **only** (motion doctrine, `backdrop-filter` WebKit pairing, deck-bug regressions, 44px touch floor). Pins **none** of §§1–4, 7. Cited here as the boundary of what is tested.
- [`src/protocol.rs`](../../src/protocol.rs)::`message_envelope_deserializes_without_persona_description_or_color` — inline `#[cfg(test)]`; pins the §3.3 `#[serde(default)]` post-ship rule (one field combo).
- [`tests/sync_events.rs`](../../tests/sync_events.rs)::`sync_event_serializes_with_snake_case_type_tags`, `targeted_sync_events_pin_their_wire_shape` — pin the §3.4 `SyncEvent` forward-compat / id-only wire shape.

**Cross-links:** [`docs/README.md`](../README.md) ·
[`architecture/01-overview.md`](../architecture/01-overview.md) (graphs) ·
[`architecture/03-data-model.md`](../architecture/03-data-model.md) (DTOs) ·
[`architecture/04-realtime-sse.md`](../architecture/04-realtime-sse.md) (`SyncEvent`) ·
[`architecture/09-testing.md`](../architecture/09-testing.md) ·
[`reference/rest-api.md`](./rest-api.md) (the VERB/path ↔ DTO matrix).
