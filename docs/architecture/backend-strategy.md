# Backend Strategy

## Status

**Accepted 2026-11-13.** The project owner approved Option E (staged hybrid) and
[ADR-0002](../adr/0002-backend-boundaries.md). Stage 1 (milestone M1) is complete and
produced its first slice: a minimal 68000 + ROM/RAM backend under
`crates/chronofork-megadrive`. The Exodus oracle path is now instantiated as an
external harness: the pinned headless oracle was built under ignored `.research/`
and the final differential evidence passes all eleven synthetic ROMs (projected
address streams, per-instruction 68000 cycle costs, selected final state and
negative controls); the earlier two-ROM run is preserved as pre-E2-15
controlled-condition history. See
[68000 cycle differential evidence](../research/cpu-cycle-differential-evidence.md).

## Requirements that discriminate between options

A viable real Mega Drive / Genesis backend must eventually provide:

1. Deterministic execution under identical ROM, configuration and input.
2. Complete snapshot/restore and fork into independent instances.
3. Multiple independent sessions without accidental shared mutable state.
4. Headless operation on Linux and native Windows.
5. CPU, memory and device inspection with a debugger path distinct from bus access.
6. A permissive license and reproducible provenance.
7. A path to differential testing against a trusted reference.
8. Long-term maintainability by a small team.

## Options

### A. Rust-native Mega Drive backend

Implement 68000, Z80, VDP, YM2612, PSG, buses and devices in Rust.

**Advantages:** full control over determinism and state layout; snapshots and
multi-instance are natural; one toolchain; no C/C++ ABI; easiest cross-platform
story; no upstream license entanglement.

**Disadvantages:** largest initial effort; high risk of subtle CPU/VDP/audio
incorrectness; debugging depth must be built from scratch; slow path to running
real software.

**Evidence fit:** best long-term fit for forkable execution, but no real hardware
yet.

### B. Adapter around Exodus / exodus-headless

Link or embed the MIT-licensed C++ core, exposing its devices through ChronoFork's
backend contract.

**Advantages:** mature 68000/Z80/VDP/audio implementation; debugging facilities,
device reflection, Active Disassembly and trace support; MIT license; headless
Linux port exists; cycle-equivalence methodology already documented.

**Disadvantages:** C++/CMake/clang/Ninja build; ABI and lifetime management; the
headless MCP server explicitly holds **one emulated system at a time**
(`.research/upstream/exodus-headless/AGENTS.md`), and its MCP surface has no
save/load-state tool. Multi-instance and snapshot support would require extending
the C++ port, not merely wrapping its MCP server. Windows and Linux builds must
both be maintained.

Deeper constraints found in
[the architecture research](../research/emulator-architecture.md):

- There is no stable C ABI or embedding library; the engine is C++17 virtual
  interfaces with `std::wstring`, RTTI and a monolithic `System`.
- The core is multi-threaded by design (one worker thread per active device,
  optimistic timeslices, conflict rollback, render/DMA/audio threads). Its rollback
  is in-flight conflict resolution, **not user-facing time travel**; there is no
  input log, movie or deterministic replay facility.
- Save states are full XML/ZIP snapshots with timestamps and no explicit version;
  they are not exposed over MCP, and cross-build compatibility is unverified.
- The daemon is single-session; multi-instance is structurally plausible by
  inspection but untested, with process-wide hazards (stdout ownership, audio,
  crash handling, CWD-relative paths).
- The 68000/Z80 cores are instruction-level, not bus-cycle-level; VDP/audio have
  documented accuracy limits.

**Evidence fit:** strongest immediate debugging depth, but its current headless
surface does not satisfy ChronoFork's snapshot/multi-instance core. It is a strong
**oracle**, not the production execution model.

### C. Process-isolated worker per session

Run an unmodified or lightly extended emulator process per session, with a typed
worker protocol.

**Advantages:** crash containment and hard isolation; works with a one-system-at-a-
time emulator; can be introduced without linking C++; maps well to future remote
backends.

