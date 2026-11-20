# Z80, PSG and YM2612 state differential evidence

- **Status:** Active — phase 8.4 evidence, executed 2026-11-20.
- **Harness:** [`scripts/oracle-audio-state-diff.py`](../../scripts/oracle-audio-state-diff.py).
- **Raw artifacts:** git-ignored `.research/evidence/8.4/oracle-audio-state-diff.{json,md}`.
- **Pinned oracle:** Exodus headless checkout at
  `ec0056151f3ff1c334d09ee375abb30a68fdfec5`, binaries in `.research/oracle/build`
  (`exodus-mcpd` `7da8dcb0…`, `exodus-probe` `07c56c54…`), module
  `Data/Modules/Sega Mega Drive 1600.xml`.
- **Related:** [Mega Drive slice](../architecture/megadrive-slice.md),
  [execution model](../architecture/execution-model.md),
  [VDP state differential](vdp-state-differential-evidence.md),
  [quality gates](../quality/quality-gates.md),
  [functional requirements](../requirements/functional.md).

## What the harness compares

The harness builds one original synthetic Mega Drive ROM. Its 68000 program
installs a Z80 program, hands the Z80 its bus, and spins on a fixed branch. The
Z80 program initialises every compared register, programs the PSG tone/volume/
noise registers, writes YM2612 register/data ports, and executes `HALT`. The Z80
`HALT` at `PC = 186` is the synchronisation boundary: both emulators must have
executed the same Z80 instruction sequence and stopped at the same instruction
before any device state is read.

It then reads **device state only**, never samples:

| Source | Oracle tool | ChronoFork source |
|---|---|---|
| Z80 register file | `cpu.registers` (`Registers/Raw Registers/*`) | `chronofork script run` `register("z80.*")` inspection pseudo-registers |
| PSG register file | `device.describe` (`Registers/Channel */…`) | `register("psg.*")` inspection pseudo-registers |
| YM2612 status byte | `cpu.read_memory` at `0xA04000` on the 68000 bus | `register("ym.status")` |

Compared fields, in the harness's fixed order (25 fields):

`z80.af`, `z80.bc`, `z80.de`, `z80.hl`, `z80.ix`, `z80.iy`, `z80.sp`,
`z80.pc`, `z80.i`, `z80.r`, `z80.iff1`, `z80.iff2`, `z80.im`,
`psg.channel1.volume`, `psg.channel1.tone`, `psg.channel2.volume`,
`psg.channel2.tone`, `psg.channel3.volume`, `psg.channel3.tone`,
`psg.channel4.volume`, `psg.channel4.noise_type`,
`psg.channel4.noise_period`, `psg.control.latched_channel`,
`psg.control.volume_latched`, `ym.status`.

## Executed result (2026-11-20)

Command:

```sh
python3 scripts/oracle-audio-state-diff.py --out-dir .research/evidence/8.4
```

Result: **PASS**, exit 0, 25/25 compared fields equal at the shared Z80 `HALT`
boundary (`PC = 186` on both emulators). Observed values:

| Field | Both emulators |
|---|---|
| `z80.af` | `23108` |
| `z80.bc` | `4660` |
| `z80.de` | `22136` |
| `z80.hl` | `39612` |
| `z80.ix` | `4951` |
| `z80.iy` | `9320` |
| `z80.sp` | `16382` |
| `z80.pc` | `186` |
| `z80.i` | `66` |
| `z80.r` | `79` |
| `z80.iff1` / `z80.iff2` | `1` / `1` |
| `z80.im` | `1` |
| `psg.channel1.volume` / `tone` | `0` / `291` |
| `psg.channel2.volume` / `tone` | `5` / `171` |
| `psg.channel3.volume` / `tone` | `15` / `1023` |
| `psg.channel4.volume` | `7` |
| `psg.channel4.noise_type` / `period` | `1` / `2` |
| `psg.control.latched_channel` / `volume_latched` | `3` / `0` |
| `ym.status` | `0` |

The oracle fixture ROM (bus request `0`) sha256 is
`2d7c469e580869ee4e2ead2e7fc14ac761068d3ad1ff48426127b785c7cbb833`; the ChronoFork
fixture ROM (bus request `1`) sha256 is
`da5d34d420172cd2e1e0bbc752b0757d69d6bc93159df947dd32416915b46c90`; the Z80 program
sha256 (identical in both) is
`bbf496123a678f373d75ab1d8f1b504b66679a03eda44519c655b3f9a5f19d1b`.
The harness also runs the negative controls: corrupting `z80.af`, `psg.channel2.tone`
and `ym.status` each produces the expected first mismatch (exit 1 in a direct
comparator call), and a child invocation pointed at an empty oracle directory
exits **3** with an explicit reason — there is never a silent PASS.

