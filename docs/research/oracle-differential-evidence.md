# Oracle Differential Instruction-Trace Evidence


> **Superseded for timing (Phase 8.2):** the instruction-quantum scheduler and
> per-ROM quantum/masked-boundary control described on this page are superseded by
> the master-cycle timeline and the [68000 cycle differential evidence](cpu-cycle-differential-evidence.md).
> The address-projection and final-state results below remain historical evidence;
> they are not a cycle-timing claim. The current Phase 8.2 differential runs thirteen
> synthetic ROMs and passes 13/13 address projections, 12/13 per-instruction cycle
> comparisons, 12/13 selected final states and 13/13 negative controls; the one
> scoped exception is ``rom_exception`` (see the W4.2/W4.3 review note in
> ``.research/consolidation-2026-11-20/reviews/w4-oracle.md`` for the oracle's saved-SR
> reserved bit and its 38-vs-34 ``TRAP`` cost). The thirteen-ROM result is in the
> [68000 cycle differential evidence](cpu-cycle-differential-evidence.md).

- **Status:** Historical 3-ROM evidence **PASS 3/3** against the pinned Exodus
  headless oracle: the three original independently authored synthetic ROMs have
  identical projected address streams, equal selected final state and passing
  negative controls. The E2-15 addition `rom_masked` omitted
  `--instructions-per-line`, ran at the then-default quantum of 64, crossed two
  HBlank boundaries while the 68000 SR interrupt mask stayed 7, and entered no
  interrupt handler. This page is preserved history; the current harness and
  result are the thirteen-ROM master-cycle differential in the
  [68000 cycle differential evidence](cpu-cycle-differential-evidence.md). The
  earlier pre-repair divergence (real 68000 absolute-long `MOVE` decoding defect)
  is preserved below as history and was fixed under
  [EPIC-0002](../epics/EPIC-0002-megadrive-slice.md) E2-13.
- **Scope (historical):** instruction-address streams, opcode/ROM-byte
  consistency and selected final architectural state for the original three
  synthetic ROMs, plus a negative gating control on the masked ROM. **Not** cycle
  accuracy, interrupt timing, interrupt delivery equivalence, rendering, audio,
  Z80, DMA or game compatibility.
- **Related:** [Mega Drive 68000 slice](../architecture/megadrive-slice.md),
  [EPIC-0002](../epics/EPIC-0002-megadrive-slice.md),
  [provenance](../licensing/provenance.md),
  [testing strategy](../quality/testing-strategy.md).

## Summary

The reproducible harness `scripts/oracle-diff.py` runs the pinned Exodus headless
oracle and ChronoFork's bounded instruction trace over the same generated ROM, then
compares the executed-address sequence, opcode/ROM-byte consistency and selected
final state. It also records the per-ROM scheduler quantum and, for the masked ROM,
checks the instruction-quantum HBlank crossing and mask-7 gating control.

| ROM | Quantum | Projected addresses | Projection SHA-256 | Final state | Negative control |
|---|---|---|---|---|---|
| `rom_alu` | 1,000,000 (explicit) | 99/99 identical | `2b6f191c…` on both sides | D0–D7, A0–A7, PC, effective SR and 64 work-RAM bytes equal | index 49 caught, exit 1 |
| `rom_mem_vdp` | 1,000,000 (explicit) | 51/51 identical | `ba4f3948…` on both sides | D0–D7, A0–A7, PC, effective SR and 64 work-RAM bytes equal | index 25 caught, exit 1 |
| `rom_masked` | **64 (CLI default, flag omitted)** | 155/155 identical | `02891e15…` on both sides | D0–D7, A0–A7, PC, effective SR and 64 work-RAM bytes equal | index 77 caught, exit 1 |

Historical 3-ROM summary: `status: pass`, 3/3 trace matches, 3/3 final-state matches,
3/3 negative controls, 3/3 scheduler controls (masked gating 1/1), harness exit 0.
`rom_masked` executed 155 instructions at quantum 64, completed two scanlines
(boundaries at ticks 64 and 128) with the SR mask at 7 on both sides, executed no
SR-writing opcode and entered no handler stub.

The canonical evidence is `.research/evidence/oracle-diff.{json,md}` (schema
`chronofork.oracle-diff.v2`). The pre-E2-15 canonical Phase-10 evidence —
two ROMs, explicit 1,000,000-instruction quantum, no masked-ROM run — is preserved
unchanged under `.research/evidence/before-e2-15/` (with a file manifest).
Identical behavioral audit runs are under `.research/evidence/repro-e2-15/` and
`/tmp/chronofork-e2-15-verify/`, and the preserved pre-repair E2-13 run is under
`.research/evidence/before-repair/`.

### History at a glance

| Stage | `rom_alu` | `rom_mem_vdp` |
|---|---|---|
| Pre-repair (original evidence) | PASS, 99/99 | **FAIL** at projected index 28, PC `0x00027A`, opcode `0x2239` rejected as illegal |
| Post-repair (E2-13 fix) | PASS, 99/99 | **PASS**, 51/51, first divergence none |

The divergence was a real pre-existing core decoding defect: the 68000 encodes a
`MOVE.B/W/L (abs).L,Dn` **load** with source mode/register `111/001` (low byte
`0x39`), but the pre-repair core accepted the absolute-*word* encoding (`0x38`) with
a 4-byte address fetch. The synthetic ROM uses the real `0x39` encoding, so the
pre-repair core rejected it as illegal. The repair makes `0x1039`/`0x2039`/`0x3039`
decode correctly and keeps `0x1038`/`0x2038`/`0x3038` as typed illegal opcodes; the
finding was never masked by weakening the comparison.

