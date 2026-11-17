# Project Conventions

> Written by `init-conventions`. Do not hand-edit — re-run the sub-skill instead; the Commit & Release Protocol reads these fields. One exception: the writing-plans `## Checks` rule may also fill `**Build:**` and `**Test:**`, which the protocol does not read. `ci-watch` also reads `**CI timeout:**`, which the protocol does not. Every other field goes through a re-run.

**Established:** 2026-11-17

## Stack

**Language / runtime:** Rust stable (edition 2024, toolchain pinned in `rust-toolchain.toml`)
**Package manager:** Cargo (workspace, resolver 3)
**Framework:** none
**Datastore:** local filesystem versioned store (no database)
**Build:** `cargo build --workspace`
**Test:** `cargo test --workspace`

## Commits

**Format:** conventional
**Scopes:** free
**Scope source:** n/a
**Fallback when scope not allowed:** omit scope

## Branching

**Model:** feature-branch
**PR required:** yes
**Protected branches:** main
**CI timeout:** 60 (defaulted)

## Versioning & Release

**Scheme:** none
**Released by:** none (defaulted)
**Milestone completion tags a release:** no
**Changelog:** none

## Deployment

**Deploy target:** none
**Environments:** none
**Deployed by:** none
