//! Deterministic Sega Mega Drive VDP: device state, ports, DMA, FIFO, rendering
//! and snapshot.
//!
//! This module tree splits the device so each concern has one owner:
//!
//! - `mod.rs` (this file): the device, its port window, the eager DMA engine,
//!   typed errors and the versioned snapshot.
//! - `regs.rs`: port offsets, register/status positions, command-code bits and
//!   status latches.
//! - `timing.rs`: line/dot position, H/V counters and the frame index.
//! - `render.rs`: scanline composition and frame assembly.
//! - `dma.rs`: DMA modes and the latched transfer state the engine reports.
//! - `fifo.rs`: the bounded write FIFO and its occupancy/backpressure rules.
//!
//! **Implemented:**
//!
//! - 64 KiB of flat VRAM (`0x0000..=0xFFFF`), stored big-endian.
//! - The VDP port window at offsets `0x00..=0x0F` relative to `0xC00000`:
//!   `0x00..=0x03` is the data port (word accesses at `0x00`/`0x02`),
//!   `0x04..=0x07` is the control port (word accesses at `0x04`/`0x06`) and
//!   `0x08..=0x0F` is the HV counter, which returns the current approximate
//!   V/H position (see the timing module); HV-counter writes are rejected.
//! - Control-port two-word command setup. The first word latches command bits
//!   `1..=0` (bits `15..=14`) and address bits `13..=0`; the second word
//!   latches command bits `5..=2` (bits `7..=4`) and address bits `15..=14`
//!   (bits `1..=0`).
//! - Control-port register writes (`0x8000 | register << 8 | value`) for
//!   registers `0x00..=0x17`.
//! - Data-port word reads and writes with auto-increment from register `0x0F`.
//!   The command code selects VRAM (read/write), CRAM (read/write) or VSRAM
//!   (write); each transferred word advances the address once, and CRAM/VSRAM
//!   command addresses are byte offsets converted to word indices. Byte
//!   accesses transfer a full word: byte writes duplicate the byte into both
//!   halves (the documented VDP data-port byte-write behaviour), and byte reads
//!   return the requested byte lane of the transferred word.
//! - Space-addressed `write_word`/`read_word` primitives for VRAM, CRAM and
//!   VSRAM with the documented wrapping (CRAM aliases every 64 words, VSRAM
//!   wraps modulo its 40 words through a 9-bit address). CRAM and VSRAM
//!   data-port reads are both decoded: VSRAM read is the command code `0b0100`
//!   and VSRAM write `0b0101` (the boot fixtures' `0x4000`/`0x0010` pair).
//! - Master-cycle line timing through [`Vdp::advance_master`]: dot, line and
//!   frame counters, the approximate H/V counters and the HV-counter port.
//!   VBlank/HBlank status comes from the position; interlace and dot-exact
//!   counter values are not modelled and are not claimed.
//! - Scanline rendering through [`Vdp::render_scanline`] and frame assembly
//!   through [`Vdp::finish_frame`]: plane A/B nametables with cell tile,
//!   palette, h/v flips and priority; 4bpp packed tile fetch; the window from
//!   registers `0x11`/`0x12`; full/column/line scroll from `0x0B`/`0x0D` (the
//!   undocumented value 3 falls back to full-screen scroll); 80 link-ordered
//!   sprites with sizes, flips, X-duplication for sprites starting left of the
//!   screen and per-line count/pixel budgets; the documented
//!   sprite-priority > plane-A > plane-B priority table; shadow/highlight
//!   operator flags when register `0x0C` bit 3 is set; and the register-`0x07`
//!   background colour on the H40 side-column border. Frame pixels are palette
//!   indices carrying the shadow/highlight flags for the media layer, and
//!   H32/H40 plus the 224/240-line height follow register `0x0C` and the video
//!   standard.
//! - Canonical device state: 64 CRAM words, 40 VSRAM words, the bounded write
//!   FIFO, DMA state, the frame index, the line/dot position and the latched
//!   status conditions.
//! - Eager DMA with pre-validated source/destination ranges: VRAM fill and VRAM
//!   copy. The three 68000-memory modes (`memory->VRAM`/`CRAM`/`VSRAM`) are
//!   **rejected** with [`VdpError::DmaMemorySourceUnavailable`] before any read
//!   or write: this slice has no 68000-memory view for the VDP, so the transfer
//!   engine's source is VRAM itself and a "memory" transfer would silently
//!   produce plausible but wrong pixels. The register path
//!   ([`Vdp::start_dma_from_registers`]) uses the hardware layout
//!   (`0x13`/`0x14` length, `0x15`/`0x16`/`0x17` source and mode, with mode 2
//!   fill and mode 3 copy; modes 0/1 select a 68000-memory destination from the
//!   command code and land in the rejection), and is gated by register `0x01`
//!   bit 4. A transfer sets `DMA_BUSY` and records its slot cost for
//!   [`Vdp::take_dma_cycles`]; the 68000-side stalls are not modelled.
//! - The bounded four-word write FIFO: data-port writes fill it in order; a
//!   write while it is full adds one deterministic 68000 wait slot (never
//!   dropping or reordering a word), and the owner drains it after an advance.
//! - The status-register bits covered by [`STATUS_MASK`], all derived from
//!   device state: PAL from the video standard, HBlank from the dot position,
//!   VBlank and the sprite/odd-frame latches from the latches, DMA-busy from
//!   the DMA state, VINT from the owning machine's pending request and FIFO
//!   empty/full from the write queue.
//! - Versioned, fixed-length snapshots through [`Vdp::snapshot_bytes`] and
//!   [`Vdp::restore_bytes`]. Format v2 (`CFVDP002`) covers the complete device
//!   state, including the PAL flag and the line/dot position (validated
//!   against the stored standard); `CFVDP001`/v1 payloads are rejected, never
//!   reinterpreted.
//!
//! # VINT-pending is derived state
//!
//! The VINT-pending status bit is a latch that the owning machine drives from
//! its canonical interrupt state. The minimal Mega Drive backend sets it while
//! its scheduler holds a pending VBlank (68000 level 6) request — including
//! while that request is masked by the `SR` interrupt-priority mask — and clears
//! it only when that request is delivered and acknowledged. It is deliberately
//! **not** serialized into the VDP snapshot: the scheduler's pending request is
//! the single source of truth, it is already snapshotted and hashed by the
//! backend, and duplicating it here would create two values that could disagree
//! after a restore. The backend recomputes the latch on restore before anything
//! inspects it. A standalone [`Vdp`] user must drive it explicitly.
//!
//! A control-port (status) read returns the status word and cancels only a
//! half-latched command word; it never acknowledges, clears or otherwise
//! consumes an interrupt request.
//!
//! VDP interrupt enable gating (register `0x00` bits 5/4), the owned
//! pending/acknowledge surface and the derived VINT latches are implemented; the
//! backend uses them for 68000 level-6/level-4 delivery.
//!
//! **Not implemented (and not claimed):** the 68000-memory DMA modes (rejected
//! with a typed error, see above; the VDP has no 68000-memory view in this
//! slice), VSRAM data-port reads, Z80/PSG interaction, audio, interlace, the
//! byte-swapped/interleaved physical VRAM layout, dot-exact H/V counters, exact
//! H32 window geometry, exact line-mode vertical scrolling (see the `render`
//! module notes), 68000-side DMA stalls and cycle-accurate port timing.
//!
//! **Deterministic reset state:** VRAM, CRAM, VSRAM, the FIFO, DMA state,
//! status latches, the frame index and the line/dot position are zero, all 24
//! registers are zero and no command is latched. In particular register `0x0F`
//! (auto-increment) starts at zero, so a program must write it before relying
//! on address advancement.

mod dma;
mod fifo;
mod regs;
mod render;
mod timing;

use std::error::Error;
use std::fmt;

use chronofork_core::codec::Writer;

use crate::cycles::{MASTER_CYCLES_PER_LINE, lines_per_frame};
use crate::interrupts::VideoStandard;

use fifo::Fifo;
use regs::{
    CODE_CRAM_READ, CODE_CRAM_WRITE, CODE_DMA, CODE_READ_INHIBIT, CODE_REGISTER_WRITE,
    CODE_VRAM_READ, CODE_VRAM_WRITE, CODE_VSRAM_READ, CODE_VSRAM_WRITE,
};
use timing::HBLANK_START_MASTER_CYCLES;

pub use dma::{DmaMode, DmaState};
pub use fifo::FIFO_CAPACITY;
pub use regs::{
    AUTO_INCREMENT_REGISTER, CONTROL_PORT_OFFSET, CRAM_WORDS, DATA_PORT_OFFSET, HV_COUNTER_OFFSET,
    PORT_WINDOW_SIZE, REGISTER_COUNT, STATUS_DMA_BUSY, STATUS_FIFO_EMPTY, STATUS_FIFO_FULL,
    STATUS_HBLANK, STATUS_MASK, STATUS_ODD_FRAME, STATUS_PAL, STATUS_SPRITE_COLLISION,
    STATUS_SPRITE_OVERFLOW, STATUS_VBLANK, STATUS_VINT_PENDING, StatusLatches, VRAM_SIZE,
    VSRAM_WORDS, VramTarget,
};
pub use render::{
    FrameBuffer, HIGHLIGHT_FLAG, MAX_FRAME_HEIGHT, MAX_FRAME_WIDTH, PALETTE_INDEX_MASK, SHADOW_FLAG,
};
pub use timing::VdpTick;

/// Magic prefix of a serialized VDP snapshot.
pub const SNAPSHOT_MAGIC: [u8; 8] = *b"CFVDP002";
/// Version of the serialized VDP snapshot format.
pub const SNAPSHOT_VERSION: u16 = 2;

// Snapshot layout, in bytes. All multi-byte integers are little-endian:
//
//   0..8    magic "CFVDP002"
//   8..10   format version (2)
//   10      flags: vblank, half-latched command, odd frame, collision,
//           overflow, PAL
//   11      command code (six bits)
//   12..14  address
//   14..38  registers (24)
//   38..166 CRAM (64 words)
//   166..246 VSRAM (40 words)
//   246     FIFO occupancy (0..=4)
//   247..255 FIFO words oldest first (4 words, zero-padded)
//   255     DMA active (0 or 1)
//   256..260 DMA source
//   260..262 DMA length
//   262     DMA mode code
//   263..271 frame index
//   271..273 line
//   273..275 dot
//   275..   VRAM (64 KiB)
const SNAPSHOT_FLAGS_OFFSET: usize = SNAPSHOT_MAGIC.len() + 2;
const SNAPSHOT_CODE_OFFSET: usize = SNAPSHOT_FLAGS_OFFSET + 1;
const SNAPSHOT_ADDRESS_OFFSET: usize = SNAPSHOT_CODE_OFFSET + 1;
const SNAPSHOT_REGISTERS_OFFSET: usize = SNAPSHOT_ADDRESS_OFFSET + 2;
const SNAPSHOT_CRAM_OFFSET: usize = SNAPSHOT_REGISTERS_OFFSET + REGISTER_COUNT;
const SNAPSHOT_VSRAM_OFFSET: usize = SNAPSHOT_CRAM_OFFSET + CRAM_WORDS * 2;
const SNAPSHOT_FIFO_COUNT_OFFSET: usize = SNAPSHOT_VSRAM_OFFSET + VSRAM_WORDS * 2;
const SNAPSHOT_FIFO_WORDS_OFFSET: usize = SNAPSHOT_FIFO_COUNT_OFFSET + 1;
const SNAPSHOT_DMA_ACTIVE_OFFSET: usize = SNAPSHOT_FIFO_WORDS_OFFSET + FIFO_CAPACITY * 2;
const SNAPSHOT_DMA_SOURCE_OFFSET: usize = SNAPSHOT_DMA_ACTIVE_OFFSET + 1;
const SNAPSHOT_DMA_LENGTH_OFFSET: usize = SNAPSHOT_DMA_SOURCE_OFFSET + 4;
const SNAPSHOT_DMA_MODE_OFFSET: usize = SNAPSHOT_DMA_LENGTH_OFFSET + 2;
const SNAPSHOT_FRAME_INDEX_OFFSET: usize = SNAPSHOT_DMA_MODE_OFFSET + 1;
const SNAPSHOT_LINE_OFFSET: usize = SNAPSHOT_FRAME_INDEX_OFFSET + 8;
const SNAPSHOT_DOT_OFFSET: usize = SNAPSHOT_LINE_OFFSET + 2;
const SNAPSHOT_VRAM_OFFSET: usize = SNAPSHOT_DOT_OFFSET + 2;
/// Exact byte length of a serialized VDP snapshot.
pub const SNAPSHOT_LEN: usize = SNAPSHOT_VRAM_OFFSET + VRAM_SIZE;

