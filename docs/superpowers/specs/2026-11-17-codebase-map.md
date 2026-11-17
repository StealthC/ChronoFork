# Codebase Map — ChronoFork

> Produced by `map-codebase` on 2026-11-17, before the roadmap brainstorm. It records the
> facts a new agent needs to start work without re-reading the whole tree. Canonical
> documentation is not duplicated here: crate purposes live in
> [AGENTS.md](../../../AGENTS.md#repository-map), architecture in
> [docs/architecture/](../../architecture/overview.md), and status in
> [docs/roadmap/milestones.md](../../roadmap/milestones.md).

**Date:** 2026-11-17
**Repository:** `StealthC/ChronoFork` @ `fa09130` (`main`)
**Confidence:** every claim below was checked against the code, the workspace manifests or
executed gates on this checkout.

## Tech stack and framework

- Rust stable, edition 2024, MSRV 1.85 (`Cargo.toml`, `rust-toolchain.toml`).
- Cargo workspace, `resolver = "3"`, 12 member crates, version `0.1.0`, MIT.
- No web framework, no async runtime, no ORM and no database. Runtime dependency: `sha2`.
  Test-only: `proptest`.
- `unsafe` is forbidden workspace-wide (`[workspace.lints.rust] unsafe_code = "forbid"`).
- Hand-rolled transport: HTTP/1.1 and MCP JSON-RPC subsets over `std::net`/`std::thread`.
- Persistence is a local filesystem store under an explicit root; no service to run.

## Directory layout

| Area | Contents |
|---|---|
| `crates/chronofork-core/` | IDs, virtual ticks, input frames, SHA-256 digests |
| `crates/chronofork-backend-api/` | Backend trait, `Capabilities`, versioned snapshot envelope |
| `crates/chronofork-sessions/` | Session lifecycle, durable storage, movies, rewind, rerecord |
| `crates/chronofork-{debug,trace,script}/` | Host observation layers above core + backend-api |
| `crates/chronofork-service/`, `chronofork-transport/` | Application service catalog and wire adapters |
| `crates/chronofork-gui/` | Presentation-only views/renderers/run loop |
| `crates/chronofork-{test-backend,megadrive}/` | Concrete backends (synthetic + minimal Mega Drive) |
| `crates/chronofork-cli/` | Composition root and every user-facing command |
| `docs/` | Durable documentation, ADRs, epics, requirements, roadmap and the planning lifecycle |
| `scripts/check.sh` | The local quality gate (CI mirrors it) |

## Entry points and how the application starts

- **Bin:** `crates/chronofork-cli/src/main.rs` (1073 lines) — argument parsing, command
  dispatch and process-level error reporting. It is the only binary.
- **Libraries:** each crate exposes `src/lib.rs`; `chronofork-cli` is the composition root
  that also selects the backend and wires the transport client.
- **Commands today:** `demo`, `megadrive`, `backend`, `session`, `tas`, `debug`, `trace`,
  `script`, `serve`, `api`, `mcp`, `experiment`, `gui`.
- **Startup shape:** no daemon is required for most work; `serve` runs a bounded HTTP/MCP
  daemon, and all state is passed explicitly (root path, token file, backend config).

## Test structure and framework

- 54 integration test files under `crates/*/tests/` plus unit tests inside `src/`.
- Framework: built-in `cargo test`; property tests use `proptest`; the CLI surface is tested
  by spawning the binary (`crates/chronofork-cli/tests/common/mod.rs`).
- Contract-focused suites: `chronofork-sessions/tests/{determinism,fork,isolation,rejection,
  snapshot_properties,ownership,movie,movie_v2,edit,rewind}.rs`; `chronofork-cli/tests/`
  covers process-level behavior for demos, TAS, debug, trace, script, GUI and M6 parity.
- **Baseline (2026-11-17, this checkout):** `./scripts/check.sh` exit 0; 988 tests passed,
  0 failed; rustdoc clean; the CI matrix (Linux, macOS, Windows, MSRV, release) is green on
  the merge commit.

## Key patterns and conventions

- Typed errors end to end; no stringly errors and no silent failure. `unwrap()` is not used in
  production code.
- Capabilities are reported honestly: `Capabilities` (`backend-api/src/descriptor.rs`) is the
  single source of truth for what a backend claims, and the CLI/tests pin it.
- Formats are versioned and reject-only: snapshot envelope version
  (`backend-api/src/snapshot.rs`), `CFMOVIE2` and `CFTRACE1` each validate before use and
  never migrate silently.
- Determinism is structural: emulated time comes from explicit virtual ticks, never host
  wall-clock input; demos must be byte-identical across runs.
- One writer per durable session (cooperative lock/lease); readers take no writer lock.
- No second source of truth: sessions own canonical state; service/transport/GUI are thin
  layers above it.

## Traps (read before changing)

- **Large, load-bearing files:** `chronofork-sessions/src/movie.rs` (3702 lines),
  `chronofork-megadrive/src/backend.rs` (2843), `chronofork-gui/src/view.rs` (2012),
  `chronofork-megadrive/src/cpu.rs` (1877), `chronofork-cli/src/main.rs` (1073). Each owns a
  format, a device model or the whole command surface — add beside them, never append blindly.
- **Change hotspots** (git history): documentation leads (`docs/README.md` 24 changes,
  `docs/roadmap/milestones.md` 22, `docs/requirements/functional.md` 21,
  `docs/quality/quality-gates.md` 19); the code hotspot is `chronofork-cli/src/main.rs` (18).
  Documentation churn is expected — keep status in its one home and link instead of copying.
- **Fix hotspots are negligible:** four files with a single historical fix each; there is no
  chronic bug area to route around.
- **Environment reads:** the only `std::env::var` is `host_label()` in
  `chronofork-sessions/src/storage/lock.rs`, which reads `HOSTNAME`/`COMPUTERNAME` for lock
  metadata and falls back to `None`. It never feeds emulated execution; keep it that way.
- **No TODO/FIXME/HACK markers** exist in `crates/` or `tests/` — pending work is tracked in
  the roadmap and issues, not in code comments.
- **Test baseline:** no failing test is present; any new failure is a regression to explain,
  not a pre-existing condition.
- **Critical files — read fully before changing:** `AGENTS.md`, `docs/planning/CONVENTIONS.md`,
  `.github/workflows/ci.yml`, `Cargo.toml`, `scripts/check.sh`, and the format owners above.

## Recommended starting points for the planned work (M8)

1. **Contract first:** `crates/chronofork-backend-api/src/{lib,descriptor,snapshot}.rs` — the
   second backend must satisfy these without changing them; an ADR is required first
   (phase 8.1, Issue #7).
2. **Use both existing backends as exemplars:** `chronofork-test-backend` is the minimal
   deterministic shape; `chronofork-megadrive` is the real-device shape (state, snapshots,
   capability report).
3. **Do not touch the upper layers to add a backend:** `sessions`, `service`, `transport` and
   `gui` must stay backend-agnostic; if a change seems required there, it is an architectural
   finding for phase 8.3, not an implementation shortcut.
4. **Pin every claim with a test** at the lowest useful layer and record the evidence under
   the ignored `.research/` tree; the gate (`./scripts/check.sh`) plus both demos are the
   minimum before calling work done.
