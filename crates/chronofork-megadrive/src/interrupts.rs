//! Deterministic VBlank/HBlank interrupt scheduler for the Mega Drive slice.
//!
//! The scheduler runs on an explicit **instruction-quantum clock**: one [`InterruptScheduler::tick`]
//! represents one completed 68000 instruction, not one CPU cycle. The caller maps
//! instructions to scanlines through [`SchedulerConfig::instructions_per_line`], and
//! the selected video standard supplies nominal line/frame constants. This keeps
//! interrupts reproducible from virtual state alone, which is what checkpoints and
//! forks require.
//!
//! **This is not cycle-accurate hardware timing.** Real VDP H/V counters, 68000 bus
//! contention, DMA stalls, prefetch and per-instruction cycle costs are approximated
//! or ignored. The model must not be cited as hardware-accurate emulation timing.
//!
//! # Model
//!
//! - NTSC: 262 lines/frame, 224 active lines, nominal 59.92 Hz.
//! - PAL: 313 lines/frame, 240 active lines, nominal 49.70 Hz.
//! - An HBlank request is raised when each active scanline completes.
//! - A VBlank request is raised when the last active scanline completes and vertical
//!   blanking begins.
//! - Vertical blanking lines produce no HBlank requests in this model.
//!
//! # Pending and acknowledge semantics
//!
//! At most one request per interrupt level is retained, mirroring the VDP's
//! per-source interrupt-pending bit. If another request of the same level arrives
//! while one is already pending, it is counted as coalesced: the pending request
//! keeps its original issued tick and the queue cannot grow without bound.
//! Requests of different levels are retained independently. Delivery order is
//! 68000 interrupt priority (VBlank level 6 before HBlank level 4), not arrival
//! order. The scheduler does not mutate the CPU:
//! [`InterruptScheduler::next_deliverable`] applies the single 68000 delivery
//! predicate from
//! [`interrupt_level_deliverable`] to a
//! supplied status register, so level 7 is always deliverable while every other
//! level must be strictly greater than the `SR` interrupt-priority mask.
//! Exception entry itself belongs to the CPU:
//! [`M68000::interrupt`](crate::cpu::M68000::interrupt) loads the accepted level
//! into the mask so a lower pending level cannot nest inside the handler, and
//! `RTE` restores the previous mask from the pushed frame.
//! Acknowledging a level clears its pending request.
//!
//! # Snapshot payload
//!
//! [`InterruptScheduler::snapshot_bytes`] uses a fixed, versioned little-endian
//! layout: magic `CFINTSCH`, `u16` version, `u8` video standard, `u32`
//! instructions-per-line, `u64` ticks, one `u64` issued counter per level
//! (HBlank then VBlank), one `u64` coalesced counter per level (same order), a
//! `u8` pending mask (bit 0 HBlank, bit 1 VBlank), then one `u64` issued tick per
//! set pending bit in HBlank/VBlank order.
//! [`InterruptScheduler::restore_bytes`] rejects unknown versions, malformed
//! payloads and snapshots produced with a different video standard or
//! instructions-per-line configuration; on error the scheduler is left unchanged.

use std::error::Error;
use std::fmt;

use crate::cpu::interrupt_level_deliverable;

/// 68000 interrupt level requested by the VDP at vertical blank.
pub const VBLANK_INTERRUPT_LEVEL: u8 = 6;
/// 68000 interrupt level requested by the VDP at horizontal blank.
pub const HBLANK_INTERRUPT_LEVEL: u8 = 4;
/// NTSC scanlines per frame (including vertical blanking).
pub const NTSC_LINES_PER_FRAME: u32 = 262;
/// NTSC active display scanlines.
pub const NTSC_ACTIVE_LINES: u32 = 224;
/// NTSC nominal frame rate in millihertz (59.92 Hz).
pub const NTSC_FRAME_RATE_MILLIHERTZ: u32 = 59_920;
/// PAL scanlines per frame (including vertical blanking).
pub const PAL_LINES_PER_FRAME: u32 = 313;
/// PAL active display scanlines.
pub const PAL_ACTIVE_LINES: u32 = 240;
/// PAL nominal frame rate in millihertz (49.70 Hz), rounded.
pub const PAL_FRAME_RATE_MILLIHERTZ: u32 = 49_701;
/// Default instruction quanta assigned to one scanline.
///
/// This is an arbitrary model parameter, not a hardware cycle count. Integrations
/// that estimate their average instruction cost can supply a different value.
pub const DEFAULT_INSTRUCTIONS_PER_LINE: u32 = 64;

const SNAPSHOT_MAGIC: [u8; 8] = *b"CFINTSCH";
const SNAPSHOT_VERSION: u16 = 1;
const SNAPSHOT_HEADER_LEN: usize = 56;
const PENDING_MASK_ALL: u8 = 0b0000_0011;

/// Nominal video timing family used by the scheduler.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VideoStandard {
    /// 262 lines/frame with 224 active lines.
    Ntsc,
    /// 313 lines/frame with 240 active lines.
    Pal,
}

impl VideoStandard {
    /// Returns the number of scanlines in one frame, including blanking.
    #[must_use]
    pub const fn lines_per_frame(self) -> u32 {
        match self {
            Self::Ntsc => NTSC_LINES_PER_FRAME,
            Self::Pal => PAL_LINES_PER_FRAME,
        }
    }

    /// Returns the number of active display scanlines.
    #[must_use]
    pub const fn active_lines(self) -> u32 {
        match self {
            Self::Ntsc => NTSC_ACTIVE_LINES,
            Self::Pal => PAL_ACTIVE_LINES,
        }
    }

    /// Returns the number of vertical blanking scanlines.
    #[must_use]
    pub const fn vertical_blank_lines(self) -> u32 {
        self.lines_per_frame() - self.active_lines()
    }

    /// Returns the nominal frame rate in millihertz.
    #[must_use]
    pub const fn nominal_frame_rate_millihz(self) -> u32 {
        match self {
            Self::Ntsc => NTSC_FRAME_RATE_MILLIHERTZ,
            Self::Pal => PAL_FRAME_RATE_MILLIHERTZ,
        }
    }