## Pinned inputs

| Item | Value |
|---|---|
| Oracle project | `exodus-for-ais` (Exodus headless fork), MIT root license |
| Oracle remote | `https://github.com/paulomanrique/exodus-for-ais` |
| Oracle branch | `linux-headless` |
| Oracle commit | `ec0056151f3ff1c334d09ee375abb30a68fdfec5` |
| Oracle checkout | ignored `.research/upstream/exodus-headless` |
| `exodus-probe` SHA-256 | `07c56c54e855cd4fdba2cf12df17b8079ff4540e0f3ab95486a42ea4cf912557` |
| `exodus-mcpd` SHA-256 | `7da8dcb00bc678dca813f1e56aed0c0deaa814613a7ad8609ce92212049f3ff6` |
| Module | `Data/Modules/Sega Mega Drive 1600.xml` |
| Trace device | `Main 68000` |
| Oracle frames | 1 |
| ChronoFork commit | `f8a0e2c6efcbf34e683e57e59bcc8495c9479e3d` (last commit; evidence worktree dirty) |
| ChronoFork evidence binary SHA-256 | `ff7a48f0912d891f0de74134c1523bbc861ced5ea153421c718e11c8804040eb` |
| E2-15 harness SHA-256 | `d7f3561f5a1fd2e632951d07a5cbebbcfb2f416187ed1133f00a064fe2208792` |
| Evidence schema | `chronofork.oracle-diff.v2` |
| Pre-repair ChronoFork binary SHA-256 | `ee46501022034f19f364657ccd9247a52f5361ea5dcb877d7f8cc6e7596b30ed` |
| Pre-repair harness SHA-256 | `8230ab267419bd483b842b3bba20ed09f507e92696da9dddd5367ce2f157e970` |
| Scheduler quantum | per ROM: 1,000,000 explicit (`rom_alu`, `rom_mem_vdp`); CLI default 64 with the flag omitted (`rom_masked`) |
| Compared RAM | 64 bytes at `0x00FF0000` |

The ChronoFork entry is the last repository commit plus uncommitted E2-15
trace/CLI/backend work; the exact binary digest is therefore recorded rather than
relying on the commit alone. No commit or push was made for this evidence. The
pre-repair binary and harness digests are listed so the historical FAIL can be tied
to exact artifacts. The old `17f7e4f8…` binary and `241a62f3…` harness digests
belong to the preserved pre-E2-15 canonical run under
`.research/evidence/before-e2-15/`.

## Package prerequisites and oracle build

Build environment observed for the oracle: Ubuntu 24.04.4 LTS under WSL2 on x86_64,
Python 3.12.3, `cmake` 3.28.3, `ninja` 1.11.1, `clang`/`clang++` 18.1.3, and
`pkg-config` resolving expat 2.6.1, zlib 1.3, libpng 1.6.43 and libjpeg 2.1.5
(`libexpat1-dev`, `zlib1g-dev`, `libpng-dev`, `libjpeg-dev`, `ninja-build`, `clang`).
No `sudo` was used during the build.

At the pinned commit, in the ignored oracle checkout:

```sh
cd .research/upstream/exodus-headless
cmake -S . -B build -G Ninja \
  -DCMAKE_C_COMPILER=clang -DCMAKE_CXX_COMPILER=clang++ \
  -DCMAKE_BUILD_TYPE=Release
cmake --build build -j"$(nproc)"
```

The build completed all 175 targets with exit 0. The harness expects the two
binaries at `.research/oracle/build/exodus-probe` and
`.research/oracle/build/exodus-mcpd` by default (or pass `--oracle-bin-dir`).
The recorded binary digests identify the exact artifacts used. Rebuilding the same
commit with the same toolchain is expected to reproduce behaviour, not necessarily
a bit-identical executable.

### Empty clone: the oracle is not provisioned, and that is an explicit failure

The checkout, both binaries and the module all live under the ignored `.research/`
tree, so a clean clone cannot run the differential. The harness makes that a
first-class outcome instead of a generic setup error:

| Outcome | Exit | Meaning |
|---|---|---|
| pass | 0 | every compared ROM matched |
| divergence | 1 | a trace, cycle or final-state comparison differs (reported as a finding) |
| setup error | 2 | the harness itself cannot proceed (malformed trace, missing Python, unbuildable CLI) |
| **oracle unavailable** | **3** | the pinned oracle is `absent` or `unpinned`; results are unavailable, never passing |

```sh
python3 scripts/oracle-diff.py --oracle-status     # report only: 0 provisioned, 3 not
python3 scripts/oracle-diff.py --require-oracle    # 3 instead of running when mandatory
python3 scripts/oracle-diff.py --dump-roms         # the offline half: no oracle needed
CHRONOFORK_ORACLE_ROOT=/path/to/exodus-headless \
CHRONOFORK_ORACLE_BIN_DIR=/path/to/build python3 scripts/oracle-diff.py ...
```

`--oracle-status` prints the provenance record (`status`, pinned commit, branch,
module, both binary SHA-256 values and the MIT license note). The contract is
locked by `crates/chronofork-megadrive/tests/oracle_harness.rs`, which needs no
oracle: a missing default location must report `absent` and exit 3, while an
explicitly requested path that does not exist is reported with the flag to check.
`--require-oracle` exists so CI cannot silently omit the differential and still
report success (review Q-01/Q-05).

A separate build-validation run executed a 1024-byte synthetic smoke ROM and
produced **2367** trace records starting at reset PC `0x200`, with two runs
byte-identical at SHA-256
`d720aa8417838894ddbcf868ac6fac33b092a5a6b92173da87bcb23726cd9fc4`. That smoke ROM
is not one of the three differential ROMs below; its count and digest are recorded
here only as build/probe evidence and must not be conflated with the differential
numbers.

