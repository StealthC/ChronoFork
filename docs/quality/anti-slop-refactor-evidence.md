# Anti-Slop Refactor Evidence

- **Status:** Baseline captured at clean commit `c3e92d8`. The integrated
  refactor was verified twice: §7 records the pre-repair PASS, which t13
  superseded; **§8 is the final, authoritative PASS for the post-repair bytes
  that will be committed** — gates, demos, serialized bytes and sizes are
  unchanged, and the intentional post-repair error-contract changes are recorded
  there.
- **Scope:** Behaviour-preservation evidence for the anti-slop refactor (shared
  snapshot codec, canonical state serializer, envelope validation,
  dead-code/API cleanup, typed errors, bounded reads, test deduplication,
  property tests and benchmarks).
- **Owner:** verifier, independently of the implementers; the after-refactor
  numbers in §7 and §8 were re-measured by the verifier, not copied from the
  integrator's artifacts.
- **Raw baseline artifacts:** `.research/anti-slop/baseline/` (git-ignored), with
  a SHA-256 manifest in `.research/anti-slop/baseline/MANIFEST.txt`.
- **Independent after-refactor artifacts:** `.research/anti-slop/verification/`
  (pre-repair) and `.research/anti-slop/post-repair-verification/` (final), both
  git-ignored, including the differential probe sources and outputs.
- **Related:** [anti-slop policy](anti-slop-policy.md),
  [AGENTS.md](../../AGENTS.md),
  [quality gates](quality-gates.md),
  [testing strategy](testing-strategy.md),
  [Mega Drive 68000 slice](../architecture/megadrive-slice.md).

## 1. What "state unchanged" means

The refactor is behaviour-preserving only if **all** of the following hold. These
are the acceptance conditions for the after-refactor comparison; a failure in any
one of them is a behavioural regression, not an acceptable cleanup side effect.

| ID | Requirement | How it is proven |
|---|---|---|
| S1 | **Emulator and session semantics are identical.** For the same input, every observable event, tick, CPU register/PC/SR value, VDP port value, interrupt delivery/retention decision, capability value, fork/lineage result and pass/fail outcome is unchanged. | Both demos print the full semantic trace (ticks, vectors, masks, registers) and must match byte for byte. The 152-test suite covers snapshot round-trip, fork isolation, replay, memory isolation, interrupt gating and determinism. |
| S2 | **Serialized state bytes are identical.** For a fixed state, the canonical snapshot/checkpoint serialization produces the same bytes, the same length and therefore the same state hash. | The synthetic demo prints the checkpoint state hash and the serialized length (`459 bytes, checksum verified`). Because the state hash is computed from the serialized state (AS-03), equal hashes plus equal lengths are the primary proxy. The after-refactor evidence must additionally include either a fixed-state serialized-byte golden test or a differential test that decodes/re-encodes baseline-produced bytes and gets identical bytes back. |
| S3 | **CLI demo output is byte-identical.** Both demo commands exit 0 and write exactly the baseline stdout bytes. | `cmp` against `.research/anti-slop/baseline/*.stdout.txt`; SHA-256 listed in §4. Only stdout is normative. Cargo's stderr build lines are not normative because they contain cache/timing text. |
| S4 | **The quality gates still pass, and no behaviour-bearing test is lost.** All five gates exit 0; scenario coverage is preserved. Test deduplication may legitimately change the *count*, and added property tests may raise it, but any change must be explained and no assertion covering S1 may disappear without an equivalent assertion elsewhere. | Gate exit codes recorded in §3; per-target test inventory compared and explained. |

What is **not** covered by "state unchanged" (these may change intentionally):

- **Public API surface.** The refactor deliberately removes dead code and
  uncalled `pub` items. That is a separate, intentional change: it is
  source-compatible inside this repository because nothing calls those items, but
  external consumers of such items would no longer compile. It is explicitly
  outside the state-equality claim and must never be used to excuse a change in
  S1, S2 or S3.
- Internal module layout, private helper names, ownership of helpers, and which
  crate-internal file a function lives in.
- Typed-error representation (variant names, `Display` wording) provided the
  classification of failures and the accept/reject decisions at each boundary are
  unchanged.
- Test file organisation and test names; added property tests and benchmarks.
- Documentation and policy prose, including the rewritten anti-slop policy.
- Build timings, `target/` binary digests, and rustdoc HTML bytes.

## 2. Baseline provenance

Captured 2026-11-14 between 17:03 and 17:06 (-03:00) on the host below, starting
from a clean working tree. `git status --short --branch` printed only
`## main...origin/main` — no staged, unstaged or untracked entries at capture
time.

| Item | Value |
|---|---|
| Commit | `c3e92d8c95157ac74330a3523d8145a1a42c89f5` (`c3e92d8`) |
| Commit subject | `docs: reconcile E2-15 status and add anti-slop policy` |
| Commit date | `2026-11-14T16:52:04-03:00` |
| Branch | `main`, upstream `origin/main`, no ahead/behind marker |
| Working tree | clean; 82 tracked files; `git describe --always --dirty` = `c3e92d8` |
| Parent commits | `0c44896` (E2-15 implementation), `f8a0e2c` (oracle differential evidence) |
| Host | WSL2, `Linux Ether 6.18.58.1-microsoft-standard-WSL2`, x86_64, 8 vCPU |
| OS | Ubuntu 24.04.4 LTS (Noble) |
| Toolchain | `rust-toolchain.toml` → stable; `rustc 1.100.0 (0ef1bc386 2026-11-09)`; `cargo 1.100.0 (3acc79c0c 2026-11-05)`; `rustfmt 1.11.0-stable`; `clippy 0.1.100` |
| Locale | `LANG=C.UTF-8`, `TERM=xterm-256color` |
| Raw provenance | `.research/anti-slop/baseline/git-info.txt`, `environment.txt` |

"Clean tree" here means the tracked baseline commit was checked out with no
modifications before any evidence was produced. The only working-tree addition
made by this task is this document itself; no tracked file was modified
(`git diff --stat` is empty and `git status --short` lists only this new file).

## 3. Baseline quality gates

Every gate was executed twice: once through `./scripts/check.sh` (the repository
gate) and once as an individual command so that each exit code is attributable.
Both runs passed.

| # | Gate | Exact command | Exit | Result | Evidence |
|---|---|---|---|---|---|
| 1 | Formatting | `cargo fmt --all -- --check` | 0 | passed (no diff) | `gate-fmt.txt` (empty output) |
| 2 | Compilation | `cargo check --workspace` | 0 | passed | `gate-check.txt` |
| 3 | Lints | `cargo clippy --workspace --all-targets -- -D warnings` | 0 | passed, no warnings | `gate-clippy.txt` |
| 4 | Tests | `cargo test --workspace` | 0 | passed | `gate-test.txt` |
| 5 | Docs | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | 0 | passed, 6 doc files generated for `chronofork_backend_api` | `gate-doc.txt` |
| — | Full script | `./scripts/check.sh` | 0 | all five gates passed end to end | `check-sh.stdout.txt`, `check-sh.stderr.txt`, `check-sh.exit.txt` |

Individual gate exit codes are in `.research/anti-slop/baseline/gates.exit.txt`:

```text
gate=fmt exit=0
gate=check exit=0
gate=clippy exit=0
gate=test exit=0
gate=doc exit=0
```

The full `./scripts/check.sh` run printed the five `==>` gate markers followed by
`==> all quality gates passed`. Its complete stdout is preserved as
`.research/anti-slop/baseline/check-sh.stdout.txt` (12,149 bytes, SHA-256
`18a716135c2dbd67779c3cc67bed0582ae5d2c071cae7a114cb3e426119d6719`);
stderr is `check-sh.stderr.txt` (1,699 bytes) and the exit code is 0.

