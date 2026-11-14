# Non-Functional Requirements

These requirements constrain how ChronoFork is built. They are not aspirational
marketing claims; each has a verification path.

## Determinism and reproducibility

| ID | Requirement | Verification |
|---|---|---|
| NFR-DET-001 | Host wall-clock time must not silently affect emulated hardware. | Code review; no clock API in core/backend contracts |
| NFR-DET-002 | Emulated time must be explicit and monotonic. | `Tick` type and overflow tests |
| NFR-DET-003 | Identical state + input sequence must produce identical state digests. | Determinism tests on the synthetic backend |
| NFR-DET-004 | Replay from a checkpoint must reproduce the branch digest. | Restore/replay acceptance test |
| NFR-DET-005 | Cross-platform determinism for a real backend must be measured, not assumed. | Differential trace tests pass for three synthetic ROMs (including the masked-ROM default-quantum negative gating control) against the pinned oracle; the earlier two-ROM run is preserved as pre-E2-15 controlled-condition history; cross-platform determinism is still unmeasured. |
| NFR-DET-006 | Nondeterminism must be documented and must not be advertised as deterministic replay. | Capability report + review |

## Isolation and correctness

| ID | Requirement | Verification |
|---|---|---|
| NFR-ISO-001 | One mutable backend instance per session. | Type design + acceptance tests |
| NFR-ISO-002 | Fork must not share mutable state with its source. | Source/branch isolation tests |
| NFR-ISO-003 | Memory writes in one session must not affect another. | Memory isolation test |
| NFR-ISO-004 | A failed restore must not leave partially corrupted state. | Backend contract + backend-specific tests (future real backend) |
| NFR-ISO-005 | Snapshot compatibility failures must be explicit typed errors. | Corruption/version tests |

## Durability and durable storage (accepted design; not implemented)

> Status: design accepted by [ADR-0003](../adr/0003-durable-session-storage.md). No
> durable storage, metadata or rehydration path exists in the current tree. Each
> verification below is a planned Fase B gate, not a recorded result.

| ID | Requirement | Verification |
|---|---|---|
| NFR-DUR-001 | A committed checkpoint must restore after a process restart with the same tick and state hash. | Planned: capture, restart, reopen and compare tick/state hash (Fase B) |
| NFR-DUR-002 | Publishing a snapshot must be atomic: a crash must never expose a partially written snapshot as committed. | Planned: fault injection at each tmp+fsync+rename step; assert staging is invisible and no partial object loads (Fase B) |
| NFR-DUR-003 | Checkpoint integrity must cover envelope magic/version/checksum and the metadata binding; corruption, truncation or a swapped file must be rejected with a typed error. | Planned: corruption, truncation, checksum-swap and metadata-mismatch tests (Fase B) |
| NFR-DUR-004 | Storage, metadata, outer-envelope and backend compatibility must be reject-only; no silent migration or reinterpretation. | Planned: compatibility-matrix tests for unknown/future versions (Fase B) |
| NFR-DUR-005 | Names, logical creation sequence and advisory host timestamps must never enter a checkpoint's bytes or state hash; host time must never order or validate snapshots. | Planned: metadata/timestamp variation test showing identical checkpoint bytes, state hash and replay result (Fase B) |
| NFR-STOR-001 | Storage paths must be confined to the canonicalized root; all managed components are fixed names or canonical numeric keys, and traversal and symlink following are rejected. | Planned: traversal, symlink and wrong-file-type tests (Fase B) |
| NFR-STOR-002 | Every file and listing read must be bounded; lengths are validated against limits before allocation. | Planned: oversized, growing and truncated input tests (Fase B) |
| NFR-STOR-003 | Retention, listing and deletion must be explicit; opening, capturing, closing and exit must not delete data automatically. | Planned: filesystem inspection after each operation and after simulated restart (Fase B) |
| NFR-STOR-004 | Invalid, partial or orphaned objects must be reported as typed anomalies, not repaired, migrated or silently skipped. | Planned: staging/tombstone/corrupt-object recovery tests (Fase B) |