## Synthetic differential ROMs

All three ROMs are 4096 bytes, independently authored and generated
deterministically by `scripts/oracle-diff.py`. They contain the Mega Drive header
fields needed by the oracle loader, load the initial stack pointer and PC from the
reset vectors, and end in a marker self-loop. None uses absolute-word loads,
PC-relative addressing, or any instruction outside the documented ChronoFork
subset.

The marker addresses and SHA-256 digests below are the ones the generator produces
after 927792f cleared the architecturally undefined reset `X` in the shared
register setup with a two-byte `ADD.B D0,D0`; every marker moved by `+2`
(`rom_alu` `0x000352`→`0x000354`, `rom_mem_vdp` `0x0002E0`→`0x0002E2`,
`rom_masked` `0x0002EE`→`0x0002F0`) and the earlier digests are superseded. The
`--stop-pc` transcripts, the command examples and the review tables all quote those
corrected addresses.

| ROM | SHA-256 | Size | Marker PC | Purpose |
|---|---|---|---|---|
| `rom_alu` | `aaa4b3d3dd866bb9dfb3c96de4dec903bd1b346dce24891bc20a874162c8996a` | 4096 | `0x000354` | ALU, shifts, flags and branch conditions; no absolute-long loads |
| `rom_mem_vdp` | `21a6e4a611e4026cf2c70e2e66bb4694a2ddce16d80c4f65319a84e5e2f2fe25` | 4096 | `0x0002E2` | Byte/word/long memory forms, absolute-long loads, VDP control/data writes |
| `rom_masked` | `f8cb96dc8455d96fe92fcd4870c75baffec958dba9d24a6613025bba8fc6ad44` | 4096 | `0x0002F0` | Ten-iteration self-checking arithmetic recurrence, SR mask never written, 155 executed instructions at the default quantum |

`rom_masked` is the E2-15 masked-ROM control. It contains **no** SR-writing opcode
(no `MOVE` to SR, `ORI`/`ANDI`/`EORI` to SR, `STOP` or `RTE`), so it stays at the
68000 reset interrupt-priority mask 7 for the entire run; the oracle also takes no
interrupt at mask 7. Its 155-instruction run crosses two HBlank scheduling
boundaries (ticks 64 and 128 at quantum 64). Its level-4/level-6 autovector table
entries at `0x70`/`0x78` point at distinct `BRA.S *` stubs at `0x40`/`0x48`, so any
gating regression that delivered an interrupt would park the CPU at a detectable
address and diverge from the oracle rather than silently passing.

The ROMs explicitly initialise every compared data/address register they use; the
reset vector sets the same stack pointer on both sides. This matters because the
oracle starts the 68000 with undefined D0–D7 and A0–A6 (`0xFFFFFFFF`) while
ChronoFork zeroes them. VDP data-port **reads** are deliberately absent: the
oracle's VRAM read prefetch differs from the minimal VDP model, so only VDP
control/data-port **writes** are exercised and compared as part of the instruction
stream.

## Method and controlled conditions

1. **Oracle trace.** Run `exodus-probe` for one frame with
   `--trace-device "Main 68000"` and dump the trace. The format is
   `cycle, address(hex), opcode, args, time`; the `opcode` column is a human-readable
   mnemonic+args rendering, and the oracle's `cycle` and `time` columns are
   oracle-specific and are never compared.
2. **ChronoFork trace.** Run `chronofork megadrive trace` with the same ROM, an
   instruction budget, `--stop-pc <marker>` and `--ram-bytes 64`. `rom_alu` and
   `rom_mem_vdp` pass an explicit `--instructions-per-line 1000000`; `rom_masked`
   **omits** the flag and therefore runs at the CLI default quantum of 64. The
   harness reads `instructions_per_line` back from each state file and records it
   per ROM in JSON, Markdown and console output.
3. **Projection.** Reduce both traces to the ordered sequence of executed
   instruction addresses up to but excluding the marker self-loop. The oracle keeps
   executing the marker loop until the frame ends, so its total record count is
   larger than the projected count; the loop is excluded on both sides. Repeated
   addresses (loops and branches) are retained.
4. **Strict comparison.** Assert equal projection length and byte-for-byte equal
   address sequences. On mismatch, report the first divergent index, expected and
   actual address, five preceding records, and exit 1. Every ChronoFork projected
   opcode word is additionally checked against the ROM word at that address
   (`opcodes_match_rom_bytes`); the oracle mnemonic and the raw opcode word are not
   compared directly because they are different formats.
5. **Final state.** After the trace run, query the oracle over its JSON-RPC MCP
   surface (`session.load_module`, `session.load_rom`, `run.step_frames`,
   `cpu.registers`, `cpu.read_memory` at `0xFF0000` length 64) in a private base
   path. Compare D0–D7, A0–A7, PC, effective SR fields (IPM/S/CCR X/N/Z/V/C) and
   64 work-RAM bytes.
6. **Negative control.** Corrupt one projected oracle record in a copy and require
   the comparator to exit 1 and report that exact index.
7. **Scheduler/gating control.** Read the per-ROM quantum from the state file and
   derive the instruction-quantum scanline boundaries from the recorded tick,
   quantum and video standard. For `rom_masked`, additionally require more than one
   full quantum, at least one HBlank boundary, SR mask 7 on both sides, no
   SR-writing opcode executed or present in the program region, and no handler-stub
   entry. Derived boundary counts are ChronoFork control quantities, not oracle
   timing comparisons.