**Disadvantages:** process startup and memory cost per session; snapshot/fork still
requires the worker to serialize state; if the worker only exposes MCP without
state transfer, process-level snapshot cannot be correct or portable; IPC
complexity and latency; Windows process management.

**Evidence fit:** good isolation boundary, but not a solution to the missing
save-state surface by itself.

### D. External emulation service

Treat a remote or separately versioned emulator as a service.

**Advantages:** language and process isolation; can host multiple backends.

**Disadvantages:** network dependency, latency, authentication, deployment and
determinism concerns; premature for a local-first bootstrap.

### E. Staged hybrid (accepted)

Keep the backend-neutral host (done). Use an **external Exodus headless reference
harness** only for differential traces and real-ROM validation, not as the
production backend. Implement a Rust-native Mega Drive backend as the production
path, validated against the reference harness. Revisit a C++ adapter only if the
reference harness proves multi-instance and state-export guarantees.

**Advantages:** gets real trace oracles early; preserves ChronoFork's snapshot and
determinism control; avoids making a hard-to-reverse wrapper the product core;
keeps licensing simple.

**Disadvantages:** two codebases to understand; the reference harness build is
pinned and recorded but not yet bit-reproduced; multi-instance/state-export
guarantees are unproven; Rust-native core is still a large effort.

## Comparison

| Criterion | A Rust-native | B C++ adapter | C Process worker | D Service | E Staged hybrid |
|---|---|---|---|---|---|
| Initial effort | Very high | Medium | Medium–high | High | High |
| Correctness risk | High initially | Lower | Lower | Lower | Medium |
| Debugging depth now | None | High | High (if exposed) | High | High (oracle) |
| Snapshot control | Native | Requires C++ work | Requires worker state export | Depends on service | Native in production |
| Multi-instance | Native | Unproven | Strong isolation, one system/process | Possible | Native |
| Portability | Strong | CMake/clang on Linux + Windows | Platform-specific IPC | Network | Strong |
| Licensing | Clean | MIT upstream, notices required | MIT upstream if Exodus | Varies | Clean |
| Long-term maintenance | One language | C++ + Rust + ABI | C++ + Rust + IPC | Service ops | Two languages |
| Fit to forkable core | Best | Weak until extended | Medium | Weak | Best |

## Recommendation

**Accepted: Option E (staged hybrid).**

1. Approve the backend-neutral host architecture (already implemented).
2. Build a **read-only differential harness** around exodus-headless in a separate
   worker, purely to capture traces from real ROMs. Do not make it the canonical
   session backend.
3. Implement the Rust-native Mega Drive backend incrementally, using the harness
   as an oracle and the synthetic backend as a contract test.
4. Re-evaluate Option B/C only if:
   - the Exodus headless build is reproducible on both Linux and Windows;
   - multiple system instances can be created in one process (or a worker protocol
     is accepted);
   - save/load state is exposed with a versioned, deterministic format;
   - license/provenance review remains clean.

This keeps ChronoFork's differentiator — independent forkable sessions — under its
own control. It also avoids committing the product to an emulator whose current
headless server is explicitly single-system and lacks a state-transfer surface.

## Blockers and evidence gaps

- The pinned headless port was built for E2-12 under ignored `.research/`, but its
  upstream equivalence gate was not rerun. Build reproducibility depends on the
  recorded toolchain and package versions.
- The 18,941-record equivalence claim in
  `.research/upstream/exodus-headless/tests/equivalence/README.md` is an upstream
  claim; this bootstrap did not rerun it.
- The MCP session test requires a real ROM, which is not in the repository.
- Multi-instance behavior of the underlying Exodus SDK (not just the MCP server) is
  still under investigation.
- BizHawk is not a reuse candidate for production because its license explicitly
  warns of incompatible GPL combinations and unclear third-party provenance.

## Decision recorded

Option E was approved on 2026-11-13 and recorded as
[ADR-0002](../adr/0002-backend-boundaries.md); the Status section above is
authoritative. This document keeps the comparison and the rationale for that
decision, including the option-by-option tradeoffs and the evidence gaps that
remain open. No further approval is pending, and any later change of strategy
requires a new or amended ADR rather than a new request here.
