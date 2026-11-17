//! Pacing and the host-time run loop.
//!
//! This module owns the only time seam in the crate. [`HostClock`] (`now_nanos`) is
//! injected: [`ManualClock`] is the deterministic implementation used by tests **and by the
//! CLI**, so no emitted byte depends on host time. [`SystemClock`] is the library's
//! real-clock implementation of the same trait; no v1 CLI process path builds it (the CLI
//! injects `ManualClock` and exposes only the deterministic `pulse`), so live host-time
//! playback is a follow-up.
//!
//! [`Pacer`] owns the cadence: `fps` (frames per second), `ticks_per_frame` (canonical
//! `session.run` steps per presented frame) and `max_catch_up_frames` (how many frames a
//! late host may execute in one step before the rest are counted as dropped). The two
//! entry points are deliberately different (`gui.md` §5.2):
//!
//! - [`Pacer::pulse`] is explicit and clock-independent: it advances exactly `frames`
//!   whole presentation frames, never drops and never partially advances a frame, so the
//!   process surface stays byte-deterministic.
//! - [`Pacer::advance_elapsed`] is the real host-time run loop required by FR-EXEC-007:
//!   it converts elapsed host time into due frames, advances at most
//!   `max_catch_up_frames` and reports the remainder as dropped, so a slow host cannot
//!   spiral.
//!
//! The pacer holds no session and no model: each advanced frame calls the caller's
//! `advance_frame` closure with the canonical step count for that frame, and the closure
//! performs the one `session.run` call and folds the result into the presentation model.
//! That is what keeps this module free of channel, view and model types — and what makes
//! "exactly `frames` frames, exactly `frames * ticks_per_frame` canonical steps" a
//! counter assertion in a unit test rather than a claim.
//!
//! Host time never enters canonical state or emitted data: [`Pacer`] stores only the
//! cumulative nanos of the frames it has advanced ([`Pacer::last_nanos`]), which is a
//! deterministic function of the requested frames and `fps`. The clock itself is read at
//! exactly one place, [`crate::command::CommandSession::advance_elapsed`], and its value
//! appears only in the [`ElapsedReport`].

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use crate::{
    DEFAULT_FPS, DEFAULT_MAX_CATCH_UP_FRAMES, DEFAULT_TICKS_PER_FRAME, GuiError, MAX_FPS,
    MAX_MAX_CATCH_UP_FRAMES, MAX_TICKS_PER_FRAME, MIN_FPS,
};

/// Number of nanoseconds in one second, the base of the cadence arithmetic.
const NANOS_PER_SECOND: u64 = 1_000_000_000;

/// The one time seam of the crate.
///
/// An implementation reports a monotonic non-decreasing nanosecond count; the crate never
/// calls a platform clock directly, so a test can supply [`ManualClock`] and reproduce an
/// instant exactly. The trait is `Send + Sync` because one clock is shared by the command
/// session and handed to a caller behind an `Arc`.
pub trait HostClock: Send + Sync {
    /// Returns the current host time in nanoseconds.
    fn now_nanos(&self) -> u64;
}

/// The library's real-clock implementation of [`HostClock`]: monotonically non-decreasing
/// nanoseconds since construction.
///
/// [`Instant`] is monotonic by contract, so two calls never go backwards even across a
/// wall-clock adjustment. The origin is the moment the clock was created, which keeps the
/// value small and independent of the host's epoch; no absolute time is ever reported.
///
/// This is a library capability, not a v1 process path: no CLI process builds it, because
/// the CLI injects [`ManualClock`] and exposes only the deterministic `pulse` command. Its
/// consumer is [`crate::command::CommandSession::advance_elapsed`], driven by tests at
/// chosen instants; live host-time playback is a follow-up.
#[derive(Debug)]
pub struct SystemClock {
    origin: Instant,
}

impl SystemClock {
    /// Creates a clock whose origin is now.
    #[must_use]
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl HostClock for SystemClock {
    fn now_nanos(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }
}

/// The deterministic clock: a value the caller sets, never read from the host.
///
/// This is what the CLI uses and what every pacing test uses, so a run is reproducible: the
/// same sequence of [`ManualClock::set_nanos`] and [`ManualClock::advance_nanos`] calls
/// yields the same [`ElapsedReport`]s, and no emitted byte depends on scheduling.
#[derive(Debug, Default)]
pub struct ManualClock {
    nanos: AtomicU64,
}

impl ManualClock {
    /// Creates a clock at zero.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a clock already set to `nanos`.
    #[must_use]
    pub fn at(nanos: u64) -> Self {
        Self {
            nanos: AtomicU64::new(nanos),
        }
    }