8. **Determinism.** Run the oracle probe twice and the ChronoFork trace twice per
   ROM; require byte-identical trace files and states across runs. This historical
   evidence was also reproduced into fresh output directories (an ignored
   `.research/evidence/repro-e2-15/` directory and `/tmp/chronofork-e2-15-verify/`)
   with identical behavioral evidence.

Controlled conditions that make the comparison meaningful in this slice:

- **Per-ROM scheduler quantum.** `rom_alu` and `rom_mem_vdp` use an explicit
  1,000,000-instruction quantum, so no HBlank/VBlank request is raised inside
  their short measured windows and both sides run an interrupt-free stream. Their
  results are unchanged from the preserved Phase-10 run.
- **Masked default-quantum window.** `rom_masked` runs at the CLI default quantum
  of 64 with `--instructions-per-line` omitted. The program contains no
  SR-writing opcode, so the 68000 reset interrupt-priority mask 7 holds throughout;
  HBlank requests raised at ticks 64 and 128 stay pending and no handler runs. The
  pinned oracle also honours mask 7 and takes no interrupt. This is negative
  evidence for gating, not interrupt-delivery equivalence.
- **Marker-bounded windows.** Each ROM ends in a known self-loop; comparing only the
  pre-marker addresses removes frame-length and loop-count differences from the
  strict comparison.
- **Explicit register initialisation.** Every ROM writes every compared register, so
  the oracle's undefined reset data/address values cannot affect the result.
- **Effective, not raw, SR.** The oracle reports `0x7FE0` at rest while ChronoFork
  reports `0x2700`; only IPM, S and CCR are compared.
- **No VDP read comparison.** VDP writes are executed; VDP data-port readback is
  excluded because the minimal VDP model does not reproduce oracle prefetch.
- **Same ROM bytes on both sides.** ROM SHA-256 is recorded and the oracle and
  ChronoFork runs consume the same generated file.

## Pre-repair divergence (preserved history)

The original evidence run (pre-repair binary
`ee465010…`, harness `8230ab26…`) passed `rom_alu` and found a real divergence in
`rom_mem_vdp`.

| Item | Pre-repair value |
|---|---|
| Oracle total records | 2363 |
| Oracle projected records | 51 |
| Oracle projected SHA-256 | `ba4f39489fa56b5fa068af4f4bd21e5aaf311a8138df22231f6c0e04bfe62b47` |
| ChronoFork retained records | 29 |
| ChronoFork executed records | 28 |
| ChronoFork projected records | 28 |
| ChronoFork projected SHA-256 | `d25857b3328c09776b48dbe0d7438041ea2181f276f823b4adafbec08e17e6a2` |
| First divergence | index **28**, expected `0x00027A`, actual missing |
| Reason | record-count mismatch (expected 51 projected records, got 28) |
| Oracle instruction at `0x00027A` | `MOVE.l $00FF0020, D1` (opcode `0x2239`) |
| ChronoFork failure | `illegal opcode 0x2239 at 0x00027A` |
| ChronoFork stopped state | 28 executed, PC `0x0000027C`, raw SR `0x2704` |
| Negative control | index 25 caught, exit 1 (passed) |

The five preceding projected records matched at `0x000262`, `0x000266`,
`0x00026C`, `0x000272` and `0x000278`. ChronoFork retained 29 records because the
rejected instruction is recorded after its opcode fetch and before decode, while
only 28 instructions completed.

**Root cause.** The real 68000 absolute-long source mode is `111/001`, giving load
opcodes `0x1039`/`0x2039`/`0x3039` for `MOVE.B/W/L (abs).L,Dn`. The pre-repair core
decoded the absolute-*word* encoding (`0x1038`/`0x2038`/`0x3038`, source mode
`111/000`) while fetching a 4-byte address, and the in-repo demo ROM/tests used the
same non-standard load form, so they passed against the same wrong assumption. The
generated differential ROM used the real `0x2239` long form.

**Repair (EPIC-0002 E2-13).** The decoder now matches source mode/register `111/001`
(`0x1039`/`0x2039`/`0x3039`) with a 4-byte address fetch; the `0x..38` forms fall
through to the typed `IllegalOpcode` error. The demo ROM's absolute-long loads were
changed from `0x3438`/`0x3438`/`0x3638` to `0x3439`/`0x3439`/`0x3639` with the
logical program and demo output unchanged, and regression tests cover byte/word/long
`(abs).L,Dn` loads, rejection of the `0x..38` forms and the `Dn,(abs).L` store
encodings. The pre-repair evidence is preserved under
`.research/evidence/before-repair/` (JSON, Markdown and artifacts); it was copied,
not overwritten.

## Historical 3-ROM trace results (preserved)

Projection = executed-address sequence up to but excluding the marker loop.

| ROM | Quantum | Oracle total records | Oracle projected | ChronoFork retained records | ChronoFork executed | ChronoFork projected | Addresses match |
|---|---|---:|---:|---:|---:|---:|---|
| `rom_alu` | 1,000,000 | 2375 | 99 | 99 | 99 | 99 | **yes** |
| `rom_mem_vdp` | 1,000,000 | 2363 | 51 | 51 | 51 | 51 | **yes** |
| `rom_masked` | **64 (CLI default)** | 2388 | 155 | 155 | 155 | 155 | **yes** |

### `rom_alu` — pass

- Oracle projected SHA-256:
  `2b6f191cf42101174b7b64b2272cf6277c72ff62a9608eeb23ca977fa70df569`
- ChronoFork projected SHA-256: the same digest.
- First divergence: none.
- Oracle trace dump SHA-256: `a9a8c02d3e9b67177367afb54d050d0e89c73cb3989cf93eb7391fb813802528`
  (identical across both runs).