    const fn code(self) -> u8 {
        match self {
            Self::Ntsc => 0,
            Self::Pal => 1,
        }
    }

    fn from_code(code: u8) -> Result<Self, SchedulerError> {
        match code {
            0 => Ok(Self::Ntsc),
            1 => Ok(Self::Pal),
            other => Err(SchedulerError::InvalidVideoStandard { value: other }),
        }
    }
}

impl fmt::Display for VideoStandard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Ntsc => "NTSC",
            Self::Pal => "PAL",
        })
    }
}

/// An interrupt source modelled by the scheduler.
///
/// The discriminants are the 68000 interrupt levels driven by the VDP.
#[allow(clippy::enum_variant_names)] // These are the hardware names for both sources.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum InterruptLevel {
    /// Horizontal blank interrupt, 68000 level 4.
    HBlank = 4,
    /// Vertical blank interrupt, 68000 level 6.
    VBlank = 6,
}

impl InterruptLevel {
    /// Every scheduler level in ascending 68000 priority order.
    pub const ALL: [Self; 2] = [Self::HBlank, Self::VBlank];

    /// Returns the 68000 interrupt level as an explicit numeric value.
    #[must_use]
    pub const fn level(self) -> u8 {
        self as u8
    }

    /// Converts an explicit 68000 interrupt level into a scheduler level.
    #[must_use]
    pub const fn from_level(level: u8) -> Option<Self> {
        match level {
            HBLANK_INTERRUPT_LEVEL => Some(Self::HBlank),
            VBLANK_INTERRUPT_LEVEL => Some(Self::VBlank),
            _ => None,
        }
    }

    const fn index(self) -> usize {
        match self {
            Self::HBlank => 0,
            Self::VBlank => 1,
        }
    }

    const fn snapshot_field(self) -> &'static str {
        match self {
            Self::HBlank => "pending_hblank_tick",
            Self::VBlank => "pending_vblank_tick",
        }
    }
}

impl fmt::Display for InterruptLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::HBlank => "HBlank",
            Self::VBlank => "VBlank",
        })
    }
}

/// Video timing and instruction-quantum configuration for the scheduler.
///
/// The configuration is fixed for the lifetime of a scheduler instance so that a
/// snapshot cannot silently change timing. Construct a new scheduler to change it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SchedulerConfig {
    standard: VideoStandard,
    instructions_per_line: u32,
}

impl SchedulerConfig {
    /// Creates a configuration with an explicit instruction quantum per scanline.
    ///
    /// # Errors
    ///
    /// Returns [`SchedulerError::InvalidInstructionsPerLine`] when
    /// `instructions_per_line` is zero, which would make line timing undefined.
    pub fn new(
        standard: VideoStandard,
        instructions_per_line: u32,
    ) -> Result<Self, SchedulerError> {
        if instructions_per_line == 0 {
            return Err(SchedulerError::InvalidInstructionsPerLine {
                value: instructions_per_line,
            });
        }
        Ok(Self {
            standard,
            instructions_per_line,
        })
    }

    /// Returns the default NTSC configuration.
    #[must_use]
    pub const fn ntsc() -> Self {
        Self {
            standard: VideoStandard::Ntsc,
            instructions_per_line: DEFAULT_INSTRUCTIONS_PER_LINE,
        }
    }

    /// Returns the default PAL configuration.
    #[must_use]
    pub const fn pal() -> Self {
        Self {
            standard: VideoStandard::Pal,
            instructions_per_line: DEFAULT_INSTRUCTIONS_PER_LINE,
        }
    }

    /// Returns the configured video standard.
    #[must_use]
    pub const fn standard(self) -> VideoStandard {
        self.standard
    }

    /// Returns the number of instruction quanta assigned to one scanline.
    #[must_use]
    pub const fn instructions_per_line(self) -> u32 {
        self.instructions_per_line
    }

    /// Returns the scanlines per frame for the configured standard.
    #[must_use]
    pub const fn lines_per_frame(self) -> u32 {
        self.standard.lines_per_frame()
    }

    /// Returns the active display scanlines for the configured standard.
    #[must_use]
    pub const fn active_lines(self) -> u32 {
        self.standard.active_lines()
    }

    /// Returns the nominal frame rate in millihertz for the configured standard.
    #[must_use]
    pub const fn frame_rate_millihz(self) -> u32 {
        self.standard.nominal_frame_rate_millihz()
    }
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self::ntsc()
    }
}

/// One outstanding interrupt request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PendingInterrupt {
    level: InterruptLevel,
    issued_tick: u64,
}

impl PendingInterrupt {
    /// Returns the requested 68000 interrupt level.
    #[must_use]
    pub const fn level(self) -> InterruptLevel {
        self.level
    }

    /// Returns the absolute instruction-quantum tick when the request was raised.
    #[must_use]
    pub const fn issued_tick(self) -> u64 {
        self.issued_tick
    }
}

/// A scheduler configuration or snapshot failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SchedulerError {
    /// The instruction quantum per scanline was zero.
    InvalidInstructionsPerLine {
        /// Rejected value.
        value: u32,
    },
    /// Advancing the virtual clock would overflow `u64`.
    TickOverflow,
    /// The snapshot does not start with the scheduler magic.
    SnapshotMagicMismatch,
    /// The snapshot version is newer or unknown.
    UnsupportedSnapshotVersion {
        /// Version found in the payload.
        found: u16,
    },
    /// The snapshot's video-standard tag is not defined.
    InvalidVideoStandard {
        /// Rejected tag.
        value: u8,
    },
    /// The snapshot was produced for a different video standard.
    SnapshotStandardMismatch {
        /// Standard required by this scheduler.
        expected: VideoStandard,
        /// Standard encoded in the snapshot.
        found: VideoStandard,
    },
    /// The snapshot was produced with a different instruction quantum per line.
    SnapshotInstructionsPerLineMismatch {
        /// Value required by this scheduler.
        expected: u32,
        /// Value encoded in the snapshot.
        found: u32,
    },
    /// The snapshot ended in the middle of a field.
    SnapshotTruncated {
        /// Field that could not be read completely.
        field: &'static str,
    },
    /// The snapshot contains bytes after the last expected field.
    SnapshotTrailingBytes {
        /// Number of unexpected trailing bytes.
        count: usize,
    },
    /// The pending mask contains bits with no defined interrupt level.
    InvalidPendingMask {
        /// Rejected mask.
        value: u8,
    },
    /// A pending request claims to have been raised in the future.
    SnapshotPendingInFuture {
        /// Interrupt level of the pending request.
        level: InterruptLevel,
        /// Tick encoded in the snapshot.
        issued_tick: u64,
        /// Current scheduler tick used for validation.
        ticks: u64,
    },
    /// A pending request does not lie on a scanline boundary.
    SnapshotPendingMisaligned {
        /// Interrupt level of the pending request.
        level: InterruptLevel,
        /// Tick encoded in the snapshot.
        issued_tick: u64,
    },
}

