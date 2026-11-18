//! Deterministic VBlank/HBlank interrupt scheduler over the master-cycle clock.
//!
//! Canonical emulated time is an integer master-clock cycle. A scanline is
//! [`MASTER_CYCLES_PER_LINE`] master cycles, so the scheduler advances with
//! [`InterruptScheduler::advance_master`] by exactly the cycle cost the 68000
//! reports (`InstructionOutcome::cycles * 7`). The selected video standard
//! supplies the frame geometry.
//!
//! **This is not fully cycle-accurate hardware timing.** The VDP's real H/V
//! counters, 68000 bus contention, DMA stalls and prefetch are not modelled;
//! instruction cost is added at the instruction boundary, so a request can be
//! noticed at the end of the instruction that crossed the line rather than at
//! the exact internal microcycle. The model must not be cited as
//! hardware-accurate VDP timing.
//!
//! # Model
//!
//! - NTSC: 262 lines/frame, 224 active lines, 59.92 Hz.
//! - PAL: 313 lines/frame, 240 active lines, 49.70 Hz.
//! - An HBlank request is raised when each active scanline completes (at
//!   `(line + 1) * 3420` master cycles).
//! - A VBlank request is raised when the last active scanline completes.
//! - Vertical blanking lines produce no HBlank requests.
//!
//! # Pending and acknowledge semantics
//!
//! At most one request per interrupt level is retained. If another request of
//! the same level arrives while one is pending it is counted as coalesced: the
//! pending request keeps its original issued tick and the queue stays bounded.
//! Requests of different levels are independent. Delivery order is 68000
//! priority (VBlank level 6 before HBlank level 4). The scheduler does not
//! mutate the CPU: [`InterruptScheduler::next_deliverable`] applies
//! [`interrupt_level_deliverable`] to a supplied status register.
//!
//! # Snapshot payload
//!
//! [`InterruptScheduler::snapshot_bytes`] uses a fixed, versioned little-endian
//! layout: magic `CFINTSCH`, `u16` version 2, the video-standard byte, the
//! master-cycle clock as `u64`, one `u64` issued counter per level (HBlank then
//! VBlank), one `u64` coalesced counter per level (same order), a `u8` pending
//! mask (bit 0 HBlank, bit 1 VBlank), then one `u64` issued tick per set pending
//! bit in HBlank/VBlank order.

use std::error::Error;
use std::fmt;

use chronofork_core::codec::{CodecError, Reader, Writer};

use crate::cpu::interrupt_level_deliverable;
use crate::cycles::MASTER_CYCLES_PER_LINE;

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

const SNAPSHOT_MAGIC: [u8; 8] = *b"CFINTSCH";
const SNAPSHOT_VERSION: u16 = 2;
const SNAPSHOT_HEADER_LEN: usize = 52;
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

    /// Returns the one-byte tag serialized into snapshots and config hashes.
    #[must_use]
    pub const fn code(self) -> u8 {
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

/// Video timing configuration for the scheduler.
///
/// The configuration is fixed for the lifetime of a scheduler so a snapshot
/// cannot silently change timing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SchedulerConfig {
    standard: VideoStandard,
}

impl SchedulerConfig {
    /// Creates a configuration for `video`.
    #[must_use]
    pub const fn new(video: VideoStandard) -> Self {
        Self { standard: video }
    }

    /// Returns the default NTSC configuration.
    #[must_use]
    pub const fn ntsc() -> Self {
        Self::new(VideoStandard::Ntsc)
    }

    /// Returns the default PAL configuration.
    #[must_use]
    pub const fn pal() -> Self {
        Self::new(VideoStandard::Pal)
    }