/// Master-cycle cost of one word in the eager DMA model.
pub const DMA_MASTER_CYCLES_PER_WORD: u32 = 2;

const SNAPSHOT_FLAG_VBLANK: u8 = 1 << 0;
const SNAPSHOT_FLAG_COMMAND_PENDING: u8 = 1 << 1;
const SNAPSHOT_FLAG_ODD_FRAME: u8 = 1 << 2;
const SNAPSHOT_FLAG_SPRITE_COLLISION: u8 = 1 << 3;
const SNAPSHOT_FLAG_SPRITE_OVERFLOW: u8 = 1 << 4;
/// PAL video standard (the device was last advanced under PAL). Flag bit 5 was
/// reserved before timing existed; no earlier valid v2 payload used it, so
/// accepting it now reinterprets no accepted state.
const SNAPSHOT_FLAG_PAL: u8 = 1 << 5;
const SNAPSHOT_FLAG_MASK: u8 = SNAPSHOT_FLAG_VBLANK
    | SNAPSHOT_FLAG_COMMAND_PENDING
    | SNAPSHOT_FLAG_ODD_FRAME
    | SNAPSHOT_FLAG_SPRITE_COLLISION
    | SNAPSHOT_FLAG_SPRITE_OVERFLOW
    | SNAPSHOT_FLAG_PAL;

/// A VDP port access or snapshot that this slice cannot satisfy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VdpError {
    /// A port offset outside `0x00..=0x0F`.
    UnsupportedOffset {
        /// Requested port offset.
        offset: u8,
    },
    /// A word access at an odd port offset.
    OddOffset {
        /// Requested port offset.
        offset: u8,
    },
    /// A write to the read-only HV-counter port.
    UnsupportedWrite {
        /// Requested port offset.
        offset: u8,
    },
    /// A VDP register number outside `0x00..=0x17`.
    UnsupportedRegister {
        /// Requested register number.
        register: u8,
    },
    /// A data-port access whose command code does not select implemented VRAM
    /// read or write behaviour (including CRAM/VSRAM and half-completed
    /// commands).
    UnsupportedTarget {
        /// Current six-bit command code.
        code: u8,
    },
    /// A DMA command whose destination space, mode or copy flag cannot be
    /// decoded (for example a DX command with no data target, or a copy
    /// without command bit CD4).
    DmaUnsupported {
        /// Current six-bit command code.
        code: u8,
    },
    /// A snapshot whose length is not [`SNAPSHOT_LEN`].
    ///
    /// A `CFVDP001` v1 payload always lands here: its fixed length differs, so
    /// it is rejected before any field is interpreted.
    SnapshotLengthMismatch {
        /// Required length.
        expected: usize,
        /// Supplied length.
        found: usize,
    },
    /// A snapshot without the expected magic prefix.
    SnapshotMagicMismatch,
    /// A snapshot with an unsupported format version.
    SnapshotVersionUnsupported {
        /// Version found in the snapshot header.
        found: u16,
    },
    /// A snapshot whose flag byte has reserved bits set.
    SnapshotFlagsInvalid {
        /// Flag byte found in the snapshot header.
        flags: u8,
    },
    /// A snapshot whose command code exceeds six bits.
    SnapshotCodeInvalid {
        /// Command code found in the snapshot header.
        code: u8,
    },
    /// A snapshot whose FIFO occupancy exceeds [`FIFO_CAPACITY`].
    SnapshotFifoCountInvalid {
        /// Occupancy found in the snapshot.
        count: u8,
    },
    /// A snapshot whose DMA mode code is not a defined mode.
    SnapshotDmaModeInvalid {
        /// Mode code found in the snapshot.
        mode: u8,
    },
    /// A snapshot whose boolean field is neither 0 nor 1.
    SnapshotBooleanInvalid {
        /// Name of the offending field.
        field: &'static str,
        /// Value found in the snapshot.
        value: u8,
    },
    /// A snapshot line outside the frame of its video standard.
    SnapshotLineOutOfRange {
        /// Line found in the snapshot.
        line: u16,
        /// Last valid line (exclusive) for the stored standard.
        lines_per_frame: u16,
    },
    /// A snapshot dot outside one master-cycle line.
    SnapshotDotOutOfRange {
        /// Dot found in the snapshot.
        dot: u16,
        /// Master cycles in one line.
        master_cycles_per_line: u64,
    },
    /// A DMA source range outside the modelled 64 KiB VRAM view.
    DmaSourceOutOfRange {
        /// Requested source byte address.
        source: u32,
        /// Number of words requested.
        words: u32,
    },
    /// A DMA destination range outside its memory space.
    DmaDestinationOutOfRange {
        /// Destination memory space.
        target: VramTarget,
        /// First destination word index.
        word_index: u32,
        /// Number of words requested.
        words: u32,
        /// Number of words the destination can hold.
        capacity: u32,
    },
    /// A DMA transfer whose source is 68000 memory.
    ///
    /// This slice has no 68000-memory view for the VDP: the transfer engine
    /// reads its source from VRAM, so a `memory->VRAM`/`CRAM`/`VSRAM` transfer
    /// would silently read VRAM instead of the programmed memory address and
    /// produce plausible but wrong pixels. The mode is rejected with a typed
    /// error before any byte is read or written.
    DmaMemorySourceUnavailable {
        /// Rejected transfer mode.
        mode: DmaMode,
        /// Number of words the rejected transfer would have moved.
        words: u32,
    },
    /// A DMA source and destination overlap inside VRAM.
    DmaSourceDestinationOverlap {
        /// Source byte address.
        source: u32,
        /// Destination byte address.
        destination: u32,
        /// Number of words requested.
        words: u32,
    },
}

impl fmt::Display for VdpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedOffset { offset } => write!(
                f,
                "VDP access at unsupported offset 0x{offset:02X} (window is 0x00..=0x0F)"
            ),
            Self::OddOffset { offset } => {
                write!(f, "odd VDP word access at offset 0x{offset:02X}")
            }
            Self::UnsupportedWrite { offset } => {
                write!(f, "write to read-only VDP HV-counter offset 0x{offset:02X}")
            }
            Self::UnsupportedRegister { register } => write!(
                f,
                "VDP register 0x{register:02X} is outside the implemented 0x00..=0x17 range"
            ),
            Self::UnsupportedTarget { code } => write!(
                f,
                "VDP data-port access with unsupported command code 0x{code:02X}"
            ),
            Self::DmaUnsupported { code } => write!(
                f,
                "VDP data-port access with DMA command code 0x{code:02X} has no decodable transfer"
            ),
            Self::SnapshotLengthMismatch { expected, found } => {
                write!(f, "VDP snapshot is {found} bytes, expected {expected}")
            }
            Self::SnapshotMagicMismatch => f.write_str("VDP snapshot magic mismatch"),
            Self::SnapshotVersionUnsupported { found } => {
                write!(f, "VDP snapshot version {found} is not supported")
            }
            Self::SnapshotFlagsInvalid { flags } => {
                write!(
                    f,
                    "VDP snapshot flag byte 0x{flags:02X} has reserved bits set"
                )
            }
            Self::SnapshotCodeInvalid { code } => {
                write!(f, "VDP snapshot command code 0x{code:02X} exceeds six bits")
            }
            Self::SnapshotFifoCountInvalid { count } => write!(
                f,
                "VDP snapshot FIFO occupancy {count} exceeds the {FIFO_CAPACITY}-word queue"
            ),
            Self::SnapshotDmaModeInvalid { mode } => {
                write!(f, "VDP snapshot DMA mode code 0x{mode:02X} is not defined")
            }
            Self::SnapshotBooleanInvalid { field, value } => {
                write!(f, "VDP snapshot {field} flag {value} is not 0 or 1")
            }
            Self::SnapshotLineOutOfRange {
                line,
                lines_per_frame,
            } => write!(
                f,
                "VDP snapshot line {line} is outside the {lines_per_frame}-line frame"
            ),
            Self::SnapshotDotOutOfRange {
                dot,
                master_cycles_per_line,
            } => write!(
                f,
                "VDP snapshot dot {dot} is outside the {master_cycles_per_line}-cycle line"
            ),
            Self::DmaSourceOutOfRange { source, words } => write!(
                f,
                "VDP DMA source 0x{source:05X} for {words} words is outside VRAM"
            ),
            Self::DmaDestinationOutOfRange {
                target,
                word_index,
                words,
                capacity,
            } => write!(
                f,
                "VDP DMA destination {target:?} from word {word_index} for {words} words \
                 exceeds the {capacity}-word space"
            ),
            Self::DmaMemorySourceUnavailable { mode, words } => write!(
                f,
                "VDP DMA mode {mode:?} for {words} words needs a 68000-memory source, which \
                 this slice does not model"
            ),
            Self::DmaSourceDestinationOverlap {
                source,
                destination,
                words,
            } => write!(
                f,
                "VDP DMA source 0x{source:05X} and destination 0x{destination:05X} overlap \
                 over {words} words"
            ),
        }
    }
}

impl Error for VdpError {}

/// A VDP-generated interrupt source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VdpInterruptSource {
    /// Vertical blanking (68000 level 6).
    VBlank,
    /// Horizontal blanking (68000 level 4).
    HBlank,
}

/// The VDP's pending interrupt requests.
///
/// The requests are owned by the VDP and gated by register `0x00` bits 5 (VBlank)
/// and 4 (HBlank). They are derived from the master-cycle timeline, so they are
/// not serialized; the owning backend re-derives them from the scheduler's
/// retained requests after a restore.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VdpInterrupts {
    /// Vertical-blanking request is pending.
    pub vblank_pending: bool,
    /// Horizontal-blanking request is pending.
    pub hblank_pending: bool,
}

impl VdpInterrupts {
    /// Returns whether `source` is pending.
    #[must_use]
    pub const fn pending(self, source: VdpInterruptSource) -> bool {
        match source {
            VdpInterruptSource::VBlank => self.vblank_pending,
            VdpInterruptSource::HBlank => self.hblank_pending,
        }
    }
}

/// Partial 315-5313 VDP: port window, status register, device state and
/// snapshot.
///
/// Accesses are made through port offsets relative to `0xC00000`, matching the
/// big-endian 16-bit bus of the Mega Drive. All state changes are confined to
/// the methods on this type; there is no hidden global state and no host clock
/// input.
#[derive(Clone)]
pub struct Vdp {
    vram: Vec<u8>,
    registers: [u8; REGISTER_COUNT],
    cram: [u16; CRAM_WORDS],
    vsram: [u16; VSRAM_WORDS],
    fifo: Fifo,
    dma: DmaState,
    code: u8,
    address: u16,
    command_pending: bool,
    frame_index: u64,
    line: u16,
    dot: u16,
    /// Video standard the timing last advanced under; drives the PAL status
    /// bit. Snapshotted through the flag byte.
    video: VideoStandard,
    status_latches: StatusLatches,
    /// Derived VINT-pending latch driven by the owning machine's interrupt
    /// source. Deliberately absent from the serialized snapshot.
    vint_pending: bool,
    /// VBlank request pending, gated by register `0x00` bit 5. Derived state,
    /// not serialized.
    vblank_pending: bool,
    /// HBlank request pending, gated by register `0x00` bit 4. Derived state,
    /// not serialized.
    hblank_pending: bool,
    /// Whether the most recently rendered line reached the per-line sprite
    /// count or pixel budget. Derived render observation, not serialized.
    sprite_limit_reached: bool,
    /// Slot cost of the most recent eager DMA transfer, in master cycles.
    /// Derived observation consumed by the owner, not serialized.
    dma_cycles: u32,
    /// Data-port accesses whose command code is not modelled, discarded exactly
    /// like the pinned oracle's `default:` cases instead of failing the host.
    ///
    /// Diagnostic only, like [`Vdp::dma_cycles`]: it is deliberately **not**
    /// serialized by [`Vdp::snapshot_bytes`], so two machines that discarded a
    /// different number of accesses still serialise byte-for-byte equal.
    discarded_data_port_accesses: u64,
    /// Accumulated 68000-cycle wait slots for data-port writes that found the
    /// FIFO full. Derived observation consumed by the owner, not serialized.
    fifo_wait_cycles: u32,
}

