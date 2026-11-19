# 68000 cycle differential evidence (Phase 8.2)

**Status:** executed and **PASS**. This page is the durable evidence for the
Phase 8.2 cycle comparison. It supersedes the M3 instruction-quantum timing
claims in [oracle-differential-evidence.md](oracle-differential-evidence.md)
and the quantum rows/fixtures in
[m3-hardware-acceptance.md](../quality/m3-hardware-acceptance.md).

## Method

Canonical time is the master-clock cycle. One scanline is exactly **3420 master
cycles**; each instruction advances the backend by `InstructionOutcome.cycles * 7`
(`MASTER_CYCLES_PER_68000_CYCLE = 7`). `chronofork megadrive trace --cycles`
appends the completed instruction's 68000 cycle cost as a fourth tab-separated
field. The harness compares that field with the pinned Exodus oracle's accumulated
`cycle` column: per projected instruction, the oracle cost is the difference to the
next oracle record, while ChronoFork's fourth field is the completed instruction
cost. Absolute cycle bases and the oracle `time` column are not compared.

The corpus has eleven independently authored synthetic ROMs: the three existing
ones (`rom_alu`, `rom_mem_vdp`, `rom_masked`) and eight later additions committed
under [`scripts/fixtures/`](../../scripts/fixtures/): EA forms (`rom_ea`),
shifts/rotates (`rom_shift`), BCD (`rom_bcd`), MOVEM/MOVEP (`rom_movem`),
multiply/divide (`rom_muldiv`), EXG/TAS/ADDX/SUBX (`rom_exg_tas`), the
system/status-register forms (`rom_status_reg`) and bit operations/DBcc
(`rom_bits_dbcc`). `scripts/oracle-diff.py` generates every ROM from
its own assembler and checks that the committed fixture bytes match. All register
setup uses explicit `MOVEA.L #imm,An` and the program clears `X` with `ADD.B
D0,D0` before the measured section (see the reset-state note under limits).

## Command and pinned inputs

```sh
python3 scripts/oracle-diff.py --skip-build \
  --chronofork-bin target/debug/chronofork \
  --out-dir .research/evidence/8.2-fix-r6
```

- Pinned oracle checkout: `.research/upstream/exodus-headless` at
  `ec0056151f3ff1c334d09ee375abb30a68fdfec5` (branch `linux-headless`).
- Pinned binaries: `.research/oracle/build/exodus-probe` (sha256
  `07c56c54e855cd4fdba2cf12df17b8079ff4540e0f3ab95486a42ea4cf912557`) and
  `.research/oracle/build/exodus-mcpd`
  (`7da8dcb00bc678dca813f1e56aed0c0deaa814613a7ad8609ce92212049f3ff6`).
- Raw artifacts: `.research/evidence/8.2-fix-r6/` (`oracle-diff.json`,
  `oracle-diff.md`, per-ROM oracle/ChronoFork traces, projections and states).
  These are ignored research artifacts; the durable summary is this page.
- Exit status of the recorded run: **0**.

## Results

- **Address projections: 11/11 match.** Every projected executed-address sequence
  agrees with the oracle, every ChronoFork opcode word matches the ROM bytes, and
  the address negative control passes 11/11.
- **Per-instruction cycles: 11/11 match.** Every compared instruction's cost agrees
  with the oracle, with no first mismatch on any ROM. The cycle negative control
  (first projected cost corrupted by one cycle) is detected 11/11.
- **Selected final state: 11/11 match.** Registers, effective SR and the compared
  64 work-RAM bytes agree on every ROM. The final-state negative control passes
  11/11.

| ROM | projected instructions | oracle total records | cycles match | first cycle mismatch | final state |
|---|---:|---:|---|---|---|
| `rom_alu` | 100 | 2375 | **yes** | none | yes |
| `rom_mem_vdp` | 52 | 2363 | **yes** | none | yes |
| `rom_masked` | 156 | 2389 | **yes** | none | yes |
| `rom_ea` | 24 | 2370 | **yes** | none | yes |
| `rom_shift` | 26 | 2369 | **yes** | none | yes |
| `rom_bcd` | 22 | 2372 | **yes** | none | yes |
| `rom_movem` | 22 | 2366 | **yes** | none | yes |
| `rom_muldiv` | 26 | 2338 | **yes** | none | yes |
| `rom_exg_tas` | 41 | 2370 | **yes** | none | yes |
| `rom_status_reg` | 39 | 2360 | **yes** | none | yes |
| `rom_bits_dbcc` | 42 | 2364 | **yes** | none | yes |