impl fmt::Display for SchedulerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInstructionsPerLine { value } => {
                write!(f, "instructions per line must be non-zero, got {value}")
            }
            Self::TickOverflow => f.write_str("interrupt scheduler tick overflow"),
            Self::SnapshotMagicMismatch => {
                f.write_str("interrupt scheduler snapshot magic mismatch")
            }
            Self::UnsupportedSnapshotVersion { found } => {
                write!(
                    f,
                    "interrupt scheduler snapshot version {found} is not supported"
                )
            }
            Self::InvalidVideoStandard { value } => {
                write!(
                    f,
                    "interrupt scheduler snapshot has unknown video standard {value}"
                )
            }
            Self::SnapshotStandardMismatch { expected, found } => write!(
                f,
                "interrupt scheduler snapshot is {found}, scheduler is configured for {expected}"
            ),
            Self::SnapshotInstructionsPerLineMismatch { expected, found } => write!(
                f,
                "interrupt scheduler snapshot uses {found} instructions per line, expected {expected}"
            ),
            Self::SnapshotTruncated { field } => {
                write!(
                    f,
                    "interrupt scheduler snapshot is truncated reading {field}"
                )
            }
            Self::SnapshotTrailingBytes { count } => {
                write!(
                    f,
                    "interrupt scheduler snapshot has {count} trailing byte(s)"
                )
            }
            Self::InvalidPendingMask { value } => {
                write!(
                    f,
                    "interrupt scheduler snapshot pending mask 0x{value:02X} is invalid"
                )
            }
            Self::SnapshotPendingInFuture {
                level,
                issued_tick,
                ticks,
            } => write!(
                f,
                "interrupt scheduler snapshot has {level} pending at tick {issued_tick}, \
                 after current tick {ticks}"
            ),
            Self::SnapshotPendingMisaligned { level, issued_tick } => write!(
                f,
                "interrupt scheduler snapshot has {level} pending at unaligned tick {issued_tick}"
            ),
        }
    }
}

impl Error for SchedulerError {}

/// Deterministic VBlank/HBlank scheduler over an instruction-quantum clock.
///
/// The clock starts at tick 0 on line 0 of frame 0, at the beginning of the
/// active display. See the module documentation for the timing model and
/// pending-request semantics.
#[derive(Clone, Debug)]
pub struct InterruptScheduler {
    config: SchedulerConfig,
    ticks: u64,
    pending: [Option<PendingInterrupt>; 2],
    issued: [u64; 2],
    coalesced: [u64; 2],
}

impl InterruptScheduler {
    /// Creates a scheduler at tick 0 with no pending requests.
    #[must_use]
    pub const fn new(config: SchedulerConfig) -> Self {
        Self {
            config,
            ticks: 0,
            pending: [None; 2],
            issued: [0; 2],
            coalesced: [0; 2],
        }
    }

    /// Returns the fixed configuration.
    #[must_use]
    pub const fn config(&self) -> SchedulerConfig {
        self.config
    }

    /// Returns the current virtual tick (completed instruction quanta).
    #[must_use]
    pub const fn ticks(&self) -> u64 {
        self.ticks
    }

    /// Returns the completed frame count.
    #[must_use]
    pub const fn frame(&self) -> u64 {
        self.completed_lines() / self.config.standard.lines_per_frame() as u64
    }

    /// Returns the scanline currently executing inside the frame.
    ///
    /// Line 0 is the first active line. The value reaches
    /// [`SchedulerConfig::active_lines`] when vertical blanking begins and stays
    /// below [`SchedulerConfig::lines_per_frame`] until the next frame starts.
    #[must_use]
    pub const fn line_in_frame(&self) -> u32 {
        (self.completed_lines() % self.config.standard.lines_per_frame() as u64) as u32
    }

    /// Returns the instruction quanta already executed inside the current line.
    #[must_use]
    pub const fn quantum_in_line(&self) -> u32 {
        (self.ticks % self.config.instructions_per_line as u64) as u32
    }

    /// Advances the clock by one instruction quantum.
    ///
    /// # Errors
    ///
    /// Returns [`SchedulerError::TickOverflow`] if the virtual clock would exceed
    /// `u64::MAX`.
    pub fn tick(&mut self) -> Result<(), SchedulerError> {
        self.advance(1)
    }

    /// Advances the clock by `quanta` instruction quanta, raising every scanline
    /// event crossed by the interval in deterministic order.
    ///
    /// Splitting the same total into different `advance`/`tick` chunk sizes
    /// produces identical scheduler state. Cost is proportional to the number of
    /// completed scanlines.
    ///
    /// # Errors
    ///
    /// Returns [`SchedulerError::TickOverflow`] if the virtual clock would exceed
    /// `u64::MAX`. On error no clock or event state is changed.
    pub fn advance(&mut self, quanta: u64) -> Result<(), SchedulerError> {
        if quanta == 0 {
            return Ok(());
        }
        let new_ticks = self
            .ticks
            .checked_add(quanta)
            .ok_or(SchedulerError::TickOverflow)?;
        let instructions_per_line = u64::from(self.config.instructions_per_line);
        let lines_per_frame = u64::from(self.config.standard.lines_per_frame());
        let active_lines = u64::from(self.config.standard.active_lines());
        let first_completed_line = self.ticks / instructions_per_line;
        let completed_line_limit = new_ticks / instructions_per_line;

        for completed_line in first_completed_line..completed_line_limit {
            let event_tick = (completed_line + 1) * instructions_per_line;
            let line = completed_line % lines_per_frame;
            if line < active_lines {
                self.record(InterruptLevel::HBlank, event_tick);
            }
            if line + 1 == active_lines {
                self.record(InterruptLevel::VBlank, event_tick);
            }
        }
        self.ticks = new_ticks;
        Ok(())
    }