## Ownership and concurrency (accepted design; not implemented)

> Status: design accepted by [ADR-0003](../adr/0003-durable-session-storage.md); not
> implemented.

| ID | Requirement | Verification |
|---|---|---|
| NFR-CONC-001 | One writer per durable session/checkpoint at a time; contention must be a typed error, with no silent retry and no hidden global lock. | Planned: two-writer contention test and code review of lock scope (Fase B) |
| NFR-CONC-002 | An expired or corrupt writer lock must never be stolen automatically; recovery requires an explicit owner-checked action. | Planned: stale/corrupt lock recovery tests, including owner mismatch (Fase B) |
| NFR-CONC-003 | Lock-free readers must observe a complete old object, a complete new object or a typed not-found result, never a torn file. | Planned: concurrent publish/read and delete/read tests (Fase B) |

## Portability

| ID | Requirement | Verification |
|---|---|---|
| NFR-PORT-001 | Core and sessions must build on Linux and native Windows. | Linux verified; Windows CI pending |
| NFR-PORT-002 | Avoid platform-specific path/process assumptions in generic crates. | Code review; `std`-only generic APIs |
| NFR-PORT-003 | Do not claim WSL success proves native Windows support. | Documentation + planned CI |
| NFR-PORT-004 | Support macOS where feasible. | Planned CI |

## Quality and maintainability

| ID | Requirement | Verification |
|---|---|---|
| NFR-QUAL-001 | `cargo fmt --check`, `cargo check`, Clippy `-D warnings`, tests must pass. | Local gate and CI |
| NFR-QUAL-002 | No `unsafe` without a dedicated, documented exception. | `unsafe_code = "forbid"` |
| NFR-QUAL-003 | Avoid `unwrap` in production paths. | `clippy::unwrap_used = "deny"` |
| NFR-QUAL-004 | Public APIs have explicit error types, not stringly errors. | Review |
| NFR-QUAL-005 | New behavior requires automated tests and documented limitations. | Review + CI |
| NFR-QUAL-006 | Avoid unnecessary dependencies and abstraction layers. | Dependency review |
| NFR-QUAL-007 | Use versioned external formats and reject incompatible input. | Snapshot tests |

## Security and repository hygiene

| ID | Requirement | Verification |
|---|---|---|
| NFR-SEC-001 | Never commit ROMs, firmware, BIOS images, commercial assets or secrets. | `.gitignore`, pre-commit inspection |
| NFR-SEC-002 | Reference clones are ignored and not submodules. | `.research/` ignored; `git status` |
| NFR-SEC-003 | Do not run untrusted scripts with host privileges. | Future scripting sandbox design |
| NFR-SEC-004 | Do not silently upload session data or telemetry. | No network code in bootstrap |
| NFR-SEC-005 | Preserve license notices in distributions. | `THIRD_PARTY_NOTICES.md` |

## Documentation and agent usability

| ID | Requirement | Verification |
|---|---|---|
| NFR-DOC-001 | Official docs are navigable from `docs/README.md`. | Link audit |
| NFR-DOC-002 | `AGENTS.md` is harness-independent and actionable. | Review |
| NFR-DOC-003 | Durable decisions are recorded in ADRs, not only chat. | ADR index |
| NFR-DOC-004 | Unverified upstream claims are labelled. | Research reports |
| NFR-DOC-005 | Operational planning memory is ignored, not committed as docs. | `.gitignore` |

## Performance

No performance target is claimed at bootstrap. The synthetic backend is intentionally
small and is not a benchmark. Future requirements must define:

- session creation/fork latency;
- snapshot size and capture/restore time;
- step throughput for a real core;
- trace overhead and ring-buffer memory bounds;
- concurrent session limits.

Premature optimization is prohibited before correctness tests exist.