### Forms corrected by this repair

The previous run recorded first mismatches for taken conditional branches,
long register-source ADD/SUB/AND/OR, `TST.L`, register and memory shifts,
`JMP`/`JSR` control addressing, MOVEM mask-dependent cost and MUL early-out.
Those costs were corrected in the timing table and the same corpus now matches
end to end:

| form | documented cost now implemented |
|---|---|
| taken `BRA`/`BSR`/`Bcc`; untaken `Bcc` | `BRA` 10, `BSR` 18, taken `Bcc` 10, untaken byte 8 / word 12 |
| long `ADD`/`SUB`/`AND`/`OR` `<ea>,Dn` | 6 + EA, plus 2 when the source is `Dn`/`An`/immediate |
| memory-destination `/Dn,<ea>` and quick/immediate RMW | documented memory base plus the destination EA once |
| `CLR`/`NEG`/`NEGX`/`NOT` | 4/6 register, 8/12 + EA memory; `TST` is 4 + EA |
| register shift/rotate | 6 (byte/word) or 8 (long) + 2 per count; memory shift 8 + EA |
| `MOVEM` | direction/mode base + 4 (word) or 8 (long) per register |
| `MULU`/`MULS` | 38 + 2n from the source (set bits / bit transitions) |
| `DIVU`/`DIVS` | 140 / 168 worst-case base, as in the oracle |
| `JMP`/`JSR`/`LEA`/`PEA` | control-addressing tables (for example `JMP (xxx).L` 12, `JSR (xxx).L` 20, `LEA (xxx).L` 12, `PEA (xxx).L` 20) |

The repair also fixed a real BCD addition bug found by the final-state
comparison: `ABCD` must apply the decimal correction when the low-nibble sum
exceeds nine even if the binary result's low digit does not (`0x19 + 0x27` is
`0x46`, not `0x40`). The BCD ROM now agrees with the oracle in register D1 and
work RAM.

### Forms added by the encoding-overlap repair

The documented-instruction audit found three encodings that were not implemented
and, for EXG and ADDX/SUBX, silently executed as a different instruction because
they share the AND/ADD/SUB family masks. They are now their own decode families
with unit tests and are covered by the new `rom_exg_tas` differential:

| instruction | forms now implemented | oracle coverage |
|---|---|---|
| `EXG` | `Dx,Dy`, `Ax,Ay`, `Dx,Ay`; 6 cycles; no flag changes | three register-exchange forms, final state |
| `ADDX`/`SUBX` | all sizes; `Dn,Dn` and `-(An),-(An)`; X folded into X/C/V, sticky Z | register forms in the ROM; memory forms unit-tested |
| `TAS` | `Dn` and data-alterable memory; sets bit 7, N/Z from the value read, V/C clear, X preserved; 4 or 10 + EA cycles | three register flag outcomes |

A decode-audit table now pins every documented MC68000 encoding family and the
special encodings that live inside another family's mask; the EXG-as-AND case is a
regression test that fails on the old classification. No documented encoding is
left to fall through into a different instruction.

### System/status-register forms corrected by the round-4 repair

The previous corpus never exercised the status-register instructions, so the
cycle table had four wrong system costs and one instruction-semantics bug. The
new `rom_status_reg` ROM compares every corrected form against the oracle:

| form | documented cost now implemented |
|---|---|
| `MOVE.W <ea>,SR` / `MOVE.W <ea>,CCR` | 12 + source EA (`Dn` 12, immediate 16, memory adds the word EA) |
| `MOVE.W SR,<ea>` | 8 for `Dn`/`An`; 6 for memory destinations (the pinned oracle charges this flat, without a separate write-EA cost) |
| `ORI`/`ANDI`/`EORI` to `CCR`/`SR` | 20 cycles, fixed |

The ROM also exposed a real semantic defect: `ORI.W`/`ANDI.W`/`EORI.W` to `SR`
masked the immediate to eight bits, so `ANDI.W #$2700,SR` cleared the supervisor
bit and `EORI.W #$0100,SR` left user mode. The full 16-bit immediate is now used
for the SR forms (the CCR forms still use the low byte), with unit tests.

