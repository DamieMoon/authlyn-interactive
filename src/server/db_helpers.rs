//! Small shared SurrealDB row-projection helpers used across the server
//! handlers.
//!
//! These exist to deserialize a single `meta::id(id) AS id_key` column — the
//! one-field shape that `RETURN`/`SELECT` produce for existence checks and
//! "give me back the new record's key" writes. Each handler module previously
//! declared its own byte-identical local copy; this is the one definition they
//! all share.

use surrealdb::types::SurrealValue;

/// A row carrying only a record's bare key, projected via
/// `meta::id(id) AS id_key`. Used for existence probes (`Option<IdRow>` ->
/// `.is_some()`) and to read back the key of a freshly written row.
#[derive(SurrealValue)]
pub(crate) struct IdRow {
    pub id_key: String,
}
