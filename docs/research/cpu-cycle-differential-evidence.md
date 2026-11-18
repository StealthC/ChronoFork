# 68000 cycle differential evidence (Phase 8.2)

**Status:** executed, with recorded divergences. This page is the durable evidence
for the Phase 8.2 cycle comparison. It supersedes the M3 instruction-quantum
timing claims in [oracle-differential-evidence.md](oracle-differential-evidence.md)
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

The harness covers the three pre-existing synthetic ROMs plus five new
independently authored ROMs committed under [`scripts/fixtures/`](../../scripts/fixtures/):
EA forms (`rom_ea`), shifts/rotates (`rom_shift`), BCD (`rom_bcd`), MOVEM/MOVEP
(`rom_movem`) and multiply/divide (`rom_muldiv`). All register setup uses explicit
`MOVEA.L #imm,An` so the comparison reaches the instruction mix under test.

## Command and pinned inputs

```sh
python3 scripts/oracle-diff.py --skip-build \
  --chronofork-bin target/debug/chronofork \
  --out-dir .research/evidence/8.2
```

- Pinned oracle checkout: `.research/upstream/exodus-headless` at
  `ec0056151f3ff1c334d09ee375abb30a68fdfec5` (branch `linux-headless`).
- Pinned binaries: `.research/oracle/build/exodus-probe` (sha256 prefix
  `07c56c54e855cd4f`) and `.research/oracle/build/exodus-mcpd`
  (`7da8dcb00bc678dc`).
- Raw artifacts: `.research/evidence/8.2/` (`oracle-diff.json`,
  `oracle-diff.md`, per-ROM oracle/ChronoFork traces and projections). These are
  ignored research artifacts; the durable summary is this page.

## Results

- **Address projections: 8/8 match.** Every projected executed-address sequence
  agrees with the oracle, every ChronoFork opcode word matches the ROM bytes, and
  the address negative control passes 8/8.
- **Per-instruction cycles: 1/8 fully match** (`rom_bcd`). The other seven report
  their first mismatch below; the cycle negative control (first projected cost
  corrupted by one cycle) is detected 8/8.
- **Selected final state: 5/8 match.** `rom_alu`, `rom_mem_vdp`, `rom_masked`,
  `rom_ea` and `rom_shift` match; `rom_bcd`, `rom_movem` and `rom_muldiv` report
  differences in registers/flags or work RAM. The script exits 1 because cycle
  and final-state divergences are findings, not passes.

| ROM | projected instructions | cycles match | first cycle mismatch | oracle | ChronoFork |
|---|---:|---|---:|---:|
| `rom_alu` | 99 | no | index 16, `0x00023C`, opcode `0x6602` (taken BNE) | 10 | 8 |
| `rom_mem_vdp` | 51 | no | index 31, `0x000288`, opcode `0x6702` (taken BEQ) | 10 | 8 |
| `rom_masked` | 155 | no | index 19, `0x000242`, opcode `0xD280` (ADD.L D0,(A0)) | 8 | 6 |
| `rom_ea` | 23 | no | index 18, `0x000248`, opcode `0xD282` (ADD.L D2,(A0)) | 8 | 6 |
| `rom_shift` | 25 | no | index 17, `0x00023E`, opcode `0xE348` (LSL.W #1,D0) | 8 | 6 |
| `rom_bcd` | 21 | **yes** | — | — | — |
| `rom_movem` | 21 | no | index 17, `0x00023E`, opcode `0x48E0` (MOVEM.L regs,-(A0)) | 32 | 22 |
| `rom_muldiv` | 25 | no | index 19, `0x000242`, opcode `0xC0C1` (MULU D1,D0) | 42 | 70 |

The mismatches are consistent with the documented Phase 8.2 model limits rather
than with a hidden or edited expectation:

- conditional branches use a fixed 8/10-cycle model while the oracle charges a
  taken-branch penalty;
- memory-destination read-modify-write forms are one effective-address cost low;
- register shifts/rotates do not add the oracle's extra cycle for the operated
  register form;
- MOVEM cost is mask/register-count dependent in the oracle, while the table uses
  a base plus one effective-address cost;
- MUL/DIV use the documented worst-case value (70/140) in ChronoFork while the
  oracle's early-out cost depends on the operands.

## Honest limits

- Functional 68000 prefetch is not cycle-modelled; instruction cost is added at
  the instruction boundary, not distributed across internal microcycles.
- MUL/DIV use worst-case costs, so operand-dependent early-out timing is not
  claimed.
- No interrupt-latency equivalence, bus-contention, DMA-stall or VDP H/V-counter
  timing is claimed.
- The address stream and selected final state are compared for synthetic ROMs
  only; no game compatibility, Z80, audio, rendering or DMA claim follows.
- The seven first-cycle mismatches above are **unresolved model gaps** for a later
  cycle-accuracy pass. They were not hidden by editing the comparator, the oracle
  output or a test expectation.

## Supersessions

- The M3 instruction-quantum model (one instruction = one tick, configurable
  instructions per line, default 64) is superseded by the master-cycle timeline.
- The M3 oracle evidence page's quantum/masked-boundary control rows and the
  `--instructions-per-line` fixture descriptions are superseded by this page.
- The M3 hardware acceptance row M3-A52 (oracle differential gate) is superseded
  for timing claims by this cycle differential; its address/final-state evidence
  remains historical.