### Test count and inventory

`cargo test --workspace` passed **152 tests, 0 failed, 0 ignored** across 14
test-result lines (9 test binaries plus 5 doc-test targets). The breakdown is
identical in the `./scripts/check.sh` run and in an independent merged-stream
run (`test-workspace.merged.txt`), and was re-confirmed with `--list` per target.

| Target | Tests |
|---|---|
| `chronofork-backend-api` unit tests | 0 |
| `chronofork-cli` unit tests (`src/main.rs`) | 0 |
| `chronofork-cli` `tests/demo.rs` | 4 |
| `chronofork-cli` `tests/trace.rs` | 10 |
| `chronofork-core` unit tests | 8 |
| `chronofork-megadrive` unit tests | 114 |
| `chronofork-sessions` unit tests | 4 |
| `chronofork-sessions` `tests/acceptance.rs` | 5 |
| `chronofork-test-backend` unit tests | 7 |
| Doc tests (5 crates) | 0 |
| **Total** | **152** |

This baseline supersedes the earlier, smaller snapshots recorded in
[quality gates](quality-gates.md); the growth is E2-15 interrupt-slot,
mask-gating, `RTE` and VINT-state coverage.

## 4. Baseline CLI demos

Both commands were run from the repository root at commit `c3e92d8` and exited 0
with empty functional stderr (only Cargo's `Finished`/`Running` lines). Each was
executed three times; every run produced byte-identical stdout.

| Demo | Command | Exit | stdout bytes | stdout SHA-256 | stderr SHA-256 (informational) |
|---|---|---|---|---|---|
| Synthetic session slice | `cargo run -p chronofork-cli -- demo` | 0 | 986 | `9b4dd564081e792ed44f92eed562f2e63a355ccca7e973682c1f7537d9b84652` | `90b6ed19ef62a0667ab3d9717fb02a594fa9e3e527c351ad0e93aae1f9aac035` |
| Mega Drive slice | `cargo run -p chronofork-cli -- megadrive demo` | 0 | 1,582 | `912a5bb6f5597179d76eca94f673112b6084e1357ff151eaac0368da30348996` | `92637bf4978d7cb21bbd48577952a3cbd21154bd83f3645ba8bb33c40426699d` |

Repeat runs (`demo.repeat1`, `demo.repeat2`, and the `megadrive-demo.repeat*`
equivalents) hash to the same stdout values and `cmp` byte-identically against
the canonical files; all repeat exit codes are 0. The first capture with
`cargo run -q` produced the same two stdout hashes, so Cargo's quiet flag does
not affect the program's own output.

### 4.1 `demo` stdout (exact baseline bytes)

```text
chronofork demo: deterministic session vertical slice
backend: ChronoFork Synthetic Test Backend (not a Mega Drive emulator) (0.1.0)
checkpoint: snapshot-1 at tick 12 hash 8b0827965107fc97c3257a1f2e0c211eae8faf14d03e4125186c3d6bb4f0dbd3
serialized checkpoint: 459 bytes, checksum verified
forks: session-2, session-3, session-4
branch session-2: tick 18 hash eaa0d9010248237641a79d8262882837fcc5cbeb707ee94145549dc98145f598
branch session-3: tick 18 hash e6fb480b78f50bbbce5ccd32c3172c744ba37d89d8e3bbbcbfa2257334126af5
branch session-4: tick 18 hash 898674dfb8fe659184090188e50dd343b89335d936b22e00412e76107e1c8e61
source isolation: source remains at tick 12 hash 8b0827965107fc97c3257a1f2e0c211eae8faf14d03e4125186c3d6bb4f0dbd3
memory isolation: branch C mutation did not affect branch A
replay verification: restored branch A reproduced its state hash
compare: branch A and B differ at tick 18 (eaa0d9010248237641a79d8262882837fcc5cbeb707ee94145549dc98145f598)
chronofork demo: PASS
```

### 4.2 `megadrive demo` stdout (exact baseline bytes)

```text
chronofork megadrive demo: 68000 + VDP port + interrupt scheduler slice
backend: ChronoFork Mega Drive 68000 (VDP port + interrupt scheduler, no rendering/audio) (0.1.0)
scope: 68000 subset, ROM/RAM, VDP status/control/VRAM window, deterministic interrupt scheduler
limitations: no rendering, audio, Z80, DMA, controllers or cycle-accurate timing
vdp: register 15 (auto-increment)=2 status=0x0100 address=0x0002
vdp: program wrote and read back VRAM word 0x1234 through the data port
interrupt: masked HBlank pending at tick 64 (level 4, vector 0x70, mask 7, deliverable=0)
interrupt: mask 7 blocked delivery at tick 65; request retained, no handler ran
fork isolation: fork=35b200747e43eb2d3f435fc8de4a22030014d438c232074d8e5982b06615f464 source=a8d2693cac9ccae7d866ef2e7062fdf23507e4db6fdf3c94ef2e78ef50a19a9a
snapshot: VDP, scheduler and CPU state reproduced from the checkpoint (masked HBlank preserved)
interrupt: masked VBlank pending level 6 vector 0x78 at tick 896 (mask 7); VDP VBlank/VINT bits set and snapshot restored them
interrupt: interrupt ROM retained HBlank at tick 7 under mask 7 (no handler)
interrupt: MOVE.W #$2300,SR lowered the mask to 3 at tick 8; the retained HBlank became deliverable
interrupt: HBlank delivered at tick 9 with handler IPM 4; handler ran; RTE at tick 11 returned to pc=0x214 with SR=0x2300 (mask 3, CCR restored)
interrupt: VBlank entered at IPM 6 and blocked the pending HBlank (no nested handler); RTE returned to pc=0x21C with SR=0x2300 (mask restored); snapshot preserved the pending VBlank/VINT state
chronofork megadrive demo: PASS
```

## 5. Checkpoint, fork and state hashes

Every hash printed by the two demos at baseline. `Tick` is the virtual tick stated
on the line; `—` means the demo does not print a tick for that value.

### 5.1 Synthetic session slice (`demo`)

| Item | Tick | SHA-256 of state | Printed on line |
|---|---|---|---|
| Checkpoint `snapshot-1` | 12 | `8b0827965107fc97c3257a1f2e0c211eae8faf14d03e4125186c3d6bb4f0dbd3` | `checkpoint: snapshot-1 at tick 12 hash …` |
| Source after forks (isolation) | 12 | `8b0827965107fc97c3257a1f2e0c211eae8faf14d03e4125186c3d6bb4f0dbd3` (unchanged) | `source isolation: source remains at tick 12 hash …` |
| Fork `session-2` (branch A) | 18 | `eaa0d9010248237641a79d8262882837fcc5cbeb707ee94145549dc98145f598` | `branch session-2: tick 18 hash …` |
| Fork `session-3` (branch B) | 18 | `e6fb480b78f50bbbce5ccd32c3172c744ba37d89d8e3bbbcbfa2257334126af5` | `branch session-3: tick 18 hash …` |
| Fork `session-4` (branch C) | 18 | `898674dfb8fe659184090188e50dd343b89335d936b22e00412e76107e1c8e61` | `branch session-4: tick 18 hash …` |
| Replay of restored branch A | — | reproduced the branch A hash; line prints no digest | `replay verification: restored branch A reproduced its state hash` |
| Serialized checkpoint | — | not hex-printed; **459 bytes**, checksum verified | `serialized checkpoint: 459 bytes, checksum verified` |
| Compare A vs B | 18 | differs; prints branch A hash `eaa0d9…` | `compare: branch A and B differ at tick 18 (…)` |

### 5.2 Mega Drive slice (`megadrive demo`)

| Item | Tick | SHA-256 of state | Printed on line |
|---|---|---|---|
| Fork state | — | `35b200747e43eb2d3f435fc8de4a22030014d438c232074d8e5982b06615f464` | `fork isolation: fork=… source=…` |
| Pre-fork source state | — | `a8d2693cac9ccae7d866ef2e7062fdf23507e4db6fdf3c94ef2e78ef50a19a9a` | same line |

The Mega Drive hashes are taken after the masked HBlank request (level 4, vector
`0x70`, mask 7) was retained rather than delivered; the snapshot line then asserts
that VDP, scheduler and CPU state were reproduced from the checkpoint with the
masked HBlank preserved. These hashes therefore cover CPU registers, SR/IPM, VDP
status/control/VRAM and pending-scheduler bytes as serialized at `c3e92d8`.

## 6. Artifacts and reproduction

All raw evidence is under `.research/anti-slop/baseline/` (git-ignored by
`.gitignore:48-49`). Integrity is recorded in `MANIFEST.txt`, which can be
verified with:

```sh
cd .research/anti-slop/baseline && sha256sum -c MANIFEST.txt
```

Key artifacts:

| Artifact | Contents |
|---|---|
| `git-info.txt` | Commit, branch, upstream, clean `git status --short --branch`, last three commits, tracked-file count |
| `environment.txt` | Kernel, distro, CPU count, `rustc`/`cargo`/`rustfmt`/`clippy` versions, toolchain file |
| `check-sh.{stdout,stderr,exit}.txt` | Full output and exit code of `./scripts/check.sh` |
| `gate-{fmt,check,clippy,test,doc}.txt`, `gates.exit.txt` | Individual gate outputs and exit codes |
| `test-workspace.merged.txt` | `cargo test --workspace` with stdout/stderr interleaved, mapping each target to its count |
| `demo.stdout.txt`, `demo.stderr.txt`, `demo.exit.txt` | Canonical `demo` capture |
| `megadrive-demo.stdout.txt`, `megadrive-demo.stderr.txt`, `megadrive-demo.exit.txt` | Canonical `megadrive demo` capture |
| `demo.repeat{1,2}.*`, `megadrive-demo.repeat{1,2}.*` | Determinism repeats |
| `MANIFEST.txt` | SHA-256 of every artifact in this directory except itself |

Reproduce the baseline (also the exact procedure for the after-refactor check):

```sh
git rev-parse HEAD
git status --short --branch

./scripts/check.sh; echo "check.sh exit=$?"

cargo run -p chronofork-cli -- demo \
  > /tmp/demo.stdout 2> /tmp/demo.stderr; echo "demo exit=$?"
sha256sum /tmp/demo.stdout
cmp /tmp/demo.stdout .research/anti-slop/baseline/demo.stdout.txt && echo "demo stdout identical"

cargo run -p chronofork-cli -- megadrive demo \
  > /tmp/megadrive-demo.stdout 2> /tmp/megadrive-demo.stderr; echo "megadrive exit=$?"
sha256sum /tmp/megadrive-demo.stdout
cmp /tmp/megadrive-demo.stdout .research/anti-slop/baseline/megadrive-demo.stdout.txt \
  && echo "megadrive stdout identical"
```

Expected stdout digests:
`9b4dd564081e792ed44f92eed562f2e63a355ccca7e973682c1f7537d9b84652` (`demo`) and
`912a5bb6f5597179d76eca94f673112b6084e1357ff151eaac0368da30348996`
(`megadrive demo`).

## 7. After-refactor verification (independent, pre-repair)

> **Superseded.** This section verified the tree after integration (t8). Repair
> round t13 then changed five tracked files, so the §7 verdict no longer binds
> the final bytes; §8 re-verifies the post-repair tree and is authoritative. §7
> is kept as the history of the pre-repair integration.

Re-run by the verifier on the integrated final working tree, from the commands
themselves rather than from the integrator's recorded artifacts. Measurements
were taken between 2026-11-14T18:16:21-03:00 and 18:23:10-03:00. Raw evidence is
`.research/anti-slop/verification/` (`START-STATE.txt`, `STRUCTURAL-CHECKS.txt`,
`ARTIFACT-HASHES.txt`, gate/demo/bench/probe outputs).

### 7.1 Tree and environment under test

- HEAD is still `c3e92d8c95157ac74330a3523d8145a1a42c89f5`; the refactor is the
  uncommitted working-tree change on top of it: 20 modified tracked files
  (1,409 insertions, 964 deletions) plus 7 untracked additions
  (`crates/chronofork-cli/tests/common/mod.rs`,
  `crates/chronofork-core/src/codec.rs`,
  `crates/chronofork-core/tests/codec_properties.rs`,
  `crates/chronofork-megadrive/examples/bench_step.rs`,
  `crates/chronofork-megadrive/tests/snapshot_properties.rs`,
  `crates/chronofork-sessions/tests/snapshot_properties.rs`, and this document).
- `git diff | sha256sum` was
  `e8df60d6a573851a8446b558624645a7acb601f712bb24372fb0bed7a4008840`
  before **and** after verification, so the verifier edited no implementation
  file (the only tracked file it changes is this evidence document).
- Environment is the same host and toolchain as §2. Every Cargo invocation in
  this section ran with `CARGO_HOME=/tmp/t7-cargo` because the sandboxed default
  `~/.cargo` registry is read-only and would otherwise block resolution of the
  `proptest` dev-dependency. This affects dependency-cache location only, not
  the compiled code or emitted bytes.
- The baseline artifacts were still intact at the end: 26/26
  `sha256sum -c MANIFEST.txt` checksums OK.

### 7.2 The same commands, exit codes and measurements

| Gate | Command | After exit | Baseline exit | After result | Baseline result |
|---|---|---|---|---|---|
| Formatting | `cargo fmt --all -- --check` | 0 | 0 | passed, no diff | passed, no diff |
| Compilation | `cargo check --workspace` | 0 | 0 | passed | passed |
| Lints | `cargo clippy --workspace --all-targets -- -D warnings` | 0 | 0 | passed, no warnings | passed, no warnings |
| Tests | `cargo test --workspace` | 0 | 0 | **194 passed, 0 failed, 0 ignored** | 152 passed, 0 failed, 0 ignored |
| Docs | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | 0 | 0 | passed; 6 doc files | passed; 6 doc files |
| Full script | `./scripts/check.sh` | 0 | 0 | all five gates + "all quality gates passed" | same |
| Whitespace | `git diff --check` | 0 | 0 | clean | clean |

The full after `./scripts/check.sh` stdout is
`.research/anti-slop/verification/after-check-sh.stdout.txt` (15,172 bytes,
SHA-256 `f72be6840a38778c061caa01186c20093f75bd1405501ccdf77f3cf0f11c182d`);
stderr is `after-check-sh.stderr.txt` (2,113 bytes,
`bc116431883f344637490c8abfe20afca8d2de19af55741b1071e63219d52db7`) and
`after-check-sh.exit.txt` contains `0`. Individual gate exit codes are in
`after-gates.exit.txt` (`fmt`, `check`, `clippy`, `test`, `doc` all 0), and
standalone rustdoc output is in `after-rustdoc.*` (exit 0).

| Demo | Command | Exit (base/after) | stdout bytes (base/after) | stdout SHA-256 (identical) | `cmp` result |
|---|---|---|---|---|---|
| Synthetic session slice | `cargo run -p chronofork-cli -- demo` | 0 / 0 | 986 / 986 | `9b4dd564081e792ed44f92eed562f2e63a355ccca7e973682c1f7537d9b84652` | byte-identical |
| Mega Drive slice | `cargo run -p chronofork-cli -- megadrive demo` | 0 / 0 | 1,582 / 1,582 | `912a5bb6f5597179d76eca94f673112b6084e1357ff151eaac0368da30348996` | byte-identical |

Each demo was run three times on the final tree; all repeats exit 0 and are
byte-identical to the baseline files
(`after-demo.repeat1/2.*`, `after-megadrive-demo.repeat1/2.*`). stderr contains
only Cargo's `Finished`/`Running` lines (hashes in `ARTIFACT-HASHES.txt`). Because
stdout is byte-identical, every value in the §5 checkpoint/fork/state table is
reproduced exactly from the new build.

### 7.3 Direct serialized-byte proof (differential probe)

Demo stdout proves printed hashes; to prove the *bytes*, the verifier wrote one
small probe source that exercises the public API and compiled it twice: against
a clean `git archive c3e92d8` export (`probe/baseline-tree/`, 82 files) and
against the final tree. Probe source SHA-256 is
`cb9ca7b262a66a83d37cd35fd6eba2260690a83a40e663e0e0e2f1fc11ad7ca6` (identical
copy in both manifests). Commands:

```sh
cd .research/anti-slop/verification/probe/baseline
CARGO_HOME=/tmp/t7-cargo CARGO_TARGET_DIR=../target-baseline cargo run --quiet \
  > ../probe-baseline.out
cd ../after
CARGO_HOME=/tmp/t7-cargo CARGO_TARGET_DIR=../target-after cargo run --quiet \
  > ../probe-after.out
diff ../probe-baseline.out ../probe-after.out
```

Both probes exit 0 and their 19-line outputs are **byte-identical**
(`diff` empty; both SHA-256
`89e6a4025587213d17767375e13fc95cbdc5cfb4009505381cc5e1f3219c9f63`), so the
canonical bytes, not merely their hashes, match:

| Probed item | Bytes | SHA-256 (baseline = after) |
|---|---|---|
| Mega Drive canonical payload after 1,000 steps (`interrupt_rom(3)`, NTSC, 4 instr/line) | 131,255 | payload `ff437abf3f8b5483130a8f9044ef5b8e5e7c1abd496ae39ee52c88340468c4fa`; state `7759ef1cf32b29887f96b9d3f939548603927acbd855574210f53bb04f6a9056`; tick 1000 |
| Session checkpoint envelope for that state | 131,431 | `d99e7ecb113fbb93e9079f930d0aa1f4b339dab70dbcc1d735daeee23206f84b` |
| Fork checkpoint envelope after 37 further steps | 131,431 | `10ed238c79a4c04ab244ea2a7db01c64ab8d233aab8d85f7913a6c33c8ab7ee4`; fork state `6b3d5e45b039c8595e9689348071a36909e27c021c397e04df2ccbd1e6c78add`, tick 1037 |
| `demo_rom` payload after 144 steps (default NTSC config) | 131,263 | payload `fd40ee95d54d7d4f10d033f447a3c159d4bef4a5baf51f1e7a8df4524f64540f`; state `a6be94fc7b1c76d8b0cdc1cec03d527c01d019edacd0a9422b9c198294cb19fd` |
| Synthetic backend checkpoint envelope after 12 steps | 459 | `fb3b794d52d11756df6c8e42ef5e6231072f8f23c39d6ab8d30a8204e5a502e8`; state `8b0827965107fc97c3257a1f2e0c211eae8faf14d03e4125186c3d6bb4f0dbd3`, tick 12 |

This satisfies §1 S2 directly: both the backend payload and the outer session
envelope are byte-identical across the refactor for fixed workloads, including
the 459-byte checkpoint length printed by the demo.

### 7.4 Test count and deduplication accounting

`cargo test --workspace` passes 194 tests (0 failed, 0 ignored) in 17 result
lines (12 test binaries + 5 doc-test targets). Every target has at least as many
tests as at baseline; four new property-test binaries were added:

| Target | Baseline | After | Change |
|---|---|---|---|
| `chronofork-backend-api` unit | 0 | 3 | +3 `validate_envelope` tests |
| `chronofork-cli` unit | 0 | 0 | — |
| `chronofork-cli` `tests/demo.rs` | 4 | 5 | +1 trace wiring smoke; semantic assertions moved to their owning layers |
| `chronofork-cli` `tests/trace.rs` | 10 | 10 | unchanged format/determinism/option coverage |
| `chronofork-core` unit | 8 | 14 | +6 codec tests |
| `chronofork-core` `tests/codec_properties.rs` | 0 | 4 | new property tests |
| `chronofork-megadrive` unit | 114 | 124 | +10 codec/state/API/debugger tests |
| `chronofork-megadrive` `tests/snapshot_properties.rs` | 0 | 9 | new property tests |
| `chronofork-sessions` unit | 4 | 5 | +1 state-hash-from-body test |
| `chronofork-sessions` `tests/acceptance.rs` | 5 | 5 | unchanged |
| `chronofork-sessions` `tests/snapshot_properties.rs` | 0 | 5 | new property tests |
| `chronofork-test-backend` unit | 7 | 10 | +3 (input byte, bounded reads, state hash) |
| Doc tests | 0 | 0 | — |
| **Total** | **152** | **194** | **+42, all additions** |

No behaviour-bearing target lost tests. The growth is the expected cost of the
property tests and of tests added by the structural changes, not a weakening of
the comparison.

### 7.5 Structural confirmation

- **One codec.** The only byte reader/writer abstraction is
  `chronofork_core::codec::{Reader, Writer}` (one `Reader` struct, one `Writer`
  struct). Every format writer uses it (session envelope, Mega Drive payload,
  scheduler snapshot, VDP snapshot, synthetic payload, config hash). Four of the
  five read paths use `Reader` (session envelope, Mega Drive payload, scheduler
  snapshot, synthetic payload).
- **One canonical state serializer per backend.** Mega Drive has exactly one
  field-order writer, `state_body` (`backend.rs:665`); `snapshot()` derives both
  payload and hash from `state_body_bytes()`, and the per-step `state_hash`
  path calls `state_hash_for` → `state_body`, so no second layout exists.
  The synthetic backend likewise has one `state_body` feeding both payload and
  hash, with tests `state_hash_hashes_the_payload_state_body` and
  `state_hash_is_derived_from_the_payload_state_body`.
- **One backend envelope validator.** `chronofork_backend_api::validate_envelope`
  is the only implementation and is called by both backends' `restore`
  (`megadrive/src/backend.rs:437`, `test-backend/src/lib.rs:342`). The session
  layer's checksum belongs to the separate outer checkpoint envelope, and its
  early backend-id/config-hash guards are pre-existing, distinct-contract
  pre-checks rather than a second backend validator.
- **No uncalled API.** The four APIs removed by the refactor
  (`Session::initial_tick`, `Session::snapshot_ids`,
  `BackendSnapshot::into_payload`, `InterruptScheduler::acknowledge_next`) have
  no remaining occurrence anywhere in `crates/` or `docs/` source. There are no
  `#[allow(dead_code)]`/`#[allow(unused)]` attributes, `clippy -D warnings` is
  clean, and a heuristic scan of all 86 distinct distinctive public function
  names in `crates/*/src` found zero with no reference outside their definition.
  Evidence: `STRUCTURAL-CHECKS.txt`.

### 7.6 Benchmark on the final tree

`cargo run -p chronofork-megadrive --example bench_step --release` was run three
times with `CARGO_HOME=/tmp/t7-cargo`; all exit 0. Values do not affect the
behavioural verdict but confirm the refactor did not collapse throughput:

| Metric | Run 1 | Run 2 | Run 3 |
|---|---|---|---|
| instructions/s | 7,565 | 7,570 | 7,545 |
| median s / 10,000 steps | 1.322396775 | 1.322849275 | 1.326067216 |
| serialized checkpoint bytes | 131,431 | 131,431 | 131,431 |
| backend payload bytes | 131,255 | 131,255 | 131,255 |
| final state hash | `7759ef1c…a9056` | same | same |
| `restore_state_hash_matches` | 1 | 1 | 1 |

There is no benchmark in the t1 baseline (it was added after), so the comparison
is against the integrator's pre-verification measurement band (7,385–7,554
steps/s); the independent re-run is inside that band or slightly faster. Only
timing lines vary; every digest and size line is constant across runs and matches
§7.3.