    /// Sets the clock to `nanos`.
    ///
    /// Setting a value below the current one is allowed at this layer: a monotonic clock
    /// never does it, and [`Pacer::advance_elapsed`] treats a backwards reading as "no time
    /// has elapsed" rather than advancing a negative number of frames.
    pub fn set_nanos(&self, nanos: u64) {
        self.nanos.store(nanos, Ordering::SeqCst);
    }

    /// Advances the clock by `delta` nanoseconds, saturating at [`u64::MAX`].
    ///
    /// Returns the new value. The read-modify-write loop keeps the clock monotonic under
    /// concurrent use and cannot wrap.
    pub fn advance_nanos(&self, delta: u64) -> u64 {
        let mut current = self.nanos.load(Ordering::SeqCst);
        loop {
            let next = current.saturating_add(delta);
            match self
                .nanos
                .compare_exchange(current, next, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => return next,
                Err(observed) => current = observed,
            }
        }
    }

    /// Returns the current value without going through the trait.
    #[must_use]
    pub fn peek_nanos(&self) -> u64 {
        self.nanos.load(Ordering::SeqCst)
    }
}

impl HostClock for ManualClock {
    fn now_nanos(&self) -> u64 {
        self.peek_nanos()
    }
}

/// The validated cadence of one window.
///
/// Every field is checked once, in [`PacerConfig::new`], against the documented bounds
/// (`gui.md` §7), so no later frame computation has to re-validate a value: an invalid
/// cadence cannot exist as a [`PacerConfig`]. The fields are private for that reason — a
/// public field would let a struct literal bypass the check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PacerConfig {
    fps: u32,
    ticks_per_frame: u64,
    max_catch_up_frames: u64,
}

impl PacerConfig {
    /// Creates a cadence, validating every bound.
    ///
    /// # Errors
    ///
    /// Returns [`GuiError::BadValue`] when `fps` is outside `MIN_FPS..=MAX_FPS`, when
    /// `ticks_per_frame` is outside `1..=MAX_TICKS_PER_FRAME`, or when
    /// `max_catch_up_frames` is outside `1..=MAX_MAX_CATCH_UP_FRAMES`. A zero or
    /// out-of-range value is a rejected configuration, never a clamped one.
    pub fn new(fps: u32, ticks_per_frame: u64, max_catch_up_frames: u64) -> Result<Self, GuiError> {
        if !(MIN_FPS..=MAX_FPS).contains(&fps) {
            return Err(GuiError::bad_value(
                "fps",
                &format!("{fps} is outside {MIN_FPS}..={MAX_FPS}"),
            ));
        }
        if ticks_per_frame == 0 || ticks_per_frame > MAX_TICKS_PER_FRAME {
            return Err(GuiError::bad_value(
                "ticks_per_frame",
                &format!("{ticks_per_frame} is outside 1..={MAX_TICKS_PER_FRAME}"),
            ));
        }
        if max_catch_up_frames == 0 || max_catch_up_frames > MAX_MAX_CATCH_UP_FRAMES {
            return Err(GuiError::bad_value(
                "max_catch_up_frames",
                &format!("{max_catch_up_frames} is outside 1..={MAX_MAX_CATCH_UP_FRAMES}"),
            ));
        }
        Ok(Self {
            fps,
            ticks_per_frame,
            max_catch_up_frames,
        })
    }

    /// Returns the presentation cadence in frames per second.
    #[must_use]
    pub const fn fps(&self) -> u32 {
        self.fps
    }

    /// Returns the canonical steps advanced per presented frame.
    #[must_use]
    pub const fn ticks_per_frame(&self) -> u64 {
        self.ticks_per_frame
    }

    /// Returns the largest number of frames one late step may catch up.
    #[must_use]
    pub const fn max_catch_up_frames(&self) -> u64 {
        self.max_catch_up_frames
    }