impl Vdp {
    /// Creates a VDP in the deterministic reset state described by the module
    /// documentation: zeroed memory, registers and queues, and no pending
    /// command or status condition.
    #[must_use]
    pub fn new() -> Self {
        Self {
            vram: vec![0; VRAM_SIZE],
            registers: [0; REGISTER_COUNT],
            cram: [0; CRAM_WORDS],
            vsram: [0; VSRAM_WORDS],
            fifo: Fifo::empty(),
            dma: DmaState::default(),
            code: 0,
            address: 0,
            command_pending: false,
            frame_index: 0,
            line: 0,
            dot: 0,
            video: VideoStandard::Ntsc,
            status_latches: StatusLatches::default(),
            vint_pending: false,
            vblank_pending: false,
            hblank_pending: false,
            sprite_limit_reached: false,
            dma_cycles: 0,
            discarded_data_port_accesses: 0,
            fifo_wait_cycles: 0,
        }
    }

    /// Returns the ten-bit status register.
    ///
    /// Every bit comes from device state instead of a constant: PAL from the
    /// video standard, HBlank from the dot position, VBlank and the sprite
    /// latches from the status latches, DMA-busy from the DMA state, VINT from
    /// the owning machine's pending request and FIFO empty/full from the write
    /// queue. The upper bits are zero rather than the floating values real
    /// hardware returns. Reading the status through a control-port access does
    /// not acknowledge the VINT-pending latch; only
    /// [`Vdp::set_vint_pending`] changes it.
    #[must_use]
    pub fn status(&self) -> u16 {
        let mut status = 0;
        if self.video == VideoStandard::Pal {
            status |= STATUS_PAL;
        }
        if self.fifo.is_empty() {
            status |= STATUS_FIFO_EMPTY;
        }
        if self.fifo.is_full() {
            status |= STATUS_FIFO_FULL;
        }
        if self.dma.active() {
            status |= STATUS_DMA_BUSY;
        }
        if self.dot >= HBLANK_START_MASTER_CYCLES {
            status |= STATUS_HBLANK;
        }
        if self.status_latches.vblank {
            status |= STATUS_VBLANK;
        }
        if self.status_latches.odd_frame {
            status |= STATUS_ODD_FRAME;
        }
        if self.status_latches.sprite_collision {
            status |= STATUS_SPRITE_COLLISION;
        }
        if self.status_latches.sprite_overflow {
            status |= STATUS_SPRITE_OVERFLOW;
        }
        if self.vint_pending {
            status |= STATUS_VINT_PENDING;
        }
        status
    }

    /// Sets or clears the video VBlank status latch (bit 3).
    ///
    /// [`Vdp::advance_master`] derives this latch from the VDP's own line/dot
    /// position; the minimal Mega Drive backend also re-syncs it to the
    /// scheduler's deterministic scanline position after each step so the two
    /// canonical timelines agree. It reflects the vertical blanking interval,
    /// not a pending CPU request.
    pub fn set_vblank(&mut self, active: bool) {
        self.status_latches.vblank = active;
    }

    /// Sets or clears the derived VBlank-interrupt-pending latch (status bit 7).
    ///
    /// The minimal Mega Drive backend drives this latch from the scheduler's
    /// pending VBlank (level 6) request: set while the request is pending — even
    /// while the 68000 `SR` mask blocks delivery — and cleared when the request
    /// is delivered and acknowledged. The latch is **not** part of
    /// [`Vdp::snapshot_bytes`]; see the module documentation for why it is
    /// derived. A standalone `Vdp` user must call this explicitly.
    pub fn set_vint_pending(&mut self, pending: bool) {
        self.vint_pending = pending;
    }

    /// Returns the derived VINT-pending latch (status bit 7).
    ///
    /// This is the same value [`Vdp::status`] encodes as
    /// [`STATUS_VINT_PENDING`], exposed separately for tests and diagnostics.
    #[must_use]
    pub const fn vint_pending(&self) -> bool {
        self.vint_pending
    }

    /// Returns whether register `0x00` enables `source`.
    ///
    /// Bit 5 is the VBlank enable and bit 4 the HBlank enable, matching the
    /// phase 8.3 design's modelled register layout.
    #[must_use]
    pub fn interrupt_enabled(&self, source: VdpInterruptSource) -> bool {
        let bit = match source {
            VdpInterruptSource::VBlank => 0x20,
            VdpInterruptSource::HBlank => 0x10,
        };
        self.registers[0] & bit != 0
    }

    /// Requests `source`, gated by register `0x00`.
    ///
    /// A disabled source is dropped: it does not set a pending request. A
    /// request that is already pending is coalesced (the latch stays set).
    pub fn request_interrupt(&mut self, source: VdpInterruptSource) {
        if !self.interrupt_enabled(source) {
            return;
        }
        match source {
            VdpInterruptSource::VBlank => self.vblank_pending = true,
            VdpInterruptSource::HBlank => self.hblank_pending = true,
        }
    }

    /// Returns the pending interrupt requests without consuming them.
    ///
    /// A control-port status read reports these requests but never acknowledges
    /// them: only [`Vdp::acknowledge`] clears a request, so a program observing
    /// the status register cannot lose an interrupt by polling it.
    #[must_use]
    pub const fn take_interrupts(&self) -> VdpInterrupts {
        VdpInterrupts {
            vblank_pending: self.vblank_pending,
            hblank_pending: self.hblank_pending,
        }
    }

    /// Acknowledges one delivered `source`, leaving the other source pending.
    pub fn acknowledge(&mut self, source: VdpInterruptSource) {
        match source {
            VdpInterruptSource::VBlank => self.vblank_pending = false,
            VdpInterruptSource::HBlank => self.hblank_pending = false,
        }
    }

    /// Clears both pending requests.
    ///
    /// The owning backend calls this before re-deriving requests after a
    /// restore, so no stale request survives the snapshot replacement.
    pub fn clear_interrupts(&mut self) {
        self.vblank_pending = false;
        self.hblank_pending = false;
    }

    /// Returns the flat 64 KiB VRAM contents.
    #[must_use]
    pub fn vram(&self) -> &[u8] {
        &self.vram
    }

    /// Returns the 64 CRAM colour words.
    #[must_use]
    pub const fn cram(&self) -> &[u16; CRAM_WORDS] {
        &self.cram
    }

    /// Returns the 40 VSRAM vertical-scroll words.
    #[must_use]
    pub const fn vsram(&self) -> &[u16; VSRAM_WORDS] {
        &self.vsram
    }

    /// Returns one CRAM colour word, or `None` when `index` is not a CRAM
    /// entry.
    ///
    /// This is a direct index into the 64 stored words; the data-port address
    /// decoder wraps addresses instead (see [`Vdp::write_word`]).
    #[must_use]
    pub fn cram_entry(&self, index: usize) -> Option<u16> {
        self.cram.get(index).copied()
    }

    /// Returns one VSRAM word, or `None` when `index` is not a VSRAM entry.
    ///
    /// This is a direct index into the 40 stored words; the data-port address
    /// decoder wraps addresses instead (see [`Vdp::write_word`]).
    #[must_use]
    pub fn vsram_word(&self, index: usize) -> Option<u16> {
        self.vsram.get(index).copied()
    }

    /// Returns the number of words queued in the write FIFO.
    #[must_use]
    pub const fn fifo_len(&self) -> usize {
        self.fifo.len()
    }

    /// Returns the current DMA state.
    #[must_use]
    pub const fn dma(&self) -> &DmaState {
        &self.dma
    }

    /// Returns the latched status conditions.
    #[must_use]
    pub const fn status_latches(&self) -> StatusLatches {
        self.status_latches
    }

    /// Returns one VDP register, or `None` when `index` is outside
    /// `0x00..=0x17`.
    #[must_use]
    pub fn register(&self, index: usize) -> Option<u8> {
        self.registers.get(index).copied()
    }

    /// Returns all 24 implemented VDP registers.
    #[must_use]
    pub const fn registers(&self) -> &[u8; REGISTER_COUNT] {
        &self.registers
    }

    /// Returns the current 16-bit command/address register.
    #[must_use]
    pub const fn address(&self) -> u16 {
        self.address
    }

    /// Writes one word into `target` at `address`.
    ///
    /// This is the space-addressed primitive behind the data port: it does not
    /// read or advance the command/address register, and it never applies
    /// auto-increment, so a port access transfers exactly one word. Address
    /// decoding follows the documented memory layouts:
    ///
    /// - VRAM (and the DMA copy view of it) keeps a 16-bit address; word
    ///   accesses align down to an even byte address.
    /// - CRAM exposes 64 words through a 7-bit address whose bit 6 is not
    ///   decoded, so addresses `64..=127` alias entries `0..=63`.
    /// - VSRAM exposes 40 words through a 9-bit address; the address wraps
    ///   modulo 40 (`(address & 0x01FF) % 40`).
    ///
    /// `address` is a word index for CRAM and VSRAM; the data port converts its
    /// byte-oriented command address with `address >> 1` before calling this
    /// primitive, so an auto-increment of two moves to the next entry. Values
    /// are stored verbatim: this slice does not yet mask CRAM to its nine-bit
    /// colour fields.
    pub fn write_word(
        &mut self,
        target: VramTarget,
        address: u16,
        value: u16,
    ) -> Result<(), VdpError> {
        match target {
            VramTarget::Vram | VramTarget::VramCopy => {
                let index = usize::from(address & 0xFFFE);
                let bytes = value.to_be_bytes();
                self.vram[index] = bytes[0];
                self.vram[index + 1] = bytes[1];
            }
            VramTarget::Cram => {
                let index = usize::from(address) & (CRAM_WORDS - 1);
                self.cram[index] = value;
            }
            VramTarget::Vsram => {
                let index = (usize::from(address) & 0x01FF) % VSRAM_WORDS;
                self.vsram[index] = value;
            }
        }
        Ok(())
    }

    /// Reads one word from `target` at `address`.
    ///
    /// This is the space-addressed primitive behind the data port and uses the
    /// same address decoding as [`Vdp::write_word`]; `address` is a word index
    /// for CRAM and VSRAM. It does not read or advance the command/address
    /// register. VRAM words are big-endian.
    pub fn read_word(&mut self, target: VramTarget, address: u16) -> Result<u16, VdpError> {
        let word = match target {
            VramTarget::Vram | VramTarget::VramCopy => {
                let index = usize::from(address & 0xFFFE);
                u16::from_be_bytes([self.vram[index], self.vram[index + 1]])
            }
            VramTarget::Cram => self.cram[usize::from(address) & (CRAM_WORDS - 1)],
            VramTarget::Vsram => self.vsram[(usize::from(address) & 0x01FF) % VSRAM_WORDS],
        };
        Ok(word)
    }

    /// Starts one eager DMA transfer into the command address.
    ///
    /// The whole source and destination range is validated before any byte is
    /// written, so a rejected transfer leaves VRAM, CRAM, VSRAM and the DMA
    /// state untouched. This slice has no view of 68000 memory, so the `source`
    /// operand names a VRAM byte address for the memory and copy modes; fill
    /// uses its low byte as the constant fill value. The transfer completes
    /// eagerly, sets the DMA-busy state and records its slot cost for
    /// [`Vdp::take_dma_cycles`].
    ///
    /// Word counts are the literal transfer length (this slice does not model
    /// the hardware's length-minus-one encoding).
    pub fn start_dma(&mut self, source: u32, length: u16, mode: DmaMode) -> Result<(), VdpError> {
        self.start_dma_words(source, u32::from(length), mode, length)
    }

