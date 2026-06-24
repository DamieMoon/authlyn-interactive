# 06 — Markup Engine

The chat markup engine turns a raw message `body` string into a render tree
(`Vec<Node>`) and renders that tree to Leptos elements. It is the **always-on
spine**: `src/markup/` and `src/protocol.rs` are the only modules compiled into
all three feature graphs (ssr, hydrate, nova — see [01 — Overview](01-overview.md)
and the `[features]` `#`-comments in `../../Cargo.toml`). Everything in
`src/markup/` is therefore **serde/std-only** — zero axum, surrealdb, tokio,
leptos, or web-sys — so it compiles to `wasm32-unknown-unknown`.

The Leptos *view* layer (`src/ui/markup_view.rs`, `src/ui/crest.rs`) is **not**
always-on: it lives in the ssr+hydrate graph and pulls `leptos::prelude`. The
parser and the crest algebra never reach into it.

> Pinning note: the parser has **no dedicated `tests/*.rs` suite**. Its canonical
> pins are the **61 inline `#[test]` in `src/markup/mod.rs`** plus **6 in
> `src/markup/crest.rs`**. `tests/mentions.rs` pins only the *server* mention
> resolution layer (it never calls the parser directly). A reader hunting "the
> pinning test" under `tests/` will not find one for `parse` /
> `collect_mentions` / `strip_color_tokens`.

---

## 1. Pipeline overview

```
raw body (String)                  protocol.rs:259 — server stores verbatim
   │
   ▼  markup::parse(&str)                                    (mod.rs:161)
blocks::parse_blocks  ── split on '\n' ──┐
   │                                     ├─ ``` fence  → CodeBlock (verbatim)
   │                                     ├─ #/##/###/-# → Heading/Subtext
   │                                     └─ plain lines → buffered into one run
   ▼
for each non-fence block:
   tokenize(&str) ─────────►  Vec<Tok>           inline scan      (tokenize.rs)
   build_tree(Vec<Tok>) ────►  Vec<Node>         stack tree       (tree.rs)
   │
   ▼
Vec<Node>  (the AST)
   │
   ├─ RENDER  ui::markup_view::render_body ─► Leptos AnyView      (markup_view.rs)
   │          styled by _markup.scss + _wave_b.scss
   │
   └─ MENTIONS markup::collect_mentions ─► Vec<String> ──► server resolve
              (AST walk; server/messages/posting.rs:161 → pinged_users)
```

Two passes, in this order:

1. **Block pass** (`blocks::parse_blocks`) — line-oriented. Splits on `\n`.
   Each line is one of: a fence boundary (a line that trims to exactly
   ` ``` `), a line-leading block marker (`#`/`##`/`###`/`-#` **followed by a
   space**), or a plain line. Consecutive plain lines are buffered and joined
   with `\n` into a single inline run (the embedded newlines survive, rendered
   via `white-space: pre-wrap`).
2. **Inline pass** (`tokenize` → `build_tree`) — runs on the inline content of
   each non-fence block (a plain-line run, or the text after a `#`/`-#` marker).
   `tokenize` produces a flat `Vec<Tok>`; `build_tree` folds it into the nested
   `Vec<Node>`.

Fenced code blocks **skip the inline pass entirely** — their body is captured
verbatim as `Node::CodeBlock`.

`parse` is `mod.rs:161`; it is a one-liner delegating to `blocks::parse_blocks`.

---

## 2. Grammar reference

Mirror of the authoritative header at `src/markup/mod.rs:9-41`. Every construct
is **lenient**: a malformed or unmatched form falls back to literal text (see
§3). Pins are inline tests in `src/markup/mod.rs` (cited by test name).

### Inline (anywhere within a line)

