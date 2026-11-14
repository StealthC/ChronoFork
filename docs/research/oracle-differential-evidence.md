# Oracle Differential Instruction-Trace Evidence

- **Status:** Final post-repair evidence **PASS 2/2** against the pinned Exodus
  headless oracle: both synthetic ROMs have identical projected address streams,
  equal selected final state and passing negative controls. The earlier pre-repair
  divergence (real 68000 absolute-long `MOVE` decoding defect) is preserved below
  as history and was fixed under [EPIC-0002](../epics/EPIC-0002-megadrive-slice.md)
  E2-13.
- **Scope:** instruction-address streams, opcode/ROM-byte consistency and selected
  final architectural state for two independently authored synthetic ROMs. **Not**
  cycle accuracy, interrupt behaviour, rendering, audio, Z80, DMA or game
  compatibility.
- **Related:** [Mega Drive 68000 slice](../architecture/megadrive-slice.md),
  [EPIC-0002](../epics/EPIC-0002-megadrive-slice.md),
  [provenance](../licensing/provenance.md),
  [testing strategy](../quality/testing-strategy.md).

## Summary

The reproducible harness `scripts/oracle-diff.py` runs the pinned Exodus headless
oracle and ChronoFork's bounded instruction trace over the same generated ROM, then
compares the executed-address sequence, opcode/ROM-byte consistency and selected
final state.

| ROM | Projected addresses | Projection SHA-256 | Final state | Negative control |
|---|---|---|---|---|
| `rom_alu` | 99/99 identical | `2b6f191c…` on both sides | D0–D7, A0–A7, PC, effective SR and 64 work-RAM bytes equal | index 49 caught, exit 1 |
| `rom_mem_vdp` | 51/51 identical | `ba4f3948…` on both sides | D0–D7, A0–A7, PC, effective SR and 64 work-RAM bytes equal | index 25 caught, exit 1 |

Overall post-repair summary: `status: pass`, 2/2 trace matches, 2/2 final-state
matches, 2/2 negative controls, harness exit 0. The canonical evidence is
`.research/evidence/oracle-diff.{json,md}`; an identical audit run is under
`.research/evidence/after-repair/`, and the preserved pre-repair run is under
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
| ChronoFork commit | `ec4c1ca7c36558e3d95f0463ee4e4d3cd9902c04` (last commit; evidence worktree dirty) |
| ChronoFork post-repair binary SHA-256 | `17f7e4f8fabe14ef38886b392cf6c642c0c744555fa39631dc219202bbfbe911` |
| Post-repair harness SHA-256 | `241a62f321b79ecd07577e56f0104b87953a0a97219cae47bf7dbbbac4543403` |
| Pre-repair ChronoFork binary SHA-256 | `ee46501022034f19f364657ccd9247a52f5361ea5dcb877d7f8cc6e7596b30ed` |
| Pre-repair harness SHA-256 | `8230ab267419bd483b842b3bba20ed09f507e92696da9dddd5367ce2f157e970` |
| Scheduler quantum | 1,000,000 instructions per scanline (explicit) |
| Compared RAM | 64 bytes at `0x00FF0000` |

The ChronoFork entry is the last repository commit plus uncommitted trace/CLI/
harness/repair work; the exact binary digest is therefore recorded rather than
relying on the commit alone. No commit or push was made for this evidence. The
pre-repair binary and harness digests are listed so the historical FAIL can be tied
to exact artifacts.

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

A separate build-validation run executed a 1024-byte synthetic smoke ROM and
produced **2367** trace records starting at reset PC `0x200`, with two runs
byte-identical at SHA-256
`d720aa8417838894ddbcf868ac6fac33b092a5a6b92173da87bcb23726cd9fc4`. That smoke ROM
is not one of the two differential ROMs below; its count and digest are recorded
here only as build/probe evidence and must not be conflated with the differential
numbers.

## Synthetic differential ROMs

Both ROMs are 4096 bytes, independently authored and generated deterministically by
`scripts/oracle-diff.py`. They contain the Mega Drive header fields needed by the
oracle loader, load the initial stack pointer and PC from the reset vectors, and end
in a marker self-loop. Neither ROM uses absolute-word loads, PC-relative addressing,
`RTE`, or any instruction outside the documented ChronoFork subset.