- ChronoFork trace SHA-256: `b661c49d2e670793b4a774d82232b69f65e792c017fb5f83a74d728bbde59302`
  (identical across both runs); state SHA-256:
  `c7aa97e22f4347f6d466f0d32cdf938773c67ce59df2594062a7f7d363cedcd4`.
- Opcode audit: oracle projected mnemonic+args SHA-256
  `75b26e38de2db0f4eb7c76581aea71b4239101de5618c492ad01a62182a2df0f`;
  ChronoFork projected opcode-word SHA-256
  `27c1cf052e4d8152f671e7355fe43b7334ab876f992f107fdaa71d72c38daf19`;
  every projected opcode matches the ROM word, first mismatch none.
- The oracle's 2375 total records include the marker loop to the end of the frame;
  the 99 projected records are the compared instruction stream.

### `rom_mem_vdp` — pass after repair

- Oracle projected SHA-256:
  `ba4f39489fa56b5fa068af4f4bd21e5aaf311a8138df22231f6c0e04bfe62b47`
- ChronoFork projected SHA-256: the same digest.
- First divergence: none. Before repair this was index 28 at `0x00027A`.
- Oracle trace dump SHA-256: `1f41315142fc1a18002b8943778e5cb42325f4a7521f23de8d77fb1bc6af5a23`
  (identical across both runs).
- ChronoFork trace SHA-256: `79e0e4333d9bb65dcb6382b2bae082f1877723381e158b9c9afcf29db954e09b`
  (identical across both runs); state SHA-256:
  `87ef30c2f97de62dc848fd5d77caa4a6ed341b4bbc2b69330078efcef7bce9a0`.
- Opcode audit: oracle projected mnemonic+args SHA-256
  `df6322a157ba4108d3fe0f5d45e4f27f3acb740e308451475f315f07d5516d7e`;
  ChronoFork projected opcode-word SHA-256
  `eebc428599cbcd96c6fffec1ad63c055806ca2235932edc4831503e91b545c9e`;
  every projected opcode matches the ROM word, first mismatch none.
- The repaired stream now executes `MOVE.l $00FF0020, D1` at `0x00027A`
  (ChronoFork record `28  00027a  2239`, ROM word `0x2239`) and continues through
  all 51 projected records to the marker.

### `rom_masked` — pass at the CLI default quantum

- Oracle projected SHA-256:
  `02891e15765a342496196564e17d1649f10d39a069b4494fa96d7fb11ca3cd11`
- ChronoFork projected SHA-256: the same digest.
- First divergence: none.
- Oracle trace dump SHA-256:
  `2924bda26dad3c5b69a24dd076a7fdcb141fc31cda0e6b8318891bdff8f21032`
  (identical across both runs).
- ChronoFork trace SHA-256:
  `870c850de53422c97a7ecd168766ba0ece135142a5abd0f85b65670f61ec7dc5`
  (identical across both runs); state SHA-256:
  `8426771b2319596861fa377e88225961d83a3f0ba9e8637f5e31c7f1f04162a4`.
- Opcode audit: oracle projected mnemonic+args SHA-256
  `51a168e07a3666046a2ae10cef0e4086e14df072708ebcde2ed64d05793a3435`;
  ChronoFork projected opcode-word SHA-256
  `03b45e8715f25aca92886430dae61dfe03151a2015a8120543843cdeffe5e71a`;
  every projected opcode matches the ROM word, first mismatch none.
- All 155 projected records match the oracle, so the oracle also executed no
  handler and the two sides agree on the plain masked stream. The oracle's 2388
  total records include the marker loop to the end of the frame.

### Masked-ROM default-quantum gating control (E2-15)

`rom_masked` was run **without** `--instructions-per-line`. The state file records
`instructions_per_line=64`, so the harness verifies the CLI default rather than
imitating it with an explicit flag.

| Item | Observed |
|---|---|
| Per-ROM quantum | 64 (`cli-default`, flag omitted) |
| Executed instructions / tick | 155 / 155 |
| Full quanta completed | 2 scanlines (`tick // 64`) |
| HBlank boundaries crossed | 2 (event ticks 64 and 128) |
| VBlank boundaries crossed | 0 |
| SR mask, ChronoFork / oracle | 7 / 7 |
| SR-writing opcodes executed | 0 |
| SR-writing opcodes in program region | 0 |
| Handler-stub entries (`0x40`, `0x48`, `0x70`, `0x78`) | 0 |
| Control checks | all passed |

The program's arithmetic self-checks final registers D0=`0x00000000`,
D1=`0x00000001`, D3=`0x00000035`, D4=`0x00000036`, D5–D7=`0x00000055`, and its
work-RAM signature covers `0x00FF0000`–`0x00FF0013` (see the final-state section).
Because the program contains no SR-writing opcode and no exception is entered,
the SR mask is 7 at every instruction boundary; the harness still checks the final
mask on both sides and scans the program region for SR-writing opcodes rather than
assuming it.

The autovector table entries at `0x70`/`0x78` point at `BRA.S *` stubs, so a gating
regression would park the CPU at `0x40`/`0x48` and break the projected-stream and
final-state comparisons. The pass is therefore **negative evidence for mask
gating** — no handler executes while masked — not a positive comparison of
interrupt delivery, acknowledge, nesting or interrupt timing.

## Historical 3-ROM final-state comparison (preserved)

Method: after one frame, one MCP session per ROM reads `cpu.registers` for
`Main 68000` and `cpu.read_memory` for 64 bytes at `0xFF0000`; ChronoFork writes the
same fields to a deterministic key=value state file. Raw SR words are recorded but
not compared; effective IPM, S and CCR fields are compared instead. All three ROMs
match on registers, effective SR and 64 work-RAM bytes.

