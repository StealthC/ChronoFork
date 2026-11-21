# ADR-0014: v1 scope — local, single-user, loopback, interactive MCP

- **Status:** **Accepted** (owner direction recorded 2026-11-20 in the independent-review
  questionnaire; R2/R3 and the adversarial critique C1 corrections applied 2026-11-20;
  independent review V4 passed 2026-11-20). This accepts the v1 scope decision; it does not
  certify implementation, boot, security or M8 closure.
- **Date:** 2026-11-20.
- **Milestone:** M8 (functional Mega Drive emulator) and the product-consolidation phase.
- **Related:** [ADR-0012](0012-agent-native-interfaces.md) (service/transport boundary; §5
  concurrency is **superseded in part** by [ADR-0016](0016-session-service-and-agent-api.md)),
  [ADR-0013](0013-human-facing-gui.md) (presentation only; its native-toolkit follow-up is
  deferred with 8.8),
  [ADR-0015](0015-verification-contract.md) (execution/verification split),
  [ADR-0016](0016-session-service-and-agent-api.md) (service and agent API).
- **Review evidence:** independent review 2026-11-20, baseline `b8d63a1`. The durable report
  exists in this tree at
  [`docs/reviews/2026-11-20-independent-review/REVIEW.md`](../reviews/2026-11-20-independent-review/REVIEW.md)
  (with `areas/`, `critique-final.md`, `SYNTHESIS-DRAFT.md`, `baseline.md` and `benchmarks/`).
  Consolidation review: `.research/consolidation-2026-11-20/reviews/`
  (`adr-0014-roadmap.md`, `adr-0015-rust.md`, `adr-0015-perf.md`,
  `adr-0016-interfaces.md`, `adr-0016-concurrency.md`,
  `critique-consolidation.md`), the V2 review
  (`review-corrected-adrs.md`) and the promotion task W0.4.

## Context

ChronoFork describes itself as a platform for humans, scripts and AI agents. The current
service already exposes 16 operations over a transport-agnostic catalog and bounded HTTP/1.1
and MCP adapters (M6). The independent review confirmed, with code and runtime evidence, that:

- the MCP surface has no debug, trace, script, TAS or memory-write operations, so an agent
  cannot conduct interactive reverse engineering over MCP (M6-FU1);
- the service holds one global `Mutex<SessionManager>` for the whole step loop; independent
  sessions do not execute in parallel and a long `run` blocks other sessions' reads and
  creates (`crates/chronofork-service/src/service.rs:114-122`, `:461-491`);
- the transport has no TLS, no non-loopback authentication model, static tokens and no
  durable per-principal ownership across restarts (M6-FU2/FU3);
- a slowloris-style pre-authentication connection exhaustion is demonstrable with the
  current worker pool;