    /// Returns the documented frame length in nanoseconds: `1_000_000_000 / fps`.
    ///
    /// The division is integer division over a validated `fps` of at least `MIN_FPS`, so
    /// the result is at least one and can never divide by zero.
    #[must_use]
    pub const fn frame_ns(&self) -> u64 {
        NANOS_PER_SECOND / (self.fps as u64)
    }
}

impl Default for PacerConfig {
    fn default() -> Self {
        Self {
            fps: DEFAULT_FPS,
            ticks_per_frame: DEFAULT_TICKS_PER_FRAME,
            max_catch_up_frames: DEFAULT_MAX_CATCH_UP_FRAMES,
        }
    }
}

/// What one explicit [`Pacer::pulse`] advanced.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PulseReport {
    frames: u64,
    steps: u64,
    ticks_per_frame: u64,
    fps: u32,
    elapsed_nanos: u64,
    dropped: u64,
}

impl PulseReport {
    /// Returns the number of whole frames advanced, which is exactly the number requested.
    #[must_use]
    pub const fn frames(&self) -> u64 {
        self.frames
    }

    /// Returns the canonical steps advanced: `frames * ticks_per_frame`.
    #[must_use]
    pub const fn steps(&self) -> u64 {
        self.steps
    }

    /// Returns the canonical steps advanced per frame.
    #[must_use]
    pub const fn ticks_per_frame(&self) -> u64 {
        self.ticks_per_frame
    }

    /// Returns the cadence in frames per second.
    #[must_use]
    pub const fn fps(&self) -> u32 {
        self.fps
    }

    /// Returns the cumulative pacer time after the pulse (`last_nanos` in `gui.md` §5.2).
    ///
    /// This is a function of the requested frames and the cadence — not of a host clock —
    /// which is why the process surface is reproducible.
    #[must_use]
    pub const fn elapsed_nanos(&self) -> u64 {
        self.elapsed_nanos
    }

    /// Returns the frames a pulse did not advance: always zero.
    ///
    /// The explicit path never drops a frame; the report carries the field so the `pulse`
    /// block and the [`ElapsedReport`] describe the same quantity with the same type.
    #[must_use]
    pub const fn dropped(&self) -> u64 {
        self.dropped
    }
}

/// What one host-time [`Pacer::advance_elapsed`] step advanced, caught up or dropped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ElapsedReport {
    due: u64,
    advanced: u64,
    dropped: u64,
    steps: u64,
    elapsed_nanos: u64,
}

impl ElapsedReport {
    /// Returns the number of frames the elapsed time asked for.
    #[must_use]
    pub const fn due(&self) -> u64 {
        self.due
    }

    /// Returns the number of frames advanced: `min(due, max_catch_up_frames)`.
    #[must_use]
    pub const fn advanced(&self) -> u64 {
        self.advanced
    }

    /// Returns the frames given up to keep a slow host from spiralling: `due - advanced`.
    #[must_use]
    pub const fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Returns the canonical steps advanced: `advanced * ticks_per_frame`.
    #[must_use]
    pub const fn steps(&self) -> u64 {
        self.steps
    }

    /// Returns the cumulative pacer time after the step (`last_nanos` in `gui.md` §5.2).
    #[must_use]
    pub const fn elapsed_nanos(&self) -> u64 {
        self.elapsed_nanos
    }
}

/// The cadence owner: frame length, catch-up cap and cumulative advanced time.
///
/// The pacer is a plain value (`Debug`, `Eq`) with no interior clock: a caller reads its
/// [`HostClock`] at the chosen instant and passes the number to
/// [`Pacer::advance_elapsed`], which is what makes the run loop deterministic under
/// [`ManualClock`] and testable at exact instants.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Pacer {
    config: PacerConfig,
    frame_ns: u64,
    last_nanos: u64,
}

impl Pacer {
    /// Creates a pacer for an already-validated cadence, with zero advanced time.
    #[must_use]
    pub const fn new(config: PacerConfig) -> Self {
        Self {
            config,
            frame_ns: config.frame_ns(),
            last_nanos: 0,
        }
    }

    /// Returns the cadence this pacer applies.
    #[must_use]
    pub const fn config(&self) -> PacerConfig {
        self.config
    }

    /// Returns the frame length in nanoseconds.
    #[must_use]
    pub const fn frame_ns(&self) -> u64 {
        self.frame_ns
    }

    /// Returns the cumulative time of the frames advanced so far, in nanoseconds.
    #[must_use]
    pub const fn last_nanos(&self) -> u64 {
        self.last_nanos
    }