### Bit operations and DBcc corrected by the round-6 repair

The round-6 repair replaced the placeholder four-cycle bit table and the flat
ten-cycle `DBcc` cost with the pinned oracle's per-form values:

| form | documented cost now implemented |
|---|---|
| `BTST` | dynamic 6 (`Dn`/`An`), static 10; memory 4/8 + byte EA |
| `BCHG`/`BSET` | dynamic 8, static 12 on `Dn`; memory 8/12 + byte EA |
| `BCLR` | dynamic 10, static 14 on `Dn`; memory 8/12 + byte EA |
| `DBcc` | 10 taken, 14 counter expired, 12 condition true |

`rom_bits_dbcc` exercises every static/dynamic form on `Dn` and `(A0)`, the
three `DBRA` outcomes (two taken iterations followed by expiry) and `DBEQ` with
the condition true. Its 42 projected records match the oracle's addresses,
per-instruction costs and final state.

## Honest limits

- Functional 68000 prefetch is not cycle-modelled; instruction cost is added at
  the instruction boundary, not distributed across internal microcycles.
- `MULU`/`MULS` use the documented operand-dependent early-out and now match the
  oracle for this corpus. `DIVU`/`DIVS` use the documented worst-case base; the
  pinned oracle also uses that base for these forms, so no operand-dependent
  divide timing is claimed.
- The oracle powers up with the architecturally undefined `X` flag set
  (`SR` raw `0x7FE0`) while ChronoFork resets `CCR` to zero (`SR` raw `0x2700`).
  Effective fields are compared, and every ROM now clears `X` explicitly with
  `ADD.B D0,D0` so an unwritten reset flag cannot masquerade as a state
  divergence. Raw `SR` words are still not compared (the oracle sets unused
  upper bits).
- No interrupt-latency equivalence, bus-contention, DMA-stall or VDP
  H/V-counter timing is claimed. The ROMs keep SR mask 7 where they do not
  change it, so no interrupt handler runs in the measured window.
- `TAS` memory forms are unit-tested but kept out of the oracle ROM: the Mega
  Drive's TAS read-modify-write write-back quirk is a bus/hardware behaviour, not
  a 68000-core behaviour, so comparing it would compare unrelated hardware. The
  register forms exercise the flag semantics against the oracle.
- The pinned oracle's `MOVE.W SR,<ea>` model charges a flat 6 cycles for every
  memory destination instead of the manual's write-EA table. ChronoFork matches
  the oracle for the differential; the flat value is recorded here as an
  oracle-model limit, not presented as a hardware-accuracy claim.
- `MOVE to CCR` (0x44C0) is a documented MC68000 instruction and is exercised by
  `rom_status_reg` (12 + source EA). `MOVE from CCR` (0x42C0) is the 68010
  addition outside the MC68000 set; it is implemented for compatibility but has
  no MC68000 cycle-equivalence claim and is not in the differential corpus.
- The address stream, per-instruction cycle cost and selected final state are
  compared for these synthetic ROMs only; no game compatibility, Z80, audio,
  rendering or DMA claim follows.
- Instruction semantics beyond the compared observables are covered by the
  ROMs' self-checking flag branches and the final-state comparison. No
  expectation, comparator or core table was edited to hide a divergence; the
  only fixture change is the explicit, documented `X` initialisation.

## Supersessions

- The M3 instruction-quantum model (one instruction = one tick, configurable
  instructions per line, default 64) is superseded by the master-cycle timeline.
- The M3 oracle evidence page's quantum/masked-boundary control rows and the
  `--instructions-per-line` fixture descriptions are superseded by this page.
- The M3 hardware acceptance row M3-A52 (oracle differential gate) is superseded
  for timing claims by this cycle differential; its address/final-state evidence
  remains historical.
- The previous Phase 8.2 run recorded 1/8 cycle and 5/8 final-state matches as
  unresolved model gaps; the follow-up repair closed those. The 8-ROM run that
  closed them was superseded by the 9-ROM run (EXG/TAS/ADDX/SUBX), then by the
  10-ROM status-register run and now by this 11-ROM run that adds bit/DBcc
  coverage; this page replaces those results rather than amending them.
