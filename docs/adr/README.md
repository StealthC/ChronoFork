# Architecture Decision Records

ADRs capture decisions with long-term consequences. They are durable project
documentation, not meeting notes.

## Status vocabulary

- **Proposed** — under discussion; not approved; do not implement as settled.
- **Accepted** — approved and in effect.
- **Rejected** — considered and not adopted.
- **Superseded** — replaced by a later ADR; link the replacement.
- **Deprecated** — still in effect but should not be used for new work.

## Index

| ADR | Title | Status |
|---|---|---|
| [0001](0001-rust-workspace.md) | Rust workspace and crate boundaries | Accepted |
| [0002](0002-backend-boundaries.md) | Backend strategy and boundaries | Accepted |
| [0003](0003-durable-session-storage.md) | Durable session storage, ownership and metadata | Accepted |
| [0004](0004-tas-movie-format.md) | TAS movie format and deterministic replay | Accepted; implemented and verified for the synthetic backend |
| [0005](0005-partial-replay-and-branch-comparison.md) | Partial replay, seek and branch comparison | Accepted; implemented and independently verified for the synthetic backend |

## Template

Each ADR contains: context, problem, alternatives, decision, rationale,
consequences, status and evidence. An ADR that changes a major architecture,
license or infrastructure choice must be approved by the project owner before
implementation.
