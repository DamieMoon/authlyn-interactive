//! Storage module (ssr graph): the SurrealDB data model, embedded at compile
//! time. The entire authoritative schema lives in `schema.surql` (21 SCHEMAFULL
//! tables, all `DEFINE`/backfill statements with their migration-hazard
//! rationale in `--` comments); this module just exposes it as a string. The
//! only consumer is [`crate::db::apply_schema`].

/// The whole SurrealQL schema, `include_str!`'d from `schema.surql` and applied
/// verbatim on boot. Statement order is load-bearing (idempotent backfills must
/// precede any row-revalidating UPDATE) — edit `schema.surql`, never this const.
pub const SCHEMA: &str = include_str!("schema.surql");
