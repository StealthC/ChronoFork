# VDP state differential evidence

Status: **PASS with recorded, scoped divergences** (2026-11-19, Phase 8.3
Task 13).

This page records the executed VDP differential between ChronoFork and the
pinned headless Exodus oracle. It is the evidence behind the rendering/DMA/timing
capability claims in
[megadrive-slice.md](../architecture/megadrive-slice.md) and the VDP quality gate
in [quality-gates.md](../quality/quality-gates.md). Everything below was run in
this checkout; no value is projected from a design document.

## Command

```sh
cargo build -p chronofork-cli
python3 scripts/oracle-vdp-diff.py --out-dir .research/evidence/8.3
```

The harness writes machine-readable
`.research/evidence/8.3/oracle-vdp-diff.json` and human-readable
`.research/evidence/8.3/oracle-vdp-diff.md`, including per-program ROM SHA-256,
stop markers, instruction counts, ticks, state hashes and VRAM hashes. The run
reported:

```text
VDP differential: pass
  register-vram: pass (253 instructions, tick 22064)
  status-counters: pass (2647 instructions, tick 75656)
  negative control: pass
```

## Pinned oracle

- Exodus headless source revision:
  `ec0056151f3ff1c334d09ee375abb30a68fdfec5` (`linux-headless`).
- `exodus-mcpd` SHA-256:
  `7da8dcb00bc678dca813f1e56aed0c0deaa814613a7ad8609ce92212049f3ff6`.
- System module: `Data/Modules/Sega Mega Drive 1600.xml`.
- Transport: newline-delimited JSON-RPC 2.0 over stdio; breakpoints stop the
  emulator before the marker instruction, which is the synchronized instruction
  boundary both emulators reach.

## Programs

Both programs are independently authored synthetic cartridges (no copyrighted
data) generated deterministically by the harness.

### `register-vram`

Programs VDP registers `0x00`-`0x17` (interrupt sources disabled, H40, plane A /
window / sprite / horizontal-scroll bases, auto-increment 2, DMA registers left
untriggered), writes a flat VRAM block at `0x0000`, a plane A row at `0xC000`, a
sprite attribute row at `0xD800`, the horizontal scroll table at `0xFC00`, 16
CRAM words, 16 VSRAM words, then reads status, H/V counters and all 16 CRAM words
back into work RAM and stops at the marker.

- ROM SHA-256: `9a6e6957770d4af0a66fa9cc23a0252bb672eee6461aa7bce3daf5f8cf196aaf`
- Marker: `0x0708`; 253 instructions; ChronoFork tick 22064
- ChronoFork state hash:
  `7e6ca5db584f68b799ad9e5c5e9821a0190c8e34659d9fc0b2d181edd8a49a08`
- ChronoFork VRAM SHA-256:
  `94eadb0c71ab3a7b18523aec9fe163653116c752e5b9ba8c830469780e7bf192`
- Result: **pass**. The full 64 KiB VRAM image, the 16 programmed CRAM words
  read back through the data port, the complete VDP register file, and every
  non-readback work-RAM word match their expected values; PC and the 68000 S/IPM
  bits match Exodus.

### `status-counters`

Samples the VDP status register and H/V counters at four instruction boundaries
(immediately, after 64 NOPs, after 512 more NOPs, after 2048 more NOPs) and stops
at the marker. This checks that the sample stream is deterministic and that the
status bit *dynamics* agree, while recording the absolute values.

- ROM SHA-256: `7258233e225481b3299ce6803d19f4b724a81d15ad7fb471e1038065983a5e34`
- Marker: `0x16F6`; 2647 instructions; ChronoFork tick 75656
- ChronoFork state hash:
  `6839a81f2a94661246fb8432d29dc164c08630a1b345d6423b29ac94a19dfe4b`
- ChronoFork VRAM SHA-256 (untouched VRAM):
  `de2f256064a0af797747c2b97505dc0b9f3df0de4f489eac731c23ae9ca9cc31`
- Result: **pass** with the counter/status values recorded as scoped
  divergences (below); PC, S/IPM and all other RAM match.

## Compared fields

| Field | Oracle side | ChronoFork side | Result |
|---|---|---|---|
| 68000 PC | `cpu.registers` | final-state file | exact match |
| 68000 SR (S/IPM) | `cpu.registers` | final-state file | exact match on `0xFF00`; CCR is a function of the compared instruction results and is not claimed when the scoped counter readback differs |
| 68000 D0/D1/D3 | `cpu.registers` | final-state file | exact match (D2 is the VDP readback scratch; D0-D3 are zeroed with `MOVEQ #0` because Exodus powers up D/A with `0xFFFFFFFF` and ChronoFork with zero) |
| Work RAM | `cpu.read_memory` | final-state file | exact match except the VDP readback ranges recorded below |
| VDP registers `0x00`-`0x17` | not exposed by the oracle | `chronofork debug vdp` | exact match against the programmed values |
| VRAM (64 KiB) | not exposed by the oracle | `chronofork debug vdp` VRAM windows + SHA-256 | exact match against the expected image built from the writes |
| CRAM (16 words) | prefetch-shifted data-port readback only | data-port readback into work RAM | ChronoFork matches the written words; the oracle's lagging readback is recorded as a scoped limit |
| VSRAM | stale write latch only | data-port reads undecoded | not content-compared; covered by backend unit tests |
| Status register | guest readback into RAM | guest readback into RAM | absolute values recorded; phase-dependent bits diverge (scoped) |
| H/V counters | guest readback into RAM | guest readback into RAM | absolute values recorded; H counter and power-on phase diverge (scoped) |

