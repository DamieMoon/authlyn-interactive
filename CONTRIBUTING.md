# Contributing to authlyn-interactive

Thanks for your interest. Please read this before opening a pull request.

## Inbound license (required)

This project is **dual-licensed** — AGPL-3.0-or-later plus a commercial license from
the sole copyright holder (see [`LICENSING.md`](./LICENSING.md)). For the project to
remain dual-licensable, every contribution must be licensed to the copyright holder
under the terms of [`LICENSING.md` §3](./LICENSING.md#3-contributions-inbound).

In short, by contributing you grant the copyright holder a perpetual, irrevocable,
sublicensable **copyright license** — and a **patent license** that also runs to all
recipients of the Work — over your contribution, with the right to license it under the
AGPL **and** under commercial terms, and you represent that the contribution is your
original work (or that you hold the rights necessary to grant this). The full,
controlling text is §3 of `LICENSING.md` — read it.

## How to accept — sign off every commit

You accept the §3 inbound terms by signing off each commit:

```sh
git commit -s -m "your message"
```

This appends a trailer:

```
Signed-off-by: Your Name <your@email>
```

By adding that line you certify that you have the right to submit the contribution and
that you **accept the inbound license terms in [`LICENSING.md` §3](./LICENSING.md#3-contributions-inbound)**.
**Pull requests whose commits are not signed off are not merged.** If you contribute
through a channel without a commit (a patch, or a snippet in an issue or discussion),
your material is not incorporated until you state your acceptance of `LICENSING.md` §3
in writing or re-submit it as a signed-off commit.

A formal Contributor License Agreement (CLA) restating these terms is forthcoming; a CLA
you later sign ratifies the grant you make here.

If a contribution includes third-party material, conspicuously identify it and the
license it is under.

## Before you submit

- Format and lint: `cargo fmt --all`, then clippy on both graphs
  (`cargo clippy --features ssr --no-deps -- -D warnings` and
  `cargo clippy --features hydrate --target wasm32-unknown-unknown --no-deps -- -D warnings`).
- Tests need a live SurrealDB: `cargo test --features ssr` (see `README.md` / `CLAUDE.md`).
- Keep changes focused, and explain the invariant or behavior you touched.