    /// Performs a DMA transfer of an explicit word count.
    ///
    /// `encoded_length` is what the canonical [`DmaState`] records: the word
    /// count for the direct entry point, or the hardware length-minus-one
    /// register value on the register path. The count is a `u32` here because
    /// the full-VRAM clear is 65 536 words, one past a `u16`.
    fn start_dma_words(
        &mut self,
        source: u32,
        words: u32,
        mode: DmaMode,
        encoded_length: u16,
    ) -> Result<(), VdpError> {
        // The three 68000-memory modes are rejected before any range check,
        // memory read or mutation: `self.read_vram_word(read)` would otherwise
        // read VRAM where the program meant 68000 memory, and every destination
        // of a half-completed transfer would be committed on the way. Only the
        // VRAM-source modes (fill and copy) are modelled.
        if matches!(
            mode,
            DmaMode::MemoryToVram | DmaMode::MemoryToCram | DmaMode::MemoryToVsram
        ) {
            return Err(VdpError::DmaMemorySourceUnavailable { mode, words });
        }
        let bytes = words * 2;
        if mode != DmaMode::Fill {
            let end = source
                .checked_add(bytes)
                .ok_or(VdpError::DmaSourceOutOfRange { source, words })?;
            if end > VRAM_SIZE as u32 {
                return Err(VdpError::DmaSourceOutOfRange { source, words });
            }
        }

        let (target, first_index) = match mode {
            DmaMode::MemoryToVram | DmaMode::Fill | DmaMode::Copy => {
                let index = u32::from(self.address & 0xFFFE);
                // A fill walks the 64 KiB VRAM sequentially and wraps at the end,
                // which is how the boot fixtures clear VRAM with one 65 536-word
                // fill from address 0. The copy forms keep the pre-validated
                // range because their source and destination share the same space
                // and a wrapping copy would need the aliasing rules this slice
                // does not model.
                if mode != DmaMode::Fill && index + bytes > VRAM_SIZE as u32 {
                    return Err(VdpError::DmaDestinationOutOfRange {
                        target: VramTarget::Vram,
                        word_index: index / 2,
                        words,
                        capacity: (VRAM_SIZE / 2) as u32,
                    });
                }
                if mode != DmaMode::Fill && source < index + bytes && index < source + bytes {
                    return Err(VdpError::DmaSourceDestinationOverlap {
                        source,
                        destination: index,
                        words,
                    });
                }
                (VramTarget::Vram, index)
            }
            DmaMode::MemoryToCram => {
                let index = (u32::from(self.address) >> 1) & (CRAM_WORDS as u32 - 1);
                if index + words > CRAM_WORDS as u32 {
                    return Err(VdpError::DmaDestinationOutOfRange {
                        target: VramTarget::Cram,
                        word_index: index,
                        words,
                        capacity: CRAM_WORDS as u32,
                    });
                }
                (VramTarget::Cram, index)
            }
            DmaMode::MemoryToVsram => {
                let index = (u32::from(self.address) >> 1) & 0x01FF;
                if index + words > VSRAM_WORDS as u32 {
                    return Err(VdpError::DmaDestinationOutOfRange {
                        target: VramTarget::Vsram,
                        word_index: index,
                        words,
                        capacity: VSRAM_WORDS as u32,
                    });
                }
                (VramTarget::Vsram, index)
            }
        };

        let fill_byte = (source & 0xFF) as u8;
        let fill_word = u16::from_be_bytes([fill_byte, fill_byte]);
        for offset in 0..words {
            let word = if mode == DmaMode::Fill {
                fill_word
            } else {
                let read = u16::try_from(source + offset * 2).unwrap_or(0);
                self.read_vram_word(read)
            };
            let index = offset;
            let address = match target {
                VramTarget::Vram => {
                    let byte_address = (first_index + index * 2) % VRAM_SIZE as u32;
                    u16::try_from(byte_address).unwrap_or(0)
                }
                VramTarget::Cram | VramTarget::Vsram => {
                    u16::try_from(first_index + index).unwrap_or(0)
                }
                VramTarget::VramCopy => 0,
            };
            self.write_word(target, address, word)?;
        }

        self.dma = DmaState::from_parts(source, encoded_length, mode, true);
        self.dma_cycles = words * DMA_MASTER_CYCLES_PER_WORD;
        Ok(())
    }

    /// Runs the DMA transfer programmed through registers `0x13..=0x17`.
    ///
    /// The hardware register layout is used: `0x13`/`0x14` are the little-endian
    /// length in words, `0x15`/`0x16`/`0x17` are the source address low/mid/high
    /// byte, and bits `7..=6` of `0x17` select the mode:
    ///
    /// - `0b00`/`0b01`: 68000 memory to the space selected by the latched command
    ///   code (`0x01` VRAM, `0x03` CRAM, `0x04` VSRAM);
    /// - `0b10`: VRAM fill, using the source low byte as the fill value;
    /// - `0b11`: VRAM copy, which additionally requires command bit CD4.
    ///
    /// Register `0x01` bit 4 gates DMA: while it is clear, a data-port access with
    /// a DMA command performs no transfer and mutates nothing. Registers
    /// `0x13`/`0x14` use the documented **length-minus-one** encoding, so the
    /// transfer moves `encoded + 1` words: `0xFFFF` means 65 536 words, the whole
    /// 64 KiB of VRAM, exactly the clear-VRAM fill the boot fixtures program. (The
    /// direct [`Vdp::start_dma`] entry point keeps its literal word count, which is
    /// a test-facing API rather than a hardware register.) A data-port access with
    /// the DMA command code triggers the transfer, as on hardware.
    pub fn start_dma_from_registers(&mut self) -> Result<(), VdpError> {
        if self.registers[0x01] & 0x10 == 0 {
            return Ok(());
        }
        let encoded = u16::from(self.registers[0x13]) | (u16::from(self.registers[0x14]) << 8);
        // Hardware length-minus-one: 0xFFFF encodes the full 65 536-word VRAM
        // clear, decoded in `u32` below. The direct `start_dma` entry point takes
        // a literal word count instead.
        let source = u32::from(self.registers[0x15])
            | (u32::from(self.registers[0x16]) << 8)
            | (u32::from(self.registers[0x17] & 0x3F) << 16);
        let mode = match (self.registers[0x17] >> 6) & 0x03 {
            0 | 1 => match self.code & 0x0F {
                CODE_VRAM_WRITE => DmaMode::MemoryToVram,
                CODE_CRAM_WRITE => DmaMode::MemoryToCram,
                CODE_VSRAM_WRITE => DmaMode::MemoryToVsram,
                _ => return Err(VdpError::DmaUnsupported { code: self.code }),
            },
            2 => DmaMode::Fill,
            _ => {
                if self.code & CODE_READ_INHIBIT == 0 {
                    return Err(VdpError::DmaUnsupported { code: self.code });
                }
                DmaMode::Copy
            }
        };
        // `encoded` is the hardware length-minus-one value; the transfer moves
        // `encoded + 1` words, computed in `u32` so 0xFFFF is 65 536 and not a
        // wrapped zero.
        self.start_dma_words(source, u32::from(encoded) + 1, mode, encoded)
    }

    /// Returns whether the eager DMA transfer's busy window is still open.
    #[must_use]
    pub const fn dma_active(&self) -> bool {
        self.dma.active()
    }

    /// Returns how many words the current transfer still has to move.
    ///
    /// The transfer is eager, so a completed transfer reports zero.
    #[must_use]
    pub const fn dma_words_remaining(&self) -> u32 {
        0
    }

    /// Takes the DMA slot cost recorded by the last transfer, in master cycles.
    ///
    /// Taking the cost closes the `DMA_BUSY` window; the count is then zero. It
    /// leaves the FIFO wait accounting and the VBlank/HBlank pending requests
    /// untouched: those are owned by [`Vdp::take_fifo_wait_cycles`] and
    /// [`Vdp::acknowledge`]/[`Vdp::take_interrupts`] respectively, so an owner
    /// that only consumes the DMA cost cannot silently lose waits or
    /// interrupts.
    /// Number of data-port accesses discarded because their command code is not
    /// modelled.
    ///
    /// The pinned oracle discards such an access and leaves the VDP unaffected
    /// (`Devices/315-5313/S315-5313_General.cpp:2575` for writes, `:2458` for
    /// reads), so the slice does the same rather than aborting the host, and
    /// keeps this counter so the discard is never silent. It is diagnostic
    /// state: it does not enter the canonical snapshot (see
    /// [`Vdp::discarded_data_port_accesses`]'s field) and no restore needs it.
    pub const fn discarded_data_port_accesses(&self) -> u64 {
        self.discarded_data_port_accesses
    }

    pub fn take_dma_cycles(&mut self) -> u32 {
        let cycles = self.dma_cycles;
        self.dma_cycles = 0;
        if self.dma.active() {
            self.dma =
                DmaState::from_parts(self.dma.source(), self.dma.length(), self.dma.mode(), false);
        }
        cycles
    }

    /// Reads one byte from the VDP port at `offset`.
    ///
    /// Even offsets select the high byte and odd offsets the low byte of the
    /// word returned by [`Vdp::read16`] semantics. A data-port byte read still
    /// transfers a whole word and applies auto-increment, matching the VDP's
    /// word-oriented data port.
    pub fn read8(&mut self, offset: u8) -> Result<u8, VdpError> {
        Self::validate_offset(offset)?;
        let word = match offset & 0x0C {
            0x00 => self.read_data_port()?,
            0x04 => self.read_control_port(),
            _ => self.hv_counter(),
        };
        if offset & 1 == 0 {
            Ok((word >> 8) as u8)
        } else {
            Ok(word as u8)
        }
    }

    /// Writes one byte to the VDP port at `offset`.
    ///
    /// Byte writes to the data and control ports duplicate the byte into both
    /// halves of the 16-bit port word before it is processed. This matches the
    /// documented data-port byte-write behaviour and gives control-port byte
    /// writes a deterministic meaning. HV-counter writes are rejected, and a
    /// data-port byte write while only the first word of a two-word command is
    /// latched fails with [`VdpError::UnsupportedTarget`] without touching
    /// VRAM.
    pub fn write8(&mut self, offset: u8, value: u8) -> Result<(), VdpError> {
        Self::validate_offset(offset)?;
        let word = (u16::from(value) << 8) | u16::from(value);
        match offset & 0x0C {
            0x00 => self.write_data_port(word),
            0x04 => self.write_control_port(word),
            _ => Err(VdpError::UnsupportedWrite { offset }),
        }
    }

    /// Reads a big-endian word from the VDP port at `offset`.
    ///
    /// Word accesses require an even offset. Data-port reads use the command
    /// code set up through the control port and apply auto-increment; reads
    /// from the control port return the status register and cancel any
    /// half-completed command; HV-counter reads return the current approximate
    /// V/H position (V in the high byte).
    ///
    /// A control-port read does not acknowledge any interrupt: the derived
    /// VINT-pending latch is unchanged by the read.
    pub fn read16(&mut self, offset: u8) -> Result<u16, VdpError> {
        Self::validate_offset(offset)?;
        if offset & 1 != 0 {
            return Err(VdpError::OddOffset { offset });
        }
        match offset & 0x0C {
            0x00 => self.read_data_port(),
            0x04 => Ok(self.read_control_port()),
            _ => Ok(self.hv_counter()),
        }
    }

    /// Writes a big-endian word to the VDP port at `offset`.
    ///
    /// Word accesses require an even offset. Data-port writes use the command
    /// code set up through the control port and apply auto-increment; a
    /// data-port write while only the first word of a two-word command is
    /// latched fails with [`VdpError::UnsupportedTarget`] without touching
    /// VRAM. Control-port writes implement register selection and the two-word
    /// command setup described in the module documentation. HV-counter writes
    /// are rejected.
    pub fn write16(&mut self, offset: u8, value: u16) -> Result<(), VdpError> {
        Self::validate_offset(offset)?;
        if offset & 1 != 0 {
            return Err(VdpError::OddOffset { offset });
        }
        match offset & 0x0C {
            0x00 => self.write_data_port(value),
            0x04 => self.write_control_port(value),
            _ => Err(VdpError::UnsupportedWrite { offset }),
        }
    }