### 7.7 Verdict and residual risks (pre-repair, superseded)

**Verdict: PASS (pre-repair, superseded by §8).** All six gates at baseline are
matched or exceeded on the tree as integrated after t8, both demos are
byte-identical to the baseline, the checkpoint/fork/state hashes and serialized
sizes match, a differential probe proves the serialized bytes themselves are
identical, and the structural goals (one codec, one state serializer per backend,
one envelope validator, no uncalled API from the audit) are present. No
behavioural discrepancy was found; nothing was explained away.

**Supersession:** t13's repair round then changed
`crates/chronofork-megadrive/src/{backend,bus}.rs`,
`crates/chronofork-sessions/src/{session,manager}.rs` and
`THIRD_PARTY_NOTICES.md`, so this verdict does not bind the committed bytes. The
authoritative final verdict is §8.8; §7.3-§7.6 remain valid as the pre-repair
measurements that §8 reproduces.

Residual risks and qualifications (pre-repair; updated list in §8.8):

1. **VDP read path is index-based, not codec-based.** `Vdp::restore_bytes`
   deliberately reads fixed offsets rather than using `Reader`, because its error
   contract requires `SnapshotLengthMismatch` to take precedence over field
   errors; the rationale is documented in the source, the write path uses
   `Writer`, and adversarial property tests cover the parser. This is the one
   documented exception to "every parser uses the shared codec".
