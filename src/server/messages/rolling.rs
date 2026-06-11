//! `POST /channels/{cid}/roll` — the Fate Engine (W4/T6): server-authoritative
//! dice. ssr-only.
//!
//! The client sends only an EXPRESSION; the server parses it against a
//! constrained grammar, rolls with its own RNG ([`rand::thread_rng`]), and
//! persists the formatted result as a `kind='roll'` message — so a client can
//! never compute (and therefore never forge) an outcome. The companion guard
//! lives in [`super::editing`]: a roll is FULLY immutable (edit and delete
//! both 403, even for the author).
//!
//! ## Grammar (the documented contract)
//! - `NdM`, `NdM+K`, `NdM-K` — `1 ≤ N ≤ 100` dice of `2 ≤ M ≤ 1000` sides
//!   with an optional flat modifier `|K| ≤ 1000`. Bare `dM` reads as `1dM`
//!   (the tabletop shorthand); `d` is case-insensitive; NO whitespace.
//! - `coin` — Heads or Tails.
//! - `oracle` — one answer from [`ORACLE_ANSWERS`].
//! - Anything else → 400.
//!
//! ## Result format (rendered verbatim by clients)
//! - dice: `2d20+3 → [14,8]+3 = 25` (canonical lowercase expr; no modifier ⇒
//!   no `±K` segment: `2d20 → [14,8] = 22`)
//! - coin: `coin → Heads`
//! - oracle: `oracle → Yes, but…`
//!
//! Persona attribution is the SAME validated path as a normal send
//! ([`super::posting::resolve_send_persona`] — `can_edit_persona` re-checked
//! for both the suggested and the stored persona) and the row is persisted by
//! the same [`super::posting::persist_message`] (identity snapshot included).

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use rand::Rng;

use crate::protocol::{RollRequest, SendMessageResponse};
use crate::server::auth::AuthAccount;
use crate::server::errors::{error_response, json_rejection_response};
use crate::server::state::AppState;

use super::posting::{persist_message, resolve_send_persona};
use super::{channel_access, AccessOutcome};

/// Most dice in one roll (`NdM`: N ≤ 100).
const MAX_DICE: u32 = 100;
/// Most sides on one die (`NdM`: M ≤ 1000). A die also needs at least 2 faces.
const MAX_SIDES: u32 = 1000;
/// Largest flat modifier magnitude (`±K`: |K| ≤ 1000).
const MAX_MODIFIER: i64 = 1000;

/// The oracle's documented answer set (`/oracle`): a balanced yes/no/maybe
/// spread with roleplay-friendly "but…" hooks. Public so tests (and any future
/// client hinting) pin the exact set.
pub const ORACLE_ANSWERS: [&str; 6] = [
    "Yes.",
    "Yes, but…",
    "Maybe.",
    "Ask again later.",
    "No, but…",
    "No.",
];

/// A parsed, bounds-checked roll expression. Parsing is PURE (no RNG) so the
/// 400 path never burns entropy and the grammar is testable in isolation.
enum RollSpec {
    /// `n` dice of `m` sides plus a flat `modifier` (0 when absent — `+0` and
    /// "no modifier" format identically without one, so nothing is lost).
    Dice {
        n: u32,
        m: u32,
        modifier: i64,
    },
    Coin,
    Oracle,
}

/// Parse an ASCII-digits-only number. Rust's integer `parse` accepts a
/// leading sign (`"+2".parse::<u32>()` is `Ok(2)`), which would let `1d6++2`
/// or `+1d6` slip through the strict grammar — so every numeric segment goes
/// through this digits-only gate (sign placement is the GRAMMAR's job).
/// Overflow (e.g. a 30-digit side count) also lands here as `None`.
fn parse_digits(s: &str) -> Option<u32> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// Parse + bounds-check one expression. Errors are the user-facing 400 texts.
fn parse_roll_expr(expr: &str) -> Result<RollSpec, &'static str> {
    let expr = expr.trim();
    match expr.to_ascii_lowercase().as_str() {
        "coin" => return Ok(RollSpec::Coin),
        "oracle" => return Ok(RollSpec::Oracle),
        _ => {}
    }
    // Dice: `N?dM([+-]K)?`, case-insensitive `d`, no whitespace.
    let Some((count_part, rest)) = expr.split_once(['d', 'D']) else {
        return Err("unknown roll — try NdM (e.g. 2d20+3), coin, or oracle");
    };
    let n: u32 = if count_part.is_empty() {
        1 // bare `dM` is the tabletop shorthand for one die
    } else {
        parse_digits(count_part).ok_or("dice count must be a number (NdM)")?
    };
    let (sides_part, modifier) = match rest.split_once(['+', '-']) {
        Some((sides, k)) => {
            let k = parse_digits(k).ok_or("modifier must be a number (±K)")?;
            // `split_once` consumed the separator; recover the sign from the
            // original (the separator sits right after the sides digits).
            let signed = if rest.as_bytes()[sides.len()] == b'-' {
                -i64::from(k)
            } else {
                i64::from(k)
            };
            (sides, signed)
        }
        None => (rest, 0),
    };
    let m: u32 = parse_digits(sides_part).ok_or("die sides must be a number (NdM)")?;
    if n == 0 || n > MAX_DICE {
        return Err("dice count must be 1..=100");
    }
    if !(2..=MAX_SIDES).contains(&m) {
        return Err("die sides must be 2..=1000");
    }
    if modifier.abs() > MAX_MODIFIER {
        return Err("modifier must be within ±1000");
    }
    Ok(RollSpec::Dice { n, m, modifier })
}