    /// Serializes the complete VDP state into [`SNAPSHOT_LEN`] bytes.
    ///
    /// Layout, in order (all multi-byte integers little-endian): 8-byte magic,
    /// `u16` format version, one flag byte (bit 0 VBlank, bit 1 pending command
    /// half, bit 2 odd frame, bit 3 sprite collision, bit 4 sprite overflow,
    /// bit 5 PAL), one command code byte, `u16` address, 24 registers, 64 CRAM
    /// words, 40 VSRAM words, one FIFO occupancy byte, four FIFO words oldest
    /// first (zero-padded), one DMA active byte, `u32` DMA source, `u16` DMA
    /// length, one DMA mode byte, `u64` frame index, `u16` line, `u16` dot and
    /// 64 KiB of VRAM. The format is versioned and self-describing; consumers
    /// must not reinterpret bytes from another version. `CFVDP001` v1 payloads
    /// are rejected by [`Vdp::restore_bytes`].
    ///
    /// The derived VINT-pending latch is intentionally excluded: it is a
    /// function of the owning machine's canonical interrupt state, which that
    /// machine snapshots and hashes separately.
    #[must_use]
    pub fn snapshot_bytes(&self) -> Vec<u8> {
        let mut bytes = Writer::with_capacity(SNAPSHOT_LEN);
        bytes.write_bytes(&SNAPSHOT_MAGIC);
        bytes.write_u16_le(SNAPSHOT_VERSION);
        let mut flags = 0;
        if self.status_latches.vblank {
            flags |= SNAPSHOT_FLAG_VBLANK;
        }
        if self.command_pending {
            flags |= SNAPSHOT_FLAG_COMMAND_PENDING;
        }
        if self.status_latches.odd_frame {
            flags |= SNAPSHOT_FLAG_ODD_FRAME;
        }
        if self.status_latches.sprite_collision {
            flags |= SNAPSHOT_FLAG_SPRITE_COLLISION;
        }
        if self.status_latches.sprite_overflow {
            flags |= SNAPSHOT_FLAG_SPRITE_OVERFLOW;
        }
        if self.video == VideoStandard::Pal {
            flags |= SNAPSHOT_FLAG_PAL;
        }
        bytes.write_u8(flags);
        bytes.write_u8(self.code);
        bytes.write_u16_le(self.address);
        bytes.write_bytes(&self.registers);
        for word in &self.cram {
            bytes.write_u16_le(*word);
        }
        for word in &self.vsram {
            bytes.write_u16_le(*word);
        }
        // The FIFO occupancy is bounded by FIFO_CAPACITY, so the cast is lossless.
        bytes.write_u8(self.fifo.len() as u8);
        for word in self.fifo.words() {
            bytes.write_u16_le(word);
        }
        bytes.write_u8(u8::from(self.dma.active()));
        bytes.write_u32_le(self.dma.source());
        bytes.write_u16_le(self.dma.length());
        bytes.write_u8(self.dma.mode().code());
        bytes.write_u64_le(self.frame_index);
        bytes.write_u16_le(self.line);
        bytes.write_u16_le(self.dot);
        bytes.write_bytes(&self.vram);
        bytes.into_bytes()
    }

    /// Restores state from a [`Vdp::snapshot_bytes`] payload.
    ///
    /// Length, magic, version, reserved flag bits, the command-code width, the
    /// FIFO occupancy, the DMA mode code, the DMA active byte and the line/dot
    /// position (against the stored video standard) are validated before any
    /// field is mutated, so a rejected snapshot leaves the VDP exactly as it
    /// was.
    ///
    /// The write path uses the shared [`chronofork_core::codec::Writer`]. The
    /// read path stays index-based because this format has a compile-time fixed
    /// length and its error contract requires the exact-length mismatch
    /// ([`VdpError::SnapshotLengthMismatch`]) to win over every field error; a
    /// general cursor would have to reinterpret a bounds failure as that
    /// mismatch. The layout constants make each field access infallible once the
    /// length and magic checks pass.
    ///
    /// The derived VINT-pending latch is not serialized and is left untouched;
    /// the owning machine must recompute it from its canonical interrupt state
    /// after restoring (the minimal Mega Drive backend does this before returning
    /// from its own restore). This method never acknowledges an interrupt.
    pub fn restore_bytes(&mut self, bytes: &[u8]) -> Result<(), VdpError> {
        if bytes.len() != SNAPSHOT_LEN {
            return Err(VdpError::SnapshotLengthMismatch {
                expected: SNAPSHOT_LEN,
                found: bytes.len(),
            });
        }
        if &bytes[..SNAPSHOT_MAGIC.len()] != SNAPSHOT_MAGIC.as_slice() {
            return Err(VdpError::SnapshotMagicMismatch);
        }
        let version = read_u16_le(bytes, 8);
        if version != SNAPSHOT_VERSION {
            return Err(VdpError::SnapshotVersionUnsupported { found: version });
        }
        let flags = bytes[SNAPSHOT_FLAGS_OFFSET];
        if flags & !SNAPSHOT_FLAG_MASK != 0 {
            return Err(VdpError::SnapshotFlagsInvalid { flags });
        }
        let code = bytes[SNAPSHOT_CODE_OFFSET];
        if code & !0x3F != 0 {
            return Err(VdpError::SnapshotCodeInvalid { code });
        }
        let fifo_count = bytes[SNAPSHOT_FIFO_COUNT_OFFSET];
        let mut fifo_words = [0_u16; FIFO_CAPACITY];
        for (index, word) in fifo_words.iter_mut().enumerate() {
            *word = read_u16_le(bytes, SNAPSHOT_FIFO_WORDS_OFFSET + index * 2);
        }
        let fifo = Fifo::try_restore(usize::from(fifo_count), fifo_words)
            .ok_or(VdpError::SnapshotFifoCountInvalid { count: fifo_count })?;
        let dma_mode = bytes[SNAPSHOT_DMA_MODE_OFFSET];
        let mode = DmaMode::from_code(dma_mode)
            .ok_or(VdpError::SnapshotDmaModeInvalid { mode: dma_mode })?;
        let dma_active = match bytes[SNAPSHOT_DMA_ACTIVE_OFFSET] {
            0 => false,
            1 => true,
            value => {
                return Err(VdpError::SnapshotBooleanInvalid {
                    field: "DMA active",
                    value,
                });
            }
        };
        let video = if flags & SNAPSHOT_FLAG_PAL != 0 {
            VideoStandard::Pal
        } else {
            VideoStandard::Ntsc
        };
        let line = read_u16_le(bytes, SNAPSHOT_LINE_OFFSET);
        let dot = read_u16_le(bytes, SNAPSHOT_DOT_OFFSET);
        if line >= lines_per_frame(video) {
            return Err(VdpError::SnapshotLineOutOfRange {
                line,
                lines_per_frame: lines_per_frame(video),
            });
        }
        if u64::from(dot) >= MASTER_CYCLES_PER_LINE {
            return Err(VdpError::SnapshotDotOutOfRange {
                dot,
                master_cycles_per_line: MASTER_CYCLES_PER_LINE,
            });
        }

        let address = read_u16_le(bytes, SNAPSHOT_ADDRESS_OFFSET);
        let mut registers = [0; REGISTER_COUNT];
        registers.copy_from_slice(
            &bytes[SNAPSHOT_REGISTERS_OFFSET..SNAPSHOT_REGISTERS_OFFSET + REGISTER_COUNT],
        );
        let mut cram = [0_u16; CRAM_WORDS];
        for (index, word) in cram.iter_mut().enumerate() {
            *word = read_u16_le(bytes, SNAPSHOT_CRAM_OFFSET + index * 2);
        }
        let mut vsram = [0_u16; VSRAM_WORDS];
        for (index, word) in vsram.iter_mut().enumerate() {
            *word = read_u16_le(bytes, SNAPSHOT_VSRAM_OFFSET + index * 2);
        }
        let dma = DmaState::from_parts(
            read_u32_le(bytes, SNAPSHOT_DMA_SOURCE_OFFSET),
            read_u16_le(bytes, SNAPSHOT_DMA_LENGTH_OFFSET),
            mode,
            dma_active,
        );
        let frame_index = read_u64_le(bytes, SNAPSHOT_FRAME_INDEX_OFFSET);

        self.status_latches = StatusLatches {
            vblank: flags & SNAPSHOT_FLAG_VBLANK != 0,
            odd_frame: flags & SNAPSHOT_FLAG_ODD_FRAME != 0,
            sprite_collision: flags & SNAPSHOT_FLAG_SPRITE_COLLISION != 0,
            sprite_overflow: flags & SNAPSHOT_FLAG_SPRITE_OVERFLOW != 0,
        };
        self.command_pending = flags & SNAPSHOT_FLAG_COMMAND_PENDING != 0;
        self.code = code;
        self.address = address;
        self.registers = registers;
        self.cram = cram;
        self.vsram = vsram;
        self.fifo = fifo;
        self.dma = dma;
        self.frame_index = frame_index;
        self.line = line;
        self.dot = dot;
        self.video = video;
        self.sprite_limit_reached = false;
        self.dma_cycles = 0;
        self.fifo_wait_cycles = 0;
        self.vblank_pending = false;
        self.hblank_pending = false;
        self.vram.copy_from_slice(&bytes[SNAPSHOT_VRAM_OFFSET..]);
        Ok(())
    }

    fn validate_offset(offset: u8) -> Result<(), VdpError> {
        if usize::from(offset) >= PORT_WINDOW_SIZE {
            return Err(VdpError::UnsupportedOffset { offset });
        }
        Ok(())
    }

    fn read_data_port(&mut self) -> Result<u16, VdpError> {
        self.command_pending = false;
        if self.code & CODE_DMA != 0 {
            return Err(VdpError::DmaUnsupported { code: self.code });
        }
        if self.code & CODE_READ_INHIBIT != 0 {
            return Err(VdpError::UnsupportedTarget { code: self.code });
        }
        // The mirror image of the write decode: VRAM (0b0000), VSRAM (0b0100)
        // and CRAM (0b1000) reads are the decoded directions. `0b1100` is the
        // oracle's undocumented 8-bit VRAM read (`:2447`): a capability this
        // slice does not model, so it stays a typed rejection and is **not**
        // counted as a discard. Every other code hits the oracle's read
        // `default:` (`:2458`), where no fetch happens and the oracle reports
        // the error to the caller instead of leaving the port hanging; this
        // slice reports the same way with a typed error and, unlike the write
        // path, commits no state, so the access stays atomic.
        let target = match self.code & 0x0F {
            CODE_VRAM_READ => VramTarget::Vram,
            CODE_VSRAM_READ => VramTarget::Vsram,
            CODE_CRAM_READ => VramTarget::Cram,
            _ => return Err(VdpError::UnsupportedTarget { code: self.code }),
        };
        let address = Self::word_address(target, self.address);
        let word = self.read_word(target, address)?;
        self.increment_address();
        Ok(word)
    }