2. **Two `StateView` assembly sites in the Mega Drive backend.** `state_body_bytes`
   and `state_hash` each build the same struct, but the struct-literal field list
   is exhaustive and both hash paths go through the single `state_body` writer, so
   adding a field cannot silently skip hashing; the split is a measured
   performance decision (~18% step cost), documented in code.
3. **"No uncalled public API" is not compiler-proven.** Clippy cannot report
   unused `pub` items. The claim rests on absence of the audited names, absence
   of dead-code attributes, and a zero-reference scan of distinctive public
   functions; tooling such as a public-API diff would be needed for a proof.
4. **The session layer keeps its own early compatibility guards**
   (backend id and configuration hash) before restore. These pre-date the
   refactor and have a different error contract; they are not a second copy of
   `validate_envelope` but they do reject some states earlier than the backend.
5. **The differential probe covers fixed workloads, not the full state space.**
   It covers two Mega Drive workloads (1,000 and 144 steps), one fork, and one
   synthetic checkpoint; parser round-trips and arbitrary-byte rejection are
   covered separately by the property tests.
6. **Test deduplication changed counts.** The baseline-to-after table shows all
   additions and no target loss, but "one scenario per layer" was assessed from
   target ownership and counts, not from reading every removed assertion; the
   CLI semantic assertions were replaced by layer-owned session/megadrive tests.