| Construct  | Syntax            | `Node`        | Leniency / notes | Pinned by |
|------------|-------------------|---------------|------------------|-----------|
| bold       | `**text**`        | `Bold`        | toggles against innermost frame | `bold_italic_and_color` |
| italic     | `*text*`          | `Italic`      | RP convention: `*she waves*` → italic | `rp_action_asterisks_become_italic` |
| color      | `[name]…[/name]`  | `Color`       | `name` ∈ the 8-entry palette; open/close **id-matched** | `bold_italic_and_color`, `nested_color_with_bold_and_italic` |
| inline code| `` `text` ``      | `Code(String)`| **verbatim** — no inner markup, no autolink, no mention | `inline_code_is_literal_inside` |
| dialogue   | `"text"`          | `Dialogue`    | renderer **re-emits** the quotes; toggles | `dialogue_quotes_become_a_node` |
| emoji      | `:shortcode:`     | `Emoji(String)`| `[a-z0-9_]+`, **lowercase-only**; resolved at render | `emoji_shortcode_becomes_a_node` |
| image      | `![alt](url)`     | `Image(alt,url)`| alt → first `]`, url → first `)` | `image_becomes_a_node` |
| link       | `[text](url)`     | `Link(text,url)`| `url` whitelisted to http/https/relative (§4) | `explicit_markdown_link_becomes_a_link_node` |
| autolink   | bare `http(s)://…`| `Link(url,url)`| trailing punctuation trimmed; empty host rejected | `bare_http_and_https_urls_autolink` |
| spoiler    | `\|\|text\|\|`    | `Spoiler`     | hidden until click; toggles | `spoiler_becomes_a_node` |
| mention    | `@username`       | `Mention(name)`| leading letter/`_`, not digit, not mid-word | `bare_mention_becomes_a_node` |

The fixed color palette (`markup::Color`, `mod.rs:67`) is **red, orange, yellow,
green, blue, purple, pink, gray** — no hex, no fonts, by design (`mod.rs:41`).
`Color::ALL` / `Color::name` / `Color::from_name` are the public surface.

### Block / line-leading (Discord-style)

The marker **must start the line and be followed by a space** — a bare `#` or
`#foo` is literal (`bare_hash_without_space_is_literal`).

| Construct       | Syntax        | `Node`            | Pinned by |
|-----------------|---------------|-------------------|-----------|
| heading L1/L2/L3| `# ` / `## ` / `### ` | `Heading(level, …)` | `headings_by_level` |
| subtext         | `-# `         | `Subtext(…)`      | `subtext_block` |
| fenced code     | line `` ``` `` … `` ``` `` | `CodeBlock(String)` | `fenced_code_block_verbatim` |

Heading and subtext children are **parsed inline**, so a heading can still hold
bold/color/link/mention (`heading_keeps_inline_markup`). The fence body is
**not** — it is verbatim.

### The `Node` enum (14 variants)

`mod.rs:116`. Inline: `Text`, `Bold`, `Italic`, `Color`, `Code`, `Dialogue`,
`Emoji`, `Image`, `Link`, `Spoiler`, `Mention`. Block (top level only):
`Heading`, `Subtext`, `CodeBlock`. (`Text` is shared.) `Node` derives
`Clone, Debug, PartialEq` — the `PartialEq` is what every inline test asserts
against.

---

## 3. The leniency / totality contract

**`parse` never fails and never panics.** Any unmatched opener, mismatched
closer, unknown `[tag]`, bare `#` without a space, or unterminated fence
collapses to literal text — every input renders as *something* reasonable. This
is the load-bearing property that lets the server store arbitrary user text and
the renderer treat the AST as total.

Two mechanisms guarantee it:

1. **Tokenizer fallback** (`tokenize.rs`): every branch that fails to recognise
   a marker does `buf.push(<char>)` and advances one char — the marker becomes
   literal text. Examples: an unterminated `` ` `` (`tokenize.rs:60`), a `[`
   that is neither a color tag nor a link (`tokenize.rs:84`), a `!` with no
   valid image (`:93`), a `:` with no valid shortcode (`:110`), an `@` that is
   mid-word or malformed (`:130`). The final catch-all (`:142`) consumes one
   full UTF-8 char, keeping `i` on a boundary.
2. **Tree unwind** (`tree.rs:63-72`): after consuming all tokens, any frame
   still open is popped, its **opener string is re-emitted as literal text**,
   and its children splice into the parent. So `[green]hi` →
   `Text("[green]hi")` (`unclosed_color_unwinds_to_literal_opener`),
   `**oops` → `Text("**oops")` (`unmatched_bold_is_literal`).

### Toggle vs. id-matched — the one asymmetry

The tree builder treats two classes of span differently (`tree.rs`):

- **Bold / italic / dialogue / spoiler** *toggle* against **only the innermost
  open frame** (`tree.rs:81-98`, `toggle`). A `**` closes a bold only if bold is
  on top; otherwise it opens one. Pathological interleavings (e.g.
  `**a *b** c*`) therefore degrade to literal openers via the unwind rather than
  matching across nesting levels.
- **Color** is *open/close id-matched* (`tree.rs:44-51`): a `[/red]` closes only
  if the innermost frame is a `Color(Red)`; a non-matching `[/c]` is pushed as
  literal text (`stray_color_close_is_literal`), and an unknown palette name
  never tokenizes as a color tag at all (`unknown_color_name_is_literal`).

Adjacent `Text` nodes are merged on insert (`push_text`, `tree.rs:114`) so the
AST stays compact — `push_node` routes `Text` through `push_text` and pushes
everything else directly.

Relevant pins (all `src/markup/mod.rs`): `unmatched_bold_is_literal`,
`stray_color_close_is_literal`, `unknown_color_name_is_literal`,
`unclosed_color_unwinds_to_literal_opener`, `bare_hash_without_space_is_literal`,
`unterminated_inline_code_backtick_is_literal`, `unterminated_fence_is_literal`,
`lone_brackets_pass_through`, `unclosed_dialogue_is_literal_quote`,
`unclosed_spoiler_and_single_pipe_are_literal`.

---

## 4. Security model — how the parser keeps the renderer XSS-safe

The renderer (`markup_view.rs`) interpolates parsed strings into Leptos
elements. Leptos **auto-escapes** all interpolated text and attribute values, so
the markup engine's job is to (a) never emit a dangerous URL scheme and (b) keep
code verbatim. There is no HTML passthrough anywhere.

### URL scheme whitelist

`is_safe_url_scheme` (`tokenize.rs:251`) is the gate. **Only http / https** (or
a scheme-relative / relative reference with no scheme) linkify. `javascript:`,
`data:`, `file:`, `vbscript:`, and any other explicit scheme are rejected — the
whole `[text](url)` stays **literal text**, never an `<a href>`. The scheme
check is **case-insensitive** (`JavaScript:` is rejected too). A leading `:`
(empty scheme name) is rejected; a `/`, `?`, or `#` before any `:` is treated as
relative and allowed.

- Pins (`src/markup/mod.rs`): `javascript_and_other_unsafe_schemes_never_linkify`
  (covers `javascript:`, mixed-case, `data:`, `file:`, `vbscript:`),
  `relative_link_target_is_allowed`, `malformed_bare_scheme_is_literal`.
- The renderer additionally hardens every link with
  `rel="noopener noreferrer nofollow"` and `target="_blank"`
  (`markup_view.rs:59-69`) — defends `window.opener` tampering and referrer
  leakage. This `rel` is **unpinned** (render-side, no unit test).

### Code is verbatim

No inner markup, no autolink, no mention scanning inside an inline `` `code` ``
span or a ` ``` ` fence. Inline code is captured before the tree builder ever
sees it (`tokenize.rs:51-62`, emitted as `Tok::Code` with literal contents);
fence bodies bypass the inline pass in `blocks.rs`.

- Pins (`src/markup/mod.rs`): `inline_code_is_literal_inside`,
  `url_inside_inline_code_stays_literal`, `mention_inside_inline_code_stays_literal`,
  `fenced_code_block_verbatim`, `collect_mentions_finds_nested_but_skips_code`.

### Mentions are syntactic only

`Node::Mention` records the bare username as typed. The parser **does not**
decide whether the name is a real member — that resolution is **server-side**
(§5). So a `@anyone` in a message is inert at parse time; it only becomes a ping
when the server matches it to a guild member.

---

## 5. Mention pipeline

### Tokenizer recognition (`parse_mention`, `tokenize.rs:309`)

A mention is `@` + a leading ASCII **letter or `_`** (not a digit), followed by
a run of `[A-Za-z0-9_]`. Two extra rules:

- **Mid-word suppression** (`tokenize.rs:113-133`): if the char immediately
  before `@` is a word char (`[A-Za-z0-9_]`), the `@` is **not** a mention —
  `user@host.com`, `a@b`, `parse@there` keep their `@` literal. The check reads
  the **last byte of `buf`**; it is correct only because `@` and all word chars
  are ASCII (see §8 hotspot). An `@` after punctuation/space still mentions
  (`(@nick)` → `@nick`).
- **Digit-led rejected**: `@123` is literal (a bare number is far more likely
  "@ 123" prose than a handle); `@user123` works.

The node **preserves case as typed**; lowercasing happens at collection time.

- Pins (`src/markup/mod.rs`): `bare_mention_becomes_a_node`,
  `mention_preserves_case_in_node`, `malformed_at_signs_stay_literal`,
  `mid_word_at_stays_literal`, `mention_terminates_at_non_word_char`,
  `mention_nested_in_bold_and_color`.

### Extraction (`collect_mentions`, `mod.rs:172`)

Walks the **parsed AST** (so the same leniency the renderer obeys applies):
recurses into every container node (bold/italic/color/heading/subtext/dialogue/
spoiler) and collects `Node::Mention` names. The leaf set
`Text | Code | CodeBlock | Emoji | Image | Link` is **skipped**
(`mod.rs:202-207`) — crucially `Code`/`CodeBlock`, so an `@name` in code is never
a ping. Output is **lowercased, de-duplicated, first-appearance-ordered**.

- Pins (`src/markup/mod.rs`): `collect_mentions_lowercases_and_dedupes_in_order`,
  `collect_mentions_finds_nested_but_skips_code`, `collect_mentions_empty_when_none`.

### Server resolution

`server/messages/posting.rs:161` calls `collect_mentions(&body)` →
`resolve_mentions` (matches the lowercased names against real guild members,
case-insensitively) → `pinged_users` (a record-id array, empty when nobody is
mentioned), stored on the message (`posting.rs:288-312`). Each reader's
`is_pinged` is projected per-reader. A non-member / unknown handle resolves to
nobody. This end-to-end resolution is the **only** layer pinned under `tests/`:

- `tests/mentions.rs::mentioning_a_member_pings_only_that_reader`
- `tests/mentions.rs::case_insensitive_mention_resolves`
- `tests/mentions.rs::mentioning_a_nonmember_or_unknown_pings_nobody`
- `tests/mentions.rs::mention_inside_inline_code_does_not_ping`
- `tests/mentions.rs::no_mention_leaves_is_pinged_false`
- `tests/mentions.rs::pagination_and_cursor_unaffected_by_pinged_users`

See [05 — Auth & Privacy](05-auth-privacy.md) for the privacy-404 that makes a
ping to a non-member meaningless (they can't read the channel at all) and
[03 — Data Model](03-data-model.md) for the message `pinged_users` field.

---

## 6. The render view (`src/ui/markup_view.rs`)

ssr+hydrate Leptos. `render_body(body) → AnyView` parses then maps each `Node`
to an element via `render_node` (`markup_view.rs:22`):

| `Node`        | Emits |
|---------------|-------|
| `Text`        | the string (Leptos-escaped) |
| `Bold`/`Italic` | `<strong>` / `<em>` |
| `Color(c,…)`  | `<span class="mk-{name}">` |
| `Code`        | `<code class="mk-code">` (verbatim) |
| `Heading(l,…)`| `<h1/h2/h3 class="mk-h{l}">` |
| `Subtext`     | `<small class="mk-subtext">` |
| `CodeBlock`   | `<pre class="mk-pre"><code>` |
| `Dialogue`    | `<span class="mk-dialogue">` with the quotes **re-emitted** |
| `Emoji`       | resolved via `EmojiResolver` context (below) |
| `Image`       | `<img class="mk-image" loading="lazy">` |
| `Link`        | `<a class="mk-link" target="_blank" rel="noopener noreferrer nofollow">` |
| `Mention`     | `<span class="mk-mention">@name</span>` — a non-interactive pill, **not** a link in v1 |
| `Spoiler`     | `<span class="mk-spoiler">` with a per-node `RwSignal`; click sets `revealed=true` |

Two non-obvious decisions:

- **Spoiler reactivity is per-node** (`markup_view.rs:76-88`): each spoiler gets
  its own `RwSignal::new(false)` and a `class:revealed` toggle. No `web_sys` —
  so it compiles for ssr too (inert until hydrated). (Unpinned — view layer.)
- **Emoji resolves via context, non-reactively.** `Node::Emoji(name)` looks up
  `EmojiResolver` (`use_context`, provided once at `src/ui/shell/mod.rs:298`);
  an absent context (ssr / outside a guild) falls back to the literal `:name:`
  (`markup_view.rs:50-52`). `EmojiResolver::resolve` reads the custom-emoji map
  with **`get_untracked`** (`src/ui/emoji/mod.rs:39`) **on purpose** — chat
  history must not re-render when someone uploads a new guild emoji. This looks
  like a reactivity bug to an unaware reader; it is deliberate. (Unpinned.)

Resolution order for `:name:`: custom guild emoji image → standard unicode glyph
→ literal `:name:`. The renderer relies on the tokenizer's URL whitelist (§4) so
`<a href>` is always http/https/relative — it does no scheme check of its own.

---

## 7. Crest algebra (`src/markup/crest.rs`)

A separate, dependency-free sub-module of `markup/` (`pub mod crest`,
`mod.rs:61`). It is **heraldic blazonry derived purely from `(name, debut)`** —
pure math, no leptos/web-sys/DOM, so it compiles to every graph like the rest of
`markup/`. The SVG *view* that draws a `Blazon` lives in `src/ui/crest.rs`
(ssr+hydrate). Design spec §9.7 (Vapenskölden).

### Derivation

`Blazon::derive(name, debut)` (`crest.rs:93`):

1. `name_hash(name, debut)` — a **64-bit FNV-1a** over `name.trim().to_lowercase()`
   then a `0x1f` unit separator then `debut` (`crest.rs:124`). **Hand-rolled,
   deliberately NOT `DefaultHasher`/`RandomState`** — those seed randomly per
   process, which would make the crest differ across reloads **and between the
   ssr render and the hydrate render**. This is the non-obvious *why*: ssr/hydrate
   determinism.
2. Slice the hash into **disjoint bitfields** (`crest.rs:96-108`): `& 0x7` →
   field tincture; `>> 8 & 0x7` → contrast tincture, **re-rolled to differ from
   the field** so the division/ordinary stay legible; `>> 16 & 0x7 % 5` →
   division; `>> 24 & 0xF % 7` → ordinary. `initial(name)` → the uppercase first
   `char` (`'?'` when empty).

The `Blazon` struct (`crest.rs:77`) carries `{ field, contrast, division,
ordinary, initial }`, all `Color`/enum/`char` — `Clone, Copy, Eq`.

### Blazon space

8 fields × 7 contrasts × 5 divisions × 7 ordinaries ≈ **1 960 layouts**, × ~26
common initials ≈ **50 000** visually-distinct crests. Visible heraldic repeats
appear (birthday bound) around **~226 personas** — a property of heraldry, not a
hash collision; the 64-bit hash itself birthday-collides only near ~5×10⁹ inputs
(`crest.rs:11-15`).

### Single palette source

Crest tinctures **reuse the one `markup::Color` palette** (`crest.rs:20`,
`use super::Color`) → the `--tint-*` CSS tokens. There is exactly one palette
source in the codebase — no second palette for crests.

### `initial` is duplicated, not imported

`initial` (`crest.rs:148`) returns a single `char` and is **duplicated rather
than imported from `ui::avatar::monogram`** (`crest.rs:145-147`): `ui/` pulls
leptos and isn't compiled for the nova graph, while this module is always-on.
(It also differs semantically: German `ß` → `'S'` here vs. `monogram`'s `"SS"`.)

### View (`src/ui/crest.rs`)

`<Crest name debut? class?>` derives the `Blazon` **once** and builds the SVG as
a **single `inner_html` String** (`crest.rs:40`) — no per-crest reactive node,
so 100+-card grids stay cheap. `render_blazon` emits `fill="var(--tint-NAME)"`
fills; `escape_text` hand-escapes the persona initial (`& < >`). Zero web-sys.
Consumed by `src/ui/shell/wardrobe.rs:211`.

### Pins (`src/markup/crest.rs`)

| Invariant | Test |
|-----------|------|
| same `(name, debut)` → same blazon (process-seed-free) | `derive_is_deterministic` |
| contrast tincture always `!= field` | `contrast_always_differs_from_field` |
| `debut` diverges like-named personas | `debut_diverges_like_named_personas` |
| name is case- and whitespace-insensitive | `name_is_case_and_whitespace_insensitive` |
| `initial` uppercase-first-char with `'?'` fallback | `initial_is_uppercase_first_char_with_fallback` |
| distribution spreads, never panics (>200 distinct / 500 names) | `distribution_touches_many_blazons_without_panic` |

---

## 8. Complexity hotspots

Read the code, not this list, before changing any of these.

- **Mid-word `@` suppression** (`tokenize.rs:113-133`): decides on the **last
  byte of `buf`**. Correct only under the ASCII-boundary assumption — a non-ASCII
  trailing char would never match `is_ascii_alphanumeric`, so the `@` would
  mention. Holds because `@` and all word chars are ASCII.
- **`autolink_len`** (`tokenize.rs:211-244`): two-stage — consume to a stop char
  (`is_ascii_whitespace` or `< > " ` `` ` `` `|`), then **trim trailing
  punctuation** (`. , ; : ! ? ) ]`), then **reject empty host**. Operates on
  bytes via `split_last` on `host[..len]`; off-by-one-prone. Pinned by
  `autolink_trims_trailing_sentence_punctuation_and_parens`. The stop-char and
  trim sets are load-bearing heuristics documented **only here and in the
  `tokenize.rs` prose** — enumerate them when touching this.
- **`[` disambiguation order** (`tokenize.rs:72-86`): tries `parse_color_tag`
  **before** `parse_link` (narrower palette grammar first). The order is
  load-bearing — `[red](x)` would mis-parse as a link if link were tried first.
- **Tree asymmetry** (`tree.rs:44-51` + `81-98`): color is open/close-id-matched
  while bold/italic/dialogue/spoiler toggle against only the innermost frame
  (§3). Pathological interleavings degrade to literal via the unwind at
  `tree.rs:63-72`.
- **`Blazon::derive` bitfield slicing** (`crest.rs:96-117`): `>> 24 & 0xF` feeds
  a **7-entry** array via `%`, so there is intentional, benign **modulo bias**;
  the contrast re-roll guarantees `field != contrast`.
- **`strip_color_tokens` is a SECOND scanner** (`mod.rs:222-273`): a hand-written
  scanner (`color_tag_len`, `image_run_len`) **separate from `tokenize.rs`** that
  must stay behaviorally in sync with the tokenizer's color+image leniency. It is
  the copy-as-markdown helper (drops only well-formed `[name]`/`[/name]` for the
  8 palette names, passes unknown `[tags]`, treats `![alt](url)` verbatim,
  UTF-8-boundary-safe). The only thing linking the two scanners is the
  round-trip test — **a drift risk**.
  - Pins (`src/markup/mod.rs`): `strip_color_drops_all_eight_palette_names`,
    `strip_color_keeps_image_syntax_intact_even_with_color_alt`,
    `strip_color_is_utf8_boundary_safe`,
    `strip_color_round_trip_through_parse_drops_color_nodes`,
    `strip_color_preserves_non_color_bracket_runs`. Consumed by
    `src/ui/shell/act/message.rs:533`.

---

## 9. CSS map & fidelity gotchas

Markup CSS is **split across two files** — non-obvious, and undocumented in
either file. See [08 — Styling & Chrome](08-styling-chrome.md) for the token
system; all colors come from `--tint-*` in `style/_tokens.scss`.

| File | `@use` in main.scss | Classes |
|------|---------------------|---------|
| `style/_markup.scss` | line 18 | `.mk-{color}` / `.mk-bg-{color}`, the per-persona name tint, the color-picker swatches, block `.mk-h1..3` / `.mk-subtext` / `.mk-link` / `.mk-mention` / `.mk-code` / `.mk-pre` |
| `style/_wave_b.scss` | line 31 | the **inline** classes `_markup.scss` omits: `.mk-image`, `.mk-spoiler` (+ `.revealed`), `.dialogue-style .mk-dialogue` (per-user toggle) |

Two load-bearing details:

- **Long selector for the persona name tint** (`_markup.scss:64`):
  `.content .messages .msg .who.mk-#{$n}` (specificity 0,4,0) is required to beat
  the default `.who` color (also 0,4,0). A "simplified" `.msg .who.mk-*` (0,3,0)
  **loses** and the name stays the default slate. The `_markup.scss:54-67`
  comment says: do not simplify.
- **44px touch floor on color swatches** (`_markup.scss:76-89`):
  `.swatch-pick` is `2.75rem × 2.75rem` per the owner ruling 2026-06-17. See the
  touch-target floor rule in [08 — Styling & Chrome](08-styling-chrome.md) /
  `../../CLAUDE.md`.

**No `style_lint` guards exist for any `mk-*` / mention / spoiler / dialogue
class.** Unlike most chrome, the markup classes are **not** in the curated
`tests/style_lint.rs` registries — visual regressions here are
**owner-deck-only** (verified on the iPhone/WebKit deck, see
[09 — Testing](09-testing.md) and `../../CLAUDE.md` *UI fidelity*).

---

## 10. Known doc/source drift

- The 3-way graph membership (always-on, wasm-safe) is asserted in the module
  headers (`mod.rs:7`, `crest.rs:3-5`) but has **no dedicated cross-graph compile
  test** in `tests/`. It is **process-enforced**, not test-pinned: the `/check`
  step `cargo clippy --features hydrate --target wasm32-unknown-unknown` would
  fail if anything in `markup/` pulled a non-wasm dep (see `../../CLAUDE.md`
  *Quality gate* and `../../Cargo.toml` `[features]`). Treat "markup/ is
  wasm-safe" as an **(unpinned)** invariant guarded by CI, not a unit test.

---

## Source map

Key files:

- `src/markup/mod.rs` — public API (`parse`, `collect_mentions`,
  `strip_color_tokens`), `Color`, `Node`; **61 inline `#[test]` = the canonical
  parser pin**.
- `src/markup/tokenize.rs` — inline scanner (`Tok`, `tokenize`); URL whitelist
  (`is_safe_url_scheme`), `parse_link` / `parse_image` / `parse_color_tag` /
  `autolink_len` / `parse_emoji` / `parse_mention`; `buf.push` leniency.
- `src/markup/tree.rs` — stack tree builder (`build_tree`); toggle vs.
  id-matched, unclosed-frame unwind, `push_text` merging.
- `src/markup/blocks.rs` — block pass (`parse_blocks`, `parse_line_block`);
  fence handling, plain-line buffering.
- `src/markup/crest.rs` — heraldic algebra (`Blazon::derive`, `name_hash`,
  `Division`, `Ordinary`); hand-rolled FNV for ssr/hydrate determinism; **6
  inline `#[test]`**.
- `src/ui/markup_view.rs` — Leptos render (`render_body`, `render_node`);
  ssr+hydrate, escapes everything, `rel` hardening, per-node spoiler signal.
- `src/ui/crest.rs` — Leptos `<Crest>` SVG view (single `inner_html`,
  `var(--tint-*)` fills).
- `src/ui/emoji/mod.rs` — `EmojiResolver` (`resolve`, `get_untracked`);
  provided at `src/ui/shell/mod.rs:298`.
- `style/_markup.scss` — palette + block markup CSS (`@use`d at `main.scss:18`).
- `style/_wave_b.scss` — inline image/spoiler/dialogue CSS (`@use`d at
  `main.scss:31`).

Consumers: `src/server/messages/posting.rs:161` (`collect_mentions` →
`pinged_users`), `src/ui/shell/act/message.rs:533` (`strip_color_tokens`),
`src/ui/{modal.rs:499, shell/wardrobe.rs:449, shell/channel/mod.rs:1178/1189/1336}`
(`render_body`), `src/server/personas/core.rs:597` +
`src/ui/shell/{wardrobe.rs:17, channel/meta.rs:14, channel/mod.rs:54}`
(`Color`), `src/protocol.rs:259` (doc reference — `body` may contain markup).

Tests that pin the claims:

- **Parser (inline, `src/markup/mod.rs`)** — totality:
  `unmatched_bold_is_literal`, `stray_color_close_is_literal`,
  `unknown_color_name_is_literal`, `unclosed_color_unwinds_to_literal_opener`,
  `bare_hash_without_space_is_literal`,
  `unterminated_inline_code_backtick_is_literal`, `unterminated_fence_is_literal`;
  security: `javascript_and_other_unsafe_schemes_never_linkify`,
  `relative_link_target_is_allowed`, `malformed_bare_scheme_is_literal`,
  `inline_code_is_literal_inside`, `url_inside_inline_code_stays_literal`,
  `mention_inside_inline_code_stays_literal`, `fenced_code_block_verbatim`;
  mentions: `bare_mention_becomes_a_node`, `mention_preserves_case_in_node`,
  `malformed_at_signs_stay_literal`, `mid_word_at_stays_literal`,
  `mention_terminates_at_non_word_char`,
  `collect_mentions_lowercases_and_dedupes_in_order`,
  `collect_mentions_finds_nested_but_skips_code`, `collect_mentions_empty_when_none`;
  strip: `strip_color_drops_all_eight_palette_names`,
  `strip_color_keeps_image_syntax_intact_even_with_color_alt`,
  `strip_color_is_utf8_boundary_safe`,
  `strip_color_round_trip_through_parse_drops_color_nodes`; autolink:
  `bare_http_and_https_urls_autolink`,
  `autolink_trims_trailing_sentence_punctuation_and_parens`.
- **Crest (inline, `src/markup/crest.rs`)** — `derive_is_deterministic`,
  `contrast_always_differs_from_field`, `debut_diverges_like_named_personas`,
  `name_is_case_and_whitespace_insensitive`,
  `initial_is_uppercase_first_char_with_fallback`,
  `distribution_touches_many_blazons_without_panic`.
- **Server mention resolution (`tests/mentions.rs`)** —
  `mentioning_a_member_pings_only_that_reader`,
  `case_insensitive_mention_resolves`,
  `mentioning_a_nonmember_or_unknown_pings_nobody`,
  `mention_inside_inline_code_does_not_ping`, `no_mention_leaves_is_pinged_false`,
  `pagination_and_cursor_unaffected_by_pinged_users`.
- **Unpinned (process-enforced, not unit-tested):** markup/ wasm-safety
  (CI `clippy --features hydrate --target wasm32`); the view layer
  (`render_body`, spoiler signal, emoji `get_untracked`, link `rel`); all
  `mk-*` CSS (no `style_lint` guard — owner-deck-only).