### Scoped divergences (recorded, not hidden)

The minimal slice and Exodus bring the VDP up at different scanline phases: at
reset ChronoFork reports V counter `0x00` while Exodus reports `0xB7`, and the
status bit bases differ (`0x0100` vs `0x3208`; the undocumented `0x2000`/`0x1000`
bits and FIFO bits are not modelled by the slice). The *change* across samples
still matches for the HBlank bit (`0x0100 -> 0x0104 -> 0x0100` on ChronoFork and
`0x3208 -> 0x320C -> 0x3208` on Exodus). The harness therefore records every
status/counter readback with both values and a delta instead of comparing the
absolute word; it never treats them as silently equal. `register-vram` records
10 scoped words (2 status/counter readbacks plus 8 CRAM readback words where the
oracle's data-port prefetch returns the preceding entry), and `status-counters`
records 8 status/counter readbacks. The oracle CRAM readback is therefore not a
straight content comparison either: half the words arrive one entry late in the
pinned headless build. These 18 values are listed program by program in
`.research/evidence/8.3/oracle-vdp-diff.md`.

### Oracle VDP memory probe

The harness first writes `0xAAAA`, `0xBBBB`, `0xCCCC`, `0xDDDD` to VRAM and reads
them back through the data port. The pinned headless oracle returns
`0xDDDD` four times: its data-port read buffer returns the stale write latch
instead of the addressed word, and debug memory reads at `0xC00000..0xC0000F`
bypass the device (they return zero and do not mutate it). The oracle's VDP
reflection pages expose only settings and layer-visibility booleans, with no
registers or memory. This is why VDP memory content is compared on the
ChronoFork side against the program's written values, not byte-for-byte against
Exodus.

### Negative control

The negative control flips one hard-compared work-RAM word (`0x00FF0020`) and one
VRAM byte (`0x0000`). The comparator reports the first mismatch
(`ram[0x00FF0020]: oracle=0xFF00 chronofork=0x0000`), proving that a corrupted
observation fails the run instead of being absorbed by the scoped-divergence
classification.

## Frame hashes and limits

- **Frames are deterministic but not cross-emulator compared.** The headless
  oracle has no framebuffer, so pixels are pinned internally. The
  `crates/chronofork-megadrive/tests/vdp_frame.rs` tests assert one frame per
  frame boundary, RGBA alpha `0xFF`, identical checksums across identical runs,
  and fixed full-frame SHA-256 fixtures for a deterministic scene with plane
  pixels plus shadow/highlight operators: H32 (256x224)
  `43e111026c99e6fe3b30693aed707bf3cb5ce1143d2bfffd507c2d03e49d638f` and H40
  (320x224) `3fb74d04b24dd9b355365b375da8edaadef5cddd97624e687587513da3517056`.
  Those fixtures hash the canonical width/height plus palette-index frame bytes,
  so a renderer change that is internally consistent still fails them. The media
  layer's RGBA conversion is covered separately by the frame tests. No
  cross-emulator pixel comparison is made, and the deterministic VRAM SHA-256
  values above remain the scene fixtures for this differential.
- H counter is approximate; the H counter and the power-on scanline phase are
  the recorded scoped counter divergences. Absolute H/V values at matched
  instruction indices are not claimed equal.
- H32 window geometry, line-mode vertical scrolling and column-scroll details are
  approximations (see the slice page); interlace is absent.
- Physical byte-swapped VRAM layout is not modelled.
- VDP data-port VSRAM reads are undecoded in ChronoFork; CRAM/VSRAM canonical
  content and round-trip are covered by `vdp_ports`/`vdp_snapshot` unit tests.
- DMA source is a VRAM address in this slice, and DMA is exercised by
  `vdp_dma`/`vdp_fifo` unit tests; it is not content-compared against the oracle
  because DMA state is not externally observable.
- The oracle readback probe above documents the memory-read limit; no
  cross-emulator pixel comparison and no game-compatibility claim follow from
  this differential.

## Reproduction notes

- Run the harness with the local `target/debug/chronofork` binary; if it is
  missing the harness records a **skip** (exit 3) with the reason instead of
  passing.
- If `.research/oracle/build/exodus-mcpd` is absent, the harness records a
  **skip** with the reason and exits 3; it never writes a success record.
- Exit codes: `0` pass (with any scoped divergences recorded), `1` unexpected
  divergence, `2` harness/setup error, `3` skipped (oracle or binary absent).