    /// Returns the configured video standard.
    #[must_use]
    pub const fn standard(self) -> VideoStandard {
        self.standard
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

    /// Returns the master cycles in one scanline, from the shared time base.
    #[must_use]
    pub const fn master_cycles_per_line(self) -> u64 {
        MASTER_CYCLES_PER_LINE
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

    /// Returns the absolute master-cycle tick when the request was raised.
    #[must_use]
    pub const fn issued_tick(self) -> u64 {
        self.issued_tick
    }
}

/// A scheduler configuration or snapshot failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SchedulerError {
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

/// Deterministic VBlank/HBlank scheduler over the master-cycle clock.
///
/// The clock starts at tick 0 on line 0 of frame 0, at the beginning of the
/// active display.
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

    /// Returns the current master-cycle clock.
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
    #[must_use]
    pub const fn line_in_frame(&self) -> u32 {
        (self.completed_lines() % self.config.standard.lines_per_frame() as u64) as u32
    }

    /// Returns the master cycles already executed inside the current line.
    #[must_use]
    pub const fn master_cycle_in_line(&self) -> u64 {
        self.ticks % MASTER_CYCLES_PER_LINE
    }

    /// Advances the master-cycle clock, raising every scanline event crossed.
    ///
    /// Splitting the same total into different chunk sizes produces identical
    /// scheduler state. Cost is proportional to the number of completed lines.
    ///
    /// # Errors
    ///
    /// Returns [`SchedulerError::TickOverflow`] if the clock would exceed
    /// `u64::MAX`. On error no clock or event state is changed.
    pub fn advance_master(&mut self, cycles: u64) -> Result<(), SchedulerError> {
        if cycles == 0 {
            return Ok(());
        }
        let new_ticks = self
            .ticks
            .checked_add(cycles)
            .ok_or(SchedulerError::TickOverflow)?;
        let lines = u64::from(self.config.standard.lines_per_frame());
        let active = u64::from(self.config.standard.active_lines());
        let first_completed_line = self.ticks / MASTER_CYCLES_PER_LINE;
        let completed_line_limit = new_ticks / MASTER_CYCLES_PER_LINE;

        let (hblank_events, vblank_events) =
            event_counts(first_completed_line, completed_line_limit, lines, active);
        self.apply_events(
            InterruptLevel::HBlank,
            hblank_events,
            first_completed_line,
            completed_line_limit,
            lines,
            active,
        );
        self.apply_events(
            InterruptLevel::VBlank,
            vblank_events,
            first_completed_line,
            completed_line_limit,
            lines,
            active,
        );
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
    #[must_use]
    pub fn next_deliverable(&self, status_register: u16) -> Option<InterruptLevel> {
        self.pending_events()
            .find(|event| interrupt_level_deliverable(event.level().level(), status_register))
            .map(|event| event.level())
    }

    /// Acknowledges and clears the pending request for `level`.
    pub fn acknowledge(&mut self, level: InterruptLevel) -> bool {
        self.pending[level.index()].take().is_some()
    }

    /// Returns how many requests of `level` were raised since creation or restore.
    #[must_use]
    pub fn requests_issued(&self, level: InterruptLevel) -> u64 {
        self.issued[level.index()]
    }

    /// Returns how many requests of `level` were coalesced.
    #[must_use]
    pub fn requests_coalesced(&self, level: InterruptLevel) -> u64 {
        self.coalesced[level.index()]
    }

    /// Serializes the complete scheduler state into the versioned snapshot.
    #[must_use]
    pub fn snapshot_bytes(&self) -> Vec<u8> {
        let pending_len = self.pending_len();
        let mut bytes = Writer::with_capacity(SNAPSHOT_HEADER_LEN + 8 * pending_len);
        bytes.write_bytes(&SNAPSHOT_MAGIC);
        bytes.write_u16_le(SNAPSHOT_VERSION);
        bytes.write_u8(self.config.standard.code());
        bytes.write_u64_le(self.ticks);
        for level in InterruptLevel::ALL {
            bytes.write_u64_le(self.issued[level.index()]);
        }
        for level in InterruptLevel::ALL {
            bytes.write_u64_le(self.coalesced[level.index()]);
        }
        let mut mask = 0_u8;
        for level in InterruptLevel::ALL {
            if self.pending[level.index()].is_some() {
                mask |= 1_u8 << level.index();
            }
        }
        bytes.write_u8(mask);
        for level in InterruptLevel::ALL {
            if let Some(event) = self.pending[level.index()] {
                bytes.write_u64_le(event.issued_tick);
            }
        }
        bytes.into_bytes()
    }

    /// Restores scheduler state from [`Self::snapshot_bytes`] output.
    ///
    /// Validation completes before any state is changed, so an error leaves the
    /// scheduler exactly as it was.
    pub fn restore_bytes(&mut self, bytes: &[u8]) -> Result<(), SchedulerError> {
        let parsed = parse_snapshot(self.config, bytes)?;
        self.ticks = parsed.ticks;
        self.issued = parsed.issued;
        self.coalesced = parsed.coalesced;
        self.pending = parsed.pending;
        Ok(())
    }

    const fn completed_lines(&self) -> u64 {
        self.ticks / MASTER_CYCLES_PER_LINE
    }

    fn apply_events(
        &mut self,
        level: InterruptLevel,
        events: u64,
        first_line: u64,
        line_limit: u64,
        lines: u64,
        active: u64,
    ) {
        if events == 0 {
            return;
        }
        let index = level.index();
        let coalesced = if self.pending[index].is_some() {
            events
        } else {
            let first = first_event_line(level, first_line, line_limit, lines, active);
            let issued_tick = (first + 1) * MASTER_CYCLES_PER_LINE;
            self.pending[index] = Some(PendingInterrupt { level, issued_tick });
            events - 1
        };
        self.issued[index] = self.issued[index].saturating_add(events);
        self.coalesced[index] = self.coalesced[index].saturating_add(coalesced);
    }
}

/// Counts completed scanlines in `[first, limit)` that raise each request.
///
/// The pattern repeats every `lines` lines, so this is constant time apart from
/// at most one frame of remainder.
fn event_counts(first: u64, limit: u64, lines: u64, active: u64) -> (u64, u64) {
    let total = limit - first;
    let whole_frames = total / lines;
    let remainder = total % lines;
    let first_line = first % lines;
    let mut hblank = whole_frames * active;
    let mut vblank = whole_frames;
    for step in 0..remainder {
        let line = (first_line + step) % lines;
        if line < active {
            hblank += 1;
        }
        if line + 1 == active {
            vblank += 1;
        }
    }
    (hblank, vblank)
}

/// Returns the first completed line in `[first, limit)` that raises `level`.
fn first_event_line(level: InterruptLevel, first: u64, limit: u64, lines: u64, active: u64) -> u64 {
    let first_line = first % lines;
    for step in 0..(limit - first).min(lines) {
        let line = (first_line + step) % lines;
        let matches = match level {
            InterruptLevel::HBlank => line < active,
            InterruptLevel::VBlank => line + 1 == active,
        };
        if matches {
            return first + step;
        }
    }
    first
}

#[derive(Debug)]
struct SnapshotState {
    ticks: u64,
    issued: [u64; 2],
    coalesced: [u64; 2],
    pending: [Option<PendingInterrupt>; 2],
}

fn parse_snapshot(config: SchedulerConfig, bytes: &[u8]) -> Result<SnapshotState, SchedulerError> {
    let mut reader = Reader::new(bytes);
    if reader.read_array::<8>().map_err(truncated_at("magic"))? != SNAPSHOT_MAGIC {
        return Err(SchedulerError::SnapshotMagicMismatch);
    }
    let version = reader.read_u16_le().map_err(truncated_at("version"))?;
    if version != SNAPSHOT_VERSION {
        return Err(SchedulerError::UnsupportedSnapshotVersion { found: version });
    }
    let standard =
        VideoStandard::from_code(reader.read_u8().map_err(truncated_at("video_standard"))?)?;
    if standard != config.standard() {
        return Err(SchedulerError::SnapshotStandardMismatch {
            expected: config.standard(),
            found: standard,
        });
    }
    let ticks = reader.read_u64_le().map_err(truncated_at("ticks"))?;
    let mut issued = [0_u64; 2];
    for counter in &mut issued {
        *counter = reader
            .read_u64_le()
            .map_err(truncated_at("requests_issued"))?;
    }
    let mut coalesced = [0_u64; 2];
    for counter in &mut coalesced {
        *counter = reader
            .read_u64_le()
            .map_err(truncated_at("requests_coalesced"))?;
    }
    let pending_mask = reader.read_u8().map_err(truncated_at("pending_mask"))?;
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
        let issued_tick = reader
            .read_u64_le()
            .map_err(truncated_at(level.snapshot_field()))?;
        if issued_tick > ticks {
            return Err(SchedulerError::SnapshotPendingInFuture {
                level,
                issued_tick,
                ticks,
            });
        }
        if issued_tick % MASTER_CYCLES_PER_LINE != 0 {
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

fn truncated_at(field: &'static str) -> impl FnOnce(CodecError) -> SchedulerError {
    move |_error| SchedulerError::SnapshotTruncated { field }
}

#[cfg(test)]
mod tests {
    use super::{
        HBLANK_INTERRUPT_LEVEL, InterruptLevel, InterruptScheduler, NTSC_ACTIVE_LINES,
        NTSC_LINES_PER_FRAME, PendingInterrupt, SchedulerConfig, SchedulerError,
        VBLANK_INTERRUPT_LEVEL, VideoStandard,
    };
    use crate::cycles::MASTER_CYCLES_PER_LINE;

    fn scheduler() -> InterruptScheduler {
        InterruptScheduler::new(SchedulerConfig::ntsc())
    }

    #[test]
    fn the_first_hblank_is_on_the_3420_master_cycle_boundary() {
        let mut scheduler = scheduler();
        scheduler
            .advance_master(MASTER_CYCLES_PER_LINE - 1)
            .expect("advance");
        assert!(!scheduler.has_pending());
        scheduler.advance_master(1).expect("cross line");
        assert_eq!(scheduler.ticks(), MASTER_CYCLES_PER_LINE);
        assert!(scheduler.is_pending(InterruptLevel::HBlank));
        assert_eq!(
            scheduler.next_pending().expect("pending").level(),
            HBLANK_INTERRUPT_LEVEL
        );
        assert_eq!(
            scheduler
                .pending_events()
                .next()
                .expect("event")
                .issued_tick(),
            MASTER_CYCLES_PER_LINE
        );
    }

    #[test]
    fn ntsc_vblank_fires_after_224_active_lines() {
        let mut scheduler = scheduler();
        scheduler
            .advance_master(MASTER_CYCLES_PER_LINE * u64::from(NTSC_ACTIVE_LINES))
            .expect("advance");
        assert!(scheduler.is_pending(InterruptLevel::VBlank));
        assert_eq!(scheduler.line_in_frame(), 224);
        assert_eq!(scheduler.frame(), 0);
        assert_eq!(
            scheduler.pending_events().next().expect("vblank").level(),
            InterruptLevel::VBlank,
            "VBlank is delivered before HBlank"
        );
        assert_eq!(
            scheduler
                .next_deliverable(0x2000)
                .expect("deliverable")
                .level(),
            VBLANK_INTERRUPT_LEVEL
        );
        scheduler.acknowledge(InterruptLevel::VBlank);
        assert!(scheduler.is_pending(InterruptLevel::HBlank));
    }

    #[test]
    fn pal_uses_313_lines_and_240_active_lines() {
        let mut scheduler = InterruptScheduler::new(SchedulerConfig::pal());
        assert_eq!(scheduler.config().standard(), VideoStandard::Pal);
        assert_eq!(scheduler.config().lines_per_frame(), 313);
        assert_eq!(scheduler.config().active_lines(), 240);
        scheduler
            .advance_master(MASTER_CYCLES_PER_LINE * 240)
            .expect("advance");
        assert!(scheduler.is_pending(InterruptLevel::VBlank));
        assert_eq!(scheduler.line_in_frame(), 240);
        scheduler
            .advance_master(MASTER_CYCLES_PER_LINE * 73)
            .expect("finish frame");
        assert_eq!(scheduler.frame(), 1);
        assert_eq!(scheduler.line_in_frame(), 0);
    }

    #[test]
    fn advancing_in_chunks_is_equivalent_to_one_step() {
        let mut single = scheduler();
        single
            .advance_master(MASTER_CYCLES_PER_LINE * 300)
            .expect("one step");
        let mut chunked = scheduler();
        for _ in 0..300 {
            chunked
                .advance_master(MASTER_CYCLES_PER_LINE)
                .expect("line step");
        }
        assert_eq!(single.snapshot_bytes(), chunked.snapshot_bytes());
    }

    #[test]
    fn requests_coalesce_and_keep_the_first_issue_tick() {
        let mut scheduler = scheduler();
        scheduler
            .advance_master(MASTER_CYCLES_PER_LINE * 2)
            .expect("two lines");
        assert_eq!(scheduler.pending_len(), 1);
        let first = scheduler.pending_events().next().expect("pending");
        scheduler
            .advance_master(MASTER_CYCLES_PER_LINE)
            .expect("third line");
        assert_eq!(scheduler.pending_len(), 1);
        assert_eq!(
            scheduler.pending_events().next().expect("pending"),
            first,
            "coalescing preserves the original issue tick"
        );
        assert_eq!(scheduler.requests_issued(InterruptLevel::HBlank), 3);
        assert_eq!(scheduler.requests_coalesced(InterruptLevel::HBlank), 2);
    }

    #[test]
    fn level_seven_is_always_deliverable_and_masked_levels_stay_pending() {
        let mut scheduler = scheduler();
        scheduler
            .advance_master(MASTER_CYCLES_PER_LINE)
            .expect("hblank");
        assert_eq!(scheduler.next_deliverable(0x2700), None);
        scheduler.acknowledge(InterruptLevel::HBlank);
        assert!(scheduler.pending_len() == 0);
        assert_eq!(HBLANK_INTERRUPT_LEVEL, 4);
        assert_eq!(VBLANK_INTERRUPT_LEVEL, 6);
        assert_eq!(NTSC_ACTIVE_LINES, 224);
        assert_eq!(NTSC_LINES_PER_FRAME, 262);
    }

    #[test]
    fn snapshot_round_trips_master_cycles_and_pending_events() {
        let mut scheduler = scheduler();
        scheduler
            .advance_master(MASTER_CYCLES_PER_LINE * 226)
            .expect("advance");
        let bytes = scheduler.snapshot_bytes();
        assert_eq!(bytes.len(), 52 + 16);
        let mut restored = InterruptScheduler::new(SchedulerConfig::ntsc());
        restored.restore_bytes(&bytes).expect("restore");
        assert_eq!(restored.snapshot_bytes(), bytes);
        assert_eq!(restored.ticks(), scheduler.ticks());
        assert_eq!(restored.pending_len(), 2);
    }

    #[test]
    fn snapshot_rejects_wrong_magic_version_trailing_bytes_and_masks() {
        let mut scheduler = scheduler();
        scheduler
            .advance_master(MASTER_CYCLES_PER_LINE)
            .expect("advance");
        let good = scheduler.snapshot_bytes();

        let mut bad_magic = good.clone();
        bad_magic[0] ^= 0xFF;
        assert_eq!(
            scheduler.restore_bytes(&bad_magic),
            Err(SchedulerError::SnapshotMagicMismatch)
        );

        let mut bad_version = good.clone();
        bad_version[8..10].copy_from_slice(&99_u16.to_le_bytes());
        assert_eq!(
            scheduler.restore_bytes(&bad_version),
            Err(SchedulerError::UnsupportedSnapshotVersion { found: 99 })
        );

        let mut extra = good.clone();
        extra.push(0);
        assert_eq!(
            scheduler.restore_bytes(&extra),
            Err(SchedulerError::SnapshotTrailingBytes { count: 1 })
        );

        let mut bad_mask = good;
        bad_mask[51] = 0x80;
        assert_eq!(
            scheduler.restore_bytes(&bad_mask),
            Err(SchedulerError::InvalidPendingMask { value: 0x80 })
        );
        assert!(scheduler.is_pending(InterruptLevel::HBlank));
    }

    #[test]
    fn snapshot_rejects_a_different_video_standard() {
        let mut ntsc = scheduler();
        ntsc.advance_master(MASTER_CYCLES_PER_LINE)
            .expect("advance");
        let bytes = ntsc.snapshot_bytes();
        let mut pal = InterruptScheduler::new(SchedulerConfig::pal());
        assert_eq!(
            pal.restore_bytes(&bytes),
            Err(SchedulerError::SnapshotStandardMismatch {
                expected: VideoStandard::Pal,
                found: VideoStandard::Ntsc,
            })
        );
    }

    #[test]
    fn snapshot_rejects_pending_ticks_off_the_line_boundary() {
        let mut scheduler = scheduler();
        scheduler
            .advance_master(MASTER_CYCLES_PER_LINE * 2)
            .expect("advance");
        let mut bytes = scheduler.snapshot_bytes();
        let last = bytes.len() - 8;
        bytes[last..].copy_from_slice(&((MASTER_CYCLES_PER_LINE * 2) - 1).to_le_bytes());
        assert!(matches!(
            scheduler.restore_bytes(&bytes),
            Err(SchedulerError::SnapshotPendingMisaligned { .. })
        ));
    }

    #[test]
    fn advance_overflow_is_typed_and_leaves_state_unchanged() {
        let mut scheduler = scheduler();
        scheduler.advance_master(u64::MAX).expect("max");
        let before = scheduler.snapshot_bytes();
        assert_eq!(
            scheduler.advance_master(1),
            Err(SchedulerError::TickOverflow)
        );
        assert_eq!(scheduler.snapshot_bytes(), before);
    }

    #[test]
    fn pending_interrupt_accessors_match_construction() {
        let event = PendingInterrupt {
            level: InterruptLevel::VBlank,
            issued_tick: 3420,
        };
        assert_eq!(event.level(), InterruptLevel::VBlank);
        assert_eq!(event.issued_tick(), 3420);
        assert_eq!(InterruptLevel::from_level(4), Some(InterruptLevel::HBlank));
        assert_eq!(InterruptLevel::from_level(5), None);
        assert_eq!(InterruptLevel::HBlank.to_string(), "HBlank");
        assert_eq!(VideoStandard::Ntsc.to_string(), "NTSC");
        assert_eq!(
            SchedulerConfig::new(VideoStandard::Ntsc),
            SchedulerConfig::ntsc()
        );
    }
}
