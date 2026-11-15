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

## Durability and durable storage (implemented; Linux acceptance platform)

> Status: implemented in `chronofork-sessions` and independently verified in two
> rounds (2026-11-15). The first round found one medium, non-blocking CLI lock-scope
> finding (F1); the repair made the continue-and-save workflow hold the writer lock
> across read-validate-publish, and round 2 re-verified F1 closed with no regression.
> Durability is claimed only for local Linux filesystems whose directory `fsync` and
> rename behavior is known to be sound ([ADR-0003](../adr/0003-durable-session-storage.md)
> §4); Windows and network filesystems remain unverified. Raw evidence is stored under
> the ignored `.research/m2-durable-sessions/fase-b/verification/` paths (round 1 and
> `round2/`) and `.research/m2-durable-sessions/fase-b-repair-f1/`; the durable record
> is the [M2 acceptance plan](../quality/m2-durable-sessions-acceptance.md) and
> [quality gates](../quality/quality-gates.md).

| ID | Requirement | Verification |
|---|---|---|
| NFR-DUR-001 | A committed checkpoint must restore after a process restart with the same tick and state hash. | Verified 2026-11-15 (round 2): a separate CLI process saves, exits, and a second process loads the committed session/snapshot and reports the same tick and state hash; reopen and determinism targets pass. Residual: Linux-local filesystem only. |
| NFR-DUR-002 | Publishing a snapshot must be atomic: a crash must never expose a partially written snapshot as committed. | Verified: permission-fault injection around staging, rename and parent sync; staging is never loaded, a failed publish never advances visible state, and a post-rename sync failure returns typed `DurabilityUnconfirmed` with the lock released. Scope: injected filesystem faults, not power loss; Windows fsync unverified. |
| NFR-DUR-003 | Checkpoint integrity must cover envelope magic/version/checksum and the metadata binding; corruption, truncation or a swapped file must be rejected with a typed error. | Verified: rejection target covers magic/version/checksum, truncation, trailing bytes, swapped files, mirrored-field mismatch and identity mismatch; CLI corrupt-metadata and tampered-checkpoint cases exit 1 typed while list reports anomalies without repair. |
| NFR-DUR-004 | Storage, metadata, outer-envelope and backend compatibility must be reject-only; no silent migration or reinterpretation. | Verified: unknown/future storage layout, metadata record version, outer envelope and backend identity/version/config hash are rejected with typed errors; no migration, repair or fallback path exists. |
| NFR-DUR-005 | Names, logical creation sequence and advisory host timestamps must never enter a checkpoint's bytes or state hash; host time must never order or validate snapshots. | Verified: metadata-only rename leaves checkpoint bytes and state hash unchanged; roots created at different host times with identical inputs produce byte-identical checkpoints and state hashes; `created_at` appears only in metadata and never in ordering or validation. |
| NFR-STOR-001 | Storage paths must be confined to the canonicalized root; all managed components are fixed names or canonical numeric keys, and traversal and symlink following are rejected. | Verified: traversal components, non-canonical keys, managed symlinks and wrong file types are rejected, and a hostile tree cannot escape the root. Session/snapshot names are labels that never become path components: `../…` and `a/b` names are accepted as labels and create nothing outside the root, while only control-character and over-long names return `InvalidName` (`confinement.rs`, A6). Residual: checks are path-based and not race-free against a hostile process concurrently replacing a component. |
| NFR-STOR-002 | Every file and listing read must be bounded; lengths are validated against limits before allocation. | Verified: oversized, growing and truncated inputs are rejected; lengths are checked before allocation and reads use a bounded `take(limit+1)`; list caps, name bounds and lock-record bounds are enforced. |
| NFR-STOR-003 | Retention, listing and deletion must be explicit; opening, capturing, closing and exit must not delete data automatically. | Verified: open/capture/close/restart delete nothing; listing is deterministic; delete requires an explicit target, a session with snapshots is rejected non-cascading, and no automatic garbage collection exists. |
| NFR-STOR-004 | Invalid, partial or orphaned objects must be reported as typed anomalies, not repaired, migrated or silently skipped. | Verified: staging, tombstones, corrupt records and leftover lock-protocol records are typed anomalies; they are never loaded, resurrected, repaired or reclaimed automatically. |

## Ownership and concurrency (implemented; cooperative lock)

> Status: implemented and verified (round 2, 2026-11-15). The per-session writer lock
> is cooperative metadata, not authentication. The supported read-modify-publish
> workflow (`with_writer_lock` plus `save_snapshot_with_lock`) holds one lock across
> the read, validate and publish sequence; the CLI continue-and-save uses it. The
> public convenience `save_snapshot` still acquires its own lock only at publish time,
> so a third-party caller that reads a base first can reproduce the old stale-base
> window. A crashed holder needs the explicit owner/generation-checked recovery, and a
> long in-memory `--steps` advance lengthens contention by design. `NFR-CONC-003`
> remains **Partial**: concurrent publish/read and delete/read interleavings are not
> separately tested; the verified scope and residuals are in that row.

| ID | Requirement | Verification |
|---|---|---|
| NFR-CONC-001 | One writer per durable session/checkpoint at a time; contention must be a typed error, with no silent retry and no hidden global lock. | Verified: ownership target asserts typed `WriterContended` naming the owner for a second writer, including empty/partial lock records; a real cross-process holder makes a CLI `save --session` exit 1 typed before reading a base; real two-process CLI races produced one winner and one typed contention with no stale-base double publish after the F1 repair. The root allocation lock is short-lived and root-scoped; the only atomic is a process-local token nonce, not a lock. Residual: cooperative lock and the publish-only third-party `save_snapshot` caveat above. |
| NFR-CONC-002 | An expired or corrupt writer lock must never be stolen automatically; recovery requires an explicit owner-checked action. | Verified: expired leases are not auto-stolen; forced recovery requires the exact owner and generation; a changed successor returns `NotLockOwner` and is preserved; explicit recovery of an unreadable record returns `CorruptLock` and removes nothing. A crashed holder leaves a stale lock that needs the same explicit recovery. |
| NFR-CONC-003 | Lock-free readers must observe a complete old object, a complete new object or a typed not-found result, never a torn file. | **Partial** (concurrent interleavings not yet tested): readers take no writer lock. With a writer lock held but no publish in progress, the lock-free reader loads the complete old canonical bytes and deterministic list output (`ownership.rs::lock_free_reader_never_observes_a_torn_checkpoint`), and independent readers are side-effect-free (`isolation.rs::independent_readers_are_side_effect_free`). Unknown keys return typed not-found (`rejection.rs`), and sequential delete-then-list tests show the removed object absent while other sessions are unchanged (`isolation.rs`, `retention.rs`). A concurrent publish/read or delete/read interleaving is not exercised by any test — the real race tests are writer-vs-writer; those clauses rest on the immutable published object and the atomic visibility-removing rename (ADR-0003 §4/§9) and remain unverified. NFR-CONC-003 is not an M2-A exit criterion; M2 completion rests on the executed M2-A rows. |


## Portability

| ID | Requirement | Verification |
|---|---|---|
| NFR-PORT-001 | Core and sessions must build on Linux and native Windows. | Linux runtime verified; Windows/macOS compile-checked in CI; native Windows runtime pending |
| NFR-PORT-002 | Avoid platform-specific path/process assumptions in generic crates. | Code review; `std`-only generic APIs |
| NFR-PORT-003 | Do not claim WSL success proves native Windows support. | Documentation; Windows compile check only (no runtime evidence) |
| NFR-PORT-004 | Support macOS where feasible. | macOS compile check in CI; runtime pending |

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
