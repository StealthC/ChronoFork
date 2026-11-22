# Mega Drive 68000 Slice

- **Status:** Partial real-hardware slice implemented (68000 + ROM/RAM + VDP port
  window + VBlank/HBlank scheduler with SR-mask gating, `RTE` and VDP
  VINT-pending/acknowledge + bounded debug instruction trace, on the master-cycle
  timeline, with eleven-ROM oracle differential evidence: address projections,
  per-instruction cycle costs and selected final states all match 11/11); not a
  complete emulator. Controller-port data-register reads for real-hardware
  input TAS are implemented and independently verified for the TH-high 3-button subset on
  ports 1–2 ([ADR-0009](../adr/0009-real-hardware-input-tas.md); t5 PASS, 2026-11-16), with
  the canonical state, demos and M1 evidence unchanged. A/Start, TH/6-button/Start
  multiplexing, controller timing, analog/keyboard hardware and port 3 remain absent.
  M4 debugging depth (bounded non-canonical observation, views/disassembly and
  CPU/VDP/interrupt/bus inspectors) is implemented and independently verified for this
  slice (t5 PASS, 2026-11-16). M8 phase 8.1 adds a modelled cartridge — header/region/
  checksum, Sega mapper, 64 KiB battery SRAM, TMSS lockout and version register — with
  payload/state/config v3 (`CFMD68K3`/`CFMDSTATE3`/`CFMDCONF3`; implemented, reviewed and
  verified 2026-11-17). Phase 8.2 moved the identity to v4 with the master-cycle timeline
  and extended CPU snapshot fields. Phase 8.3 (complete VDP) adds the plane/window/sprite
  compositor, DMA, the FIFO/CPU wait model, VDP-owned VBlank/HBlank requests, H/V counters
  and deterministic headless video frames, and moves the payload to v5
  (`CFMD68K5`/`CFMDSTATE5`/`CFMDCONF5`); booting a commercial title is phase 8.6 evidence,
  not claimed here.
