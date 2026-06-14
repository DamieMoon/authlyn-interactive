# Skeleton A (Omloppsbana) Implementation Plan

> For agentic workers: REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task; steps use checkbox syntax for tracking.

**Goal:** Ship `sk-orbit` (Omloppsbana) — the first of the three W5 structural skeletons — as a sibling shell branch under the already-wired `.app.sk-orbit` root class: full-viewport channel panes in a horizontal swipe strip, a holographic channel pill opening a zoomable orbit-map picker (pill-tap entry ONLY; pinch entry judge-killed), a floating composer orb with a length-charged send ring + effect blossom, a right-edge HoloPanel slide-over (personas + station), the radial long-press menu placed on message rows, and the directional `--warp-dir` sign + per-server accent finally rendered in real color. Task 0 adds the `guild.accent_color` backend enabler that the warp-jump (#A) and per-server-accent (#G) effects need.

**Architecture:** A new `SkOrbitShell` component (`src/ui/shell/sk_orbit/mod.rs`) renders as a peer branch of the retained W3 chrome, selected by `s.prefs.skeleton`, INSIDE the single `AppShell` `.app` `<div>`. It reuses every existing pane (`ChannelPane`, `FriendsPane`, …) and shared sub-component (`RailGuilds`, `ChannelList`) verbatim via `use_context::<Shell>()` — zero new state, zero new props, no shell remount on switch. All gesture DECISIONS (axis-lock, strip offset, rubber-band, charge fraction, orbit-map node layout, peek-settle) are extracted into pure free functions unit-tested with plain `#[test]`, mirroring `holopanel.rs`. The orbit chrome is built on the Foundation HoloPanel engine (`Edge::Right` slide-over), the `glass-etched`/`glass-live` mixins, the transform-free `.content`/`.channel-view` warp wrapper, `content-visibility` rows, and the visual-haptics helper.

**Tech Stack:** Leptos 0.8 (hydrate WASM + ssr stub split, handlers bound ungated), SurrealDB 3.x (`option<string>` schema field, no backfill), axum 0.8 (`PATCH /guilds/{id}` reused, `require_manager`-gated), SCSS partials gated by `tests/style_lint.rs` (transform/opacity keyframes only), integration tests `cargo test --features ssr` against a live dev SurrealDB on 127.0.0.1:8000.

---

## File Structure

| File | Create / Modify | Single responsibility |
|------|-----------------|------------------------|
| `src/storage/schema.surql` | Modify (after line 113) | Add `guild.accent_color` as `option<string>` (NONE-coercion-safe, no backfill, no ASSERT). |
| `src/protocol.rs` | Modify (lines 145-149, 176-182, 193-196) | Add `accent_color` to `GuildSummary` + `GuildDetail`; add `accent_color: Option<String>` to `PatchGuildRequest`. |
| `src/server/guilds/mod.rs` | Modify (71-120, 343-386, 392-427, 436-473, 262) | Read `accent_color` in `load_my_guilds`/`load_guild_detail`; write it (palette-validated) in `patch_guild`; 4 DTO literals. |
| `src/server/accent.rs` | Create | `pub const ACCENT_PALETTE: [&str; 8]` + `pub fn normalize_accent(raw: &str) -> Option<String>` — the server-side palette validator, unit-tested. |
| `src/client/api.rs` | Modify (224-233) | Fix the EXISTING `patch_guild` literal to add `..Default::default()` (it breaks the moment `PatchGuildRequest` gains a field), then add `set_guild_accent(gid, accent)` sending `PatchGuildRequest { accent_color: Some(..), ..Default::default() }`. |
| `src/ui/shell/act/guild.rs` | Modify (after 186; ssr stub after 257) | Add `set_guild_accent(s, gid, accent)` action (hydrate-real + ssr stub). |
| `src/ui/shell/act/mod.rs` | Modify (77-80) | Re-export `set_guild_accent`. |
| `src/ui/accent.rs` | Create | `pub fn accent_glow_css(name: &str) -> String` + `pub fn accent_var_css(name: &str) -> String` — pure palette→CSS-token mappers, unit-tested. |
| `src/ui/mod.rs` | Modify | `pub mod accent;`. |
| `src/ui/shell/mod.rs` | Modify (37, 426-432, 436-831, 519-540) | Add `mod sk_orbit;` (line 37, before `mod state;`); bind `style:--glow-accent`/`style:--accent` on `.app`; wrap the W3 chrome in the `s.prefs.skeleton` sibling switch (inline branch, not a component — see Task 2.1); add the accent picker button to the owner cluster. |
| `src/ui/shell/holopanel.rs` | Modify (component signature + `PanelDrag` + body) | Foundation-engine extension: add `open: bool` (mount-time animate to the open detent) + `on_close: Option<Callback<()>>` (fired on Esc AND snap-to-closed) props — orbit is its first consumer and needs button-summon + callback-close (Task 7.0). Both optional; legacy drag-summon unchanged. |
| `src/ui/shell/sk_orbit/mod.rs` | Create | `SkOrbitShell` + the orbit chrome (pill, orbit map, composer orb, slide-over, pane mount); imports the pure-fn submodules. |
| `src/ui/shell/sk_orbit/strip.rs` | Create | Pure swipe-strip math: `axis_lock`, `strip_offset`, `row_swipe_wins`, `reply_armed`, `commit_swipe`, `StripCommit`, `commit_target` (picker→switch index mapping) — all `#[test]`ed. (No `peek_settles`: peek-never-marks-read holds structurally — name-only neighbors never become current — so a settle timer would be dead code; deferred to the lazy-neighbor follow-up.) |
| `src/ui/shell/sk_orbit/orbit_map.rs` | Create | Pure orbit-map geometry: `MapGeom`, `map_geom`, `node_angle`, `node_pos`, `NodePos` — all `#[test]`ed. |
| `src/ui/shell/sk_orbit/charge.rs` | Create | Pure charge-ring math: `charge_fraction` (log curve over word count), `CIRC`, `dash_offset` — all `#[test]`ed. |
| `src/ui/shell/sk_orbit/warp.rs` | Create | Pure `warp_dir(from_idx, to_idx) -> i8` directional sign — `#[test]`ed. |
| `style/_sk_orbit.scss` | Create | All `.app.sk-orbit …` orbit chrome styling (strip, pill, orbit-map, orb + charge ring, slide-over), transform/opacity keyframes only, exactly-once safe-area insets. |
| `style/main.scss` | Modify (after line 25) | `@use "sk_orbit";` after `nav`. |
| `style/_foundation.scss` | Modify (151-153) | Update the "awaits guild.accent_color" comment now that the field exists. |
| `tests/schema_apply.rs` | Modify (after 334) | Prod-shaped guard: `accent_color` `option<string>` applies over a populated `guild` table with no crash-loop, legacy reads NONE, written value persists. |
| `tests/accent.rs` | Create | Server integration test: `PATCH /guilds/{id}` accent is `require_manager`-gated, palette-validated (400 on junk), readable back via `GET /guilds/{id}`. |

---

## Task 0 — Backend enabler: `guild.accent_color`

This field is already anticipated by the codebase (`style/_foundation.scss:151-153`, Open Question #5). It is purely cosmetic and gates nothing.

SPEC-DIVERGENCE — deliberate interim, confirm with owner (the spec specifies a DIFFERENT source than this plan ships): spec §1-G ("derived server-side from the guild icon at upload"), §3 ("derived server-side at upload via the image crate"), and §12 (W6) all say `guild.accent_color` is AUTO-DERIVED from the uploaded guild icon via the `image` crate. This plan instead ships a MANUAL owner palette picker (🎨, 8 named swatches) + `normalize_accent` validator — NO derivation, NO guild-icon dependency. Rationale: Skeleton A needs a per-guild accent NOW (for the warp-jump #A and per-server-accent #G effects), but guild-icon upload + image-crate dominant-color derivation is a W6 feature that does not exist yet, so a manual picker is the bridge. The end EFFECT (a per-server accent tinting the warp/chrome) matches the spec; only the SOURCE of the value differs. The schema field (`option<string>` palette name) + the DTOs are forward-compatible: W6 can switch the WRITE path from picker to icon-derived (mapping the icon's dominant color to the nearest palette name, or widening to a free hex) WITHOUT a schema/DTO change. ACTION: confirm with the owner that a manual picker is acceptable for this wave (it is the pragmatic unblock), and record that W6 may replace the population path. This note exists so the plan does not silently override a stated spec mechanism.

### Task 0.1 — Schema field (NONE-coercion-safe, no ASSERT)

**Files:**
- Modify: `src/storage/schema.surql` (insert after line 113, the last `guild` field, before the `channel` block at line 115)
- Modify: `tests/schema_apply.rs` (add a test after line 334, the end of the `effect` migration test)

Steps:

- [ ] 0.1.1 — Write the failing prod-shaped migration guard in `tests/schema_apply.rs`. Append after line 334 (mirrors `applying_effect_over_populated_messages_keeps_legacy_rows_with_effect_none`, lines 251-334; uses the already-imported `common` + `SurrealValue`):

```rust
/// Task 0: `guild.accent_color` is `option<string>` added to the populated
/// `guild` table. Applying the real schema over a pre-existing guild row must
/// NOT crash-loop (no backfill needed for option<>), the legacy row reads back
/// accent_color = NONE, and a value written after apply persists (proving it's
/// a real defined field, not silently stripped by SCHEMAFULL). Mirrors
/// `applying_effect_over_populated_messages_keeps_legacy_rows_with_effect_none`.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn applying_accent_color_over_populated_guilds_keeps_legacy_rows_with_accent_none() {
    let db = common::raw_db().await;

    // A minimal pre-`accent_color` guild schema: every guild field EXCEPT
    // accent_color, so re-applying the full schema introduces only it.
    db.query(
        "DEFINE TABLE guild SCHEMAFULL;\
         DEFINE FIELD name ON guild TYPE string;\
         DEFINE FIELD owner ON guild TYPE record<account>;\
         DEFINE FIELD icon ON guild TYPE option<record<media_blob>>;\
         DEFINE FIELD created_at ON guild TYPE datetime DEFAULT time::now();\
         DEFINE FIELD deleted_at ON guild TYPE option<datetime>;",
    )
    .await
    .expect("old schema transport")
    .check()
    .expect("old schema apply");

    // A populated legacy guild (record links are not referentially enforced, so
    // the dangling owner id is fine).
    db.query("CREATE guild:legacy SET name = 'G', owner = account:y;")
        .await
        .expect("seed legacy transport")
        .check()
        .expect("seed legacy guild");

    // Apply the REAL schema: adds accent_color (option<string>) over the
    // populated table. Must not crash-loop — no backfill for option<>.
    db.query(authlyn_interactive::storage::SCHEMA)
        .await
        .expect("apply real schema transport")
        .check()
        .expect("apply real schema");

    #[derive(SurrealValue)]
    struct Row {
        name: String,
        accent_color: Option<String>,
    }
    let mut resp = db
        .query("SELECT name, accent_color FROM guild:legacy;")
        .await
        .expect("query")
        .check()
        .expect("check");
    let row: Option<Row> = resp.take(0).expect("take");
    let row = row.expect("legacy guild row survives the migration");
    assert_eq!(row.name, "G", "name must survive the apply untouched");
    assert_eq!(row.accent_color, None, "legacy guilds read back accent_color = NONE");

    // Existence probe: an accent written AFTER the apply must persist.
    let mut resp = db
        .query(
            "UPDATE guild:legacy SET accent_color = 'purple';\
             SELECT VALUE accent_color FROM guild:legacy;",
        )
        .await
        .expect("update transport")
        .check()
        .expect("updating a legacy guild with an accent must be accepted");
    let accents: Vec<Option<String>> = resp.take(1).expect("take accent");
    assert_eq!(
        accents,
        vec![Some("purple".to_string())],
        "accent_color must be a real defined field that persists, not silently stripped"
    );
}
```

- [ ] 0.1.2 — Run it and confirm it FAILS (the real schema has no `accent_color`, so the post-apply `UPDATE … SET accent_color` is stripped by SCHEMAFULL and the existence probe reads `[None]`, or the SELECT decode fails). Start the dev DB first if needed (skill `dev-db`):

```bash
cargo test --features ssr applying_accent_color_over_populated_guilds_keeps_legacy_rows_with_accent_none 2>&1 | tail -20
```

Expected: `assertion failed` on the existence-probe `assert_eq!` (got `[None]`, expected `[Some("purple")]`).

- [ ] 0.1.3 — Add the field to `src/storage/schema.surql`. Insert immediately after line 113 (`DEFINE FIELD IF NOT EXISTS deleted_at ON guild …`):

```surql
-- Per-server accent (Open Question #5 / _foundation.scss): a markup-palette
-- name (red…gray, the SAME 8-name vocabulary as persona.color / --tint-*) or
-- NONE for the default electric-blue accent. option<> ⇒ NONE is valid on every
-- pre-existing guild row, so NO backfill is needed and adding it can't trip the
-- SCHEMAFULL NONE-coercion crash (same reasoning as account.security_question
-- and message.effect). Purely cosmetic; gates nothing. NO format ASSERT: the
-- palette is validated server-side (server/accent.rs), mirroring persona.color
-- (a free TYPE string with no ASSERT). If a format ASSERT is ever added it MUST
-- use DEFINE FIELD OVERWRITE, not IF NOT EXISTS (enum-OVERWRITE invariant).
DEFINE FIELD IF NOT EXISTS accent_color ON guild TYPE option<string>;
```

- [ ] 0.1.4 — Run the guard again and confirm it PASSES:

```bash
cargo test --features ssr applying_accent_color_over_populated_guilds_keeps_legacy_rows_with_accent_none 2>&1 | tail -5
```

Expected: `test applying_accent_color_over_populated_guilds_keeps_legacy_rows_with_accent_none ... ok`.

- [ ] 0.1.5 — Commit:

```bash
git add src/storage/schema.surql tests/schema_apply.rs
git commit -m "$(cat <<'EOF'
feat(schema): add guild.accent_color (option<string>, no backfill) (W5/P2)

Per-server accent named from the 8-color markup palette (the persona.color
vocabulary), or NONE for the default. option<string> ⇒ NONE is valid on every
legacy guild row, so no backfill and no SCHEMAFULL NONE-coercion crash. No
format ASSERT — validated server-side (server/accent.rs); enum-OVERWRITE rule
noted in the schema comment for any future ASSERT. Unblocks warp-jump (#A) and
per-server accent (#G) rendering real colors.

Tests: applying_accent_color_over_populated_guilds_keeps_legacy_rows_with_accent_none
Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 0.2 — Server-side palette validator (`server/accent.rs`)

**Files:**
- Create: `src/server/accent.rs`
- Modify: `src/server/mod.rs` (add `pub mod accent;` near the other `pub mod` lines)

Steps:

- [ ] 0.2.1 — Create `src/server/accent.rs` with the validator AND its failing-first unit tests. The 8 names match `--tint-*` in `style/_tokens.scss:80-87`:

```rust
//! W5/P2 per-server accent palette (Open Question #5). The accent is a name
//! from the same 8-color markup palette as `persona.color` / the `--tint-*`
//! CSS tokens (`style/_tokens.scss`). Validated here server-side (the schema
//! field carries no ASSERT, mirroring persona.color); the empty string clears
//! the accent back to the default. Always-on module — imports zero ssr/hydrate
//! crates so the same names are reachable from any graph if ever needed.

/// The 8 valid accent names — the markup palette / `--tint-*` vocabulary.
pub const ACCENT_PALETTE: [&str; 8] = [
    "red", "orange", "yellow", "green", "blue", "purple", "pink", "gray",
];

/// Normalize a client-sent accent into the stored form, or reject it.
/// - trims + lowercases first;
/// - empty (after trim) ⇒ `Some(String::new())` (clears the accent);
/// - a palette name ⇒ `Some(name)`;
/// - anything else ⇒ `None` (caller returns 400).
pub fn normalize_accent(raw: &str) -> Option<String> {
    let v = raw.trim().to_lowercase();
    if v.is_empty() {
        return Some(String::new());
    }
    if ACCENT_PALETTE.contains(&v.as_str()) {
        Some(v)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_names_normalize_to_themselves() {
        for name in ACCENT_PALETTE {
            assert_eq!(normalize_accent(name).as_deref(), Some(name));
        }
    }

    #[test]
    fn empty_clears_the_accent() {
        assert_eq!(normalize_accent("").as_deref(), Some(""));
        assert_eq!(normalize_accent("   ").as_deref(), Some(""));
    }

    #[test]
    fn case_and_whitespace_are_normalized() {
        assert_eq!(normalize_accent("  PURPLE ").as_deref(), Some("purple"));
        assert_eq!(normalize_accent("Red").as_deref(), Some("red"));
    }

    #[test]
    fn out_of_palette_is_rejected() {
        assert_eq!(normalize_accent("chartreuse"), None);
        assert_eq!(normalize_accent("#ff00ff"), None);
        assert_eq!(normalize_accent("blue; DROP TABLE"), None);
    }
}
```

- [ ] 0.2.2 — Register the module in `src/server/mod.rs` (add alongside the existing `pub mod` declarations):

```rust
pub mod accent;
```

- [ ] 0.2.3 — Run the unit tests and confirm they pass (these are pure `#[test]`s, no DB):

```bash
cargo test --features ssr accent::tests 2>&1 | tail -8
```

Expected: 4 tests pass (`palette_names_normalize_to_themselves`, `empty_clears_the_accent`, `case_and_whitespace_are_normalized`, `out_of_palette_is_rejected`).

- [ ] 0.2.4 — Commit:

```bash
git add src/server/accent.rs src/server/mod.rs
git commit -m "$(cat <<'EOF'
feat(server): accent palette validator normalize_accent (W5/P2)

The 8-name markup palette (red…gray, = --tint-* / persona.color), trimmed +
lowercased; empty clears, out-of-palette rejected (caller 400s). Schema field
carries no ASSERT, so this is the single server-side gate.

Tests: accent::tests::{palette_names_normalize_to_themselves,empty_clears_the_accent,case_and_whitespace_are_normalized,out_of_palette_is_rejected}
Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 0.3 — Protocol DTO fields

**Files:**
- Modify: `src/protocol.rs` (`GuildSummary` 145-149, `GuildDetail` 176-182, `PatchGuildRequest` 193-196)

Steps:

- [ ] 0.3.1 — Add `accent_color` to `GuildSummary` (after `pub name: String,` at line 148). `#[serde(default)]` for post-ship wire-compat, matching `MemberSummary::avatar_id`; `String` keeps the existing `PartialEq` derive working:

```rust
/// One guild as it appears in a list (the caller's guild rail).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GuildSummary {
    pub id: String,
    pub name: String,
    /// Per-server accent: a markup-palette name (red…gray) tinting this guild's
    /// chrome, or empty for the default. `#[serde(default)]` for post-ship
    /// wire-compat (older/native clients deserialize cleanly).
    #[serde(default)]
    pub accent_color: String,
}
```

- [ ] 0.3.2 — Add the same field to `GuildDetail` (after `pub owner_id: String,` at line 180):

```rust
/// Response from `GET /guilds/{id}` — the guild plus its channel list.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GuildDetail {
    pub id: String,
    pub name: String,
    pub owner_id: String,
    /// Per-server accent (see `GuildSummary::accent_color`).
    #[serde(default)]
    pub accent_color: String,
    pub channels: Vec<ChannelSummary>,
}
```

- [ ] 0.3.3 — Add the optional accent field to `PatchGuildRequest` (after `pub name: Option<String>,` at line 195):

```rust
/// Body of `PATCH /guilds/{id}` — every field optional (partial update).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PatchGuildRequest {
    pub name: Option<String>,
    /// Markup-palette accent name (red…gray) or empty to clear. Validated
    /// server-side against the same palette as persona.color.
    #[serde(default)]
    pub accent_color: Option<String>,
}
```

- [ ] 0.3.4 — Build the protocol crate graph to surface the 4 now-incomplete `GuildSummary`/`GuildDetail` literals (compile error is the "failing test" here — `protocol` is always-on, so the ssr build covers it):

```bash
cargo build --features ssr 2>&1 | grep -E "missing field|guilds/mod.rs" | head -10
```

Expected: `missing field \`accent_color\`` errors at the 4 literals in `src/server/guilds/mod.rs` (lines ~109, ~262, ~372, ~462). These are fixed in Task 0.4.

NOTE — a SECOND break is INVISIBLE to this `--features ssr` build: the EXISTING hydrate-only `patch_guild` literal at `src/client/api.rs:228-230` constructs `PatchGuildRequest { name: Some(..) }` WITHOUT `..Default::default()` (unlike `patch_channel` at api.rs:239-242, which has it), so it ALSO now fails `missing field accent_color` — but `client/api.rs` is `#[cfg(feature = "hydrate")]`, so the ssr build never compiles it and the error first surfaces under the hydrate-wasm32 clippy in Task 0.5.5. Task 0.5.1 fixes it; do NOT skip it, or 0.5.5 fails to compile.

- [ ] 0.3.5 — Commit (the protocol change is the unit; the server literals land in 0.4):

```bash
git add src/protocol.rs
git commit -m "$(cat <<'EOF'
feat(protocol): add accent_color to guild DTOs + PatchGuildRequest (W5/P2)

GuildSummary + GuildDetail gain a #[serde(default)] accent_color: String (wire-
compat with older/native clients); PatchGuildRequest gains accent_color:
Option<String> for the partial update. String keeps GuildSummary's PartialEq.
Server read/write literals land next.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 0.4 — Server read + write (`require_manager`-gated)

**Files:**
- Modify: `src/server/guilds/mod.rs` — `load_my_guilds` (71-120), `load_guild_detail` (343-386), `patch_guild` (392-427), `list_deleted_guilds` (436-473), create-response literal (262)

Steps:

- [ ] 0.4.1 — Write the failing server integration test `tests/accent.rs`. It drives the real `Router` via the existing harness — NO new `common` helpers are needed (the harness exposes exactly what guild suites already use: `common::arena()`, `common::register_account(&router, user, pass) -> cookie`, `common::send(&router, Method, path, Some(&cookie), Some(&body)) -> (StatusCode, Option<String>, Value)`). This mirrors `tests/guilds.rs` verbatim (its `create_guild` helper + the `create_lists_and_details_with_default_channel` / `nonmember_get_guild_is_404` patterns). Write the COMPLETE compiling file:

```rust
//! W5/P2: guild.accent_color over the REST surface — manager-gated write,
//! palette validation (400 on junk), round-trips through GET /guilds/{id}.
//! Mirrors tests/guilds.rs's harness use (common::arena / register_account /
//! send) — no new common helpers.
#![cfg(feature = "ssr")]

mod common;

use axum::http::{Method, StatusCode};
use serde_json::json;

/// Create a guild as `cookie`, returning its id (mirrors tests/guilds.rs).
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

#[tokio::test]
async fn patch_guild_accent_is_manager_gated_and_palette_validated() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "owner-a", "password123").await;
    let gid = create_guild(&a.router, &owner, "Accent Guild").await;

    // 1. A valid palette accent is accepted (204) and reads back on the detail.
    let (status, _, _) = common::send(
        &a.router,
        Method::PATCH,
        &format!("/guilds/{gid}"),
        Some(&owner),
        Some(&json!({ "accent_color": "purple" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "owner setting a valid accent must be 204"
    );
    let (status, _, body) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["accent_color"], "purple",
        "accent must round-trip on GET /guilds/{{id}}"
    );

    // 2. Empty clears it back to the default.
    let (status, _, _) = common::send(
        &a.router,
        Method::PATCH,
        &format!("/guilds/{gid}"),
        Some(&owner),
        Some(&json!({ "accent_color": "" })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, _, body) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(body["accent_color"], "", "empty accent clears back to default");

    // 3. An out-of-palette value is rejected with 400 (server/accent.rs gate).
    let (status, _, _) = common::send(
        &a.router,
        Method::PATCH,
        &format!("/guilds/{gid}"),
        Some(&owner),
        Some(&json!({ "accent_color": "chartreuse" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "out-of-palette accent must be 400"
    );

    // 4. A non-member cannot set it — the privacy-404 (resolve_membership) /
    //    require_manager gate. (A registered intruder is never a member of gid.)
    let intruder = common::register_account(&a.router, "intruder", "password123").await;
    let (status, _, _) = common::send(
        &a.router,
        Method::PATCH,
        &format!("/guilds/{gid}"),
        Some(&intruder),
        Some(&json!({ "accent_color": "red" })),
    )
    .await;
    assert!(
        status == StatusCode::NOT_FOUND || status == StatusCode::FORBIDDEN,
        "a non-manager must not set the accent (privacy-404 or 403), got {status}"
    );
}
```

NOTE: verify against the real `patch_guild` what a successful PATCH returns — the plan's Task 0.4.3 returns `StatusCode::NO_CONTENT.into_response()` (204) and the EXISTING name-rename branch already returns 204, so 204 is correct. If the in-tree `patch_guild` instead returns 200, change the two `NO_CONTENT` asserts to `OK` (read the handler's final `StatusCode` at execution). The intruder gate returns the privacy-404 for a non-member (`server/access.rs resolve_membership`); a member-who-is-not-manager would 403 — both are accepted. This file needs NO changes to `tests/common/mod.rs`.

- [ ] 0.4.2 — Run it and confirm it FAILS on the ASSERTED behavior (NOT a compile error — the file above compiles against the real harness). The server neither reads nor writes accent yet: `PatchGuildRequest.accent_color` is ignored on PATCH, and `GuildDetail.accent_color` (added in Task 0.3.2) serializes as the default `""`:

```bash
cargo test --features ssr --test accent 2>&1 | tail -20
```

Expected: assertion-1 failure — `body["accent_color"]` is `""` (the serde default), expected `"purple"`. (If it instead fails to COMPILE, the harness symbol names drifted — re-check against `tests/guilds.rs`; do not proceed until the failure is the assertion, per TDD.)

- [ ] 0.4.3 — Add the accent write branch to `patch_guild`. The existing `name` branch at lines 412-416 MOVES `gid` via `.bind(("gid", gid))`; change it to `.bind(("gid", gid.clone()))` so the accent branch can re-bind. Insert the accent branch after the `name` block's closing `}` (after line 425, still inside the handler, before the final `StatusCode::NO_CONTENT.into_response()` at line 426). Import the validator at the top of the file (`use crate::server::accent::normalize_accent;`):

```rust
    if let Some(raw) = req.accent_color {
        let Some(accent) = crate::server::accent::normalize_accent(&raw) else {
            return error_response(StatusCode::BAD_REQUEST, "invalid accent color");
        };
        if let Err(e) = state
            .db
            .query("UPDATE type::record('guild', $gid) SET accent_color = $accent;")
            .bind(("gid", gid.clone()))
            .bind(("accent", accent))
            .await
            .and_then(|r| r.check())
        {
            tracing::error!(error = %e, "patch_guild accent update failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
        state.emit(SyncEvent::ListsChanged);
    }
```

(Also change line 415 from `.bind(("gid", gid))` to `.bind(("gid", gid.clone()))`.)

- [ ] 0.4.4 — Read `accent_color` in `load_guild_detail` (343-386): add it to `GuildRow` (after `owner_key: String,` at line 347), select it (line 359 query), and set it in the `GuildDetail` literal (after `owner_id: g.owner_key,` at line 375). SurrealDB returns `option<string>` as `Option<String>` → coalesce with `.unwrap_or_default()`:

```rust
    #[derive(SurrealValue)]
    struct GuildRow {
        name: String,
        owner_key: String,
        accent_color: Option<String>,
    }
```
query first statement →
```rust
            "SELECT name, meta::id(owner) AS owner_key, accent_color FROM type::record('guild', $gid)
                WHERE deleted_at = NONE;
```
literal →
```rust
    Ok(Some(GuildDetail {
        id: gid.to_string(),
        name: g.name,
        owner_id: g.owner_key,
        accent_color: g.accent_color.unwrap_or_default(),
        channels: chans
```

- [ ] 0.4.5 — Read `accent_color` in `load_my_guilds` (71-120): add it to `Row` (after `name: String,` at line 75), select it (line 90 query), and set it in the `GuildSummary` map (after `name: r.name,` at line 111):

```rust
    #[derive(SurrealValue)]
    struct Row {
        id_key: String,
        name: String,
        accent_color: Option<String>,
    }
```
query first statement →
```rust
            "SELECT meta::id(guild) AS id_key, guild.name AS name, guild.accent_color AS accent_color FROM guild_member
                WHERE account = type::record('account', $account)
                  AND guild.deleted_at = NONE;
```
map →
```rust
        .map(|r| GuildSummary {
            id: r.id_key,
            name: r.name,
            accent_color: r.accent_color.unwrap_or_default(),
        })
```

- [ ] 0.4.6 — Fix the two remaining `GuildSummary` literals so they compile. Create-response at line 262 (a fresh guild has no accent):

```rust
            (StatusCode::CREATED, Json(GuildSummary { id, name, accent_color: String::new() })).into_response()
```

`list_deleted_guilds` map at lines 462-465 (trashed guilds don't need a tinted rail entry):

```rust
            .map(|r| GuildSummary {
                id: r.id_key,
                name: r.name,
                accent_color: String::new(),
            })
```

- [ ] 0.4.7 — Run the integration test and confirm it PASSES:

```bash
cargo test --features ssr --test accent 2>&1 | tail -6
```

Expected: `test patch_guild_accent_is_manager_gated_and_palette_validated ... ok`.

- [ ] 0.4.8 — Commit:

```bash
git add src/server/guilds/mod.rs tests/accent.rs
git commit -m "$(cat <<'EOF'
feat(server): read/write guild.accent_color via PATCH /guilds/{id} (W5/P2)

patch_guild gains an accent branch behind the existing require_manager gate:
normalize_accent validates the palette (400 on junk), UPDATE persists, emits
ListsChanged. load_my_guilds + load_guild_detail select accent_color and
coalesce option<string>→String; create/trash literals default to empty.

Tests: patch_guild_accent_is_manager_gated_and_palette_validated
Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 0.5 — Client API + act layer

**Files:**
- Modify: `src/client/api.rs` (FIX the existing `patch_guild` literal at 228-230 to add `..Default::default()`; then add `set_guild_accent` after `patch_guild` at 224-233)
- Modify: `src/ui/shell/act/guild.rs` (hydrate fn after `rename_server` at 186; ssr stub after line 257)
- Modify: `src/ui/shell/act/mod.rs` (re-export list 77-80)

Steps:

- [ ] 0.5.1 — FIRST fix the EXISTING `patch_guild` literal at `src/client/api.rs:228-230`. It omits `..Default::default()`, so adding `accent_color` to `PatchGuildRequest` (Task 0.3.3) makes it fail `missing field accent_color`. `PatchGuildRequest` already derives `Default` (Task 0.3.3 keeps the `#[derive(... Default ...)]`), so add the spread (matching `patch_channel` at api.rs:239-242). Change:

```rust
        &PatchGuildRequest {
            name: Some(name.to_string()),
        },
```
to:
```rust
        &PatchGuildRequest {
            name: Some(name.to_string()),
            ..Default::default()
        },
```

- [ ] 0.5.2 — Add the client fn to `src/client/api.rs` after `patch_guild` (line 233). It sends only the accent field via `..Default::default()`:

```rust
/// PATCH /guilds/{gid} — set the per-server accent (owner/admin only). An
/// empty string clears it back to the default. Sends ONLY accent_color.
pub async fn set_guild_accent(gid: &str, accent: &str) -> Result<(), ApiError> {
    patch_json(
        &format!("/guilds/{gid}"),
        &PatchGuildRequest {
            accent_color: Some(accent.to_string()),
            ..Default::default()
        },
    )
    .await
}
```

- [ ] 0.5.3 — Add the hydrate-real action to `src/ui/shell/act/guild.rs` after `rename_server` (line 186). It patches the rail list in place on success (mirrors `rename_server`):

```rust
/// Set the open guild's per-server accent (owner/admin). On success, patch the
/// rail entry in place so the accent var rebinds without a refetch.
#[cfg(feature = "hydrate")]
pub fn set_guild_accent(s: Shell, gid: String, accent: String) {
    spawn_local(async move {
        match api::set_guild_accent(&gid, &accent).await {
            Ok(()) => s.sel.guilds.update(|gs| {
                if let Some(g) = gs.iter_mut().find(|g| g.id == gid) {
                    g.accent_color = accent.clone();
                }
            }),
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}
```

- [ ] 0.5.4 — Add the ssr stub to `src/ui/shell/act/guild.rs` (after the `rename_server` stub at line 257, in the `---- ssr stubs ----` block):

```rust
#[cfg(not(feature = "hydrate"))]
pub fn set_guild_accent(_s: Shell, _gid: String, _accent: String) {}
```

- [ ] 0.5.5 — Re-export it in `src/ui/shell/act/mod.rs` — add `set_guild_accent` to the `pub use guild::{…}` list (lines 77-80):

```rust
pub use guild::{
    create_server, load_deleted_guilds, move_guild_to_bounds, open_server, refresh_guilds,
    rename_server, restore_deleted_guild, select_server_for_sheet, set_guild_accent, swap_guild,
};
```

- [ ] 0.5.6 — Verify both graphs compile (this is the "test" — the action is bound ungated, so the ssr stub must typecheck too; the hydrate-wasm32 run is the one that proves the 0.5.1 `client/api.rs` fix, since the ssr build never compiles that file):

```bash
cargo clippy --features ssr -- -D warnings 2>&1 | tail -5 && cargo clippy --features hydrate --target wasm32-unknown-unknown -- -D warnings 2>&1 | tail -5
```

Expected: both finish with no warnings/errors.

- [ ] 0.5.7 — Commit:

```bash
git add src/client/api.rs src/ui/shell/act/guild.rs src/ui/shell/act/mod.rs
git commit -m "$(cat <<'EOF'
feat(ui): set_guild_accent client fn + act action (W5/P2)

Also fix the existing patch_guild literal (client/api.rs) to spread
..Default::default() — adding accent_color to PatchGuildRequest broke it
(missing field), invisibly to the ssr build (client/api.rs is hydrate-only).
api::set_guild_accent PATCHes only accent_color; act::set_guild_accent patches
the rail list in place on success (hydrate-real + ssr no-op stub, bound
ungated). Re-exported through act/mod.rs.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 0.6 — Accent → CSS-token pure mapper (`ui/accent.rs`)

**Files:**
- Create: `src/ui/accent.rs`
- Modify: `src/ui/mod.rs` (`pub mod accent;`)

Steps:

- [ ] 0.6.1 — Create `src/ui/accent.rs` with the pure mappers + failing-first tests. They map a palette name to the `--glow-accent` rgba and the `--accent` solid; an empty/unknown name returns `String::new()` so the inline `style:` sets nothing and the `_tokens.scss` defaults win. The rgba values mirror the `--tint-*` hexes (`_tokens.scss:80-87`) at the `--glow-accent` 0.55 alpha:

```rust
//! W5/P2 per-server accent → CSS token mapper (Open Question #5). Maps a
//! markup-palette accent name (red…gray, the `server/accent.rs` vocabulary) to
//! the CSS custom-property values bound on the `.app` root, so the warp-jump
//! streak (#A) and accent family (#G) render the guild's color. An empty or
//! unknown name returns `String::new()` → the inline `style:` binding sets
//! nothing and the `style/_tokens.scss` defaults (--glow-accent / --accent)
//! win. Always-on (used by the shell view); imports zero ssr/hydrate crates.

/// `var(--tint-NAME)` solid for the accent name, or empty for default/unknown.
/// Reuses the existing `--tint-*` tokens so there is one palette source.
pub fn accent_var_css(name: &str) -> String {
    if is_palette(name) {
        format!("var(--tint-{name})")
    } else {
        String::new()
    }
}

/// The `--glow-accent` rgba (alpha 0.55, matching `_tokens.scss:67`) for the
/// accent name, or empty for default/unknown. Hardcoded rgba mirrors the
/// `--tint-*` hexes so the glow tints with the same color the solid uses.
pub fn accent_glow_css(name: &str) -> String {
    let rgb = match name {
        "red" => "255, 138, 150",
        "orange" => "255, 180, 127",
        "yellow" => "255, 212, 127",
        "green" => "142, 230, 200",
        "blue" => "127, 182, 255",
        "purple" => "196, 168, 255",
        "pink" => "255, 154, 213",
        "gray" => "154, 167, 189",
        _ => return String::new(),
    };
    format!("rgba({rgb}, 0.55)")
}

fn is_palette(name: &str) -> bool {
    matches!(
        name,
        "red" | "orange" | "yellow" | "green" | "blue" | "purple" | "pink" | "gray"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_names_map_to_tint_var_and_rgba() {
        assert_eq!(accent_var_css("purple"), "var(--tint-purple)");
        assert_eq!(accent_glow_css("purple"), "rgba(196, 168, 255, 0.55)");
        assert_eq!(accent_var_css("green"), "var(--tint-green)");
        assert_eq!(accent_glow_css("green"), "rgba(142, 230, 200, 0.55)");
    }

    #[test]
    fn empty_and_unknown_return_blank_so_token_default_wins() {
        assert_eq!(accent_var_css(""), "");
        assert_eq!(accent_glow_css(""), "");
        assert_eq!(accent_var_css("chartreuse"), "");
        assert_eq!(accent_glow_css("chartreuse"), "");
    }
}
```

- [ ] 0.6.2 — Register in `src/ui/mod.rs` (add alongside the other `pub mod` lines):

```rust
pub mod accent;
```

- [ ] 0.6.3 — Run the unit tests (pure `#[test]`, no DB) and confirm they pass:

```bash
cargo test --features ssr accent::tests::known_names_map_to_tint_var_and_rgba accent::tests::empty_and_unknown_return_blank_so_token_default_wins 2>&1 | tail -6
```

Expected: 2 tests pass. (Note: this `accent::tests` path is in `crate::ui::accent`; the `server::accent::tests` from 0.2 are distinct — both run, both pass.)

- [ ] 0.6.4 — Commit:

```bash
git add src/ui/accent.rs src/ui/mod.rs
git commit -m "$(cat <<'EOF'
feat(ui): accent name → CSS token mapper (W5/P2)

accent_var_css → var(--tint-NAME); accent_glow_css → rgba(..., 0.55) mirroring
the --tint-* hexes. Empty/unknown → "" so the inline style: sets nothing and
the _tokens.scss --glow-accent / --accent defaults win. Pure, unit-tested.

Tests: ui::accent::tests::{known_names_map_to_tint_var_and_rgba,empty_and_unknown_return_blank_so_token_default_wins}
Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 0.7 — Wire the accent var on `.app` + the settings picker

**Files:**
- Modify: `src/ui/shell/mod.rs` — `.app` root `style:` bindings (426-432), accent picker in the owner cluster (519-540), an `accent_open` signal + a small picker `Modal`
- Modify: `style/_foundation.scss` (151-153 comment)

Steps:

- [ ] 0.7.1 — Add a derived helper near `server_name` (mod.rs:326-335) that reads the open guild's `accent_color`, plus an `accent_open` signal near the other modal signals (mod.rs:298-313). After the `server_name` closure, add:

```rust
    // The open guild's accent name (empty = default), derived from the rail
    // list so it auto-updates on a set-accent patch.
    let accent_name = move || {
        let sid = s.sel.sel_server.get();
        s.sel
            .guilds
            .get()
            .into_iter()
            .find(|g| Some(&g.id) == sid.as_ref())
            .map(|g| g.accent_color)
            .unwrap_or_default()
    };
```
and near the modal signals:
```rust
    // W5/P2: the per-server accent picker modal (owner-gated, opens from the
    // server-header cluster).
    let accent_open = RwSignal::new(false);
```

- [ ] 0.7.2 — ADD ONLY the two `style:` lines below to the EXISTING `.app` `<div>` (mod.rs:426-432). The three `class:sk-orbit`/`class:sk-deck`/`class:sk-hud` bindings are ALREADY present (shipped W5/P1 at mod.rs:429-431) — do NOT re-add them or you create duplicate `class:` bindings. Insert the two `style:` lines (the idiom is proven at `holopanel.rs:335` — `style:--p=…`) immediately after the existing `class:sk-hud=…` line, before the closing `>`. An empty string sets nothing, so the `_tokens.scss` defaults win on a default/unaccented guild. The result is:

```rust
        <div class="app"
            class:dialogue-style=move || s.prefs.dialogue_style.get()
            class:fx-max=move || s.prefs.eyecandy.get()
            class:sk-orbit=move || s.prefs.skeleton.get().as_deref() == Some("orbit")   // EXISTING
            class:sk-deck=move || s.prefs.skeleton.get().as_deref() == Some("deck")     // EXISTING
            class:sk-hud=move || s.prefs.skeleton.get().as_deref() == Some("hud")       // EXISTING
            style:--glow-accent=move || crate::ui::accent::accent_glow_css(&accent_name())  // ADD
            style:--accent=move || crate::ui::accent::accent_var_css(&accent_name())        // ADD
        >
```

- [ ] 0.7.3 — Add the accent picker button to the owner cluster (mod.rs:519-540), alongside the ⚙/✎/🗑 buttons gated by `<Show when=is_owner …>` (the UI uses the owner-only `is_owner` gate; the server `require_manager` is the real gate — admins set it via API). Add after the "Manage channels" ⚙ button (line 523):

```rust
                    <button class="row-edit" title="Server accent"
                        on:click=move |_| accent_open.set(true)>"🎨"</button>
```

- [ ] 0.7.4 — Add the picker `Modal` near the other top-level modals (mod.rs:851-877 region). It renders the 8 palette swatches + a "Default" clear, calling `act::set_guild_accent`; the swatch grid mirrors the persona color-picker pattern (`act/compose_colors.rs` / `wardrobe.rs`). Add after the wardrobe modal block:

```rust
            // W5/P2 per-server accent picker (owner-gated open). The 8 palette
            // swatches + a Default clear; each calls act::set_guild_accent on
            // the open guild. The server require_manager gate is authoritative.
            {move || accent_open.get().then(|| {
                let names = ["red", "orange", "yellow", "green", "blue", "purple", "pink", "gray"];
                let cur = accent_name();
                view! {
                    <Modal class="accent-modal" close=move || accent_open.set(false)>
                        <h3>"Server accent"</h3>
                        <div class="accent-swatches">
                            <button class="accent-swatch accent-default"
                                class:active=move || accent_name().is_empty()
                                title="Default (electric blue)"
                                on:click=move |_| {
                                    if let Some(gid) = s.sel.sel_server.get_untracked() {
                                        act::set_guild_accent(s, gid, String::new());
                                    }
                                    accent_open.set(false);
                                }>"Default"</button>
                            {names.into_iter().map(|n| {
                                let n_owned = n.to_string();
                                let is_cur = cur == n;
                                view! {
                                    <button
                                        class="accent-swatch"
                                        class:active=is_cur
                                        style:background=format!("var(--tint-{n})")
                                        title=n
                                        on:click=move |_| {
                                            if let Some(gid) = s.sel.sel_server.get_untracked() {
                                                act::set_guild_accent(s, gid, n_owned.clone());
                                            }
                                            accent_open.set(false);
                                        }>
                                    </button>
                                }
                            }).collect_view()}
                        </div>
                    </Modal>
                }
            })}
```

- [ ] 0.7.5 — Add minimal swatch styling. Append to `style/_foundation.scss` (or a small block in `_wardrobe.scss` next to the persona swatches — match where the persona color grid lives). Keep it to layout/color only (no keyframes):

```scss
// W5/P2 per-server accent picker swatches (owner-gated modal).
.accent-swatches {
	display: flex;
	flex-wrap: wrap;
	gap: 0.5rem;
	margin-top: 0.75rem;
}
.accent-swatch {
	width: 2.25rem;
	height: 2.25rem;
	border-radius: 50%;
	border: 2px solid transparent;
	cursor: pointer;
}
.accent-swatch.active {
	border-color: var(--text);
}
.accent-swatch.accent-default {
	width: auto;
	border-radius: 0.5rem;
	padding: 0 0.6rem;
}
```

- [ ] 0.7.6 — Update the stale comment in `style/_foundation.scss:151-153` now that the field exists:

```scss
// (#A/#G: the streak tints with --glow-accent, which the shell now rebinds per
// guild from guild.accent_color via an inline style: on .app root — see
// src/ui/accent.rs. A guild with no accent leaves --glow-accent at its token
// default.)
```

- [ ] 0.7.7 — Verify the shell builds on both graphs and the style lint still passes:

```bash
cargo clippy --features ssr -- -D warnings 2>&1 | tail -5 \
 && cargo clippy --features hydrate --target wasm32-unknown-unknown -- -D warnings 2>&1 | tail -5 \
 && cargo test --features ssr --test style_lint 2>&1 | tail -5
```

Expected: both clippy runs clean; `style_lint` tests pass.

- [ ] 0.7.8 — Visual smoke (headed Playwright on `localhost:3000` + dev DB ONLY — never prod). Start the dev server (`cargo leptos watch`), open a guild you own, click 🎨, pick purple, confirm the warp dip on the NEXT channel switch tints purple (the `.fx-max` streak needs eye-candy ON). Then commit:

```bash
git add src/ui/shell/mod.rs style/_foundation.scss
git commit -m "$(cat <<'EOF'
feat(ui): per-server accent picker + bind --glow-accent/--accent on .app (W5/P2)

Owner-gated 🎨 picker (8 palette swatches + Default) calls act::set_guild_accent;
.app root binds style:--glow-accent/--accent from the open guild's accent_color
(empty → token default). Renders warp-jump (#A) + per-server accent (#G) in the
guild's real color. Stale "awaits guild.accent_color" comment updated.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 1 — Pure gesture/transition math modules (no DOM, fully unit-tested)

These are the testable core of the whole skeleton (the project has NO WASM/JS UI test harness — gesture DECISIONS live in pure fns, exactly like `holopanel.rs`). Build them FIRST; the view tasks wire them.

### Task 1.1 — `warp.rs` directional `--warp-dir` sign

**Files:**
- Create: `src/ui/shell/sk_orbit/warp.rs`
- Modify: `src/ui/shell/mod.rs` (add `mod sk_orbit;` alphabetically after `mod members;` / before `mod state;` — currently line 37, alongside the other `mod` lines)
- Create (empty stub for now): `src/ui/shell/sk_orbit/mod.rs` with `pub mod warp;` so the module tree compiles

Steps:

- [ ] 1.1.1 — Create `src/ui/shell/sk_orbit/mod.rs` as a thin module root (the `SkOrbitShell` component lands in Task 5; for now it just declares the pure-fn submodules so they compile and test):

```rust
//! W5/P2 Omloppsbana (`sk-orbit`) — the spatial gesture-first structural
//! skeleton. Full-viewport channel panes in a horizontal swipe strip, a
//! holographic channel pill opening a zoomable orbit-map picker (pill-tap entry
//! ONLY — the pinch entry was judge-killed), a floating composer orb with a
//! length-charged send ring + effect blossom, and a right-edge HoloPanel
//! slide-over. The shell view (`SkOrbitShell`) reuses every existing pane via
//! `use_context::<Shell>()`; the gesture/transition DECISIONS are pure fns in
//! the submodules below (unit-tested, no DOM — the project has no WASM UI test
//! harness). Built on the Foundation substrate: portals (#54), etched glass
//! (#20), HoloPanel (#49), visual haptics (#19), the transform-free
//! .channel-view warp wrapper, and the .app.sk-orbit root class already wired
//! in `shell/mod.rs`.
//!
//! Shared/always-on math modules (no ssr/hydrate crates); the view code that
//! consumes them is feature-gated where it touches `web_sys`.

pub mod charge;
pub mod orbit_map;
pub mod strip;
pub mod warp;
```

- [ ] 1.1.2 — Add `mod sk_orbit;` to `src/ui/shell/mod.rs` alphabetically, after `mod members;` (currently line 36) and before `mod state;` (currently line 37) — match the existing block at lines 29-39 (which is `members`(36), `state`(37), `toast`(38), `wardrobe`(39); `sk_orbit` sorts before `state`):

```rust
mod sk_orbit;
```

- [ ] 1.1.3 — Write `src/ui/shell/sk_orbit/warp.rs` with the failing-first test. The sign drives `--warp-dir` (Foundation deferred it from T0.2): switching to a HIGHER channel-list index slides the incoming pane from the right (+1), a LOWER index from the left (−1), same/unknown is neutral (0):

```rust
//! W5/P2 directional warp sign (deferred from Foundation T0.2). The act layer
//! sets `--warp-dir` (+1 / -1 / 0) from the channel-list index sign of a
//! picker-driven switch; the incoming `.channel-view` slides from
//! `translateX(calc(var(--warp-dir) * 6%))` (`_content.scss:46`). Pure — no DOM.

/// The directional sign for a switch from `from_idx` to `to_idx` in the
/// channel list. Higher destination ⇒ +1 (slide in from the right), lower ⇒
/// -1 (from the left), same index ⇒ 0 (neutral dip). Either index `None`
/// (channel not in the current list — e.g. a cross-guild orbit-map dive) ⇒ 0.
pub fn warp_dir(from_idx: Option<usize>, to_idx: Option<usize>) -> i8 {
    match (from_idx, to_idx) {
        (Some(a), Some(b)) if b > a => 1,
        (Some(a), Some(b)) if b < a => -1,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn higher_destination_slides_from_right() {
        assert_eq!(warp_dir(Some(0), Some(3)), 1);
        assert_eq!(warp_dir(Some(2), Some(3)), 1);
    }

    #[test]
    fn lower_destination_slides_from_left() {
        assert_eq!(warp_dir(Some(3), Some(0)), -1);
        assert_eq!(warp_dir(Some(3), Some(2)), -1);
    }

    #[test]
    fn same_or_unknown_index_is_neutral() {
        assert_eq!(warp_dir(Some(2), Some(2)), 0);
        assert_eq!(warp_dir(None, Some(2)), 0);
        assert_eq!(warp_dir(Some(2), None), 0);
        assert_eq!(warp_dir(None, None), 0);
    }
}
```

- [ ] 1.1.4 — Run and confirm the tests pass (pure fns; the failing state was the module not existing — confirm it now compiles AND passes):

```bash
cargo test --features ssr sk_orbit::warp 2>&1 | tail -8
```

Expected: 3 tests pass.

- [ ] 1.1.5 — Commit:

```bash
git add src/ui/shell/sk_orbit/mod.rs src/ui/shell/sk_orbit/warp.rs src/ui/shell/mod.rs
git commit -m "$(cat <<'EOF'
feat(ui): sk_orbit module + warp_dir directional sign (W5/P2)

Scaffolds src/ui/shell/sk_orbit/ (the Omloppsbana skeleton) and sets the
--warp-dir sign deferred from Foundation T0.2: higher destination index → +1,
lower → -1, same/unknown → 0. Pure, unit-tested; wired into the act switch path
in a later task.

Tests: sk_orbit::warp::tests::{higher_destination_slides_from_right,lower_destination_slides_from_left,same_or_unknown_index_is_neutral}
Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 1.2 — `charge.rs` length-charged send ring math

**Files:**
- Create: `src/ui/shell/sk_orbit/charge.rs`
- (module already declared in `sk_orbit/mod.rs` from 1.1.1)

The current charge ring (`channel/mod.rs:541-547`) uses a LINEAR `chars/280` curve, documented as prose-hostile (#33). Omloppsbana's orb replaces it with a LOG curve over WORD count: `ln(1+words)/ln(1+250)`.

Steps:

- [ ] 1.2.1 — Write `src/ui/shell/sk_orbit/charge.rs` with the failing-first test:

```rust
//! W5/P2 composer-orb charge ring (#E + #33 calibration). The ring fills with
//! message LENGTH; the old linear `chars/280` was prose-hostile (#33), so this
//! uses a log curve over WORD count: a one-liner shows a sliver, a paragraph
//! ~60%, only a saga pegs it. Pure math — the SVG `stroke-dashoffset` and the
//! `--charge` custom property are computed from these. No DOM.

/// The send button's progress-ring circumference (52×52 SVG, r≈24 → C≈151),
/// matching the prototype's `CIRC=151`.
pub const CIRC: f64 = 151.0;

/// The word count saturating the ring (a "saga"). `ln(1+250)` is the curve's
/// denominator so 250 words ≈ full.
const SATURATE_WORDS: f64 = 250.0;

/// Charge fraction 0..=1 from the composed text: `ln(1+words)/ln(1+250)`,
/// clamped. Empty/whitespace ⇒ 0. Words split on ASCII/Unicode whitespace.
pub fn charge_fraction(text: &str) -> f64 {
    let words = text.split_whitespace().count() as f64;
    if words <= 0.0 {
        return 0.0;
    }
    ((1.0 + words).ln() / (1.0 + SATURATE_WORDS).ln()).clamp(0.0, 1.0)
}

/// The SVG `stroke-dashoffset` for a given charge fraction: the ring is empty
/// at offset = CIRC and full at offset = 0 (the arc is dashed the full
/// circumference and revealed as the offset shrinks).
pub fn dash_offset(fraction: f64) -> f64 {
    CIRC * (1.0 - fraction.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_zero_charge() {
        assert_eq!(charge_fraction(""), 0.0);
        assert_eq!(charge_fraction("   \n\t "), 0.0);
    }

    #[test]
    fn one_liner_shows_a_sliver_paragraph_mid_saga_pegs() {
        // Real curve values (ln(1+n)/ln(251)): 1 word ≈ 0.124, 28 words ≈ 0.609
        // ("a paragraph"), 400 words = 1.085 → CLAMPED to 1.0. The saga assertion
        // therefore exercises the clamp, not the raw curve (the curve only
        // reaches 1.0 at exactly 250 words; anything beyond rides the clamp).
        let one = charge_fraction("hi");
        let para = charge_fraction(&"word ".repeat(28));
        let saga = charge_fraction(&"word ".repeat(400));
        assert!(one > 0.0 && one < 0.2, "one-liner is a sliver, got {one}");
        assert!(para > 0.55 && para < 0.65, "a paragraph (~28 words) ≈ 0.61, got {para}");
        assert!((saga - 1.0).abs() < 1e-9, "a saga pegs at the clamp (1.0), got {saga}");
    }

    #[test]
    fn fraction_is_monotonic_in_word_count() {
        let a = charge_fraction("one two");
        let b = charge_fraction("one two three four five");
        assert!(b > a, "more words ⇒ more charge");
    }

    #[test]
    fn dash_offset_maps_empty_to_full_circ_and_full_to_zero() {
        assert!((dash_offset(0.0) - CIRC).abs() < 1e-9);
        assert!(dash_offset(1.0).abs() < 1e-9);
        assert!((dash_offset(0.5) - CIRC * 0.5).abs() < 1e-9);
    }
}
```

- [ ] 1.2.2 — Run and confirm the tests pass:

```bash
cargo test --features ssr sk_orbit::charge 2>&1 | tail -8
```

Expected: 4 tests pass.

- [ ] 1.2.3 — Commit:

```bash
git add src/ui/shell/sk_orbit/charge.rs
git commit -m "$(cat <<'EOF'
feat(ui): composer-orb charge ring log curve over word count (W5/P2 #E/#33)

charge_fraction = ln(1+words)/ln(1+250) (clamped) replaces the prose-hostile
linear chars/280; dash_offset maps 0→CIRC(151)/1→0 for the SVG ring. Pure,
unit-tested (sliver/paragraph/saga + monotonicity).

Tests: sk_orbit::charge::tests::{empty_is_zero_charge,one_liner_shows_a_sliver_paragraph_mid_saga_pegs,fraction_is_monotonic_in_word_count,dash_offset_maps_empty_to_full_circ_and_full_to_zero}
Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 1.3 — `strip.rs` axis-lock + swipe-commit + peek-settle math

**Files:**
- Create: `src/ui/shell/sk_orbit/strip.rs`
- (module already declared in `sk_orbit/mod.rs`)

This is the #1-risk core (axis-lock arbitration vs scroll + swipe-to-reply). All physics constants come from the prototype `a-orbit.html` (verified): axis commit `|dx|>10 && |dx|>|dy|*1.15`; rubber-band `0.32`; commit `≥32% displacement OR velocity >0.45 px/ms`; reply-glyph pop at 64px. Also `commit_target` (the picker→switch index mapping, the testable substitute for the roadmap's "picker channel-switch test").

PEEK-NEVER-MARKS-READ — resolved, NOT a ≥300ms timer for Phase 2: the roadmap names "peek-never-marks-read discipline" as a Phase-2 feasibility tax. READING the mark-read path settles it WITHOUT a settle timer. `act::open_channel` → `open_channel_at` (hydrate, `src/ui/shell/act/channel.rs`) advances the high-water mark by calling `set_last_seen` (channel.rs:305, inside the spawned first-page body) IMMEDIATELY on open. That is CORRECT for a COMMITTED swipe — the destination IS the active channel now, exactly as a sidebar/orbit-map tap marks it read. The "peek" harm (marking an unread channel read just by glimpsing it mid-drag) cannot occur because orbit's neighbor panes are NAME-ONLY (Task 5.2.3): a neighbor is never mounted as a real `ChannelPane`, never subscribes, never calls `open_channel`, so it never becomes "current" and never marks read. Therefore NO ≥300ms `peek_settles` gate is needed or shipped in Phase 2 — a tested-but-unused fn would be dead code. The ≥300ms settle becomes relevant ONLY when a future lazy neighbor render (Phase-7 carry / 9.4.3-b) makes a neighbor briefly "current"; `peek_settles` is therefore DEFERRED to ship WITH that render (Task 9.4.3-c), not built speculatively now.

Steps:

- [ ] 1.3.1 — Write `src/ui/shell/sk_orbit/strip.rs` with the failing-first tests:

```rust
//! W5/P2 horizontal swipe-strip physics (Omloppsbana's signature gesture) +
//! the swipe-to-reply axis arbitration. ALL decisions are pure fns so the
//! WASM-only pointer handlers stay thin and the logic is unit-tested (the
//! project has no WASM UI harness). Constants are the prototype's verified
//! values (`a-orbit.html`). No DOM.

/// Axis-lock commit slop: a gesture is uncommitted until it leaves this radius.
pub const AXIS_SLOP_PX: f64 = 10.0;
/// Horizontal dominance ratio: |dx| must beat |dy| by this factor to lock H.
pub const H_DOMINANCE: f64 = 1.15;
/// First/last-channel rubber-band resistance factor.
pub const RUBBER_BAND: f64 = 0.32;
/// Commit-on-release displacement fraction of the pane width.
pub const COMMIT_FRACTION: f64 = 0.32;
/// Commit-on-release velocity (px/ms) regardless of displacement.
pub const COMMIT_VELOCITY_PER_MS: f64 = 0.45;
/// Swipe-to-reply glyph "pop" threshold (px of row displacement).
pub const REPLY_POP_PX: f64 = 64.0;

/// The gesture's axis after the pointer leaves the slop radius. `None` until
/// committed. Horizontal wins only when dx dominates dy by `H_DOMINANCE`;
/// otherwise a vertical move past the slop locks V (a scroll). This is the
/// strip-vs-scroll arbitration.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Axis {
    Horizontal,
    Vertical,
}

/// Decide the locked axis from the running (dx, dy). `None` = not yet past slop.
pub fn axis_lock(dx: f64, dy: f64) -> Option<Axis> {
    if dx.abs() > AXIS_SLOP_PX && dx.abs() > dy.abs() * H_DOMINANCE {
        Some(Axis::Horizontal)
    } else if dy.abs() > AXIS_SLOP_PX {
        Some(Axis::Vertical)
    } else {
        None
    }
}

/// Swipe-to-reply wins over the channel strip ONLY when the press started on a
/// message row AND the horizontal travel is still small-radius (a short
/// right-drag on a row), per the #14/#5 arbitration rule. A large-radius
/// horizontal drag is a channel switch even if it began on a row.
pub fn row_swipe_wins(started_on_row: bool, dx: f64) -> bool {
    started_on_row && dx > 0.0 && dx < REPLY_POP_PX * 1.5
}

/// The reply glyph "pops" (and a haptic tick fires) at/after the pop threshold.
pub fn reply_armed(dx: f64) -> bool {
    dx >= REPLY_POP_PX
}

/// The strip's live `translateX` (px) while dragging pane index `idx` of
/// `count` panes in a viewport `width` wide, finger delta `dx`. Edges
/// rubber-band: a drag past the first/last pane is damped by `RUBBER_BAND`.
pub fn strip_offset(idx: usize, count: usize, width: f64, dx: f64) -> f64 {
    let base = -(idx as f64) * width;
    let at_first = idx == 0;
    let at_last = count == 0 || idx + 1 >= count;
    // Dragging right (dx>0) at the first pane, or left (dx<0) at the last, has
    // no neighbor — damp it.
    let extra = if (dx > 0.0 && at_first) || (dx < 0.0 && at_last) {
        dx * RUBBER_BAND
    } else {
        dx
    };
    base + extra
}

/// The committed strip move on pointer release.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StripCommit {
    Prev,
    Next,
    Stay,
}

/// Decide the commit from the release delta `dx`, elapsed `dt_ms`, and pane
/// `width`. Commits when |dx| ≥ `COMMIT_FRACTION`·width OR |velocity| >
/// `COMMIT_VELOCITY_PER_MS`. `dx<0` ⇒ Next (revealed the right neighbor),
/// `dx>0` ⇒ Prev. Edge guards (no prev at first / no next at last) are the
/// caller's job (it knows the neighbor exists); this returns the intent.
pub fn commit_swipe(dx: f64, dt_ms: f64, width: f64) -> StripCommit {
    let dt = dt_ms.max(1.0);
    let velocity = (dx / dt).abs();
    let past_displacement = dx.abs() >= COMMIT_FRACTION * width;
    if !past_displacement && velocity <= COMMIT_VELOCITY_PER_MS {
        return StripCommit::Stay;
    }
    if dx < 0.0 {
        StripCommit::Next
    } else if dx > 0.0 {
        StripCommit::Prev
    } else {
        StripCommit::Stay
    }
}

/// The destination channel INDEX for a committed strip swipe, given the current
/// index and channel count — the picker→switch decision the WASM handler runs
/// (extracted so it's unit-testable without a DOM; the project has no WASM UI
/// harness). `Next` ⇒ `cur+1` if it exists, `Prev` ⇒ `cur-1` if it exists,
/// `Stay`/edge ⇒ `None` (no switch). This is the SAME mapping `on_strip_commit`
/// (Task 5.2.1) and the orbit-map node tap drive, so testing it here covers the
/// roadmap's Phase-2 "picker channel-switch" acceptance at the act-decision
/// layer (the DOM wiring is then a thin pass-through).
pub fn commit_target(commit: StripCommit, cur_idx: usize, count: usize) -> Option<usize> {
    match commit {
        StripCommit::Next => cur_idx.checked_add(1).filter(|&j| j < count),
        StripCommit::Prev => cur_idx.checked_sub(1),
        StripCommit::Stay => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axis_lock_horizontal_needs_dominance_and_slop() {
        assert_eq!(axis_lock(5.0, 0.0), None, "under slop = uncommitted");
        assert_eq!(axis_lock(20.0, 2.0), Some(Axis::Horizontal));
        // dy too close to dx (within the 1.15 ratio) ⇒ not horizontal; if dy is
        // also past slop it locks vertical.
        assert_eq!(axis_lock(12.0, 12.0), Some(Axis::Vertical));
        assert_eq!(axis_lock(2.0, 20.0), Some(Axis::Vertical));
    }

    #[test]
    fn row_swipe_only_wins_small_radius_rightward_on_a_row() {
        assert!(row_swipe_wins(true, 30.0), "short right-drag on a row");
        assert!(!row_swipe_wins(false, 30.0), "not started on a row");
        assert!(!row_swipe_wins(true, -30.0), "leftward is not a reply");
        assert!(!row_swipe_wins(true, 200.0), "large-radius is a channel switch");
    }

    #[test]
    fn reply_glyph_pops_at_threshold() {
        assert!(!reply_armed(63.9));
        assert!(reply_armed(64.0));
    }

    #[test]
    fn strip_offset_tracks_one_to_one_in_the_middle() {
        // Middle pane (idx 1 of 3), 360px wide, dragged -100: base -360, extra -100.
        assert!((strip_offset(1, 3, 360.0, -100.0) - (-460.0)).abs() < 1e-9);
    }

    #[test]
    fn strip_offset_rubber_bands_at_edges() {
        // First pane dragged RIGHT (no prev) ⇒ damped by 0.32.
        assert!((strip_offset(0, 3, 360.0, 100.0) - (100.0 * RUBBER_BAND)).abs() < 1e-9);
        // Last pane dragged LEFT (no next) ⇒ base -720 + damped -100*0.32.
        let last = strip_offset(2, 3, 360.0, -100.0);
        assert!((last - (-720.0 + (-100.0 * RUBBER_BAND))).abs() < 1e-9);
        // First pane dragged LEFT (has a next) ⇒ full 1:1, no damping.
        assert!((strip_offset(0, 3, 360.0, -100.0) - (-100.0)).abs() < 1e-9);
    }

    #[test]
    fn commit_swipe_by_displacement_or_velocity() {
        let w = 360.0;
        // 33% displacement, slow ⇒ commit Next.
        assert_eq!(commit_swipe(-120.0, 1000.0, w), StripCommit::Next);
        // small displacement but fast flick ⇒ commit Prev.
        assert_eq!(commit_swipe(30.0, 40.0, w), StripCommit::Prev);
        // small + slow ⇒ Stay.
        assert_eq!(commit_swipe(20.0, 1000.0, w), StripCommit::Stay);
        // zero ⇒ Stay.
        assert_eq!(commit_swipe(0.0, 1000.0, w), StripCommit::Stay);
    }

    #[test]
    fn commit_target_maps_to_neighbor_index_with_edge_guards() {
        // Middle of a 4-channel guild: Next/Prev resolve to the neighbor.
        assert_eq!(commit_target(StripCommit::Next, 1, 4), Some(2));
        assert_eq!(commit_target(StripCommit::Prev, 1, 4), Some(1));
        // Edge guards: no next at the last channel, no prev at the first.
        assert_eq!(commit_target(StripCommit::Next, 3, 4), None, "no next at last");
        assert_eq!(commit_target(StripCommit::Prev, 0, 4), None, "no prev at first");
        // Stay never switches.
        assert_eq!(commit_target(StripCommit::Stay, 1, 4), None);
        // Empty/degenerate guild: nothing to switch to.
        assert_eq!(commit_target(StripCommit::Next, 0, 0), None);
    }
}
```

- [ ] 1.3.2 — Run and confirm the tests pass:

```bash
cargo test --features ssr sk_orbit::strip 2>&1 | tail -10
```

Expected: 7 tests pass.

- [ ] 1.3.3 — Commit:

```bash
git add src/ui/shell/sk_orbit/strip.rs
git commit -m "$(cat <<'EOF'
feat(ui): swipe-strip axis-lock + commit + target math (W5/P2 #5/#14)

Pure physics for Omloppsbana's signature gesture: axis_lock (H needs >10px AND
>1.15·dy, else V — the strip-vs-scroll arbitration), row_swipe_wins (reply wins
only small-radius rightward on a row, the #14/#5 rule), strip_offset (1:1 + 0.32
edge rubber-band), commit_swipe (≥32% OR >0.45px/ms), commit_target (the
picker→switch index mapping with edge guards — the testable substitute for the
roadmap's picker channel-switch test). Verified prototype constants; unit-tested.
No peek_settles: peek-never-marks-read holds structurally (name-only neighbors
never become current), so a ≥300ms timer would be dead code — deferred to the
lazy-neighbor follow-up that needs it.

Tests: sk_orbit::strip::tests::{axis_lock_horizontal_needs_dominance_and_slop,row_swipe_only_wins_small_radius_rightward_on_a_row,reply_glyph_pops_at_threshold,strip_offset_tracks_one_to_one_in_the_middle,strip_offset_rubber_bands_at_edges,commit_swipe_by_displacement_or_velocity,commit_target_maps_to_neighbor_index_with_edge_guards}
Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 1.4 — `orbit_map.rs` viewport-derived geometry

**Files:**
- Create: `src/ui/shell/sk_orbit/orbit_map.rs`
- (module already declared in `sk_orbit/mod.rs`)

All map geometry derives from the live viewport (UX-equality — no fixed-device pixel math). Constants from the prototype `mapGeom`/`buildMap`: orbit radius `clamp(min(vw/2-45, vh/2-160), 88, 170)`; far-server dock `farX=vw/2-70, farY=-(vh/2-clamp(vh*.16,96,150))`; nodes at `ci*(360/n)-90` degrees.

Steps:

- [ ] 1.4.1 — Write `src/ui/shell/sk_orbit/orbit_map.rs` with the failing-first tests:

```rust
//! W5/P2 orbit-map picker geometry (the diegetic guild/channel chooser; the
//! VISUAL survived the pinch-entry kill — pill-tap is the only entry). ALL
//! geometry derives from the live viewport (UX-equality: no fixed-device pixel
//! math; verified across the POCO C3 floor → Nothing Phone 2). Pure fns — the
//! view reads vw/vh from `window` and feeds them here. Constants are the
//! prototype's (`a-orbit.html` mapGeom/buildMap). No DOM.

/// Resolved geometry for the current viewport.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MapGeom {
    /// Channel-orbit ring radius (px).
    pub orbit_radius: f64,
    /// Far-server dock x offset from center (px, positive = right).
    pub far_x: f64,
    /// Far-server dock y offset from center (px, negative = up).
    pub far_y: f64,
}

/// A placed orbit node's center, relative to the orbit center (px).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct NodePos {
    pub x: f64,
    pub y: f64,
}

fn clamp_f(v: f64, lo: f64, hi: f64) -> f64 {
    v.max(lo).min(hi)
}

/// Derive the map geometry from viewport width/height.
pub fn map_geom(vw: f64, vh: f64) -> MapGeom {
    let orbit_radius = clamp_f((vw / 2.0 - 45.0).min(vh / 2.0 - 160.0), 88.0, 170.0);
    let far_x = vw / 2.0 - 70.0;
    let far_y = -(vh / 2.0 - clamp_f(vh * 0.16, 96.0, 150.0));
    MapGeom {
        orbit_radius,
        far_x,
        far_y,
    }
}

/// The angle (degrees) for channel node `idx` of `count` on the ring. Starts
/// at the top (-90°) and spaces evenly. `count==0` returns -90 (no nodes, but
/// callers guard count first).
pub fn node_angle(idx: usize, count: usize) -> f64 {
    if count == 0 {
        return -90.0;
    }
    idx as f64 * (360.0 / count as f64) - 90.0
}

/// The node center (relative to the orbit center) for `idx` of `count` at the
/// given `radius`. Uses the `node_angle` placement.
pub fn node_pos(idx: usize, count: usize, radius: f64) -> NodePos {
    let a = node_angle(idx, count).to_radians();
    NodePos {
        x: radius * a.cos(),
        y: radius * a.sin(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orbit_radius_clamps_on_the_poco_c3_floor() {
        // POCO C3 360x800: min(360/2-45, 800/2-160) = min(135, 240) = 135 → clamp 135.
        let g = map_geom(360.0, 800.0);
        assert!((g.orbit_radius - 135.0).abs() < 1e-9, "got {}", g.orbit_radius);
    }

    #[test]
    fn orbit_radius_hits_lower_clamp_on_a_short_viewport() {
        // A short viewport drives the min below 88 → clamped up to 88.
        let g = map_geom(300.0, 360.0);
        assert!((g.orbit_radius - 88.0).abs() < 1e-9, "got {}", g.orbit_radius);
    }

    #[test]
    fn orbit_radius_hits_upper_clamp_on_a_large_viewport() {
        // Desktop-ish 1200x900: min(555, 290)=290 → clamped down to 170.
        let g = map_geom(1200.0, 900.0);
        assert!((g.orbit_radius - 170.0).abs() < 1e-9, "got {}", g.orbit_radius);
    }

    #[test]
    fn far_dock_keeps_servers_on_screen() {
        let g = map_geom(360.0, 800.0);
        assert!((g.far_x - 110.0).abs() < 1e-9, "far_x got {}", g.far_x);
        // far_y = -(400 - clamp(128, 96, 150)) = -(400-128) = -272.
        assert!((g.far_y - (-272.0)).abs() < 1e-9, "far_y got {}", g.far_y);
    }

    #[test]
    fn first_node_is_at_top_subsequent_evenly_spaced() {
        assert!((node_angle(0, 4) - (-90.0)).abs() < 1e-9);
        assert!((node_angle(1, 4) - 0.0).abs() < 1e-9);
        assert!((node_angle(2, 4) - 90.0).abs() < 1e-9);
        // The first node sits straight up from center: x≈0, y≈-radius.
        let p = node_pos(0, 4, 100.0);
        assert!(p.x.abs() < 1e-9, "x got {}", p.x);
        assert!((p.y - (-100.0)).abs() < 1e-9, "y got {}", p.y);
    }

    #[test]
    fn node_angle_handles_single_node() {
        assert!((node_angle(0, 1) - (-90.0)).abs() < 1e-9);
    }
}
```

- [ ] 1.4.2 — Run and confirm the tests pass:

```bash
cargo test --features ssr sk_orbit::orbit_map 2>&1 | tail -10
```

Expected: 6 tests pass.

- [ ] 1.4.3 — Commit:

```bash
git add src/ui/shell/sk_orbit/orbit_map.rs
git commit -m "$(cat <<'EOF'
feat(ui): orbit-map picker geometry (viewport-derived, UX-equality) (W5/P2)

Pure geometry for the zoomable guild/channel picker (pill-tap entry only):
map_geom (orbit radius clamp(min(vw/2-45,vh/2-160),88,170); far-server dock),
node_angle (-90° start, even spacing), node_pos. No fixed-device pixel math;
floor-tested at POCO C3 360x800 + clamp edges + desktop. Unit-tested.

Tests: sk_orbit::orbit_map::tests::{orbit_radius_clamps_on_the_poco_c3_floor,orbit_radius_hits_lower_clamp_on_a_short_viewport,orbit_radius_hits_upper_clamp_on_a_large_viewport,far_dock_keeps_servers_on_screen,first_node_is_at_top_subsequent_evenly_spaced,node_angle_handles_single_node}
Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

## Task 2 — The shell seam: skeleton sibling switch (inline W3 branch, NOT a component)

This establishes the seam (gather:shell's recommendation): keep `AppShell` as the single owner of state + mount effects + overlays; replace the flat W3 chrome (mod.rs:436-831) with a `match s.prefs.skeleton` that picks `SkOrbitShell` for `orbit` and keeps the W3 chrome inline otherwise. The switch is a pure class-and-branch toggle on the same `Shell` aggregate — NO remount (pins `tests/skeleton_switch.rs::set_skeleton_surface_is_pref_only`).

### Task 2.1 — Keep the W3 chrome inline (no `W3Chrome` extraction)

**Files:**
- Modify: `src/ui/shell/mod.rs` (extract the `<nav class="rail">` … `<nav class="bottom-tabs">` block, mod.rs:436-831 — the inline block that step 2.2 wraps in the switch, NOT extracted to a component)

This is a MECHANICAL move — the block already only uses `use_context` + the local signals built in `AppShell`. The cleanest approach: keep the chrome inline but wrap it in the switch (see 2.2), OR extract to a sibling component that re-pulls context. Because the block references many `AppShell`-local signals (`new_server`, `editing_server`, `channel_manager_open`, `is_owner`, `server_name`, `guild_trash_open`, `account_open`, `accent_open`, the tab-active closures), extraction would require threading them all as props. Therefore:

- [ ] 2.1.1 — Do NOT extract to a separate component (too many local-signal props). Instead, leave the W3 chrome markup inline and wrap ONLY the navigation chrome region in a reactive branch. Confirm the decision is sound by listing the `AppShell`-local signals the W3 chrome closes over (read mod.rs:298-335 + the chrome block); they cannot cross a component boundary without prop threading, so an inline `{move || …}` branch in place is correct. No code change in this step — it records the architecture choice that 2.2 implements.

### Task 2.2 — Wrap the chrome in the skeleton switch; mount a minimal `SkOrbitShell`

**Files:**
- Modify: `src/ui/shell/mod.rs` (wrap mod.rs:436-831 region; import `SkOrbitShell`)
- Modify: `src/ui/shell/sk_orbit/mod.rs` (add the `SkOrbitShell` component — minimal: pane switch + topbar, no orbit chrome yet)

Steps:

- [ ] 2.2.1 — Add a minimal `SkOrbitShell` to `src/ui/shell/sk_orbit/mod.rs` (after the module-doc + `pub mod` lines from 1.1.1). It pulls `Shell` from context and mounts the IDENTICAL pane switch as the W3 `.content` block (mod.rs:763-769), so orbit users get a working app before the orbit chrome lands. It reuses the existing `.content`/`.channel-view`/topbar shape so all message/composer behavior is unchanged:

```rust
use leptos::prelude::*;

use super::{
    channel::ChannelPane, emoji_manager::EmojiManagerPane, friends::FriendsPane,
    lorebook::LorebookPane, members::MembersPane, Pane, Shell,
};

/// The Omloppsbana shell chrome. Renders as a sibling of the W3 chrome under
/// `.app.sk-orbit`, reusing every pane via `use_context::<Shell>()` (zero new
/// state, no remount on switch). This first cut mounts only the pane switch +
/// account control; the orbit chrome (pill, orbit map, composer orb, slide-
/// over) lands in later tasks. The full-viewport panes + swipe strip layout is
/// driven entirely by `style/_sk_orbit.scss` keyed off `.app.sk-orbit`.
#[component]
pub fn SkOrbitShell() -> impl IntoView {
    let s = use_context::<Shell>().expect("Shell provided by AppShell");
    let auth = use_context::<crate::ui::AuthCtx>().expect("AuthCtx provided at root");
    let username = move || {
        auth.user
            .get()
            .map(|u| u.username)
            .unwrap_or_default()
    };
    view! {
        <section class="content sk-orbit-content">
            <header class="topbar sk-orbit-topbar">
                <span class="muted">"Signed in as " <strong>{username}</strong></span>
                <span class="spacer"></span>
                <span class="sync-chip" class:live=move || s.sync.sse_live.get()>
                    {move || if s.sync.sse_live.get() { "● LIVE" } else { "● POLLING" }}
                </span>
            </header>
            {move || match s.sync.pane.get() {
                Pane::Friends => view! { <FriendsPane/> }.into_any(),
                Pane::Channel => view! { <ChannelPane/> }.into_any(),
                Pane::Lorebook => view! { <LorebookPane/> }.into_any(),
                Pane::Emoji => view! { <EmojiManagerPane/> }.into_any(),
                Pane::Members => view! { <MembersPane/> }.into_any(),
            }}
            <p class="error">{move || s.composer.status.get()}</p>
        </section>
    }
}
```

NOTE: the `use super::{… FriendsPane, …}` paths must resolve — `friends`, `lorebook`, `members`, `emoji_manager`, `channel` are `mod`s of `shell`, and `Pane`/`Shell` are `pub(crate)` in `shell`. If any pane component is not `pub`/`pub(crate)`-visible to `sk_orbit`, widen its visibility minimally (e.g. `pub(crate) use channel::ChannelPane;`) in `shell/mod.rs` — verify each import compiles in 2.2.4. Do NOT change pane internals.

- [ ] 2.2.2 — Export `SkOrbitShell` from `sk_orbit/mod.rs` (it's already `pub` via `#[component] pub fn`; add the import in `shell/mod.rs` near the other pane imports, mod.rs:47-54):

```rust
use sk_orbit::SkOrbitShell;
```

- [ ] 2.2.3 — Wrap the W3 chrome region in the skeleton switch. In `src/ui/shell/mod.rs`, the `.app` `<div>` currently contains the W3 `<nav class="rail">` … `<nav class="bottom-tabs">` block (436-831) followed by the shared overlays (837-891). Wrap ONLY the W3 navigation+content block (the rail, the trash panel, the channel sidebar, the `<section class="content">`, and the `<nav class="bottom-tabs">` — i.e. mod.rs:436-831) in a branch, leaving the `ChannelSheet`/account/wardrobe/confirm/toast/ceremony overlays (837-891) UNCONDITIONAL after it. Concretely, replace the opening of the W3 block (the `<nav class="rail" …>` at line 436) by opening a `{move || if s.prefs.skeleton.get().as_deref() == Some("orbit") { view! { <SkOrbitShell/> }.into_any() } else { view! { … all the W3 chrome … }.into_any() }}` wrapper, and close it just before the `ChannelSheet` block at line 837.

Because the W3 block is large and references `AppShell`-local signals, the precise edit is: insert at line 436 (before `<nav class="rail">`):

```rust
            // W5/P2: skeleton sibling switch. Orbit gets its own chrome; deck/
            // hud/None(scaffold) keep the W3 chrome (Phase-6 retirement deletes
            // the else arm + its partials). The switch is a pure branch on the
            // same Shell aggregate / same .app root — no remount (pins
            // tests/skeleton_switch.rs::set_skeleton_surface_is_pref_only).
            {move || if s.prefs.skeleton.get().as_deref() == Some("orbit") {
                view! { <SkOrbitShell/> }.into_any()
            } else {
                view! {
```

and insert at line 831 (after the closing `</nav>` of `bottom-tabs`, before the `ChannelSheet` comment at 833):

```rust
                }.into_any()
            }}
```

This makes the whole W3 chrome the `else` arm of a single `move ||` branch. The shared overlays after it stay outside the branch (skeleton-agnostic, as gather:shell requires).

- [ ] 2.2.4 — Verify both graphs compile and the switch test still passes:

```bash
cargo clippy --features ssr -- -D warnings 2>&1 | tail -8 \
 && cargo clippy --features hydrate --target wasm32-unknown-unknown -- -D warnings 2>&1 | tail -8 \
 && cargo test --features ssr --test skeleton_switch 2>&1 | tail -6
```

Expected: both clippy clean; `skeleton_switch` tests pass (the seam doesn't change `set_skeleton`'s surface).

- [ ] 2.2.5 — Visual smoke (headed Playwright, localhost:3000 + dev DB): with `Prefs.skeleton == orbit` (the default fallback / pick orbit in the ceremony), confirm the app loads, a channel opens, messages render, send works, and switching to `deck`/`hud` via the account modal flips back to the W3 chrome WITHOUT a reload (SSE chip stays LIVE, composer draft survives). Then commit:

```bash
git add src/ui/shell/mod.rs src/ui/shell/sk_orbit/mod.rs
git commit -m "$(cat <<'EOF'
feat(ui): sk-orbit shell seam — SkOrbitShell sibling branch (W5/P2)

AppShell stays the single owner of state/effects/overlays; the W3 nav+content
chrome becomes the else arm of a `move || skeleton==orbit` branch, with orbit
mounting SkOrbitShell (pane switch + topbar, reusing every pane via context).
Pure class-and-branch toggle on the same Shell aggregate — no remount. Orbit
chrome (pill/map/orb/slide-over) lands next.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

## Task 3 — `_sk_orbit.scss` scaffold + `--warp-dir` directional wiring

### Task 3.1 — Register the partial + base full-viewport layout

**Files:**
- Create: `style/_sk_orbit.scss`
- Modify: `style/main.scss` (add `@use "sk_orbit";` after line 25, `@use "nav";`)

Steps:

- [ ] 3.1.1 — Create `style/_sk_orbit.scss` with the header + base layout. ALL rules are `.app.sk-orbit`-prefixed (compound, no space before the second class — the `.app.fx-max` precedent at `_foundation.scss:77`) so they never bleed onto deck/hud or the retained W3 scaffolding. The base makes orbit a single full-viewport column (the W3 grid columns are not used under orbit):

```scss
// W5/P2 Omloppsbana (`sk-orbit`) structural skeleton. Full-viewport channel
// panes in a horizontal swipe strip, a holographic channel pill opening the
// orbit-map picker (pill-tap entry only), a floating composer orb with a charge
// ring + effect blossom, and a right-edge HoloPanel slide-over. EVERY rule is
// `.app.sk-orbit`-prefixed so it can never match a deck/hud session or the
// retained W3 scaffolding. Loaded AFTER the W3 nav partials in main.scss so an
// equal-specificity tie resolves in orbit's favour. Motion is transform/opacity
// only (tests/style_lint.rs); glass is glass-etched at Standard, glass-live
// under .fx-max. Safe-area: each edge is owned exactly once (the pill owns top,
// the orb/composer owns bottom, the slide-over owns its own edges).
@use "tokens" as *;
@use "foundation" as *;

// Orbit is a single full-viewport stage — the W3 rail/sidebar columns are not
// part of this skeleton. The shared overlays (sheet/modals/toast/ceremony) and
// the body-portal overlays (radial/lightbox/emoji) are unchanged.
.app.sk-orbit {
	display: block;
	position: relative;
	min-height: 100dvh;
}
.app.sk-orbit .content.sk-orbit-content {
	height: 100dvh;
	height: 100vh; // fallback first; dvh wins where supported
	display: flex;
	flex-direction: column;
}
// The pill owns the TOP safe-area inset under orbit (the topbar replacement);
// the orbit topbar is a thin strip — the pill floats above the pane.
.app.sk-orbit .topbar.sk-orbit-topbar {
	padding-top: calc(0.4rem + env(safe-area-inset-top, 0px)); // OWNS top edge
}
```

- [ ] 3.1.2 — Register the partial in `style/main.scss` — insert after line 25 (`@use "nav";`), before line 26 (`@use "attachments";`):

```scss
@use "sk_orbit"; // W5/P2 Omloppsbana — after the W3 scaffolding it overrides
```

- [ ] 3.1.3 — Verify the SCSS compiles (via the leptos build) and the style lint passes (no keyframes yet, but the partial is now scanned):

```bash
cargo test --features ssr --test style_lint 2>&1 | tail -5 \
 && cargo leptos build 2>&1 | tail -8
```

Expected: `style_lint` passes; the build compiles SCSS without a SASS error.

- [ ] 3.1.4 — Commit:

```bash
git add style/_sk_orbit.scss style/main.scss
git commit -m "$(cat <<'EOF'
feat(ui): _sk_orbit.scss scaffold + register after nav (W5/P2)

Base full-viewport layout for the orbit skeleton; every rule .app.sk-orbit-
prefixed so it never bleeds to deck/hud or the W3 scaffolding. Loaded after the
W3 nav partials so equal-specificity ties resolve to orbit. Pill owns the top
safe-area inset.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 3.2 — Set `--warp-dir` from the channel-list index on picker switches

**Files:**
- Modify: `src/ui/shell/act/channel.rs` (`open_channel_at`, hydrate body around lines 119-142) — set `--warp-dir` from `warp_dir(from_idx, to_idx)` before the switch
- (uses `sk_orbit::warp::warp_dir` from Task 1.1)

The Foundation deferred the directional sign to Phase 2. The act layer knows the outgoing + incoming channel and the current channel list, so it can compute the index sign and write `--warp-dir` on `:root` (or `.app`). This applies on ALL switches but is only VISIBLE in orbit (where the swipe strip + warp use it); under W3 it's harmless (the dip is still neutral-looking because the W3 transition is the same `.channel-view`).

Steps:

- [ ] 3.2.1 — In `src/ui/shell/act/channel.rs` `open_channel_at` (hydrate impl), AFTER `let same_channel = …` (line 120) and inside the `if !same_channel {` block that sets `s.sync.switching` (line 134), compute the index sign from `s.sel.channels` and write the `--warp-dir` custom property on the document root. Add this just before `s.sync.switching.set(true);` (line 135):

```rust
        // W5/P2: set the directional warp sign (deferred from Foundation T0.2)
        // from the channel-list index direction of this switch. Written on the
        // document root so `.channel-view.fx-switching` (_content.scss:46) reads
        // it. Visible in orbit (swipe strip + warp); harmless under W3.
        {
            let chans = s.sel.channels.get_untracked();
            let from_idx = s
                .sel
                .sel_channel
                .get_untracked()
                .and_then(|c| chans.iter().position(|x| x.id == c.id));
            let to_idx = chans.iter().position(|x| x.id == cid);
            let dir = crate::ui::shell::sk_orbit::warp::warp_dir(from_idx, to_idx);
            if let Some(root) = leptos::web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.document_element())
            {
                use leptos::wasm_bindgen::JsCast as _;
                if let Some(html) = root.dyn_ref::<leptos::web_sys::HtmlElement>() {
                    let _ = html.style().set_property("--warp-dir", &dir.to_string());
                }
            }
        }
```

(`cid` is in scope from line 113; `crate::ui::shell::sk_orbit::warp` is the path — `sk_orbit` is `mod sk_orbit;` in `shell`, `warp` is `pub mod`.)

- [ ] 3.2.2 — There is no automated test for the DOM write (no WASM harness); the LOGIC (`warp_dir`) is already unit-tested in Task 1.1. Verify compilation on both graphs:

```bash
cargo clippy --features ssr -- -D warnings 2>&1 | tail -6 \
 && cargo clippy --features hydrate --target wasm32-unknown-unknown -- -D warnings 2>&1 | tail -6
```

Expected: both clean (the ssr `open_channel_at` is the no-op stub; only the hydrate body has this code).

- [ ] 3.2.3 — Visual smoke (headed Playwright, localhost:3000 + dev DB, eye-candy ON): switch from channel #1 to a LOWER channel and a HIGHER channel; confirm the warp dip slides from opposite directions (`--warp-dir` flips sign). Then commit:

```bash
git add src/ui/shell/act/channel.rs
git commit -m "$(cat <<'EOF'
feat(ui): set --warp-dir directional sign on channel switch (W5/P2 #A)

open_channel_at computes warp_dir(from_idx,to_idx) from the channel-list index
direction and writes --warp-dir on the document root before the switch, so
.channel-view.fx-switching slides from the destination's side. Deferred from
Foundation T0.2; visible in orbit, harmless under W3.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

## Task 4 — Holographic channel pill + orbit-map picker (pill-tap entry ONLY)

The pill is the only entry to the picker (pinch entry judge-killed — KILL 1). The orbit-map visual survives; it draws ONLY client-visible guilds/channels (privacy-404 untouched). Tapping a node dives to that channel; tapping a far-server core glides to it. Modal-parity focus required (trap, Esc, restore-to-trigger).

PICKER CHANNEL-SWITCH TEST (roadmap Phase-2 acceptance): the project has NO WASM/DOM UI harness, so the picker→switch path cannot be driven end-to-end in `cargo test`. The decision logic IS unit-tested at the act-decision layer: the swipe strip's commit maps through `strip::commit_target` (Task 1.3, `commit_target_maps_to_neighbor_index_with_edge_guards`); the orbit-map node tap has NO index indirection to extract — each node closure already owns its `ChannelSummary` and calls `act::open_channel(s, ch.clone())` directly, so the only testable orbit-map decisions are the GEOMETRY (`map_geom`/`node_angle`/`node_pos`, Task 1.4) and the visible-only filter (it iterates `s.sel.guilds`/`s.sel.channels`, which are already privacy-scoped server-side). The live click→switch wiring is then a thin pass-through, verified by the headed smoke (4.2.4 / 9.4.2 item 2). This pure-fn coverage + headed smoke is the agreed substitute for a live integration test; it is recorded here because the named acceptance item has no other automated form.

### Task 4.1 — The pill component (top-center, glass, position dots)

**Files:**
- Modify: `src/ui/shell/sk_orbit/mod.rs` (add a `map_open` signal + the pill markup to `SkOrbitShell`)
- Modify: `style/_sk_orbit.scss` (pill styling)

Steps:

- [ ] 4.1.1 — Add a `map_open: RwSignal<bool>` to `SkOrbitShell` and render the pill above the pane. The pill shows the open channel name + server name + one position dot per channel (current highlighted), min-height 44px. Pill TAP opens the map. Add at the top of `SkOrbitShell`'s body (after the `username` closure) and render the pill as the first child of the `<section class="content sk-orbit-content">`:

```rust
    let map_open = RwSignal::new(false);
    let channel_name = move || {
        s.sel
            .sel_channel
            .get()
            .map(|c| c.name)
            .unwrap_or_else(|| "—".to_string())
    };
    let server_name = move || {
        let sid = s.sel.sel_server.get();
        s.sel
            .guilds
            .get()
            .into_iter()
            .find(|g| Some(&g.id) == sid.as_ref())
            .map(|g| g.name)
            .unwrap_or_default()
    };
```

pill markup (first child inside `<section class="content sk-orbit-content">`, before the topbar):

```rust
            <button class="sk-orbit-pill" type="button"
                aria-haspopup="dialog"
                aria-expanded=move || map_open.get().to_string()
                title="Open the orbit map"
                on:click=move |_| map_open.set(true)>
                <span class="sk-orbit-pill-name">"# "{channel_name}</span>
                <span class="sk-orbit-pill-server">{server_name}</span>
                <span class="sk-orbit-pill-dots" aria-hidden="true">
                    {move || {
                        let chans = s.sel.channels.get();
                        let cur = s.sel.sel_channel.get().map(|c| c.id);
                        chans.into_iter().map(|c| {
                            let on = Some(&c.id) == cur.as_ref();
                            view! { <span class="sk-orbit-dot" class:on=on></span> }
                        }).collect_view()
                    }}
                </span>
            </button>
```

- [ ] 4.1.2 — Style the pill in `style/_sk_orbit.scss` (glass-etched at Standard, glass-live under fx-max; top-center floating; min-height 44px). No keyframes:

```scss
.app.sk-orbit .sk-orbit-pill {
	position: fixed;
	top: calc(0.5rem + env(safe-area-inset-top, 0px));
	left: 50%;
	transform: translateX(-50%);
	z-index: 30;
	min-height: 44px;
	display: flex;
	align-items: center;
	gap: 0.5rem;
	padding: 0.35rem 0.9rem;
	border-radius: 999px;
	color: var(--text);
	@include glass-etched;
}
.app.fx-max .sk-orbit-pill {
	@include glass-live;
}
.app.sk-orbit .sk-orbit-pill-server {
	color: var(--text-muted);
	font-family: var(--font-mono); // defined token (_tokens.scss:95); no fallback needed
	font-size: 0.8em;
}
.app.sk-orbit .sk-orbit-pill-dots {
	display: inline-flex;
	gap: 3px;
}
.app.sk-orbit .sk-orbit-dot {
	width: 5px;
	height: 5px;
	border-radius: 50%;
	background: var(--text-muted);
	opacity: 0.4;
}
.app.sk-orbit .sk-orbit-dot.on {
	background: var(--accent);
	opacity: 1;
}
```

NOTE: the tokens used here are all CONFIRMED present in `style/_tokens.scss`: `--text` (:26), `--text-muted` (:28), `--accent` (:32), `--font-mono` (:95), and the `--tint-*` family (:80-87). There is NO `--font-meta` token — the mono stack is `--font-mono` (used above, no fallback needed). Any NEW custom property introduced inline by this plan (`--strip-x`, `--charge`, `--dash`, `--chip-i`, `--scene-tint`) is set by orbit's own Rust/SCSS, not a `_tokens.scss` token, so it correctly carries an inline fallback.

- [ ] 4.1.3 — Verify build + lint:

```bash
cargo leptos build 2>&1 | tail -6 && cargo test --features ssr --test style_lint 2>&1 | tail -4
```

Expected: build compiles; style_lint passes.

- [ ] 4.1.4 — Commit:

```bash
git add src/ui/shell/sk_orbit/mod.rs style/_sk_orbit.scss
git commit -m "$(cat <<'EOF'
feat(ui): orbit holographic channel pill — pill-tap picker entry (W5/P2)

Top-center floating glass pill (etched at Standard, live under fx-max), 44px
min-height, channel + server name + one position dot per channel (current
highlighted). Tap opens the orbit map (the ONLY entry — pinch was judge-killed).

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 4.2 — The orbit-map overlay (dive-to-channel + server glide)

**Files:**
- Modify: `src/ui/shell/sk_orbit/mod.rs` (the orbit-map overlay, portaled to body, focus-trapped)
- Modify: `style/_sk_orbit.scss` (orbit-map styling + transform/opacity keyframes)

Steps:

- [ ] 4.2.1 — Add the orbit-map overlay to `SkOrbitShell`, rendered under `<Show when=move || map_open.get()>` and PORTALED to `document.body` (it's a full-viewport overlay; use the `Portal` import pattern from `channel/mod.rs:47`). It reads vw/vh on render, computes geometry via `orbit_map::map_geom`, places the active server's channels as nodes via `node_pos`, docks other servers at `far_x/far_y`. A node tap calls `act::open_channel` then closes the map; a far-server core tap calls `act::open_server`. Modal-parity: a NodeRef focused on mount, Esc closes, focus restores to the pill. Add the import at the top of `sk_orbit/mod.rs`:

```rust
use leptos::portal::Portal;
use super::orbit_map::{map_geom, node_pos};
```

overlay markup (after the pane switch block, still inside `SkOrbitShell`'s `view!`):

```rust
            {move || map_open.get().then(|| view! {
                <Portal>
                    <div class="sk-orbit-map" role="dialog" aria-modal="true"
                        aria-label="Orbit map — pick a channel or server" tabindex="-1"
                        on:keydown=move |ev: leptos::ev::KeyboardEvent| {
                            if ev.key() == "Escape" {
                                ev.prevent_default();
                                map_open.set(false);
                            }
                        }>
                        <button class="sk-orbit-map-scrim" aria-label="Close orbit map"
                            on:click=move |_| map_open.set(false)></button>
                        <div class="sk-orbit-core">{server_name}</div>
                        {move || {
                            // Geometry from the live viewport (UX-equality).
                            let (vw, vh) = viewport_dims();
                            let g = map_geom(vw, vh);
                            let chans = s.sel.channels.get();
                            let n = chans.len();
                            let unread = s.notify.unread.get();
                            chans.into_iter().enumerate().map(|(i, c)| {
                                let p = node_pos(i, n, g.orbit_radius);
                                let cid = c.id.clone();
                                let has_unread = unread.contains(&c.id);
                                let ch = c.clone();
                                view! {
                                    <button class="sk-orbit-node"
                                        class:unread=has_unread
                                        style:transform=format!(
                                            "translate(calc(50vw + {}px), calc(50vh + {}px)) translate(-50%, -50%)",
                                            p.x, p.y
                                        )
                                        title=c.name.clone()
                                        on:click=move |_| {
                                            act::open_channel(s, ch.clone());
                                            map_open.set(false);
                                        }>
                                        <span class="sk-orbit-node-label">{c.name.clone()}</span>
                                        {has_unread.then(|| view! { <span class="sk-orbit-node-dot" aria-hidden="true"></span> })}
                                        {let _ = &cid;}
                                    </button>
                                }
                            }).collect_view()
                        }}
                        {move || {
                            // Other servers docked in the top corners.
                            let (vw, vh) = viewport_dims();
                            let g = map_geom(vw, vh);
                            let cur = s.sel.sel_server.get();
                            s.sel.guilds.get().into_iter()
                                .filter(|gd| Some(&gd.id) != cur.as_ref())
                                .enumerate()
                                .map(|(i, gd)| {
                                    let gid = gd.id.clone();
                                    // Alternate left/right docks so multiple far
                                    // servers stay on-screen.
                                    let side = if i % 2 == 0 { 1.0 } else { -1.0 };
                                    view! {
                                        <button class="sk-orbit-far"
                                            style:transform=format!(
                                                "translate(calc(50vw + {}px), calc(50vh + {}px)) translate(-50%, -50%)",
                                                g.far_x * side, g.far_y
                                            )
                                            title=gd.name.clone()
                                            on:click=move |_| {
                                                act::open_server(s, gid.clone());
                                                map_open.set(false);
                                            }>
                                            {gd.name.clone()}
                                        </button>
                                    }
                                }).collect_view()
                        }}
                    </div>
                </Portal>
            })}
```

add the `viewport_dims` helper near the top of `sk_orbit/mod.rs` (hydrate-real + ssr stub, mirroring `is_mobile_viewport` in `channel/mod.rs:301-311`):

```rust
/// Live viewport (width, height) in CSS px. Falls back to the POCO C3 floor
/// off-DOM / on ssr so the geometry is sane before hydrate.
#[cfg(feature = "hydrate")]
fn viewport_dims() -> (f64, f64) {
    let win = leptos::web_sys::window();
    let w = win
        .as_ref()
        .and_then(|w| w.inner_width().ok())
        .and_then(|v| v.as_f64())
        .unwrap_or(360.0);
    let h = win
        .and_then(|w| w.inner_height().ok())
        .and_then(|v| v.as_f64())
        .unwrap_or(800.0);
    (w, h)
}
#[cfg(not(feature = "hydrate"))]
fn viewport_dims() -> (f64, f64) {
    (360.0, 800.0)
}
```

NOTE: the `style:transform` uses `calc(50vw + Npx)` so the placement is viewport-centered without reading layout per node. Confirm `act::open_channel` + `act::open_server` are re-exported (gather:shell confirms `open_server` is in the `act` re-export; `open_channel` is `pub` on hydrate, ssr stub exists — verify the call site compiles on both graphs). Modal focus-restore-to-pill: add a NodeRef on the `.sk-orbit-map` div + an `on_load` focus, and on close restore focus to the pill (store a pill NodeRef). If wiring the restore is heavy, at minimum the Esc + scrim-close + focus-on-open must work; the full restore-to-trigger is a gate item — implement it via a pill `NodeRef` captured in `SkOrbitShell` and `.focus()`ed in the `map_open.set(false)` paths.

- [ ] 4.2.2 — Style the orbit-map in `style/_sk_orbit.scss`. The chat layer warps out (scale .06 + fade) on open and in on dive — these are transform/opacity keyframes (lint-safe). Add:

```scss
.app.sk-orbit .sk-orbit-map {
	position: fixed;
	inset: 0;
	z-index: 80;
	// Owns its own insets (body-level overlay, like the body-portal contract).
	padding: env(safe-area-inset-top, 0px) env(safe-area-inset-right, 0px)
		env(safe-area-inset-bottom, 0px) env(safe-area-inset-left, 0px);
	animation: sk-orbit-map-in 220ms ease-out;
}
@keyframes sk-orbit-map-in {
	from { opacity: 0; transform: scale(1.04); }
	to { opacity: 1; transform: scale(1); }
}
.app.sk-orbit .sk-orbit-map-scrim {
	position: absolute;
	inset: 0;
	background: var(--scrim);
	border: none;
}
.app.sk-orbit .sk-orbit-core {
	position: absolute;
	top: 50vh;
	left: 50vw;
	transform: translate(-50%, -50%);
	padding: 1rem 1.4rem;
	border-radius: 999px;
	@include glass-etched;
	color: var(--text);
	z-index: 1;
}
.app.sk-orbit .sk-orbit-node,
.app.sk-orbit .sk-orbit-far {
	position: absolute;
	top: 0;
	left: 0;
	z-index: 2;
	min-width: 44px;
	min-height: 44px;
	border-radius: 999px;
	@include glass-etched;
	color: var(--text);
	padding: 0.4rem 0.7rem;
}
.app.sk-orbit .sk-orbit-node-dot {
	position: absolute;
	top: 2px;
	right: 2px;
	width: 8px;
	height: 8px;
	border-radius: 50%;
	background: var(--unread-glow);
}
// Reduced-motion: the open animation is decorative; the class lacks `fx-`, so
// list it explicitly (the global kill only catches [class*="fx-"]).
@media (prefers-reduced-motion: reduce) {
	.app.sk-orbit .sk-orbit-map {
		animation: none;
	}
}
```

- [ ] 4.2.3 — Verify build + lint (the new keyframe must be transform/opacity only):

```bash
cargo leptos build 2>&1 | tail -6 \
 && cargo test --features ssr --test style_lint 2>&1 | tail -5 \
 && cargo clippy --features hydrate --target wasm32-unknown-unknown -- -D warnings 2>&1 | tail -6
```

Expected: build compiles; `style_lint` passes (`sk-orbit-map-in` animates only opacity+transform); hydrate clippy clean.

- [ ] 4.2.4 — Visual smoke (headed Playwright, localhost:3000 + dev DB): tap the pill → map opens with the active server's channels on a ring + other servers docked; tap a node → dives to that channel + map closes; tap a far core → switches server; Esc closes; focus lands back on the pill. Confirm only visible guilds/channels appear (privacy). Then commit:

```bash
git add src/ui/shell/sk_orbit/mod.rs style/_sk_orbit.scss
git commit -m "$(cat <<'EOF'
feat(ui): orbit-map picker overlay — dive-to-channel + server glide (W5/P2)

Body-portaled, focus-trapped (role=dialog, Esc, scrim-close, focus-on-open,
restore-to-pill). Active server's channels placed on a viewport-derived ring
(orbit_map::node_pos), other servers docked at far_x/far_y; node tap dives via
act::open_channel, far core via act::open_server. Draws only client-visible
guilds/channels (privacy-404 untouched). Open animation is opacity/transform.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

## Task 5 — Horizontal swipe strip (1 live pane + 2 label peeks; full neighbor render deferred)

The signature gesture: the open channel is the MIDDLE pane of a 3-slot strip; the strip transform follows the finger 1:1 via the `strip.rs` math; release commits via `commit_swipe` → `commit_target`. The gesture handler uses the proven hydrate-real + ssr-stub struct pattern (`radial::LongPress` / `holopanel::PanelDrag`), bound ungated.

SCOPE HONESTY (the spec/roadmap describe THREE *content* panes; Phase 2 ships ONE): only the MIDDLE slot is a real `ChannelPane` (owns the composer + message list, `content-visibility:auto` on its rows via the #36 foundation, `_content.scss:154`). The prev/next slots are NAME-ONLY label peeks (`# channelname`, Task 5.2.3), not the neighbor's messages — so "full-viewport channel panes in a horizontal swipe strip" is ~1/3 delivered. The lazy first-page neighbor render (read-only, NOT touching last_seen) is an explicit Phase-7 carry (9.4.3-b). Do NOT read this task's acceptance as "3-pane content strip"; it is "1 live pane + 2 label peeks".

Two Phase-2 feasibility taxes the roadmap names — both resolved, NOT shipped as code, BECAUSE the neighbors are name-only:
- **peek-never-marks-read:** holds STRUCTURALLY — a name-only neighbor never becomes "current" and never calls `act::open_channel`/`set_last_seen`, and a committed swipe correctly marks the now-active destination read (`channel.rs:305`). No ≥300ms `peek_settles` timer is shipped (it would be dead code; see the Task 1.3 intro). It becomes live only with the lazy render (9.4.3-c).
- **SSE open-channel semantics for peeked neighbors + memory mgmt:** N/A for Phase 2 — name-only neighbors hold no message state and open no neighbor SSE subscription, so there is nothing to refetch-on-settle or to free. SSE-across-swipe rides entirely on the no-remount structure (orbit is a sibling branch on the same `Shell`; the middle `ChannelPane` is not torn down by a swipe — `act::open_channel` swaps its message list in place, it does not remount the pane). That structural guarantee is the real evidence; the 9.4.2 smoke "SSE chip stays LIVE across a swipe" is a check, not the proof. The peeked-neighbor SSE design (filtered `visible_channels`, refetch-on-settle) must be done WHEN the lazy render lands.

### Task 5.1 — The `StripDrag` gesture struct (hydrate-real + ssr stub)

**Files:**
- Create: `src/ui/shell/sk_orbit/drag.rs`
- Modify: `src/ui/shell/sk_orbit/mod.rs` (`pub mod drag;`)

This mirrors `holopanel::PanelDrag` exactly: always-on struct, `#[cfg(hydrate)]` fields, hydrate-real impl + ssr no-op stub. It writes the live `translateX` to a `--strip-x` custom property (no per-move signal re-render — the lightbox/holopanel discipline) and, on release, computes `commit_swipe` and calls the supplied commit callback.

Steps:

- [ ] 5.1.1 — Create `src/ui/shell/sk_orbit/drag.rs` with the struct + handlers. It uses `set_pointer_capture` (confirmed in-tree, `lightbox.rs:530` / `holopanel.rs:123`), and routes `pointercancel` to the SAME release path as `pointerup` (the lightbox M-35 lesson). The commit decision uses the pure `strip.rs` fns:

```rust
//! W5/P2 swipe-strip drag engine. Mirrors `holopanel::PanelDrag` /
//! `radial::LongPress`: an always-on struct (the `<div>` binds its methods
//! ungated) with hydrate-only fields + a real impl paired to an ssr no-op stub.
//! Per-move it writes `--strip-x` (no signal re-render — the lightbox/holopanel
//! discipline); on release it runs the pure `strip` math and fires `on_commit`.
//! `pointercancel` shares the release path with `pointerup` (lightbox M-35).

use leptos::prelude::*;

use super::strip::{axis_lock, commit_swipe, strip_offset, Axis, StripCommit};

/// The strip drag engine. `on_commit(StripCommit)` is called once on a real
/// release that commits (Prev/Next); a Stay snaps back with no callback. The
/// caller owns the pane index/count + the actual channel switch.
#[derive(Clone)]
pub struct StripDrag {
    #[cfg(feature = "hydrate")]
    idx: StoredValue<usize>,
    #[cfg(feature = "hydrate")]
    count: StoredValue<usize>,
    #[cfg(feature = "hydrate")]
    on_commit: Callback<StripCommit>,
    #[cfg(feature = "hydrate")]
    strip_ref: NodeRef<leptos::html::Div>,
    /// (start_x, start_y, start_t) at pointerdown, else None.
    #[cfg(feature = "hydrate")]
    start: RwSignal<Option<(f64, f64, f64)>>,
    /// Locked axis once past slop, else None.
    #[cfg(feature = "hydrate")]
    axis: RwSignal<Option<Axis>>,
}

#[cfg(feature = "hydrate")]
impl StripDrag {
    pub fn new(
        idx: StoredValue<usize>,
        count: StoredValue<usize>,
        on_commit: Callback<StripCommit>,
        strip_ref: NodeRef<leptos::html::Div>,
    ) -> Self {
        Self {
            idx,
            count,
            on_commit,
            strip_ref,
            start: RwSignal::new(None),
            axis: RwSignal::new(None),
        }
    }

    pub fn down(&self, ev: &leptos::ev::PointerEvent) {
        use leptos::wasm_bindgen::JsCast as _;
        if let Some(el) = self.strip_ref.get_untracked() {
            let el: &leptos::web_sys::Element = (*el).unchecked_ref();
            let _ = el.set_pointer_capture(ev.pointer_id());
        }
        self.start
            .set(Some((ev.client_x() as f64, ev.client_y() as f64, ev.time_stamp())));
        self.axis.set(None);
    }

    pub fn moved(&self, ev: &leptos::ev::PointerEvent) {
        let Some((sx, sy, _)) = self.start.get_untracked() else {
            return;
        };
        let dx = ev.client_x() as f64 - sx;
        let dy = ev.client_y() as f64 - sy;
        // Lock the axis once past slop; only track horizontal drags.
        let axis = self.axis.get_untracked().or_else(|| {
            let a = axis_lock(dx, dy);
            if a.is_some() {
                self.axis.set(a);
            }
            a
        });
        if axis != Some(Axis::Horizontal) {
            return;
        }
        // Horizontal: prevent the page from scrolling and track 1:1 + rubber-band.
        ev.prevent_default();
        let width = viewport_width();
        let offset = strip_offset(
            self.idx.get_value(),
            self.count.get_value(),
            width,
            dx,
        );
        self.write_strip_x(offset);
    }

    pub fn up(&self, ev: &leptos::ev::PointerEvent) {
        let Some((sx, _, st)) = self.start.get_untracked() else {
            return;
        };
        self.start.set(None);
        let was_h = self.axis.get_untracked() == Some(Axis::Horizontal);
        self.axis.set(None);
        if !was_h {
            return;
        }
        let dx = ev.client_x() as f64 - sx;
        let dt = ev.time_stamp() - st;
        let width = viewport_width();
        let commit = commit_swipe(dx, dt, width);
        // Snap back to the resting offset for the (possibly new) index; the
        // caller's on_commit advances the channel, which re-renders the strip.
        self.write_strip_x(-(self.idx.get_value() as f64) * width);
        if commit != StripCommit::Stay {
            self.on_commit.run(commit);
        }
    }

    fn write_strip_x(&self, px: f64) {
        use leptos::wasm_bindgen::JsCast as _;
        if let Some(el) = self.strip_ref.get_untracked() {
            if let Some(html) = (*el).dyn_ref::<leptos::web_sys::HtmlElement>() {
                let _ = html
                    .style()
                    .set_property("--strip-x", &format!("{px}px"));
            }
        }
    }
}

#[cfg(feature = "hydrate")]
fn viewport_width() -> f64 {
    leptos::web_sys::window()
        .and_then(|w| w.inner_width().ok())
        .and_then(|v| v.as_f64())
        .unwrap_or(360.0)
}

/// ssr stubs: never run (pointer events are browser-only) but the `<div>`
/// bindings are always-on and must typecheck on the server.
#[cfg(not(feature = "hydrate"))]
impl StripDrag {
    pub fn down(&self, _ev: &leptos::ev::PointerEvent) {}
    pub fn moved(&self, _ev: &leptos::ev::PointerEvent) {}
    pub fn up(&self, _ev: &leptos::ev::PointerEvent) {}
}
```

NOTE: the ssr build needs a way to CONSTRUCT a `StripDrag` for the always-on `view!` binding. Mirror `holopanel.rs`: on ssr the struct has no fields, so add an ssr `new(...)` that ignores its args, OR construct it field-by-field with `#[cfg(feature="hydrate")]` on each field (the `holopanel.rs:304-317` pattern). Use the field-by-field construction in `SkOrbitShell` (Task 5.2) so no ssr `new` is needed — matching HoloPanel exactly. If a `new` is cleaner, add a `#[cfg(not(feature="hydrate"))] pub fn new(...) -> Self { Self {} }` too. Verify both graphs compile in 5.2.

- [ ] 5.1.2 — Register the module in `sk_orbit/mod.rs`:

```rust
pub mod drag;
```

- [ ] 5.1.3 — Verify both graphs compile (no test — the gesture decisions are already tested in `strip.rs`; this is the thin DOM wrapper):

```bash
cargo clippy --features ssr -- -D warnings 2>&1 | tail -8 \
 && cargo clippy --features hydrate --target wasm32-unknown-unknown -- -D warnings 2>&1 | tail -8
```

Expected: both clean.

- [ ] 5.1.4 — Commit:

```bash
git add src/ui/shell/sk_orbit/drag.rs src/ui/shell/sk_orbit/mod.rs
git commit -m "$(cat <<'EOF'
feat(ui): StripDrag swipe engine (hydrate-real + ssr stub) (W5/P2 #5)

Mirrors holopanel::PanelDrag: always-on struct, hydrate-only fields, ssr no-op
stub, bound ungated. set_pointer_capture (lightbox.rs proven); pointercancel
shares the release path with pointerup (M-35 lesson); writes --strip-x per move
(no signal re-render); runs the pure strip math on release and fires on_commit.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 5.2 — Mount the 3-pane strip + wire StripDrag + peek-never-marks-read

**Files:**
- Modify: `src/ui/shell/sk_orbit/mod.rs` (render the strip wrapper around the current `ChannelPane`, plus prev/next peek panes)
- Modify: `style/_sk_orbit.scss` (strip layout, `content-visibility` on peeked panes)

The strip mounts 3 panes only for `Pane::Channel`; other panes (Friends/Lorebook/Emoji/Members) render single (no strip). The current channel's `ChannelPane` is the MIDDLE pane (unchanged — it owns the composer + message list). The prev/next panes are lightweight neighbor previews that lazily fetch their first page on peek but DO NOT mark read.

Steps:

- [ ] 5.2.1 — In `SkOrbitShell`, compute the prev/next channel from `s.sel.channels` + `s.sel.sel_channel`, and the current index/count. Replace the single pane-switch's `Pane::Channel` arm with a strip wrapper. Add the index/count derivations + the commit handler near the top of `SkOrbitShell`:

```rust
    // Strip geometry: the current channel's index in the sidebar order.
    let cur_idx = move || {
        let chans = s.sel.channels.get();
        s.sel
            .sel_channel
            .get()
            .and_then(|c| chans.iter().position(|x| x.id == c.id))
    };
    let chan_count = move || s.sel.channels.get().len();
    let strip_ref = NodeRef::<leptos::html::Div>::new();
    // StoredValues feed the hydrate StripDrag without re-rendering it.
    #[cfg(feature = "hydrate")]
    let idx_sv = StoredValue::new(0usize);
    #[cfg(feature = "hydrate")]
    let count_sv = StoredValue::new(0usize);
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        idx_sv.set_value(cur_idx().unwrap_or(0));
        count_sv.set_value(chan_count());
    });
    // Commit: a Prev/Next swipe opens the neighbor channel (act handles the
    // switch + warp). The destination index comes from the UNIT-TESTED
    // strip::commit_target (Task 1.3) — edge guards (no prev at first / no next
    // at last / Stay) all collapse to None ⇒ no-op. A committed switch makes the
    // destination the ACTIVE channel, so marking it read on open is correct
    // (peek-never-marks-read concerns only non-current neighbors, which are
    // name-only here and never reach open_channel — see the Task 1.3 intro).
    let on_strip_commit = move |commit: super::strip::StripCommit| {
        let chans = s.sel.channels.get_untracked();
        let Some(i) = cur_idx() else { return };
        if let Some(j) = super::strip::commit_target(commit, i, chans.len()) {
            if let Some(ch) = chans.get(j).cloned() {
                act::open_channel(s, ch);
            }
        }
    };
    #[cfg(feature = "hydrate")]
    let strip_drag = super::drag::StripDrag::new(
        idx_sv,
        count_sv,
        Callback::new(on_strip_commit),
        strip_ref,
    );
    #[cfg(not(feature = "hydrate"))]
    let _ = (strip_ref, on_strip_commit);
```

NOTE: if the ssr `StripDrag` is constructed field-by-field (HoloPanel pattern) rather than via `new`, build it inline with `#[cfg(feature="hydrate")]` on each field and drop the ssr `_ =` line accordingly. The `Pane::Channel` arm must bind `strip_drag`'s handlers ungated; on ssr `strip_drag` must still exist as a value. Use whichever construction makes both graphs compile (verify in 5.2.4).

- [ ] 5.2.2 — Render the strip for `Pane::Channel`. The 3-pane strip wraps the existing `ChannelPane` (middle) plus two neighbor preview panes. The strip `<div>` carries the `--strip-x` transform + the StripDrag handlers. Replace the `Pane::Channel => view! { <ChannelPane/> }` arm:

```rust
                Pane::Channel => {
                    #[cfg(feature = "hydrate")]
                    let d = strip_drag.clone();
                    #[cfg(feature = "hydrate")]
                    let (d_down, d_move, d_up) = (d.clone(), d.clone(), d);
                    view! {
                        <div class="sk-orbit-strip" node_ref=strip_ref
                            on:pointerdown=move |ev| {
                                #[cfg(feature = "hydrate")] d_down.down(&ev);
                                #[cfg(not(feature = "hydrate"))] let _ = &ev;
                            }
                            on:pointermove=move |ev| {
                                #[cfg(feature = "hydrate")] d_move.moved(&ev);
                                #[cfg(not(feature = "hydrate"))] let _ = &ev;
                            }
                            on:pointerup=move |ev| {
                                #[cfg(feature = "hydrate")] d_up.up(&ev);
                                #[cfg(not(feature = "hydrate"))] let _ = &ev;
                            }
                            on:pointercancel=move |ev| {
                                #[cfg(feature = "hydrate")] d_up.up(&ev);
                                #[cfg(not(feature = "hydrate"))] let _ = &ev;
                            }>
                            // prev/current/next. The current pane is the real
                            // ChannelPane (owns composer + list). The neighbors
                            // are peek previews (lazy first page, NEVER mark read).
                            <div class="sk-orbit-pane sk-orbit-pane-prev" aria-hidden="true">
                                {move || neighbor_preview(s, cur_idx().and_then(|i| i.checked_sub(1)))}
                            </div>
                            <div class="sk-orbit-pane sk-orbit-pane-cur">
                                <ChannelPane/>
                            </div>
                            <div class="sk-orbit-pane sk-orbit-pane-next" aria-hidden="true">
                                {move || neighbor_preview(s, cur_idx().map(|i| i + 1).filter(|&j| j < chan_count()))}
                            </div>
                        </div>
                    }.into_any()
                }
```

NOTE: `strip_drag` must be cloned per-arm because the `move ||` pane-switch closure runs repeatedly; if `strip_drag` is not `Copy`, capture it via a `StoredValue` or clone before the `match`. Simplest: wrap `strip_drag` in `StoredValue::new(strip_drag)` (hydrate-only) and `.get_value()` inside the arm. Verify the borrow checker is satisfied in 5.2.4.

- [ ] 5.2.3 — Add a `neighbor_preview` helper to `sk_orbit/mod.rs` that renders a lightweight read-only preview of a neighbor channel's name + last messages WITHOUT marking it read. For the first cut, render just the channel name + a "swipe to open" affordance (the lazy first-page fetch is a refinement; the gate requirement is that the strip mounts 3 panes and peek never marks read — a name-only neighbor satisfies "mounted pane" + trivially "never marks read"):

```rust
fn neighbor_preview(s: Shell, idx: Option<usize>) -> impl IntoView {
    let label = idx
        .and_then(|i| s.sel.channels.get().get(i).map(|c| c.name.clone()))
        .unwrap_or_default();
    view! {
        <div class="sk-orbit-peek">
            {(!label.is_empty()).then(|| view! {
                <span class="sk-orbit-peek-name">"# "{label}</span>
            })}
        </div>
    }
}
```

NOTE — peek-never-marks-read is SATISFIED here (READ the path, don't defer it): the invariant holds because neighbor previews are name-only and NEVER touch `act::open_channel`/last-seen — a neighbor is never a mounted `ChannelPane`, never becomes "current", never marks read. A COMMITTED swipe DOES mark the destination read immediately (`open_channel_at` calls `set_last_seen` at `src/ui/shell/act/channel.rs:305` on open), and that is CORRECT — the destination is the active channel now, identical to a sidebar/orbit-map tap. There is no unread-channel-marked-by-a-glimpse harm to gate, so NO ≥300ms `peek_settles` timer is shipped (it would be dead code). The ≥300ms settle becomes relevant ONLY if a fuller neighbor render makes a neighbor briefly "current"; that lazy first-page peek-fetch (read-only, explicitly NOT touching last_seen/mark-read) + the `peek_settles` gate are booked together as the Phase-7 carry 9.4.3-b/-c — not an open correctness hole in Phase 2.

- [ ] 5.2.4 — Style the strip in `style/_sk_orbit.scss`. The strip is a 3×100vw flex row translated by `--strip-x`; peeked panes get `content-visibility:auto`. Transform is the only animated property:

```scss
.app.sk-orbit .sk-orbit-strip {
	display: flex;
	flex: 1;
	min-height: 0;
	width: 300vw;
	transform: translateX(var(--strip-x, -100vw));
	transition: transform 380ms cubic-bezier(0.18, 0.85, 0.25, 1);
	touch-action: pan-y; // we own horizontal; let vertical scroll through
}
.app.sk-orbit .sk-orbit-pane {
	width: 100vw;
	min-width: 0;
	display: flex;
	flex-direction: column;
}
.app.sk-orbit .sk-orbit-pane-prev,
.app.sk-orbit .sk-orbit-pane-next {
	content-visibility: auto;
	contain-intrinsic-size: auto 100vh;
}
.app.sk-orbit .sk-orbit-peek {
	flex: 1;
	display: flex;
	align-items: center;
	justify-content: center;
	color: var(--text-muted);
}
// The 380ms snap is the COMMIT settle; while dragging the JS clears the
// transition by writing --strip-x every move (the transition still applies but
// the rapid writes make it track the finger). Reduced-motion: instant snap.
@media (prefers-reduced-motion: reduce) {
	.app.sk-orbit .sk-orbit-strip {
		transition: none;
	}
}
```

NOTE: the resting `--strip-x: -100vw` centers the middle pane. While the JS writes per-move offsets the 380ms transition would lag; if tracking feels rubbery on-device, add a `.sk-orbit-strip.dragging { transition: none; }` class toggled by the StripDrag (set on `down`, cleared on `up`) — mirror the lightbox `.gesturing` class. Add this refinement if the real-device gate (integration task) shows lag.

- [ ] 5.2.5 — Verify both graphs compile + lint:

```bash
cargo clippy --features ssr -- -D warnings 2>&1 | tail -8 \
 && cargo clippy --features hydrate --target wasm32-unknown-unknown -- -D warnings 2>&1 | tail -8 \
 && cargo test --features ssr --test style_lint 2>&1 | tail -4 \
 && cargo leptos build 2>&1 | tail -6
```

Expected: both clippy clean; style_lint passes; build compiles.

- [ ] 5.2.6 — Visual smoke (headed Playwright, localhost:3000 + dev DB; resize to 360×800 POCO C3 floor): the channel pane sits in the middle of a 3-pane strip; dragging horizontally tracks 1:1; a release past ~32% or a flick switches to the neighbor; vertical drags still scroll the message list; SSE chip stays LIVE across a swipe; the composer draft is preserved. Then commit:

```bash
git add src/ui/shell/sk_orbit/mod.rs style/_sk_orbit.scss
git commit -m "$(cat <<'EOF'
feat(ui): orbit swipe strip + axis-locked channel switch (W5/P2 #5)

Pane::Channel mounts a 3-slot 300vw strip translated by --strip-x: the MIDDLE
slot is the real ChannelPane; prev/next are NAME-ONLY label peeks (full neighbor
render deferred to Phase-7). StripDrag tracks the finger 1:1 with edge rubber-
band, axis-locks H vs vertical scroll, and commits via the pure strip math (≥32%
or >0.45px/ms) → commit_target → act::open_channel. content-visibility on the
peek slots; the live rows get it from the #36 foundation. peek-never-marks-read
holds structurally (name-only neighbors never become current); SSE rides the no-
remount structure (the middle pane swaps its list in place, not remounted).

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

## Task 6 — Floating composer orb: charge ring (#E) + effect blossom on long-hold

The orb floats bottom-right (owns the bottom safe-area inset). Tap expands the composer; long-hold (480ms) blossoms the three effect chips (whisper/shout/spell). The charge ring fills via the `charge.rs` log curve. Effects ride the validated `message.effect` field UNCHANGED. The orb badge shows the armed effect glyph. Guarded-click discipline so the blossom isn't dismissed by the trailing click.

### Task 6.1 — The orb + charge ring (wired to `charge.rs`)

**Files:**
- Modify: `src/ui/shell/sk_orbit/mod.rs` (the orb markup; the charge memo using `charge_fraction`)
- Modify: `style/_sk_orbit.scss` (orb + charge-ring styling, pre-rendered glow + opacity pulse)

The existing `ChannelPane` composer (`channel/mod.rs:1347-1865`) already renders for the current pane (it IS the middle strip pane). The orbit orb is an ADDITIONAL floating affordance that drives the SAME composer state (`s.composer.compose`, `s.composer.effect_mode`, `act::send_message`). For Phase 2 the orb is the floating SEND surface with the charge ring; the textarea stays in the `ChannelPane` composer (expanding it is a refinement).

DOUBLE-SEND-SURFACE — must reconcile (decision: option (a), hide the in-pane send): the `ChannelPane` composer ALREADY renders its own `.send` button (`channel/mod.rs:1850`) with a charge ring driven by a LINEAR `chars/280` Memo (`channel/mod.rs:541`, bound at `:1852` via `--charge`). Mounting that pane as the middle strip slot keeps its in-pane send + linear ring VISIBLE. Adding the orb with the LOG curve (`charge::charge_fraction`) would put TWO send affordances with TWO different ring fills on the same composer text on screen at once — the #33 calibration was meant to REPLACE the prose-hostile linear curve, not run beside it. RESOLUTION: under `.app.sk-orbit`, HIDE the in-pane `.composer .send` button via SCSS (Task 6.1.2) so the floating orb is the SOLE send surface and the LOG-curve ring is the only one reflecting length. Send still works through the orb (`act::send_message(s)` — the identical call the in-pane button makes). CAVEAT to book (9.4.3): the in-pane `.send` also dismisses the `:`-autocomplete popover on send (`ac_token.set(None)`, `channel/mod.rs:1860`); routing send through the orb loses that one side-effect under orbit — a small follow-up (have the orb click also clear the autocomplete token, or hoist the dismiss into `act::send_message`). NOT option (b) (point the existing Memo at `charge_fraction` and keep two buttons): two redundant send buttons is worse UX than one, and orbit's whole composer model is the single floating orb.

Steps:

- [ ] 6.1.1 — Add the charge memo + the orb markup to `SkOrbitShell`. The memo uses the new log curve; the orb is a fixed bottom-right button rendering the SVG charge ring. Add the memo near the top:

```rust
    let charge = Memo::new(move |_| {
        s.composer.compose.with(|c| super::charge::charge_fraction(c))
    });
    let armed_glyph = move || match s.composer.effect_mode.get().as_deref() {
        Some("whisper") => "🤫",
        Some("shout") => "📣",
        Some("spell") => "✨",
        _ => "",
    };
```

orb markup (render as a child of `<section class="content sk-orbit-content">`, after the pane switch, so it floats over the strip; the `--charge`/`--dash` props drive the ring):

```rust
            <div class="sk-orbit-orb-wrap">
                <button class="sk-orbit-orb" type="button"
                    class:charging=move || charge.get() > 0.0
                    class:armed=move || s.composer.effect_mode.get().is_some()
                    style:--charge=move || format!("{:.3}", charge.get())
                    style:--dash=move || format!("{:.1}", super::charge::dash_offset(charge.get()))
                    title="Send"
                    on:click=move |_| act::send_message(s)>
                    <svg class="sk-orbit-ring" viewBox="0 0 52 52" aria-hidden="true">
                        <circle class="sk-orbit-ring-track" cx="26" cy="26" r="24"></circle>
                        <circle class="sk-orbit-ring-arc" cx="26" cy="26" r="24"></circle>
                    </svg>
                    <span class="sk-orbit-orb-glyph">{move || {
                        let g = armed_glyph();
                        if g.is_empty() { "➤" } else { g }
                    }}</span>
                </button>
            </div>
```

- [ ] 6.1.2 — Style the orb + ring in `style/_sk_orbit.scss`. The orb owns the BOTTOM safe-area inset (the composer drops its own — see the integration safe-area task). The ring arc uses `stroke-dashoffset: var(--dash)` (the `charge.rs` value). The glow is a pre-rendered static `box-shadow` on a `::before` whose opacity pulses (the sanctioned motion-doctrine pattern — never keyframe box-shadow):

```scss
// The floating orb is the SOLE send surface under orbit: hide the in-pane
// ChannelPane `.send` button + its linear charge ring so exactly ONE ring (the
// orb's log curve) reflects message length. Send routes through the orb's
// act::send_message (the same call the in-pane button makes). The composer
// textarea itself stays (the orb only replaces the send affordance).
.app.sk-orbit .composer .send {
	display: none;
}
.app.sk-orbit .sk-orbit-orb-wrap {
	position: fixed;
	right: calc(0.9rem + env(safe-area-inset-right, 0px));
	bottom: calc(0.9rem + env(safe-area-inset-bottom, 0px)); // OWNS bottom edge
	z-index: 30;
}
.app.sk-orbit .sk-orbit-orb {
	position: relative;
	width: 52px;
	height: 52px;
	border-radius: 50%;
	@include glass-etched;
	color: var(--text);
	display: grid;
	place-items: center;
}
.app.fx-max .sk-orbit-orb {
	@include glass-live;
}
.app.sk-orbit .sk-orbit-ring {
	position: absolute;
	inset: 0;
	transform: rotate(-90deg); // start the arc at 12 o'clock
}
.app.sk-orbit .sk-orbit-ring-track {
	fill: none;
	stroke: var(--glass-line);
	stroke-width: 3;
}
.app.sk-orbit .sk-orbit-ring-arc {
	fill: none;
	stroke: var(--accent);
	stroke-width: 3;
	stroke-linecap: round;
	stroke-dasharray: 151; // CIRC
	stroke-dashoffset: var(--dash, 151);
	transition: stroke-dashoffset 120ms ease;
}
.app.sk-orbit .sk-orbit-orb.armed .sk-orbit-ring-arc {
	stroke: var(--pinged-glow);
}
// Pre-rendered glow: static max box-shadow on ::before, opacity pulsed (the
// sanctioned pattern — keyframes never animate box-shadow). Only while armed.
.app.sk-orbit .sk-orbit-orb::before {
	content: "";
	position: absolute;
	inset: -2px;
	border-radius: 50%;
	box-shadow: 0 0 16px var(--glow-accent);
	opacity: 0;
	pointer-events: none;
}
.app.sk-orbit .sk-orbit-orb.charging::before {
	opacity: 0.7;
}
@media (prefers-reduced-motion: reduce) {
	.app.sk-orbit .sk-orbit-ring-arc {
		transition: none;
	}
}
```

- [ ] 6.1.3 — Verify build + lint:

```bash
cargo leptos build 2>&1 | tail -6 \
 && cargo test --features ssr --test style_lint 2>&1 | tail -4 \
 && cargo clippy --features hydrate --target wasm32-unknown-unknown -- -D warnings 2>&1 | tail -6
```

Expected: build compiles; style_lint passes (no box-shadow in any keyframe); hydrate clippy clean.

- [ ] 6.1.4 — Visual smoke (headed Playwright, localhost:3000 + dev DB): type a one-liner → ring shows a sliver; type a paragraph → ~60%; confirm EXACTLY ONE ring reflects length (the in-pane `.composer .send` button + its linear ring are hidden under orbit — only the orb's log-curve ring shows); the orb sends on tap. Then commit:

```bash
git add src/ui/shell/sk_orbit/mod.rs style/_sk_orbit.scss
git commit -m "$(cat <<'EOF'
feat(ui): orbit composer orb + charge ring (W5/P2 #E/#33)

Fixed bottom-right glass orb (owns the bottom safe-area inset) drives the shared
composer send. SVG charge ring fills via the log curve (charge::charge_fraction
/ dash_offset, CIRC=151) — a one-liner is a sliver, a paragraph ~60%. Pre-
rendered glow on ::before, opacity-pulsed (motion-doctrine safe). Armed effect
tints the arc + glyph. The in-pane ChannelPane `.send` button + its linear
chars/280 ring are HIDDEN under .app.sk-orbit so the orb is the SOLE send
surface and exactly one (log-curve) ring reflects length (#33 replaces the
linear curve, not duplicates it).

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 6.2 — Effect blossom on long-hold (whisper/shout/spell)

**Files:**
- Create: `src/ui/shell/sk_orbit/blossom.rs` (the long-hold detector struct, hydrate-real + ssr stub, mirroring `radial::LongPress`)
- Modify: `src/ui/shell/sk_orbit/mod.rs` (`pub mod blossom;`; the blossom chips markup; wire long-hold to a `blossom_open` signal)
- Modify: `style/_sk_orbit.scss` (blossom chip styling + transform/opacity keyframe)

Steps:

- [ ] 6.2.1 — Create `src/ui/shell/sk_orbit/blossom.rs` — the long-hold detector. It mirrors `radial::LongPress` exactly (generation-counter, 480ms, move-slop disarm, hydrate-real + ssr stub), but on fire it sets a signal + fires a `Vh::Tick` haptic instead of opening a radial. The tap-vs-hold slop is load-bearing (#47 mis-send fear):

```rust
//! W5/P2 effect-blossom long-hold detector. Mirrors `radial::LongPress`: an
//! always-on struct with hydrate-only fields, a generation-counter timer
//! (480ms), move-slop disarm (so a jittery thumb tapping Send never blossoms),
//! and an ssr no-op stub. On fire it opens the effect blossom + fires a Tick
//! haptic; the trailing click is guarded so it doesn't dismiss the blossom.

use leptos::prelude::*;

/// Long-hold move slop (px): a press that drifts past this disarms (it's a drag,
/// not a hold) — the same discipline as the radial, load-bearing for #47.
pub const HOLD_SLOP_PX: f64 = 10.0;
/// Long-hold duration (ms) before the blossom opens. INTENTIONALLY distinct from
/// (and slightly longer than) the radial's `LONG_PRESS_MS = 450` (`radial.rs:50`)
/// — these are two separate detectors, and the orb is a SEND affordance, so a
/// longer hold reduces an accidental blossom on a deliberate send tap (the #47
/// mis-send concern). NOT a typo to "fix" to 450; the difference is the design.
pub const HOLD_MS: u32 = 480;

#[derive(Clone, Copy)]
pub struct BlossomHold {
    #[cfg(feature = "hydrate")]
    gen: StoredValue<u64>,
    #[cfg(feature = "hydrate")]
    origin: StoredValue<Option<(f64, f64)>>,
    /// Set true when the hold fires; the click handler reads + clears it to
    /// guard the trailing click (so the hold doesn't also send).
    #[cfg(feature = "hydrate")]
    fired: StoredValue<bool>,
}

impl BlossomHold {
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "hydrate")]
            gen: StoredValue::new(0),
            #[cfg(feature = "hydrate")]
            origin: StoredValue::new(None),
            #[cfg(feature = "hydrate")]
            fired: StoredValue::new(false),
        }
    }
}

#[cfg(feature = "hydrate")]
impl BlossomHold {
    /// pointerdown: arm the hold; on expiry (if not disarmed) open the blossom.
    pub fn down(&self, ev: &leptos::ev::PointerEvent, open: RwSignal<bool>, orb: leptos::web_sys::Element) {
        use leptos::task::spawn_local;
        self.origin
            .set_value(Some((ev.client_x() as f64, ev.client_y() as f64)));
        self.fired.set_value(false);
        let g = self.gen.get_value() + 1;
        self.gen.set_value(g);
        let gen = self.gen;
        let fired = self.fired;
        spawn_local(async move {
            gloo_timers::future::TimeoutFuture::new(HOLD_MS).await;
            if gen.try_get_value() == Some(g) {
                fired.set_value(true);
                open.set(true);
                crate::ui::shell::act::haptics::vh(&orb, crate::ui::shell::act::haptics::Vh::Tick);
            }
        });
    }

    /// pointermove: disarm if the press drifts past the slop (a drag, not a hold).
    pub fn moved(&self, ev: &leptos::ev::PointerEvent) {
        if let Some((ox, oy)) = self.origin.get_value() {
            let dx = ev.client_x() as f64 - ox;
            let dy = ev.client_y() as f64 - oy;
            if (dx * dx + dy * dy).sqrt() > HOLD_SLOP_PX {
                self.cancel();
            }
        }
    }

    /// pointerup/cancel: disarm the pending timer (a completed/aborted press).
    pub fn cancel(&self) {
        self.gen.set_value(self.gen.get_value() + 1);
        self.origin.set_value(None);
    }

    /// True if the hold fired (consume to guard the trailing click).
    pub fn take_fired(&self) -> bool {
        let f = self.fired.get_value();
        self.fired.set_value(false);
        f
    }
}

/// ssr stubs: never run; the orb bindings are always-on and must typecheck.
#[cfg(not(feature = "hydrate"))]
impl BlossomHold {
    pub fn moved(&self, _ev: &leptos::ev::PointerEvent) {}
    pub fn cancel(&self) {}
    pub fn take_fired(&self) -> bool {
        false
    }
}
```

NOTE: confirm `gloo_timers::future::TimeoutFuture` is the in-tree timer (it is — `act/channel.rs:137` uses it). `StoredValue::try_get_value` is the safe accessor (mirror `radial.rs`'s `try_*` discipline; if the exact method name differs, use `try_with_value`/`get_value` as the radial does — verify against `radial.rs` at execution). The `down` method takes the orb `Element` for the haptic; the ssr stub omits `down` (the orb only binds it on hydrate via a `#[cfg(feature="hydrate")]` closure).

- [ ] 6.2.2 — Register the module + add a `blossom_open` signal + a `BlossomHold` to `SkOrbitShell`, and wire the orb's pointer handlers + guarded click. In `sk_orbit/mod.rs`:

```rust
pub mod blossom;
```
in `SkOrbitShell`:
```rust
    let blossom_open = RwSignal::new(false);
    let hold = blossom::BlossomHold::new();
    let orb_ref = NodeRef::<leptos::html::Button>::new();
```
update the orb `<button>` to add `node_ref=orb_ref` + the pointer handlers + guard the click:
```rust
                <button class="sk-orbit-orb" type="button"
                    node_ref=orb_ref
                    class:charging=move || charge.get() > 0.0
                    class:armed=move || s.composer.effect_mode.get().is_some()
                    style:--charge=move || format!("{:.3}", charge.get())
                    style:--dash=move || format!("{:.1}", super::charge::dash_offset(charge.get()))
                    title="Send (hold for effects)"
                    on:pointerdown=move |ev| {
                        #[cfg(feature = "hydrate")]
                        if let Some(el) = orb_ref.get_untracked() {
                            use leptos::wasm_bindgen::JsCast as _;
                            let el: leptos::web_sys::Element = (*el).clone().unchecked_into();
                            hold.down(&ev, blossom_open, el);
                        }
                        #[cfg(not(feature = "hydrate"))] let _ = &ev;
                    }
                    on:pointermove=move |ev| hold.moved(&ev)
                    on:pointerup=move |ev| { hold.cancel(); let _ = &ev; }
                    on:pointercancel=move |ev| { hold.cancel(); let _ = &ev; }
                    on:click=move |_| {
                        // Guard: if the hold fired, swallow the trailing click
                        // (it opened the blossom, it must not also send).
                        if hold.take_fired() { return; }
                        act::send_message(s);
                    }>
```

- [ ] 6.2.3 — Render the blossom chips under `<Show when=move || blossom_open.get()>`, three glass chips arcing above the orb. Each chip ARMS its effect (`s.composer.effect_mode`), the server contract unchanged. Add after the orb button (inside `.sk-orbit-orb-wrap`):

```rust
                {move || blossom_open.get().then(|| {
                    let chips = [("whisper", "🤫"), ("shout", "📣"), ("spell", "✨")];
                    view! {
                        <div class="sk-orbit-blossom" role="menu" aria-label="Message effect">
                            {chips.into_iter().enumerate().map(|(i, (name, glyph))| {
                                let name_owned = name.to_string();
                                view! {
                                    <button class="sk-orbit-chip" role="menuitem"
                                        tabindex=if i == 0 { "0" } else { "-1" }
                                        style:--chip-i=i.to_string()
                                        title=name
                                        on:click=move |_| {
                                            // Toggle-arm this effect, then close.
                                            s.composer.effect_mode.update(|e| {
                                                *e = if e.as_deref() == Some(name_owned.as_str()) {
                                                    None
                                                } else {
                                                    Some(name_owned.clone())
                                                };
                                            });
                                            blossom_open.set(false);
                                        }>
                                        {glyph}
                                    </button>
                                }
                            }).collect_view()}
                        </div>
                    }
                })}
```

- [ ] 6.2.4 — Style the blossom in `style/_sk_orbit.scss`. The chips fan up from the orb via transform (the per-chip `--chip-i` offsets them); the open is an opacity/transform keyframe. `role=menuitem` + roving tabindex from day one (the radial a11y debt NOT copied):

```scss
.app.sk-orbit .sk-orbit-blossom {
	position: absolute;
	right: 0;
	bottom: 60px;
	display: flex;
	flex-direction: column-reverse;
	gap: 0.5rem;
}
.app.sk-orbit .sk-orbit-chip {
	width: 44px;
	height: 44px;
	border-radius: 50%;
	@include glass-etched;
	color: var(--text);
	display: grid;
	place-items: center;
	animation: sk-orbit-chip-in 180ms ease-out backwards;
	animation-delay: calc(var(--chip-i, 0) * 40ms);
}
@keyframes sk-orbit-chip-in {
	from { opacity: 0; transform: translateY(8px) scale(0.85); }
	to { opacity: 1; transform: translateY(0) scale(1); }
}
@media (prefers-reduced-motion: reduce) {
	.app.sk-orbit .sk-orbit-chip {
		animation: none;
	}
}
```

- [ ] 6.2.5 — Verify both graphs compile + lint:

```bash
cargo clippy --features ssr -- -D warnings 2>&1 | tail -8 \
 && cargo clippy --features hydrate --target wasm32-unknown-unknown -- -D warnings 2>&1 | tail -8 \
 && cargo test --features ssr --test style_lint 2>&1 | tail -4 \
 && cargo leptos build 2>&1 | tail -6
```

Expected: both clippy clean; style_lint passes (`sk-orbit-chip-in` is transform/opacity only); build compiles.

- [ ] 6.2.6 — Visual smoke (headed Playwright, localhost:3000 + dev DB): a quick TAP on the orb sends (no blossom); a 480ms HOLD opens the three chips + (if haptics enabled) a tick; tapping a chip arms that effect (ring tints, glyph swaps) and the trailing click does NOT send; the next send carries the effect. Then commit:

```bash
git add src/ui/shell/sk_orbit/blossom.rs src/ui/shell/sk_orbit/mod.rs style/_sk_orbit.scss
git commit -m "$(cat <<'EOF'
feat(ui): orbit effect blossom on orb long-hold (W5/P2 #E/#17/#47)

BlossomHold mirrors radial::LongPress (480ms generation-counter, 10px move-slop
disarm so a jittery Send tap never blossoms, ssr stub). On fire: opens three
glass chips (whisper/shout/spell, role=menuitem + roving tabindex from day one)
+ a Tick haptic; the trailing click is guarded so the hold never also sends.
Chips arm the existing s.composer.effect_mode — message.effect wire unchanged.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

## Task 7 — Right-edge HoloPanel slide-over (personas + station)

A's first HoloPanel gesture-family consumer (spec §12(0)). Summoned by an EXPLICIT affordance (a button on the pill / orb area), NOT a chat-layer right-edge swipe (DEMOTED — fights iOS back-swipe). Internal swipe-right-to-close + Esc-to-close must reach Modal-parity (§13). Sections: "Your persona in #channel" (persona cards, wear-on-tap via `act::wear_persona`) + "Station" (Ghost Quill / Effects / Aurora-max / Push toggles).

CRITICAL ENGINE GAP (must fix in 7.0 before consuming HoloPanel): orbit is HoloPanel's FIRST consumer (verified: `grep -rn "HoloPanel" src/` finds only `holopanel.rs` itself). As shipped (`holopanel.rs`, read it), HoloPanel is EXCLUSIVELY drag-summoned and CANNOT be opened by a button or closed by a callback:
- `progress` starts at `0.0` and is raised ONLY by a pointer drag (`PanelDrag::moved`). `_holopanel.scss:25` renders `.holopanel--right` at `translateX(calc((1 - var(--p)) * 100%))` — i.e. fully OFF-SCREEN at `--p=0`. There is NO `open`/`initial_progress` prop, so mounting the panel under `<Show when=station_open>` leaves it at `--p=0`, invisible.
- `on_commit` fires only AFTER a committed drag (`PanelDrag::up`), so `on_commit=move |_| station_open.set(true)` can never set it false — neither swipe-to-close nor Esc reaches the parent. Esc only sets `progress=0` internally; the `<Show>` stays open (panel re-renders off-screen but mounted; re-open is a no-op).
- HoloPanel's own doc says focus-restore-to-trigger is "the parent's job" — currently unwired.

So this task FIRST extends the HoloPanel ENGINE (7.0) with an `open` prop (mount-time animate-in to the open detent) and an `on_close` callback (fired on Esc AND on the snap-to-closed drag path), then consumes it with parent-owned focus-restore-to-summon-button (7.1-7.5). Option (a) from review; option (b) — "← button only, no Esc/swipe close" — was rejected because it fails the §13 per-skeleton "every dialog-like overlay reaches Modal-parity (Esc, restore-to-trigger)" requirement.

**Files:**
- Modify: `src/ui/shell/holopanel.rs` (ADD `open: bool` + `on_close: Option<Callback<()>>` props; mount-time animate to the open detent when `open`; fire `on_close` on Esc and on the snap-to-closed drag release; this is a Foundation-engine change — keep its existing tests green and ADD a unit test for the new pure decision)
- Modify: `src/ui/shell/sk_orbit/mod.rs` (a `station_open` signal; a `station_btn_ref` for focus-restore; the summon button; the `<HoloPanel edge=Edge::Right open=true on_close=…>` mount)
- Modify: `style/_sk_orbit.scss` (slide-over content styling — NOT the panel transform, which HoloPanel owns)

Steps:

- [ ] 7.0 — Extend the HoloPanel engine so it can be button-opened and callback-closed (its first real consumer needs both; the change is small and keeps every existing pure fn + test intact). Make these EXACT edits to `src/ui/shell/holopanel.rs`:

  (a) Add two props to the `#[component] pub fn HoloPanel(...)` signature (after `desktop_chrome`, before `children`):

```rust
    /// Start OPEN: on mount, animate to the open (last) detent instead of the
    /// closed `--p=0` resting state. For parent-`<Show>`-mounted panels summoned
    /// by an explicit affordance (the engine is otherwise drag-summoned only).
    #[prop(optional)]
    open: bool,
    /// Fired when the panel asks the parent to dismiss it — on Esc AND on a
    /// drag/flick that snaps back below the open detent (swipe-to-close). The
    /// PARENT owns un-mounting (e.g. flips its `<Show>` signal) and focus
    /// restore-to-trigger; the engine only signals intent. `None` ⇒ legacy
    /// drag-summoned behaviour (Esc just snaps to `--p=0`, no parent notify).
    #[prop(optional, into)]
    on_close: Option<Callback<()>>,
```

  (b) Carry `on_close` into `PanelDrag` so its `up`/`keydown` can fire it. Add a hydrate-only field to the struct (after `start`):

```rust
    /// Parent dismiss callback (Esc + snap-to-closed). `None` = legacy.
    #[cfg(feature = "hydrate")]
    on_close: Option<Callback<()>>,
```
and set it in the `PanelDrag { … }` construction in the component body by adding a `#[cfg(feature = "hydrate")] on_close,` line (Leptos `Callback`/`Option<Callback>` are `Copy`, so moving it into the hydrate struct and naming it in the cfg-disjoint ssr `let _` tuple — see (e) — do not conflict).

  (c) In `PanelDrag::up` (hydrate impl), the `else` branch is the snap-to-closed path — fire `on_close` there so a swipe/flick that doesn't re-commit-open dismisses the panel:

```rust
        if commits_open(p, velocity) {
            let target = self.detents.with_value(|d| nearest_detent(d, p));
            self.progress.set(target.at);
            self.on_commit.run(target.key);
        } else {
            self.progress.set(0.0);
            // Snap-to-closed: ask the parent to dismiss (it owns un-mount +
            // focus restore). Legacy drag-summoned panels pass no on_close and
            // keep the old "just snap to 0" behaviour.
            if let Some(cb) = self.on_close {
                cb.run(());
            }
        }
```

  (d) In `PanelDrag::keydown`, the `"Escape"` arm must ALSO fire `on_close` (today it only snaps `progress=0`, never telling the parent — so the `<Show>` stays open):

```rust
            "Escape" => {
                ev.prevent_default();
                self.start.set(None);
                self.progress.set(0.0);
                if let Some(cb) = self.on_close {
                    cb.run(());
                }
            }
```

  (e) Mount-time open: FOLD the open-animate into the EXISTING `panel_ref.on_load` callback (do NOT register a second `on_load` on the same `NodeRef` — keep it to one). When `open`, after focusing, raise `progress` to the open detent so the SCSS `transition` on `--p` slides the panel in (the engine is otherwise drag-summoned). "Open detent" = the last detent's `at` (HoloPanel detents are sorted ascending; the fully-open one is last), falling back to `1.0`. First compute `detents_open_at` BEFORE `detents` is moved into `StoredValue::new(detents)`:

```rust
    // The fully-open detent (last, since detents are ascending) — the mount-time
    // open target. Computed before `detents` moves into the gesture state.
    let detents_open_at = detents.last().map(|d| d.at).unwrap_or(1.0);
```
then REPLACE the existing on_load block:
```rust
    #[cfg(feature = "hydrate")]
    panel_ref.on_load(|el| {
        let _ = el.focus();
    });
```
with the open-aware version (a single `on_load`):
```rust
    #[cfg(feature = "hydrate")]
    panel_ref.on_load(move |el| {
        let _ = el.focus();
        // Button-summoned open: the parent mounts us under <Show> with open=true;
        // raise progress to the open detent so the SCSS --p transition slides us
        // in. Leptos applies the initial --p=0, then this set, and the CSS
        // transition interpolates (no rAF defer needed).
        if open {
            progress.set(detents_open_at);
        }
    });
```
NOTE: `open` and `on_close` must be consumed on ssr so they don't read as unused. Extend the existing `#[cfg(not(feature = "hydrate"))] let _ = (detents, on_commit);` line to `let _ = (detents, on_commit, open, on_close, detents_open_at);` (on hydrate: `on_close` is moved into the struct, `open` + `detents_open_at` are captured by the `on_load` closure, so all are used). `detents_open_at` is computed on BOTH graphs (it borrows `detents` before the hydrate-only move; the ssr `let _` then consumes `detents`).

  (f) Add a unit test next to the existing ones proving the mount-time open target selection (pure — the `detents.last().at` choice), so the new decision is covered without a DOM:

```rust
    #[test]
    fn open_target_is_the_last_ascending_detent() {
        let detents = [Detent { at: 0.5, key: "d1" }, Detent { at: 1.0, key: "d2" }];
        // Mount-time `open` raises progress to the fully-open (last) detent.
        assert_eq!(detents.last().map(|d| d.at), Some(1.0));
        // Single-detent panel (orbit's case) opens to that one detent.
        let single = [Detent { at: 1.0, key: "open" }];
        assert_eq!(single.last().map(|d| d.at), Some(1.0));
    }
```

  (g) Verify the engine still compiles on BOTH graphs and ALL holopanel tests pass (the existing four + the new one; no existing test changes — the new props are optional, so the only existing caller, none, and the tests, are unaffected):

```bash
cargo test --features ssr holopanel 2>&1 | tail -8 \
 && cargo clippy --features ssr -- -D warnings 2>&1 | tail -5 \
 && cargo clippy --features hydrate --target wasm32-unknown-unknown -- -D warnings 2>&1 | tail -5
```

Expected: 6 holopanel unit tests pass (the 5 existing — `progress_clamps_and_scales`, `commit_past_halfway_or_on_flick`, `tap_slop_separates_tap_from_drag`, `scrim_tracks_progress_capped`, `nearest_detent_selection_picks_closest` — plus the new `open_target_is_the_last_ascending_detent`); both clippy graphs clean.

- [ ] 7.0.1 — Commit the engine extension on its own (it's a Foundation change consumed next):

```bash
git add src/ui/shell/holopanel.rs
git commit -m "$(cat <<'EOF'
feat(ui): HoloPanel open + on_close props — button-summon & callback-close (W5/P2 #49)

The engine was drag-summoned only: progress started at 0 (fully off-screen) with
no way to open by a button, and Esc/snap-to-closed never told the parent to
un-mount (on_commit fires only on a committed OPEN drag). Orbit (its first
consumer) needs an explicit-affordance summon. Adds `open` (mount-time animate to
the open detent) + `on_close` (fired on Esc AND the snap-to-closed drag release);
both optional, so legacy drag-summoned behaviour is unchanged. Parent still owns
un-mount + focus restore-to-trigger.

Tests: holopanel::tests::open_target_is_the_last_ascending_detent
Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

- [ ] 7.1 — Add a `station_open` signal + a `station_btn_ref` (for focus restore-to-trigger, the §13 Modal-parity requirement) + a summon button to `SkOrbitShell`. The button lives near the orb (an explicit affordance — the demoted edge-swipe is NOT used). Add:

```rust
    let station_open = RwSignal::new(false);
    // The summon button — focus restores here when the panel closes (§13).
    let station_btn_ref = NodeRef::<leptos::html::Button>::new();
```
summon button (next to the orb, inside `.sk-orbit-orb-wrap` or as a sibling fixed button):

```rust
            <button class="sk-orbit-station-btn" type="button"
                node_ref=station_btn_ref
                aria-haspopup="dialog"
                aria-expanded=move || station_open.get().to_string()
                title="Personas & station settings"
                on:click=move |_| station_open.set(true)>"☰"</button>
```

- [ ] 7.2 — Mount the HoloPanel under `<Show when=move || station_open.get()>`, edge `Right`, single detent, `open=true` (mount-time slide-in, the 7.0 prop), `on_close` wired to a `close_station` helper that BOTH flips the signal false AND restores focus to the summon button (§13). HoloPanel focuses itself on mount + owns Esc + Tab-trap; the parent owns un-mount (`<Show>`) + focus-restore. Import the engine and add the mount as a child of `SkOrbitShell`'s `view!` — render it OUTSIDE the strip so a pane transform never captures it (HoloPanel is `position:fixed`, so DOM position is otherwise free). Add the close helper near `station_open`:

```rust
    // Close the slide-over AND restore focus to the summon button (§13 Modal-
    // parity restore-to-trigger). Used by on_close (Esc + swipe-to-close) and
    // the explicit ← button.
    let close_station = move || {
        station_open.set(false);
        #[cfg(feature = "hydrate")]
        if let Some(btn) = station_btn_ref.get_untracked() {
            let _ = btn.focus();
        }
    };
```
import:
```rust
use crate::ui::shell::holopanel::{Detent, Edge, HoloPanel};
```
mount (after the strip/orb, still in `SkOrbitShell`):

```rust
            {move || station_open.get().then(|| view! {
                <HoloPanel
                    edge=Edge::Right
                    open=true
                    detents=vec![Detent { at: 1.0, key: "open" }]
                    // Single-detent: a committed OPEN drag just re-asserts open
                    // (no-op). Dismissal flows through on_close (Esc + swipe-to-
                    // close → snap-to-closed), which restores focus to the button.
                    on_commit=move |_key: &'static str| {}
                    on_close=move |_| close_station()
                >
                    <div class="sk-orbit-station">
                        <button class="sk-orbit-station-close" title="Close" aria-label="Close"
                            on:click=move |_| close_station()>"←"</button>
                        <h2>{move || {
                            let cn = s.sel.sel_channel.get().map(|c| c.name).unwrap_or_default();
                            format!("Your persona in #{cn}")
                        }}</h2>
                        <div class="sk-orbit-persona-grid">
                            {move || {
                                let active = s.social.active_persona.get();
                                s.social.personas.get().into_iter().map(|p| {
                                    let pid = p.id.clone();
                                    let is_active = active.as_deref() == Some(p.id.as_str());
                                    view! {
                                        <button class="sk-orbit-persona-card"
                                            class:active=is_active
                                            title=p.name.clone()
                                            on:click=move |_| act::wear_persona(s, pid.clone())>
                                            {p.name.clone()}
                                        </button>
                                    }
                                }).collect_view()
                            }}
                        </div>
                        <h2>"Station"</h2>
                        <label class="sk-orbit-toggle">
                            <input type="checkbox"
                                prop:checked=move || s.prefs.ghost_quill.get()
                                on:change=move |ev| {
                                    let on = event_target_checked(&ev);
                                    s.prefs.ghost_quill.set(on);
                                    act::set_ghost_quill(on);
                                }/>
                            "Ghost Quill (live co-writer)"
                        </label>
                        <label class="sk-orbit-toggle">
                            <input type="checkbox"
                                prop:checked=move || s.prefs.eyecandy.get()
                                on:change=move |ev| {
                                    let on = event_target_checked(&ev);
                                    s.prefs.eyecandy.set(on);
                                    act::set_eyecandy(on);
                                }/>
                            "Aurora-max (eye-candy tier)"
                        </label>
                    </div>
                </HoloPanel>
            })}
```

NOTE: confirm the persona DTO field names (`p.id`, `p.name`) + `s.social.personas` / `s.social.active_persona` types against `state.rs:251-256` + the persona protocol DTO at execution. Confirm `act::wear_persona`, `act::set_ghost_quill`, `act::set_eyecandy` are re-exported (gather:primitives + gather:shell confirm `wear_persona` in the persona re-export and the prefs setters exist). Confirm `event_target_checked` is the in-tree helper (it's a Leptos export used elsewhere in the shell — verify, else use `ev.target()` + `dyn_into::<HtmlInputElement>().checked()`). The Push toggle + Effects toggle can be added the same way once their pref signals/actions are confirmed; ship Ghost Quill + Aurora-max first (both confirmed), add the others if their setters exist.

CLOSE/FOCUS NOTE (the §13 gate item): every dismissal path now routes through `close_station` (Esc + swipe-to-close via the 7.0 `on_close`; the explicit ← button directly) so the `<Show>` un-mounts AND focus returns to the summon `☰` button. HoloPanel's Esc/Tab-trap fire while the panel root is focused — its `panel_ref.on_load` focuses the root on mount, so the mounted panel DOES receive keystrokes (verify in the 7.5 smoke: Esc with focus inside the panel closes it AND lands focus back on ☰). The single OPEN detent means a committed open-drag is a harmless no-op (`on_commit` = `{}`); only `on_close` can dismiss, which is exactly the swipe-toward-closed (drag right for `Edge::Right`) snap-back path 7.0(c) wired.

- [ ] 7.3 — Style the slide-over CONTENT in `style/_sk_orbit.scss` (the panel transform + scrim + four-edge insets are HoloPanel's, `_holopanel.scss` — do NOT re-own them). Only the inner layout + the summon button:

```scss
.app.sk-orbit .sk-orbit-station-btn {
	position: fixed;
	right: calc(0.9rem + env(safe-area-inset-right, 0px));
	bottom: calc(4.2rem + env(safe-area-inset-bottom, 0px)); // above the orb
	z-index: 30;
	width: 44px;
	height: 44px;
	border-radius: 50%;
	@include glass-etched;
	color: var(--text);
}
.app.sk-orbit .sk-orbit-station {
	// The HoloPanel root is position:fixed + owns insets; this is the inner
	// scroll surface. Give it the panel's visible width.
	width: min(86vw, 380px);
	height: 100%;
	overflow-y: auto;
	padding: 1rem;
	@include glass-etched;
	color: var(--text);
}
.app.fx-max .sk-orbit-station {
	@include glass-live;
}
.app.sk-orbit .sk-orbit-persona-grid {
	display: flex;
	flex-wrap: wrap;
	gap: 0.5rem;
	margin: 0.5rem 0 1rem;
}
.app.sk-orbit .sk-orbit-persona-card {
	padding: 0.5rem 0.8rem;
	border-radius: 0.6rem;
	border: 2px solid transparent;
	background: var(--card);
	color: var(--text);
}
.app.sk-orbit .sk-orbit-persona-card.active {
	border-color: var(--accent);
}
.app.sk-orbit .sk-orbit-toggle {
	display: flex;
	align-items: center;
	gap: 0.5rem;
	padding: 0.4rem 0;
}
```

- [ ] 7.4 — Verify both graphs compile + lint:

```bash
cargo clippy --features ssr -- -D warnings 2>&1 | tail -8 \
 && cargo clippy --features hydrate --target wasm32-unknown-unknown -- -D warnings 2>&1 | tail -8 \
 && cargo test --features ssr --test style_lint 2>&1 | tail -4 \
 && cargo leptos build 2>&1 | tail -6
```

Expected: all clean.

- [ ] 7.5 — Visual smoke (headed Playwright, localhost:3000 + dev DB): tap ☰ → the panel slides in from the right; persona cards wear on tap; the Ghost Quill / Aurora-max toggles flip the prefs live; swipe-right INSIDE the panel closes it; Esc closes; focus is trapped + restored. Confirm NO chat-layer right-edge swipe summons it (the demoted gesture). Then commit:

```bash
git add src/ui/shell/sk_orbit/mod.rs style/_sk_orbit.scss
git commit -m "$(cat <<'EOF'
feat(ui): orbit right-edge HoloPanel slide-over (personas + station) (W5/P2 #26)

A's first HoloPanel gesture-family consumer (Edge::Right, single detent), using
the new open/on_close props (Task 7.0). Summoned by an explicit ☰ button (open=
true mount-time slide-in) — the chat-layer right-edge swipe is DEMOTED (fights
iOS back-swipe). Dismissal (Esc + swipe-to-close → on_close, and the ← button)
routes through close_station, which un-mounts the <Show> AND restores focus to
the ☰ button (§13 Modal-parity). Sections: persona cards (wear via
act::wear_persona) + station toggles (Ghost Quill, Aurora-max). HoloPanel owns
the transform/scrim/insets + Esc/Tab-trap; this adds the inner content + the
parent-owned focus-restore.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

## Task 8 — Radial menu placement, swipe-to-reply, scene-light (#I/#14/#B)

The radial long-press (#I) is already implemented in `ChannelPane` (`radial.rs`, bound on the `.messages` `<ul>`) and the current channel pane IS the middle strip pane — so the radial ALREADY works in orbit with zero new code (it derives affordances from `message_actions`, never re-branched). This task VERIFIES that, tunes the strip-vs-row arbitration, and adds scene-light (#B, ÖG).

### Task 8.1 — Verify the radial works in orbit (no re-branch) + arbitrate strip vs row-swipe

**Files:**
- Modify: `src/ui/shell/sk_orbit/drag.rs` (the StripDrag `moved` must yield to a row-swipe when `row_swipe_wins` — so a short right-drag STARTED on a message row does NOT switch channels)
- (the radial itself is untouched — it's inherited via `ChannelPane`)

Steps:

- [ ] 8.1.1 — The radial is inherited (the `ChannelPane` `.messages` `<ul>` binds `radial::LongPress` already). There is no code to add for the radial; CONFIRM it by reading `channel/mod.rs:805-823` (the delegated handlers) and `channel/radial.rs:161,405` (the two `message_actions` call sites). Record in the commit that the radial + its 450ms threshold + `message_actions` predicate flow into orbit unchanged because orbit mounts the same `ChannelPane`. No edit in this step.

- [ ] 8.1.2 — Arbitrate the strip against swipe-to-reply. The StripDrag must NOT treat a short right-drag that STARTED on a message row as a channel switch (that gesture belongs to swipe-to-reply). Add a `started_on_row` flag to `StripDrag` (set in `down` by hit-testing the event target), and in `moved`/`up` bail out of the horizontal strip when `strip::row_swipe_wins(started_on_row, dx)`. In `src/ui/shell/sk_orbit/drag.rs`, add the field + hit-test:

```rust
    #[cfg(feature = "hydrate")]
    started_on_row: RwSignal<bool>,
```
in `new` add `started_on_row: RwSignal::new(false),`. In `down`, after capturing the pointer, hit-test:

```rust
        // Did this press start on a message row? If so a small-radius rightward
        // drag is a swipe-to-reply, not a channel switch (the #14/#5 arbitration).
        let on_row = {
            use leptos::wasm_bindgen::JsCast as _;
            ev.target()
                .and_then(|t| t.dyn_into::<leptos::web_sys::Element>().ok())
                .and_then(|e| e.closest("li[id^='msg-']").ok().flatten())
                .is_some()
        };
        self.started_on_row.set(on_row);
```
in `moved`, after computing `dx`/`dy` and BEFORE writing the strip offset, bail when the row-swipe owns the gesture:

```rust
        if super::strip::row_swipe_wins(self.started_on_row.get_untracked(), dx) {
            // Let the row's own swipe-to-reply handle it; don't move the strip.
            return;
        }
```

AND, the FIRST time the axis locks Horizontal (StripDrag is now claiming the gesture), DISARM the inherited radial so a long-press timer armed on `down` (the radial arms on EVERY pointerdown over a row) can't fire mid-swipe. The radial does NOT call `set_pointer_capture`, but StripDrag DOES (in `down`) — once StripDrag captures, subsequent `pointermove`/`pointerup` retarget to the strip div, so the radial's own slop-disarm/`cancel` (bound on the `.messages <ul>`) may never receive them and its 450ms timer would otherwise survive a committed swipe. In `moved`, in the branch where `axis_lock` first returns `Some(Axis::Horizontal)` (i.e. right where the existing code does `self.axis.set(a)` for a horizontal lock, or just before the horizontal `ev.prevent_default()`), call:

```rust
        // Horizontal lock: StripDrag owns the gesture and (via set_pointer_capture
        // in `down`) will steal the pointer stream from the radial's <ul>
        // listeners, so the radial's own pointermove/up disarm can't fire. Bump
        // the radial generation + close any open menu so a press armed on `down`
        // never blossoms mid-swipe (open_channel_at also disarms on commit, but
        // that's too late for the in-flight drag). pub(super) — reachable from
        // this sibling module under `shell`.
        #[cfg(feature = "hydrate")]
        crate::ui::shell::channel::disarm_radial();
```
Do this ONCE per gesture (guard it so it only runs on the transition into the horizontal lock, not every move) — e.g. only in the `or_else` arm that just set `self.axis` to `Some(Axis::Horizontal)`.

NOTE — event-order trace (record this analysis in the 8.1 commit; it is the rigor the arbitration rests on). The radial is bound on the inner `.messages <ul>`; StripDrag on the wrapping `.sk-orbit-strip` div. Both see each pointer event via bubbling (inner first). For each gesture:
  1. **Vertical scroll over a row:** down → radial arms (450ms) + StripDrag records start; moves are vertical → `axis_lock` returns `Vertical` (or stays None under slop) so StripDrag never `prevent_default`s and never captures-to-steal; the radial's own pointermove past its slop disarms it; the browser scrolls. Winner: native scroll; radial disarmed; strip idle. ✓
  2. **Short right-drag on a row (reply):** down → radial arms + `started_on_row=true`; the small-radius rightward move makes `row_swipe_wins` true → StripDrag `moved` bails (no offset, no capture-steal, no horizontal lock) → radial is NOT force-disarmed here, so the row's swipe-to-reply (Phase-7 follow-up) and/or a continued hold still belong to the row. Winner: the row. ✓
  3. **Long horizontal swipe starting on a row:** down → radial arms + `started_on_row=true`; as `dx` grows past `REPLY_POP_PX*1.5`, `row_swipe_wins` turns false AND `axis_lock` returns Horizontal → StripDrag claims it, `prevent_default`s, and the new `disarm_radial()` cancels the armed 450ms timer so no menu pops at the end of the swipe. Winner: the strip; radial cleanly disarmed. ✓
  4. **Long-press hold on a row (no move):** down → radial arms + StripDrag records start; no move past slop → `axis_lock` stays None, StripDrag never locks/`prevent_default`s/captures; at 450ms the radial fires its menu normally. Winner: the radial. ✓
On commit, `open_channel_at` ALSO calls `disarm_radial()` (channel.rs:106) — a belt-and-suspenders second disarm, harmless (idempotent generation bump).

NOTE: orbit does not yet implement the row's swipe-to-reply visual (the ↩ glyph offset). The ARBITRATION (strip yields) is the load-bearing gate item; the row-swipe VISUAL + `act::start_reply` trigger is a follow-up (book it in the integration task). `disarm_radial` is `pub(super) fn disarm_radial()` in `channel/mod.rs:170` — reachable from `sk_orbit` (a descendant of `shell`) as `crate::ui::shell::channel::disarm_radial`. The `closest("li[id^='msg-']")` selector matches the message rows (`channel/mod.rs` ids them `msg-…`; confirm the exact id prefix against the `<li id=…>` in `ChannelPane` at execution — the radial uses the same `closest("li[id^='msg-']")`).

- [ ] 8.1.3 — Verify both graphs compile:

```bash
cargo clippy --features ssr -- -D warnings 2>&1 | tail -6 \
 && cargo clippy --features hydrate --target wasm32-unknown-unknown -- -D warnings 2>&1 | tail -6
```

Expected: both clean.

- [ ] 8.1.4 — Visual smoke (headed Playwright, localhost:3000 + dev DB; touch emulation / real iOS if available): long-press a message row in orbit → the radial blossoms with the correct actions (reply/copy on a roll, +edit/delete on your own user message, nothing on a system message); a short right-drag starting on a row does NOT switch channels; a drag starting in the empty pane area DOES switch. Then commit:

```bash
git add src/ui/shell/sk_orbit/drag.rs
git commit -m "$(cat <<'EOF'
feat(ui): arbitrate swipe-strip vs swipe-to-reply + radial (W5/P2 #14)

StripDrag hit-tests the press target; a small-radius rightward drag that STARTED
on a message row yields to swipe-to-reply (strip::row_swipe_wins) instead of
switching channels. On the first horizontal axis-lock StripDrag also calls
channel::disarm_radial(): it set_pointer_captures in `down` and so steals the
pointer stream from the radial's <ul> listeners (the radial does NOT capture),
which would otherwise leave a 450ms long-press timer armed through a committed
swipe. Event-order traced for all four gestures (scroll / short row-reply /
long horizontal swipe-from-row / hold) in the plan. The radial long-press (#I)
is otherwise inherited unchanged from ChannelPane (orbit mounts the same pane),
so message_actions stays the single predicate — never re-branched per skeleton.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 8.2 — Scene-light (#B, ÖG) — ambient wash from active speakers' tints

**Files:**
- Modify: `style/_sk_orbit.scss` (an ÖG-only ambient layer behind the pane, fed by a tint custom property)
- Modify: `src/ui/shell/sk_orbit/mod.rs` (set a `--scene-tint` from the most-recent message's persona color, fx-max only)

Scene-light is eye-candy-only ([ÖG]) — it must NOT render at Standard tier. It washes the pane chrome with the tint of the currently-active speaker. Keep it cheap: derive a single `--scene-tint` from the latest message's persona color.

Steps:

- [ ] 8.2.1 — In `SkOrbitShell`, derive a `--scene-tint` from the most-recent message's persona color and bind it on the content section. It's only VISIBLE under `.fx-max` (the SCSS gates it), so binding it always is harmless:

```rust
    let scene_tint = move || {
        s.msg.messages.with(|ms| {
            ms.last()
                .and_then(|m| m.persona_color.clone())
                .filter(|c| !c.is_empty())
                .map(|c| format!("var(--tint-{c})"))
                .unwrap_or_default()
        })
    };
```
bind on `<section class="content sk-orbit-content">`:
```rust
            style:--scene-tint=move || scene_tint()
```

NOTE: confirm `MessageEnvelope.persona_color` exists + is the palette name (gather:design references `persona_color`; verify the field name + that it's the 8-name palette at execution against `protocol.rs`). If the field is absent or a hex, adapt the mapping (or skip the ÖG layer — it's [ÖG], non-blocking for the Phase-2 gate).

- [ ] 8.2.2 — Add the ÖG ambient layer in `style/_sk_orbit.scss` (only under `.app.fx-max`). It's a static radial-gradient wash whose color is `--scene-tint`; opacity is the only thing that could animate, and here it's static (no keyframe needed — keeps it lint-trivial and battery-cheap):

```scss
// Scene-light (#B, ÖG): an ambient wash from the active speaker's tint. Only
// at the eye-candy tier (.fx-max) — NEVER at Standard. Static gradient (no
// keyframe); the tint updates reactively via --scene-tint.
.app.fx-max.sk-orbit .sk-orbit-content::before {
	content: "";
	position: absolute;
	inset: 0;
	z-index: 0;
	pointer-events: none;
	background: radial-gradient(
		120% 80% at 50% 100%,
		var(--scene-tint, transparent) 0%,
		transparent 60%
	);
	opacity: 0.12;
}
```

- [ ] 8.2.3 — Verify build + lint + both clippy graphs:

```bash
cargo leptos build 2>&1 | tail -6 \
 && cargo test --features ssr --test style_lint 2>&1 | tail -4 \
 && cargo clippy --features hydrate --target wasm32-unknown-unknown -- -D warnings 2>&1 | tail -6
```

Expected: build compiles; style_lint passes; hydrate clippy clean.

- [ ] 8.2.4 — Visual smoke (headed Playwright, localhost:3000 + dev DB, eye-candy ON): post messages as differently-tinted personas; the pane's ambient wash shifts toward the latest speaker's tint. Toggle eye-candy OFF → the wash disappears (Standard tier). Then commit:

```bash
git add src/ui/shell/sk_orbit/mod.rs style/_sk_orbit.scss
git commit -m "$(cat <<'EOF'
feat(ui): orbit scene-light ambient wash (W5/P2 #B, OG-only)

A static radial-gradient wash on the pane, tinted by --scene-tint (the latest
message's persona palette color). Gated to .fx-max.sk-orbit only — never at
Standard tier. No keyframe (battery-cheap); tint updates reactively.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

## Task 9 — Integration: safe-area audit, reduced-motion audit, binding-kills, full gate + smoke

This is the wave-gate discharge for Skeleton A. No new features — it CLOSES the feasibility tax and proves the binding invariants on the real app.

### Task 9.1 — Safe-area exactly-once-per-edge audit

**Files:**
- Modify: `style/_sk_orbit.scss` (resolve any double-counted inset)

Under `.app.sk-orbit`, EACH edge inset must be applied by EXACTLY ONE rule (the §13 invariant). The orbit owners established across the tasks:
- TOP → the pill (`_sk_orbit.scss` `.sk-orbit-pill` `top: calc(… + env(safe-area-inset-top))`) AND the orbit topbar (`.topbar.sk-orbit-topbar` `padding-top: calc(… + env(…top))`). **These two BOTH pay top — that is a double-count to FIX.**
- BOTTOM → the orb (`.sk-orbit-orb-wrap` `bottom: calc(… + env(…bottom))`) AND the station button (`.sk-orbit-station-btn` `bottom: calc(… + env(…bottom))`). Two fixed elements at the bottom-right may each legitimately offset from the inset (they don't stack the inset on the SAME box — each is an independent fixed element anchored to the viewport edge, so each reads the inset once for ITS own position; this is correct, like `.bottom-tabs` landscape L/R clearance). But the composer (in `ChannelPane`) ALSO pays the bottom inset (`_content.scss:930`) — under orbit the orb owns bottom, so the composer must DROP its bottom inset under `.app.sk-orbit`.
- LEFT/RIGHT → `.app` grid padding (`_layout.scss:12-13`) is NOT used under orbit (orbit is `display:block`, Task 3.1.1). So the orbit fixed elements (orb/station-btn/pill) each own their own right/left inset — correct.

Steps:

- [ ] 9.1.1 — Audit: grep every `env(safe-area-inset-*)` under `.app.sk-orbit` and the inherited owners, and write down the one owner per edge:

```bash
grep -n "safe-area-inset" style/_sk_orbit.scss style/_content.scss style/_holopanel.scss
```

- [ ] 9.1.2 — Fix the TOP double-count: the pill floats ABOVE the topbar, so let the PILL own the top inset and remove it from the orbit topbar (the topbar sits below the pill, no notch contact). In `style/_sk_orbit.scss`, change `.topbar.sk-orbit-topbar` `padding-top` to a plain value (no `env`):

```scss
.app.sk-orbit .topbar.sk-orbit-topbar {
	padding-top: 0.4rem; // pill owns the top safe-area inset, not the topbar
}
```

- [ ] 9.1.3 — Fix the composer bottom double-count: under orbit the orb owns the bottom inset, so the `ChannelPane` composer must drop its own bottom inset. Add to `style/_sk_orbit.scss`:

```scss
// The composer (inside the current ChannelPane) cedes the bottom safe-area
// inset to the floating orb under orbit, so the home-indicator inset is
// counted exactly once (the orb owns it).
.app.sk-orbit .composer {
	padding-bottom: 0.35rem;
}
```

- [ ] 9.1.4 — Verify the build + a device-matrix visual check (POCO C3 360×800, iPhone SE 375×667, iPhone 13 mini 375×812 notch, Nothing Phone 2 412×892) via Playwright `browser_resize`: confirm the pill clears the notch, the orb clears the home indicator, NO element is pushed off-screen, and nothing double-pads. Then commit:

```bash
cargo leptos build 2>&1 | tail -5
git add style/_sk_orbit.scss
git commit -m "$(cat <<'EOF'
fix(ui): orbit safe-area exactly-once per edge (W5/P2 §13)

Pill owns the TOP inset (topbar drops its env() — it sits below the pill); the
floating orb owns the BOTTOM inset (the ChannelPane composer drops its bottom
inset under .app.sk-orbit so the home-indicator inset is counted once). L/R are
owned per-fixed-element (orbit is display:block, no .app grid padding).

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 9.2 — Reduced-motion audit (every orbit motion has a reduced form)

**Files:**
- Modify: `style/_sk_orbit.scss` (ensure every animated element/pseudo whose class lacks `fx-` is listed in a `prefers-reduced-motion` kill, and resting state is correct)

The element-level reduced-motion kill (`_motion.scss:366-386`) only catches `[class*="fx-"]` and does NOT reach `::before`/`::after`. Every orbit motion uses `sk-orbit-*` class names (no `fx-`), so each MUST be explicitly killed.

Steps:

- [ ] 9.2.1 — Audit every `animation`/`transition` in `_sk_orbit.scss`:

```bash
grep -nE "animation:|transition:" style/_sk_orbit.scss
```

Confirm each has a matching `@media (prefers-reduced-motion: reduce)` entry: the strip (`.sk-orbit-strip` transition — done in 5.2.4), the map open (`.sk-orbit-map` animation — done 4.2.2), the chips (`.sk-orbit-chip` animation — done 6.2.4), the charge-ring arc (`.sk-orbit-ring-arc` transition — done 6.1.2). The orb glow `::before` is opacity-only via a class toggle (no animation) — it's fine. Scene-light is static (no animation) — fine.

- [ ] 9.2.2 — Consolidate all reduced-motion kills into ONE `@media (prefers-reduced-motion: reduce)` block at the END of `_sk_orbit.scss` for auditability (remove the scattered per-task blocks if cleaner, or keep them — the requirement is coverage, not placement). Verify the resting state of each element looks correct with motion killed (the strip rests at `--strip-x` with no transition; the map is instantly visible; chips are instantly placed; the ring is instantly at its dashoffset).

- [ ] 9.2.3 — Visual check with reduced-motion forced (Playwright `browser_emulate_media` / DevTools "Emulate prefers-reduced-motion"): swipe a channel (instant snap, no slide-lag), open the map (instant, no scale-in), long-hold the orb (chips appear instantly), type (ring jumps, no ease). All STATE survives; only MOTION dies. Then commit:

```bash
cargo test --features ssr --test style_lint 2>&1 | tail -4
git add style/_sk_orbit.scss
git commit -m "$(cat <<'EOF'
fix(ui): orbit reduced-motion kills for every sk-orbit motion (W5/P2)

Every orbit animation/transition (strip snap, map scale-in, effect chips, charge
ring) has a prefers-reduced-motion kill — the global [class*="fx-"] kill doesn't
reach sk-orbit-* names or ::before/::after. State survives, motion dies; resting
state verified correct.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 9.3 — Binding-kills + invariants verification (manual, recorded)

No code; a recorded audit that the three binding kills + the load-bearing invariants hold in orbit. Read the cited code and confirm:

- [ ] 9.3.1 — **KILL 1 (pinch entry):** grep the orbit module for any pinch/two-pointer handling; confirm the orbit map is entered ONLY by the pill tap (Task 4.1) and there is NO `pts.size===2` / pinch dispatch anywhere in `sk_orbit/`:

```bash
grep -rniE "pinch|two.?pointer|pts\.size|scale.*pinch" src/ui/shell/sk_orbit/ || echo "no pinch entry — KILL 1 honored"
```

- [ ] 9.3.2 — **KILL 2 (deck-plate transit stamp):** confirm orbit's channel switch is NON-blocking — the swipe strip's spatial motion IS the transition; there is NO centered "DECK NN · #channel" plate overlay that delays the swap. The warp dip (`--warp-dir`) is the only switch ornament and it does not gate the swap (it's a CSS transition on the already-swapped pane). Record: orbit adds no blocking arrival plate.

- [ ] 9.3.3 — **KILL 3 (Persona Orrery):** confirm the typing surface in orbit does NOT render draft-length-derived magnitude. Orbit inherits `ChannelPane`'s typing indicator (the constellation #D) unchanged; orbit adds NO star-size/brightness scaling from draft length. The Ghost Quill BOTH-ways gate is untouched (typing-draft TEXT never rides SSE — inherited). Record: orbit adds no draft-magnitude leak.

- [ ] 9.3.4 — **message_actions never re-branched:** confirm orbit routes ALL message affordances through `ChannelPane`'s inherited radial (which uses `message_actions`, `channel/mod.rs:133`). Orbit adds NO per-message action surface that re-derives affordances:

```bash
grep -rn "message_actions\|reply.*copy.*edit.*delete" src/ui/shell/sk_orbit/ || echo "no re-branch — orbit inherits the predicate via ChannelPane"
```

- [ ] 9.3.5 — **Switch-never-remounts / SSE / draft / selection:** confirmed structurally (orbit is a sibling branch on the same `Shell` aggregate, Task 2). The live proof is the smoke in 9.4. Record the four invariants as smoke-verified below.

### Task 9.4 — Full gate + the exact headed-Playwright smoke checklist

**Files:** none (verification only).

Steps:

- [ ] 9.4.1 — Run the FULL gate (all three clippy graphs MUST be `-D warnings`-clean; tests need a live dev SurrealDB on 127.0.0.1:8000 — start it with the `dev-db` skill if needed; NEVER point at prod):

```bash
cargo fmt --all --check \
 && cargo clippy --features ssr -- -D warnings \
 && cargo clippy --features hydrate --target wasm32-unknown-unknown -- -D warnings \
 && cargo clippy --features freya \
 && cargo test --features ssr \
 && cargo leptos build --release \
 && cargo build --bin authlyn-native --features freya
```

Expected: fmt clean; all three clippy graphs clean (ssr + hydrate-wasm32 with `-D warnings`, freya at least error-free); `cargo test --features ssr` reports `0 failed` (incl. the new `accent`, `schema_apply`, `sk_orbit::*`, `ui::accent::*`, `server::accent::*`, and the unchanged `skeleton_switch`/`style_lint` suites); release leptos build succeeds; native build succeeds.

- [ ] 9.4.2 — Headed-Playwright visual smoke on `localhost:3000` + the `dev` DB ONLY (start `cargo leptos watch`; inject the `authlyn_session` cookie with `secure:false` if testing WebKit — the localhost Secure-cookie trap). Run THIS exact checklist, capturing a screenshot at each milestone:

  1. **Boot + ceremony:** clear localStorage → the skeleton ceremony shows (no silent default) → pick "Omloppsbana" → orbit shell mounts.
  2. **Pill + orbit-map:** the pill floats top-center with channel + server name + position dots → tap → the orbit map opens (active server's channels on a ring, other servers docked) → tap a node → dives to that channel + map closes → tap the pill → tap a far server core → switches server → Esc closes the map → focus returns to the pill.
  3. **Swipe strip:** the channel pane is the middle of a 3-pane strip → drag horizontally (tracks 1:1) → release past ~32% or flick → switches to the neighbor channel → at the first/last channel the drag rubber-bands → vertical drag scrolls the message list (axis-lock holds).
  4. **Charge ring + orb:** type a one-liner (ring sliver) → a paragraph (~60%) → confirm EXACTLY ONE ring reflects length (the in-pane ChannelPane send button + its linear ring are hidden under orbit) → tap the orb → sends (ring pulses).
  5. **Effect blossom:** long-hold the orb 480ms (intentionally longer than the radial's 450ms — separate detector, fewer accidental blossoms on a send tap) → three chips fan up + (if haptics on) a tick → tap "spell" → ring tints + glyph swaps → the trailing click does NOT send → next send carries the effect.
  6. **Slide-over:** tap ☰ → the HoloPanel slides in from the right → wear a persona (card highlights) → toggle Ghost Quill / Aurora-max (prefs flip live) → swipe-right inside the panel closes it → Esc closes → focus trapped + restored. Confirm NO chat-layer right-edge swipe summons it.
  7. **Radial + arbitration:** long-press a message row → the radial blossoms with the right actions (reply+copy on a roll, +edit/delete on your own user message, nothing on a system row) → a short right-drag starting ON a row does NOT switch channels → a drag starting in empty pane area DOES.
  8. **Warp tint (#A/#G):** set a server accent (🎨 → purple) → switch channels with eye-candy ON → the warp dip/streak tints purple → switch direction flips the slide side (`--warp-dir`).
  9. **Scene-light (#B):** eye-candy ON → post as differently-tinted personas → the pane ambient wash shifts toward the latest speaker → eye-candy OFF → the wash disappears.
  10. **Switch-never-remounts:** with a channel open + draft typed + scrolled mid-history, switch skeleton orbit→deck→orbit via the account modal → the SSE chip stays LIVE (no reconnect in the network panel), the composer draft survives, the selected channel + scroll position survive.
  11. **Device matrix:** `browser_resize` to 360×800 (POCO C3 floor), 375×667 (SE), 375×812 (13 mini notch), 412×892 (Nothing 2) → at every size the pill/orb/map/strip are usable, nothing is hardcoded-375 (geometry is `clamp()`/`%`/`dvh`), notch + home-indicator insets are respected exactly once.
  12. **content-visibility 3-point:** a reply-quote scrollIntoView lands on a skipped row; near-top history backfill keeps the anchor; jump-to-unread scrolls correctly. The LIVE message rows (`.messages li`) get `content-visibility:auto` from the shared #36 foundation rule (`_content.scss:154`, NOT from `_sk_orbit.scss` — Task 5.2.4 only adds it to the prev/next PEEK panes), so this item tests an actually-applied property; orbit inherits it because the middle pane is the same `ChannelPane`.
  13. **Reduced-motion:** force `prefers-reduced-motion` → every orbit motion is instant, all state intact.

- [ ] 9.4.3 — Record the smoke result (pass/fail per item) in the PR/handoff. Any FAIL is a bug to fix before the wave gate, not to explain away (owner value: granskningsfynd åtgärdas). The remaining KNOWN follow-ups to book explicitly (NOT Phase-2 blockers, recorded for the wave): (a) the swipe-to-reply ROW VISUAL (↩ glyph offset + `act::start_reply` trigger) — only the arbitration shipped; (b) the neighbor-pane LAZY first-page peek-fetch (Phase 2 shipped NAME-ONLY peeks — this delivers the full neighbor message render, read-only, explicitly NOT touching last_seen/mark-read, AND designs the peeked-neighbor SSE semantics — filtered `visible_channels`, refetch-on-settle — which are N/A while neighbors are name-only); (c) the ≥300ms peek-settle gate (`peek_settles`, removed from Phase-2 strip.rs as dead code) lands TOGETHER with (b) — once a lazy-rendered neighbor can briefly become "current" mid-drag, its mark-read must wait ≥300ms; this is NOT an open Phase-2 correctness hole (Phase-2 committed switches correctly mark the now-active channel read, and name-only neighbors never mark read — see Task 1.3 intro); (d) the charge-transfer mote (#33 ceremony, portaled); (e) the composer EXPAND-on-orb-tap (the textarea stays in ChannelPane); (f) the real-iOS axis-lock + radial-450ms + touch-AT VoiceOver gate (iPhone 13 mini, iOS 26.5 — NOT emulated). These are the Phase-7 per-skeleton gate items + ÖG refinements.

- [ ] 9.4.4 — Final commit (gate evidence; if 9.4.1/9.4.2 surfaced fixes, they are their own commits first):

```bash
git commit --allow-empty -m "$(cat <<'EOF'
chore(ui): W5/P2 Omloppsbana gate pass — full gate + visual smoke (W5/P2)

cargo fmt --check + clippy ssr/hydrate-wasm32 (-D warnings) + freya + cargo test
--features ssr (0 failed) + cargo leptos build --release + native build all green.
Headed-Playwright smoke (localhost + dev DB) verified: ceremony, pill→orbit-map
dive/glide, 3-pane swipe strip + axis-lock, charge ring, effect blossom, right-
edge slide-over, radial + strip-vs-row arbitration, warp tint (#A/#G), scene-
light (#B), switch-never-remounts (SSE/draft/selection), device matrix, reduced-
motion. Three binding kills (pinch entry / deck-plate / Persona Orrery) honored;
message_actions never re-branched. Phase-7 real-iOS + ÖG refinements booked.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Open questions / decisions deferred to execution

1. **WASM bundle budget (Foundation Open Q #1):** orbit's `sk_orbit/` code (4 pure-math modules + 2 gesture structs + the shell view) counts against the owner-signed hydrate budget. Measure `target/site/pkg/*.wasm` after `cargo leptos build --release` and report the delta; if it breaches the owner's number, the orbit-map far-server glide + scene-light are the first cut candidates.
2. **`event_target_checked` / `StoredValue::try_get_value` / `MessageEnvelope.persona_color` / persona DTO field names:** the plan uses these by the names the gatherers cited; confirm each exact symbol at execution (mirror the proven call sites — `radial.rs` for `try_*`, the existing composer for `event_target_*`) and substitute if a name differs. None are load-bearing to the design; they are wiring details.
3. **Peek-settle mark-read (≥300ms) — RESOLVED, no longer open:** the mark-read path was read at plan-revision time. `act::open_channel` → `open_channel_at` marks read IMMEDIATELY on open (`set_last_seen`, `src/ui/shell/act/channel.rs:305`). For a committed swipe this is CORRECT (the destination is the active channel). The "peek" harm cannot occur because orbit's neighbors are name-only and never call `open_channel`. So peek-never-marks-read holds with NO ≥300ms timer; `peek_settles` was REMOVED from the shipped `strip.rs` (it would be dead code) and is deferred to ship WITH the lazy neighbor render (9.4.3-b/-c) that would first make a neighbor briefly "current".
4. **Composer textarea in orbit:** Phase 2 keeps the textarea in the inherited `ChannelPane` composer (the orb is the floating SEND surface). The "tap orb → expand a dedicated composer sheet" flow (prototype `expandComposer`) is an ÖG refinement (9.4.3-e), not a Phase-2 blocker.
5. **`guild.accent_color` ASSERT:** Task 0 deliberately ships NO format ASSERT (validated server-side, mirroring `persona.color`). If the owner later wants a DB-level ASSERT it MUST use `DEFINE FIELD OVERWRITE` (the enum-OVERWRITE invariant) and add a prod-shaped guard cloning the `message_effect_guard` test — recorded in the schema comment.
