//! Wire-format helper: convert raw SurrealDB `datetime` columns to a
//! fixed-precision RFC 3339 string for inclusion in the JSON wire DTOs
//! (`MessageEnvelope.sent_at`, `InboxEnvelope.created_at`).
//!
//! ## Why this exists
//!
//! Both `server::messages::load_messages` and `server::keyshare::drain` SELECT
//! a `datetime` column and surface it on the wire as a `String`. The historic
//! convention — projecting the column via `<string>col AS col` so
//! `#[derive(SurrealValue)]` could read it as a `String` directly — turned out
//! to be load-bearing-and-wrong for ordering. SurrealDB's `Datetime::Display`
//! routes through chrono's `to_rfc3339_opts(SecondsFormat::AutoSi, true)`,
//! which emits VARIABLE-LENGTH sub-second suffixes per row:
//!
//! | Sub-second value     | Suffix shape       |
//! |----------------------|--------------------|
//! | `0`                  | `Z`                |
//! | millis-aligned       | `.NNNZ`            |
//! | micros-aligned       | `.NNNNNNZ`         |
//! | nanos                | `.NNNNNNNNNZ`      |
//!
//! ASCII collation orders `.` (46) < digit (48-57) < `Z` (90), so two rows
//! that are chronologically adjacent but sit on opposite sides of a
//! format-class boundary lex-mis-order (`"…12:00:00.123Z" < "…12:00:00Z"`).
//! SurrealDB's `ORDER BY` over the lex-projected alias inherits that flip.
//! Empirically: 1–10 misordered rows per 100-message page on the Pi 4
//! (10–100 ns clock tick).
//!
//! The fix at the SQL layer is to project the raw `datetime` column and
//! `ORDER BY` it under SurrealDB's native datetime semantics. The row
//! struct receives a `surrealdb::types::Datetime`. This helper is the
//! single Rust-side conversion point so the format string can't drift
//! between `messages.rs` and `keyshare.rs`.
//!
//! ## Format choice: fixed 9-digit (`SecondsFormat::Nanos`)
//!
//! Chronologically-ascending values produced by this function are also
//! lex-ascending: the suffix is always exactly 9 digits plus the trailing
//! `Z`, so digit-by-digit string compare matches numeric compare. That
//! keeps any downstream client-side ordering safe even without re-deriving
//! the datetime parse on every row.
//!
//! ## Wire-shape change vs the pre-fix output
//!
//! Pre-fix output was the variable-precision `AutoSi` shape (`12:00:00Z`,
//! `12:00:00.500Z`, …). Post-fix output is uniformly the 9-digit shape
//! (`12:00:00.000000000Z`). Both are valid RFC 3339, both round-trip
//! through `DateTime::parse_from_rfc3339` and through SurrealDB's
//! `type::datetime($s)` parameter cast — so existing cursor inputs from
//! pre-fix clients still resolve correctly. No protocol-version bump
//! required.

use chrono::SecondsFormat;
use surrealdb::types::Datetime;

/// Format a SurrealDB `Datetime` row column as a fixed 9-digit
/// sub-second RFC 3339 string for the JSON wire DTO.
pub(crate) fn to_rfc3339_fixed(dt: Datetime) -> String {
    // `Datetime` derefs to `chrono::DateTime<Utc>` — `to_rfc3339_opts` is
    // chrono's method, not SurrealDB's.
    dt.to_rfc3339_opts(SecondsFormat::Nanos, true)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    /// Every input shape (zero nanos, millis, micros, nanos) normalises
    /// to the same 9-digit suffix.
    #[test]
    fn format_class_boundaries_collapse_to_fixed_9_digit() {
        let cases = [
            ("2026-05-22T12:00:00Z", "2026-05-22T12:00:00.000000000Z"),
            ("2026-05-22T12:00:00.123Z", "2026-05-22T12:00:00.123000000Z"),
            (
                "2026-05-22T12:00:00.123456Z",
                "2026-05-22T12:00:00.123456000Z",
            ),
            (
                "2026-05-22T12:00:00.123456789Z",
                "2026-05-22T12:00:00.123456789Z",
            ),
        ];
        for (input, expected) in cases {
            let dt = Datetime::from_str(input).expect("parse input");
            let out = to_rfc3339_fixed(dt);
            assert_eq!(out, expected, "input={input}");
        }
    }

    /// Chronological order matches lex order on the formatted output:
    /// the invariant the variable-precision `AutoSi` output broke.
    #[test]
    fn chronological_order_matches_lex_order_post_fix() {
        let chrono_order = [
            "2026-05-22T12:00:00Z",
            "2026-05-22T12:00:00.123Z",
            "2026-05-22T12:00:00.123456Z",
            "2026-05-22T12:00:00.123456789Z",
            "2026-05-22T12:00:00.999999999Z",
        ];
        let formatted: Vec<String> = chrono_order
            .iter()
            .map(|s| to_rfc3339_fixed(Datetime::from_str(s).expect("parse")))
            .collect();
        let mut lex_sorted = formatted.clone();
        lex_sorted.sort();
        assert_eq!(
            formatted, lex_sorted,
            "post-fix output must be lex-monotonic under chronological seeding"
        );
    }
}