7. **The verification is platform-scoped.** It was performed on the same WSL2
   x86_64 host and `rustc 1.100.0` toolchain as the baseline; no cross-platform
   determinism claim is added.
8. **The refactor is uncommitted.** The verified tree is
   `c3e92d8` + working-tree changes; the final commit must include all 20
   modified and 7 untracked files. Verifying a commit after the fact would
   require re-running this section.
9. **Timing is not an equality gate.** Benchmark figures vary run to run, and
   Cargo stderr text varies with cache state; only stdout bytes, serialized
   bytes, hashes and sizes are treated as normative.

## 8. Post-repair re-verification (final bytes)

The round-1 review findings led to repair round t13, which changed five tracked
files after §7 was written; §7 therefore no longer binds the bytes that will be
committed. This section is the independent verification of the post-repair tree
and supersedes §7's verdict. Measurements were taken 2026-11-14 between
18:54:41 and 18:58:00 (-03:00). Raw evidence is under
`.research/anti-slop/post-repair-verification/` (`START-STATE.txt`,
`CHANGED-PATH-HASHES.txt`, `POST-REPAIR-SUMMARY.txt`, `ARTIFACT-HASHES.txt`,
gate/demo/bench/probe outputs).

### 8.1 Tree under test and changed-path hashes

- HEAD remains `c3e92d8c95157ac74330a3523d8145a1a42c89f5`; the refactor plus the
  t13 repairs are the uncommitted working-tree change: **21 modified tracked
  files** (1,663 insertions, 1,002 deletions) and the same 7 untracked additions
  as §7.1.
- `git diff | sha256sum` was
  `d6221b38d6c53a904c31daabe66375133ef6e4c92ab44543e3b2ece6626f157d` before
  **and** after this verification. §7 recorded `e8df60d6…`; the difference is
  exactly the t13 repair set below, and the unchanged hash proves the verifier
  edited no implementation file.
- The five changed paths were hashed at the start and re-hashed at the end; each
  recorded hash equals the current file (`POST-REPAIR-SUMMARY.txt`):

| Path | t13 change | SHA-256 (recorded = current) |
|---|---|---|
| `crates/chronofork-megadrive/src/backend.rs` | `OutOfBounds` only for true in-window range overruns; `debugger_window_end() -> Option`; protection/alignment/no-window are `Execution` | `e81af060c32c89fd775526888fe847b144deb753c170063ee42c0687091ae490` |
| `crates/chronofork-megadrive/src/bus.rs` | start-address validation before the zero-length shortcut; total `usize::from((address & 0xFFFF) as u16)` in `peek8`/`write8` | `19eb5bbf4378275d79e436ee8bb6f456cd99eaa5c0a7b2323532a6338aad4a1b` |
| `crates/chronofork-sessions/src/session.rs` | `ensure_compatible` deleted; `BackendError::IncompatibleSnapshot` mapped to the session variant at the boundary | `277dfd226199257ab47609b7430c97acbcd95550798ff88fcc9285706ccfa15e` |
| `crates/chronofork-sessions/src/manager.rs` | `ensure_snapshot_backend` deleted; import relies on the same single predicate | `d59564fa808f1a2703974424b9fa638a5d2815fb8d26f613663a6d1ec16876b6` |
| `THIRD_PARTY_NOTICES.md` | proptest dev-only closure documented (23 packages, +21 lockfile entries) | `015a0b8ca6cee0ce847b56e7ecc708ee57bd96ba56eb428e3c3a706ee0b8efdc` |

- Every Cargo invocation used `CARGO_HOME=/tmp/t7-cargo` because the sandboxed
  default `~/.cargo` registry is read-only; this changes only where the
  dependency cache lives, not the compiled code or emitted bytes.

### 8.2 Gates and the 199-test accounting

| Gate | Command (all with `CARGO_HOME=/tmp/t7-cargo`) | Exit | Result |
|---|---|---|---|
| Full script | `./scripts/check.sh` | 0 | five `==>` gate markers + `==> all quality gates passed` |
| Formatting | `cargo fmt --all -- --check` (inside script) | 0 | clean |
| Compilation | `cargo check --workspace` (inside script) | 0 | clean |
| Lints | `cargo clippy --workspace --all-targets -- -D warnings` (inside script) | 0 | no warnings |
| Tests | `cargo test --workspace` (inside script) | 0 | **199 passed, 0 failed, 0 ignored** |
| Docs | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | 0 | standalone run clean; 6 doc files |
| Whitespace | `git diff --check` | 0 | clean |