| ROM | SHA-256 | Size | Marker PC | Purpose |
|---|---|---|---|---|
| `rom_alu` | `f80bf48a29a428b6f3399d013103a7c41c7de13a058dfc7ae1bd67901aee563f` | 4096 | `0x000352` | ALU, shifts, flags and branch conditions; no absolute-long loads |
| `rom_mem_vdp` | `9b43d7e9ebf8fa4182f5cf306489fa55a2b728267916fde71c54648f7e9d93ce` | 4096 | `0x0002E0` | Byte/word/long memory forms, absolute-long loads, VDP control/data writes |

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
   instruction budget, `--stop-pc <marker>`, `--ram-bytes 64` and an explicit
   `--instructions-per-line 1000000`.
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
7. **Determinism.** Run the oracle probe twice and the ChronoFork trace twice per
   ROM; require byte-identical trace files and states across runs. The post-repair
   evidence was also reproduced by an independent run into a separate output
   directory.

Controlled conditions that make the comparison meaningful in this slice:

- **Interrupt-free measured window.** The oracle honours the reset SR mask (`7`) and
  takes no interrupts. ChronoFork does **not** gate interrupts with the SR mask yet
  (EPIC-0002 E2-15), so the harness must pass an explicit
  `--instructions-per-line 1000000`; no interrupt is raised inside the measured
  window. The CLI's default quantum of 64 is unchanged and is never altered
  implicitly. This is a controlled workaround, not interrupt equivalence;
  interrupt delivery is not part of this evidence.
- **Marker-bounded windows.** Each ROM ends in a known self-loop; comparing only the
  pre-marker addresses removes frame-length and loop-count differences from the
  strict comparison.
- **Explicit register initialisation.** Both ROMs write every compared register, so
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

## Post-repair trace results

Projection = executed-address sequence up to but excluding the marker loop.

| ROM | Oracle total records | Oracle projected | ChronoFork retained records | ChronoFork executed | ChronoFork projected | Addresses match |
|---|---:|---:|---:|---:|---:|---|
| `rom_alu` | 2375 | 99 | 99 | 99 | 99 | **yes** |
| `rom_mem_vdp` | 2363 | 51 | 51 | 51 | 51 | **yes** |

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

## Post-repair final-state comparison

Method: after one frame, one MCP session per ROM reads `cpu.registers` for
`Main 68000` and `cpu.read_memory` for 64 bytes at `0xFF0000`; ChronoFork writes the
same fields to a deterministic key=value state file. Raw SR words are recorded but
not compared; effective IPM, S and CCR fields are compared instead. Both ROMs match
on registers, effective SR and 64 work-RAM bytes.

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

## Negative control

The comparator is driven from the oracle projection; one projected record is
corrupted in a copy and the comparator must exit 1 and report the corrupted index.
Both post-repair controls pass.

| ROM | Corrupted index | Reported index | Exit code | Status |
|---|---:|---:|---:|---|
| `rom_alu` | 49 | 49 | 1 | passed |
| `rom_mem_vdp` | 25 | 25 | 1 | passed |

## Determinism and reproduction of the result

| ROM | Oracle trace run 1 = run 2 | ChronoFork trace run 1 = run 2 | ChronoFork state run 1 = run 2 |
|---|---|---|---|
| `rom_alu` | yes (byte-identical) | yes (byte-identical) | yes (byte-identical) |
| `rom_mem_vdp` | yes (byte-identical) | yes (byte-identical) | yes (byte-identical) |

The post-repair evidence was generated three times into separate output directories
(`.research/evidence/after-repair/`, the canonical `.research/evidence/`, and an
independent `/tmp` run) and produced the same behavioral evidence: ROM hashes,
record counts, projection/mnemonic/opcode digests, final-state match and
negative-control indices. The harness exits 0 for the post-repair run.

## Evidence locations

All of this is ignored by Git and is not distributed with ChronoFork:

- `.research/evidence/oracle-diff.json` and `oracle-diff.md` — canonical post-repair
  evidence.