### `rom_alu` — match

- PC: oracle `0x00000352`, ChronoFork `0x00000352`.
- Effective SR: IPM 7, S=1, CCR=0 on both sides (raw oracle `0x7FE0`, raw
  ChronoFork `0x2700`).
- D0–D7: `0x00000001`, `0x00000001`, `0x00000012`, `0x00000002`, `0x00000000`,
  `0x00000012`, `0x00000012`, `0x00000012` on both sides.
- A0–A7: `0x00FF1000`, `0x00FF1010`, `0x00FF1020`, `0x00FF1030`, `0x00FF1040`,
  `0x00FF1050`, `0x00FF1060`, `0x00FFFF00` on both sides.
- Work RAM at `0xFF0000`:
  `00000001000000020000001200000002000000000000001200000012000000121200001200000000000000000000000000000000000000000000000000000000`
  on both sides.

### `rom_mem_vdp` — match

- PC: oracle `0x000002E0`, ChronoFork `0x000002E0`.
- Effective SR: IPM 7, S=1, CCR=0 on both sides (raw oracle `0x7FE0`, raw
  ChronoFork `0x2700`).
- D0–D7: `0x11223344`, `0x00000001`, `0x11223344`, `0xAABBCCDD`, `0x0000CCDD`,
  `0x0000CCDD`, `0x000000DD`, `0x000000DD` on both sides.
- A0–A7: `0x00FF1000`, `0x00FF1010`, `0x00FF1020`, `0x00FF1030`, `0x00FF1040`,
  `0x00FF1050`, `0x00FF1060`, `0x00FFFF00` on both sides.
- Work RAM at `0xFF0000`:
  `000000000000000000000000000000000000000000000000000000000000000011223344ccdddd000000000000000000112233440000ccdd000000ddccdd0000`
  on both sides.

### `rom_masked` — match

- PC: oracle `0x000002EE`, ChronoFork `0x000002EE`.
- Effective SR: IPM 7, S=1, CCR=0 on both sides (raw oracle `0x7FE0`, raw
  ChronoFork `0x2700`).
- D0–D7: `0x00000000`, `0x00000001`, `0x00000005`, `0x00000035`, `0x00000036`,
  `0x00000055`, `0x00000055`, `0x00000055` on both sides.
- A0–A7: `0x00FF1000`, `0x00FF1010`, `0x00FF1020`, `0x00FF1030`, `0x00FF1040`,
  `0x00FF1050`, `0x00FF1060`, `0x00FFFF00` on both sides.
- Work RAM at `0xFF0000`:
  `00000037000000350000003600000000370000550000000000000000000000000000000000000000000000000000000000000000000000000000000000000000`
  on both sides. This encodes the stored signature (55, 53, 54, 0, byte 0x37,
  word 0x0055) inside the compared 64-byte window.

## Negative control

The comparator is driven from the oracle projection; one projected record is
corrupted in a copy and the comparator must exit 1 and report the corrupted index.
All three historical controls passed.

| ROM | Corrupted index | Reported index | Exit code | Status |
|---|---:|---:|---:|---|
| `rom_alu` | 49 | 49 | 1 | passed |
| `rom_mem_vdp` | 25 | 25 | 1 | passed |
| `rom_masked` | 77 | 77 | 1 | passed |

## Determinism and reproduction of the result

| ROM | Oracle trace run 1 = run 2 | ChronoFork trace run 1 = run 2 | ChronoFork state run 1 = run 2 |
|---|---|---|---|
| `rom_alu` | yes (byte-identical) | yes (byte-identical) | yes (byte-identical) |
| `rom_mem_vdp` | yes (byte-identical) | yes (byte-identical) | yes (byte-identical) |
| `rom_masked` | yes (byte-identical) | yes (byte-identical) | yes (byte-identical) |

The E2-15 evidence was generated three times into separate output directories
(the canonical `.research/evidence/`, a fresh ignored
`.research/evidence/repro-e2-15/`, and `/tmp/chronofork-e2-15-verify/`) and produced
identical behavioral evidence: ROM hashes, record counts,
projection/mnemonic/opcode/trace/state digests, final-state match, per-ROM quantum,
HBlank control values and negative-control indices. The harness exits 0.

## Evidence locations

All of this is ignored by Git and is not distributed with ChronoFork:

- `.research/evidence/oracle-diff.json` and `oracle-diff.md` — canonical E2-15
  evidence (schema v2, three ROMs, per-ROM quantum).
- `.research/evidence/before-e2-15/` — preserved, unchanged canonical Phase-10
  evidence from before the E2-15 regeneration: the old `oracle-diff.json`
  (SHA-256 `f6307be0ded33aca07de6c92e030c3eae19b6ef005a8f200d336060a211f52a7`),
  `oracle-diff.md`
  (SHA-256 `420013a52e01f633fbe9cdf2894ff985c0ff4e371eba4e5bd1a5143125dafb29`)
  and `artifacts/` (two ROMs, explicit 1,000,000-instruction quantum, no
  masked-ROM run), plus `PRESERVED.md` and a verified `SHA256SUMS` manifest.
- `.research/evidence/repro-e2-15/` — fresh ignored reproduction of the canonical
  E2-15 run with identical behavioral evidence.
- `.research/evidence/before-repair/` — preserved original pre-repair evidence
  (JSON/Markdown/artifacts) that recorded the index-28 divergence.