`post-repair-check-sh.stdout.txt` is 15,608 bytes (SHA-256
`595d0fe3cddb04c64472f33b608ecef525f0e5a9e93f6843cef1ce5aaae035ab`), stderr is
2,113 bytes (`c83f8ff2…`, Cargo-only), and the exit file contains `0`. The
standalone rustdoc stdout is empty (`e3b0c442…`) with exit 0. The 199 tests span
17 result lines (12 test binaries + 5 doc-test targets); the delta from §7's 194
is exactly the repaired contract's new coverage:

| Target | Pre-repair (§7) | Post-repair | Delta |
|---|---|---|---|
| `chronofork-backend-api` unit | 3 | 3 | 0 |
| `chronofork-cli` unit / `tests/demo.rs` / `tests/trace.rs` | 0 / 5 / 10 | 0 / 5 / 10 | 0 |
| `chronofork-core` unit / `tests/codec_properties.rs` | 14 / 4 | 14 / 4 | 0 |
| `chronofork-megadrive` unit | 124 | **126** | **+2** (`protected_and_unmapped_writes_are_not_reported_as_out_of_bounds`, `zero_length_debugger_reads_still_validate_the_start_address`) |
| `chronofork-megadrive` `tests/snapshot_properties.rs` | 9 | 9 | 0 |
| `chronofork-sessions` unit | 5 | **8** | **+3** (`restore_rejects_a_snapshot_with_a_different_backend_id`, `..._config_hash`, `importing_a_foreign_snapshot_is_rejected_at_the_session_boundary`) |
| `chronofork-sessions` `tests/acceptance.rs` / `tests/snapshot_properties.rs` | 5 / 5 | 5 / 5 | 0 |
| `chronofork-test-backend` unit | 10 | 10 | 0 |
| Doc tests | 0 | 0 | 0 |
| **Total** | **194** | **199** | **+5, all additions** |

`THIRD_PARTY_NOTICES.md` now documents the development-only tooling:
`proptest 1.11.0` (MIT OR Apache-2.0), dev-dependency only, whose closure
resolves 23 packages and adds 21 `Cargo.lock` entries (16 → 37, no duplicates),
with 13 crates compiled for the x86_64-linux test target and none distributed
with ChronoFork. It is not linked into the `chronofork` binary.

### 8.3 Demos (three runs each, compared with the t1 baseline)

| Demo | Runs | Exit | stdout bytes | stdout SHA-256 | `cmp` vs `.research/anti-slop/baseline/` |
|---|---|---|---|---|---|
| `cargo run -p chronofork-cli -- demo` | 3 | 0 / 0 / 0 | 986 | `9b4dd564081e792ed44f92eed562f2e63a355ccca7e973682c1f7537d9b84652` (all 3) | **byte-identical** |
| `cargo run -p chronofork-cli -- megadrive demo` | 3 | 0 / 0 / 0 | 1,582 | `912a5bb6f5597179d76eca94f673112b6084e1357ff151eaac0368da30348996` (all 3) | **byte-identical** |

All six `cmp` invocations exited 0; stderr contains only Cargo's
`Finished`/`Running` lines (`demo.run*.stderr.txt`, `megadrive-demo.run*.stderr.txt`).
Because stdout is byte-identical, every checkpoint/fork/state value in §5 is
reproduced by the post-repair build.

### 8.4 Differential serialized-state probe (recreated)

The probe was recreated from scratch for this tree: a fresh
`git archive c3e92d8` export (82 files, `probe/baseline-tree/`) and the
post-repair tree were each compiled against the same probe source
(SHA-256 `cb9ca7b262a66a83d37cd35fd6eba2260690a83a40e663e0e0e2f1fc11ad7ca6`,
identical in both directories):

```sh
cd .research/anti-slop/post-repair-verification/probe/baseline
CARGO_HOME=/tmp/t7-cargo CARGO_TARGET_DIR=../target-baseline cargo run --quiet \
  > probe-baseline.out
cd ../after
CARGO_HOME=/tmp/t7-cargo CARGO_TARGET_DIR=../target-after cargo run --quiet \
  > probe-after.out
cmp ../probe-baseline.out ../probe-after.out
```

Both probes exited 0 and their 19-line outputs are byte-identical (`cmp` exit 0;
each SHA-256 `89e6a4025587213d17767375e13fc95cbdc5cfb4009505381cc5e1f3219c9f63`,
also identical to the pre-repair §7.3 output). The probed values therefore bind
the post-repair bytes:

| Probed item | Bytes | SHA-256 / hash (baseline = post-repair) |
|---|---|---|
| Mega Drive canonical payload after 1,000 steps | 131,255 | payload `ff437abf…c4fa`; state `7759ef1c…a9056`; tick 1000 |
| Session checkpoint envelope for that state | 131,431 | `d99e7ecb…f84b` |
| Fork checkpoint envelope after 37 further steps | 131,431 | `10ed238c…7ee4`; fork state `6b3d5e45…c78add`, tick 1037 |
| `demo_rom` payload after 144 steps | 131,263 | payload `fd40ee95…540f`; state `a6be94fc…19fd` |
| Synthetic backend checkpoint envelope after 12 steps | 459 | `fb3b794d…02e8`; state `8b082796…dbd3`, tick 12 |

### 8.5 Benchmark on the post-repair tree

`CARGO_HOME=/tmp/t7-cargo cargo run -p chronofork-megadrive --example bench_step --release`,
three runs, all exit 0:

| Metric | Run 1 | Run 2 | Run 3 |
|---|---|---|---|
| instructions/s | 7,531 | 7,340 | 7,513 |
| median s / 10,000 steps | 1.328237240 | 1.368665637 | 1.325571377 |
| serialized checkpoint bytes | 131,431 | 131,431 | 131,431 |
| backend payload bytes | 131,255 | 131,255 | 131,255 |
| final state hash | `7759ef1c…a9056` | same | same |
| `restore_state_hash_matches` | 1 | 1 | 1 |

The functional lines are constant across runs and match §8.4. Timing varies run
to run (pre-repair §7.6 measured 7,545–7,570 instructions/s); this session's band
is 7,340–7,531, so no throughput claim is made in either direction from this
benchmark. It is evidence that the post-repair build executes the fixed workload
and round-trips its checkpoint, not a performance guarantee.

### 8.6 Post-repair contract changes verified

- **`OutOfBounds` only for true in-window range overruns.**
  `map_memory_error` maps `UnmappedRead`/`UnmappedWrite`/`AddressOverflow` to
  `BackendError::OutOfBounds { address, length, limit }` only when
  `debugger_window_end(rom_len, address, writable)` returns `Some(limit)`, i.e.
  the requested start is inside the cartridge image (reads) or work RAM
  (reads/writes) and the validated range overruns that window. With no containing
  window (unmapped hole, VDP port window, address above the 32-bit bus) the error
  is `BackendError::Execution` with an explicit message, so `limit` can never be
  reported above a start that was rejected. `BusError::WriteProtected`,
  `BusError::OddAddress`, `BusError::Vdp` and `SideEffectingRead` map to
  `Execution`, their pre-refactor classification. Covered by
  `protected_and_unmapped_writes_are_not_reported_as_out_of_bounds`,
  `debugger_bus_failures_keep_their_classification`,
  `debugger_writes_are_limited_to_ram` and
  `debugger_reads_validate_the_range_before_allocating`.
- **One compatibility predicate, at the session boundary.**
  `Session::ensure_compatible` and `SessionManager::ensure_snapshot_backend` are
  deleted (no occurrence in `crates/`). `chronofork_backend_api::validate_envelope`
  remains the only implementation and is called only by the two backends'
  `restore`. `session.rs::map_backend_compatibility` translates
  `BackendError::IncompatibleSnapshot { field, expected, found }` into
  `SessionError::IncompatibleSnapshot`, preserving the session variant contract
  without repeating the backend-id/config-hash rules. Covered by
  `restore_rejects_a_snapshot_with_a_different_backend_id`,
  `restore_rejects_a_snapshot_with_a_different_config_hash` and
  `importing_a_foreign_snapshot_is_rejected_at_the_session_boundary`.
