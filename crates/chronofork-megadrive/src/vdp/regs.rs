//! VDP register, port and status-register semantics.
//!
//! The device, its snapshot and the later rendering, DMA and FIFO modules name
//! every port offset, register index and status bit once, here. Positional
//! values come from the 315-5313 programmer's reference; this slice models the
//! Mega Drive port window at offsets `0x00..=0x0F` relative to `0xC00000`.

/// Size of VDP VRAM in bytes.
pub const VRAM_SIZE: usize = 0x0001_0000;
/// Number of VDP registers implemented by this slice (`0x00..=0x17`).
pub const REGISTER_COUNT: usize = 0x18;
/// VDP register that holds the data-port auto-increment value.
pub const AUTO_INCREMENT_REGISTER: usize = 0x0F;
/// Number of CRAM colour words.
///
/// Hardware stores two 9-bit colour words per palette entry (128 bytes total);
/// this slice keeps the 64 words the port addresses.
pub const CRAM_WORDS: usize = 64;
/// Number of VSRAM vertical-scroll words (11 bits each).
pub const VSRAM_WORDS: usize = 40;
/// Number of port bytes covered by this slice (`0x00..=0x0F`).
pub const PORT_WINDOW_SIZE: usize = 0x10;
/// Offset of the data port relative to `0xC00000`.
pub const DATA_PORT_OFFSET: u8 = 0x00;
/// Offset of the control port relative to `0xC00000`.
pub const CONTROL_PORT_OFFSET: u8 = 0x04;
/// Offset of the HV-counter port relative to `0xC00000`.
pub const HV_COUNTER_OFFSET: u8 = 0x08;

/// PAL region flag (status bit 0).
///
/// Set while the device's timing was last advanced under the PAL video
/// standard; the reset state is NTSC.
pub const STATUS_PAL: u16 = 0x0001;
/// DMA-busy flag (status bit 1).
pub const STATUS_DMA_BUSY: u16 = 0x0002;
/// HBlank flag (status bit 2).
///
/// Set while the master-cycle dot is inside the approximate horizontal
/// blanking tail of the line (see [`crate::vdp::Vdp::h_counter`]).
pub const STATUS_HBLANK: u16 = 0x0004;
/// VBlank flag (status bit 3).
///
/// Driven by [`crate::vdp::Vdp::advance_master`] from the active-line count and
/// re-synced by the owning backend through `set_vblank` to the scheduler's
/// deterministic scanline position.
pub const STATUS_VBLANK: u16 = 0x0008;
/// Odd interlace frame flag (status bit 4).
pub const STATUS_ODD_FRAME: u16 = 0x0010;
/// Sprite-collision flag (status bit 5).
pub const STATUS_SPRITE_COLLISION: u16 = 0x0020;
/// Sprite-overflow flag (status bit 6).
pub const STATUS_SPRITE_OVERFLOW: u16 = 0x0040;
/// VBlank-interrupt-pending flag (status bit 7).
///
/// Driven by [`crate::vdp::Vdp::set_vint_pending`]. The minimal Mega Drive
/// backend sets it while its scheduler holds a pending VBlank (level 6)
/// request, including while the 68000 `SR` interrupt-priority mask blocks
/// delivery, and clears it only when that request is delivered and
/// acknowledged. It is derived state, not part of the serialized VDP snapshot,
/// and a control-port status read never changes it.
pub const STATUS_VINT_PENDING: u16 = 0x0080;
/// FIFO-empty flag (status bit 8).
pub const STATUS_FIFO_EMPTY: u16 = 0x0100;
/// FIFO-full flag (status bit 9).
pub const STATUS_FIFO_FULL: u16 = 0x0200;
/// Mask of the ten status bits defined by the VDP. The upper six bits read as
/// zero here; real hardware leaves them floating.
pub const STATUS_MASK: u16 = 0x03FF;

// Command-code bits. CD0/CD1 are latched from the first control word, CD2..CD5
// from the second. CD4 marks the VRAM-copy command (and inhibits read targets
// after a partial command); CD5 flags a DMA transfer request.
pub(crate) const CODE_READ_INHIBIT: u8 = 0x10;
pub(crate) const CODE_DMA: u8 = 0x20;
pub(crate) const CODE_VRAM_READ: u8 = 0x00;
pub(crate) const CODE_VRAM_WRITE: u8 = 0x01;
pub(crate) const CODE_REGISTER_WRITE: u8 = 0x02;
pub(crate) const CODE_CRAM_WRITE: u8 = 0x03;
pub(crate) const CODE_VSRAM_WRITE: u8 = 0x04;
pub(crate) const CODE_CRAM_READ: u8 = 0x08;

/// A memory space the VDP data port can address.
///
/// The command code latched on the control port selects the space; the
/// read/write direction comes from the port access itself. [`Self::VramCopy`]
/// names the VRAM space as seen by the implemented DMA VRAM-copy engine; it
/// addresses the same linear 64 KiB VRAM as
/// [`Self::Vram`] because the physical interleaved layout is not modelled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VramTarget {
    /// The flat 64 KiB video RAM.
    Vram,
    /// The 64-word colour RAM.
    Cram,
    /// The 40-word vertical-scroll RAM.
    Vsram,
    /// VRAM as addressed by the DMA copy engine.
    VramCopy,
}

/// Latched VDP status conditions that are not a pure function of another
/// field.
///
/// VBlank, odd-frame, sprite-collision and sprite-overflow are latches: once a
/// condition is observed it stays visible in the status register until the
/// owning logic clears it, so they are canonical state and are snapshotted.
/// DMA-busy, FIFO-empty/full and VINT-pending are derived from the DMA state,
/// the FIFO and the owning machine's interrupt request respectively and are
/// deliberately not duplicated here.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StatusLatches {
    /// Vertical-blanking interval (status bit 3).
    pub vblank: bool,
    /// Odd interlace frame (status bit 4). Interlace is not modelled, so this
    /// latch has no generator yet.
    pub odd_frame: bool,
    /// Sprite collision (status bit 5).
    pub sprite_collision: bool,
    /// Sprite overflow (status bit 6).
    pub sprite_overflow: bool,
}