- `.research/evidence/after-repair/` — identical audit run and artifacts.
- `.research/evidence/before-repair/` — preserved original pre-repair evidence
  (JSON/Markdown/artifacts) that recorded the index-28 divergence.
- `.research/evidence/STATUS.md` — post-repair status summary and the exact
  pre/post record pair for `0x00027A`.
- `.research/evidence/artifacts/<rom>/` — trace dumps, projections, final-state
  files, MCP transcripts, stderr/stdout and negative-control files for the canonical
  run.

## Evidence commands

Harness (generates ROMs, runs both tools twice per ROM, compares and writes the
evidence files):

```sh
python3 scripts/oracle-diff.py --out-dir .research/evidence
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

Per-ROM ChronoFork trace (run 1; the marker is `0x352` for `rom_alu` and `0x2E0`
for `rom_mem_vdp`):

```sh
target/debug/chronofork megadrive trace \
  --rom .research/evidence/artifacts/roms/<rom>.bin \
  --instructions 10000 --stop-pc <marker> \
  --instructions-per-line 1000000 --ram-bytes 64 \
  --trace-out .research/evidence/artifacts/<rom>/chronofork.run1.trace \
  --state-out .research/evidence/artifacts/<rom>/chronofork.run1.state
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

Preserving a pre-repair baseline before regenerating evidence (as was done for the
E2-13 history):

```sh
cd .research/evidence
mkdir -p before-repair
cp -a oracle-diff.json oracle-diff.md artifacts before-repair/
```

## Limitations

- **Instruction-address projection is not cycle equivalence.** Oracle cycle and time
  columns are ignored; the oracle sees a different absolute start time, and
  ChronoFork's tick model is one instruction per tick. No timing, prefetch,
  bus-cycle, or scheduler-rate equivalence is claimed.
- **Interrupt behaviour is not compared.** ChronoFork lacks SR mask gating and
  `RTE` (E2-15); the measured window is kept interrupt-free with an explicit
  1,000,000-instruction quantum. Interrupt delivery equivalence remains untested.
- **Two synthetic 4096-byte ROMs are not general conformance.** Both pass now, but
  the evidence does not establish the full 68000 instruction set, all addressing
  modes, or arbitrary program behavior.
- **Final-state coverage is narrow.** Only D0–D7, A0–A7, PC, effective SR and 64
  work-RAM bytes at `0xFF0000` are compared. VRAM, CRAM, VSRAM, the full 64 KiB of
  RAM, device state and scheduler state are not compared.
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
  D0–D7 and A0–A6); both ROMs initialise every compared register they use and the
  reset vector supplies the same stack pointer.
- **VDP data-port reads are excluded** because the oracle's VRAM read prefetch
  differs from the minimal VDP model. VDP writes are included in the instruction
  stream only; no rendering, DMA, CRAM/VSRAM, raster timing or VDP data equality is
  claimed.
- **No audio, Z80, controller, or full-hardware claim.** Only the 68000 instruction
  stream and selected state above are observed.
- **The oracle trace ring is bounded** (default 1,000,000 entries). These runs use at
  most 2375 records, so no start-of-run records were dropped; a longer run could
  silently evict its beginning.
- **No commercial ROM, BIOS or game compatibility is claimed.**
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

# 4. Run the differential harness with its own evidence directory.
python3 scripts/oracle-diff.py --out-dir .research/evidence

# 5. Inspect the recorded result and per-ROM artifacts.
sha256sum .research/evidence/oracle-diff.json .research/evidence/oracle-diff.md
```

Expected post-repair exit code: **0** — both ROMs pass projection, final-state and
negative-control comparisons. To reproduce the historical pre-repair FAIL, check out
the pre-repair core revision and harness and preserve its output under a separate
directory before running again.

The harness writes `oracle-diff.json` (machine-readable) and `oracle-diff.md`
(human-readable) plus per-ROM artifacts under
`.research/evidence/artifacts/<rom>/`: both trace dumps per run, projections,
final-state files, MCP transcripts, stderr/stdout and negative-control files. All of
this is ignored by Git.

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