    /// Advances exactly `frames` whole presentation frames, independent of any clock.
    ///
    /// `advance_frame` is called once per frame with `ticks_per_frame`, and the pacer
    /// advances its cumulative time by `frames * frame_ns` only after every call
    /// succeeded: a channel failure part-way leaves the pacer's accounting where it was,
    /// because the frames that were advanced are reported by the failing call and not by a
    /// block. No frame is ever partially advanced and none is dropped.
    ///
    /// # Errors
    ///
    /// Propagates the caller's typed [`GuiError`]; a failed frame stops the pulse at that
    /// frame.
    pub fn pulse<F>(&mut self, frames: u64, mut advance_frame: F) -> Result<PulseReport, GuiError>
    where
        F: FnMut(u64) -> Result<(), GuiError>,
    {
        let ticks = self.config.ticks_per_frame;
        for _ in 0..frames {
            advance_frame(ticks)?;
        }
        let advanced_nanos = frames.saturating_mul(self.frame_ns);
        self.last_nanos = self.last_nanos.saturating_add(advanced_nanos);
        Ok(PulseReport {
            frames,
            steps: frames.saturating_mul(ticks),
            ticks_per_frame: ticks,
            fps: self.config.fps,
            elapsed_nanos: self.last_nanos,
            dropped: 0,
        })
    }