    /// Returns whether any interrupt request is pending.
    #[must_use]
    pub fn has_pending(&self) -> bool {
        self.pending.iter().any(Option::is_some)
    }

    /// Returns whether the given level has a pending request.
    #[must_use]
    pub fn is_pending(&self, level: InterruptLevel) -> bool {
        self.pending[level.index()].is_some()
    }

    /// Returns the number of pending requests (zero, one or two).
    #[must_use]
    pub fn pending_len(&self) -> usize {
        self.pending.iter().filter(|slot| slot.is_some()).count()
    }

    /// Iterates pending requests in 68000 delivery-priority order.
    ///
    /// VBlank (level 6) is yielded before HBlank (level 4). At most one request
    /// per level exists, so this order is total and deterministic.
    pub fn pending_events(&self) -> impl Iterator<Item = PendingInterrupt> + '_ {
        InterruptLevel::ALL
            .into_iter()
            .rev()
            .filter_map(|level| self.pending[level.index()])
    }

    /// Returns the highest-priority pending request without acknowledging it.
    #[must_use]
    pub fn next_pending(&self) -> Option<InterruptLevel> {
        self.pending_events().next().map(|event| event.level())
    }

    /// Returns the highest-priority pending request the 68000 would take with
    /// `status_register` in `SR`.
    ///
    /// This is the scheduler-facing view of
    /// [`interrupt_level_deliverable`],
    /// the single source of truth for the 68000 rule: level 7 is always
    /// deliverable, and every other level must be strictly greater than the
    /// interrupt-priority mask `(status_register >> 8) & 7`. Only the IPM bits
    /// of the status register are consulted. This method does not acknowledge
    /// anything.
    #[must_use]
    pub fn next_deliverable(&self, status_register: u16) -> Option<InterruptLevel> {
        self.pending_events()
            .find(|event| interrupt_level_deliverable(event.level().level(), status_register))
            .map(|event| event.level())
    }

    /// Acknowledges and clears the pending request for `level`.
    ///
    /// Returns `false` when no request of that level was pending.
    pub fn acknowledge(&mut self, level: InterruptLevel) -> bool {
        self.pending[level.index()].take().is_some()
    }

    /// Acknowledges and returns the highest-priority pending request.
    pub fn acknowledge_next(&mut self) -> Option<InterruptLevel> {
        let event = self.pending_events().next()?;
        let level = event.level();
        self.pending[level.index()] = None;
        Some(level)
    }

    /// Returns how many requests of `level` were raised since creation or restore.
    ///
    /// This diagnostic counter saturates at `u64::MAX`.
    #[must_use]
    pub fn requests_issued(&self, level: InterruptLevel) -> u64 {
        self.issued[level.index()]
    }

    /// Returns how many requests of `level` were coalesced into an already
    /// pending request since creation or restore.
    ///
    /// This diagnostic counter saturates at `u64::MAX`.
    #[must_use]
    pub fn requests_coalesced(&self, level: InterruptLevel) -> u64 {
        self.coalesced[level.index()]
    }

    /// Serializes the complete scheduler state into the versioned snapshot
    /// payload documented at the module level.
    #[must_use]
    pub fn snapshot_bytes(&self) -> Vec<u8> {
        let pending_len = self.pending_len();
        let mut bytes = Vec::with_capacity(SNAPSHOT_HEADER_LEN + 8 * pending_len);
        bytes.extend_from_slice(&SNAPSHOT_MAGIC);
        bytes.extend_from_slice(&SNAPSHOT_VERSION.to_le_bytes());
        bytes.push(self.config.standard.code());
        bytes.extend_from_slice(&self.config.instructions_per_line.to_le_bytes());
        bytes.extend_from_slice(&self.ticks.to_le_bytes());
        for level in InterruptLevel::ALL {
            bytes.extend_from_slice(&self.issued[level.index()].to_le_bytes());
        }
        for level in InterruptLevel::ALL {
            bytes.extend_from_slice(&self.coalesced[level.index()].to_le_bytes());
        }
        let mut mask = 0_u8;
        for level in InterruptLevel::ALL {
            if self.pending[level.index()].is_some() {
                mask |= 1_u8 << level.index();
            }
        }
        bytes.push(mask);
        for level in InterruptLevel::ALL {
            if let Some(event) = self.pending[level.index()] {
                bytes.extend_from_slice(&event.issued_tick.to_le_bytes());
            }
        }
        bytes
    }

    /// Restores scheduler state from [`Self::snapshot_bytes`] output.
    ///
    /// The snapshot must have been produced by a scheduler with the same
    /// [`SchedulerConfig`]. Validation completes before any state is changed, so
    /// an error leaves the scheduler exactly as it was.
    ///
    /// # Errors
    ///
    /// Returns a [`SchedulerError`] for unknown versions, configuration
    /// mismatches, malformed pending fields, truncation or trailing bytes.
    pub fn restore_bytes(&mut self, bytes: &[u8]) -> Result<(), SchedulerError> {
        let parsed = parse_snapshot(self.config, bytes)?;
        self.ticks = parsed.ticks;
        self.issued = parsed.issued;
        self.coalesced = parsed.coalesced;
        self.pending = parsed.pending;
        Ok(())
    }

    const fn completed_lines(&self) -> u64 {
        self.ticks / self.config.instructions_per_line as u64
    }

    fn record(&mut self, level: InterruptLevel, issued_tick: u64) {
        let index = level.index();
        self.issued[index] = self.issued[index].saturating_add(1);
        if self.pending[index].is_some() {
            self.coalesced[index] = self.coalesced[index].saturating_add(1);
        } else {
            self.pending[index] = Some(PendingInterrupt { level, issued_tick });
        }
    }
}