/// Roll a parsed spec with the SERVER's RNG and format the result body.
/// Synchronous on purpose: `ThreadRng` is `!Send`, so it must never live
/// across an await — and there is none here.
fn run_roll(spec: &RollSpec) -> String {
    let mut rng = rand::thread_rng();
    match *spec {
        RollSpec::Dice { n, m, modifier } => {
            let rolls: Vec<i64> = (0..n).map(|_| rng.gen_range(1..=i64::from(m))).collect();
            let total: i64 = rolls.iter().sum::<i64>() + modifier;
            let rolls_str = rolls
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(",");
            // Canonical expr (lowercase d, explicit count) + the modifier
            // segment only when one was given.
            let modifier_str = match modifier {
                0 => String::new(),
                k if k > 0 => format!("+{k}"),
                k => k.to_string(),
            };
            format!("{n}d{m}{modifier_str} → [{rolls_str}]{modifier_str} = {total}")
        }
        RollSpec::Coin => {
            let face = if rng.gen_bool(0.5) { "Heads" } else { "Tails" };
            format!("coin → {face}")
        }
        RollSpec::Oracle => {
            let answer = ORACLE_ANSWERS[rng.gen_range(0..ORACLE_ANSWERS.len())];
            format!("oracle → {answer}")
        }
    }
}

// ---------------------------------------------------------------------------
// POST /channels/{cid}/roll
// ---------------------------------------------------------------------------

/// POST /channels/{cid}/roll — parse a constrained dice expression, roll it
/// server-side, and persist the formatted result as an immutable
/// `kind='roll'` message authored by the caller (persona-aware like a normal
/// send). Non-members get the privacy-404; a bad expression is a 400.
#[tracing::instrument(skip_all, fields(account = %account.0, channel = %cid))]
pub async fn roll_message(
    State(state): State<AppState>,
    Path(cid): Path<String>,
    account: AuthAccount,
    payload: Result<Json<RollRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    // Validate the expression BEFORE touching the DB (cheap 400), mirroring
    // post_message's body-first validation order.
    let spec = match parse_roll_expr(&req.expr) {
        Ok(spec) => spec,
        Err(msg) => return error_response(StatusCode::BAD_REQUEST, msg),
    };

    let access = match channel_access(&state, &cid, &account.0).await {
        Ok(a) => a,
        Err(e) => {
            tracing::error!(error = %e, "channel_access failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    let stored_persona = match access {
        AccessOutcome::Ok(ctx) => {
            if ctx.kind != "text" {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "cannot post messages to a non-text channel",
                );
            }
            ctx.active_persona
        }
        AccessOutcome::ChannelNotFound | AccessOutcome::NotMember => {
            return error_response(StatusCode::NOT_FOUND, "channel not found");
        }
    };
    // MANDATORY persona double-check — the same shared path as post_message
    // (suggested first, stored wear second, both re-gated by can_edit_persona).
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

    // Server-authoritative outcome: rolled HERE, after every auth gate.
    let body = run_roll(&spec);

    // Same persist path as a normal send (persona snapshot included); a roll
    // carries no attachments, no reply, no pings, no effect.
    match persist_message(
        &state,
        &cid,
        &account.0,
        persona.as_deref(),
        &body,
        "roll",
        &[],
        None,
        &[],
        None,
    )
    .await
    {
        Ok(id) => {
            // A roll is a message like any other on the notify side: Web Push
            // to the guild's other members + the SSE bus (notify-and-fetch).
            crate::server::push::notify_new_message(state.clone(), id.clone(), account.0.clone());
            state.emit(crate::protocol::SyncEvent::MessageCreated {
                channel_id: cid.clone(),
            });
            (StatusCode::CREATED, Json(SendMessageResponse { id })).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "persist roll failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}