    fn write_data_port(&mut self, value: u16) -> Result<(), VdpError> {
        if self.command_pending {
            // Only the first word of a two-word control command has been
            // latched, so the write target and address are not fully decoded
            // yet. Reject the access without mutating memory; the half-latched
            // command stays pending and can still be completed by its second
            // control word.
            return Err(VdpError::UnsupportedTarget { code: self.code });
        }
        if self.code & CODE_DMA != 0 {
            // A data-port access while the DMA command code is latched triggers
            // the transfer programmed through registers 0x13..0x17.
            self.start_dma_from_registers()?;
            return Ok(());
        }
        // A data-port write is only meaningful for the three *write* codes:
        // VRAM (0b0001), CRAM (0b0011) and VSRAM (0b0101), decoded from the low
        // four code bits exactly as the pinned oracle does
        // (`Devices/315-5313/S315-5313_General.cpp:2507`). Anything else —
        // including a *read* code such as `0b0100`, which has no write to
        // perform — is discarded with no effect on the VDP (`:2575-2580`), and
        // the target address still advances by the auto-increment register,
        // because every data-port write chains the previous write address plus
        // that register (`Devices/315-5313/S315-5313_Ports.cpp:1321-1331`).
        // The discard is counted so it can never be silent.
        let target = match self.code & 0x0F {
            CODE_VRAM_WRITE => VramTarget::Vram,
            CODE_CRAM_WRITE => VramTarget::Cram,
            CODE_VSRAM_WRITE => VramTarget::Vsram,
            _ => {
                self.discarded_data_port_accesses =
                    self.discarded_data_port_accesses.saturating_add(1);
                self.increment_address();
                return Ok(());
            }
        };
        let address = Self::word_address(target, self.address);
        self.write_word(target, address, value)?;
        // The data write completes eagerly; the queue tracks occupancy and
        // backpressure. A full queue costs one 68000-cycle wait slot per extra
        // write without dropping or reordering it.
        if !self.fifo.push(value) {
            self.fifo_wait_cycles = self.fifo_wait_cycles.saturating_add(1);
        }
        self.increment_address();
        Ok(())
    }

    /// Converts the byte-oriented command address to the space's word index.
    ///
    /// VRAM addresses bytes directly; CRAM and VSRAM address words, and the
    /// command address carries the byte offset, so two bytes advance one entry.
    const fn word_address(target: VramTarget, address: u16) -> u16 {
        match target {
            VramTarget::Vram | VramTarget::VramCopy => address,
            VramTarget::Cram | VramTarget::Vsram => address >> 1,
        }
    }

    fn read_control_port(&mut self) -> u16 {
        self.command_pending = false;
        self.status()
    }

    fn write_control_port(&mut self, word: u16) -> Result<(), VdpError> {
        if self.command_pending {
            self.code = (self.code & 0x03) | (((word >> 4) as u8 & 0x0F) << 2);
            self.address |= (word & 0x0003) << 14;
            self.command_pending = false;
            return Ok(());
        }

        let low_code = ((word >> 14) & 0x03) as u8;
        if low_code == CODE_REGISTER_WRITE {
            // Register write: 1 0 RRRRR DDDDDDDD. The register range is
            // validated before any state, including the latched command
            // registers, is mutated.
            let register = ((word >> 8) & 0x1F) as u8;
            let index = usize::from(register);
            if index >= REGISTER_COUNT {
                return Err(VdpError::UnsupportedRegister { register });
            }
            // Real hardware latches the first command word before performing
            // the register write, leaving the read-inhibit bit set and the
            // data-port target invalid until a new address is set up.
            self.code = low_code | CODE_READ_INHIBIT;
            self.address = word & 0x3FFF;
            self.registers[index] = (word & 0xFF) as u8;
            self.command_pending = false;
            return Ok(());
        }

        self.code = low_code | CODE_READ_INHIBIT;
        self.address = word & 0x3FFF;
        self.command_pending = true;
        Ok(())
    }

    /// Drains the write FIFO after the owner advanced the device timeline.
    ///
    /// The data writes already completed eagerly, so draining only releases the
    /// occupancy/backpressure bookkeeping. The owning backend calls this after
    /// every step's [`Vdp::advance_master`].
    pub fn drain_fifo(&mut self) {
        self.fifo.drain();
    }

    /// Takes the accumulated FIFO wait slots, in 68000 clock cycles.
    ///
    /// Each data-port write that found the four-word queue full contributes one
    /// slot. The backend converts the count to master cycles with
    /// `wait * MASTER_CYCLES_PER_68000_CYCLE`.
    pub fn take_fifo_wait_cycles(&mut self) -> u32 {
        let wait = self.fifo_wait_cycles;
        self.fifo_wait_cycles = 0;
        wait
    }

    /// Reads a big-endian VRAM word for the DMA engine.
    fn read_vram_word(&self, address: u16) -> u16 {
        let index = usize::from(address & 0xFFFE);
        u16::from_be_bytes([self.vram[index], self.vram[index + 1]])
    }

    fn increment_address(&mut self) {
        let increment = u16::from(self.registers[AUTO_INCREMENT_REGISTER]);
        self.address = self.address.wrapping_add(increment);
    }
}

impl Default for Vdp {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Vdp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Vdp")
            .field("vblank", &self.status_latches.vblank)
            .field("vint_pending", &self.vint_pending)
            .field("command_pending", &self.command_pending)
            .field("code", &format_args!("0x{:02X}", self.code))
            .field("address", &format_args!("0x{:04X}", self.address))
            .field("registers", &self.registers)
            .field("cram_len", &self.cram.len())
            .field("vsram_len", &self.vsram.len())
            .field("fifo_len", &self.fifo.len())
            .field("dma_active", &self.dma.active())
            .field("frame_index", &self.frame_index)
            .field("line", &self.line)
            .field("dot", &self.dot)
            .field("video", &self.video)
            .field("sprite_limit_reached", &self.sprite_limit_reached)
            .field("dma_cycles", &self.dma_cycles)
            .field("fifo_wait_cycles", &self.fifo_wait_cycles)
            .field("vblank_pending", &self.vblank_pending)
            .field("hblank_pending", &self.hblank_pending)
            .field("vram_len", &self.vram.len())
            .finish()
    }
}