    /// Converts elapsed host time into frames, advancing at most `max_catch_up_frames`.
    ///
    /// `due` is `(now_nanos - last_nanos) / frame_ns`, `advanced` is `min(due, cap)` and
    /// `dropped` is the remainder; `last_nanos` moves by `advanced * frame_ns`, so the
    /// dropped frames are given up rather than accumulated into the next step. A reading at
    /// or before `last_nanos` is treated as no elapsed time and advances nothing, so a
    /// non-monotonic reading cannot rewind the cadence.
    ///
    /// # Errors
    ///
    /// Propagates the caller's typed [`GuiError`]; a failed frame stops the step at that
    /// frame and leaves the cumulative time where it was.
    pub fn advance_elapsed<F>(
        &mut self,
        now_nanos: u64,
        mut advance_frame: F,
    ) -> Result<ElapsedReport, GuiError>
    where
        F: FnMut(u64) -> Result<(), GuiError>,
    {
        let ticks = self.config.ticks_per_frame;
        let due = now_nanos.saturating_sub(self.last_nanos) / self.frame_ns;
        let advanced = due.min(self.config.max_catch_up_frames);
        for _ in 0..advanced {
            advance_frame(ticks)?;
        }
        let advanced_nanos = advanced.saturating_mul(self.frame_ns);
        self.last_nanos = self.last_nanos.saturating_add(advanced_nanos);
        Ok(ElapsedReport {
            due,
            advanced,
            dropped: due - advanced,
            steps: advanced.saturating_mul(ticks),
            elapsed_nanos: self.last_nanos,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{HostClock, ManualClock, Pacer, PacerConfig, SystemClock};
    use crate::{
        DEFAULT_FPS, DEFAULT_MAX_CATCH_UP_FRAMES, DEFAULT_TICKS_PER_FRAME, GuiError, GuiErrorCode,
        MAX_FPS, MAX_MAX_CATCH_UP_FRAMES, MAX_TICKS_PER_FRAME, MIN_FPS,
    };

    fn config(fps: u32, ticks: u64, catch_up: u64) -> PacerConfig {
        PacerConfig::new(fps, ticks, catch_up).expect("valid cadence")
    }

    #[test]
    fn the_documented_defaults_and_ranges_are_the_lib_constants() {
        let defaults = PacerConfig::default();
        assert_eq!(defaults.fps(), DEFAULT_FPS);
        assert_eq!(defaults.ticks_per_frame(), DEFAULT_TICKS_PER_FRAME);
        assert_eq!(defaults.max_catch_up_frames(), DEFAULT_MAX_CATCH_UP_FRAMES);
        assert_eq!(
            PacerConfig::new(MIN_FPS, 1, 1).expect("minimum").fps(),
            MIN_FPS
        );
        assert_eq!(
            PacerConfig::new(MAX_FPS, MAX_TICKS_PER_FRAME, MAX_MAX_CATCH_UP_FRAMES)
                .expect("maximum")
                .max_catch_up_frames(),
            MAX_MAX_CATCH_UP_FRAMES
        );
    }

    #[test]
    fn an_out_of_range_cadence_is_a_typed_bad_value() {
        for fps in [0, MAX_FPS + 1, u32::MAX] {
            let error = PacerConfig::new(fps, 1, 1).expect_err("fps out of range");
            assert_eq!(error.code(), GuiErrorCode::BadValue);
            assert!(error.to_string().contains("fps"));
        }
        for ticks in [0, MAX_TICKS_PER_FRAME + 1, u64::MAX] {
            let error = PacerConfig::new(DEFAULT_FPS, ticks, 1).expect_err("ticks out of range");
            assert_eq!(error.code(), GuiErrorCode::BadValue);
            assert!(error.to_string().contains("ticks_per_frame"));
        }
        for catch_up in [0, MAX_MAX_CATCH_UP_FRAMES + 1] {
            let error =
                PacerConfig::new(DEFAULT_FPS, 1, catch_up).expect_err("catch-up out of range");
            assert_eq!(error.code(), GuiErrorCode::BadValue);
            assert!(error.to_string().contains("max_catch_up_frames"));
        }
    }

    #[test]
    fn the_frame_length_is_integer_nanoseconds_of_one_second() {
        assert_eq!(config(1, 1, 1).frame_ns(), 1_000_000_000);
        assert_eq!(config(2, 1, 1).frame_ns(), 500_000_000);
        assert_eq!(config(60, 1, 1).frame_ns(), 16_666_666);
        assert_eq!(config(240, 1, 1).frame_ns(), 4_166_666);
        assert!(config(MAX_FPS, 1, 1).frame_ns() >= 1);
        assert_eq!(Pacer::new(config(60, 1, 1)).frame_ns(), 16_666_666);
    }

    #[test]
    fn pulse_advances_exactly_the_requested_frames_and_steps() {
        let mut pacer = Pacer::new(config(60, 7, 4));
        let mut frames = 0_u64;
        let mut steps = 0_u64;
        let report = pacer
            .pulse(5, |per_frame| {
                frames += 1;
                steps += per_frame;
                Ok(())
            })
            .expect("pulse");
        assert_eq!(frames, 5, "one call per frame");
        assert_eq!(steps, 35, "five frames of seven steps");
        assert_eq!(report.frames(), 5);
        assert_eq!(report.steps(), 35);
        assert_eq!(report.ticks_per_frame(), 7);
        assert_eq!(report.fps(), 60);
        assert_eq!(report.dropped(), 0);
        assert_eq!(report.elapsed_nanos(), 5 * 16_666_666);
        assert_eq!(pacer.last_nanos(), report.elapsed_nanos());
    }

    #[test]
    fn pulse_time_is_cumulative_and_adds_whole_frames() {
        let mut pacer = Pacer::new(config(1, 2, 2));
        let first = pacer.pulse(3, |_| Ok(())).expect("first pulse");
        assert_eq!(first.elapsed_nanos(), 3_000_000_000);
        let second = pacer.pulse(2, |_| Ok(())).expect("second pulse");
        assert_eq!(second.elapsed_nanos(), 5_000_000_000);
        assert_eq!(second.steps(), 4);
        assert_eq!(pacer.last_nanos(), 5_000_000_000);
    }

    #[test]
    fn a_zero_frame_pulse_advances_nothing_and_reports_no_time() {
        let mut pacer = Pacer::new(config(60, 1, 4));
        let mut calls = 0_u64;
        let report = pacer
            .pulse(0, |_| {
                calls += 1;
                Ok(())
            })
            .expect("zero pulse");
        assert_eq!(calls, 0, "a zero-frame pulse calls no frame hook");
        assert_eq!(report.frames(), 0);
        assert_eq!(report.steps(), 0);
        assert_eq!(report.elapsed_nanos(), 0);
        assert_eq!(report.dropped(), 0);
        assert_eq!(pacer.last_nanos(), 0);
    }

    #[test]
    fn a_failing_frame_stops_the_pulse_and_leaves_the_time_untouched() {
        let mut pacer = Pacer::new(config(1, 1, 1));
        let mut calls = 0_u64;
        let error = pacer
            .pulse(4, |_| {
                calls += 1;
                if calls == 2 {
                    return Err(GuiError::channel("run failed"));
                }
                Ok(())
            })
            .expect_err("second frame fails");
        assert_eq!(error.code(), GuiErrorCode::Channel);
        assert_eq!(calls, 2);
        assert_eq!(pacer.last_nanos(), 0, "a failed pulse commits no time");
    }

    #[test]
    fn advance_elapsed_caps_the_catch_up_and_counts_the_dropped_frames() {
        let mut pacer = Pacer::new(config(1, 1, 4));
        let mut advanced = 0_u64;
        let report = pacer
            .advance_elapsed(3_500_000_000, |per_frame| {
                advanced += per_frame;
                Ok(())
            })
            .expect("first step");
        assert_eq!(report.due(), 3);
        assert_eq!(report.advanced(), 3);
        assert_eq!(report.dropped(), 0);
        assert_eq!(report.steps(), 3);
        assert_eq!(advanced, 3);
        assert_eq!(report.elapsed_nanos(), 3_000_000_000);

        let report = pacer
            .advance_elapsed(10_000_000_000, |_| Ok(()))
            .expect("late step");
        assert_eq!(report.due(), 7, "seven frames are due after 10 s");
        assert_eq!(report.advanced(), 4, "capped at max_catch_up_frames");
        assert_eq!(report.dropped(), 3, "the remainder is dropped, not queued");
        assert_eq!(report.elapsed_nanos(), 7_000_000_000);

        let report = pacer
            .advance_elapsed(7_000_000_000, |_| Ok(()))
            .expect("caught up");
        assert_eq!(report.due(), 0);
        assert_eq!(report.advanced(), 0);
        assert_eq!(report.dropped(), 0);
        assert_eq!(report.elapsed_nanos(), 7_000_000_000);
    }

    #[test]
    fn a_backwards_or_static_reading_advances_nothing() {
        let mut pacer = Pacer::new(config(1, 1, 4));
        pacer.pulse(2, |_| Ok(())).expect("pulse");
        assert_eq!(pacer.last_nanos(), 2_000_000_000);
        for now in [2_000_000_000, 1_000_000_000, 0] {
            let mut calls = 0_u64;
            let report = pacer
                .advance_elapsed(now, |_| {
                    calls += 1;
                    Ok(())
                })
                .expect("no step");
            assert_eq!(calls, 0, "nothing is due at {now}");
            assert_eq!(report.due(), 0);
            assert_eq!(report.advanced(), 0);
            assert_eq!(report.dropped(), 0);
            assert_eq!(report.elapsed_nanos(), 2_000_000_000);
        }
    }

    #[test]
    fn a_manual_clock_is_explicit_and_saturating() {
        let clock = ManualClock::new();
        assert_eq!(clock.now_nanos(), 0);
        assert_eq!(clock.advance_nanos(150), 150);
        assert_eq!(clock.now_nanos(), 150);
        clock.set_nanos(7);
        assert_eq!(clock.now_nanos(), 7);
        clock.set_nanos(u64::MAX);
        assert_eq!(clock.advance_nanos(1), u64::MAX, "saturates, never wraps");
        assert_eq!(ManualClock::at(42).peek_nanos(), 42);
        let shared = ManualClock::at(5);
        assert_eq!(HostClock::now_nanos(&shared), 5);
    }

    #[test]
    fn the_system_clock_is_monotonic_and_starts_near_zero() {
        let clock = SystemClock::new();
        let first = clock.now_nanos();
        let second = clock.now_nanos();
        assert!(second >= first, "monotonic non-decreasing");
        assert!(
            first < 60_000_000_000,
            "the origin is construction, not an epoch"
        );
        assert!(SystemClock::default().now_nanos() < 60_000_000_000);
    }

    #[test]
    fn the_pacer_stores_no_host_time_and_holds_no_session() {
        let mut pacer = Pacer::new(config(60, 1, 4));
        let report = pacer.pulse(1, |_| Ok(())).expect("pulse");
        assert_eq!(report.elapsed_nanos(), pacer.frame_ns());
        // The pacer's only stored time is the cumulative advanced time; a second pacer with
        // the same cadence reports the same value for the same frames.
        let mut twin = Pacer::new(config(60, 1, 4));
        assert_eq!(
            twin.pulse(1, |_| Ok(())).expect("twin pulse"),
            report,
            "the report is a function of the cadence and the frame count"
        );
    }
}