- `.research/evidence/after-repair/` — preserved audit run from the E2-13 repair.
- `.research/evidence/STATUS.md` — post-repair E2-13 status summary and the exact
  pre/post record pair for `0x00027A`.
- `.research/evidence/artifacts/<rom>/` — trace dumps, projections, final-state
  files, MCP transcripts, stderr/stdout and negative-control files for the canonical
  E2-15 run.

## Evidence commands

Harness (the current script generates the thirteen-ROM corpus, runs both tools twice
per ROM, compares address streams, per-instruction cycles and final state, runs
negative controls and writes the evidence files). The commands below are
historical: they now run the current corpus, not the 3-ROM result on this page.

```sh
python3 scripts/oracle-diff.py --out-dir .research/evidence
```

Fresh-directory reproduction and the independent `/tmp` check:

```sh
python3 scripts/oracle-diff.py --out-dir .research/evidence/repro-e2-15
python3 scripts/oracle-diff.py --out-dir /tmp/chronofork-e2-15-verify
```

Preserving the pre-E2-15 canonical evidence before regenerating (already done;
copy, never overwrite):

```sh
cd .research/evidence
test ! -e before-e2-15
mkdir -p before-e2-15
cp -a oracle-diff.json oracle-diff.md artifacts before-e2-15/
cd before-e2-15
find . -type f -print0 | sort -z | xargs -0 sha256sum > SHA256SUMS
sha256sum -c SHA256SUMS --quiet
```

Per-ROM oracle probe (run 1; run 2 is the same command with `run2` output names;
working directory is the pinned oracle checkout, so the module path resolves):

```sh
.research/oracle/build/exodus-probe \
  --rom .research/evidence/artifacts/roms/<rom>.bin \
  --frames 1 --no-mkdir \
  --trace-dump .research/evidence/artifacts/<rom>/oracle.run1.trace \
  --trace-device "Main 68000" \
  "Data/Modules/Sega Mega Drive 1600.xml"
```

Per-ROM ChronoFork trace (run 1; markers: `0x354` `rom_alu`, `0x2E2`
`rom_mem_vdp`, `0x2F0` `rom_masked`). `rom_alu` and `rom_mem_vdp` pass the explicit
1,000,000 quantum; `rom_masked` **omits** `--instructions-per-line` so the CLI
default of 64 applies and the state file records it:

```sh
# rom_alu and rom_mem_vdp
target/debug/chronofork megadrive trace \
  --rom .research/evidence/artifacts/roms/<rom>.bin \
  --instructions 10000 --stop-pc <marker> \
  --instructions-per-line 1000000 --ram-bytes 64 \
  --trace-out .research/evidence/artifacts/<rom>/chronofork.run1.trace \
  --state-out .research/evidence/artifacts/<rom>/chronofork.run1.state

# rom_masked: no --instructions-per-line (CLI default 64)
target/debug/chronofork megadrive trace \
  --rom .research/evidence/artifacts/roms/rom_masked.bin \
  --instructions 10000 --stop-pc 0x2F0 --ram-bytes 64 \
  --trace-out .research/evidence/artifacts/rom_masked/chronofork.run1.trace \
  --state-out .research/evidence/artifacts/rom_masked/chronofork.run1.state
```

Oracle final-state MCP sequence (one session per ROM, private `--base-path`):

```text
initialize -> notifications/initialized
session.load_module {"path": "Data/Modules/Sega Mega Drive 1600.xml"}
session.load_rom {"path": "<rom>"}
run.step_frames {"frames": 1}
cpu.registers {"device": "Main 68000"}
cpu.read_memory {"device": "Main 68000", "address": "0xFF0000", "length": 64}
shutdown
```

Preserving the pre-repair E2-13 baseline (done before that repair) used the same
copy-not-overwrite pattern:

```sh
cd .research/evidence
mkdir -p before-repair
cp -a oracle-diff.json oracle-diff.md artifacts before-repair/
```

## Limitations

- **Instruction-address projection and selected state are not cycle equivalence.**
  Oracle cycle and time columns are ignored; the oracle sees a different absolute
  start time, and ChronoFork's tick model is one instruction per tick. No timing,
  prefetch, bus-cycle, scheduler-rate or per-instruction-cost equivalence is claimed.
- **Interrupt timing and interrupt delivery are not compared.** The masked-ROM
  result is **negative evidence for mask gating**: at SR mask 7 the program contains
  no SR-writing opcode, the scheduler raises HBlank boundaries behind it, no handler
  stub is entered and both sides agree on the plain instruction stream and final
  state. It does not compare interrupt delivery order, acknowledge semantics,
  nesting, latency, vector fetch or handler execution against the oracle. The
  E2-15 gating/RTE/VINT behaviour is covered by Rust tests, not by this differential
  evidence.
- **The explicit 1,000,000-instruction quantum is a controlled window, not
  equivalence.** `rom_alu` and `rom_mem_vdp` avoid interrupts that way; `rom_masked`
  uses the CLI default 64 and stays masked instead. The harness reads the quantum
  back from each state file and reports it; it does not prove that a different
  quantum would reproduce the same observable stream.
- **Three synthetic 4096-byte ROMs are not general conformance.** All pass now, but
  the evidence does not establish the full 68000 instruction set, all addressing
  modes, exception behaviour or arbitrary program behavior.
- **Final-state coverage is narrow.** Only D0–D7, A0–A7, PC, effective SR and 64
  work-RAM bytes at `0xFF0000` are compared. VRAM, CRAM, VSRAM, the full 64 KiB of
  RAM, VDP registers/status beyond program-visible effects, device state and
  scheduler/interrupt state are not compared. The derived HBlank boundary counts are
  ChronoFork control quantities from the documented instruction-quantum model, not
  oracle timing measurements.