/// Reads a little-endian `u16` at a validated fixed offset.
///
/// The caller has checked [`SNAPSHOT_LEN`], so the slice indexes cannot panic.
fn read_u16_le(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

/// Reads a little-endian `u32` at a validated fixed offset.
fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

/// Reads a little-endian `u64` at a validated fixed offset.
fn read_u64_le(bytes: &[u8], offset: usize) -> u64 {
    let mut value = [0_u8; 8];
    value.copy_from_slice(&bytes[offset..offset + 8]);
    u64::from_le_bytes(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_vram_address(vdp: &mut Vdp, address: u16, code: u16) -> Result<(), VdpError> {
        vdp.write16(CONTROL_PORT_OFFSET, code | (address & 0x3FFF))?;
        vdp.write16(CONTROL_PORT_OFFSET, (address >> 14) & 0x0003)
    }

    fn set_vram_write_address(vdp: &mut Vdp, address: u16) -> Result<(), VdpError> {
        set_vram_address(vdp, address, 0x4000)
    }

    fn set_vram_read_address(vdp: &mut Vdp, address: u16) -> Result<(), VdpError> {
        set_vram_address(vdp, address, 0x0000)
    }

    #[test]
    fn register_writes_select_register_and_value() {
        let mut vdp = Vdp::new();
        vdp.write16(CONTROL_PORT_OFFSET, 0x8004)
            .expect("register 0 write");
        vdp.write16(CONTROL_PORT_OFFSET, 0x8F02)
            .expect("register 15 write");

        assert_eq!(vdp.register(0), Some(0x04));
        assert_eq!(vdp.register(15), Some(0x02));
        assert_eq!(vdp.register(REGISTER_COUNT), None);
        for index in 1..REGISTER_COUNT {
            if index != AUTO_INCREMENT_REGISTER {
                assert_eq!(vdp.register(index), Some(0));
            }
        }
    }

    #[test]
    fn register_write_uses_mirror_and_rejects_unknown_register() {
        let mut vdp = Vdp::new();
        let mut expected = [0_u8; REGISTER_COUNT];
        vdp.write16(CONTROL_PORT_OFFSET + 2, 0x8F01)
            .expect("register write through mirror");
        expected[AUTO_INCREMENT_REGISTER] = 0x01;
        assert_eq!(vdp.registers(), &expected);

        assert_eq!(
            vdp.write16(CONTROL_PORT_OFFSET, 0x9810),
            Err(VdpError::UnsupportedRegister { register: 0x18 })
        );
        assert_eq!(vdp.registers(), &expected);
        // A register write also latches the command word as its address, as on
        // real hardware; the rejected register write must not change that.
        assert_eq!(vdp.address(), 0x0F01);
    }

    #[test]
    fn vram_writes_follow_auto_increment_and_wrap() {
        let mut vdp = Vdp::new();
        vdp.write16(CONTROL_PORT_OFFSET, 0x8F02)
            .expect("auto-increment 2");
        set_vram_write_address(&mut vdp, 0x0100).expect("vram write address");
        assert_eq!(vdp.address(), 0x0100);

        vdp.write16(DATA_PORT_OFFSET, 0x1234).expect("first word");
        vdp.write16(DATA_PORT_OFFSET, 0xABCD).expect("second word");
        assert_eq!(&vdp.vram()[0x0100..0x0104], &[0x12, 0x34, 0xAB, 0xCD]);
        assert_eq!(vdp.address(), 0x0104);

        // The data port mirrors at 0x02 and auto-increment wraps at 64 KiB.
        vdp.write16(DATA_PORT_OFFSET + 2, 0x5678)
            .expect("mirrored word");
        assert_eq!(&vdp.vram()[0x0104..0x0106], &[0x56, 0x78]);
        assert_eq!(vdp.address(), 0x0106);

        set_vram_write_address(&mut vdp, 0xFFFE).expect("top of vram");
        vdp.write16(DATA_PORT_OFFSET, 0xBEEF)
            .expect("wrapping word");
        assert_eq!(&vdp.vram()[0xFFFE..], &[0xBE, 0xEF]);
        assert_eq!(vdp.address(), 0x0000);
    }

    #[test]
    fn vram_word_and_byte_reads_follow_command_code() {
        let mut vdp = Vdp::new();
        vdp.write16(CONTROL_PORT_OFFSET, 0x8F02)
            .expect("auto-increment 2");
        set_vram_write_address(&mut vdp, 0x0200).expect("vram write address");
        vdp.write16(DATA_PORT_OFFSET, 0x1111).expect("first word");
        vdp.write16(DATA_PORT_OFFSET, 0x2222).expect("second word");

        set_vram_read_address(&mut vdp, 0x0200).expect("vram read address");
        assert_eq!(vdp.read16(DATA_PORT_OFFSET).expect("word read"), 0x1111);
        assert_eq!(vdp.read16(DATA_PORT_OFFSET).expect("word read"), 0x2222);
        assert_eq!(vdp.address(), 0x0204);

        // A byte read still transfers a whole word; the lane selects which
        // byte of that word is returned.
        set_vram_read_address(&mut vdp, 0x0200).expect("vram read address");
        assert_eq!(vdp.read8(DATA_PORT_OFFSET).expect("high byte"), 0x11);
        assert_eq!(vdp.read8(DATA_PORT_OFFSET + 1).expect("low byte"), 0x22);
        assert_eq!(vdp.address(), 0x0204);
    }

    #[test]
    fn data_port_byte_writes_duplicate_into_both_halves() {
        let mut vdp = Vdp::new();
        set_vram_write_address(&mut vdp, 0x0300).expect("vram write address");
        vdp.write8(DATA_PORT_OFFSET, 0x5A).expect("byte write");
        assert_eq!(&vdp.vram()[0x0300..0x0302], &[0x5A, 0x5A]);
        assert_eq!(vdp.address(), 0x0300);
    }

    #[test]
    fn status_reports_vblank_and_fifo_empty() {
        let mut vdp = Vdp::new();
        assert_eq!(vdp.status(), STATUS_FIFO_EMPTY);
        assert_eq!(vdp.status() & !STATUS_MASK, 0);
        assert_eq!(vdp.status() & STATUS_VBLANK, 0);

        vdp.set_vblank(true);
        assert_eq!(vdp.status(), STATUS_FIFO_EMPTY | STATUS_VBLANK);
        assert_eq!(
            vdp.read16(CONTROL_PORT_OFFSET).expect("status read"),
            STATUS_FIFO_EMPTY | STATUS_VBLANK
        );
        assert_eq!(
            vdp.read8(CONTROL_PORT_OFFSET).expect("high status byte"),
            0x01
        );
        assert_eq!(
            vdp.read8(CONTROL_PORT_OFFSET + 1).expect("low status byte"),
            0x08
        );

        vdp.set_vblank(false);
        assert_eq!(vdp.status(), STATUS_FIFO_EMPTY);
        assert_eq!(
            vdp.read8(CONTROL_PORT_OFFSET + 1).expect("low status byte"),
            0x00
        );
    }

    #[test]
    fn vint_pending_bit_is_driven_and_status_reads_never_clear_it() {
        let mut vdp = Vdp::new();
        assert_eq!(vdp.status() & STATUS_VINT_PENDING, 0);
        assert!(!vdp.vint_pending());

        vdp.set_vint_pending(true);
        assert!(vdp.vint_pending());
        assert_eq!(
            vdp.status(),
            STATUS_FIFO_EMPTY | STATUS_VINT_PENDING,
            "bit 7 reports the pending VBlank request"
        );
        // A control-port read reports the bit but does not acknowledge it.
        assert_eq!(
            vdp.read16(CONTROL_PORT_OFFSET).expect("status read"),
            STATUS_FIFO_EMPTY | STATUS_VINT_PENDING
        );
        assert!(
            vdp.vint_pending(),
            "a status read must not acknowledge VINT"
        );
        assert_eq!(
            vdp.read8(CONTROL_PORT_OFFSET).expect("high status byte"),
            0x01
        );
        assert_eq!(
            vdp.read8(CONTROL_PORT_OFFSET + 1).expect("low status byte"),
            0x80
        );
        assert!(
            vdp.vint_pending(),
            "byte status reads must not acknowledge VINT either"
        );

        // The video-blank flag (bit 3) and interrupt-pending flag (bit 7) are
        // independent: blanking can end while the request is still pending.
        vdp.set_vblank(true);
        assert_eq!(vdp.status() & 0x0088, STATUS_VBLANK | STATUS_VINT_PENDING);
        vdp.set_vblank(false);
        assert_eq!(vdp.status() & 0x0088, STATUS_VINT_PENDING);

        // Only the owner of the canonical request clears the latch.
        vdp.set_vint_pending(false);
        assert_eq!(vdp.status() & STATUS_VINT_PENDING, 0);
    }

    #[test]
    fn snapshot_excludes_the_derived_vint_latch() {
        let mut vdp = Vdp::new();
        vdp.set_vint_pending(true);
        let snapshot = vdp.snapshot_bytes();

        let mut restored = Vdp::new();
        restored.restore_bytes(&snapshot).expect("restore snapshot");
        assert_eq!(
            restored.snapshot_bytes(),
            snapshot,
            "the derived latch is not serialized"
        );
        assert!(
            !restored.vint_pending(),
            "restore_bytes must not invent derived state"
        );

        // A restore into a VDP that already has the derived latch set leaves it
        // untouched, so the owning machine can recompute or preserve it.
        let mut already_pending = Vdp::new();
        already_pending.set_vint_pending(true);
        already_pending
            .restore_bytes(&snapshot)
            .expect("restore snapshot");
        assert!(already_pending.vint_pending());
        assert_eq!(
            already_pending.status() & STATUS_VINT_PENDING,
            STATUS_VINT_PENDING
        );
    }

    #[test]
    fn hv_counter_reads_are_deterministic_and_writes_rejected() {
        let mut vdp = Vdp::new();
        assert_eq!(vdp.read16(HV_COUNTER_OFFSET).expect("hv read"), 0);
        assert_eq!(vdp.read16(0x0E).expect("hv mirror read"), 0);
        assert_eq!(vdp.read8(0x09).expect("hv byte read"), 0);
        assert_eq!(
            vdp.write16(HV_COUNTER_OFFSET, 0),
            Err(VdpError::UnsupportedWrite {
                offset: HV_COUNTER_OFFSET
            })
        );
        assert_eq!(
            vdp.write8(0x0C, 0),
            Err(VdpError::UnsupportedWrite { offset: 0x0C })
        );
    }

    #[test]
    fn unsupported_port_accesses_are_typed_errors() {
        let mut vdp = Vdp::new();
        assert_eq!(
            vdp.read16(PORT_WINDOW_SIZE as u8),
            Err(VdpError::UnsupportedOffset {
                offset: PORT_WINDOW_SIZE as u8
            })
        );
        assert_eq!(
            vdp.write8(0x10, 0),
            Err(VdpError::UnsupportedOffset { offset: 0x10 })
        );
        assert_eq!(
            vdp.read16(DATA_PORT_OFFSET + 1),
            Err(VdpError::OddOffset { offset: 1 })
        );
        assert_eq!(
            vdp.write16(CONTROL_PORT_OFFSET + 1, 0),
            Err(VdpError::OddOffset { offset: 5 })
        );
    }

    #[test]
    fn boot_vsram_write_macro_reaches_vsram_word_zero() {
        // The exact control-word pair the boot fixtures issue right after the
        // 64 KiB clear: `0x4000` then `0x0010`, i.e. CD3..CD0 = 0b0101 (VSRAM
        // write) at command address 0. It must land in VSRAM word 0 and the same
        // code must stay a typed error on a read, because the direction comes
        // from the access, not from the code alone in the opposite sense.
        let mut vdp = Vdp::new();
        vdp.write16(CONTROL_PORT_OFFSET, 0x4000)
            .expect("boot first half");
        vdp.write16(CONTROL_PORT_OFFSET, 0x0010)
            .expect("boot second half");
        vdp.write16(DATA_PORT_OFFSET, 0x0123).expect("vsram write");
        assert_eq!(vdp.vsram_word(0), Some(0x0123));

        // The VSRAM write code is not a read direction.
        vdp.write16(CONTROL_PORT_OFFSET, 0x4000)
            .expect("boot first half");
        vdp.write16(CONTROL_PORT_OFFSET, 0x0010)
            .expect("boot second half");
        assert_eq!(
            vdp.read16(DATA_PORT_OFFSET),
            Err(VdpError::UnsupportedTarget { code: 0x05 })
        );

        // And the boot pair is not a DMA target: with CD5 clear it never
        // triggers the register-programmed engine.
        assert!(!vdp.dma_active());
        assert_eq!(vdp.take_dma_cycles(), 0);
    }

    #[test]
    fn data_port_decodes_supported_targets_and_rejects_unknown_ones() {
        let mut vdp = Vdp::new();
        // CRAM write command: CD3..CD0 = 0x3.
        vdp.write16(CONTROL_PORT_OFFSET, 0xC000)
            .expect("cram first half");
        vdp.write16(CONTROL_PORT_OFFSET, 0x0000)
            .expect("cram second half");
        vdp.write16(DATA_PORT_OFFSET, 0x0EEE).expect("cram write");
        assert_eq!(vdp.cram_entry(0), Some(0x0EEE));

        // The `0x4000`/`0x0010` pair is CD3..CD0 = 0b0101, the documented
        // VSRAM **write** code (`0x40000010` is the standard VSRAM macro), so it
        // writes VSRAM word 0. An earlier table had VSRAM read and write
        // swapped, which made this legal write a typed error.
        vdp.write16(CONTROL_PORT_OFFSET, 0x4000)
            .expect("vsram write first half");
        vdp.write16(CONTROL_PORT_OFFSET, 0x0010)
            .expect("vsram write second half");
        vdp.write16(DATA_PORT_OFFSET, 0x0E3C).expect("vsram write");
        assert_eq!(vdp.vsram_word(0), Some(0x0E3C));

        // A code outside the decoded targets is **discarded**, not rejected:
        // `0xC000` / `0x0030` latches CD3..CD0 = 0b1111 and the write has no
        // effect on the VDP, exactly like the pinned oracle's write `default:`
        // (`S315-5313_General.cpp:2575`). The target address still advances by
        // the auto-increment register and the discard is counted.
        assert_eq!(vdp.discarded_data_port_accesses(), 0);
        let address_before = vdp.address();
        vdp.write16(CONTROL_PORT_OFFSET, 0xC000)
            .expect("unknown first half");
        vdp.write16(CONTROL_PORT_OFFSET, 0x0030)
            .expect("unknown second half");
        assert_eq!(vdp.write16(DATA_PORT_OFFSET, 0xFFFF), Ok(()));
        assert_eq!(vdp.discarded_data_port_accesses(), 1);
        assert_eq!(vdp.vsram_word(0), Some(0x0E3C), "no VSRAM mutation");
        assert_eq!(vdp.cram_entry(0), Some(0x0EEE), "no CRAM mutation");

        // A *read* code on a data-port write is the same case: no write to
        // perform, so it is discarded with the counter and the address advance.
        vdp.write16(CONTROL_PORT_OFFSET, 0x0000)
            .expect("read code first half");
        vdp.write16(CONTROL_PORT_OFFSET, 0x0010)
            .expect("read code second half");
        assert_eq!(vdp.write16(DATA_PORT_OFFSET, 0x1234), Ok(()));
        assert_eq!(vdp.discarded_data_port_accesses(), 2);
        assert_eq!(
            vdp.address().wrapping_sub(address_before),
            0,
            "both discards advanced by the auto-increment register, which is 0 here"
        );

        // The counter is diagnostic state and does **not** enter the canonical
        // snapshot: a machine that discarded accesses still serialises
        // byte-for-byte like one that discarded none, and a restore cannot carry
        // the count (which is why no payload version bump is needed).
        let mut twin = Vdp::new();
        assert_eq!(twin.discarded_data_port_accesses(), 0);
        assert_eq!(twin.snapshot_bytes().len(), vdp.snapshot_bytes().len());
        twin.restore_bytes(&vdp.snapshot_bytes()).expect("restore");
        assert_eq!(twin.snapshot_bytes(), vdp.snapshot_bytes());
        assert_eq!(
            twin.discarded_data_port_accesses(),
            0,
            "a restore cannot carry a diagnostic count"
        );

        // DMA transfer: CD5 set in the second command word. The registers are
        // still zero, so the trigger performs a zero-length transfer.
        vdp.write16(CONTROL_PORT_OFFSET, 0x4000)
            .expect("dma first half");
        vdp.write16(CONTROL_PORT_OFFSET, 0x0080)
            .expect("dma second half");
        vdp.write16(DATA_PORT_OFFSET, 0).expect("dma trigger");
        assert_eq!(vdp.take_dma_cycles(), 0);

        // A register write leaves the data-port target invalid.
        vdp.write16(CONTROL_PORT_OFFSET, 0x8F02)
            .expect("register write");
        assert_eq!(
            vdp.read16(DATA_PORT_OFFSET),
            Err(VdpError::UnsupportedTarget { code: 0x12 })
        );

        // A half-completed read command inhibits reads.
        vdp.write16(CONTROL_PORT_OFFSET, 0x0000)
            .expect("read first half");
        assert_eq!(
            vdp.read16(DATA_PORT_OFFSET),
            Err(VdpError::UnsupportedTarget { code: 0x10 })
        );

        // VSRAM **read** is CD3..CD0 = 0b0100, latched by `0x0000`/`0x0010`.
        // The read uses the same `word_address` (address >> 1) and the same
        // 9-bit modulo-40 wrap as the write.
        let mut vdp = Vdp::new();
        vdp.write16(CONTROL_PORT_OFFSET, 0x0000)
            .expect("vsram read first half");
        vdp.write16(CONTROL_PORT_OFFSET, 0x0010)
            .expect("vsram read second half");
        vdp.write_word(VramTarget::Vsram, 0, 0x1AB2)
            .expect("seed vsram");
        assert_eq!(vdp.read16(DATA_PORT_OFFSET), Ok(0x1AB2));
        // An odd command address still selects a word: `>> 1` is the decode.
        vdp.write16(CONTROL_PORT_OFFSET, 0x0002)
            .expect("vsram read first half, word 1");
        vdp.write16(CONTROL_PORT_OFFSET, 0x0010)
            .expect("vsram read second half");
        vdp.write_word(VramTarget::Vsram, 1, 0x2BC3)
            .expect("seed vsram word 1");
        assert_eq!(vdp.read16(DATA_PORT_OFFSET), Ok(0x2BC3));
    }

    #[test]
    fn data_write_rejects_half_completed_command() {
        let mut vdp = Vdp::new();
        vdp.write16(CONTROL_PORT_OFFSET, 0x8F02)
            .expect("auto-increment 2");
        vdp.write16(CONTROL_PORT_OFFSET, 0x4000 | 0x0100)
            .expect("first command half");
        assert_eq!(vdp.address(), 0x0100);

        let before = vdp.vram().to_vec();
        // Only the first control word is latched (code 0x11), so the write
        // target and address are not fully decoded: reject both word and byte
        // data writes without altering VRAM or the latched address.
        assert_eq!(
            vdp.write16(DATA_PORT_OFFSET, 0xDEAD),
            Err(VdpError::UnsupportedTarget { code: 0x11 })
        );
        assert_eq!(
            vdp.write8(DATA_PORT_OFFSET, 0x5A),
            Err(VdpError::UnsupportedTarget { code: 0x11 })
        );
        assert_eq!(vdp.vram(), before.as_slice());
        assert_eq!(vdp.address(), 0x0100);

        // The rejected accesses did not cancel the half-latched command, so
        // completing it still writes normally.
        vdp.write16(CONTROL_PORT_OFFSET, 0x0000)
            .expect("second command half");
        vdp.write16(DATA_PORT_OFFSET, 0x1234)
            .expect("completed command write");
        assert_eq!(&vdp.vram()[0x0100..0x0102], &[0x12, 0x34]);
        assert_eq!(vdp.address(), 0x0102);
    }

    #[test]
    fn snapshot_round_trips_complete_state() {
        let mut vdp = Vdp::new();
        vdp.write16(CONTROL_PORT_OFFSET, 0x8014)
            .expect("register 0 write");
        vdp.write16(CONTROL_PORT_OFFSET, 0x8F02)
            .expect("auto-increment 2");
        set_vram_write_address(&mut vdp, 0x1234).expect("vram write address");
        vdp.write16(DATA_PORT_OFFSET, 0xCAFE).expect("vram write");
        vdp.set_vblank(true);

        let snapshot = vdp.snapshot_bytes();
        assert_eq!(snapshot.len(), SNAPSHOT_LEN);

        let mut restored = Vdp::new();
        restored
            .write16(CONTROL_PORT_OFFSET, 0x8F00)
            .expect("different auto-increment");
        restored.restore_bytes(&snapshot).expect("restore snapshot");

        assert_eq!(restored.snapshot_bytes(), snapshot);
        assert_eq!(restored.status(), vdp.status());
        assert_eq!(restored.registers(), vdp.registers());
        assert_eq!(restored.address(), vdp.address());
        assert_eq!(restored.vram(), vdp.vram());
        assert_eq!(restored.cram(), vdp.cram());
        assert_eq!(restored.vsram(), vdp.vsram());
        assert_eq!(restored.fifo_len(), vdp.fifo_len());
        assert_eq!(restored.dma(), vdp.dma());
        assert_eq!(restored.line(), vdp.line());
        assert_eq!(restored.dot(), vdp.dot());
        assert_eq!(restored.frame_index(), vdp.frame_index());
        assert_eq!(&restored.vram()[0x1234..0x1236], &[0xCA, 0xFE]);
    }

    #[test]
    fn snapshot_preserves_half_completed_command() {
        let mut vdp = Vdp::new();
        vdp.write16(CONTROL_PORT_OFFSET, 0x8F02)
            .expect("auto-increment 2");
        vdp.write16(CONTROL_PORT_OFFSET, 0x4000)
            .expect("first command half");
        let snapshot = vdp.snapshot_bytes();

        let mut restored = Vdp::new();
        restored.restore_bytes(&snapshot).expect("restore snapshot");
        restored
            .write16(CONTROL_PORT_OFFSET, 0x0000)
            .expect("second command half");
        restored
            .write16(DATA_PORT_OFFSET, 0x1234)
            .expect("vram write");
        assert_eq!(&restored.vram()[0..2], &[0x12, 0x34]);
        assert_eq!(restored.address(), 0x0002);
    }

    #[test]
    fn invalid_snapshots_are_rejected_without_mutation() {
        let good = Vdp::new().snapshot_bytes();
        assert_eq!(good.len(), SNAPSHOT_LEN);
        let mut vdp = Vdp::new();
        vdp.write16(CONTROL_PORT_OFFSET, 0x8F02)
            .expect("auto-increment 2");
        vdp.set_vblank(true);
        let before = vdp.snapshot_bytes();

        assert_eq!(
            vdp.restore_bytes(&good[..good.len() - 1]),
            Err(VdpError::SnapshotLengthMismatch {
                expected: SNAPSHOT_LEN,
                found: SNAPSHOT_LEN - 1
            })
        );
        assert_eq!(
            vdp.restore_bytes(&[]),
            Err(VdpError::SnapshotLengthMismatch {
                expected: SNAPSHOT_LEN,
                found: 0
            })
        );

        let mut bad_magic = good.clone();
        bad_magic[0] ^= 0xFF;
        assert_eq!(
            vdp.restore_bytes(&bad_magic),
            Err(VdpError::SnapshotMagicMismatch)
        );

        let mut bad_version = good.clone();
        bad_version[8] = SNAPSHOT_VERSION as u8 + 1;
        assert_eq!(
            vdp.restore_bytes(&bad_version),
            Err(VdpError::SnapshotVersionUnsupported {
                found: SNAPSHOT_VERSION + 1
            })
        );

        let mut bad_flags = good.clone();
        bad_flags[SNAPSHOT_FLAGS_OFFSET] = 0x80;
        assert_eq!(
            vdp.restore_bytes(&bad_flags),
            Err(VdpError::SnapshotFlagsInvalid { flags: 0x80 })
        );

        let mut bad_code = good.clone();
        bad_code[SNAPSHOT_CODE_OFFSET] = 0x40;
        assert_eq!(
            vdp.restore_bytes(&bad_code),
            Err(VdpError::SnapshotCodeInvalid { code: 0x40 })
        );

        let mut bad_fifo = good.clone();
        bad_fifo[SNAPSHOT_FIFO_COUNT_OFFSET] = FIFO_CAPACITY as u8 + 1;
        assert_eq!(
            vdp.restore_bytes(&bad_fifo),
            Err(VdpError::SnapshotFifoCountInvalid {
                count: FIFO_CAPACITY as u8 + 1
            })
        );

        let mut bad_dma_mode = good.clone();
        bad_dma_mode[SNAPSHOT_DMA_MODE_OFFSET] = 0xFF;
        assert_eq!(
            vdp.restore_bytes(&bad_dma_mode),
            Err(VdpError::SnapshotDmaModeInvalid { mode: 0xFF })
        );

        let mut bad_dma_active = good.clone();
        bad_dma_active[SNAPSHOT_DMA_ACTIVE_OFFSET] = 2;
        assert_eq!(
            vdp.restore_bytes(&bad_dma_active),
            Err(VdpError::SnapshotBooleanInvalid {
                field: "DMA active",
                value: 2
            })
        );

        let mut bad_line = good.clone();
        bad_line[SNAPSHOT_LINE_OFFSET..SNAPSHOT_LINE_OFFSET + 2]
            .copy_from_slice(&lines_per_frame(VideoStandard::Ntsc).to_le_bytes());
        assert_eq!(
            vdp.restore_bytes(&bad_line),
            Err(VdpError::SnapshotLineOutOfRange {
                line: lines_per_frame(VideoStandard::Ntsc),
                lines_per_frame: lines_per_frame(VideoStandard::Ntsc)
            })
        );

        let mut bad_dot = good.clone();
        bad_dot[SNAPSHOT_DOT_OFFSET..SNAPSHOT_DOT_OFFSET + 2]
            .copy_from_slice(&(MASTER_CYCLES_PER_LINE as u16).to_le_bytes());
        assert_eq!(
            vdp.restore_bytes(&bad_dot),
            Err(VdpError::SnapshotDotOutOfRange {
                dot: MASTER_CYCLES_PER_LINE as u16,
                master_cycles_per_line: MASTER_CYCLES_PER_LINE
            })
        );

        assert_eq!(vdp.snapshot_bytes(), before);
    }

    #[test]
    fn snapshot_round_trips_the_pal_standard_and_validates_against_it() {
        let mut vdp = Vdp::new();
        vdp.advance_master(MASTER_CYCLES_PER_LINE * 300, VideoStandard::Pal);
        let snapshot = vdp.snapshot_bytes();
        assert_ne!(snapshot[SNAPSHOT_FLAGS_OFFSET] & SNAPSHOT_FLAG_PAL, 0);

        let mut restored = Vdp::new();
        restored.restore_bytes(&snapshot).expect("PAL snapshot");
        assert_eq!(restored.video_standard(), VideoStandard::Pal);
        assert_eq!(restored.line(), 300);
        assert_eq!(restored.status() & STATUS_PAL, STATUS_PAL);
        assert_eq!(restored.snapshot_bytes(), snapshot);

        // Line 300 is valid for the 313-line PAL frame but not for the
        // 262-line NTSC frame, so clearing the PAL flag must reject it.
        let mut ntsc_flag = snapshot.clone();
        ntsc_flag[SNAPSHOT_FLAGS_OFFSET] &= !SNAPSHOT_FLAG_PAL;
        assert_eq!(
            restored.restore_bytes(&ntsc_flag),
            Err(VdpError::SnapshotLineOutOfRange {
                line: 300,
                lines_per_frame: lines_per_frame(VideoStandard::Ntsc)
            })
        );
        assert_eq!(
            restored.snapshot_bytes(),
            snapshot,
            "a rejected restore never mutates"
        );
    }

    #[test]
    fn snapshot_fifo_words_are_canonical_beyond_the_occupancy() {
        let mut vdp = Vdp::new();
        let mut bytes = vdp.snapshot_bytes();
        // Claim two queued words and leave the remaining slots as arbitrary
        // stale bytes; the canonical encoder must zero them.
        bytes[SNAPSHOT_FIFO_COUNT_OFFSET] = 2;
        bytes[SNAPSHOT_FIFO_WORDS_OFFSET..SNAPSHOT_FIFO_WORDS_OFFSET + 2]
            .copy_from_slice(&0x1234_u16.to_le_bytes());
        bytes[SNAPSHOT_FIFO_WORDS_OFFSET + 2..SNAPSHOT_FIFO_WORDS_OFFSET + 4]
            .copy_from_slice(&0x5678_u16.to_le_bytes());
        for word in 2..FIFO_CAPACITY {
            let offset = SNAPSHOT_FIFO_WORDS_OFFSET + word * 2;
            bytes[offset] = 0xAA;
            bytes[offset + 1] = 0xBB;
        }
        vdp.restore_bytes(&bytes).expect("restore non-empty FIFO");
        assert_eq!(vdp.fifo_len(), 2);

        let mut canonical = bytes.clone();
        for word in 2..FIFO_CAPACITY {
            let offset = SNAPSHOT_FIFO_WORDS_OFFSET + word * 2;
            canonical[offset] = 0;
            canonical[offset + 1] = 0;
        }
        assert_eq!(vdp.snapshot_bytes(), canonical);

        // The oldest word is still first: a round trip of the canonical bytes
        // is byte-identical and keeps the occupancy.
        let mut again = Vdp::new();
        again
            .restore_bytes(&canonical)
            .expect("restore canonical FIFO");
        assert_eq!(again.snapshot_bytes(), canonical);
    }

    #[test]
    fn arbitrary_snapshots_are_rejected_without_panicking() {
        let mut vdp = Vdp::new();
        let before = vdp.snapshot_bytes();
        let mut state = 0x1357_9bdf_2468_ace0_u64;
        let mut reject = |length: usize| {
            let mut bytes = vec![0_u8; length];
            for byte in &mut bytes {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *byte = (state >> 24).to_le_bytes()[0];
            }
            assert!(
                vdp.restore_bytes(&bytes).is_err(),
                "arbitrary {length}-byte VDP snapshot must be rejected"
            );
        };
        for length in [0, 1, 7, 8, 9, 10, 11, 12, 13, 14] {
            reject(length);
        }
        for length in [SNAPSHOT_LEN - 1, SNAPSHOT_LEN, SNAPSHOT_LEN + 1] {
            reject(length);
        }
        for _ in 0..64 {
            reject(SNAPSHOT_LEN);
        }
        assert_eq!(vdp.snapshot_bytes(), before);
    }
}