- **Current milestone:** [M8 — Functional Mega Drive](../roadmap/milestones.md#m8--functional-mega-drive)
  is **in progress**. Phase 8.2 (the documented MC68000 set and master-cycle timeline,
  with 11/11 address, per-instruction cycle and final-state differential evidence) is
  implemented, verified (round 12), independently reviewed (round 11) and merged to `main`
  through the phase-8.2 PR (issue #15). Phase 8.3 (the complete VDP) is implemented,
  verified (rounds 1–3) and independently reviewed (rounds 2–3), and merged to `main`
  through the phase-8.3 PR (issue #16). Phase 8.4 adds the Z80 core, the Z80 memory
  map and 68000/Z80 bus arbitration, the YM2612 and PSG models, the deterministic
  48 kHz mixer/resampler and audio capture through `MediaSource`, and moved the
  payload to v6 (`CFMD68K6`/`CFMDSTATE6`/`CFMDCONF6`); it is implemented, its
  phase gate was executed on the phase branch and the phase is
  [closed in the planning lifecycle](../planning/ROADMAP.md), with independent
  verification, review and integration pending. Timer IRQ delivery to the
  Z80 and the rest of M8 (8.5–8.8) remain. Cartridge and controllers are already
  implemented as described below. This page remains the canonical
  capability/limitation home; it is updated as each phase lands, never in advance.
- **Crate:** `crates/chronofork-megadrive`
- **Backend ID:** `chronofork.megadrive.m68000`
- **Related:** [ADR-0002](../adr/0002-backend-boundaries.md),
  [EPIC-0002](../epics/EPIC-0002-megadrive-slice.md)

## Purpose

This slice starts ChronoFork's Rust-native production backend under the accepted
staged-hybrid strategy. It proves that a real 68000 reset/execute/VDP/interrupt/
snapshot/fork path can sit behind the same backend contract as the synthetic
validator, without pretending the result is a playable Mega Drive emulator.

No upstream emulator source code has been incorporated. The program and all test
fixtures are independently authored synthetic data.

## Implemented

### Memory bus

| Region | Address | Behavior |
|---|---|---|
| Cartridge ROM | `0x000000..0x3FFFFF` | Immutable, big-endian; bank-mapped through the Sega mapper; only image bytes are valid |
| Mapper registers | `0x00A130F0..0x00A130FF` | Write-only: `0xA130F1` is the SRAM control byte, odd registers `0xA130F3..0xA130FF` select the seven bank windows |
| Battery SRAM | `0x00200000..0x0020FFFF` | 64 KiB, mapped over the ROM window while `0xA130F1` bit 0 is set; bit 1 write-protects it |
| TMSS register | `0x00A14000..0x00A14003` | A long write whose bit 9 is set releases the enabled VDP-write lockout |
| Version register | `0x00A10001` | Cartridge-reported export/PAL/no-disk/hardware byte |
| Work RAM | `0xE00000..0xFFFFFF` | 64 KiB, big-endian, decoded through address bits 15–0: the primary `0xFF0000..0xFFFFFF` window and the `0xE00000..0xFEFFFF` mirror |
| VDP ports | `0xC00000..0xC0000F` | Data port `0x00..0x03`, control/status port `0x04..0x07`, HV counter `0x08..0x0F` |
| Controller data registers | `0x00A10003`, `0x00A10005` | Byte reads only, driven by the current step's `InputFrame` ports 0/1; a write is acknowledged and discarded; see below |
| Controller control registers | `0x00A10008..0x00A1000F` | Port 1/2 control pairs (`08..0B`) are read/write latched bytes in canonical state (payload v7); the port 3/EXT control pairs (`0C..0F`) are the ADR-0014 constant-zero stub — reads return `0x00` and writes are discarded |
| YM2612 registers | `0x00A04000..0x00A04003` | Two address/data port pairs through the bus arbiter; reads return the YM2612 status byte |
| Z80 RAM window | `0x00A00000..0x00A01FFF` | 8 KiB Z80 RAM; accesses stall while the Z80 owns the bus |
| Z80 bank register | `0x00A06000` | Write-only nine-bit bank register; reads return the latched value |
| Z80 bus control | `0x00A11100`, `0x00A11200` | Bus request and active-low reset latches (bit 8 on reads) |
| Undecoded I/O window | `0x00A10000..0x00A1001F` (minus the rows above) | Deterministic open bus: reads return `0xFF`, writes are acknowledged and discarded |
| Everything else | — | Explicit `UnmappedRead` / `UnmappedWrite` error |

- Word and long accesses require even addresses.
- ROM writes are rejected; the mapper register file is write-only, so a read there is a
  typed `UnmappedRead`.
- Multi-byte and debugger writes validate the entire range before mutating RAM, so
  a failed write cannot leave partial state.
- The machine decodes 24 address bits, so bits 31–24 of a 68000 address register are
  ignored and every access is truncated before it is mapped (`0xFFFFF7FE` addresses
  `0x00FFF7FE`). A word or long access truncates each byte lane on its own.
- Work RAM is mirrored over the whole `0xE00000..0xFFFFFF` range through address bits
  15–0, and a debugger read or write that crosses a 64 KiB block boundary is served
  byte by byte from the same contiguous 64 KiB (it never aliases the block start).
- In the controller/IO window `0xA10000..0xA1001F` the version register, the two
  controller data registers (byte reads of the executing step's `InputFrame`; writes
  are acknowledged and discarded) and the eight `0xA10008..=0xA1000F` control
  registers are mapped: the four port 1/2 bytes (`0xA10008..=0xA1000B`) latch, and the
  four port 3/EXT **control** bytes (`0xA1000C..=0xA1000F`) are the ADR-0014
  constant-zero stub (a read returns `0x00`, a write is acknowledged and discarded, so
  the stub lanes never change). Port 3/EXT data (`0xA10006`/`0xA10007`) is not modelled
  and stays a typed `UnmappedRead`/`UnmappedWrite`; every other byte of the window is
  deterministic open bus (`0xFF` on read, discarded on write). Outside the window the
  explicit typed error contract is unchanged: the 68000-side PSG window (`0xC00011`
  and above) is still `UnmappedRead`/`UnmappedWrite`. The 68000 YM2612 and Z80 windows
  are decoded as listed above; the Z80-side map (8 KiB RAM mirrored through
  `0x3FFF`, YM2612 at `0x4000`, bank write at `0x6000`, the **VDP port window** at
  `0x7F00..=0x7F03` (the arbiter maps it to `0xC00000 | offset`, so these four bytes
  are the VDP **data port** lanes, never the PSG; the TMSS write lockout applies to this
  window exactly as it does to the 68000 VDP path, and a VDP rejection on it is either a
  **capability gap** — the `0b1100` 8-bit VRAM read, counted in
  `Vdp::capability_gap_accesses` and kept a typed rejection — or an invalid target, which is
  served with no effect and counted in `Vdp::discarded_data_port_accesses`; the Z80 core has
  no exception channel and the pinned oracle's executable path logs a diagnostic and
  completes the access instead of raising), the PSG at `0x7F11` plus its
  mirror `0x7F10..=0x7F17`, and a 32 KiB banked window at `0x8000`) is reached
  through the Z80 bus view, and a PSG read returns the documented open value
  `0xFF`. The Z80 `SOUND` region is decoded across `0x4000..=0x5FFF` by an
  `0x0003` port mask and the PSG mirror drives the same PSG instance the 68000
  window `0xC00010..=0xC00017` drives. The 68000 I/O window
  `0x00A10006`/`0x00A10007` (port 3/EXT data) is a real register this slice does not
  model: the access is **accepted with no effect and counted** in
  `MegadriveBus::port3_unmodelled_accesses` instead of faulted, because faulting turned a
  legal guest store into a guest address error that skipped the instruction in a
  commercial boot (W1X). A **word** access to the PSG is one 16-bit
  transaction whose latch selector is bit 15 and whose channel/type/data fields come
  from the low byte (the fields consumed are `((word >> 8) & 0x80) | (word & 0x7F)`);
  a long access is two such transactions, and byte writes pass their byte through.
  This is a model that matches the pinned oracle's executable path
  (`Devices/M68000/M68000.cpp:2191-2204`, `Devices/315-5313/S315-5313_Ports.cpp:1639-1643`,
  `Devices/SN76489/SN76489.cpp:723-748`, `ExodusSDK/DeviceInterface/Data.inl:560-562`),
  **not** independent physical evidence of the real PSG lanes.

## Guest-access continuity (W1 phase)

No **guest** memory access may end the run. Each channel converts deliberately;
there is no catch-all, and host-level failures keep their typed identity.

**Qualification (review round 4, W1Z).** Mapping holes are served with the model's
defined read value and an observable count; declared capabilities that are **not**
implemented (memory-sourced DMA and the VDP `0x0C` 8-bit read) produce an explicit
diagnostic stop through a typed error, with a separate capability count — never an
`Ok` that claims a read or a transfer that did not happen. This distinction qualifies
the continuity rule above for unowned accesses; it neither revokes it nor permits
swallowing host errors. The rule was written for mapping gaps, and applying it
unqualified to declared capabilities would advance the guest with wrong state instead
of announcing the limitation.

- **Z80 memory and I/O:** an address no device owns is served by the undecoded
  path — a read returns `bus::Z80_UNDECODED_READ_VALUE`, a write is ignored, and
  the access is counted by `MegadriveBus::unmapped_z80_accesses`. The banked
  window `0x8000..=0xFFFF` is **not** universal ROM: the arbiter translates it to
  the 68000 address `(bank << 15) | offset` and forwards the write
  (`Devices/MD1600IO/MDBusArbiter.cpp:537-560`), so a bank aimed at work RAM really
  writes RAM, a bank aimed at the cartridge is decoded but read-only (the write has
  no effect and is *not* counted), and only a bank that translates into a hole is
  counted as unserved. Reads keep the slice's prior raw-ROM behaviour; a bank
  selected beyond the cartridge image reads the undecoded value.
  The value is `0x00`, taken from the pinned oracle's **executable** reference
  (`Devices/Z80/Z80.cpp:1455-1474` reads into a default-constructed `Z80Byte` and
  `System/BusInterface.cpp:2438-2490` leaves it untouched when no map entry
  resolves; the write path `:2491+` performs nothing). **Open conflict:** arbiter
  comments say undriven lines read as set for a Z80 read into the 68000 space
  (`Devices/MD1600IO/MDBusArbiter.cpp:366-370`) and that the Z80 likely drives all
  lines high before a read (`:427-435`). The conflict is unresolved; if hardware
  evidence settles on `0xFF`, `Z80_UNDECODED_READ_VALUE` is the only point to
  change. The counter is diagnostic: it is not part of any snapshot and a valid
  backend restore (which clones the bus before committing) resets it, so it
  describes execution since that restore; it never
  moves the payload or the state hash.
- **68000 data accesses:** `UnmappedRead`, `UnmappedWrite` and `WriteProtected`
  already become the guest address-error exception (vector 3). A **VDP** port
  error (`BusError::Vdp`) still surfaces as a host `BackendError::Execution`, and
  effective-address overflow (`CpuError::AddressOverflow`) does too; both are
  tracked outside this phase (VDP command semantics and the CPU address-arithmetic
  residual).
- VDP mirrors beyond `0xC0000F` and the physical byte-swapped/interleaved VRAM
  layout are not modelled.
- Bus reads take `&mut self` because a VDP port read can advance the VDP address.
  Debugger reads (`read_memory`) are side-effect free: they use `peek8`, which
  rejects the side-effecting VDP window instead of changing device state.

### 68000 instruction set

The Phase 8.2 implementation covers the documented MC68000 instruction groups:

| Group | Instructions |
|---|---|
| Data movement | `MOVE`/`MOVEA` (all documented source/destination modes), `MOVEQ`, `MOVEM`, `MOVEP`, `LEA`, `PEA`, `CLR`, `TST`, `EXT`, `SWAP`, `EXG` |
| Arithmetic | `ADD`/`ADDA`/`ADDQ`/`ADDI`/`ADDX`, `SUB`/`SUBA`/`SUBQ`/`SUBI`/`SUBX`, `CMP`/`CMPA`/`CMPI`/`CMPM`, `NEG`, `NEGX`, `MULU`, `MULS`, `DIVU`, `DIVS` |
| Logic | `AND`/`ANDI`, `OR`/`ORI`, `EOR`/`EORI`, `NOT` |
| Shifts/rotates | `ASL`, `ASR`, `LSL`, `LSR`, `ROL`, `ROR`, `ROXL`, `ROXR` in register and memory forms |
| BCD | `ABCD`, `SBCD`, `NBCD` |
| Bit operations | `BTST`, `BCHG`, `BCLR`, `BSET` (static and dynamic bit number), `TAS` (register and memory) |
| Control flow | `BRA`, `BSR` and every `Bcc` condition in 8- and 16-bit displacement forms, `DBcc`, `Scc`, `JMP`, `JSR`, `RTS`, `NOP`, `RTR` |
| System/privileged | `RTE`, `TRAP`, `TRAPV`, `CHK`, `ILLEGAL`, `LINK`, `UNLK`, `MOVE` to `SR`/`CCR` and from `SR` (MC68000), `MOVE` from `CCR` (68010 extension; see below), `MOVE` to/from `USP`, `ORI`/`ANDI`/`EORI` to `CCR`/`SR`, `STOP`, `RESET`, interrupt entry |

Every documented MC68000 instruction reports the instruction-boundary cycle cost
used by the oracle-backed timing table (see the timing and differential sections
below); effective-address cost is added exactly once per operand. One oracle-model
difference is recorded rather than hidden: the pinned oracle charges a flat 6
cycles for `MOVE from SR` to memory instead of a write-EA table, and ChronoFork
matches the oracle for the differential. `MOVE to CCR` (`0x44C0`) is a documented
MC68000 instruction and is exercised by the differential ROM; only `MOVE from CCR`
(`0x42C0`) is the 68010 addition. `MOVE from CCR` is implemented for compatibility
but is outside the MC68000 timing table and the differential corpus. Unrecognised
and reserved encodings raise the guest illegal-instruction exception (vector 4)
with the documented six-byte frame rather than a host error.

Reset loads the supervisor stack pointer from address `0x000000` and the program
counter from `0x000004`; `SR = 0x2700`, all general registers clear.

Flags implemented: `N`, `Z`, `V`, `C`, `X` for the documented arithmetic, logic,
shift, BCD and bit operations. `SR` upper bits are preserved.

The interrupt entry API pushes a word raw `SR` and a long return `PC` (six bytes),
sets the supervisor bit, preserves the CCR, loads the accepted interrupt level into
the interrupt-priority mask, reads the target long from the supplied vector-table
address and jumps to it. It wakes a halted processor. Invalid levels are rejected
before any state changes; stack or vector faults return typed errors (a failed first
push leaves `SR`/`PC` untouched, while the six-byte frame can already be pushed
before a later vector or target fault). `RTE` validates the complete frame before
changing canonical state, restores the exact raw `SR` (IPM, `S`, CCR), restores the
PC and advances A7 by six. `MOVE.W #imm,SR` (`0x46FC`) replaces the raw `SR` so a
program can change its mask. Privileged operations (`MOVE` to `SR`, `STOP`, `RESET`,
`RTE`, `MOVE` to/from `USP`) raise the privilege-violation exception (vector 8) in
user mode. Guest exception vectors actually delivered are address error (3), illegal
instruction (4), zero divide (5), `CHK` (6), `TRAPV` (7), privilege violation (8),
autovector levels 4 and 6 (28/30) and `TRAP #n` (32+n); vector 1 is the reset
vector, and bus-error (2) and trace (9) exception delivery are not implemented.

### VDP (phase 8.3)

- 64 KiB flat VRAM, 64 CRAM words and 40 VSRAM words, big-endian, zeroed at reset.
- Data port (`0xC00000..=0xC00003`), control/status port
  (`0xC00004..=0xC00007`) and HV-counter port (`0xC00008..=0xC0000F`).
- Two-word command setup and register writes `0x8000 | register << 8 | value` for
  registers `0x00..=0x17`. Data-port addressing decodes the hardware command
  codes `0b0000` VRAM read, `0b0001` VRAM write, `0b0011` CRAM write, `0b0100`
  VSRAM read, `0b0101` VSRAM write and `0b1000` CRAM read (CRAM aliases every 64
  words; VSRAM wraps modulo its 40 words through a 9-bit address, shared by the
  read and write paths); one auto-increment per word including byte accesses,
  with byte writes duplicated into both halves and byte reads selecting a lane.
  A data-port **write** whose code is not one of the three write codes is
  discarded with no effect on the VDP, exactly like the pinned oracle's
  `default:` (`Devices/315-5313/S315-5313_General.cpp:2575`); the target address
  still advances by the auto-increment register, and the slice counts the discard
  in `Vdp::discarded_data_port_accesses` so it is never silent. That counter is
  diagnostic state, deliberately **not** part of the canonical snapshot
  (`Vdp::snapshot_bytes`), so it needs no payload-version change; it describes
  execution *since* a valid restore, so `Vdp::restore_bytes` resets it (and a
  rejected restore leaves it untouched). A data-port
  **read** whose code is not one of the three read codes — including the
  unmodelled `0b1100` 8-bit VRAM read — stays a typed rejection, matching the
  oracle's read `default:` that reports the error instead of hanging.
- Status register fully derived from the dot/line position: PAL, DMA busy, HBlank,
  VBlank, odd frame, sprite collision/overflow latches, VINT pending, FIFO empty and
  FIFO full. A control-port status read reports the current value, cancels only a
  half-latched command word and never acknowledges an interrupt.
- HBlank/VBlank requests are generated by the VDP at the line/dot boundary and gated
  by register `0x00` bits 4/5; VBlank (level 6) is delivered before HBlank (level 4)
  and the 68000 `SR` mask still decides admission. Acknowledging the delivered source
  leaves the other request pending.
- Line timing: 3420 master cycles per line, 262-line NTSC and 313-line PAL frames,
  a monotonic frame counter, and strict line/dot snapshot validation. The H counter
  and the power-on scanline phase are documented as approximate.
- Rendering: plane A/B and window nametables with horizontal/vertical scrolling,
  sprite attribute-table entries with collision and overflow latches, priority
  composition, shadow/highlight palette flags, the register-`0x07` border and a
  deterministic H40/H32 frame buffer. The compositor is a pure function of the
  canonical VDP state and the master-cycle timeline. H32 window geometry, line-mode
  vertical scrolling and column-scroll details are approximations; interlace is
  absent and physical byte-swapped VRAM is not modelled.
- DMA: VRAM fill and VRAM copy, pre-validated with typed errors, DMA-busy status and
  deterministic busy windows. The 68000-memory modes (`memory→VRAM`, `memory→CRAM`
  and `memory→VSRAM`) are **rejected** with
  `VdpError::DmaMemorySourceUnavailable { mode, words }` before any byte is read or
  written: the transfer engine's source is a VRAM address in this slice, so the old
  behaviour silently copied VRAM onto VRAM and claimed a memory transfer. The
  register path (`0x13..0x17`, modes 0/1) and the data-port DMA command reach the same
  rejection; only modes 2 (fill) and 3 (copy) transfer. 68000-side DMA and its stalls
  are not modelled.
- FIFO: a bounded four-word write queue with occupancy/backpressure; full-queue
  writes cost a 68000 wait slot that the backend converts to master cycles.
- Snapshot (`CFVDP002`) carries the registers, CRAM, VSRAM, FIFO, DMA, frame index,
  line/dot and VRAM with atomic validated restore; `CFVDP001` v1 is rejected.
- Media: `MegadriveBackend` implements `MediaSource` and emits one deterministic
  `VideoFrame` per completed VDP frame (`256x224`/`320x224` in NTSC and `256x240`/
  `320x240` in PAL), converting palette indices through 9-bit CRAM RGB to RGBA8 with
  opaque alpha. A one-frame queue returns `None` on a second take before a new frame.
  The same seam emits signed 16-bit stereo `AudioChunk`s at exactly 48 000 frames per
  emulated second: the integer YM2612/PSG mixer/resampler runs off elapsed master
  cycles, the backend holds one bounded 64-frame chunk (about 1.3 ms) per take, and a
  one-second mixer queue drops the oldest frames deterministically if a consumer falls
  behind. See [audio-state-differential-evidence.md](../research/audio-state-differential-evidence.md).
- Evidence: the pinned Exodus VDP state differential is recorded in
  [vdp-state-differential-evidence.md](../research/vdp-state-differential-evidence.md);
  the backend tests `vdp_ports`, `vdp_timing`, `vdp_planes`, `vdp_sprites`,
  `vdp_compose`, `vdp_dma`, `vdp_fifo`, `vdp_interrupts`, `vdp_snapshot` and
  `vdp_frame` cover the same surface.

### Deterministic interrupt scheduler

- Master-cycle model: canonical time is the master-clock cycle. Each executed
  instruction advances the backend by its 68000 cycle cost times seven, and one
  scanline is exactly 3420 master cycles (`MASTER_CYCLES_PER_LINE`). This is
  explicitly **not** fully cycle-accurate hardware timing: functional prefetch,
  bus contention, DMA stalls and interrupt latency are not modelled. The former M3
  instruction-quantum model is superseded.
- NTSC constants (262 lines/frame, 224 active) and PAL constants (313/240), with
  the fixed 3420-master-cycle scanline; `SchedulerConfig::new(video)/ntsc()/pal()`
  carries only the video standard.
- The VDP requests HBlank when each active scanline completes at a line/dot boundary
  and VBlank when vertical blanking begins; blanking lines are otherwise quiet. The
  scheduler delivers, acknowledges and snapshots pending requests as described here.
- At most one request per level is retained; repeated requests coalesce with
  diagnostic counters, and a masked request keeps its original issue tick. Delivery
  order is VBlank (level 6) before HBlank (level 4).
- The scheduler keeps pending requests independently of the CPU mask and exposes
  `next_deliverable(raw_sr)`, which applies the shared 68000 rule: level 7 is always
  deliverable and every other level must be strictly above the `SR` interrupt
  priority mask. The backend, not the scheduler, performs delivery and acknowledge.
- Versioned snapshot (`CFINTSCH`, 56 bytes plus pending tick fields) with atomic
  validated restore and configuration-mismatch rejection.

### Backend integration

- `MegadriveFactory` validates ROM size (8 bytes .. 4 MiB), hashes the immutable
  ROM, video standard and scheduler configuration into `config_hash` (`CFMDCONF7`).
- `MegadriveBackend` implements the existing `Backend` contract: execution,
  snapshots, restore, inspection, memory reads/writes.
- Snapshot payload magic is `CFMD68K7`, payload version **7**, followed by the
  68000 register file (including `USP`, `SSP`, the 68000 cycle counter and the
  `STOP` latch), `PC`, `SR`, halted flag, tick, 64 KiB RAM, the complete `CFVDP002`
  VDP snapshot (including the frame index), the cartridge state, the complete
  master-cycle scheduler snapshot and the phase-8.4 block (the Z80 register snapshot
  and 8 KiB RAM, the Z80 bank register and master-cycle carry, both arbitration
  latches, the eight port 1/2/3/EXT control register bytes at `0xA10008..=0xA1000F`,
  the complete YM2612 and PSG state and the mixer frame phase plus its three
  resampler phases/histories). Versions 1-6 are rejected with a typed error
  rather than reinterpreted.
- The state hash (`CFMDSTATE7`) covers the extended 68000 state, tick, RAM, VDP
  snapshot, scheduler snapshot and the phase-8.4 Z80/arbitration/YM2612/PSG/mixer
  block, so pending interrupts, stack-pointer mirrors,
  VRAM/CRAM/VSRAM and the frame index are part of checkpoint identity.
- Restore validates backend identity, backend version, envelope version, config
  hash, payload magic/version/length, the VDP and scheduler snapshots, and the
  recomputed state hash. The replacement bus and scheduler are built before they
  are committed, so a rejected restore leaves the machine untouched.
- At most one interrupt is delivered at each instruction boundary, before the next
  instruction executes, and only when `next_deliverable(cpu.sr())` admits it: level 7
  is always taken and every other level must be strictly above the IPM. A request
  raised by the scheduler at the end of a step is therefore inspectable at that tick
  and is taken deterministically on a later step if the mask admits it. VBlank
  (level 6) is taken before HBlank (level 4).
- Masked requests are retained with their original issue tick and are not
  acknowledged, dropped or duplicated. Delivery acknowledges only the level that
  entered the CPU, so a pending request at the other level survives. A failed
  exception entry acknowledges nothing. Exception entry loads the accepted level
  into the handler's IPM, so a lower-priority VDP request cannot nest before `RTE`
  restores the interrupted raw `SR`.
- VDP VBlank status is synchronized from the scheduler's scanline position. The
  derived VINT-pending status bit is recomputed from the scheduler's pending VBlank
  request after every step, after delivery and on restore; it is not part of the
  serialized VDP snapshot.
- `MachineInspection::registers` additionally exposes `vdp_status`,
  `vdp_auto_increment`, `vdp_address`, `irq_mask`, `irq_pending_level`,
  `irq_deliverable`, `irq_vector`, `scheduler_frame` and `scheduler_line` as
  pseudo-registers. `irq_deliverable` is 1 when the highest-priority pending
  request is admitted by the current SR mask and 0 while it is masked. Phase 8.4
  extends the same open map with `z80.af/bc/de/hl/ix/iy/sp/pc/i/r/im/iff1/iff2/
  halted/ei_delay/credit/bank`, `arbitration.requested/reset_asserted/z80_runs`,
  `ym.status/lfo_phase/address_latch0/address_latch1`, `psg.latch/tone0-2/
  attenuation0-3/noise_control/noise_lfsr` and `audio.pending_frames/
  dropped_frames/frame_accumulator`. The schema has no dedicated device-inspection
  section; this keeps the generic backend API unchanged.
- Capabilities truthfully report `breakpoints = false` and `tracing = false`: host
  breakpoints are a debugger feature and the bounded trace is a concrete-type debug
  facility, not the generic `Backend` `tracing` capability. Since M4 the slice reports
  `watchpoints = true` and `debug_events = true` for its optional observation layer; since
  phase 8.3 it reports `rendering = true` (deterministic headless frames) and since phase
  8.4 `audio = true` (48 kHz stereo capture through `MediaSource`).

### Z80, bus arbitration and audio (phase 8.4)

- **Z80 core:** the documented instruction set (main table, `CB`, `ED`, `DD`/`FD`
  prefixes), IM 0/1/2 and NMI acceptance, the one-instruction `EI` delay, HALT wake
  and a versioned 43-byte register snapshot. One Z80 T-state is
  `MASTER_CYCLES_PER_Z80_T_STATE = 15` master cycles. Documented limits: `IXH`/
  `IXL`/`IYH`/`IYL` and `WZ`/`MEMPTR` are not modelled; undocumented `ED` opcodes
  are `NOP`s; block-operation undocumented X/Y flags stay clear; repeated `DD`/`FD`
  prefixes cost one bounded step.
- **Z80 memory map:** 8 KiB RAM mirrored through `0x3FFF`, YM2612 at
  `0x4000..=0x4003` (repeated across `0x4000..=0x5FFF` by the `0x0003` mask),
  bank-register write at `0x6000`, VDP port window at `0x7F00..=0x7F03`, PSG at
  `0x7F11` plus the mirror `0x7F10..=0x7F17` (read returns `0xFF`) and a 32 KiB
  banked window at `0x8000..=0xFFFF` that forwards writes to its translated 68000
  target. The 68000 sees the 8 KiB Z80 RAM at
  `0xA00000..=0xA01FFF`, the nine-bit bank register at `0xA06000` and the
  request/reset latches at `0xA11100`/`0xA11200`.
- **Arbitration:** `0xA11100` carries the Z80 bus request with the hardware `ZBR`
  polarity: setting write bit 8 **asserts** the request, which stops the Z80 and gives
  the 68000 the Z80 window, and clearing it returns the bus to the Z80. Read bit 8
  reports ownership — one while the Z80 still has the bus, zero once it has yielded — so
  a guest poll loop on `0xA11100` terminates as on hardware. `0xA11200` asserts/releases
  the active-low reset. The model
  is functional, not microcycle-exact: the yield happens on the request write rather than
  at the Z80 instruction boundary, and while the Z80 owns the bus a 68000 access to
  the Z80 RAM window is stalled by a fixed 60 master cycles per byte lane. The
  Z80 runs while no request is asserted and the reset is released. `Backend::step` runs
  the Z80 inside each 68000
  instruction's master-cycle window while arbitration allows it and carries the last
  instruction's overshoot in canonical `z80_credit`; `advance_z80` remains available
  for explicit single-instruction control.
- **YM2612:** six four-operator FM channels, eight algorithms, feedback, envelope
  generators, LFO AM/PM, detune/multiple, timer A/B counters/status and the
  channel-6 DAC, reachable through two register port pairs on both the Z80 and the
  68000 window. Timer overflows latch the chip IRQ line and status flags; delivery
  to the Z80 is phase 8.5. `SSG-EG`, channel-3 special mode, CSM and the `0x21`/
  `0x2B` test bits are stored but not applied; the status busy bit is absent; the
  envelope/amplitude curves and LFO are deterministic integer approximations.
- **PSG:** three square-wave tone channels with ten-bit dividers, a 15-bit maximal
  white-noise LFSR (period 32767 from the `0x4000` reset value) with periodic mode, the
  documented noise shift periods — the chip-clock divisions `N/512`, `N/1024` and
  `N/2048` and the full tone-2 period, i.e. 32/64/128 internal sample ticks, so the
  fastest shifts at `53 693 175 / (32 × 240)` ≈ 6991 Hz at the NTSC master clock —
  sixteen attenuation levels
  and the latch/data write protocol. The attenuation table is a monotonic integer
  approximation of the documented 2 dB steps; the PSG is write-only and a read
  returns the open value `0xFF`.
- **Mixer and resampler:** the YM2612 and PSG sample clocks (one sample per 1008 and
  240 master cycles) are advanced by elapsed master cycles, linearly interpolated
  with an integer rational phase and mixed with a saturating `i16` add into stereo
  frames. Output frames are emitted when `master_cycles * 48_000` reaches the
  configured master clock (NTSC by default, PAL through `for_video_standard`), so one
  emulated second is exactly 48 000 frames; the result is independent of how the
  caller chunks `advance_master`. PSG is panned centre; YM2612 left/right follow its
  documented stereo enable bits. Each device's interpolated output lags its newest
  sample by at most one device period (≤1008 master cycles), which is documented
  model latency, not host latency. A restore starts the media queues empty, like the
  video frame queue; canonical mixer phase and both device clocks round-trip.
- **Snapshot:** payload v7 (`CFMD68K7`/`CFMDSTATE7`/`CFMDCONF7`, `PAYLOAD_VERSION = 7`)
  embeds the Z80 register snapshot and 8 KiB RAM, the Z80 bank register and
  master-cycle carry, both arbitration latches, the eight port 1/2/3/EXT control
  register bytes, the complete YM2612 (856 B) and PSG (34 B) state and the mixer frame
  phase plus its three resampler phases/histories (56 B) in both the payload and the
  state hash. v1–v6 payloads are rejected with typed errors and a rejected restore
  leaves the machine byte-identical.
- **Evidence:** the pinned-oracle device-state differential
  ([audio-state-differential-evidence.md](../research/audio-state-differential-evidence.md))
  compares 25 Z80/PSG/YM2612-status fields at a shared Z80 HALT boundary and records
  the YM2612-reflection limit; the arbitration-polarity divergence that run documents is
  superseded by the phase-8.4 repair, which implements the hardware `ZBR` polarity (the
  evidence page carries the attributed supersession note). `z80_*`, `psg`,
  `audio_mix`, `ym2612*`, `arbitration` and `audio_snapshot` tests cover the same
  surface internally. No cross-emulator **sample** equality is claimed.

### Bounded instruction trace (debug only)

- `trace` module: `TraceEntry` carries the zero-based instruction `index`, fetched
  `pc` and `opcode` word; `InstructionTrace` is a fixed-capacity ring.
- Disabled by default. `M68000` and `MegadriveBackend` expose enable/disable/reset/
  trace/take; the generic `Backend` trait has no trace method.
- One record is appended after a successful opcode fetch and before decode, so an
  instruction that then fails as illegal or unsupported is still visible. A fetch
  that fails before the opcode word records nothing; interrupt exception entry is
  not an instruction and is not recorded.
- Capacity must be at least 1 (`TraceError::ZeroCapacity`). Indexes keep counting
  when the ring evicts its oldest records, and `dropped`/`first_index` expose loss;
  storage is allocated lazily and bounded by the capacity.
- Debug data only: enabling the trace leaves snapshot payload bytes, versions, state
  hashes and configuration hashes identical, and restore does not rewind the trace.
- CLI `megadrive trace` runs an instruction budget or stops before `--stop-pc`,
  writes TSV `index<TAB>pc_hex<TAB>opcode_hex` (and, with `--cycles`, a fourth
  decimal 68000-cycle field) and a deterministic key=value final state (executed,
  tick/master_cycles, cpu_cycles, USP/SSP, video, halted, D0–D7, A0–A7, PC, raw
  SR, RAM base/length/hex), and honours `--video`, `--trace-capacity`, `--out`/
  `--trace-out` (default stdout) and required `--state-out`. The instruction-quantum
  flag is removed.

### Oracle differential method and limits

The reproducible harness `scripts/oracle-diff.py` compares the minimal core against
a pinned Exodus headless oracle for eleven independently authored synthetic ROMs; the
full numbers, commands and limitations are in the
[68000 cycle differential evidence](../research/cpu-cycle-differential-evidence.md).

- Both sides produce an instruction trace; the harness projects each to the ordered
  sequence of executed instruction **addresses** before a marker self-loop and
  compares the per-instruction 68000 cycle cost (`cycle` column delta versus
  ChronoFork's fourth trace field). Because the oracle renders
  opcodes as mnemonic+args while ChronoFork stores raw words, every ChronoFork
  projected opcode is checked against the ROM word at that address instead of being
  compared directly to the oracle column.
- It asserts equal projection length and addresses, reports record counts and
  projection SHA-256, reports the first divergence with preceding context, and exits
  1 on divergence. A per-ROM negative control corrupts one projected record and
  requires the comparator to fail at that index.
- Final state is compared through the oracle's MCP `cpu.registers` and
  `cpu.read_memory` calls: D0–D7, A0–A7, PC, effective SR fields (IPM/S/CCR) and 64
  work-RAM bytes at `0xFF0000`. Raw SR words differ and are not compared.
- Phase 8.2 compares the master-cycle timeline: the oracle's accumulated `cycle`
  column difference per instruction is checked against ChronoFork's fourth trace
  field (`InstructionOutcome.cycles`, emitted by `--cycles`). Absolute cycle bases
  and the oracle `time` column are not compared. M3's per-ROM instruction quantum
  and masked-boundary control are superseded.
- The eleven synthetic ROMs are the three historical ones plus EA, shift, BCD,
  MOVEM/MOVEP, mul/div, EXG/TAS/ADDX/SUBX, system/status-register and bit/DBcc
  fixtures under `scripts/fixtures/`. The regenerated repair run passes: address projections
  11/11, per-instruction cycle comparisons 11/11, selected final states 11/11, and
  address/cycle/final-state negative controls 11/11; the harness exits 0. No
  divergence remains in the compared observables and no expectation was edited to
  obtain the pass. Full numbers, commands and the pinned
  oracle revision `ec005615…` are in the
  [cycle differential evidence](../research/cpu-cycle-differential-evidence.md).
- History: the pre-repair address divergence at `0x00027A` (absolute-long decode)
  remains recorded in the historical
  [oracle differential evidence](../research/oracle-differential-evidence.md); the
  M3 quantum/masked-boundary results there are marked superseded for timing.

**This is not cycle, interrupt-timing or interrupt-delivery equivalence.** The
masked-ROM pass shows that no handler executes while the SR mask is 7 and that the
plain instruction stream and selected final state agree; it does not compare
delivery order, acknowledge semantics, latency, vector fetch or handler execution
against the oracle. It also does not compare timing, prefetch, bus cycles, rendering,
DMA or audio samples, and it does not establish general 68000 or game
compatibility. The separate phase-8.4 device-state differential compares Z80, PSG
and YM2612 status state at a shared Z80 HALT boundary; it is not a sample
comparison; the arbitration-polarity divergence its recorded run documents is
superseded by the phase-8.4 repair, which implements the hardware `ZBR` polarity.

### Known limitations of the integration

- **No declared checkpoint payload bound (temporary, tracked).** The service layer decides
  `max_bytes_per_session` before it captures a checkpoint and needs a payload size it can
  bound; after `run`/trace/debug write that bound must come from
  `Backend::snapshot_payload_bound()`, which this backend does not declare yet. Under a
  **finite** byte quota a post-run `session.snapshot`/`session.fork` over the service is
  therefore refused with `limit_exceeded` **before allocating** (the unbounded in-process
  movie/player/rewind paths are unaffected). This is **not** intended behavior: owner
  decision O-5 defers the fix to W1.8 in the
  [consolidation plan](../superpowers/plans/2026-11-20-consolidation-and-agent-api.md),
  which declares the bound here (validated against every supported payload) and adds the
  service+Mega Drive regression.

- **Privilege is enforced for the implemented system instructions.** `MOVE` to
  `SR`, `STOP`, `RESET`, `RTE` and `MOVE` to/from `USP` raise vector 8 in user mode;
  the exception framework provides vectors 3-8/24-31/32+n, short frames and the
  documented 14-byte address-error frame. The documented MC68000 instruction set
  is implemented; guest bus-error (vector 2) and trace (vector 9) delivery,
  automatic trace mode and the remaining exception-group edge cases are absent.
- **Only VBlank (level 6) and HBlank (level 4) sources are modelled.** The "level 7
  is always deliverable" branch is a tested predicate rule in the shared
  68000/scheduler API, not a wired level-7 interrupt source. Other exception groups
  (timer sources, timer IRQ delivery to the Z80, bus-error recovery and trace) are
  absent. Unmapped and
  write-protected guest data accesses take the guest address-error exception
  (vector 3) with the documented 14-byte frame and function code; odd/misaligned EA
  accesses, unrecognised/reserved encodings and the implemented
  `TRAP`/`TRAPV`/`CHK`/`ILLEGAL` forms also use 68000 vectors. Host errors are
  reserved for host-level failures (a frame or vector fetch that cannot complete,
  stack/address overflow, trace capacity), not for guest data accesses. Instruction
  and extension-word fetches are not converted: an unmapped or protected fetch
  remains a typed host bus error, which is the declared guest bus-error (vector 2)
  residual, distinct from the data accesses that take vector 3.
- **No interrupt-timing equivalence is claimed.** Delivery happens at instruction
  boundaries on the master-cycle timeline (cycle cost added at the boundary), at
  most one exception per boundary; the oracle differential does not compare delivery
  order, latency, acknowledge semantics, vector fetch or handler execution.
- **VINT pending/acknowledge semantics are minimal.** Status bit 7 is driven only by
  the scheduler's pending VBlank request. VDP interrupt-enable registers, VDP-side
  acknowledge logic, controller-port interrupt acknowledge and other VDP interrupt
  sources are not modelled. A status-port read never acknowledges a request; only
  backend delivery does. Only VBlank (bit 3), derived VINT pending (bit 7) and
  FIFO-empty (bit 8) can change; all other status bits are documented constants.
- **Pending requests are one per level.** A masked request is retained with its
  original issue tick and repeated requests of the same level coalesce (diagnostic
  counters record coalescing); there is no interrupt queue.
- Reset CCR/X is architecturally undefined: the oracle powers on with `X=1` and
  unused raw-SR bits set (raw `SR 0x7FE0`), while ChronoFork resets to raw `SR
  0x2700` (CCR `0`). Effective fields are compared and the harness ROMs clear `X`
  explicitly with `ADD.B D0,D0` before the measured section, so an unwritten reset
  flag cannot masquerade as a state divergence. Raw `SR` words are still not
  compared.

### Tests

`chronofork-megadrive` contributes unit and integration tests covering:

- reset vectors and register initialization;
- every implemented 68000 instruction form, flags, `DBRA` termination and
  interrupt-entry success/error paths;
- the 68000 mask rule (`interrupt_level_deliverable` / `accepts_interrupt_level`):
  level 7 always admissible and every other level strictly above the IPM; invalid
  levels rejected before state changes;
- interrupt entry loading the accepted level into the handler IPM, including
  level 7, while preserving the CCR; `MOVE.W #imm,SR` writing the raw `SR`/IPM; and
  `RTE` restoring raw `SR`, mode and CCR, PC and A7 while rejecting underflow,
  unmapped frames and odd PCs without touching state;
- ROM/RAM big-endian access, alignment, unmapped and atomic-write errors;
- VDP port mapping through the bus, register/VRAM round-trips, byte-lane behavior,
  side-effect-free debugger reads, mapped byte reads of the two controller data
  registers and explicit other-controller-access errors (the YM2612 window is wired
  and the PSG window returns its documented open value);
- Z80 instruction groups, interrupts, memory map and bank window, arbitration
  latches and the 60-master-cycle stall, including a synthetic Z80 program that
  progresses through ordinary `Backend::step` calls;
- YM2612 registers, timers/status, DAC, envelopes, algorithms and LFO, and PSG
  latch/data protocol, tone/noise channels, attenuation and open-value reads;
- the integer mixer and rational resampler: exactly 48 000 stereo frames per
  emulated second with tolerance 0, chunking independence, saturating mix,
  centre-panned PSG and bounded media queues;
- payload v6 round-trip and byte-identical restore of the Z80, arbitration, YM2612,
  PSG and mixer phase, typed rejection of v1–v5 and the inspection pseudo-registers;
- VDP snapshot round-trip and malformed-input rejection; VDP VINT-pending driven by
  the owning machine, status reads never clearing it, and the derived latch being
  excluded from the VDP snapshot;
- deterministic tick-driven HBlank/VBlank generation, coalescing, priority,
  scanline timing and scheduler snapshot rejection; `next_deliverable` reflecting
  the 68000 mask;
- VDP register/VRAM programming by the synthetic 68000 program through the bus;
- masked HBlank retention until the program lowers the mask, delivery of only the
  delivered level, VBlank-before-HBlank delivery, and the masked request surviving
  with its issue tick;
- VBlank handlers at IPM 6 blocking pending-HBlank re-entry until `RTE` restores
  the program mask, followed by exactly one later HBlank delivery;
- masked VBlank keeping VINT pending across many steps without acknowledging,
  status-port reads not acknowledging masked requests, and VBlank acknowledge
  clearing only the VBlank request;
- snapshot/restore including VDP and pending-interrupt state, derived VINT
  recomputation, old/malformed payload rejection and configuration mismatch;
- session fork/restore with VDP and pending-interrupt state, including forking a
  masked request and delivering it after the mask is lowered, plus source isolation;
- device-level determinism across two independent machines;
- controller data-register reads: the exact `0xA10003`/`0x00A10005` byte map, active-low
  TH-high bit values, two independent ports, neutral `0x7F`, A/Start/high-bit inertness,
  typed writes and debugger rejection, step-scoped clearing on success and failure, no
  controller field in the snapshot payload/state/config hashes, checkpoint replay with the
  next supplied frame, and `controller_rom()` observed work-RAM values (6 integration tests
  in `tests/controller.rs`, plus in-crate unit/property tests in `snapshot_properties.rs`);
- bounded trace semantics: zero-capacity rejection, eviction/dropped/index
  reporting, clear/take, illegal-opcode recording, `MOVE.W #imm,SR`/`RTE`
  recording, and byte-identical snapshots, state hashes and payload versions with
  tracing on and off.

CLI integration tests cover deterministic trace and state files across two runs,
`--stop-pc`, stdout trace mode, a zero instruction budget, option validation, help
text, the master-cycle timeline and removed instruction-quantum flag, and
deterministic `megadrive demo` output including masked HBlank/VBlank retention,
VINT status, mask lowering, handler IPM, `RTE` restoration and no nested HBlank
handler. The `tas_hardware.rs` CLI process target (13 tests) covers
hardware record/play/verify/edit/studio with `--rom`, two-controller v2 input, the
synthetic-default regression and the typed ROM/backend errors. `cargo test --workspace`
is the source of truth for the current count.

## Implemented: controller-port input for TAS

[ADR-0009](../adr/0009-real-hardware-input-tas.md) is accepted, implemented and
independently verified for real-hardware input TAS (t5 PASS, 2026-11-16; M3-A45–A53), on
the deterministic minimal-slice scope and with the canonical Mega Drive state unchanged.

- Byte reads of `0xA10003` (controller 1) and `0xA10005` (controller 2) are driven by
  the current step's `InputFrame` ports 0 and 1. Only the TH-high phase of a 3-button
  pad is represented: bits 0–5 are Up/Down/Left/Right/B/C active-low, bit 6 is the
  TH-high level (1) and bit 7 is 0; a neutral port reads `0x7F`. A and Start require
  the TH-low phase and are **not** readable; the A/Start bits in the frame are inert.
- The input is transient and per-step: it is not serialized into the snapshot, not
  part of the state body, state hash or configuration hash, and cleared after every
  step. A restored machine uses the next step's input. No canonical Mega Drive byte
  changes; the differential payload/envelope/meta probe, the golden snapshot/state
  assertions, the demos, the oracle/state probes and all existing M2/M3 movie fixtures are
  byte-identical to the pre-change baseline.
- Every other controller-window access, including all writes and the version,
  control, port-3 and TX/RX addresses, stays a typed `UnmappedRead`/`UnmappedWrite`;
  the side-effect-free debugger path does not map step-scoped input.
- The CLI exposes `--rom` on `tas record`/`play`/`verify`/`studio`/`edit` and keeps
  `compare` backend-agnostic. The synthetic default path and its bytes are unchanged.
- Acceptance evidence is the [M3 hardware acceptance
  plan](../quality/m3-hardware-acceptance.md) rows M3-A45–A53, `executed - passed`
  (t5, 2026-11-16), and the ignored `.research/m3-hardware/verification/` artifacts. No
  controller, TH/6-button/Start, timing, commercial-ROM or accuracy claim is made.

## Implemented: cartridge, region and TMSS (M8 phase 8.1)

Phase 8.1 turns the immutable ROM blob into a modelled cartridge
([design](../superpowers/specs/2026-11-17-8.1-cartridge-region-tmss-design.md),
[plan](../superpowers/plans/2026-11-17-8.1-cartridge-region-tmss.md)). The
`chronofork-megadrive` cartridge device owns the image, the parsed header, the Sega mapper
register file, 64 KiB battery SRAM and the TMSS latch; the bus delegates every cartridge
window to it.

- **Header.** System signature at `0x100` (`SEGA MEGA DRIVE` / `SEGA GENESIS`; other
  signatures rejected), bounded ASCII domestic/overseas names and serial, stored checksum
  at `0x18E` with the computed Mega Drive **word sum over `0x200..`**, ROM start/end at
  `0x1A0`/`0x1A4`, the SRAM specification (`RA` marker at `0x1B0`, start `0x1B4`, end
  `0x1B8`) and the region bitfield at `0x1F0`. A missing or unknown region, a short image
  and a declared end beyond the image are typed errors; a checksum mismatch is diagnostic
  only.
- **Sega mapper.** Always present and identity by default, so images up to 4 MiB read
  linearly. Window 0 is fixed to bank 0; windows 1–6 at `0x080000 + i × 0x80000` use the
  nine-bit registers `0xA130F3..0xA130FF`. A selected page beyond the image is a typed
  `UnmappedRom`; odd word/long accesses and ROM-window writes are typed errors.
- **Battery SRAM.** 64 KiB canonical state, zeroed at construction, mapped only while
  `0xA130F1` bit 0 is set and write-protected by bit 1 (a rejected write mutates no byte).
  `import_sram` is the explicit construction-time host save-file seam and requires exactly
  64 KiB; the CLI imports and exports raw 64 KiB images.
- **TMSS.** Modelled and **disabled by default**. `Enabled` blocks VDP writes until a long
  write whose bit 9 is set reaches `0xA14000`; while locked a VDP write is acknowledged and
  discarded, leaving the VDP registers and its address latch untouched. `0xA10001`
  reports `(export << 7) | (pal << 6) | no-disk | tmss-capable`.
- **Video standard.** Derived from the header (PAL only for an `E`-only region) with an
  explicit `MegadriveConfig { video, tmss }` override for tests and region-mismatched
  dumps. Both are configuration, never mutable state.
- **State (current, after the W1 bus phase).** Payload v7 (`CFMD68K7`, `PAYLOAD_VERSION = 7`),
  state hash v7 (`CFMDSTATE7`) and configuration hash v7 (`CFMDCONF7`). Phase 8.1 used
  v3, phase 8.2 v4, phase 8.3 v5 and phase 8.4 v6. The v7 body keeps the seven mapper
  banks, the SRAM control byte, 64 KiB SRAM, the TMSS model/lock latch, the extended
  68000/scheduler state and the complete VDP state, and adds the eight port 1/2/3/EXT
  control register bytes at `0xA10008..=0xA1000F` on top of the phase-8.4 block (the Z80
  register snapshot and 8 KiB Z80 RAM, the nine-bit Z80 bank register, the Z80
  master-cycle carry, both arbitration latches, the complete YM2612 and PSG state and the
  mixer frame phase plus its three resampler phases/histories). The state hash is derived
  from the same body; v1-v6 payloads are rejected with typed errors, a payload carrying
  another TMSS model is rejected even when its hash was recomputed, and a rejected
  restore leaves the machine untouched. Queued video frames and audio chunks are derived output: a restore starts
  both media queues empty. The configuration hash covers the ROM SHA-256, video
  standard, TMSS model and a mapper/SRAM feature tag; SRAM bytes are state and never
  configuration.
- **Inspection.** `CartridgeView` plus bounded `cartridge.*`, `mapper.bank0..6`, `sram.*`
  and `tmss.locked` pseudo-registers.
- **CLI.** `chronofork megadrive cartridge --rom <FILE> [--sram-in <FILE>]
  [--sram-out <FILE>]` prints the parsed facts, and `chronofork megadrive boot --rom <FILE>
  [--frames <N>] [--tmss enabled|disabled]` steps the synthetic boot cartridge to its
  marker with a bounded frame budget (non-zero exit when the marker is not reached).
- **Evidence.** The independently authored 2 MiB `cartridge_boot_rom()` unlocks TMSS,
  enables SRAM, selects bank 2, writes SRAM and reaches `BOOT_MARKER`; the two curated
  fixtures (Kid Chameleon, Chakan) are parsed and their system/region/serial/checksum
  pinned when the ignored `.research/roms/` files are present. Booting a commercial title
  to its main loop is **not** claimed here — that is phase 8.6.

## Not implemented

- VDP interlace, physical byte-swapped VRAM layout, exact H32 window geometry,
  line-mode vertical scrolling, column-scroll details, dot-exact H/V counters and the
  command code `0b1100` (8-bit VRAM read). VDP rendering, planes, window, sprites,
  scrolling, CRAM/VSRAM addressing (read and write), DMA, FIFO/CPU wait cycles and line
  timing are implemented (phase 8.3; the VSRAM read/write codes and the length-minus-one
  DMA length are the W1 phase).
- YM2612 register/channel state equality against another emulator: the pinned
  oracle publishes no YM2612 register page, so only the readable status byte is
  compared; the device models are functional, not hardware-exact (see the phase 8.4
  section and the [audio state differential](../research/audio-state-differential-evidence.md)).
- Audio sample capture to a file or device (WAV/streaming) belongs to phase 8.7;
  phase 8.4 captures boundedly into `MediaSource` only, with no host audio device.
- Z80 timer IRQ delivery, microcycle-exact bus arbitration (the model yields on the
  request write rather than at the Z80 instruction boundary; see the **Arbitration**
  bullet in the phase-8.4 section above), timer/other interrupt
  sources and controller-port interrupt acknowledge.
  Guest bus-error (vector 2) and trace (vector 9) exception delivery are not
  implemented; privileges are enforced for the implemented system instructions.
- Controller input beyond the TH-high 3-button subset: A and Start, the TH latch/direction
  register and TH-low phase, 6-button pad (X/Y/Z/Mode), controller timing/interrupts, port 3,
  and analog/keyboard hardware. The implemented data-register reads are described above and
  in [ADR-0009](../adr/0009-real-hardware-input-tas.md).
- Cartridge mapper variants beyond the Sega banking above (interleaved dumps, 32X/Pico
  hardware, images above the 64 MiB mapper ceiling) and SRAM banking schemes other than the
  64 KiB window.
- 68000 prefetch, bus-cycle timing, bus contention, DMA stalls and interrupt
  latency. The instruction set and documented addressing modes are implemented, and
  the register file carries separate user/supervisor stacks.
- The Mega Drive TAS read-modify-write write-back quirk (some consoles suppress the
  write-back phase) is a bus/hardware behaviour, not a 68000-core behaviour; the
  core performs the documented write. The oracle fixture therefore exercises the
  TAS register forms and the memory write is covered by unit tests.
- Hardware cycle accuracy. Each instruction advances the master-cycle timeline by
  its documented 68000 cycle cost times seven; cost is added at the instruction
  boundary rather than distributed across internal microcycles. This is
  deterministic instruction-level timing, not a bus-cycle model.
- A ROM database, region-mismatch handling beyond the derived standard and its explicit
  override, or any commercial-ROM boot. Header/checksum/region parsing and the Sega mapper
  are implemented; the `megadrive trace` and TAS `--rom` commands accept an explicitly
  supplied image, and the demo uses an independently authored synthetic ROM.
- General structured/timestamped tracing or bus/device traces, historical trace
  queries, and a reusable in-product trace-comparison API. Only the narrow debug TSV
  and the external oracle harness exist.
- Differential evidence beyond the eleven synthetic ROMs, positive differential
  comparison of interrupt delivery/timing, and any differential claim about DMA
  content, full-hardware timing or game compatibility. The 11-ROM run matches
  instruction addresses, per-instruction cycle costs and selected final state; the
  masked-ROM run is negative gating evidence only. The phase-8.3 VDP differential
  compares the oracle-visible status/counter readbacks at synchronized boundaries and
  ChronoFork's canonical registers/VRAM/CRAM against the programmed scenes; the oracle
  cannot expose VDP memory content, and the absolute status/counter phase is recorded
  as a scoped divergence in
  [vdp-state-differential-evidence.md](../research/vdp-state-differential-evidence.md).
  The phase-8.4 audio/Z80 differential compares 25 device-state fields at a shared
  Z80 HALT boundary, records the YM2612-reflection limit (the arbitration-polarity
  divergence its recorded run documents is superseded by the phase-8.4 repair), and
  claims no cross-emulator sample equality
  ([audio-state-differential-evidence.md](../research/audio-state-differential-evidence.md)).

Unrecognised and reserved opcodes raise the guest illegal-instruction exception;
unmapped port accesses return typed errors. Neither is silently skipped.

## CLI demo

```sh
cargo run -p chronofork-cli -- megadrive demo
```

The demo executes `1 + 2`, programs VDP register `0x0F` (auto-increment 2), writes
and reads back VRAM word `0x1234`, and captures the VDP status. On the master-cycle
timeline with the reset mask 7 it shows the first HBlank request pending at tick
3472 with no handler, then demonstrates snapshot/fork/restore with the pending
request preserved. A mask-7 interrupt ROM reaches masked VBlank with the VDP
VBlank/VINT-pending status bits set and restored from a checkpoint. The mask-3
interrupt ROM delays past two scanlines, lowers the mask at tick 7378, delivers the
retained HBlank at tick 7406 with the handler at IPM 4, and `RTE` at tick 7574
returns to PC `0x000210` with raw `SR` `0x2300` (mask and CCR restored). VBlank is
then entered at IPM 6 before the pending HBlank, so no HBlank handler nests before
`RTE` restores the program mask; the checkpoint preserves the pending VBlank/VINT
state. It prints `chronofork megadrive demo: PASS`.

Timing note: the round-4 system-timing correction moved the interrupt ROM's
retained-HBlank line from tick 3472 to tick 3458; the demo's first masked-HBlank
line remains 3472.

## CLI instruction trace

```sh
cargo run -p chronofork-cli -- megadrive trace \
  --rom <rom-path> --state-out <state-path> \
  --instructions 10000 --stop-pc 0x352 --cycles \
  --ram-bytes 64 --trace-out <trace-path>
```

`megadrive trace` is the evidence-producing CLI surface. It runs a decimal
instruction budget (or stops before `--stop-pc`), records a bounded ring of
executed `index`/`pc`/`opcode` records (plus a fourth 68000-cycle field with
`--cycles`) and writes the TSV trace plus a deterministic key=value final state.
`--trace-capacity` bounds the ring (default: the instruction budget, capped);
`--video` selects NTSC/PAL timeline constants; `--trace-out` defaults to stdout;
`--state-out` is required. The trace is debug data: it does not change snapshots,
state hashes or the payload version.

Interrupt delivery is gated by the 68000 SR interrupt-priority mask, so a
differential window that must be interrupt-free uses a program that never writes
`SR` (the reset mask 7 suppresses VDP levels). There is no instruction-quantum flag:
canonical time is the 3420-master-cycle scanline, and the state file records
`tick`/`master_cycles` and the accumulated 68000 `cpu_cycles`.

## Debugging depth (M4, implemented and verified)

[ADR-0010](../adr/0010-debugging-depth.md) and the [M4 acceptance
plan](../quality/m4-debugging-depth-acceptance.md) implement a debug-only observation
layer for this slice: an optional bounded `DebugAccess` event ring (watchpoint hits, RAM
writes, VDP port and controller reads, interrupt request/delivery/acknowledgement), pure
CPU/VDP/interrupt/bus view types, a bounded 68000-subset disassembler whose instruction
lengths are cross-checked against execution, and the `chronofork debug` CLI surface.
Debug state is absent from the canonical payload, state hash and configuration hash; the
independent verifier's 1,700-step off/on probe found zero canonical differences, demo and
movie bytes stayed identical, and every M4-A1–A17 row is `executed - passed` (t5 PASS
2026-11-16). Stops are at instruction boundaries; no mid-instruction, cycle-exact or
reverse execution is claimed. Bus events are byte-granular, VDP events carry the device
width, instruction fetches appear only through a matching read watchpoint, and the logpoint
ring is fixed at 256 entries with a 4,096 compile-time bound.

## Next increments

1. Boot a curated commercial title to its main loop with video and audio, and record the
   compatibility matrix (phase 8.6). The cartridge device, header identity, Sega mapper,
   SRAM and TMSS landed in phase 8.1.
2. Guest bus-error and trace exception delivery (vectors 2 and 9) and the
   remaining exception-edge behaviour, driven by differential tests.
3. Broaden the differential corpus beyond the eleven synthetic ROMs while keeping
   instruction-address/cycle/state comparison distinct from hardware timing
   equivalence. A positive comparison of interrupt delivery or latency would need
   an oracle model of those semantics; the current masked-ROM pass is negative
   gating evidence only.
4. Additional interrupt sources (a wired level-7 source, timers, VDP
   interrupt-enable registers) and timer IRQ delivery to the Z80 (phase 8.5) if the slice
   grows to model them.
5. Deeper controller protocol (TH latch/TH-low phase, A/Start, 6-button pad, port 3) only
   if the slice grows to model it; the TH-high 3-button reads and `--rom` hardware TAS are
   implemented and verified under [ADR-0009](../adr/0009-real-hardware-input-tas.md).
6. Rendering/DMA only after the non-rendering state model is stable.