- **Opcodes are audited, not cross-format compared.** The oracle dump exposes
  mnemonic+args while ChronoFork exposes raw opcode words; agreement rests on the
  identical address streams, ROM-byte verification of every projected opcode, the
  ROMs' self-checking flag branches and the final-state comparison.
- **Raw SR words at the recorded final state are not directly comparable** (oracle
  `0x7FE0`, ChronoFork `0x2700`); only effective fields are compared.
- **Reset CCR/X differs before ROM arithmetic.** The oracle powers on with SR
  `0x7FFF` (CCR `0x1F`, so `X=1`), while ChronoFork resets to `0x2700` (CCR `0`).
  `MOVE` preserves `X`, so a MOVE-only differential window can differ in `X` even
  when `N`/`Z`/`V`/`C` match. The harness ROMs clear `X` with arithmetic, so the
  recorded final effective-SR equality is valid for these ROMs; it is not a general
  reset-flag equivalence.
- **The oracle's reset data/address registers are undefined** (`0xFFFFFFFF` for
  D0–D7 and A0–A6); every ROM initialises every compared register it uses and the
  reset vector supplies the same stack pointer.
- **VDP data-port reads are excluded** because the oracle's VRAM read prefetch
  differs from the minimal VDP model. VDP writes are included in the instruction
  stream only; no rendering, DMA, CRAM/VSRAM, raster timing or VDP data equality is
  claimed.
- **No audio, Z80, controller, or full-hardware claim.** Only the 68000 instruction
  stream and selected state above are observed.
- **No commercial ROM, BIOS or game-compatibility claim.** The ROMs are small,
  independently authored synthetic programs.
- **The oracle trace ring is bounded** (default 1,000,000 entries). These runs use at
  most 2388 records, so no start-of-run records were dropped; a longer run could
  silently evict its beginning.
- **The oracle binary is pinned by commit and SHA-256, not reproduced
  bit-for-bit.** Rebuilding the same commit with the recorded toolchain is expected
  to reproduce behaviour, not necessarily an identical executable.
- **Evidence artifacts are not committed.** Oracle code/binaries, ROMs, traces,
  transcripts and the generated evidence JSON/Markdown live under ignored
  `.research/` paths and are not distributed with ChronoFork.

## Reproduction steps

Prerequisites:

1. Ubuntu 24.04-class Linux with `cmake` 3.28, `ninja` 1.11, `clang`/`clang++` 18,
   `pkg-config`, Python 3 and the expat/zlib/libpng/libjpeg development packages
   listed above.
2. The pinned oracle checkout at
   `.research/upstream/exodus-headless` (commit
   `ec0056151f3ff1c334d09ee375abb30a68fdfec5`, branch `linux-headless`).
3. ChronoFork's CLI built with `cargo build -p chronofork-cli`.

Steps:

```sh
# 1. Build the oracle at the pinned commit.
cd .research/upstream/exodus-headless
cmake -S . -B build -G Ninja \
  -DCMAKE_C_COMPILER=clang -DCMAKE_CXX_COMPILER=clang++ \
  -DCMAKE_BUILD_TYPE=Release
cmake --build build -j"$(nproc)"

# 2. Put the binaries where the harness expects them, or pass --oracle-bin-dir.
mkdir -p ../../oracle/build
cp build/exodus-probe build/exodus-mcpd ../../oracle/build/

# 3. Build the ChronoFork CLI.
cd ../../..
cargo build -p chronofork-cli

# 4. Run the current thirteen-ROM differential harness with its own evidence directory.
python3 scripts/oracle-diff.py --out-dir .research/evidence

# 5. Inspect the recorded result and per-ROM artifacts.
sha256sum .research/evidence/oracle-diff.json .research/evidence/oracle-diff.md
```

Expected exit code: **1** — the command above runs the **current thirteen-ROM
master-cycle** harness, not the 3-ROM result documented on this page. Address
projections pass 13/13 and the address/cycle/final-state negative controls pass
13/13, but ``rom_exception`` reports two scoped oracle-side divergences (the
oracle's saved-SR reserved bit and its 38-vs-34 ``TRAP`` cost), so the summary is
``DIVERGENCE`` and the harness exits 1 until that comparison is adjudicated; see the
W4.2/W4.3 review note and the
[68000 cycle differential evidence](cpu-cycle-differential-evidence.md). The 3/3
and quantum-64 material on this page is retained historical evidence and is **not**
reproduced by this command. To reproduce the historical pre-repair FAIL, check out
the pre-repair core revision and harness and preserve its output under a separate
directory before running again.

The harness writes `oracle-diff.json` (machine-readable, schema v2) and
`oracle-diff.md` (human-readable) plus per-ROM artifacts under
`.research/evidence/artifacts/<rom>/`: both trace dumps per run, projections,
final-state files, MCP transcripts, stderr/stdout and negative-control files. All of
this is ignored by Git. The preserved pre-E2-15 evidence under
`.research/evidence/before-e2-15/` is not touched by a new run.

## Provenance

The oracle is used as an **external process** at a pinned revision. No upstream
Exodus/exodus-for-ais source code, headers, fixtures or binaries are incorporated
into ChronoFork or committed to the repository; the harness is original Python 3
standard-library code and the ROM generator is original. Oracle binaries, ROMs,
traces and MCP transcripts remain under ignored `.research/` paths and are not
redistributed. Because no upstream code or fixture was incorporated,
`THIRD_PARTY_NOTICES.md` does not change as a result of this evidence. See
[provenance](../licensing/provenance.md) for the upstream license and the open
per-file copyright question that must be resolved before any future reuse.