#[derive(Debug)]
struct SnapshotState {
    ticks: u64,
    issued: [u64; 2],
    coalesced: [u64; 2],
    pending: [Option<PendingInterrupt>; 2],
}

fn parse_snapshot(config: SchedulerConfig, bytes: &[u8]) -> Result<SnapshotState, SchedulerError> {
    let mut reader = SnapshotReader::new(bytes);
    if reader.read_array::<8>("magic")? != SNAPSHOT_MAGIC {
        return Err(SchedulerError::SnapshotMagicMismatch);
    }
    let version = reader.read_u16("version")?;
    if version != SNAPSHOT_VERSION {
        return Err(SchedulerError::UnsupportedSnapshotVersion { found: version });
    }
    let standard = VideoStandard::from_code(reader.read_u8("video_standard")?)?;
    let instructions_per_line = reader.read_u32("instructions_per_line")?;
    if instructions_per_line == 0 {
        return Err(SchedulerError::InvalidInstructionsPerLine {
            value: instructions_per_line,
        });
    }
    if standard != config.standard() {
        return Err(SchedulerError::SnapshotStandardMismatch {
            expected: config.standard(),
            found: standard,
        });
    }
    if instructions_per_line != config.instructions_per_line() {
        return Err(SchedulerError::SnapshotInstructionsPerLineMismatch {
            expected: config.instructions_per_line(),
            found: instructions_per_line,
        });
    }

    let ticks = reader.read_u64("ticks")?;
    let mut issued = [0_u64; 2];
    for counter in &mut issued {
        *counter = reader.read_u64("requests_issued")?;
    }
    let mut coalesced = [0_u64; 2];
    for counter in &mut coalesced {
        *counter = reader.read_u64("requests_coalesced")?;
    }

    let pending_mask = reader.read_u8("pending_mask")?;
    if pending_mask & !PENDING_MASK_ALL != 0 {
        return Err(SchedulerError::InvalidPendingMask {
            value: pending_mask,
        });
    }
    let mut pending = [None; 2];
    for level in InterruptLevel::ALL {
        let index = level.index();
        if pending_mask & (1_u8 << index) == 0 {
            continue;
        }
        let issued_tick = reader.read_u64(level.snapshot_field())?;
        if issued_tick > ticks {
            return Err(SchedulerError::SnapshotPendingInFuture {
                level,
                issued_tick,
                ticks,
            });
        }
        if issued_tick % u64::from(instructions_per_line) != 0 {
            return Err(SchedulerError::SnapshotPendingMisaligned { level, issued_tick });
        }
        pending[index] = Some(PendingInterrupt { level, issued_tick });
    }

    let trailing = reader.remaining();
    if trailing != 0 {
        return Err(SchedulerError::SnapshotTrailingBytes { count: trailing });
    }
    Ok(SnapshotState {
        ticks,
        issued,
        coalesced,
        pending,
    })
}