> **Fixture note (2026-11-20, repair R2-F4).** The paragraph above describes the recorded
> run's fixtures. The harness now uses the **same** bus-request byte (0, the Z80 owning the
> bus) on one byte-identical ROM on both emulators, so both sides' `rom_sha256` is
> `2d7c469e580869ee4e2ead2e7fc14ac761068d3ad1ff48426127b785c7cbb833`; the request-asserted
> probe ROM carries byte 1 and is reported separately. Re-run artifacts:
> `.research/8.4-r2/evidence-r3/` (ignored, worktree-local).

## Internal PCM hash fixtures (exact, not cross-emulator)

The phase pins its internal PCM determinism by hash, not by comparing samples with
another emulator. The T4/T5 fixtures in
[`tests/audio_mix.rs`](../../crates/chronofork-megadrive/tests/audio_mix.rs) and
[`tests/audio_snapshot.rs`](../../crates/chronofork-megadrive/tests/audio_snapshot.rs)
assert equality of independently executed runs. The exact hash for the documented
one-second `audio_mix` fixture (FM voice on YM2612 channel 1 with full-volume PSG
tone channel 0, `MASTER_CLOCK_HZ_NTSC`, one second of emulated time) is:

| Fixture | Frames | PCM SHA-256 |
|---|---|---|
| `programmed_ym(0x20, 0x22)` + `programmed_psg`, one NTSC second, left/right interleaved little-endian `i16` | 48000 | `7385543399a5e19c9bae4f84b0f5eaee8f63d11ee6086eb0f961492a0200d16e` |

It was produced by an out-of-tree probe that links the same
`chronofork-megadrive` crate and reproduces the test fixture, so no repository
source was changed to obtain it. The pinned payload/state/config hashes for the
six-step demo machine are recorded in `backend::tests::snapshot_payload_and_hashes_are_golden`
(`CFMD68K6`, payload length `206251`, payload sha256
`fa87fe3f519c9f940194f1e7bf0e1db091dbbe43ec39de5cfb859979ae6311e1`, state hash
`2e2db861007ad5ab9eee89ae3c5eea87ea05b28505b1b343da42b5788b91998b`). The phase-8.4 repair
round re-pinned the payload and state hashes for R1-F2/R1-F3 (the pre-repair values were
`e69b6da1…` and `b45ae6c1…`), while the payload length and the configuration hash
(`695ab251aa18ec298e9461b77d46faccc7e302d5b12c87156d5fc714a3af7268`) are unchanged.

**No cross-emulator sample equality is claimed.** The oracle run never reads audio
samples, the harness has no sample path, and the emitted WAV/streaming capture
modes belong to phase 8.7.

## Scoped arbitration divergence (documented, not hidden)

The pinned oracle models hardware `ZBR` polarity: asserting the 68000's Z80 bus
request takes the bus *away* from the Z80, so a running Z80 stops at its next
instruction boundary. ChronoFork's phase-8.4 arbitration model is the documented
functional inverse: its Z80 executes while the request latch is set and the reset
is released (a functional model, not microcycle-exact; see
[`docs/architecture/megadrive-slice.md`](../architecture/megadrive-slice.md)).

The harness therefore runs the same 68000 and Z80 programs with only the bus-request
byte set per emulator (`1` for ChronoFork, `0` for the oracle), so that both Z80
cores execute the identical program to the shared `HALT` boundary. It also runs the
literal request-asserted ROM on the oracle as a probe: the oracle stops the Z80 at
`PC = 4`, while ChronoFork reaches `PC = 186`. That probe is reported as a scoped
**arbitration-model divergence**; it is not part of the device-state equality claim
and the harness does not assert 68000 or arbitration equivalence.