- the **wire implementation is MCP `2025-03-26`** (`crates/chronofork-transport/src/mcp.rs:29`;
  the string appears in tests and CLI help), while the current specification is newer
  (`2026-07-28` per the review's research). ADR-0016 carries the migration as post-M8; the
  deferred table below is the single list.

The review also established that the value path is blocked by correctness, not by a lack of
multi-user infrastructure: no curated commercial ROM progresses beyond bootstrap on the
baseline. Adding TLS, multi-user ownership or a browser-facing deployment now would expand
the attack surface and the schedule without unblocking the product.

## Problem

What is the public scope of v1 so that the project can deliver real value without claiming
security or concurrency guarantees it does not provide?

## Decision

1. **v1 is local and single-user.** The supported deployment is a human or an agent on the
   same host, using the CLI, the process-local service or a loopback HTTP/MCP endpoint.
   Loopback is the only supported bind address for v1.
   **Owner decision (2026-11-20): a non-loopback bind refuses to start**, with a typed
   error and a test. Today `chronofork serve --bind` accepts any address and only warns on
   stderr (`crates/chronofork-cli/src/agent/serve.rs:90-94`); that behavior is not the
   contract. A future `--unsafe-bind` (or any exposed deployment) requires its own ADR with
   TLS and an authorization model.
2. **One writer per session; no multi-user ownership promise.** Tokens remain static and
   process-local. Durable objects have no per-principal owner. The documentation must say so
   in the capability/limitations home rather than implying enterprise-grade isolation.
   **File security (R2 correction 9):** a token file with group/other-readable permissions is
   refused (or a typed warning is emitted before serving), and the store/artifact roots are
   created `0700` with files `0600`.
3. **The MCP surface is interactive and minimal, not a mirror of the CLI.** v1 adds the
   agent operations needed for the KidEngine end-to-end flow (run with stop conditions,
   memory read/write/diff/search, disassembly, trace, debug, frame capture, live-session
   save/load, fork/compare, capabilities/cancel/progress). The exact operation set is
   [ADR-0016](0016-session-service-and-agent-api.md); it is expected to be roughly nine
   composed operations, not the ~30 CLI verbs.
4. **No TLS, no SSE/streaming, no non-loopback deployment, no multi-node in v1.** A future
   ADR may add them. Until then, the product must not advertise them.
5. **Protocol hygiene now, protocol migration later.** `Origin` validation and pre-auth
   timeouts are in scope now. `Origin` matters because a loopback listener is still reachable
   by any local process and by a browser (DNS-rebinding class); the policy is an explicit
   allowlist of trusted origins (or no `Origin`-bearing request accepted at all when the
   surface does not need a browser), plus `Host`/loopback checks and tests — **not** "accept
   any `localhost:<port>`", which authenticates nothing. A pre-auth connection deadline and a
   per-peer pre-auth connection cap are required: pre-auth slowloris is a demonstrated
   availability defect (review A6-F4), not a theoretical hardening item. The MCP protocol
   version upgrade is scheduled after M8 because it changes the wire contract and must not
   block the emulator.
6. **The v1 security statement is a cooperative boundary, not a hostile-code sandbox.**
   Scripting remains a deterministic capability boundary; it is not OS/process isolation.
   This is existing policy and is restated here so that "agent-native" is not misread as a
   security guarantee.

### Explicitly out of scope for v1 (deferred, not silently absent)

One list, and the canonical home for status/follow-ups stays
[`docs/roadmap/milestones.md`](../roadmap/milestones.md) (linked, not copied):

| Deferred | Note |
|---|---|
| TLS, multi-user/durable principals, token rotation/expiry/audit log | Requires its own ADR; see ADR-0016 §5 |
| Any non-loopback bind, multi-node | Owner decision above: refuse to start |
| SSE/streaming, keep-alive, chunked bodies | Bounded HTTP/1.1 subset only |
| MCP protocol `2026-07-28` migration | Post-M8; the wire today is `2025-03-26` (`chronofork-transport/src/mcp.rs:29`) |
| Phase 8.7 (execution modes/capture) and 8.8 (native window) | **Maintained in the open M8** (owner decision O-2, 2026-11-20); the consolidation track does not amend the DoD and does not close the milestone |
| M9 (multi-backend) | After M8; prepare the contract, do not implement |
| async/Tokio, actors, process-per-session | Per-session owner is the smallest correct step (ADR-0016 §1) |
| Incremental/dirty hash, COW/dedup/persistent structures | Need usage evidence; `Arc` + `reap()` cover the immediate case (ADR-0015 §6) |
| OS keyring for the static token | Packaging concern; keep the file for v1 |
| `script.run` and `tas.record` over transport | **Not exposed in v1.** The M6-FU1 follow-up stays CLI-local until a later decision that also covers the script deadline gate; this supersedes the earlier "exposed per ADR-0016 or listed as deferred" wording (findings R2-F9/R3-F9/R3-F12) |

## Alternatives considered

| Alternative | Why not now |
|---|---|
| Multi-agent, multi-user and TLS in v1 | Requires per-session ownership, quotas, durable principals, token rotation and TLS; 2–3 additional phases and a security review. It does not unblock boot or the KidEngine flow. |
| CLI-only, no MCP | Less platform work, but contradicts the product thesis and leaves agents with a one-shot surface that cannot attach to a live session. |
| Full MCP mirror of every CLI verb (~30 operations) | Larger, less composable surface; more tokens and maintenance; does not solve batching, cancellation or quotas. |
| Non-loopback bind with static tokens | Unsafe by construction; would require TLS and a real authorization model before it could be honest. |

## Consequences

### Positive

- The team can focus on boot correctness, the verification contract, the agent API and
  reproducibility instead of infrastructure that does not yet unlock value.
- Security claims stay honest: loopback, single-user, static tokens, process-local ownership.
- The MCP surface can be designed around the KidEngine flow rather than around the CLI's
  internal command list.

### Negative / risks

- The product cannot claim multi-agent concurrency until a later ADR; the service may still
  serialize *by design* during v1, but it must not claim parallelism it does not have.
  **ADR-0016 §1 supersedes ADR-0012 §5's manager-lock step path**, and the "reads remain
  available" wording of `docs/adr/0012-agent-native-interfaces.md:110`,
  `docs/architecture/overview.md` and `docs/architecture/agent-interfaces.md` must be removed
  unless a measured bound backs it (finding R5-K11).
- A loopback-only product is less useful for remote/team deployments; that is an accepted
  v1 limitation.
- Static tokens and process-local ownership mean a restart loses live ownership; durable
  objects have no principal. This must be documented and tested as a limitation, not hidden.

### Compatibility and versioning

- No persisted format changes are required by this ADR.
- The service operation catalog will grow (ADR-0016); old operations remain valid unless a
  later ADR explicitly deprecates them.
- The MCP protocol version upgrade is deferred; the current version remains until the
  migration ADR/phase.

## Verification

- Non-loopback bind is **refused with a typed error before serving**, and a test asserts the
  refusal (not a warning, not "documented as unsupported").
- File security: a world/group-readable token file is refused (or a typed warning is emitted
  before serving) and the store/artifact roots are `0700`/`0600`, with a test.
- The capability/limitations documentation states: no TLS, no multi-user ownership, no
  non-loopback, static tokens, process-local live sessions.
- The MCP surface exposes **only** the approved agent operations of ADR-0016. `script.run` and
  `tas.record` are **not** exposed in v1 (declared deferred in the table above); any other CLI
  surface absent from the catalog is listed there, so there is no silent absence.
- Slowloris pre-auth test: truncated connections cannot exhaust the worker pool; a legitimate
  `/health` request succeeds within the documented bound; the bound is expressed as a function
  of the configured `workers` count and a per-peer pre-auth connection cap.
- `Origin` policy test: the allowlist is explicit and a request with an untrusted/missing
  `Origin` on an `Origin`-bearing route is rejected deterministically.
- No test or demo claims multi-session parallelism until the per-session execution model is
  implemented and measured. Determinism tests for the execution modes (unpaced/paced/stepped)
  are **not** dropped: they remain part of W2/G-DET even though the capture surface (8.7) is
  delivered later.

## KidEngine flow and consolidation use cases

The v1 agent surface is judged by the KidEngine flow, not by a verb count. The UC numbering and
its acceptance are defined **once, here (normative)**; the work items and gates that execute
them live in the
[consolidation plan](../superpowers/plans/2026-11-20-consolidation-and-agent-api.md)
(W5.6, G-BOOT-1/2, G-API), and status lives in
[`docs/roadmap/milestones.md`](../roadmap/milestones.md). No other document redefines the UCs:

| ID | Flow (executable) | Proof |
|---|---|---|
| UC-1 | Boot smoke: load a **redistributable synthetic** ROM, run one frame, report PC/frame hash/VDP status | Committed synthetic fixture passes; a local commercial fixture is optional attached evidence and is **never** a CI pass-by-absence |
| UC-2 | Fork/compare: checkpoint → 2 forks → distinct input → run → compare → restore | Ticks/hashes and first divergent tick in the report |
| UC-3 | Interactive RE over MCP: run with `stop_on`, memory read/write/diff/search, disasm, trace, breakpoint, live-session save/load | Requires the minimal agent API (W3.4) **and** disassembler coverage for the instructions the CPU executes; 10k steps in one call **and** the frozen round-trip script |
| UC-4 | Reproducible report: the KidEngine scenario artifact with inputs, ticks, hashes and findings, re-executable | Byte-identical re-run (modulo declared non-deterministic fields) |

A hash of a frame or audio buffer proves repeatability, not a correct image or non-silent
audio: UC-1/UC-4 must attach at least one content-level predicate (see G-BOOT-2 in the
consolidation plan).

## Milestone consequence (owner decision 2026-11-20)

Consolidation (8.6c) is a **phase of the open M8**, not a milestone closing act. **M8 remains
open** and 8.7/8.8 are executed afterwards; the M8 Definition of Done is **not** amended.
Consequently:

- finishing 8.6c does not claim M8 completion and must not be reported as such;
- 8.7/8.8 stay in `docs/planning/ROADMAP.md`/`MILESTONE.md` and in the DoD
  (`docs/planning/MILESTONE.md:30-32`: cross-mode equality and PNG/WAV capture);
- if the owner ever wants to redefine the product/milestone instead, that is a separate act
  listing every affected requirement — it is not implied by this ADR.

## I/O and bus policy (owner decision 2026-11-20)

For the boot path the policy is **deterministic open bus for genuinely undecoded space, and a
typed error only where the hardware faults**, with the documented control registers mapped
first. Concretely:

- `0xA10008`/`09`, `0A`/`0B`, `0C`/`0D`, `0E`/`0F` are the **control** registers of ports
  1/2/3/EXT: readable/writable, reset value 0. Port 3/EXT **control** is a stub (0); port
  3/EXT **data** stays a typed error. This corrects the 8.5 plan, which still treats port 3
  as an error (`docs/superpowers/plans/2026-11-17-8.5-timers-controllers.md:119`) — the exact
  second address *Chakan* reads (`0xA1000C`) before falling into its fatal handler.
- The address-space normalization (24-bit truncation) and the RAM mirror
  `0xE00000..0xFFFFFF` stay bus concerns (W1.1/W1.2).
- The team resolves the per-range bus policy from hardware references and tests **inside the
  approved loopback/scope direction**; the owner's decision is the policy direction above,
  not the choice of register.

## Audio direction (owner decision 2026-11-20)

**Do not adopt `ymfm` now.** Complete and verify the project's own YM2612 (including its
timers/status/DAC), PSG and mixer/resampler already implemented on `agent/8.4-z80-audio`
(14 implementation commits plus uncommitted T4 work), with a differential/tolerance review of
the audio evidence. Commit count is not a quality argument: the differential decides whether
the implementation is good enough. An eventual synthesizer swap would replace the audio core
only — it would not discard the Z80/bus/PSG/mixer work. Any adoption of an external core
(`ymfm` BSD-3 or a future alternative) requires its own ADR with license, pin, build,
determinism and provenance, and is out of scope for this consolidation.

## Open questions

1. Should the v1 MCP endpoint be a separate `chronofork mcp` process from `chronofork serve`,
   or the same daemon with a loopback-only listener? (Implementation detail; ADR-0016.)
2. ~~What is the exact non-loopback failure mode?~~ **Decided (owner, 2026-11-20): refuse to
   start with a typed error and a test.**
3. Should the static token remain a file, or move to an OS keyring for v1? Recommendation:
   keep the file for v1; keyring is a packaging concern, not a blocker.
4. When an external audio core is reconsidered, which evidence (differential accuracy,
   state/snapshot determinism, maintenance, license) decides the swap? — post-M8.
