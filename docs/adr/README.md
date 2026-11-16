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
| [0006](0006-movie-format-v2-anchors-and-multi-device-input.md) | Movie format v2 — checkpoint anchors and multi-device input | Accepted (owner-approved 2026-11-15); implemented and independently verified for the synthetic backend (t7 PASS) |
| [0007](0007-in-process-rewind-and-interactive-stepping.md) | In-process rewind state cache and interactive stepping | Accepted (owner-approved 2026-11-15); implemented and independently verified for the synthetic backend (t5 PASS) |
| [0008](0008-rerecord-and-branch-editing.md) | Rerecord and branch editing | **Accepted** (owner-approved 2026-11-15); implemented and independently verified for the synthetic backend (t10 round-2 PASS, 2026-11-16) |
| [0009](0009-real-hardware-input-tas.md) | Real-hardware input TAS — controller-port reads and Mega Drive backend selection | **Accepted** (owner-approved 2026-11-15T23:38:06-03:00); implemented and independently verified for the deterministic minimal-slice scope (t5 PASS 2026-11-16; M3 complete, t6 promotion, t7 audit pending) |
| [0010](0010-debugging-depth.md) | Debugging depth — breakpoints, watchpoints, views, disassembly and event observation | **Accepted** (owner-approved 2026-11-16 by the explicit M4 implementation directive); implemented and independently verified for the minimal slice (t5 PASS, 2026-11-16) |

## Template

Each ADR contains: context, problem, alternatives, decision, rationale,
consequences, status and evidence. An ADR that changes a major architecture,
license or infrastructure choice must be approved by the project owner before
implementation.