> **Superseded by the t5 repair (R1-F3, 2026-11-20).** The section above is the dated
> record of the run that produced this page; it is kept verbatim. ChronoFork now follows
> the hardware `ZBR` line: asserting write bit 8 of `0xA11100` stops the Z80 and gives the
> 68000 the Z80 window, clearing it returns the bus to the Z80, and read bit 8 reports Z80
> ownership, so the same-ROM probe's divergence no longer holds and the `1`/`0`
> bus-request-byte difference is no longer needed. The divergence above is historical, not
> a current limit; no 68000 or arbitration equivalence is claimed either way. The phase-8.4
> repair also re-based the PSG noise shift periods on the documented `N/512`, `N/1024` and
> `N/2048` chip-clock divisions (32/64/128 internal ticks) and made the tone-2-clocked mode
> shift once per full tone-2 period.
>
> Re-probe of the internal PCM fixture at the repair HEAD (same command as the table
> above): 48 000 frames, PCM SHA-256
> `7385543399a5e19c9bae4f84b0f5eaee8f63d11ee6086eb0f961492a0200d16e` — **unchanged** from
> the table, because that fixture leaves the PSG noise channel at attenuation 15 (silent)
> and keys only YM2612 channel 1. Log: `.research/8.4-r2/pcm-probe.log` (ignored,
> worktree-local).
>
> **Re-run result (2026-11-20).** The harness was re-run with the **same** bus-request byte
> on both emulators (the `1`/`0` workaround removed, one identical ROM on both sides):
> `python3 scripts/oracle-audio-state-diff.py --out-dir .research/8.4-r2/evidence-r3` exits
> **0** with **PASS** for all 25 compared device-state fields at the shared Z80 `HALT`
> boundary (`HALT PC 186`, both corrupted-field controls and the missing-oracle exit-3
> control pass, no first mismatch). Both sides now run a byte-identical ROM
> (`sha256 2d7c469e580869ee4e2ead2e7fc14ac761068d3ad1ff48426127b785c7cbb833`,
> `z80_program_sha256 bbf49612…`). The request-asserted probe that the historical run
> recorded as a divergence now **agrees**: oracle Z80 `PC = 4`, ChronoFork Z80 `PC = 4`
> (`z80_pc_agrees: true`), where the recorded run had ChronoFork reaching `PC = 186`. That
> is the demonstrated effect of the arbitration repair; the analogous PCM re-probe above is
> unchanged. No cross-emulator **sample** equality is claimed, and the functional
> (non-microcycle-exact) yield remains a ChronoFork limit. Artifacts:
> `.research/8.4-r2/evidence-r3/` (ignored, worktree-local).

## Exclusions (explicit reasons)

- **YM2612 register/channel state is not observable in the pinned oracle.** Its
  YM2612 reflection publishes only an Audio Logging page; `device.describe` lists
  no register, channel, operator or LFO page. Only the readable YM2612 status byte
  is compared. The following YM2612 model limits are also carried from the device
  documentation: `SSG-EG` (`0x90`–`0x9F`), channel-3 special mode, CSM mode and the
  `0x21`/`0x2B` test bits are stored but not applied; the envelope/amplitude curves
  and LFO are deterministic integer approximations; the status byte models only the
  timer A/B overflow flags (no busy bit); timer IRQ delivery to the Z80 is phase 8.5.
  None of those unmodelled bits is claimed equal to hardware.
- **PSG noise shift register** is clocked state, not a static register; the two
  emulators do not share an emulated-time budget at the boundary, so it is excluded
  and only its type/period control is compared.
- **PSG channel-4 tone register** is published by the oracle but not modelled by
  ChronoFork (the noise channel has no tone divider); it is excluded rather than
  reinterpreted.
- **Z80 alternate registers** (`AF'`/`BC'`/`DE'`/`HL'`) are published by the oracle
  but not exposed by ChronoFork's inspection pseudo-registers; `WZ`/`MEMPTR` is not
  modelled by ChronoFork. Both are excluded for lack of a counterpart.

## Reproducing

```sh
cargo build -p chronofork-cli           # target/debug/chronofork
python3 scripts/oracle-audio-state-diff.py --out-dir .research/evidence/8.4
python3 scripts/oracle-audio-state-diff.py --check-oracle \
  --oracle-bin-dir /tmp/empty/bin --oracle-root /tmp/empty/root \
  --out-dir /tmp/empty/out                # exits 3 with a reason
```

The harness exits 0 only when every compared field matches, all negative controls
detect their corruption and the missing-oracle child exits 3. A first mismatch is
printed and written to the report with field name, oracle value and ChronoFork
value; the generated report is never edited to force a pass.