struct SnapshotReader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> SnapshotReader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    const fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.position)
    }

    fn read_exact(
        &mut self,
        field: &'static str,
        length: usize,
    ) -> Result<&'a [u8], SchedulerError> {
        let end = self
            .position
            .checked_add(length)
            .ok_or(SchedulerError::SnapshotTruncated { field })?;
        if end > self.bytes.len() {
            return Err(SchedulerError::SnapshotTruncated { field });
        }
        let slice = &self.bytes[self.position..end];
        self.position = end;
        Ok(slice)
    }

    fn read_array<const N: usize>(
        &mut self,
        field: &'static str,
    ) -> Result<[u8; N], SchedulerError> {
        let slice = self.read_exact(field, N)?;
        let mut output = [0_u8; N];
        output.copy_from_slice(slice);
        Ok(output)
    }

    fn read_u8(&mut self, field: &'static str) -> Result<u8, SchedulerError> {
        Ok(self.read_array::<1>(field)?[0])
    }

    fn read_u16(&mut self, field: &'static str) -> Result<u16, SchedulerError> {
        Ok(u16::from_le_bytes(self.read_array::<2>(field)?))
    }

    fn read_u32(&mut self, field: &'static str) -> Result<u32, SchedulerError> {
        Ok(u32::from_le_bytes(self.read_array::<4>(field)?))
    }

    fn read_u64(&mut self, field: &'static str) -> Result<u64, SchedulerError> {
        Ok(u64::from_le_bytes(self.read_array::<8>(field)?))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_INSTRUCTIONS_PER_LINE, HBLANK_INTERRUPT_LEVEL, InterruptLevel, InterruptScheduler,
        NTSC_ACTIVE_LINES, NTSC_LINES_PER_FRAME, PendingInterrupt, SchedulerConfig, SchedulerError,
        VBLANK_INTERRUPT_LEVEL, VideoStandard,
    };
    use crate::cpu::interrupt_level_deliverable;

    fn scheduler(instructions_per_line: u32) -> InterruptScheduler {
        let config = SchedulerConfig::new(VideoStandard::Ntsc, instructions_per_line)
            .expect("positive instruction quantum");
        InterruptScheduler::new(config)
    }

    fn hblank() -> InterruptLevel {
        InterruptLevel::HBlank
    }

    fn vblank() -> InterruptLevel {
        InterruptLevel::VBlank
    }

    fn pending(scheduler: &InterruptScheduler) -> Vec<PendingInterrupt> {
        scheduler.pending_events().collect()
    }

    #[test]
    fn configuration_and_levels_are_explicit() {
        assert_eq!(VBLANK_INTERRUPT_LEVEL, 6);
        assert_eq!(HBLANK_INTERRUPT_LEVEL, 4);
        assert_eq!(InterruptLevel::VBlank.level(), 6);
        assert_eq!(InterruptLevel::HBlank.level(), 4);
        assert_eq!(InterruptLevel::from_level(6), Some(vblank()));
        assert_eq!(InterruptLevel::from_level(4), Some(hblank()));
        assert_eq!(InterruptLevel::from_level(5), None);
        assert_eq!(InterruptLevel::from_level(0), None);

        assert_eq!(VideoStandard::Ntsc.lines_per_frame(), 262);
        assert_eq!(VideoStandard::Ntsc.active_lines(), 224);
        assert_eq!(VideoStandard::Ntsc.vertical_blank_lines(), 38);
        assert_eq!(VideoStandard::Ntsc.nominal_frame_rate_millihz(), 59_920);
        assert_eq!(VideoStandard::Pal.lines_per_frame(), 313);
        assert_eq!(VideoStandard::Pal.active_lines(), 240);
        assert_eq!(VideoStandard::Pal.vertical_blank_lines(), 73);
        assert_eq!(VideoStandard::Pal.nominal_frame_rate_millihz(), 49_701);

        assert_eq!(SchedulerConfig::default(), SchedulerConfig::ntsc());
        assert_eq!(
            SchedulerConfig::ntsc().instructions_per_line(),
            DEFAULT_INSTRUCTIONS_PER_LINE
        );
        assert_eq!(SchedulerConfig::ntsc().lines_per_frame(), 262);
        assert_eq!(SchedulerConfig::pal().active_lines(), 240);
        assert_eq!(SchedulerConfig::pal().frame_rate_millihz(), 49_701);
        assert_eq!(
            SchedulerConfig::new(VideoStandard::Ntsc, 0),
            Err(SchedulerError::InvalidInstructionsPerLine { value: 0 })
        );
        let custom = SchedulerConfig::new(VideoStandard::Pal, 7).expect("valid config");
        assert_eq!(custom.standard(), VideoStandard::Pal);
        assert_eq!(custom.instructions_per_line(), 7);
        assert_eq!(custom.lines_per_frame(), 313);
    }

    #[test]
    fn new_scheduler_starts_at_frame_zero_line_zero() {
        let scheduler = scheduler(4);
        assert_eq!(scheduler.ticks(), 0);
        assert_eq!(scheduler.frame(), 0);
        assert_eq!(scheduler.line_in_frame(), 0);
        assert_eq!(scheduler.quantum_in_line(), 0);
        assert!(!scheduler.has_pending());
        assert_eq!(scheduler.pending_len(), 0);
        assert_eq!(scheduler.next_pending(), None);
        assert_eq!(scheduler.snapshot_bytes().len(), 56);
    }

    #[test]
    fn hblank_and_vblank_fire_on_scanline_boundaries() {
        let mut scheduler = scheduler(4);
        for _ in 0..3 {
            scheduler.tick().expect("tick");
        }
        assert_eq!(scheduler.ticks(), 3);
        assert_eq!(scheduler.line_in_frame(), 0);
        assert_eq!(scheduler.quantum_in_line(), 3);
        assert!(!scheduler.has_pending());

        scheduler.tick().expect("tick");
        assert_eq!(scheduler.ticks(), 4);
        assert_eq!(scheduler.line_in_frame(), 1);
        assert_eq!(scheduler.quantum_in_line(), 0);
        assert!(scheduler.is_pending(hblank()));
        assert!(!scheduler.is_pending(vblank()));
        assert_eq!(scheduler.requests_issued(hblank()), 1);
        assert_eq!(scheduler.requests_coalesced(hblank()), 0);
        let first = pending(&scheduler);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].level(), hblank());
        assert_eq!(first[0].issued_tick(), 4);

        let vblank_tick = u64::from(NTSC_ACTIVE_LINES) * 4;
        scheduler
            .advance(vblank_tick - scheduler.ticks())
            .expect("advance to vertical blank");
        assert_eq!(scheduler.ticks(), vblank_tick);
        assert_eq!(scheduler.frame(), 0);
        assert_eq!(scheduler.line_in_frame(), NTSC_ACTIVE_LINES);
        assert_eq!(scheduler.requests_issued(hblank()), 224);
        assert_eq!(scheduler.requests_issued(vblank()), 1);

        let both = pending(&scheduler);
        assert_eq!(both.len(), 2);
        assert_eq!(both[0].level(), vblank());
        assert_eq!(both[0].issued_tick(), vblank_tick);
        assert_eq!(both[1].level(), hblank());
        assert_eq!(both[1].issued_tick(), 4);
        assert_eq!(scheduler.next_pending(), Some(vblank()));
        assert!(scheduler.is_pending(vblank()));
        assert!(scheduler.is_pending(hblank()));
    }

    #[test]
    fn blanking_lines_are_quiet_and_frames_roll_over() {
        let mut scheduler = scheduler(4);
        let frame_ticks = u64::from(NTSC_LINES_PER_FRAME) * 4;
        scheduler.advance(frame_ticks).expect("advance one frame");
        assert_eq!(scheduler.ticks(), frame_ticks);
        assert_eq!(scheduler.frame(), 1);
        assert_eq!(scheduler.line_in_frame(), 0);
        assert_eq!(scheduler.quantum_in_line(), 0);
        assert_eq!(scheduler.requests_issued(hblank()), 224);
        assert_eq!(scheduler.requests_issued(vblank()), 1);
        assert_eq!(scheduler.pending_len(), 2);
        assert_eq!(scheduler.requests_coalesced(hblank()), 223);
        assert_eq!(scheduler.requests_coalesced(vblank()), 0);

        scheduler
            .advance(4)
            .expect("advance first line of next frame");
        assert_eq!(scheduler.frame(), 1);
        assert_eq!(scheduler.line_in_frame(), 1);
        assert_eq!(scheduler.requests_issued(hblank()), 225);
        assert_eq!(scheduler.requests_issued(vblank()), 1);
        assert_eq!(pending(&scheduler)[1].issued_tick(), 4);
    }

    #[test]
    fn multiple_pending_requests_coalesce_and_acknowledge() {
        let mut scheduler = scheduler(4);
        scheduler.advance(8).expect("advance two lines");
        assert_eq!(scheduler.requests_issued(hblank()), 2);
        assert_eq!(scheduler.requests_coalesced(hblank()), 1);
        assert_eq!(scheduler.pending_len(), 1);
        assert_eq!(pending(&scheduler)[0].issued_tick(), 4);

        assert!(scheduler.acknowledge(hblank()));
        assert!(!scheduler.acknowledge(hblank()));
        assert!(!scheduler.is_pending(hblank()));
        assert!(!scheduler.has_pending());

        let vblank_tick = u64::from(NTSC_ACTIVE_LINES) * 4;
        scheduler
            .advance(vblank_tick - scheduler.ticks())
            .expect("advance to vertical blank");
        assert!(scheduler.is_pending(hblank()));
        assert!(scheduler.is_pending(vblank()));
        assert_eq!(scheduler.acknowledge_next(), Some(vblank()));
        assert!(!scheduler.is_pending(vblank()));
        assert_eq!(scheduler.acknowledge_next(), Some(hblank()));
        assert_eq!(scheduler.acknowledge_next(), None);
        assert!(!scheduler.has_pending());
    }

    #[test]
    fn delivery_view_respects_the_68000_mask() {
        let mut scheduler = scheduler(4);
        let vblank_tick = u64::from(NTSC_ACTIVE_LINES) * 4;
        scheduler
            .advance(vblank_tick)
            .expect("advance to vertical blank");
        assert_eq!(scheduler.next_pending(), Some(vblank()));
        assert_eq!(scheduler.next_deliverable(0x2700), None);
        assert_eq!(scheduler.next_deliverable(0x2600), None);
        assert_eq!(scheduler.next_deliverable(0x2500), Some(vblank()));
        assert_eq!(scheduler.next_deliverable(0x2300), Some(vblank()));
        assert!(scheduler.acknowledge(vblank()));
        assert_eq!(scheduler.next_deliverable(0x2400), None);
        assert_eq!(scheduler.next_deliverable(0x2300), Some(hblank()));
    }

    #[test]
    fn delivery_view_applies_the_level_seven_rule() {
        let mut scheduler = scheduler(4);
        scheduler
            .advance(u64::from(NTSC_ACTIVE_LINES) * 4)
            .expect("advance to vertical blank");
        // The VDP only drives levels 6 and 4, so mask 7 delivers nothing even
        // though level 7 itself would always be taken. The rule is visible in
        // the shared predicate and in the CPU view of it.
        assert!(!interrupt_level_deliverable(VBLANK_INTERRUPT_LEVEL, 0x2700));
        assert!(!interrupt_level_deliverable(HBLANK_INTERRUPT_LEVEL, 0x2700));
        assert!(interrupt_level_deliverable(7, 0x2700));
        assert!(interrupt_level_deliverable(7, 0x0700));
        assert_eq!(scheduler.next_deliverable(0x2700), None);
    }

    #[test]
    fn scheduler_and_cpu_agree_on_the_shared_delivery_predicate() {
        let mut scheduler = scheduler(4);
        scheduler
            .advance(u64::from(NTSC_ACTIVE_LINES) * 4)
            .expect("advance to vertical blank");
        assert_eq!(scheduler.pending_len(), 2, "VBlank and HBlank pending");

        let mut cpu = crate::cpu::M68000::new();
        for status_register in [0x2700_u16, 0x2600, 0x2500, 0x2400, 0x2300, 0x2000, 0x0000] {
            let expected = InterruptLevel::ALL
                .into_iter()
                .rev()
                .find(|level| interrupt_level_deliverable(level.level(), status_register));
            assert_eq!(
                scheduler.next_deliverable(status_register),
                expected,
                "scheduler must reuse the CPU predicate for SR 0x{status_register:04X}"
            );
            // The CPU-facing view applies exactly the same predicate to the
            // same raw SR value.
            cpu.restore_state([0; 8], [0; 8], 0, status_register, false);
            assert_eq!(
                cpu.accepts_interrupt_level(VBLANK_INTERRUPT_LEVEL),
                interrupt_level_deliverable(VBLANK_INTERRUPT_LEVEL, status_register)
            );
            assert_eq!(
                cpu.accepts_interrupt_level(HBLANK_INTERRUPT_LEVEL),
                interrupt_level_deliverable(HBLANK_INTERRUPT_LEVEL, status_register)
            );
        }
    }

    #[test]
    fn event_generation_is_independent_of_step_size() {
        let mut single = scheduler(4);
        single.advance(1_048).expect("single advance");

        let mut ticked = scheduler(4);
        for _ in 0..1_048 {
            ticked.tick().expect("tick");
        }

        let mut chunked = scheduler(4);
        for step in [1_u64, 3, 7, 13, 100, 924] {
            chunked.advance(step).expect("chunked advance");
        }

        assert_eq!(single.snapshot_bytes(), ticked.snapshot_bytes());
        assert_eq!(single.snapshot_bytes(), chunked.snapshot_bytes());
        assert_eq!(single.requests_issued(hblank()), 224);
        assert_eq!(single.requests_issued(vblank()), 1);
        assert_eq!(single.frame(), 1);
        assert_eq!(single.line_in_frame(), 0);
        assert_eq!(pending(&single), pending(&ticked));
        assert_eq!(pending(&single), pending(&chunked));
    }

    #[test]
    fn zero_quanta_advance_is_a_no_op() {
        let mut scheduler = scheduler(4);
        scheduler.advance(0).expect("zero advance");
        assert_eq!(scheduler.ticks(), 0);
        assert_eq!(scheduler.snapshot_bytes().len(), 56);
    }

    #[test]
    fn snapshot_round_trip_preserves_all_scheduler_state() {
        let mut original = scheduler(4);
        let vblank_tick = u64::from(NTSC_ACTIVE_LINES) * 4;
        original.advance(vblank_tick + 4).expect("advance");
        let bytes = original.snapshot_bytes();
        assert_eq!(bytes.len(), 56 + 16);

        let mut restored = scheduler(4);
        restored.restore_bytes(&bytes).expect("restore");
        assert_eq!(restored.snapshot_bytes(), bytes);
        assert_eq!(restored.ticks(), original.ticks());
        assert_eq!(restored.frame(), original.frame());
        assert_eq!(restored.line_in_frame(), original.line_in_frame());
        assert_eq!(restored.quantum_in_line(), original.quantum_in_line());
        assert_eq!(restored.config(), original.config());
        for level in InterruptLevel::ALL {
            assert_eq!(
                restored.requests_issued(level),
                original.requests_issued(level)
            );
            assert_eq!(
                restored.requests_coalesced(level),
                original.requests_coalesced(level)
            );
        }
        assert_eq!(pending(&restored), pending(&original));

        original.tick().expect("original tick");
        restored.tick().expect("restored tick");
        assert_eq!(restored.snapshot_bytes(), original.snapshot_bytes());
    }

    #[test]
    fn snapshot_round_trip_with_only_vblank_pending() {
        let mut original = scheduler(4);
        let vblank_tick = u64::from(NTSC_ACTIVE_LINES) * 4;
        original.advance(vblank_tick).expect("advance");
        assert!(original.acknowledge(hblank()));
        let bytes = original.snapshot_bytes();
        assert_eq!(bytes.len(), 56 + 8);
        assert_eq!(bytes[55], 0b0000_0010);

        let mut restored = scheduler(4);
        restored.restore_bytes(&bytes).expect("restore");
        assert_eq!(restored.snapshot_bytes(), bytes);
        assert_eq!(restored.pending_len(), 1);
        let pending = pending(&restored);
        assert_eq!(pending[0].level(), vblank());
        assert_eq!(pending[0].issued_tick(), vblank_tick);
    }

    #[test]
    fn restore_rejects_incompatible_configuration() {
        let source = scheduler(4);
        let bytes = source.snapshot_bytes();

        let mut pal = InterruptScheduler::new(SchedulerConfig::pal());
        let before = pal.snapshot_bytes();
        assert_eq!(
            pal.restore_bytes(&bytes),
            Err(SchedulerError::SnapshotStandardMismatch {
                expected: VideoStandard::Pal,
                found: VideoStandard::Ntsc,
            })
        );
        assert_eq!(pal.snapshot_bytes(), before);

        let mut different_quantum = scheduler(8);
        let before = different_quantum.snapshot_bytes();
        assert_eq!(
            different_quantum.restore_bytes(&bytes),
            Err(SchedulerError::SnapshotInstructionsPerLineMismatch {
                expected: 8,
                found: 4,
            })
        );
        assert_eq!(different_quantum.snapshot_bytes(), before);
    }

    #[test]
    fn invalid_snapshots_are_rejected_without_mutating_state() {
        let mut scheduler = scheduler(4);
        scheduler.advance(8).expect("advance");
        let pristine = scheduler.snapshot_bytes();
        assert_eq!(pristine.len(), 64);

        let mut wrong_magic = pristine.clone();
        wrong_magic[0] = b'X';
        assert_eq!(
            scheduler.restore_bytes(&wrong_magic),
            Err(SchedulerError::SnapshotMagicMismatch)
        );
        assert_eq!(scheduler.snapshot_bytes(), pristine);

        let mut wrong_version = pristine.clone();
        wrong_version[8..10].copy_from_slice(&2_u16.to_le_bytes());
        assert_eq!(
            scheduler.restore_bytes(&wrong_version),
            Err(SchedulerError::UnsupportedSnapshotVersion { found: 2 })
        );
        assert_eq!(scheduler.snapshot_bytes(), pristine);

        let mut wrong_standard = pristine.clone();
        wrong_standard[10] = 9;
        assert_eq!(
            scheduler.restore_bytes(&wrong_standard),
            Err(SchedulerError::InvalidVideoStandard { value: 9 })
        );
        assert_eq!(scheduler.snapshot_bytes(), pristine);

        let mut zero_quantum = pristine.clone();
        zero_quantum[11..15].copy_from_slice(&0_u32.to_le_bytes());
        assert_eq!(
            scheduler.restore_bytes(&zero_quantum),
            Err(SchedulerError::InvalidInstructionsPerLine { value: 0 })
        );
        assert_eq!(scheduler.snapshot_bytes(), pristine);

        let mut bad_mask = pristine.clone();
        bad_mask[55] = 0x80;
        assert_eq!(
            scheduler.restore_bytes(&bad_mask),
            Err(SchedulerError::InvalidPendingMask { value: 0x80 })
        );
        assert_eq!(scheduler.snapshot_bytes(), pristine);

        assert_eq!(
            scheduler.restore_bytes(&pristine[..pristine.len() - 1]),
            Err(SchedulerError::SnapshotTruncated {
                field: "pending_hblank_tick",
            })
        );
        assert_eq!(scheduler.snapshot_bytes(), pristine);

        let mut trailing = pristine.clone();
        trailing.push(0);
        assert_eq!(
            scheduler.restore_bytes(&trailing),
            Err(SchedulerError::SnapshotTrailingBytes { count: 1 })
        );
        assert_eq!(scheduler.snapshot_bytes(), pristine);

        let mut future = pristine.clone();
        future[56..64].copy_from_slice(&12_u64.to_le_bytes());
        assert_eq!(
            scheduler.restore_bytes(&future),
            Err(SchedulerError::SnapshotPendingInFuture {
                level: hblank(),
                issued_tick: 12,
                ticks: 8,
            })
        );
        assert_eq!(scheduler.snapshot_bytes(), pristine);

        let mut misaligned = pristine.clone();
        misaligned[56..64].copy_from_slice(&6_u64.to_le_bytes());
        assert_eq!(
            scheduler.restore_bytes(&misaligned),
            Err(SchedulerError::SnapshotPendingMisaligned {
                level: hblank(),
                issued_tick: 6,
            })
        );
        assert_eq!(scheduler.snapshot_bytes(), pristine);

        assert_eq!(
            scheduler.restore_bytes(&[]),
            Err(SchedulerError::SnapshotTruncated { field: "magic" })
        );
        assert_eq!(scheduler.snapshot_bytes(), pristine);
    }

    #[test]
    fn tick_overflow_is_a_typed_error() {
        let mut scheduler = scheduler(4);
        let mut bytes = scheduler.snapshot_bytes();
        bytes[15..23].copy_from_slice(&u64::MAX.to_le_bytes());
        scheduler.restore_bytes(&bytes).expect("restore max tick");
        assert_eq!(scheduler.ticks(), u64::MAX);
        assert_eq!(scheduler.tick(), Err(SchedulerError::TickOverflow));
        assert_eq!(scheduler.advance(1), Err(SchedulerError::TickOverflow));
        assert_eq!(scheduler.ticks(), u64::MAX);
        assert_eq!(scheduler.snapshot_bytes(), bytes);
    }
}
