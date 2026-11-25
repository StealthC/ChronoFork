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
| [0009](0009-real-hardware-input-tas.md) | Real-hardware input TAS — controller-port reads and Mega Drive backend selection | **Accepted** (owner-approved 2026-11-15T23:38:06-03:00); implemented and independently verified for the deterministic minimal-slice scope (t5 PASS 2026-11-16; M3 complete, t6 promotion, t7 promotion audit PASS 2026-11-16) |
| [0010](0010-debugging-depth.md) | Debugging depth — breakpoints, watchpoints, views, disassembly and event observation | **Accepted** (owner-approved 2026-11-16 by the explicit M4 implementation directive); implemented and independently verified for the minimal slice (t5 PASS, 2026-11-16) |
| [0011](0011-tracing-and-scripting.md) | Tracing and scripting — host trace surface and sandboxed script runtime | **Accepted** (owner-approved 2026-11-16 by the explicit Stage 5 implementation directive); implemented and independently verified for the documented minimal-slice scope (final T12 PASS, 2026-11-16; merged to `main` at `0c4e9f0`, PR #4) |
| [0012](0012-agent-native-interfaces.md) | Agent-native interfaces — one service, HTTP and MCP adapters | **Accepted** (owner-approved 2026-11-17 by the explicit Stage 6 implementation directive); implemented and independently verified on `agent/m6-agent-interfaces` at base `93d47ed` (pass-2 PASS 2026-11-17 after the M6-R1–R10 repair; round-2 review pass). M6 is `Complete` and merged to `main` at `cb05836` (PR #6) |
| [0013](0013-human-facing-gui.md) | Human-facing GUI — presentation over canonical views, not a state owner | **Accepted and implemented** (owner-approved 2026-11-17 by the explicit Stage 7 implementation directive; implemented and independently verified 2026-11-17 on `agent/m7-gui` at base `93d47ed` — final PASS4 and review REVIEW5 PASS). M7 is `Complete` and merged to `main` at `cb05836` (PR #6) |
| [0014](0014-v1-scope-single-user-loopback.md) | v1 scope — local, single-user, loopback, interactive MCP | **Accepted** (owner direction recorded 2026-11-20; R2/R3 corrections and the owner decisions applied; independent review V4 passed 2026-11-20). Acceptance is the scope decision, not implementation verification |
| [0015](0015-verification-contract.md) | Execution and verification are separate responsibilities | **Accepted** (owner direction recorded 2026-11-20; R1 C1–C10 and R4 N1–N11/G1–G6 corrections applied; independent review V4 passed 2026-11-20). Acceptance is the decision, not implementation or benchmark verification |
| [0016](0016-session-service-and-agent-api.md) | Session service — per-session execution and an agent-usable API | **Accepted** (owner direction recorded 2026-11-20; R2 F1–F10, R5 K1–K16 corrections applied; independent review V5 passed 2026-11-20). Acceptance is the service/API decision, not implementation or security verification |
| [0017](0017-vdp-read-cache-and-payload-v8.md) | VDP read cache (`0x0C` 8-bit VRAM read) and payload v8 | **Accepted** (owner direction recorded 2026-11-24: "autorizo você a seguir com o plano recomendado", authorising Option B — the faithful read cache with `CFVDP003` and payload v8). Implemented and verified in the `agent/0c-vram-read-v8` worktree; acceptance is the design decision, not a physical-hardware certification |
| [0018](0018-ym2612-global-selector-and-payload-v9.md) | YM2612 global bank+register selector and payload v9 | **Accepted** (owner acceptance recorded verbatim 2026-11-25: "Aceito o ADR-0018 como está (Recomendado)"). Acceptance and implementation authorisation, not merge authorisation. Implementation status is recorded in the ADR |

## Template

Each ADR contains: context, problem, alternatives, decision, rationale,
consequences, status and evidence. An ADR that changes a major architecture,
license or infrastructure choice must be approved by the project owner before
implementation.