- **Zero-length address-validation order fixed.**
  `MegadriveBus::read_memory` validates `u32::try_from(address)` *before* the
  `length == 0` shortcut, so an address above the 32-bit bus is rejected for an
  empty read exactly as at baseline, while any representable address with length
  zero still returns an empty vector without touching a window. Covered by
  `zero_length_debugger_reads_still_validate_the_start_address` plus the backend
  zero-length assertions.
- None of these changes alter emulated execution, serialized bytes, state hashes
  or demo output: §8.3 and §8.4 reproduce all of them exactly.

### 8.7 Structural recheck on the post-repair tree

- One codec (`chronofork-core::codec::{Reader, Writer}`), one `state_body` field
  writer per backend feeding both payload and hash, and one
  `validate_envelope` implementation — unchanged by t13.
- The four previously audited uncalled APIs (`Session::initial_tick`,
  `Session::snapshot_ids`, `BackendSnapshot::into_payload`,
  `InterruptScheduler::acknowledge_next`) remain absent, as do the two deleted
  session compatibility helpers. No `#[allow(dead_code)]`/`#[allow(unused)]`
  attributes exist and `clippy -D warnings` is clean.
- The documented VDP index-based read exception is unchanged; all writers and
  the other four parsers still use the shared codec.
- Evidence: `POST-REPAIR-SUMMARY.txt` and the gate artifacts in the same
  directory.

### 8.8 Verdict (final) and residual risks

**Verdict: PASS for the post-repair bytes that will be committed.** With
`CARGO_HOME=/tmp/t7-cargo`, the full gate passes (199 tests, 0 failed), rustdoc
passes, both demos are byte-identical to the t1 baseline in three runs each, the
recreated differential probe is byte-identical to the clean-baseline export, the
benchmark exits 0 with constant functional output, `git diff --check` is clean,
the five changed paths are hash-recorded and match, and the verifier edited no
implementation file. No behavioural discrepancy was found and none was explained
away.

Residual risks and qualifications:

1. **VDP read path is index-based, not codec-based** (documented exact-length
   error-precedence exception; property-tested). All writers and the other four
   readers use the shared codec.
2. **Two `StateView` assembly sites in the Mega Drive backend**; the struct
   literal is exhaustive and both hash paths go through the single `state_body`
   writer. The split is a measured performance decision documented in code.
3. **"No uncalled public API" is not compiler-proven.** The claim rests on
   absence of the audited names, no dead-code attributes, a zero-reference scan
   of distinctive public functions, and clean Clippy; a public-API diff tool
   would be needed for a proof.
4. **The debugger error contract changed intentionally.** `WriteProtected`,
   `OddAddress` and no-containing-window accesses that §7's tree reported as
   `OutOfBounds` are now `Execution`. Callers matching on `OutOfBounds` see a
   different variant for those cases; this is the reviewed repair, is covered by
   tests, and does not change emulated execution or serialized state.
5. **Session compatibility rejection moved.** With the pre-checks deleted, a
   mismatched snapshot is rejected by the backend's `validate_envelope` and
   mapped to `SessionError::IncompatibleSnapshot`; the variant is preserved, but
   the error message/field source (`validate_envelope`) and any ordering relative
   to other backend restore failures can differ from the removed session checks.
6. **The differential probe covers fixed workloads**, not the full state space:
   two Mega Drive workloads, one fork and one synthetic checkpoint; parser
   round-trips and arbitrary-byte rejection rely on the property tests.
7. **Test deduplication accounting** is based on per-target counts and layer
   ownership, not a line-by-line review of every removed assertion.
8. **Platform-scoped.** Same WSL2 x86_64 host and `rustc 1.100.0` as the baseline;
   no cross-platform determinism claim is added.
9. **The tree is uncommitted.** The verified bytes are `c3e92d8` + 21 modified and
   7 untracked files; the hashes bind that working tree, so any later edit
   invalidates this section. The final commit must include all 28 paths and can be
   re-checked against `POST-REPAIR-SUMMARY.txt`.
10. **Timing is not an equality gate.** Benchmark figures vary per run and this
    session's band is slightly below the pre-repair band; only stdout bytes,
    serialized bytes, hashes and sizes are normative.
11. **`CARGO_HOME=/tmp/t7-cargo`** is a dependency-cache path required by the
    read-only default registry; it does not affect compiled behaviour.
12. **Serialized bytes are proven for the probed states only**; for other states
   the evidence is the state hash (computed from serialized bytes), the sizes
   printed by the demos, and the round-trip/property tests.

## 9. Limitations

- The baseline is scoped to this host and toolchain (WSL2 x86_64, `rustc 1.100.0`).
  The refactor must not introduce platform-dependent output; no cross-platform
  determinism claim is made here.
- `cargo doc` is gated by exit code with `RUSTDOCFLAGS="-D warnings"`; generated
  HTML bytes are not compared.
- Demo stdout does not expose serialized checkpoint hex. In the baseline this was
  a proxy via the state hash, the printed 459-byte length and the codec
  round-trip/version tests; §7.3 now closes it with a differential probe that
  compares the raw serialized bytes of the same fixed states across the baseline
  and refactored trees.
- The demo covers a synthetic session scenario and one bounded Mega Drive slice
  (68000 subset, VDP port window, interrupt scheduler). It is not evidence about
  rendering, DMA, audio, Z80, controllers, privilege enforcement or game
  compatibility, none of which exist in this baseline.
- Cargo stderr lines are retained as artifacts but are not an equality gate,
  because they include cache state and elapsed-time text.
- The after-refactor run used `CARGO_HOME=/tmp/t7-cargo` because the sandboxed
  default registry is read-only; this is a dependency-cache path only and does
  not affect compiled behaviour.

## 10. Final integration gate (t12 closure)

This section closes the refactor for handoff. It adds no behavioral claim beyond
§8; it records the docs-only delta that arrived after t15 bound the code bytes,
the final gate on the resulting tree, the documentation links, and the residual
risks that remain when the tree is committed. Raw evidence:
`.research/anti-slop/final/` (git-ignored).

### 10.1 Code bytes are unchanged since §8 (t15 binding intact)

The five code paths recorded in §8.1 were re-hashed on the final tree; every
value equals §8.1 and `POST-REPAIR-SUMMARY.txt`, so the t13 repair is unchanged
and the §8 behavioral verdict still binds:

| Path | SHA-256 (re-hashed, matches §8.1) |
|---|---|
| `crates/chronofork-megadrive/src/backend.rs` | `e81af060c32c89fd775526888fe847b144deb753c170063ee42c0687091ae490` |
| `crates/chronofork-megadrive/src/bus.rs` | `19eb5bbf4378275d79e436ee8bb6f456cd99eaa5c0a7b2323532a6338aad4a1b` |
| `crates/chronofork-sessions/src/session.rs` | `277dfd226199257ab47609b7430c97acbcd95550798ff88fcc9285706ccfa15e` |
| `crates/chronofork-sessions/src/manager.rs` | `d59564fa808f1a2703974424b9fa638a5d2815fb8d26f613663a6d1ec16876b6` |
| `THIRD_PARTY_NOTICES.md` | `015a0b8ca6cee0ce847b56e7ecc708ee57bd96ba56eb428e3c3a706ee0b8efdc` |

### 10.2 Docs-only delta since §8

`§8.1` recorded 21 modified tracked files because it was captured before the
documentation task (t11). The final working tree is **25 modified tracked files
+ 7 untracked paths**; the four additional tracked files are docs-only and are
recorded here with their final hashes:

| Path | Change | SHA-256 (final) |
|---|---|---|
| `AGENTS.md` | anti-slop workflow section added by t11 (not edited by t12) | `7e5fea0b885d6c557246d60375774e68adee1e80b916ace2495bd6b9b7f6aa59` |
| `docs/README.md` | anti-slop policy + evidence rows by t11; t12 added the missing `AGENTS.md` row | `1ed6bf17c4af105d8fac9fa31c4a768346a23096345b810282489a48633fe722` |
| `docs/quality/anti-slop-policy.md` | rewritten as the agent playbook by t11 (not edited by t12) | `8f7c8046e365ad4d3e8ccf72747f60d7bc09e1fa71485e50f45225fef98d1f58` |
| `docs/quality/quality-gates.md` | current-status/evidence links by t11 (not edited by t12) | `78e68e34d39c281df775d6ad60e7e71fb05745b80f9152c6ad4a5f607c9d4c3d` |

`git diff --stat` on the final tree: **25 files changed, 2,193 insertions(+),
1,227 deletions(-)**. The full handoff set is:

```text
Modified (25):
AGENTS.md
Cargo.lock
Cargo.toml
THIRD_PARTY_NOTICES.md
crates/chronofork-backend-api/src/lib.rs
crates/chronofork-backend-api/src/snapshot.rs
crates/chronofork-cli/src/main.rs
crates/chronofork-cli/tests/demo.rs
crates/chronofork-cli/tests/trace.rs
crates/chronofork-core/Cargo.toml
crates/chronofork-core/src/lib.rs
crates/chronofork-megadrive/Cargo.toml
crates/chronofork-megadrive/src/backend.rs
crates/chronofork-megadrive/src/bus.rs
crates/chronofork-megadrive/src/cpu.rs
crates/chronofork-megadrive/src/interrupts.rs
crates/chronofork-megadrive/src/vdp.rs
crates/chronofork-sessions/Cargo.toml
crates/chronofork-sessions/src/manager.rs
crates/chronofork-sessions/src/session.rs
crates/chronofork-sessions/src/snapshot.rs
crates/chronofork-test-backend/src/lib.rs
docs/README.md
docs/quality/anti-slop-policy.md
docs/quality/quality-gates.md

Untracked (7):
crates/chronofork-cli/tests/common/
crates/chronofork-core/src/codec.rs
crates/chronofork-core/tests/
crates/chronofork-megadrive/examples/
crates/chronofork-megadrive/tests/
crates/chronofork-sessions/tests/snapshot_properties.rs
docs/quality/anti-slop-refactor-evidence.md
```

### 10.3 Final gate on the final tree

All Cargo commands used `CARGO_HOME=/tmp/t7-cargo` (read-only default registry;
dependency-cache path only). Exact results:

| Command | Exit | Result |
|---|---:|---|
| `CARGO_HOME=/tmp/t7-cargo ./scripts/check.sh` | 0 | fmt/check/clippy `-D warnings` clean; `cargo test --workspace` **199 passed, 0 failed, 0 ignored** (17 result lines); rustdoc `-D warnings` clean |
| `CARGO_HOME=/tmp/t7-cargo cargo run -p chronofork-cli -- demo` | 0 | 986-byte stdout, SHA-256 `9b4dd564081e792ed44f92eed562f2e63a355ccca7e973682c1f7537d9b84652`, `cmp`-identical to the t1 baseline |
| `CARGO_HOME=/tmp/t7-cargo cargo run -p chronofork-cli -- megadrive demo` | 0 | 1,582-byte stdout, SHA-256 `912a5bb6f5597179d76eca94f673112b6084e1357ff151eaac0368da30348996`, `cmp`-identical to the t1 baseline |
| `RUSTDOCFLAGS="-D warnings" CARGO_HOME=/tmp/t7-cargo cargo doc --workspace --no-deps` | 0 | clean; all workspace crates documented |
| `CARGO_HOME=/tmp/t7-cargo cargo run -p chronofork-megadrive --example bench_step --release` | 0 | 7,172 steps/s this run; checkpoint 131,431 B; final state hash `7759ef1cf32b29887f96b9d3f939548603927acbd855574210f53bb04f6a9056`; `restore_state_hash_matches=1` (timing varies; not an equality gate) |
| `git diff --check` | 0 | no output |
| relative-link check over `anti-slop-policy.md`, `docs/README.md`, `AGENTS.md`, `anti-slop-refactor-evidence.md` | 0 | `links ok`; the additional anchor/fragment check also passed (`anchors ok`) |
| baseline manifest (`sha256sum -c .research/anti-slop/baseline/MANIFEST.txt`) | 0 | 26/26 OK |

### 10.4 Documentation links

The evidence header links the [anti-slop policy](anti-slop-policy.md) and
[AGENTS.md](../../AGENTS.md). The docs index links the
[anti-slop policy](anti-slop-policy.md), the
[anti-slop refactor evidence](anti-slop-refactor-evidence.md) and now
[AGENTS.md](../../AGENTS.md) directly. The policy links this evidence and AGENTS.md;
the quality gates link the recorded evidence sections. All relative paths and
fragment anchors resolve.

### 10.5 Tree, assets and source compatibility

- HEAD is still `c3e92d8c95157ac74330a3523d8145a1a42c89f5` and `origin/main` is the
  same commit; branch `main` is in sync; there are **no tags** (0 refs) and no
  commit, push or tag was created by this refactor.
- No tracked ROM/BIOS/firmware/archive/secret file exists; the tracked tree is
  text source/docs/tests, and `.planning/` (and any `.research/` evidence) is
  ignored and untracked. The seven untracked paths above are the intended new
  source/test/example/evidence files.
- The only source-compatibility change is the documented removal of four
  uncalled public methods: `Session::initial_tick`, `Session::snapshot_ids`,
  `BackendSnapshot::into_payload`, `InterruptScheduler::acknowledge_next`. The
  public additions (`chronofork_core::codec`, `validate_envelope`,
  `VideoStandard::code`) are additive and all have in-repo callers/tests. The
  intentional debugger error-classification repair is described in §8.6 and
  risk 4 below; it changes error variants, not emulated behavior or bytes.

### 10.6 Residual risks and deferred items

The twelve risks in §8.8 remain authoritative. For handoff, the ones that matter
most are:

1. VDP read stays index-based (documented exact-length error-precedence
   exception; property-tested); the other readers/writers use the codec.
2. The Mega Drive backend has two `StateView` assembly sites; both hash paths use
   the single `state_body` writer, and the split is a measured performance
   decision.
3. "No uncalled public API" is not compiler-proven; it rests on the audited-name
   absence, no dead-code allows, a zero-reference scan and clean Clippy.
4. Intentional debugger error-contract change: `WriteProtected`, `OddAddress` and
   no-containing-window accesses are `Execution`, not `OutOfBounds`; callers
   matching that variant see the change, while emulated state and bytes do not.
5. Session compatibility rejection moved into `validate_envelope` with
   `SessionError::IncompatibleSnapshot` preserved; error text/field source and
   ordering may differ from the removed pre-checks.
6. The differential probe covers fixed workloads, not the full state space;
   parser breadth rests on property tests.
7. Test deduplication is assessed by per-target counts and layer ownership.
8. Platform-scoped evidence: same WSL2 x86_64 host and `rustc 1.100.0`.
9. The tree is uncommitted; any later edit invalidates the hashes in §8/§10 and
   requires re-verification. The commit must include all 25 modified + 7
   untracked paths listed in §10.2.
10. Benchmark timing is not a gate; only bytes, hashes and sizes are normative.
11. `CARGO_HOME=/tmp/t7-cargo` is a cache path only.
12. Serialized bytes are proven for probed states; other states rely on the
    state hash, demo sizes and round-trip/property tests.

Deferred: a public-API diff tool, CI wiring, a cross-platform determinism claim,
and full emulator differential coverage. None blocks this refactor's contract.
