//! ChronoFork's first real-hardware backend slice.
//!
//! It is a minimal 68000 + cartridge/RAM + VDP-port machine with a deterministic
//! master-cycle interrupt scheduler, **not** a complete Mega Drive.
//! The cartridge device models the header, region, Sega mapper, 64 KiB battery
//! SRAM and TMSS lockout (see [`crate::cartridge`]); payload and hash formats are
//! v6. The VDP implements CRAM/VSRAM addressing, plane/window/sprite composition,
//! DMA, FIFO/CPU wait accounting, line/dot timing and VDP-owned VBlank/HBlank
//! interrupt pending/acknowledge. The YM2612 and PSG models feed the integer
//! 48 kHz mixer exposed through the media seam, and the Z80 runs inside each
//! 68000 instruction's master-cycle window while arbitration allows it.
//! TH-latched/6-button controller protocols, YM timer IRQ delivery to the Z80,
//! VSRAM data-port reads, interlace, the byte-swapped physical VRAM layout,
//! dot-exact H/V counters and cycle-accurate timing are not implemented. Controller port
//! reads are limited to the TH-high 3-button subset of the two data registers
//! (see [`controller_rom`] and the controller model below). The backend
//! advertises only what it actually provides and uses an independently authored
//! synthetic ROM in tests and the demo.
//!
//! # Interrupt policy
//!
//! Each executed instruction advances the scheduler by its 68000 cycle cost times
//! seven master cycles (3420 master cycles per scanline). Before each instruction,
//! **at most one** pending VBlank (level 6) or HBlank (level 4)
//! request is delivered through [`M68000::interrupt`] using the 68000 autovector
//! address (`0x78` or `0x70`); VBlank is delivered first when both are
//! deliverable. Delivery is gated by the single 68000 rule exposed as
//! [`crate::cpu::interrupt_level_deliverable`] and
//! [`M68000::accepts_interrupt_level`]: level 7 is always taken, and every other
//! level must be strictly greater than the `SR` interrupt-priority mask. The
//! backend asks the scheduler for `next_deliverable(self.cpu.sr())` and never
//! delivers from `next_pending`; masked requests are not delivered. The VDP only
//! drives levels 6 and 4, so the "level 7 is always deliverable" branch cannot be
//! reached through a VDP request; it lives in the shared predicate and is tested
//! at the CPU/scheduler boundary. The backend uses that same predicate, so a
//! future level-7 source would be admitted even with mask 7.
//!
//! When a request is delivered, the CPU exception entry is performed first and
//! the request is acknowledged only after the entry succeeds. The entry API takes
//! the accepted level explicitly: it pushes the previous raw `SR`, sets `S`,
//! preserves the CCR and loads the accepted level into the interrupt-priority
//! mask, so a lower-priority VDP source cannot nest inside the handler before
//! `RTE` restores the frame (see [`M68000::interrupt`] and [`interrupt_rom`]).
//!
//! A masked request stays pending with its original issue tick and is not
//! acknowledged, dropped or duplicated. The scheduler retains at most one request
//! per level, so additional requests of the same level coalesce into it; the
//! pending request remains inspectable until the program lowers the mask (for
//! example with `MOVE.W #imm,SR`) and a later instruction boundary delivers it.
//! Delivery acknowledges exactly the level that entered the CPU, so a pending
//! request at the other level is retained. A failed exception entry acknowledges
//! nothing.
//!
//! The VDP status register exposes this state. Bit 3 is the video VBlank flag
//! derived from the scheduler's scanline position. Bit 7 (`VINT pending`) is set
//! while a VBlank (level 6) request is pending — including while the `SR` mask
//! blocks it — and clears when that request is delivered and acknowledged. The
//! VINT latch is derived state: the scheduler's pending request is the single
//! canonical source, it is already serialized and hashed in the backend payload,
//! and the backend recomputes the latch after every step and immediately after a
//! restore, so [`Backend::inspect`] is correct without an extra step. A VDP
//! status-port read never acknowledges or clears a request: only delivery does.
//! Acknowledging VBlank therefore cannot lose a pending HBlank, and acknowledging
//! HBlank cannot lose a pending VBlank.
//!
//! The 68000 reset `SR` is `0x2700`, so a ROM that never writes `SR` remains
//! interrupt-free. [`interrupt_rom`] provides an independently authored synthetic
//! program that masks the VDP levels explicitly, retains an HBlank request while
//! masked, lowers the mask with `MOVE.W #imm,SR`, takes HBlank/VBlank through
//! handlers that end in `RTE`, and returns to the interrupted program with the
//! saved raw `SR` (`IPM`, `S` and `CCR`).
//!
//! # Instruction trace
//!
//! [`MegadriveBackend::enable_trace`] turns on a bounded, debug-only instruction
//! trace (see [`crate::trace`]). It records executed address/opcode pairs for
//! differential evidence and does not change emulated behavior: snapshots, state
//! hashes and configuration hashes are byte-identical with tracing on or off, the
//! payload version is unchanged, and restore does not rewind trace contents. The
//! generic [`Backend`] interface exposes no trace method, so the backend
//! capability `tracing` stays `false`; use the concrete type.
//!
//! # Debug observation
//!
//! [`Backend::debug_access`] returns this backend's [`DebugAccess`], so the host
//! can install a bounded event ring and up to
//! [`MAX_WATCHPOINTS`](chronofork_backend_api::MAX_WATCHPOINTS) watchpoints. The
//! bus reports one `Watchpoint` event per matching byte lane of every emulated
//! read or write (instruction fetches included), one `BusWrite` per work-RAM byte
//! write, one `VdpPort` per VDP port access and one `ControllerRead` per
//! controller data-register read. The scheduler side reports
//! `InterruptRequested` when a per-level issued counter advances without a
//! coalesced increment, `InterruptDelivered` after a successful CPU interrupt
//! entry and `InterruptAcknowledged` after the scheduler acknowledges exactly the
//! delivered level. Events created during a step carry that step's pre-step tick.
//!
//! Observation is **not** canonical state: it is absent from the serialized state
//! body, the state hash, the configuration hash, the payload version and every
//! snapshot, restoring a snapshot does not rewind it, and the debugger
//! `read_memory`/`write_memory` paths never produce an event. The descriptor
//! therefore reports `watchpoints = true` and `debug_events = true`, while
//! `breakpoints` stays `false`: breakpoints are a host debugger feature built on
//! `inspect`, not a backend capability.
//!
//! # Controller input
//!
//! [`Backend::step`] makes its `InputFrame` visible to the two controller data
//! registers only while that step executes: `0xA10003` (port 1) is driven by
//! `ports()[0]` and `0xA10005` (port 2) by `ports()[1]`. Each read returns the
//! TH-high 3-button subset — Up, Down, Left, Right, B and C, active-low, with TH
//! high in bit 6 and bit 7 zero — so a neutral port reads `0x7F`. A and Start are
//! not readable, and every other controller-window access, including all writes,
//! stays a typed unmapped error.
//!
//! The input is transient: the backend sets it at step entry and clears it on
//! every exit path, so it cannot leak into a later step, a snapshot, a restored
//! machine or inspection. It is not serialized and not hashed; a restored machine
//! uses the next caller-supplied frame. See ADR-0009.
//!
//! Differential runs that must remain interrupt-free can rely on the reset mask 7
//! (a program that never writes `SR` takes no VDP interrupt); there is no
//! instruction-quantum knob and the master-cycle timeline is never changed
//! implicitly.

use crate::vdp::{CRAM_WORDS, HIGHLIGHT_FLAG, SHADOW_FLAG};
use chronofork_backend_api::{
    BACKEND_SNAPSHOT_FORMAT_VERSION, Backend, BackendDescriptor, BackendError, BackendFactory,
    BackendId, BackendSnapshot, Capabilities, DebugAccess, DebugConfig, DebugEventKind,
    DebugEventLog, MachineInspection, MediaSource, StepOutcome, VideoFrame, validate_envelope,
};
use chronofork_core::codec::{CodecError, Reader, Writer};
use chronofork_core::{InputFrame, StateHash, Tick, hash_bytes};

use crate::audio::mixer::SNAPSHOT_LEN as MIXER_SNAPSHOT_LEN;
use crate::audio::psg::SNAPSHOT_LEN as PSG_SNAPSHOT_LEN;
use crate::audio::ym2612::SNAPSHOT_LEN as YM_SNAPSHOT_LEN;
use crate::audio::{AudioMixer, YmBank};
use crate::bus::{
    BusArbitration, BusError, IO_CONTROL_BASE, IO_CONTROL_LEN, MegadriveBus, RAM_BASE, RAM_END,
    RAM_MIRROR_BASE, RAM_SIZE, ROM_BASE, ROM_END, Z80_RAM_SIZE, bus_address, io_control_offset,
};
use crate::cartridge::{Cartridge, CartridgeError, SRAM_SIZE, Tmss, TmssModel};
use crate::cpu::{CpuError, M68000, interrupt_level_deliverable};
use crate::cycles::MASTER_CYCLES_PER_68000_CYCLE;
use crate::interrupts::{
    InterruptLevel, InterruptScheduler, SCHEDULER_MAX_SNAPSHOT_LEN, SchedulerConfig,
    SchedulerError, VideoStandard,
};
use crate::trace::{InstructionTrace, TraceError};
use crate::vdp::{
    AUTO_INCREMENT_REGISTER, SNAPSHOT_LEN as VDP_SNAPSHOT_LEN, Vdp, VdpInterruptSource,
};
use crate::z80::{SNAPSHOT_LEN as Z80_SNAPSHOT_LEN, Z80, Z80Error, Z80Outcome};

/// Stable backend identifier.
pub const BACKEND_ID: &str = "chronofork.megadrive.m68000";

/// Stereo frames in one audio chunk handed to a capture consumer.
///
/// 64 frames are about 1.3 ms at 48 kHz: small enough that a consumer polling
/// after every step sees fresh audio, and the mixer's bounded queue absorbs the
/// gaps when it polls less often. The backend holds at most one chunk, so a
/// second take before new audio has accumulated returns `None`.
pub const AUDIO_CHUNK_FRAMES: usize = 64;
/// Payload format version.
///
/// Version 3 added the cartridge state (seven mapper banks, SRAM control byte,
/// 64 KiB battery SRAM, TMSS model/lock latch); version 5 adds the rendered
/// frame counter through the embedded VDP snapshot; version 6 adds the Z80
/// register file and 8 KiB Z80 RAM, the 68000/Z80 arbitration latches, the Z80
/// master-cycle carry, the nine-bit Z80 bank register, the complete YM2612 and
/// PSG state and the 48 kHz mixer frame phase and device resamplers; version 7
/// adds the eight port 1/2/3/EXT control register bytes at `0xA10008..=0xA1000F`,
/// which are canonical state because they affect later reads; version 8 adds the
/// VDP read buffer through the embedded VDP snapshot (`CFVDP003`), which is
/// canonical state because it is the value the next data-port read returns.
/// Version 8 is the only accepted version; payloads v1-v7 are rejected with a
/// typed error instead of being reinterpreted.
pub const PAYLOAD_VERSION: u16 = 8;
/// 68000 autovector address used for a level-4 HBlank request.
pub const HBLANK_VECTOR_ADDRESS: u32 = 0x70;
/// 68000 autovector address used for a level-6 VBlank request.
pub const VBLANK_VECTOR_ADDRESS: u32 = 0x78;

/// Snapshot payload magic for payload version 8.
pub const PAYLOAD_MAGIC: [u8; 8] = *b"CFMD68K8";
/// State-hash magic for state hash version 8.
pub const STATE_MAGIC: &[u8; 10] = b"CFMDSTATE8";
/// Configuration-hash magic for configuration hash version 8.
pub const CONFIG_MAGIC: &[u8; 9] = b"CFMDCONF8";

/// Fixed byte length of the Z80, arbitration, I/O and audio state tail.
///
/// It follows the variable-length interrupt-scheduler snapshot, so the payload
/// parser splits the scheduler by subtracting this length from the remaining
/// bytes. Every component is fixed-length by construction.
const AUDIO_STATE_TAIL_LEN: usize = Z80_SNAPSHOT_LEN
    + Z80_RAM_SIZE
    + 8
    + 2
    + 2
    + 8
    + YM_SNAPSHOT_LEN
    + PSG_SNAPSHOT_LEN
    + MIXER_SNAPSHOT_LEN;

/// Exact payload bytes of a state whose interrupt scheduler has no pending request.
///
/// This is [`state_body`] with the scheduler at its zero-pending length plus the
/// payload prefix of [`PAYLOAD_MAGIC`] and the version word. Every
/// component of the body except the scheduler snapshot is fixed-length by
/// construction (see [`AUDIO_STATE_TAIL_LEN`] and `state_body` itself), so this is
/// also the 68000/register/RAM/VDP/cartridge/SRAM/TMSS component sum the
/// serializer's capacity expression is built from.
const FIXED_STATE_BODY_LEN: usize = 32
    + 32
    + 4
    + 2
    + 1
    + 8
    + 4
    + 4
    + 8
    + 1
    + RAM_SIZE
    + VDP_SNAPSHOT_LEN
    + 14
    + 1
    + SRAM_SIZE
    + 2
    + AUDIO_STATE_TAIL_LEN;

/// Hard upper bound on the payload of any [`MegadriveBackend::snapshot`].
///
/// The v7 payload is `PAYLOAD_MAGIC` (8 bytes) + the `u16` version + the state
/// body. The scheduler snapshot is the only variable-length component, and the
/// module that owns its format declares the exact maximum it can serialize
/// (`SCHEDULER_MAX_SNAPSHOT_LEN`); every other component is fixed, so the bound is
/// this backend's fixed component sum plus that declared maximum. A state with both
/// requests pending reaches it **exactly**; the tightness tests in this module cover
/// NTSC and PAL, imported SRAM and a restored state.
pub const SNAPSHOT_PAYLOAD_BOUND: usize =
    PAYLOAD_MAGIC.len() + 2 + FIXED_STATE_BODY_LEN + SCHEDULER_MAX_SNAPSHOT_LEN;

/// Cartridge capability tag folded into the configuration hash.
///
/// Bumping the tag is how a mapper/SRAM model change invalidates old
/// configuration hashes without touching the ROM identity.
const CONFIG_FEATURE_TAG: &[u8; 16] = b"sega-mapper-sram";

/// Configuration that selects how a cartridge is presented.
///
/// `video` overrides the header-derived standard; `tmss` selects the modelled
/// lockout. Both are configuration, not mutable state: they are covered by the
/// configuration hash and never serialized.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MegadriveConfig {
    /// Explicit video standard; `None` derives it from the header region.
    pub video: Option<VideoStandard>,
    /// TMSS model; defaults to [`TmssModel::Disabled`].
    pub tmss: TmssModel,
}

/// Factory for independent minimal Mega Drive machines.
#[derive(Clone, Debug)]
pub struct MegadriveFactory {
    descriptor: BackendDescriptor,
    cartridge: Cartridge,
    config_hash: StateHash,
    scheduler_config: SchedulerConfig,
    video: VideoStandard,
}

impl MegadriveFactory {
    /// Creates a factory from an independently provided cartridge image.
    ///
    /// The header derives the video standard (PAL only for an `E`-only region)
    /// and TMSS defaults to [`TmssModel::Disabled`]. The image must contain a
    /// valid cartridge header and must not exceed the mapper's 64 MiB addressable
    /// ceiling. No ROM is bundled with this crate.
    pub fn new(rom: Vec<u8>) -> Result<Self, BackendError> {
        Self::with_config(rom, MegadriveConfig::default())
    }

    /// Creates a factory with an explicit cartridge configuration.
    ///
    /// An explicit `video` overrides the header-derived standard (for tests and
    /// region-mismatched dumps) and `tmss` selects the lockout. Both are covered
    /// by the configuration hash.
    pub fn with_config(rom: Vec<u8>, config: MegadriveConfig) -> Result<Self, BackendError> {
        let mut cartridge = Cartridge::parse(rom).map_err(map_cartridge_config)?;
        cartridge.set_tmss_model(config.tmss);
        let video = config
            .video
            .unwrap_or_else(|| cartridge.derived_video_standard());
        let scheduler_config = SchedulerConfig::new(video);
        Self::from_parts(cartridge, config.tmss, scheduler_config)
    }

    /// Creates a factory with an explicit interrupt-scheduler configuration.
    ///
    /// The scheduler standard is the machine's video standard and TMSS defaults
    /// to [`TmssModel::Disabled`]. The configuration is part of the configuration
    /// hash, so a snapshot made with a different video standard is rejected.
    pub fn with_scheduler_config(
        rom: Vec<u8>,
        scheduler_config: SchedulerConfig,
    ) -> Result<Self, BackendError> {
        let cartridge = Cartridge::parse(rom).map_err(map_cartridge_config)?;
        Self::from_parts(cartridge, TmssModel::Disabled, scheduler_config)
    }

    fn from_parts(
        cartridge: Cartridge,
        tmss: TmssModel,
        scheduler_config: SchedulerConfig,
    ) -> Result<Self, BackendError> {
        let video = scheduler_config.standard();
        let config_hash = config_hash_for(cartridge.rom(), video, tmss);
        let capabilities = Capabilities {
            execution: true,
            save_states: true,
            memory_read: true,
            memory_write: true,
            register_inspection: true,
            instruction_stepping: true,
            // The bus itself matches watchpoints and produces typed bus/device
            // events. Breakpoints stay a host feature over `inspect`.
            watchpoints: true,
            debug_events: true,
            state_cloning: true,
            deterministic_replay: true,
            // Frame production is implemented and covered by `vdp_frame`;
            // 48 kHz stereo chunks are produced by the tested mixer and exposed
            // through the media seam.
            rendering: true,
            audio: true,
            ..Capabilities::none()
        };
        Ok(Self {
            descriptor: BackendDescriptor::new(
                BackendId::new(BACKEND_ID),
                "ChronoFork Mega Drive 68000 + Z80 (VDP, interrupts, video frames and 48 kHz audio)",
                env!("CARGO_PKG_VERSION"),
                capabilities,
                false,
            ),
            cartridge,
            config_hash,
            scheduler_config,
            video,
        })
    }

    /// Returns the immutable cartridge image.
    #[must_use]
    pub fn rom(&self) -> &[u8] {
        self.cartridge.rom()
    }

    /// Returns the video standard this factory configures.
    #[must_use]
    pub const fn video_standard(&self) -> VideoStandard {
        self.video
    }

    /// Returns the configuration digest.
    #[must_use]
    pub const fn config_hash(&self) -> StateHash {
        self.config_hash
    }

    /// Returns the fixed interrupt-scheduler configuration.
    #[must_use]
    pub const fn scheduler_config(&self) -> SchedulerConfig {
        self.scheduler_config
    }

    /// Creates one concrete minimal Mega Drive machine.
    ///
    /// This is the same machine [`BackendFactory::create`] returns, but with the
    /// concrete type available for VDP, cartridge and scheduler inspection.
    pub fn create_machine(&self) -> Result<MegadriveBackend, BackendError> {
        let mut bus = MegadriveBus::with_video_standard(self.cartridge.clone(), self.video);
        let mut cpu = M68000::new();
        cpu.reset(&mut bus).map_err(map_cpu)?;
        let mut z80 = Z80::new();
        z80.reset(&mut bus);
        Ok(MegadriveBackend {
            descriptor: self.descriptor.clone(),
            config_hash: self.config_hash,
            bus,
            cpu,
            scheduler: InterruptScheduler::new(self.scheduler_config),
            tick: Tick::ZERO,
            z80,
            z80_credit: 0,
            mixer: AudioMixer::for_video_standard(self.video),
            pending_audio: None,
            pending_frame: None,
        })
    }
}

impl BackendFactory for MegadriveFactory {
    fn descriptor(&self) -> &BackendDescriptor {
        &self.descriptor
    }

    fn create(&self) -> Result<Box<dyn Backend>, BackendError> {
        Ok(Box::new(self.create_machine()?))
    }
}

/// One independent minimal Mega Drive machine.
#[derive(Debug)]
pub struct MegadriveBackend {
    descriptor: BackendDescriptor,
    config_hash: StateHash,
    bus: MegadriveBus,
    cpu: M68000,
    scheduler: InterruptScheduler,
    tick: Tick,
    /// The Z80 core.
    ///
    /// It is canonical state and payload v6 serializes the register file.
    /// The core runs inside [`Backend::step`] whenever the 68000 has requested
    /// the bus and released the reset; [`MegadriveBackend::advance_z80`] remains
    /// available for explicit single-instruction control.
    z80: Z80,
    /// Z80 master cycles already executed ahead of the 68000 step timeline.
    ///
    /// The Z80 runs at instruction granularity, so the last instruction of a
    /// step can overshoot the step's master-cycle window. The overshoot is
    /// subtracted from the next step's budget and is canonical state: payload v6
    /// serializes it so a restored machine keeps the same Z80 cadence.
    z80_credit: u64,
    /// Integer YM2612/PSG mixer and 48 kHz output clock.
    ///
    /// Its phase and device sample clocks are canonical state from payload v6;
    /// the queued frames are derived output and are never serialized.
    mixer: AudioMixer,
    /// Bounded one-chunk audio queue; derived output, never serialized.
    pending_audio: Option<chronofork_backend_api::AudioChunk>,
    /// Bounded one-frame media queue; derived output, never serialized.
    pending_frame: Option<VideoFrame>,
}

impl MegadriveBackend {
    /// Returns the current canonical tick, measured in master-clock cycles.
    #[must_use]
    pub const fn tick(&self) -> Tick {
        self.tick
    }

    /// Returns the canonical master-clock cycle counter.
    #[must_use]
    pub const fn master_cycles(&self) -> u64 {
        self.tick.raw()
    }

    /// Returns the Z80 core.
    #[must_use]
    pub const fn z80(&self) -> &Z80 {
        &self.z80
    }

    /// Returns the YM2612 FM synthesiser.
    #[must_use]
    pub const fn ym2612(&self) -> &crate::audio::Ym2612 {
        self.bus.ym()
    }

    /// Returns the SN76489-compatible PSG.
    #[must_use]
    pub const fn psg(&self) -> &crate::audio::Psg {
        self.bus.psg()
    }

    /// Returns the audio mixer.
    #[must_use]
    pub const fn audio_mixer(&self) -> &AudioMixer {
        &self.mixer
    }

    /// Returns the current 68000.
    #[must_use]
    pub const fn cpu(&self) -> &M68000 {
        &self.cpu
    }

    /// Returns the machine bus for read-only debug inspection.
    ///
    /// Views such as [`crate::BusView`] and the disassembler's
    /// [`crate::WindowMemory`] take the bus or its ROM/RAM slices. The accessor
    /// hands out `&MegadriveBus`, so a view can never issue a mutating bus
    /// access or produce a debug event.
    #[must_use]
    pub const fn bus(&self) -> &MegadriveBus {
        &self.bus
    }

    /// Returns the current VDP state.
    #[must_use]
    pub const fn vdp(&self) -> &Vdp {
        self.bus.vdp()
    }

    /// Returns the cartridge device.
    #[must_use]
    pub const fn cartridge(&self) -> &Cartridge {
        self.bus.cartridge()
    }

    /// Returns the TMSS latch owned by the cartridge.
    #[must_use]
    pub const fn tmss(&self) -> &Tmss {
        self.bus.tmss()
    }

    /// Writes one byte to a cartridge-controlled window.
    ///
    /// This is the test/debug accessor for mapper and SRAM writes. It maps the
    /// cartridge's typed error to the backend execution error contract; emulated
    /// code reaches the same windows through the CPU.
    pub fn write_cartridge(&mut self, address: u32, value: u8) -> Result<(), BackendError> {
        self.bus
            .cartridge_mut()
            .write8(address, value)
            .map_err(|error| BackendError::Execution(error.to_string()))
    }

    /// Returns the current interrupt scheduler.
    #[must_use]
    pub const fn interrupts(&self) -> &InterruptScheduler {
        &self.scheduler
    }

    /// Returns the scheduler configuration used by this machine.
    #[must_use]
    pub const fn scheduler_config(&self) -> SchedulerConfig {
        self.scheduler.config()
    }

    /// Enables the bounded debug instruction trace on the 68000.
    ///
    /// See [`crate::trace`] for the recording rules and bounded-ring behavior.
    /// The trace is disabled by default and is not canonical state: it is absent
    /// from snapshots, state hashes and configuration hashes, and restore does not
    /// restore its contents.
    ///
    /// # Errors
    ///
    /// Returns [`TraceError::ZeroCapacity`] when `capacity` is zero, leaving any
    /// existing trace unchanged.
    pub fn enable_trace(&mut self, capacity: usize) -> Result<(), TraceError> {
        self.cpu.enable_trace(capacity)
    }

    /// Disables the instruction trace and discards its records.
    pub fn disable_trace(&mut self) {
        self.cpu.disable_trace();
    }

    /// Clears an enabled trace and restarts its indexes at zero.
    pub fn reset_trace(&mut self) {
        self.cpu.reset_trace();
    }

    /// Returns the active instruction trace, or `None` when disabled.
    #[must_use]
    pub const fn trace(&self) -> Option<&InstructionTrace> {
        self.cpu.trace()
    }

    /// Removes and returns the instruction trace, disabling further recording.
    pub fn take_trace(&mut self) -> Option<InstructionTrace> {
        self.cpu.take_trace()
    }

    /// Serializes the canonical mutable-state body for the current machine
    /// state.
    ///
    /// [`Self::snapshot`] calls this once and derives both the payload and the
    /// state hash from the returned bytes.
    fn state_body_bytes(&self) -> Vec<u8> {
        let (d, a, pc, sr, halted) = self.cpu.snapshot_state();
        let (usp, ssp, stopped, cycles) = self.cpu.extended_state();
        let vdp = self.bus.vdp().snapshot_bytes();
        let scheduler = self.scheduler.snapshot_bytes();
        let z80 = self.z80.snapshot_bytes();
        let ym = self.bus.ym().snapshot_bytes();
        let psg = self.bus.psg().snapshot_bytes();
        let mixer = self.mixer.snapshot_bytes();
        state_body(&StateView {
            d: &d,
            a: &a,
            pc,
            sr,
            halted,
            usp,
            ssp,
            cpu_cycles: cycles,
            stopped,
            tick: self.tick,
            ram: self.bus.ram(),
            vdp: &vdp,
            banks: &self.bus.cartridge().banks(),
            sram_control: self.bus.cartridge().sram_control(),
            sram: self.bus.cartridge().sram(),
            tmss_model: self.bus.tmss().model(),
            tmss_locked: self.bus.tmss().locked(),
            scheduler: &scheduler,
            z80: &z80,
            z80_ram: self.bus.z80_ram(),
            z80_credit: self.z80_credit,
            arbitration: self.bus.arbitration(),
            z80_bank: self.bus.z80_bank(),
            io_control: self.bus.io_control(),
            ym: &ym,
            psg: &psg,
            mixer: &mixer,
        })
    }

    /// Hashes the canonical state for a step outcome and restore validation.
    ///
    /// This is the per-step hot path. The view construction and hashing stay in
    /// one chain: routing them through [`Self::state_body_bytes`] was measured
    /// ~18% slower in `bench_step` because the extra boundary changed codegen.
    /// The serialized field layout still has exactly one writer,
    /// [`state_body`].
    fn state_hash(&self) -> StateHash {
        let (d, a, pc, sr, halted) = self.cpu.snapshot_state();
        let (usp, ssp, stopped, cycles) = self.cpu.extended_state();
        let vdp = self.bus.vdp().snapshot_bytes();
        let scheduler = self.scheduler.snapshot_bytes();
        let z80 = self.z80.snapshot_bytes();
        let ym = self.bus.ym().snapshot_bytes();
        let psg = self.bus.psg().snapshot_bytes();
        let mixer = self.mixer.snapshot_bytes();
        state_hash_for(&StateView {
            d: &d,
            a: &a,
            pc,
            sr,
            halted,
            usp,
            ssp,
            cpu_cycles: cycles,
            stopped,
            tick: self.tick,
            ram: self.bus.ram(),
            vdp: &vdp,
            banks: &self.bus.cartridge().banks(),
            sram_control: self.bus.cartridge().sram_control(),
            sram: self.bus.cartridge().sram(),
            tmss_model: self.bus.tmss().model(),
            tmss_locked: self.bus.tmss().locked(),
            scheduler: &scheduler,
            z80: &z80,
            z80_ram: self.bus.z80_ram(),
            z80_credit: self.z80_credit,
            arbitration: self.bus.arbitration(),
            z80_bank: self.bus.z80_bank(),
            io_control: self.bus.io_control(),
            ym: &ym,
            psg: &psg,
            mixer: &mixer,
        })
    }

    fn sync_vdp_vblank(&mut self) {
        let vblank = self.scheduler.line_in_frame() >= self.scheduler.config().active_lines();
        self.bus.vdp_mut().set_vblank(vblank);
    }

    /// Mirrors the scheduler's retained line events into VDP-owned requests.
    ///
    /// The scheduler remains the master-cycle line-event source; the VDP owns
    /// the pending/acknowledge surface and the register-`0x00` enable gating.
    /// A request the program has disabled is dropped by the VDP, while the
    /// scheduler keeps its retained request so enabling the bit later can still
    /// deliver it.
    fn sync_vdp_interrupts(&mut self) {
        for (level, source) in [
            (InterruptLevel::VBlank, VdpInterruptSource::VBlank),
            (InterruptLevel::HBlank, VdpInterruptSource::HBlank),
        ] {
            if self.scheduler.is_pending(level) {
                self.bus.vdp_mut().request_interrupt(source);
            }
        }
    }

    /// Recomputes the derived VINT-pending status bit from the VDP's owned
    /// VBlank request.
    ///
    /// The latch is set while the VDP holds an enabled VBlank request, whether
    /// or not the 68000 `SR` mask currently admits it, and clears only when
    /// delivery acknowledges that request. It is derived state, so this must run
    /// after every mutation of pending state and after a restore.
    fn sync_vdp_vint_pending(&mut self) {
        let pending = self.bus.vdp().take_interrupts().vblank_pending;
        self.bus.vdp_mut().set_vint_pending(pending);
    }

    fn sync_vdp_status(&mut self) {
        self.sync_vdp_vblank();
        self.sync_vdp_interrupts();
        self.sync_vdp_vint_pending();
    }

    /// Delivers at most one pending request admitted by the 68000 mask rule.
    ///
    /// The scheduler is asked for `next_deliverable(self.cpu.sr())`, which applies
    /// the shared `interrupt_level_deliverable` predicate (level 7 always, every
    /// other level strictly above the `SR` interrupt-priority mask). A masked
    /// request is left untouched: it stays pending with its original issue tick
    /// and is neither acknowledged nor coalesced away by this method. When a level
    /// is delivered, only that level is acknowledged, so a pending request of the
    /// other level survives.
    fn deliver_pending_interrupt(&mut self) -> Result<(), BackendError> {
        // VBlank (level 6) is checked before HBlank (level 4), preserving the
        // hardware priority order. A request is deliverable only when the VDP
        // still owns it (register 0x00 enabled it) and the 68000 SR mask admits
        // the level according to the shared predicate.
        for (level, source) in [
            (InterruptLevel::VBlank, VdpInterruptSource::VBlank),
            (InterruptLevel::HBlank, VdpInterruptSource::HBlank),
        ] {
            if !self.scheduler.is_pending(level)
                || !self.bus.vdp().take_interrupts().pending(source)
                || !interrupt_level_deliverable(level.level(), self.cpu.sr())
            {
                continue;
            }
            self.cpu
                .interrupt(level.level(), vector_address(level), &mut self.bus)
                .map_err(map_cpu)?;
            self.bus
                .debug_observer_mut()
                .record(DebugEventKind::InterruptDelivered {
                    level: level.level(),
                });
            // Acknowledge exactly the delivered level and source: selecting a
            // different retained request could acknowledge a masked one.
            let acknowledged = self.scheduler.acknowledge(level);
            debug_assert!(acknowledged, "a deliverable level must have been pending");
            self.bus.vdp_mut().acknowledge(source);
            if acknowledged {
                self.bus
                    .debug_observer_mut()
                    .record(DebugEventKind::InterruptAcknowledged {
                        level: level.level(),
                    });
            }
            self.sync_vdp_vint_pending();
            return Ok(());
        }
        Ok(())
    }

    /// Returns the per-level scheduler counter pairs observed for request events,
    /// in [`InterruptLevel::ALL`] order.
    fn scheduler_counters(&self) -> ([u64; 2], [u64; 2]) {
        (
            InterruptLevel::ALL.map(|level| self.scheduler.requests_issued(level)),
            InterruptLevel::ALL.map(|level| self.scheduler.requests_coalesced(level)),
        )
    }

    /// Reports scheduler request-counter changes to the debug observer.
    ///
    /// A coalesced request advances its level's coalesced counter as well, so the
    /// observer records no duplicate request event for a request that merged into
    /// one that is already pending.
    fn observe_scheduler_requests(&mut self) {
        let (issued, coalesced) = self.scheduler_counters();
        self.bus
            .debug_observer_mut()
            .observe_scheduler(issued, coalesced);
    }

    /// Executes one instruction with the step's controller input already set on
    /// the bus.
    ///
    /// The caller ([`Backend::step`]) owns the transient set/clear pair; this
    /// helper must never be called on its own.
    fn step_with_current_input(&mut self) -> Result<StepOutcome, BackendError> {
        // Debug observation is debug-only: stamping the pre-step tick here is
        // the only state it shares with the machine. Every event created below
        // carries this tick.
        self.bus.debug_begin_step(self.tick);
        // Take at most one interrupt at the instruction boundary, then execute
        // one instruction. This models the 68000 taking one request between
        // instructions instead of nesting several handlers in a single tick.
        self.deliver_pending_interrupt()?;
        let outcome = self.cpu.step(&mut self.bus).map_err(map_cpu)?;
        // A data-port write that found the VDP FIFO full stalls the CPU by one
        // 68000 cycle per overflow slot; convert the slots to master cycles and
        // add them to this instruction's cost. This is deterministic integer
        // arithmetic, never host time.
        let fifo_wait = self.bus.take_vdp_wait_cycles();
        let instruction_cycles = u64::from(outcome.cycles)
            .checked_mul(MASTER_CYCLES_PER_68000_CYCLE)
            .ok_or(BackendError::TickOverflow)?;
        let wait_cycles = u64::from(fifo_wait)
            .checked_mul(MASTER_CYCLES_PER_68000_CYCLE)
            .ok_or(BackendError::TickOverflow)?;
        // A 68000 access to the Z80 window while the Z80 owns the bus is stalled
        // by a fixed, documented number of master cycles (one Z80 M1 fetch per
        // byte lane). Folding it into this instruction's cost puts the stall on
        // the canonical tick and therefore in the state hash.
        let z80_stall_cycles = self.bus.take_z80_stall_cycles();
        let master_cycles = instruction_cycles
            .checked_add(wait_cycles)
            .and_then(|cycles| cycles.checked_add(z80_stall_cycles))
            .ok_or(BackendError::TickOverflow)?;
        let next_tick = self
            .tick
            .raw()
            .checked_add(master_cycles)
            .ok_or(BackendError::TickOverflow)?;
        self.scheduler
            .advance_master(master_cycles)
            .map_err(map_scheduler)?;
        // The VDP advances on the same master-cycle timeline, then its FIFO
        // drains and any eager DMA busy window closes for the next step. Taking
        // the DMA cost only closes DMA_BUSY; FIFO waits were already taken above
        // and VDP interrupt pendings are untouched, so the status re-sync below
        // is purely the derived-state recomputation (scheduler mirror + VINT
        // latch), not a repair of an incidental clear.
        let standard = self.scheduler.config().standard();
        let vdp_tick = self.bus.vdp_mut().advance_master(master_cycles, standard);
        self.bus.vdp_mut().drain_fifo();
        self.bus.vdp_mut().take_dma_cycles();
        self.tick = Tick::from_raw(next_tick);
        if vdp_tick.frame_started {
            self.capture_frame()?;
        }
        self.observe_scheduler_requests();
        self.sync_vdp_status();
        // The Z80 runs inside the 68000 instruction's master-cycle window: its
        // instruction time is budgeted from the window (with the carry from the
        // previous step) instead of extending the tick a second time. The audio
        // devices then advance by the whole window, so audio remains a pure
        // function of the canonical tick.
        self.run_z80(master_cycles)?;
        self.advance_audio(master_cycles);
        Ok(StepOutcome {
            tick: self.tick,
            state_hash: self.state_hash(),
            halted: self.cpu.halted(),
        })
    }
}

/// Converts palette-index pixels into RGBA8 with opaque alpha.
///
/// CRAM is the documented 9-bit `0BBBGGGRRR` word, expanded by bit replication;
/// the shadow flag halves each channel and the highlight flag adds `0x80`.
fn rgba_from_indices(indices: &[u8], cram: &[u16; CRAM_WORDS]) -> Vec<u8> {
    fn expand(value: u16) -> u8 {
        let value = (value & 0x7) as u8;
        (value << 5) | (value << 2) | (value >> 1)
    }
    let mut rgba = Vec::with_capacity(indices.len() * 4);
    for &pixel in indices {
        let colour = cram[usize::from(pixel & 0x3F)];
        let mut r = expand(colour);
        let mut g = expand(colour >> 3);
        let mut b = expand(colour >> 6);
        if pixel & SHADOW_FLAG != 0 {
            r >>= 1;
            g >>= 1;
            b >>= 1;
        } else if pixel & HIGHLIGHT_FLAG != 0 {
            r = r.saturating_add(0x80);
            g = g.saturating_add(0x80);
            b = b.saturating_add(0x80);
        }
        rgba.extend_from_slice(&[r, g, b, 0xFF]);
    }
    rgba
}

impl MegadriveBackend {
    /// Executes one Z80 instruction when bus arbitration allows it.
    ///
    /// Returns `Z80Outcome { t_states: 0 }` while the 68000 owns the bus or the
    /// Z80 is held in reset, so a caller can poll without changing any state.
    /// When the Z80 executes, its documented T-states convert to master cycles
    /// with [`Z80Outcome::master_cycles`] and advance the canonical tick, the
    /// interrupt scheduler and the VDP exactly as a 68000 instruction does.
    ///
    /// [`Backend::step`] also runs the Z80 inside the 68000 instruction's
    /// master-cycle window whenever arbitration allows it; this method remains
    /// the explicit single-instruction control for tests and debuggers, and it
    /// advances the tick, the scheduler, the VDP and the audio clock by the
    /// instruction's documented master cycles. The arbitration model is
    /// functional, not microcycle-exact: the 68000 is not frozen while the Z80
    /// owns the bus, but every 68000 access to the Z80 RAM window is stalled by
    /// [`Z80_STALL_MASTER_CYCLES`](crate::bus::Z80_STALL_MASTER_CYCLES). This
    /// explicit path does not consume the step carry: it advances the tick by the
    /// instruction cost and leaves the next step's budget untouched.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::TickOverflow`] on master-cycle overflow, or a
    /// mapped Z80/bus failure.
    pub fn advance_z80(&mut self) -> Result<Z80Outcome, BackendError> {
        if !self.bus.arbitration().z80_runs() {
            return Ok(Z80Outcome { t_states: 0 });
        }
        let outcome = self.z80.step(&mut self.bus).map_err(map_z80)?;
        let master_cycles = outcome.master_cycles();
        let next_tick = self
            .tick
            .raw()
            .checked_add(master_cycles)
            .ok_or(BackendError::TickOverflow)?;
        self.scheduler
            .advance_master(master_cycles)
            .map_err(map_scheduler)?;
        let standard = self.scheduler.config().standard();
        let vdp_tick = self.bus.vdp_mut().advance_master(master_cycles, standard);
        self.bus.vdp_mut().drain_fifo();
        self.bus.vdp_mut().take_dma_cycles();
        self.tick = Tick::from_raw(next_tick);
        if vdp_tick.frame_started {
            self.capture_frame()?;
        }
        self.sync_vdp_status();
        self.advance_audio(master_cycles);
        Ok(outcome)
    }

    /// Runs Z80 instructions inside a 68000 step's master-cycle window.
    ///
    /// The window plus the carry from the previous step is the Z80's budget.
    /// Instructions execute until the budget is spent; the overshoot of the last
    /// instruction becomes the next step's carry, so the Z80 never gains or
    /// loses emulated time and the cadence is a pure function of canonical
    /// state. While arbitration keeps the Z80 stopped its carry is discarded,
    /// because the 68000 timeline has moved on without it.
    fn run_z80(&mut self, window: u64) -> Result<(), BackendError> {
        if !self.bus.arbitration().z80_runs() {
            self.z80_credit = 0;
            return Ok(());
        }
        let budget = self.z80_credit.saturating_add(window);
        let mut consumed = 0_u64;
        while consumed < budget {
            let outcome = self.z80.step(&mut self.bus).map_err(map_z80)?;
            let master_cycles = outcome.master_cycles();
            if master_cycles == 0 {
                break;
            }
            consumed = consumed.saturating_add(master_cycles);
        }
        self.z80_credit = consumed.saturating_sub(budget);
        Ok(())
    }

    /// Advances the audio devices and the 48 kHz mixer by `master_cycles`.
    ///
    /// YM2612 timers follow the same master-cycle timeline; the mixer samples
    /// both devices at their canonical rates. When no chunk is pending and the
    /// mixer has at least [`AUDIO_CHUNK_FRAMES`] frames, one bounded chunk moves
    /// to the media queue.
    fn advance_audio(&mut self, master_cycles: u64) {
        let (ym, psg) = self.bus.audio_mut();
        ym.timers_elapsed(master_cycles);
        self.mixer.advance_master(master_cycles, ym, psg);
        if self.pending_audio.is_none() && self.mixer.pending_frames() >= AUDIO_CHUNK_FRAMES {
            self.pending_audio = self.mixer.take_chunk(AUDIO_CHUNK_FRAMES);
        }
    }

    /// Renders the completed frame from a clone of canonical VDP state.
    ///
    /// Rendering on a clone keeps sprite/status latches canonical and makes the
    /// frame a pure function of state at the boundary. The one-frame queue keeps
    /// the latest frame; a second take before a new frame returns `None`.
    fn capture_frame(&mut self) -> Result<(), BackendError> {
        let mut renderer = self.bus.vdp().clone();
        let buffer = renderer.finish_frame();
        let rgba = rgba_from_indices(&buffer.indices, renderer.cram());
        let frame = VideoFrame::new(buffer.width, buffer.height, rgba)
            .map_err(|error| BackendError::InvalidConfiguration(format!("video frame: {error}")))?;
        self.pending_frame = Some(frame);
        Ok(())
    }
}

impl MediaSource for MegadriveBackend {
    fn take_video_frame(&mut self) -> Option<VideoFrame> {
        self.pending_frame.take()
    }

    fn take_audio_chunk(&mut self) -> Option<chronofork_backend_api::AudioChunk> {
        self.pending_audio.take()
    }
}

impl Backend for MegadriveBackend {
    fn descriptor(&self) -> &BackendDescriptor {
        &self.descriptor
    }

    fn media_mut(&mut self) -> Option<&mut dyn MediaSource> {
        Some(self)
    }

    /// Declares the exact upper bound of every v7 payload this backend produces.
    ///
    /// A quota-checked capture of a state a `step`, a trace or a debug write changed
    /// cannot use the previously observed size, so the session layer needs the
    /// declaration before it allocates the record. [`SNAPSHOT_PAYLOAD_BOUND`] is that
    /// bound: the scheduler is the only variable-length component and it retains at
    /// most one request per level, so a two-pending state reaches it exactly.
    fn snapshot_payload_bound(&self) -> Option<usize> {
        Some(SNAPSHOT_PAYLOAD_BOUND)
    }

    fn step(&mut self, input: InputFrame) -> Result<StepOutcome, BackendError> {
        // Make the step's controller input visible to execution byte reads, then
        // clear it on every exit path so no input outlives the step. The clear is
        // unconditional: an interrupt-delivery, CPU, scheduler or tick failure
        // must not leave a stale controller value behind.
        let ports = input.ports();
        self.bus.set_step_input([ports[0], ports[1]]);
        let outcome = self.step_with_current_input();
        self.bus.clear_step_input();
        // A failed step never advanced the tick, so a stall it accumulated is
        // discarded instead of being charged to a later instruction.
        self.bus.clear_z80_stall_cycles();
        outcome
    }

    fn snapshot(&self) -> Result<BackendSnapshot, BackendError> {
        // Serialize the canonical state body once, then derive both the payload
        // and the state hash from it. Taking the snapshot therefore costs one
        // full-state serialization, not two.
        let body = self.state_body_bytes();
        Ok(BackendSnapshot::new(
            self.descriptor.id().clone(),
            self.descriptor.version(),
            BACKEND_SNAPSHOT_FORMAT_VERSION,
            self.tick,
            self.config_hash,
            hash_state_body(&body),
            payload_from_body(&body),
        ))
    }

    fn restore(&mut self, snapshot: &BackendSnapshot) -> Result<(), BackendError> {
        validate_envelope(
            snapshot,
            self.descriptor.id(),
            self.descriptor.version(),
            BACKEND_SNAPSHOT_FORMAT_VERSION,
            self.config_hash,
        )?;
        let parsed = parse_payload(snapshot.payload())?;
        let actual = state_hash_for(&StateView {
            d: &parsed.d,
            a: &parsed.a,
            pc: parsed.pc,
            sr: parsed.sr,
            halted: parsed.halted,
            usp: parsed.usp,
            ssp: parsed.ssp,
            cpu_cycles: parsed.cpu_cycles,
            stopped: parsed.stopped,
            tick: parsed.tick,
            ram: parsed.ram,
            vdp: parsed.vdp,
            banks: &parsed.banks,
            sram_control: parsed.sram_control,
            sram: parsed.sram,
            tmss_model: parsed.tmss_model,
            tmss_locked: parsed.tmss_locked,
            scheduler: parsed.scheduler,
            z80: parsed.z80,
            z80_ram: parsed.z80_ram,
            z80_credit: parsed.z80_credit,
            arbitration: parsed.arbitration,
            z80_bank: parsed.z80_bank,
            io_control: &parsed.io_control,
            ym: parsed.ym,
            psg: parsed.psg,
            mixer: parsed.mixer,
        });
        if actual != snapshot.state_hash() {
            return Err(BackendError::InvalidSnapshot(format!(
                "state hash mismatch after restore: expected {}, got {actual}",
                snapshot.state_hash()
            )));
        }

        // The TMSS model is configuration (it is part of the configuration hash),
        // so a payload that carries another model is rejected before anything is
        // committed even when its state hash was recomputed consistently.
        let configured_model = self.bus.tmss().model();
        if parsed.tmss_model != configured_model {
            return Err(BackendError::InvalidSnapshot(format!(
                "payload TMSS model {parsed_model:?} does not match the configured model {configured_model:?}",
                parsed_model = parsed.tmss_model
            )));
        }

        // Validate and build the replacement state before committing it, so a
        // rejected restore leaves the machine untouched.
        let mut bus = self.bus.clone();
        // Controller input is step-scoped, never serialized. Reset the cloned
        // bus explicitly so a restored machine starts from the neutral idle byte
        // and uses the next step's frame.
        bus.clear_step_input();
        bus.restore_ram(parsed.ram.to_vec())
            .map_err(|error| BackendError::InvalidSnapshot(error.to_string()))?;
        bus.vdp_mut().restore_bytes(parsed.vdp).map_err(|error| {
            BackendError::InvalidSnapshot(format!("invalid VDP state: {error}"))
        })?;
        bus.cartridge_mut()
            .restore_state(
                parsed.banks,
                parsed.sram_control,
                parsed.sram,
                parsed.tmss_model,
                parsed.tmss_locked,
            )
            .map_err(|error| {
                BackendError::InvalidSnapshot(format!("invalid cartridge state: {error}"))
            })?;
        bus.restore_z80_ram(parsed.z80_ram.to_vec())
            .map_err(|error| BackendError::InvalidSnapshot(error.to_string()))?;
        bus.restore_arbitration(parsed.arbitration);
        bus.restore_z80_bank(parsed.z80_bank);
        bus.restore_io_control(parsed.io_control);
        // Diagnostic counters describe execution since the restore (W1T/F4); the
        // VDP half is reset by `Vdp::restore_bytes` above.
        bus.reset_diagnostic_counters();
        let mut scheduler = self.scheduler.clone();
        scheduler.restore_bytes(parsed.scheduler).map_err(|error| {
            BackendError::InvalidSnapshot(format!("invalid interrupt state: {error}"))
        })?;
        let mut z80 = self.z80.clone();
        z80.restore_bytes(parsed.z80).map_err(|error| {
            BackendError::InvalidSnapshot(format!("invalid Z80 state: {error}"))
        })?;
        let mut ym = self.bus.ym().clone();
        ym.restore_bytes(parsed.ym).map_err(|error| {
            BackendError::InvalidSnapshot(format!("invalid YM2612 state: {error}"))
        })?;
        let mut psg = self.bus.psg().clone();
        psg.restore_bytes(parsed.psg).map_err(|error| {
            BackendError::InvalidSnapshot(format!("invalid PSG state: {error}"))
        })?;
        let mut mixer = AudioMixer::for_video_standard(self.scheduler.config().standard());
        mixer.restore_bytes(parsed.mixer).map_err(|error| {
            BackendError::InvalidSnapshot(format!("invalid audio mixer state: {error}"))
        })?;
        bus.replace_audio_devices(ym, psg);
        self.bus = bus;
        self.scheduler = scheduler;
        self.z80 = z80;
        self.z80_credit = parsed.z80_credit;
        self.mixer = mixer;
        self.cpu
            .restore_state(parsed.d, parsed.a, parsed.pc, parsed.sr, parsed.halted);
        self.cpu
            .restore_extended_state(parsed.usp, parsed.ssp, parsed.stopped, parsed.cpu_cycles);
        self.tick = parsed.tick;
        // The VINT-pending status bit is derived from the restored scheduler
        // state, so recompute it before anything can inspect the machine. The
        // canonical VBlank bit (bit 3) is serialized and was validated against
        // the snapshot state hash, so it is not reinterpreted here.
        self.bus.vdp_mut().clear_interrupts();
        self.sync_vdp_interrupts();
        self.sync_vdp_vint_pending();
        // The scheduler's diagnostic counters were just restored. Rebase the
        // debug-only request detection on them so a restore never reports a
        // request raised before the snapshot. The event ring, watchpoint table
        // and every other piece of observer state are deliberately left alone:
        // debug state is not canonical and restore does not rewind it.
        let (issued, coalesced) = self.scheduler_counters();
        self.bus
            .debug_observer_mut()
            .sync_scheduler(issued, coalesced);
        // Media queued before a restore belongs to the replaced state.
        self.pending_frame = None;
        self.pending_audio = None;
        Ok(())
    }

    fn inspect(&self) -> Result<MachineInspection, BackendError> {
        let mut registers = self.cpu.registers();
        let pending = self.scheduler.next_pending();
        registers.insert("vdp_status".to_owned(), u64::from(self.bus.vdp().status()));
        registers.insert(
            "vdp_auto_increment".to_owned(),
            u64::from(self.bus.vdp().registers()[AUTO_INCREMENT_REGISTER]),
        );
        registers.insert(
            "vdp_address".to_owned(),
            u64::from(self.bus.vdp().address()),
        );
        registers.insert(
            "irq_mask".to_owned(),
            u64::from((self.cpu.sr() >> 8) & 0x0007),
        );
        registers.insert(
            "irq_pending_level".to_owned(),
            u64::from(pending.map_or(0, InterruptLevel::level)),
        );
        registers.insert(
            "irq_deliverable".to_owned(),
            u64::from(pending.is_some_and(|level| self.cpu.accepts_interrupt_level(level.level()))),
        );
        registers.insert(
            "irq_vector".to_owned(),
            u64::from(pending.map_or(0, vector_address)),
        );
        registers.insert("master_cycles".to_owned(), self.tick.raw());
        registers.insert("cpu_cycles".to_owned(), self.cpu.cycles());
        registers.insert("usp".to_owned(), u64::from(self.cpu.usp()));
        registers.insert("ssp".to_owned(), u64::from(self.cpu.ssp()));
        registers.insert("stopped".to_owned(), u64::from(self.cpu.stopped()));
        // Z80, arbitration and audio pseudo-registers. The register map is an
        // open `BTreeMap`, so these extend the schema without breaking it; every
        // value is a canonical-range `u64`.
        let z80 = &self.z80;
        for (name, value) in [
            ("z80.af", u64::from(z80.af())),
            ("z80.bc", u64::from(z80.bc())),
            ("z80.de", u64::from(z80.de())),
            ("z80.hl", u64::from(z80.hl())),
            ("z80.ix", u64::from(z80.ix())),
            ("z80.iy", u64::from(z80.iy())),
            ("z80.sp", u64::from(z80.sp())),
            ("z80.pc", u64::from(z80.pc())),
            ("z80.i", u64::from(z80.i())),
            ("z80.r", u64::from(z80.r())),
            ("z80.im", u64::from(z80.im())),
            ("z80.iff1", u64::from(z80.iff1())),
            ("z80.iff2", u64::from(z80.iff2())),
            ("z80.halted", u64::from(z80.halted())),
            ("z80.ei_delay", u64::from(z80.ei_delay_pending())),
            ("z80.credit", self.z80_credit),
            ("z80.bank", u64::from(self.bus.z80_bank())),
        ] {
            registers.insert(name.to_owned(), value);
        }
        let arbitration = self.bus.arbitration();
        registers.insert(
            "arbitration.requested".to_owned(),
            u64::from(arbitration.requested()),
        );
        registers.insert(
            "arbitration.reset_asserted".to_owned(),
            u64::from(arbitration.reset_asserted()),
        );
        registers.insert(
            "arbitration.z80_runs".to_owned(),
            u64::from(arbitration.z80_runs()),
        );
        registers.insert("ym.status".to_owned(), u64::from(self.bus.ym().status()));
        registers.insert(
            "ym.lfo_phase".to_owned(),
            u64::from(self.bus.ym().lfo_phase()),
        );
        registers.insert(
            "ym.address_latch0".to_owned(),
            u64::from(self.bus.ym().address_latch(YmBank::One)),
        );
        registers.insert(
            "ym.address_latch1".to_owned(),
            u64::from(self.bus.ym().address_latch(YmBank::Two)),
        );
        let psg = self.bus.psg();
        registers.insert("psg.latch".to_owned(), u64::from(psg.latched_register()));
        for channel in 0..3 {
            registers.insert(
                format!("psg.tone{channel}"),
                u64::from(psg.tone_divider(channel).unwrap_or(0)),
            );
            registers.insert(
                format!("psg.attenuation{channel}"),
                u64::from(psg.attenuation(channel).unwrap_or(0)),
            );
        }
        registers.insert(
            "psg.noise_control".to_owned(),
            u64::from(psg.noise_control()),
        );
        registers.insert("psg.noise_lfsr".to_owned(), u64::from(psg.noise_lfsr()));
        registers.insert(
            "psg.attenuation3".to_owned(),
            u64::from(psg.attenuation(3).unwrap_or(0)),
        );
        registers.insert(
            "audio.pending_frames".to_owned(),
            self.mixer.pending_frames() as u64,
        );
        registers.insert(
            "audio.dropped_frames".to_owned(),
            self.mixer.dropped_frames(),
        );
        registers.insert(
            "audio.frame_accumulator".to_owned(),
            self.mixer.frame_phase().0,
        );
        registers.insert("scheduler_frame".to_owned(), self.scheduler.frame());
        registers.insert(
            "scheduler_line".to_owned(),
            u64::from(self.scheduler.line_in_frame()),
        );
        registers.insert(
            "scheduler_master_cycle_in_line".to_owned(),
            self.scheduler.master_cycle_in_line(),
        );
        // Bounded cartridge pseudo-registers. The full serial text is exposed by
        // `CartridgeView`; the register map is a canonical-range `u64`, so
        // `cartridge.serial` carries the first six ASCII bytes big-endian (zero
        // padded) and always fits the portable wire range.
        let cartridge = self.bus.cartridge();
        let header = cartridge.header();
        registers.insert(
            "cartridge.system".to_owned(),
            match header.system {
                crate::cartridge::CartridgeSystem::MegaDrive => 0,
                crate::cartridge::CartridgeSystem::Genesis => 1,
            },
        );
        registers.insert(
            "cartridge.region".to_owned(),
            u64::from(header.region.bits()),
        );
        registers.insert(
            "cartridge.serial".to_owned(),
            serial_fingerprint(&header.serial),
        );
        registers.insert(
            "cartridge.checksum_stored".to_owned(),
            u64::from(header.checksum_stored),
        );
        registers.insert(
            "cartridge.checksum_computed".to_owned(),
            u64::from(header.checksum_computed),
        );
        for (index, bank) in cartridge.banks().iter().enumerate() {
            registers.insert(format!("mapper.bank{index}"), u64::from(*bank));
        }
        registers.insert(
            "sram.visible".to_owned(),
            u64::from(cartridge.sram_visible()),
        );
        registers.insert(
            "sram.protected".to_owned(),
            u64::from(cartridge.sram_write_protected()),
        );
        registers.insert(
            "tmss.locked".to_owned(),
            u64::from(cartridge.tmss().locked()),
        );
        Ok(MachineInspection {
            tick: self.tick,
            state_hash: self.state_hash(),
            halted: self.cpu.halted(),
            registers,
            memory_base: u64::from(RAM_BASE),
            memory: self.bus.ram().to_vec(),
        })
    }

    fn read_memory(&self, address: u64, length: usize) -> Result<Vec<u8>, BackendError> {
        self.bus.read_memory(address, length).map_err(|error| {
            map_memory_error(
                error,
                address,
                length,
                debugger_window_end(self.bus.rom().len(), address, false),
            )
        })
    }

    fn write_memory(&mut self, address: u64, data: &[u8]) -> Result<(), BackendError> {
        self.bus.write_memory(address, data).map_err(|error| {
            map_memory_error(
                error,
                address,
                data.len(),
                debugger_window_end(self.bus.rom().len(), address, true),
            )
        })
    }

    fn debug_access(&mut self) -> Option<&mut dyn DebugAccess> {
        Some(self)
    }
}

impl DebugAccess for MegadriveBackend {
    fn configure_debug(&mut self, config: DebugConfig) -> Result<(), BackendError> {
        self.bus.debug_configure(config)?;
        // Request detection is a counter delta. Rebaseline it on the current
        // scheduler counters so enabling observation never reports a request
        // that was raised before observation started.
        let (issued, coalesced) = self.scheduler_counters();
        self.bus
            .debug_observer_mut()
            .sync_scheduler(issued, coalesced);
        // Media queued before a restore belongs to the replaced state.
        self.pending_frame = None;
        self.pending_audio = None;
        Ok(())
    }

    fn debug_event_log(&self) -> Option<&DebugEventLog> {
        self.bus.debug_event_log()
    }

    fn take_debug_event_log(&mut self) -> Option<DebugEventLog> {
        self.bus.take_debug_event_log()
    }
}

/// Exclusive end of the mapped window that contains `address`, if any.
///
/// Debugger reads may target the cartridge image, work RAM (primary window or
/// mirror) or the control register file; debugger writes may target work RAM
/// only. A mirror address is bounded by its own 64 KiB block, which is what the
/// bus serves, so an overrun reports the first byte the block cannot serve. The
/// returned value is the end of the window that actually contains the requested
/// start address, so a range overrun can report a
/// [`BackendError::OutOfBounds::limit`] that is greater than the start address
/// and names the region the request exceeded.
///
/// `None` means no mapped window contains the start address (an unmapped hole,
/// the VDP port window, or an address above the 32-bit bus). That is not a range
/// overrun and must not be reported as out-of-bounds.
fn debugger_window_end(rom_len: usize, address: u64, writable: bool) -> Option<u64> {
    // The bus decodes 24-bit addresses, so the window check uses the same
    // truncation; otherwise a mirror address above `0x00FFFFFF` would be
    // classified as an unmapped hole while the bus serves it.
    let start = bus_address(u32::try_from(address).ok()?);
    if (RAM_BASE..RAM_END).contains(&start) {
        return Some(u64::from(RAM_END));
    }
    if start >= RAM_MIRROR_BASE {
        let block_start = start & !0xFFFF;
        return Some(u64::from(block_start) + 0x1_0000);
    }
    if writable {
        // The bus serves a single debugger write window: work RAM. The control
        // register file is a readable window, but `write_memory` reaches it
        // through its own path, so a caller that starts there is classified by
        // the scratch path in `write_memory` instead of this helper.
        return io_control_offset(start)
            .map(|_| u64::from(IO_CONTROL_BASE) + IO_CONTROL_LEN as u64);
    }
    let rom_len = rom_len.min(ROM_END as usize);
    if (start as usize) < rom_len {
        return Some(u64::from(ROM_BASE) + rom_len as u64);
    }
    // The control register file is a readable, side-effect-free window.
    if io_control_offset(start).is_some() {
        return Some(u64::from(IO_CONTROL_BASE) + IO_CONTROL_LEN as u64);
    }
    None
}

/// Maps a side-effect-free debugger bus failure to the backend contract.
///
/// [`BackendError::OutOfBounds`] is reserved for a request that starts inside a
/// mapped window and whose validated range overruns it. In that case
/// `window_end` is the containing window's exclusive end, so `limit` is always
/// greater than the requested start address and explains which region the
/// request exceeded.
///
/// Requests that start outside every mapped window, writes to the read-only
/// cartridge, alignment failures and device failures are execution errors: no
/// containing window end could honestly describe them, and mapping them to
/// `OutOfBounds` would produce a limit above an address that was rejected
/// inside it.
fn map_memory_error(
    error: BusError,
    address: u64,
    length: usize,
    window_end: Option<u64>,
) -> BackendError {
    match error {
        BusError::UnmappedRead { .. }
        | BusError::UnmappedWrite { .. }
        | BusError::AddressOverflow { .. } => match window_end {
            Some(limit) => BackendError::OutOfBounds {
                address,
                length,
                limit,
            },
            None => BackendError::Execution(format!(
                "debugger memory access at {address} for {length} byte(s) is outside any mapped \
                 window: {error}"
            )),
        },
        BusError::WriteProtected { .. }
        | BusError::OddAddress { .. }
        | BusError::Vdp(_)
        | BusError::Cartridge(_)
        | BusError::SideEffectingRead { .. } => BackendError::Execution(error.to_string()),
    }
}

/// Packs the first six serial bytes big-endian for the `u64` register map.
///
/// Six bytes cover 48 bits, so the value always fits the portable service JSON
/// integer range. The complete serial text is exposed by `CartridgeView`.
fn serial_fingerprint(serial: &str) -> u64 {
    let mut bytes = [0_u8; 8];
    for (slot, byte) in bytes[2..].iter_mut().zip(serial.bytes()) {
        *slot = byte;
    }
    u64::from_be_bytes(bytes)
}

fn vector_address(level: InterruptLevel) -> u32 {
    match level {
        InterruptLevel::HBlank => HBLANK_VECTOR_ADDRESS,
        InterruptLevel::VBlank => VBLANK_VECTOR_ADDRESS,
    }
}

/// Maps a CPU execution failure to the backend contract.
///
/// `Halted` and tick overflow have dedicated variants. Every other CPU fault —
/// an illegal opcode, an unsupported encoding, an address error, a stack
/// overflow or a bus fault raised while emulated code ran — is an execution
/// failure: [`BackendError::OutOfBounds`] describes debugger-supplied memory
/// ranges, not faults inside the emulated program, so classifying those as
/// bounds errors would misreport the cause.
/// Maps a Z80/bus failure into the backend error contract.
///
/// The backend API has no Z80-specific variant, so a Z80 failure is classified
/// as [`BackendError::Execution`] with the typed Z80 message preserved; direct
/// users of the core still receive the typed [`Z80Error`].
fn map_z80(error: Z80Error) -> BackendError {
    BackendError::Execution(format!("Z80: {error}"))
}

fn map_cpu(error: CpuError) -> BackendError {
    match error {
        CpuError::Halted => BackendError::Halted,
        other => BackendError::Execution(other.to_string()),
    }
}

fn map_scheduler(error: SchedulerError) -> BackendError {
    match error {
        SchedulerError::TickOverflow => BackendError::TickOverflow,
        other => BackendError::Execution(other.to_string()),
    }
}

fn config_hash_for(rom: &[u8], video: VideoStandard, tmss: TmssModel) -> StateHash {
    let mut bytes = Writer::with_capacity(CONFIG_MAGIC.len() + 32 + 1 + 1 + 16);
    bytes.write_bytes(CONFIG_MAGIC);
    // The ROM identity is a SHA-256 digest; SRAM bytes are state and are never
    // part of the configuration.
    bytes.write_bytes(hash_bytes(rom).as_bytes());
    bytes.write_u8(video.code());
    bytes.write_u8(tmss_model_code(tmss));
    bytes.write_bytes(CONFIG_FEATURE_TAG);
    hash_bytes(&bytes.into_bytes())
}

/// Returns the one-byte configuration encoding of a TMSS model.
const fn tmss_model_code(model: TmssModel) -> u8 {
    match model {
        TmssModel::Disabled => 0,
        TmssModel::Enabled => 1,
    }
}

/// Decodes the one-byte configuration/snapshot encoding of a TMSS model.
fn tmss_model_from_code(code: u8) -> Option<TmssModel> {
    match code {
        0 => Some(TmssModel::Disabled),
        1 => Some(TmssModel::Enabled),
        _ => None,
    }
}

/// The complete mutable state that a state hash covers.
struct StateView<'a> {
    d: &'a [u32; 8],
    a: &'a [u32; 8],
    pc: u32,
    sr: u16,
    halted: bool,
    usp: u32,
    ssp: u32,
    cpu_cycles: u64,
    stopped: bool,
    tick: Tick,
    ram: &'a [u8],
    vdp: &'a [u8],
    banks: &'a [u16; 7],
    sram_control: u8,
    sram: &'a [u8],
    tmss_model: TmssModel,
    tmss_locked: bool,
    scheduler: &'a [u8],
    z80: &'a [u8],
    z80_ram: &'a [u8],
    z80_credit: u64,
    arbitration: BusArbitration,
    z80_bank: u16,
    io_control: &'a [u8; IO_CONTROL_LEN],
    ym: &'a [u8],
    psg: &'a [u8],
    mixer: &'a [u8],
}

/// Serializes the canonical mutable state body shared by the snapshot payload
/// and the state hash.
///
/// Layout, in order: the eight data registers and eight address registers as
/// big-endian `u32`s, `PC` as a big-endian `u32`, `SR` as a big-endian `u16`,
/// one halted byte, the tick as a big-endian `u64`, 64 KiB of work RAM, the VDP
/// snapshot, the seven big-endian mapper bank registers, the SRAM control byte,
/// the 64 KiB battery SRAM, the TMSS model and lock bytes, the variable-length
/// interrupt-scheduler snapshot, the 43-byte Z80 register snapshot, the 8 KiB Z80
/// RAM, the Z80 master-cycle carry, the two arbitration latch bytes, the
/// nine-bit Z80 bank register, the fixed-length YM2612, PSG and mixer
/// snapshots. The payload prefixes [`PAYLOAD_MAGIC`] and [`PAYLOAD_VERSION`]; the
/// state hash prefixes [`STATE_MAGIC`]. Both must consume these exact bytes.
fn state_body(state: &StateView<'_>) -> Vec<u8> {
    let mut body = Writer::with_capacity(
        32 + 32
            + 4
            + 2
            + 1
            + 8
            + 4
            + 4
            + 8
            + 1
            + state.ram.len()
            + state.vdp.len()
            + 14
            + 1
            + state.sram.len()
            + 2
            + state.scheduler.len()
            + AUDIO_STATE_TAIL_LEN,
    );
    for value in state.d {
        body.write_u32_be(*value);
    }
    for value in state.a {
        body.write_u32_be(*value);
    }
    body.write_u32_be(state.pc);
    body.write_u16_be(state.sr);
    body.write_u8(u8::from(state.halted));
    body.write_u64_be(state.tick.raw());
    body.write_u32_be(state.usp);
    body.write_u32_be(state.ssp);
    body.write_u64_be(state.cpu_cycles);
    body.write_u8(u8::from(state.stopped));
    body.write_bytes(state.ram);
    body.write_bytes(state.vdp);
    for bank in state.banks {
        body.write_u16_be(*bank);
    }
    body.write_u8(state.sram_control);
    body.write_bytes(state.sram);
    body.write_u8(tmss_model_code(state.tmss_model));
    body.write_u8(u8::from(state.tmss_locked));
    body.write_bytes(state.scheduler);
    body.write_bytes(state.z80);
    body.write_bytes(state.z80_ram);
    body.write_u64_be(state.z80_credit);
    body.write_u8(u8::from(state.arbitration.requested()));
    body.write_u8(u8::from(state.arbitration.reset_asserted()));
    body.write_u16_be(state.z80_bank);
    body.write_bytes(state.io_control);
    body.write_bytes(state.ym);
    body.write_bytes(state.psg);
    body.write_bytes(state.mixer);
    body.into_bytes()
}

fn payload_from_body(body: &[u8]) -> Vec<u8> {
    let mut payload = Writer::with_capacity(PAYLOAD_MAGIC.len() + 2 + body.len());
    payload.write_bytes(&PAYLOAD_MAGIC);
    payload.write_u16_le(PAYLOAD_VERSION);
    payload.write_bytes(body);
    payload.into_bytes()
}

fn hash_state_body(body: &[u8]) -> StateHash {
    let mut bytes = Writer::with_capacity(STATE_MAGIC.len() + body.len());
    bytes.write_bytes(STATE_MAGIC);
    bytes.write_bytes(body);
    hash_bytes(&bytes.into_bytes())
}

fn state_hash_for(state: &StateView<'_>) -> StateHash {
    hash_state_body(&state_body(state))
}

#[derive(Debug)]
struct ParsedState<'a> {
    d: [u32; 8],
    a: [u32; 8],
    pc: u32,
    sr: u16,
    halted: bool,
    usp: u32,
    ssp: u32,
    cpu_cycles: u64,
    stopped: bool,
    tick: Tick,
    ram: &'a [u8],
    vdp: &'a [u8],
    banks: [u16; 7],
    sram_control: u8,
    sram: &'a [u8],
    tmss_model: TmssModel,
    tmss_locked: bool,
    scheduler: &'a [u8],
    z80: &'a [u8],
    z80_ram: &'a [u8],
    z80_credit: u64,
    arbitration: BusArbitration,
    z80_bank: u16,
    io_control: [u8; IO_CONTROL_LEN],
    ym: &'a [u8],
    psg: &'a [u8],
    mixer: &'a [u8],
}

fn parse_payload(payload: &[u8]) -> Result<ParsedState<'_>, BackendError> {
    let mut reader = Reader::new(payload);
    if reader.read_array::<8>().map_err(payload_error)? != PAYLOAD_MAGIC {
        return Err(BackendError::InvalidSnapshot(
            "Mega Drive payload magic mismatch".to_owned(),
        ));
    }
    let version = reader.read_u16_le().map_err(payload_error)?;
    if version != PAYLOAD_VERSION {
        return Err(BackendError::InvalidSnapshot(format!(
            "Mega Drive payload version {version}, expected {PAYLOAD_VERSION}"
        )));
    }
    let mut d = [0_u32; 8];
    for value in &mut d {
        *value = reader.read_u32_be().map_err(payload_error)?;
    }
    let mut a = [0_u32; 8];
    for value in &mut a {
        *value = reader.read_u32_be().map_err(payload_error)?;
    }
    let pc = reader.read_u32_be().map_err(payload_error)?;
    let sr = reader.read_u16_be().map_err(payload_error)?;
    let halted = reader.read_u8().map_err(payload_error)? != 0;
    let tick = Tick::from_raw(reader.read_u64_be().map_err(payload_error)?);
    let usp = reader.read_u32_be().map_err(payload_error)?;
    let ssp = reader.read_u32_be().map_err(payload_error)?;
    let cpu_cycles = reader.read_u64_be().map_err(payload_error)?;
    let stopped = match reader.read_u8().map_err(payload_error)? {
        0 => false,
        1 => true,
        _ => {
            return Err(BackendError::InvalidSnapshot(
                "invalid CPU stopped tag".to_owned(),
            ));
        }
    };
    let ram = reader.read_exact(RAM_SIZE).map_err(payload_error)?;
    let vdp = reader.read_exact(VDP_SNAPSHOT_LEN).map_err(payload_error)?;
    let mut banks = [0_u16; 7];
    for bank in &mut banks {
        *bank = reader.read_u16_be().map_err(payload_error)?;
    }
    let sram_control = reader.read_u8().map_err(payload_error)?;
    let sram = reader.read_exact(SRAM_SIZE).map_err(payload_error)?;
    let tmss_model = tmss_model_from_code(reader.read_u8().map_err(payload_error)?)
        .ok_or_else(|| BackendError::InvalidSnapshot("invalid TMSS model tag".to_owned()))?;
    let tmss_locked = match reader.read_u8().map_err(payload_error)? {
        0 => false,
        1 => true,
        _ => {
            return Err(BackendError::InvalidSnapshot(
                "invalid TMSS lock tag".to_owned(),
            ));
        }
    };
    let remaining = reader.remaining();
    if remaining < AUDIO_STATE_TAIL_LEN {
        return Err(BackendError::InvalidSnapshot(
            "Mega Drive payload is truncated before the Z80/audio state".to_owned(),
        ));
    }
    let scheduler_len = remaining - AUDIO_STATE_TAIL_LEN;
    let scheduler = reader.read_exact(scheduler_len).map_err(payload_error)?;
    let z80 = reader.read_exact(Z80_SNAPSHOT_LEN).map_err(payload_error)?;
    let z80_ram = reader.read_exact(Z80_RAM_SIZE).map_err(payload_error)?;
    let z80_credit = reader.read_u64_be().map_err(payload_error)?;
    let requested = match reader.read_u8().map_err(payload_error)? {
        0 => false,
        1 => true,
        _ => {
            return Err(BackendError::InvalidSnapshot(
                "invalid bus-request latch tag".to_owned(),
            ));
        }
    };
    let reset_asserted = match reader.read_u8().map_err(payload_error)? {
        0 => false,
        1 => true,
        _ => {
            return Err(BackendError::InvalidSnapshot(
                "invalid Z80 reset latch tag".to_owned(),
            ));
        }
    };
    let z80_bank = reader.read_u16_be().map_err(payload_error)?;
    if z80_bank > 0x01FF {
        return Err(BackendError::InvalidSnapshot(
            "Z80 bank register is wider than nine bits".to_owned(),
        ));
    }
    let mut io_control = [0_u8; IO_CONTROL_LEN];
    io_control.copy_from_slice(reader.read_exact(IO_CONTROL_LEN).map_err(payload_error)?);
    let ym = reader.read_exact(YM_SNAPSHOT_LEN).map_err(payload_error)?;
    let psg = reader.read_exact(PSG_SNAPSHOT_LEN).map_err(payload_error)?;
    let mixer = reader
        .read_exact(MIXER_SNAPSHOT_LEN)
        .map_err(payload_error)?;
    Ok(ParsedState {
        d,
        a,
        pc,
        sr,
        halted,
        usp,
        ssp,
        cpu_cycles,
        stopped,
        tick,
        ram,
        vdp,
        banks,
        sram_control,
        sram,
        tmss_model,
        tmss_locked,
        scheduler,
        z80,
        z80_ram,
        z80_credit,
        arbitration: BusArbitration::from_parts(requested, reset_asserted),
        z80_bank,
        io_control,
        ym,
        psg,
        mixer,
    })
}

/// Translates a shared codec failure into the backend snapshot error contract.
fn payload_error(error: CodecError) -> BackendError {
    match error {
        CodecError::Truncated { .. } => {
            BackendError::InvalidSnapshot("Mega Drive payload is truncated".to_owned())
        }
        CodecError::LengthOverflow => {
            BackendError::InvalidSnapshot("payload length arithmetic overflow".to_owned())
        }
        other => BackendError::InvalidSnapshot(format!("invalid Mega Drive payload: {other}")),
    }
}

/// Builds an independently authored synthetic ROM used by tests and the CLI demo.
///
/// The program computes `1 + 2`, stores the word at work-RAM address `0x00FF0000`,
/// reloads it into `D2`, then programs VDP register `0x0F` (auto-increment 2),
/// writes and reads back the VRAM word `0x1234` at VRAM address `0`, stores the
/// read-back word at `0x00FF0004` and the VDP status at `0x00FF0006`, and loops.
///
/// The autovector table entries at `0x70` (HBlank) and `0x78` (VBlank) point at
/// tiny handlers that increment `D7`/`D6` and loop. The program never writes
/// `SR`, so the reset interrupt-priority mask 7 keeps both VDP levels pending and
/// these handlers are not entered; [`interrupt_rom`] is the mask-lowering,
/// `RTE`-returning interrupt demo. The ROM uses no copyrighted data and is not a
/// game.
#[must_use]
pub fn demo_rom() -> Vec<u8> {
    let mut rom = vec![0_u8; 0x400];
    for word in rom.chunks_exact_mut(2) {
        word.copy_from_slice(&[0x4E, 0x71]); // NOP fill
    }
    rom[0..4].copy_from_slice(&0x0100_0000_u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());
    rom[0x70..0x74].copy_from_slice(&0x0000_02C0_u32.to_be_bytes());
    rom[0x78..0x7C].copy_from_slice(&0x0000_02E0_u32.to_be_bytes());
    rom[0x200..0x214].copy_from_slice(&[
        0x70, 0x01, // MOVEQ #1,D0
        0x72, 0x02, // MOVEQ #2,D1
        0xD0, 0x41, // ADD.W D1,D0
        0x33, 0xC0, // MOVE.W D0,(0x00FF0000).L
        0x00, 0xFF, 0x00, 0x00, // absolute-long address
        0x74, 0x00, // MOVEQ #0,D2
        0x34, 0x39, // MOVE.W (0x00FF0000).L,D2
        0x00, 0xFF, 0x00, 0x00, // absolute-long address
    ]);
    rom[0x214..0x26A].copy_from_slice(&[
        0x30, 0x3C, 0x8F, 0x02, // MOVE.W #0x8F02,D0 (register 15 = 2)
        0x33, 0xC0, 0x00, 0xC0, 0x00, 0x04, // MOVE.W D0,(0x00C00004).L
        0x30, 0x3C, 0x40, 0x00, // MOVE.W #0x4000,D0 (VRAM write, address 0)
        0x33, 0xC0, 0x00, 0xC0, 0x00, 0x04, // MOVE.W D0,(0x00C00004).L
        0x30, 0x3C, 0x00, 0x00, // MOVE.W #0,D0 (address bits 15:14 = 0)
        0x33, 0xC0, 0x00, 0xC0, 0x00, 0x04, // MOVE.W D0,(0x00C00004).L
        0x30, 0x3C, 0x12, 0x34, // MOVE.W #0x1234,D0
        0x33, 0xC0, 0x00, 0xC0, 0x00, 0x00, // MOVE.W D0,(0x00C00000).L
        0x30, 0x3C, 0x00, 0x00, // MOVE.W #0x0000,D0 (VRAM read, address 0)
        0x33, 0xC0, 0x00, 0xC0, 0x00, 0x04, // MOVE.W D0,(0x00C00004).L
        0x30, 0x3C, 0x00, 0x00, // MOVE.W #0,D0 (address bits 15:14 = 0)
        0x33, 0xC0, 0x00, 0xC0, 0x00, 0x04, // MOVE.W D0,(0x00C00004).L
        0x34, 0x39, 0x00, 0xC0, 0x00, 0x00, // MOVE.W (0x00C00000).L,D2
        0x33, 0xC2, 0x00, 0xFF, 0x00, 0x04, // MOVE.W D2,(0x00FF0004).L
        0x36, 0x39, 0x00, 0xC0, 0x00, 0x04, // MOVE.W (0x00C00004).L,D3
        0x33, 0xC3, 0x00, 0xFF, 0x00, 0x06, // MOVE.W D3,(0x00FF0006).L
        0x60, 0xFE, // BRA.S -2
    ]);
    rom[0x2C0..0x2C4].copy_from_slice(&[
        0x52, 0x47, // ADDQ.W #1,D7
        0x60, 0xFE, // BRA.S -2
    ]);
    rom[0x2E0..0x2E4].copy_from_slice(&[
        0x52, 0x46, // ADDQ.W #1,D6
        0x60, 0xFE, // BRA.S -2
    ]);
    stamp_synthetic_header(&mut rom);
    rom
}

/// Builds an independently authored synthetic ROM that exercises mask-gated
/// interrupt delivery and `RTE`.
///
/// The main program at `0x200`:
///
/// 1. explicitly masks both VDP interrupt levels with `MOVE.W #$2700,SR`;
/// 2. idles for six `NOP`s, so the first HBlank request is raised and retained
///    while it is masked;
/// 3. installs the 68000 interrupt-priority mask `mask` with
///    `MOVE.W #(0x2000 | mask << 8),SR`, which is the mask-lowering step;
/// 4. clears `D0` and enters a loop that reads the VDP status port into `D5` and
///    branches back with `BRA.S`.
///
/// The HBlank autovector at `0x70` points at a handler that increments `D7`,
/// sets `N` with `MOVEQ #-1,D3` and returns with `RTE`; the VBlank autovector at
/// `0x78` does the same to `D6`. Using a non-zero handler body (rather than an
/// immediate return) makes both the handler execution and the `RTE` restoration
/// of the saved raw `SR` (`IPM`, `S` and `CCR`) observable.
///
/// 68000 exception entry loads the accepted level into the interrupt-priority
/// mask ([`M68000::interrupt`]): a HBlank handler runs with mask 4 and a VBlank
/// handler with mask 6, so a pending lower-priority VDP request is not delivered
/// while the handler runs. `RTE` restores the interrupted program's raw `SR`,
/// after which a retained request can be delivered. The first HBlank is raised
/// while the mask is still 7, the lowering instruction executes at instruction 8,
/// and the retained request is delivered at instruction 9.
///
/// Passing `3` builds the delivery scenario: HBlank (level 4) and VBlank
/// (level 6) are both above mask 3, so both are eventually delivered. Passing `7`
/// keeps every VDP request masked forever and exercises only the pending side
/// (pending HBlank/VBlank, VINT-pending status bit, status-port reads that must
/// not acknowledge). The ROM contains no copyrighted data and is not a game.
///
/// # Panics
///
/// Panics when `mask` exceeds 7, because the 68000 interrupt-priority mask is
/// three bits wide and cannot be encoded in this program.
#[must_use]
pub fn interrupt_rom(mask: u8) -> Vec<u8> {
    assert!(
        mask <= 7,
        "interrupt_rom mask must fit the 68000 IPM (0..=7)"
    );
    let mut rom = vec![0_u8; 0x400];
    for word in rom.chunks_exact_mut(2) {
        word.copy_from_slice(&[0x4E, 0x71]); // NOP fill
    }
    rom[0..4].copy_from_slice(&0x0100_0000_u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());
    rom[0x70..0x74].copy_from_slice(&0x0000_02C0_u32.to_be_bytes());
    rom[0x78..0x7C].copy_from_slice(&0x0000_02E0_u32.to_be_bytes());

    let mut main = vec![
        0x46, 0xFC, 0x27, 0x00, // MOVE.W #$2700,SR (explicitly mask the VDP levels)
        0x33, 0xFC, 0x80, 0x30, 0x00, 0xC0, 0x00, 0x04, // MOVE.W #$8030,(0x00C00004)
        0x30, 0x3C, 0x00, 0x64, // MOVE.W #100,D0 (busy delay over two scanlines)
        0x51, 0xC8, 0xFF, 0xFE, // DBRA D0,*-2 (10 cycles per iteration)
        0x46, 0xFC, // MOVE.W #imm,SR: lower the mask to `mask`
    ];
    let lowered_status = 0x2000_u16 | (u16::from(mask) << 8);
    main.extend_from_slice(&lowered_status.to_be_bytes());
    main.extend_from_slice(&[
        0x70, 0x00, // MOVEQ #0,D0 (sets Z)
        0x3A, 0x39, 0x00, 0xC0, 0x00, 0x04, // MOVE.W (0x00C00004).L,D5 (VDP status)
        0x60, 0xF8, // BRA.S back to the status read
    ]);
    rom[0x200..0x200 + main.len()].copy_from_slice(&main);

    rom[0x2C0..0x2C6].copy_from_slice(&[
        0x52, 0x47, // ADDQ.W #1,D7 (HBlank counter)
        0x76, 0xFF, // MOVEQ #-1,D3 (sets N to prove RTE restores CCR)
        0x4E, 0x73, // RTE
    ]);
    rom[0x2E0..0x2E6].copy_from_slice(&[
        0x52, 0x46, // ADDQ.W #1,D6 (VBlank counter)
        0x76, 0xFF, // MOVEQ #-1,D3
        0x4E, 0x73, // RTE
    ]);
    stamp_synthetic_header(&mut rom);
    rom
}

/// Builds an independently authored synthetic ROM that observes both controller
/// data registers and records what it read in work RAM.
///
/// The program at `0x200` clears `D0`-`D3`, then loops:
///
/// 1. reads controller port 1 (`0x00A10003`) with `MOVE.B` into `D2`;
/// 2. accumulates that byte into `D0` with `ADD.B` and stores `D0` at
///    `0x00FF0000`;
/// 3. stores the last observed port-1 byte at `0x00FF0002`;
/// 4. reads controller port 2 (`0x00A10005`) into `D3`;
/// 5. accumulates that byte into `D1` with `ADD.B` and stores `D1` at
///    `0x00FF0001`;
/// 6. stores the last observed port-2 byte at `0x00FF0003`;
/// 7. branches back with `BRA.S`.
///
/// The observed controller values therefore become canonical work RAM (and hence
/// state hash) bytes: a non-neutral port changes the machine state at the first
/// loop read, and neutral input accumulates the neutral data byte `0x7F` in both
/// accumulators. The program never writes `SR`, a VDP register or a controller
/// register and sets no interrupt handler, so at the reset interrupt-priority
/// mask 7 the VDP requests stay masked and the loop is interrupt-free. It uses
/// only instruction forms already implemented and tested by this slice and is
/// not a real-hardware read routine: it does not toggle TH and cannot observe A
/// or Start. The ROM contains no copyrighted data and is not a game.
#[must_use]
pub fn controller_rom() -> Vec<u8> {
    let mut rom = vec![0_u8; 0x400];
    for word in rom.chunks_exact_mut(2) {
        word.copy_from_slice(&[0x4E, 0x71]); // NOP fill
    }
    rom[0..4].copy_from_slice(&0x0100_0000_u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());

    let main = vec![
        0x70, 0x00, // MOVEQ #0,D0 (port-1 accumulator)
        0x72, 0x00, // MOVEQ #0,D1 (port-2 accumulator)
        0x74, 0x00, // MOVEQ #0,D2 (last port-1 observation)
        0x76, 0x00, // MOVEQ #0,D3 (last port-2 observation)
        // loop: read, accumulate and record both controller ports.
        0x14, 0x39, 0x00, 0xA1, 0x00, 0x03, // MOVE.B (0x00A10003).L,D2
        0xD0, 0x02, // ADD.B D2,D0
        0x13, 0xC0, 0x00, 0xFF, 0x00, 0x00, // MOVE.B D0,(0x00FF0000).L
        0x13, 0xC2, 0x00, 0xFF, 0x00, 0x02, // MOVE.B D2,(0x00FF0002).L
        0x16, 0x39, 0x00, 0xA1, 0x00, 0x05, // MOVE.B (0x00A10005).L,D3
        0xD2, 0x03, // ADD.B D3,D1
        0x13, 0xC1, 0x00, 0xFF, 0x00, 0x01, // MOVE.B D1,(0x00FF0001).L
        0x13, 0xC3, 0x00, 0xFF, 0x00, 0x03, // MOVE.B D3,(0x00FF0003).L
        // BRA.S back to the port-1 read at 0x208. The displacement is measured
        // from the following instruction (0x232), so it is -42 (0xD6).
        0x60, 0xD6,
    ];
    rom[0x200..0x200 + main.len()].copy_from_slice(&main);
    stamp_synthetic_header(&mut rom);
    rom
}

/// Size of the synthetic boot cartridge image: 2 MiB, enough for bank 2's
/// `0x100000..0x17FFFF` page to be mapped.
pub const BOOT_ROM_SIZE: usize = 0x0020_0000;
/// Address of the boot program's final self-loop.
pub const BOOT_MARKER: u32 = 0x0000_0224;

/// Returns the address of the synthetic boot program's final self-loop.
#[must_use]
pub const fn boot_marker() -> u32 {
    BOOT_MARKER
}

/// Builds the independently authored synthetic boot cartridge.
///
/// The image carries a valid `SEGA MEGA DRIVE` header (region `JUE`, so the
/// factory derives NTSC) and a 68000 program at `0x200` that:
///
/// 1. writes the `"SEGA"` long word to `0xA14000`, releasing the TMSS lockout;
/// 2. enables the SRAM window through `0xA130F1`;
/// 3. selects mapper bank 2 in the first bank window (`0xA130F3`);
/// 4. writes `0xC7` to the first SRAM byte;
/// 5. enters a `BRA.S -2` self-loop at [`boot_marker`].
///
/// The image is 2 MiB so the selected bank page is inside it. It contains no
/// copyrighted data and is not a game.
#[must_use]
pub fn cartridge_boot_rom() -> Vec<u8> {
    let mut rom = vec![0_u8; BOOT_ROM_SIZE];
    for word in rom.chunks_exact_mut(2) {
        word.copy_from_slice(&[0x4E, 0x71]); // NOP fill
    }
    rom[0..4].copy_from_slice(&0x0100_0000_u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());
    let program: &[u8] = &[
        0x20, 0x3C, 0x53, 0x45, 0x47, 0x41, // MOVE.L #$53454741,D0 ("SEGA")
        0x23, 0xC0, 0x00, 0xA1, 0x40, 0x00, // MOVE.L D0,($00A14000).L
        0x70, 0x01, // MOVEQ #1,D0
        0x13, 0xC0, 0x00, 0xA1, 0x30, 0xF1, // MOVE.B D0,($00A130F1).L
        0x70, 0x02, // MOVEQ #2,D0
        0x13, 0xC0, 0x00, 0xA1, 0x30, 0xF3, // MOVE.B D0,($00A130F3).L
        0x70, 0xC7, // MOVEQ #-57,D0 (low byte 0xC7)
        0x13, 0xC0, 0x00, 0x20, 0x00, 0x00, // MOVE.B D0,($00200000).L
        0x60, 0xFE, // BRA.S -2 (self-loop marker)
    ];
    rom[0x200..0x200 + program.len()].copy_from_slice(program);
    stamp_synthetic_header(&mut rom);
    rom
}

/// Stamps the shared synthetic `SEGA MEGA DRIVE` header with region `JUE`.
fn stamp_synthetic_header(rom: &mut [u8]) {
    assert!(
        crate::test_support::stamp_header(rom, "JUE"),
        "synthetic ROM images must be at least 512 bytes"
    );
}

/// Maps a cartridge construction failure to the backend configuration contract.
fn map_cartridge_config(error: CartridgeError) -> BackendError {
    BackendError::InvalidConfiguration(format!("invalid cartridge: {error}"))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chronofork_backend_api::{Backend, BackendError, BackendFactory, BackendSnapshot};
    use chronofork_core::InputFrame;
    use chronofork_sessions::SessionManager;

    use super::{
        HBLANK_VECTOR_ADDRESS, MegadriveBackend, MegadriveFactory, PAYLOAD_MAGIC, PAYLOAD_VERSION,
        SNAPSHOT_PAYLOAD_BOUND, VBLANK_VECTOR_ADDRESS, controller_rom, demo_rom, interrupt_rom,
        map_scheduler, map_z80, stamp_synthetic_header,
    };
    use crate::cartridge::{Cartridge, SRAM_SIZE, TmssModel};
    /// First index of the port 3/EXT control stub inside the control file.
    const IO_CONTROL_STUB_OFFSET: usize = 4;
    use crate::bus::{
        BusError, CONTROLLER_NEUTRAL, CONTROLLER_PORT_1, CONTROLLER_PORT_2, IO_CONTROL_BASE,
        IO_CONTROL_LEN, RAM_BASE, RAM_END, RAM_SIZE, ROM_END,
    };
    use crate::cycles::MASTER_CYCLES_PER_LINE;
    use crate::interrupts::{
        InterruptLevel, InterruptScheduler, NTSC_ACTIVE_LINES, NTSC_LINES_PER_FRAME,
        SCHEDULER_MAX_SNAPSHOT_LEN, SCHEDULER_PENDING_SLOT_LEN, SchedulerConfig, SchedulerError,
        VideoStandard,
    };
    use crate::vdp::{
        SNAPSHOT_MAGIC, STATUS_FIFO_EMPTY, STATUS_VBLANK, STATUS_VINT_PENDING, VdpError,
    };
    use crate::z80::{Z80Bus, Z80Error};

    fn factory() -> MegadriveFactory {
        MegadriveFactory::new(demo_rom()).expect("valid demo ROM")
    }

    #[test]
    fn the_z80_undecoded_counter_is_not_canonical() {
        // The counter is diagnostic: two machines that differ only in how many
        // guest accesses were served by the undecoded path must serialize to the
        // same payload, so no payload version bump is needed (W1R).
        let quiet = factory().create_machine().expect("machine");
        let mut noisy = factory().create_machine().expect("machine");
        assert_eq!(quiet.bus.unmapped_z80_accesses(), 0);
        Z80Bus::write8(&mut noisy.bus, 0x7F64, 0x5A).expect("undecoded write");
        Z80Bus::read8(&mut noisy.bus, 0x6100).expect("undecoded read");
        assert_eq!(noisy.bus.unmapped_z80_accesses(), 2);

        let quiet = quiet.snapshot().expect("snapshot");
        let noisy = noisy.snapshot().expect("snapshot");
        assert_eq!(quiet.payload().len(), noisy.payload().len());
        assert_eq!(
            quiet.payload(),
            noisy.payload(),
            "a diagnostic count cannot move the canonical payload"
        );
    }

    #[test]
    fn used_backend_restore_resets_z80_counter_and_keeps_payload_identical() {
        // The backend restore clones the bus, validates, then commits; the
        // diagnostic counter describes execution since the restore, so the
        // replacement state starts at zero (W1T/F4) while the payload stays
        // byte-identical because the counter is not canonical.
        let clean = factory()
            .create_machine()
            .expect("machine")
            .snapshot()
            .expect("snapshot");
        let mut used = factory().create_machine().expect("machine");
        Z80Bus::write8(&mut used.bus, 0x7F64, 0x5A).expect("undecoded write");
        assert_eq!(used.bus.unmapped_z80_accesses(), 1);

        used.restore(&clean).expect("restore");
        assert_eq!(
            used.bus.unmapped_z80_accesses(),
            0,
            "a valid restore resets the Z80 diagnostic count"
        );
        let restored = used.snapshot().expect("snapshot");
        assert_eq!(restored.payload(), clean.payload());

        // A rejected restore keeps the machine and its diagnostic intact.
        Z80Bus::write8(&mut used.bus, 0x6100, 0x5A).expect("undecoded write");
        assert_eq!(used.bus.unmapped_z80_accesses(), 1);
        // Tamper the payload magic so the restore is rejected before it commits.
        let mut corrupt = clean.payload().to_vec();
        corrupt[..8].copy_from_slice(b"CFMD68KX");
        let corrupt = BackendSnapshot::new(
            clean.backend_id().clone(),
            clean.backend_version().to_owned(),
            clean.format_version(),
            clean.tick(),
            clean.config_hash(),
            clean.state_hash(),
            corrupt,
        );
        assert!(used.restore(&corrupt).is_err());
        assert_eq!(
            used.bus.unmapped_z80_accesses(),
            1,
            "a rejected restore keeps the previous diagnostic"
        );
    }

    #[test]
    fn host_level_errors_are_never_swallowed() {
        // The guest-access conversion is per channel and deliberate; host-level
        // failures keep their typed identity. InvalidConfiguration and
        // InvalidSnapshot have their own rejection tests in this module.
        assert!(matches!(
            map_z80(Z80Error::SnapshotMagic),
            BackendError::Execution(_)
        ));
        assert!(matches!(
            map_scheduler(SchedulerError::TickOverflow),
            BackendError::TickOverflow
        ));
        assert!(matches!(
            map_scheduler(SchedulerError::SnapshotMagicMismatch),
            BackendError::Execution(_)
        ));
    }

    /// Factory for the mask-gating synthetic ROM with a four-instruction line.
    fn interrupt_factory(mask: u8) -> MegadriveFactory {
        let config = SchedulerConfig::ntsc();
        MegadriveFactory::with_scheduler_config(interrupt_rom(mask), config)
            .expect("valid interrupt ROM")
    }

    fn interrupt_machine(mask: u8) -> MegadriveBackend {
        interrupt_factory(mask)
            .create_machine()
            .expect("interrupt machine")
    }

    /// Factory for the controller-observing synthetic ROM.
    fn controller_factory() -> MegadriveFactory {
        MegadriveFactory::new(controller_rom()).expect("valid controller ROM")
    }

    /// Builds a ROM whose first instruction is the unimplemented `ILLEGAL`
    /// opcode, so a step fails after the transient input has been set.
    fn illegal_rom() -> Vec<u8> {
        let mut rom = [0x4E_u8, 0x71].repeat(0x200); // 0x400 bytes of NOP fill
        rom[0..4].copy_from_slice(&0x0100_0000_u32.to_be_bytes());
        rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());
        rom[0x200..0x202].copy_from_slice(&[0x4A, 0xFC]);
        super::stamp_synthetic_header(&mut rom);
        rom
    }

    /// Steps a controller machine with the same frame and returns the observed
    /// work-RAM bytes after `steps` steps.
    fn controller_run(steps: usize, input: InputFrame) -> [u8; 4] {
        let mut machine = controller_factory().create_machine().expect("machine");
        for _ in 0..steps {
            machine.step(input).expect("controller step");
        }
        let memory = machine.inspect().expect("inspect").memory;
        [memory[0], memory[1], memory[2], memory[3]]
    }

    /// Steps until the scheduler tick reaches `tick`.
    /// Steps until `predicate` holds, failing rather than looping forever.
    fn step_until(
        machine: &mut MegadriveBackend,
        predicate: impl Fn(&MegadriveBackend) -> bool,
        what: &str,
    ) {
        for _ in 0..2_000_000 {
            if predicate(machine) {
                return;
            }
            machine.step(InputFrame::NEUTRAL).expect("step until");
        }
        unreachable!("timed out waiting for {what}");
    }

    fn register(machine: &MegadriveBackend, name: &str) -> u64 {
        machine.inspect().expect("inspect").registers[name]
    }

    #[test]
    fn reset_vectors_and_demo_program_execute() {
        let factory = factory();
        assert!(factory.descriptor().capabilities().rendering);
        let mut machine = factory.create_machine().expect("machine");
        let initial = machine.inspect().expect("inspect");
        assert_eq!(initial.registers["pc"], 0x200);
        assert_eq!(initial.registers["a7"], 0x0100_0000);
        for _ in 0..6 {
            machine.step(InputFrame::NEUTRAL).expect("step");
        }
        let state = machine.inspect().expect("inspect");
        assert_eq!(state.registers["d0"], 3);
        assert_eq!(state.registers["d1"], 2);
        assert_eq!(state.registers["d2"], 3);
        assert_eq!(state.memory[0..2], [0x00, 0x03]);
        assert_eq!(state.tick.raw(), 336);
        assert_eq!(state.tick.raw(), machine.cpu().cycles() * 7);
    }

    #[test]
    fn demo_program_programs_vdp_and_reads_back_vram() {
        let factory = factory();
        let mut machine = factory.create_machine().expect("machine");
        for _ in 0..23 {
            machine.step(InputFrame::NEUTRAL).expect("step");
        }
        assert_eq!(machine.vdp().register(15), Some(2));
        assert_eq!(&machine.vdp().vram()[0..2], &[0x12, 0x34]);
        assert_eq!(machine.vdp().address(), 2);
        assert_eq!(machine.vdp().status(), STATUS_FIFO_EMPTY);

        let state = machine.inspect().expect("inspect");
        assert_eq!(state.memory[4..6], [0x12, 0x34]);
        assert_eq!(state.memory[6..8], [0x01, 0x00]);
        assert_eq!(state.registers["vdp_auto_increment"], 2);
        assert_eq!(state.registers["vdp_status"], u64::from(STATUS_FIFO_EMPTY));
        assert_eq!(state.registers["irq_pending_level"], 0);
        assert_eq!(state.registers["irq_vector"], 0);
        assert_eq!(state.registers["irq_mask"], 7);
    }

    #[test]
    fn interrupt_rom_layout_is_independent_and_matches_its_documentation() {
        let rom = interrupt_rom(3);
        assert_eq!(rom.len(), 0x400);
        assert_eq!(&rom[0..4], &0x0100_0000_u32.to_be_bytes());
        assert_eq!(&rom[4..8], &0x0000_0200_u32.to_be_bytes());
        assert_eq!(&rom[0x70..0x74], &0x0000_02C0_u32.to_be_bytes());
        assert_eq!(&rom[0x78..0x7C], &0x0000_02E0_u32.to_be_bytes());

        // MOVE.W #$2700,SR, enable the VDP interrupts, a 100-iteration DBRA
        // delay, then MOVE.W #$2300,SR.
        assert_eq!(&rom[0x200..0x204], &[0x46, 0xFC, 0x27, 0x00]);
        assert_eq!(
            &rom[0x204..0x20C],
            &[0x33, 0xFC, 0x80, 0x30, 0x00, 0xC0, 0x00, 0x04]
        ); // MOVE.W #$8030,(0x00C00004): enable VBlank and HBlank
        assert_eq!(&rom[0x20C..0x210], &[0x30, 0x3C, 0x00, 0x64]);
        assert_eq!(&rom[0x210..0x214], &[0x51, 0xC8, 0xFF, 0xFE]);
        assert_eq!(&rom[0x214..0x218], &[0x46, 0xFC, 0x23, 0x00]);
        assert_eq!(&rom[0x218..0x21A], &[0x70, 0x00]); // MOVEQ #0,D0
        assert_eq!(
            &rom[0x21A..0x220],
            &[0x3A, 0x39, 0x00, 0xC0, 0x00, 0x04] // MOVE.W (0x00C00004).L,D5
        );
        assert_eq!(&rom[0x220..0x222], &[0x60, 0xF8]); // BRA.S status read

        // Both handlers increment their counter, set N with MOVEQ #-1,D3 and RTE.
        // There is no MOVE.W #$2700,SR handler prologue: exception entry loads the
        // accepted level into the IPM.
        assert_eq!(&rom[0x2C0..0x2C6], &[0x52, 0x47, 0x76, 0xFF, 0x4E, 0x73]);
        assert_eq!(&rom[0x2E0..0x2E6], &[0x52, 0x46, 0x76, 0xFF, 0x4E, 0x73]);
        assert_ne!(
            &rom[0x2C0..0x2C2],
            &[0x46, 0xFC],
            "the HBlank handler must not self-mask"
        );
        assert_ne!(
            &rom[0x2E0..0x2E2],
            &[0x46, 0xFC],
            "the VBlank handler must not self-mask"
        );

        // The mask parameter is encoded as SR = 0x2000 | mask << 8.
        assert_eq!(&interrupt_rom(7)[0x214..0x218], &[0x46, 0xFC, 0x27, 0x00]);
        assert_eq!(&interrupt_rom(0)[0x214..0x218], &[0x46, 0xFC, 0x20, 0x00]);
    }

    #[test]
    fn controller_rom_layout_is_independent_and_matches_its_documentation() {
        let rom = controller_rom();
        assert_eq!(rom.len(), 0x400);
        assert_eq!(&rom[0..4], &0x0100_0000_u32.to_be_bytes());
        assert_eq!(&rom[4..8], &0x0000_0200_u32.to_be_bytes());

        // Four accumulators/observation registers cleared, then the loop body.
        assert_eq!(
            &rom[0x200..0x208],
            &[0x70, 0x00, 0x72, 0x00, 0x74, 0x00, 0x76, 0x00]
        );
        assert_eq!(
            &rom[0x208..0x20E],
            &[0x14, 0x39, 0x00, 0xA1, 0x00, 0x03], // MOVE.B (0x00A10003).L,D2
        );
        assert_eq!(&rom[0x20E..0x210], &[0xD0, 0x02]); // ADD.B D2,D0
        assert_eq!(
            &rom[0x210..0x216],
            &[0x13, 0xC0, 0x00, 0xFF, 0x00, 0x00], // MOVE.B D0,(0x00FF0000).L
        );
        assert_eq!(
            &rom[0x216..0x21C],
            &[0x13, 0xC2, 0x00, 0xFF, 0x00, 0x02], // MOVE.B D2,(0x00FF0002).L
        );
        assert_eq!(
            &rom[0x21C..0x222],
            &[0x16, 0x39, 0x00, 0xA1, 0x00, 0x05], // MOVE.B (0x00A10005).L,D3
        );
        assert_eq!(&rom[0x222..0x224], &[0xD2, 0x03]); // ADD.B D3,D1
        assert_eq!(
            &rom[0x224..0x22A],
            &[0x13, 0xC1, 0x00, 0xFF, 0x00, 0x01], // MOVE.B D1,(0x00FF0001).L
        );
        assert_eq!(
            &rom[0x22A..0x230],
            &[0x13, 0xC3, 0x00, 0xFF, 0x00, 0x03], // MOVE.B D3,(0x00FF0003).L
        );
        assert_eq!(&rom[0x230..0x232], &[0x60, 0xD6]); // BRA.S back to 0x208

        // The loop body is a fixed 42 bytes, so the branch displacement lands on
        // its first instruction.
        assert_eq!(0x232 - 42, 0x208);
    }

    #[test]
    fn controller_rom_records_each_button_bit_and_neutral_input() {
        let cases = [
            (0x00, CONTROLLER_NEUTRAL), // neutral reads 0x7F
            (0x01, 0x7E),               // Up
            (0x02, 0x7D),               // Down
            (0x04, 0x7B),               // Left
            (0x08, 0x77),               // Right
            (0x10, 0x6F),               // B
            (0x20, 0x5F),               // C
        ];
        for (mask, expected) in cases {
            // Eight steps: four clear instructions, the port-1 read, the add and
            // the two stores. Port 2 has not been read yet.
            let ram = controller_run(8, InputFrame::new(mask));
            assert_eq!(
                ram,
                [expected, 0, expected, 0],
                "port 1 mask 0x{mask:02X} must be observed in RAM"
            );
        }

        // Neutral input matches the documented baseline: the neutral data byte is
        // accumulated and recorded for both ports.
        assert_eq!(
            controller_run(12, InputFrame::NEUTRAL),
            [
                CONTROLLER_NEUTRAL,
                CONTROLLER_NEUTRAL,
                CONTROLLER_NEUTRAL,
                CONTROLLER_NEUTRAL
            ]
        );
        // A non-neutral frame after the loop's first iteration accumulates a
        // second observed byte, so RAM is a function of every step's frame.
        let pressed = InputFrame::with_devices([0x01, 0x20, 0, 0], [0; 4], 0);
        let two_iterations = 4 + 2 * 9 - 1;
        let ram = controller_run(two_iterations, pressed);
        assert_eq!(ram[2], 0x7E, "last port-1 byte");
        assert_eq!(ram[3], 0x5F, "last port-2 byte");
        assert_eq!(ram[0], 0xFC, "port-1 accumulator after two observations");
        assert_eq!(ram[1], 0xBE, "port-2 accumulator after two observations");
    }

    #[test]
    fn controller_rom_maps_two_ports_and_ignores_unrepresented_fields() {
        let first = InputFrame::with_devices([0x01, 0x20, 0, 0], [0; 4], 0);
        let second = InputFrame::with_devices([0x20, 0x01, 0, 0], [0; 4], 0);
        let steps = 12;
        assert_eq!(
            controller_run(steps, first),
            [0x7E, 0x5F, 0x7E, 0x5F],
            "port 1 and port 2 are independent"
        );
        assert_eq!(
            controller_run(steps, second),
            [0x5F, 0x7E, 0x5F, 0x7E],
            "swapping the masks swaps the observed ports"
        );

        // Ports 2/3, axes and keys are recorded by movies but have no emulated
        // effect here, and must never be reinterpreted as buttons or hardware.
        let inert = InputFrame::with_devices(
            [0x01, 0x20, u32::MAX, u32::MAX],
            [i16::MIN, -1, 1, i16::MAX],
            u64::MAX,
        );
        assert_eq!(controller_run(steps, inert), controller_run(steps, first));
    }

    #[test]
    fn controller_input_is_step_scoped_cleared_and_affects_canonical_state() {
        let mut pressed = controller_factory().create_machine().expect("machine");
        let mut neutral = controller_factory().create_machine().expect("machine");
        let mut button_only = controller_factory().create_machine().expect("machine");
        let frame = InputFrame::new(0x01);
        for step in 1..=8 {
            let pressed_outcome = pressed.step(frame).expect("pressed step");
            let neutral_outcome = neutral.step(InputFrame::NEUTRAL).expect("neutral step");
            let button_outcome = button_only
                .step(InputFrame::new(0x40)) // A is not in the TH-high subset
                .expect("button-only step");

            // The transient input never survives the step, even on success.
            assert_eq!(
                pressed.bus.read8(CONTROLLER_PORT_1),
                Ok(CONTROLLER_NEUTRAL),
                "step {step} must clear the transient input"
            );
            assert_eq!(pressed.bus.read8(CONTROLLER_PORT_2), Ok(CONTROLLER_NEUTRAL));

            if step < 5 {
                assert_eq!(pressed_outcome.state_hash, neutral_outcome.state_hash);
            } else {
                assert_ne!(
                    pressed_outcome.state_hash, neutral_outcome.state_hash,
                    "the controller read at step {step} must change canonical state"
                );
            }
            assert_eq!(
                button_outcome.state_hash, neutral_outcome.state_hash,
                "A/Start and high bits are inert"
            );
        }

        // Input is not part of the canonical snapshot: before the first
        // controller read the pressed and neutral machine serialize identically.
        let mut early_pressed = controller_factory().create_machine().expect("machine");
        let mut early_neutral = controller_factory().create_machine().expect("machine");
        for _ in 0..4 {
            early_pressed.step(frame).expect("pressed step");
            early_neutral
                .step(InputFrame::NEUTRAL)
                .expect("neutral step");
        }
        assert_eq!(
            early_pressed.snapshot().expect("snapshot").payload(),
            early_neutral.snapshot().expect("snapshot").payload()
        );
    }

    #[test]
    fn controller_input_is_cleared_on_a_failed_step() {
        let factory = MegadriveFactory::new(illegal_rom()).expect("valid ROM");
        let mut machine = factory.create_machine().expect("machine");
        assert!(matches!(
            machine.step(InputFrame::new(0x3F)),
            Err(BackendError::Execution(_))
        ));
        assert_eq!(machine.bus.read8(CONTROLLER_PORT_1), Ok(CONTROLLER_NEUTRAL));
        assert_eq!(machine.bus.read8(CONTROLLER_PORT_2), Ok(CONTROLLER_NEUTRAL));

        // The failed step may have advanced the fetch, but its canonical state is
        // identical to the same failure with neutral input: no input leaks in.
        let mut neutral = factory.create_machine().expect("machine");
        assert!(matches!(
            neutral.step(InputFrame::NEUTRAL),
            Err(BackendError::Execution(_))
        ));
        assert_eq!(
            machine.snapshot().expect("snapshot").state_hash(),
            neutral.snapshot().expect("snapshot").state_hash()
        );
        assert_eq!(
            machine.snapshot().expect("snapshot").payload(),
            neutral.snapshot().expect("snapshot").payload()
        );
    }

    #[test]
    fn debugger_reads_of_the_controller_window_are_typed_errors() {
        let mut machine = controller_factory().create_machine().expect("machine");
        machine.step(InputFrame::new(0x01)).expect("step");
        let before = machine.snapshot().expect("snapshot");
        for address in [CONTROLLER_PORT_1, CONTROLLER_PORT_2] {
            assert!(
                matches!(
                    machine.read_memory(u64::from(address), 1),
                    Err(BackendError::Execution(_))
                ),
                "debugger read at 0x{address:06X} must stay unmapped"
            );
        }
        assert_eq!(
            machine.snapshot().expect("snapshot").state_hash(),
            before.state_hash(),
            "a rejected debugger read must not change state"
        );
    }

    #[test]
    fn restored_controller_machine_uses_the_next_step_input() {
        let frame = InputFrame::with_devices([0x01, 0x20, 0, 0], [0; 4], 0);
        // Four steps only clear registers, so the checkpoint is input-independent.
        let mut uninterrupted = controller_factory().create_machine().expect("machine");
        for _ in 0..4 {
            uninterrupted.step(frame).expect("clear step");
        }
        let checkpoint = uninterrupted.snapshot().expect("snapshot");

        let mut expected = Vec::new();
        for _ in 0..10 {
            let outcome = uninterrupted.step(frame).expect("step");
            expected.push((outcome.tick, outcome.state_hash));
        }
        assert_eq!(
            &uninterrupted.inspect().expect("inspect").memory[0..4],
            &[0x7E, 0x5F, 0x7E, 0x5F]
        );

        // The restored machine replays the same frames and matches every tick and
        // state hash; no input was carried in the checkpoint.
        let mut restored = controller_factory().create_machine().expect("machine");
        for _ in 0..4 {
            restored.step(InputFrame::NEUTRAL).expect("neutral step");
        }
        restored.restore(&checkpoint).expect("restore");
        assert_eq!(
            restored.bus.read8(CONTROLLER_PORT_1),
            Ok(CONTROLLER_NEUTRAL)
        );
        for (index, (tick, state_hash)) in expected.iter().enumerate() {
            let outcome = restored.step(frame).expect("restored step");
            assert_eq!(outcome.tick, *tick, "tick at step {index}");
            assert_eq!(outcome.state_hash, *state_hash, "hash at step {index}");
        }
        assert_eq!(
            restored.snapshot().expect("snapshot").payload(),
            uninterrupted.snapshot().expect("snapshot").payload()
        );

        // Restoring the same checkpoint and stepping neutral diverges at the
        // first controller read: the machine uses the next supplied frame, not a
        // serialized latch.
        let mut neutral = controller_factory().create_machine().expect("machine");
        for _ in 0..4 {
            neutral.step(InputFrame::NEUTRAL).expect("neutral step");
        }
        neutral.restore(&checkpoint).expect("restore");
        assert_ne!(
            neutral.step(InputFrame::NEUTRAL).expect("step").state_hash,
            expected[0].1
        );
    }

    #[test]
    fn masked_hblank_is_retained_until_the_program_lowers_the_mask() {
        let mut machine = interrupt_machine(3);

        // The first HBlank is crossed inside the DBRA delay while the reset mask
        // 7 is still installed.
        step_until(
            &mut machine,
            |machine| machine.tick().raw() >= MASTER_CYCLES_PER_LINE,
            "first scanline",
        );
        let masked = machine.inspect().expect("inspect");
        assert_eq!(masked.registers["irq_mask"], 7);
        assert_eq!(
            masked.registers["irq_pending_level"],
            u64::from(InterruptLevel::HBlank.level())
        );
        assert_eq!(
            masked.registers["irq_vector"],
            u64::from(HBLANK_VECTOR_ADDRESS)
        );
        assert_eq!(
            masked.registers["irq_deliverable"], 0,
            "mask 7 blocks HBlank"
        );
        assert_eq!(masked.registers["d7"], 0, "no handler may run while masked");
        assert!(machine.interrupts().is_pending(InterruptLevel::HBlank));

        // The second line completes while the request and the mask are unchanged:
        // the new request coalesces into the retained one.
        step_until(
            &mut machine,
            |machine| machine.tick().raw() >= 2 * MASTER_CYCLES_PER_LINE,
            "second scanline",
        );
        let still_masked = machine.inspect().expect("inspect");
        assert_eq!(still_masked.registers["irq_mask"], 7);
        assert_eq!(still_masked.registers["irq_pending_level"], 4);
        assert_eq!(still_masked.registers["irq_deliverable"], 0);
        assert_eq!(still_masked.registers["d7"], 0);
        assert_eq!(still_masked.registers["d6"], 0);
        assert!(machine.interrupts().is_pending(InterruptLevel::HBlank));
        assert_eq!(
            machine.interrupts().requests_issued(InterruptLevel::HBlank),
            2
        );
        assert_eq!(
            machine
                .interrupts()
                .requests_coalesced(InterruptLevel::HBlank),
            1,
            "the second masked HBlank coalesces into the retained request"
        );

        // The program now lowers the mask with MOVE.W #$2300,SR. Delivery is
        // evaluated at the next instruction boundary, so the request is still
        // retained but is now deliverable.
        step_until(
            &mut machine,
            |machine| machine.cpu().pc() == 0x218,
            "mask lowering",
        );
        let lowered = machine.inspect().expect("inspect");
        assert_eq!(lowered.registers["irq_mask"], 3);
        assert_eq!(lowered.registers["irq_pending_level"], 4);
        assert_eq!(lowered.registers["irq_deliverable"], 1);
        assert_eq!(lowered.registers["d7"], 0);
        assert!(machine.interrupts().is_pending(InterruptLevel::HBlank));

        // The next boundary delivers exactly the retained HBlank request, pushes
        // the six-byte frame that points at the lowered-mask program and loads
        // IPM 4 for the handler.
        machine.step(InputFrame::NEUTRAL).expect("deliver HBlank");
        let delivered = machine.inspect().expect("inspect");
        assert_eq!(
            delivered.tick.raw(),
            machine.cpu().cycles() * 7,
            "the backend advances canonical time only by instruction cycles"
        );
        assert_eq!(delivered.registers["d7"], 1, "the handler counter ran");
        assert_eq!(delivered.registers["pc"], 0x2C2);
        assert_eq!(
            delivered.registers["irq_mask"], 4,
            "handler entry loaded IPM 4"
        );
        assert_eq!(machine.cpu().interrupt_priority_mask(), 4);
        assert_eq!(delivered.registers["irq_pending_level"], 0);
        assert!(!machine.interrupts().is_pending(InterruptLevel::HBlank));
        assert_eq!(
            machine.interrupts().requests_issued(InterruptLevel::HBlank),
            2,
            "delivery acknowledges the request, it does not reissue it"
        );

        let stack = delivered.registers["a7"];
        assert_eq!(stack, 0x00FF_FFFA, "a six-byte exception frame was pushed");
        let frame = usize::try_from(stack - u64::from(RAM_BASE)).expect("frame offset");
        let saved_sr = u16::from_be_bytes([delivered.memory[frame], delivered.memory[frame + 1]]);
        let saved_pc = u32::from_be_bytes([
            delivered.memory[frame + 2],
            delivered.memory[frame + 3],
            delivered.memory[frame + 4],
            delivered.memory[frame + 5],
        ]);
        assert_eq!(saved_sr, 0x2300);
        assert_eq!(saved_pc, 0x218);
        assert_eq!(
            delivered.memory[frame..frame + 6],
            [0x23, 0x00, 0x00, 0x00, 0x02, 0x18]
        );

        machine.step(InputFrame::NEUTRAL).expect("handler MOVEQ");
        let in_handler = machine.inspect().expect("inspect");
        assert_eq!(in_handler.registers["pc"], 0x2C4);
        assert_eq!(in_handler.registers["d3"], 0xFFFF_FFFF);
        assert_eq!(
            machine.cpu().sr(),
            0x2408,
            "N is set on top of the handler IPM 4"
        );

        machine.step(InputFrame::NEUTRAL).expect("RTE");
        let returned = machine.inspect().expect("inspect");
        assert_eq!(
            returned.registers["pc"], 0x218,
            "RTE returns to the interrupted PC"
        );
        assert_eq!(machine.cpu().sr(), 0x2300, "RTE restores the saved raw SR");
        assert_eq!(
            returned.registers["irq_mask"], 3,
            "the lowered mask is restored"
        );
        assert_eq!(
            returned.registers["a7"], 0x0100_0000,
            "RTE pops the six-byte frame"
        );
    }

    #[test]
    fn vblank_is_delivered_first_and_acknowledged_without_losing_hblank() {
        // Mask 5 admits level 6 (VBlank) but keeps level 4 (HBlank) pending, so
        // both requests exist together when the last active line completes.
        let mut machine = interrupt_machine(5);
        step_until(
            &mut machine,
            |machine| machine.tick().raw() >= MASTER_CYCLES_PER_LINE * u64::from(NTSC_ACTIVE_LINES),
            "vertical blanking",
        );
        let pending = machine.inspect().expect("inspect");
        assert_eq!(pending.registers["irq_mask"], 5);
        assert_eq!(
            pending.registers["irq_pending_level"],
            u64::from(InterruptLevel::VBlank.level())
        );
        assert_eq!(pending.registers["irq_deliverable"], 1);
        assert_eq!(pending.registers["d6"], 0);
        assert_eq!(
            pending.registers["vdp_status"] & u64::from(STATUS_VINT_PENDING),
            u64::from(STATUS_VINT_PENDING)
        );
        assert_eq!(
            pending.registers["vdp_status"] & u64::from(STATUS_VBLANK),
            u64::from(STATUS_VBLANK)
        );
        assert!(machine.interrupts().is_pending(InterruptLevel::VBlank));
        assert!(machine.interrupts().is_pending(InterruptLevel::HBlank));
        let pre_pc = pending.registers["pc"];
        let pre_sr = machine.cpu().sr();

        // VBlank (level 6) is delivered before the pending HBlank (level 4).
        // Exception entry loads IPM 6, so the pending HBlank cannot nest.
        machine.step(InputFrame::NEUTRAL).expect("deliver VBlank");
        let delivered = machine.inspect().expect("inspect");
        assert_eq!(delivered.registers["d6"], 1, "the VBlank handler ran");
        assert_eq!(delivered.registers["pc"], 0x2E2);
        assert_eq!(
            delivered.registers["irq_mask"], 6,
            "handler entry loaded IPM 6"
        );
        assert_eq!(machine.cpu().interrupt_priority_mask(), 6);
        assert!(
            !machine
                .cpu()
                .accepts_interrupt_level(InterruptLevel::HBlank.level()),
            "IPM 6 blocks the pending level-4 HBlank"
        );
        assert!(machine.interrupts().is_pending(InterruptLevel::HBlank));
        assert!(!machine.interrupts().is_pending(InterruptLevel::VBlank));
        let stack = delivered.registers["a7"];
        let frame = usize::try_from(stack - u64::from(RAM_BASE)).expect("frame offset");
        assert_eq!(
            u16::from_be_bytes([delivered.memory[frame], delivered.memory[frame + 1]]),
            pre_sr
        );
        let saved_pc = u32::from_be_bytes([
            delivered.memory[frame + 2],
            delivered.memory[frame + 3],
            delivered.memory[frame + 4],
            delivered.memory[frame + 5],
        ]);
        assert_eq!(u64::from(saved_pc), pre_pc);

        machine.step(InputFrame::NEUTRAL).expect("handler MOVEQ");
        machine.step(InputFrame::NEUTRAL).expect("RTE");
        let returned = machine.inspect().expect("inspect");
        assert_eq!(returned.registers["d6"], 1);
        assert_eq!(
            returned.registers["a7"], 0x0100_0000,
            "RTE pops the six-byte frame"
        );
        assert_eq!(returned.registers["irq_mask"], 5);
        assert!(machine.interrupts().is_pending(InterruptLevel::HBlank));
        assert_eq!(
            machine.interrupts().requests_issued(InterruptLevel::HBlank),
            u64::from(NTSC_ACTIVE_LINES),
            "one HBlank per active line"
        );
        assert_eq!(
            returned.registers["vdp_status"] & u64::from(STATUS_VINT_PENDING),
            0,
            "delivery acknowledges VINT"
        );
    }

    #[test]
    fn vblank_handler_ipm_six_blocks_hblank_reentry_until_rte() {
        let mut machine = interrupt_machine(3);
        step_until(
            &mut machine,
            |machine| machine.interrupts().is_pending(InterruptLevel::VBlank),
            "VBlank pending",
        );
        let pending = machine.inspect().expect("inspect");
        assert_eq!(
            pending.registers["irq_pending_level"],
            u64::from(InterruptLevel::VBlank.level())
        );
        assert!(machine.interrupts().is_pending(InterruptLevel::VBlank));
        assert!(machine.interrupts().is_pending(InterruptLevel::HBlank));
        let pre_pc = pending.registers["pc"];
        let pre_sr = pending.registers["sr"];
        let d7_before = pending.registers["d7"];

        // Deliver the level-6 VBlank; the handler body runs with IPM 6.
        machine.step(InputFrame::NEUTRAL).expect("deliver VBlank");
        let delivered = machine.inspect().expect("inspect");
        assert_eq!(delivered.registers["irq_mask"], 6);
        assert_eq!(delivered.registers["d6"], 1);
        assert_eq!(
            delivered.registers["irq_pending_level"],
            u64::from(InterruptLevel::HBlank.level()),
            "the acknowledged VBlank leaves the HBlank pending"
        );
        assert!(machine.interrupts().is_pending(InterruptLevel::HBlank));

        // The HBlank request is pending but IPM 6 masks level 4 at the next
        // boundary: no nested exception, no pushed frame and no D7 increment.
        machine.step(InputFrame::NEUTRAL).expect("handler body");
        let body = machine.inspect().expect("inspect");
        assert_eq!(body.registers["irq_mask"], 6);
        assert_eq!(body.registers["irq_pending_level"], 4);
        assert!(machine.interrupts().is_pending(InterruptLevel::HBlank));
        assert_eq!(body.registers["d7"], d7_before);
        assert_eq!(body.registers["d3"], 0xFFFF_FFFF);
        assert_eq!(body.registers["a7"], 0x00FF_FFFA);

        // RTE restores the interrupted raw SR with IPM 3; only then does the
        // retained HBlank enter once.
        machine.step(InputFrame::NEUTRAL).expect("VBlank RTE");
        let returned = machine.inspect().expect("inspect");
        assert_eq!(returned.registers["pc"], pre_pc);
        assert_eq!(returned.registers["sr"], pre_sr);
        assert_eq!(returned.registers["irq_mask"], 3);
        assert!(machine.interrupts().is_pending(InterruptLevel::HBlank));
        assert!(!machine.interrupts().is_pending(InterruptLevel::VBlank));

        machine
            .step(InputFrame::NEUTRAL)
            .expect("deliver retained HBlank");
        let delivered_hblank = machine.inspect().expect("inspect");
        assert_eq!(delivered_hblank.registers["d7"], d7_before + 1);
        assert_eq!(delivered_hblank.registers["irq_mask"], 4);
        assert_eq!(
            delivered_hblank.registers["vdp_status"] & u64::from(STATUS_VINT_PENDING),
            0
        );
    }

    #[test]
    fn masked_vblank_keeps_vint_pending_across_many_steps_without_acknowledging() {
        let mut machine = interrupt_machine(7);
        step_until(
            &mut machine,
            |machine| machine.interrupts().is_pending(InterruptLevel::VBlank),
            "VBlank pending",
        );
        step_until(
            &mut machine,
            |machine| machine.inspect().expect("inspect").registers["d5"] & 0x0080 == 0x0080,
            "status-port read after VBlank",
        );
        let pending = machine.inspect().expect("inspect");
        assert_eq!(pending.registers["irq_mask"], 7);
        assert_eq!(
            pending.registers["irq_pending_level"],
            u64::from(InterruptLevel::VBlank.level())
        );
        assert_eq!(pending.registers["irq_deliverable"], 0);
        assert_eq!(pending.registers["d6"], 0);
        assert_eq!(pending.registers["d7"], 0);
        assert_eq!(
            pending.registers["d5"] & 0x0080,
            0x0080,
            "the ROM's own status-port read sees VINT pending"
        );
        assert_eq!(
            pending.registers["vdp_status"] & u64::from(STATUS_VINT_PENDING),
            u64::from(STATUS_VINT_PENDING)
        );
        assert!(machine.interrupts().is_pending(InterruptLevel::VBlank));
        assert!(machine.interrupts().is_pending(InterruptLevel::HBlank));

        // More than a full frame of further masked quanta never acknowledges or
        // loses the request; frame 1 raises a second, coalesced VBlank at tick
        // 1944, and by tick 2100 vertical blanking has ended again.
        step_until(
            &mut machine,
            |machine| {
                machine.tick().raw() >= 2 * MASTER_CYCLES_PER_LINE * u64::from(NTSC_LINES_PER_FRAME)
            },
            "two full frames",
        );
        let later = machine.inspect().expect("inspect");
        assert_eq!(later.registers["irq_mask"], 7);
        assert_eq!(later.registers["irq_pending_level"], 6);
        assert_eq!(later.registers["irq_deliverable"], 0);
        assert_eq!(later.registers["d6"], 0);
        assert_eq!(later.registers["d7"], 0);
        assert_eq!(
            later.registers["vdp_status"] & u64::from(STATUS_VINT_PENDING),
            u64::from(STATUS_VINT_PENDING),
            "VINT pending stays set while the request is masked"
        );
        assert_eq!(
            later.registers["vdp_status"] & u64::from(STATUS_VBLANK),
            0,
            "video blanking has ended while the interrupt is still pending"
        );
        assert!(machine.interrupts().is_pending(InterruptLevel::VBlank));
        assert!(machine.interrupts().is_pending(InterruptLevel::HBlank));
        assert!(machine.interrupts().requests_issued(InterruptLevel::VBlank) >= 2);
        assert!(
            machine
                .interrupts()
                .requests_coalesced(InterruptLevel::VBlank)
                >= 1
        );
    }

    #[test]
    fn status_port_reads_do_not_acknowledge_masked_requests() {
        let mut machine = interrupt_machine(7);
        step_until(
            &mut machine,
            |machine| machine.interrupts().is_pending(InterruptLevel::VBlank),
            "VBlank pending",
        );
        assert!(machine.interrupts().is_pending(InterruptLevel::VBlank));
        let issued = machine.interrupts().requests_issued(InterruptLevel::VBlank);

        // The ROM executes a real MOVE.W (0x00C00004).L,D5 status-port read every
        // other instruction. Forty more quanta leave the request pending.
        for _ in 0..40 {
            machine.step(InputFrame::NEUTRAL).expect("status read loop");
        }
        assert!(machine.interrupts().is_pending(InterruptLevel::VBlank));
        assert_eq!(
            machine.interrupts().requests_issued(InterruptLevel::VBlank),
            issued,
            "status reads must neither acknowledge nor reissue the request"
        );
        assert_eq!(register(&machine, "d5") & 0x0080, 0x0080);
        assert_eq!(
            register(&machine, "vdp_status") & u64::from(STATUS_VINT_PENDING),
            u64::from(STATUS_VINT_PENDING)
        );
        assert_eq!(register(&machine, "d6"), 0);
    }

    /// Builds a ROM that reads the status port repeatedly while both VDP
    /// interrupt sources are masked, then lowers the mask. The HBlank handler
    /// increments D7 and returns with `RTE`.
    fn status_read_before_delivery_rom() -> Vec<u8> {
        let mut rom = [0x4E_u8, 0x71].repeat(0x200); // 0x400 bytes of NOP fill
        rom[0..4].copy_from_slice(&0x0100_0000_u32.to_be_bytes());
        rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());
        rom[0x70..0x74].copy_from_slice(&0x0000_02C0_u32.to_be_bytes());
        rom[0x78..0x7C].copy_from_slice(&0x0000_02E0_u32.to_be_bytes());
        let program: &[u8] = &[
            0x46, 0xFC, 0x27, 0x00, // MOVE.W #$2700,SR
            0x33, 0xFC, 0x80, 0x30, 0x00, 0xC0, 0x00, 0x04, // MOVE.W #$8030,(0xC00004)
            0x30, 0x3C, 0x03, 0xE8, // MOVE.W #1000,D0
            0x3A, 0x39, 0x00, 0xC0, 0x00, 0x04, // loop: MOVE.W (0xC00004).L,D5
            0x51, 0xC8, 0xFF, 0xF8, // DBRA D0,loop (instruction+2)
            0x46, 0xFC, 0x23, 0x00, // MOVE.W #$2300,SR (lower the mask)
            0x3A, 0x39, 0x00, 0xC0, 0x00, 0x04, // MOVE.W (0xC00004).L,D5
            0x60, 0xF8, // BRA.S back to the status read
        ];
        rom[0x200..0x200 + program.len()].copy_from_slice(program);
        rom[0x2C0..0x2C6].copy_from_slice(&[
            0x52, 0x47, // ADDQ.W #1,D7
            0x76, 0xFF, // MOVEQ #-1,D3
            0x4E, 0x73, // RTE
        ]);
        rom[0x2E0..0x2E6].copy_from_slice(&[
            0x52, 0x46, // ADDQ.W #1,D6
            0x76, 0xFF, // MOVEQ #-1,D3
            0x4E, 0x73, // RTE
        ]);
        super::stamp_synthetic_header(&mut rom);
        rom
    }

    #[test]
    fn status_reads_before_delivery_do_not_acknowledge_the_request() {
        let mut machine = MegadriveFactory::new(status_read_before_delivery_rom())
            .expect("valid status-read ROM")
            .create_machine()
            .expect("machine");
        step_until(
            &mut machine,
            |machine| machine.interrupts().is_pending(InterruptLevel::HBlank),
            "HBlank pending while masked",
        );
        assert_eq!(register(&machine, "irq_mask"), 7);
        assert_eq!(register(&machine, "d7"), 0);

        // The ROM is in its status-read loop. More status reads must neither
        // acknowledge the retained request nor enter the handler.
        for _ in 0..20 {
            machine.step(InputFrame::NEUTRAL).expect("status read");
        }
        assert!(
            machine.interrupts().is_pending(InterruptLevel::HBlank),
            "a status read must not acknowledge the pending HBlank"
        );
        assert_eq!(
            register(&machine, "d7"),
            0,
            "a status read must not enter the handler"
        );

        // The loop lowers the mask; the retained request is still delivered.
        step_until(
            &mut machine,
            |machine| register(machine, "d7") >= 1,
            "HBlank delivery after the status reads",
        );
        assert_eq!(
            register(&machine, "irq_mask"),
            4,
            "handler entry loads IPM 4"
        );
        assert!(!machine.interrupts().is_pending(InterruptLevel::HBlank));
    }

    #[test]
    fn acknowledging_one_level_never_clears_the_other() {
        // The backend can only take VBlank first (68000 priority), so the reverse
        // acknowledgement order is exercised through the same scheduler primitive
        // the backend acknowledges with.
        let config = SchedulerConfig::ntsc();

        let mut vblank_first = InterruptScheduler::new(config);
        vblank_first
            .advance_master(MASTER_CYCLES_PER_LINE * 224)
            .expect("advance to VBlank");
        assert!(vblank_first.is_pending(InterruptLevel::VBlank));
        assert!(vblank_first.is_pending(InterruptLevel::HBlank));
        assert!(vblank_first.acknowledge(InterruptLevel::VBlank));
        assert!(!vblank_first.is_pending(InterruptLevel::VBlank));
        assert!(vblank_first.is_pending(InterruptLevel::HBlank));

        let mut hblank_first = InterruptScheduler::new(config);
        hblank_first
            .advance_master(MASTER_CYCLES_PER_LINE * 224)
            .expect("advance to VBlank");
        assert!(hblank_first.acknowledge(InterruptLevel::HBlank));
        assert!(!hblank_first.is_pending(InterruptLevel::HBlank));
        assert!(hblank_first.is_pending(InterruptLevel::VBlank));
        assert!(hblank_first.acknowledge(InterruptLevel::VBlank));
        assert!(!hblank_first.has_pending());
    }

    #[test]
    fn snapshots_include_vdp_and_scheduler_state() {
        let factory = factory();
        let mut machine = factory.create_machine().expect("machine");
        for _ in 0..64 {
            machine.step(InputFrame::NEUTRAL).expect("step");
        }
        let snapshot = machine.snapshot().expect("snapshot");
        let expected = machine.inspect().expect("inspect").state_hash;
        assert_eq!(
            snapshot.format_version(),
            chronofork_backend_api::BACKEND_SNAPSHOT_FORMAT_VERSION
        );
        assert_eq!(snapshot.tick().raw(), machine.cpu().cycles() * 7);

        let vdp_bytes = machine.vdp().snapshot_bytes();
        let scheduler_bytes = machine.interrupts().snapshot_bytes();
        assert!(
            snapshot
                .payload()
                .windows(vdp_bytes.len())
                .any(|window| window == vdp_bytes.as_slice())
        );
        // The scheduler snapshot is followed by the fixed Z80/arbitration/audio
        // tail, so it is embedded rather than at the end of the payload.
        assert!(
            snapshot
                .payload()
                .windows(scheduler_bytes.len())
                .any(|window| window == scheduler_bytes.as_slice())
        );
        // The demo ROM never lowers the reset mask 7, so the HBlank request raised
        // at tick 64 is retained and unacknowledged.
        assert!(machine.interrupts().is_pending(InterruptLevel::HBlank));
        let masked = machine.inspect().expect("inspect");
        assert_eq!(masked.registers["irq_mask"], 7);
        assert_eq!(masked.registers["irq_deliverable"], 0);
        assert_eq!(masked.registers["d7"], 0);

        machine.step(InputFrame::NEUTRAL).expect("masked step");
        let masked_step = machine.inspect().expect("inspect");
        assert_eq!(
            masked_step.registers["d7"], 0,
            "mask 7 must not deliver the pending HBlank"
        );
        assert!(machine.interrupts().is_pending(InterruptLevel::HBlank));

        machine.restore(&snapshot).expect("restore");
        let restored = machine.inspect().expect("inspect");
        assert_eq!(restored.state_hash, expected);
        assert_eq!(restored.tick.raw(), snapshot.tick().raw());
        assert_eq!(
            restored.registers["irq_pending_level"],
            u64::from(InterruptLevel::HBlank.level())
        );
        assert_eq!(restored.registers["irq_deliverable"], 0);
        assert_eq!(machine.vdp().register(15), Some(2));
        assert_eq!(&machine.vdp().vram()[0..2], &[0x12, 0x34]);
        assert!(machine.interrupts().is_pending(InterruptLevel::HBlank));

        // The restored masked request is still retained and never acknowledged.
        machine.step(InputFrame::NEUTRAL).expect("replay");
        let replayed = machine.inspect().expect("inspect");
        assert_eq!(replayed.registers["d7"], 0);
        assert!(machine.interrupts().is_pending(InterruptLevel::HBlank));
    }

    #[test]
    fn snapshot_restore_preserves_masked_pending_requests_and_vint_state() {
        let mut machine = interrupt_machine(7);
        step_until(
            &mut machine,
            |machine| machine.interrupts().is_pending(InterruptLevel::VBlank),
            "VBlank pending",
        );
        let snapshot = machine.snapshot().expect("snapshot");
        let expected = machine.inspect().expect("inspect").state_hash;
        assert_eq!(snapshot.state_hash(), expected);
        assert_eq!(snapshot.payload()[8..10], PAYLOAD_VERSION.to_le_bytes());
        assert!(
            snapshot
                .payload()
                .windows(machine.interrupts().snapshot_bytes().len())
                .any(|window| window == machine.interrupts().snapshot_bytes().as_slice())
        );

        // A masked step keeps the request and never runs a handler.
        machine.step(InputFrame::NEUTRAL).expect("masked step");
        assert_eq!(register(&machine, "d6"), 0);
        assert!(machine.interrupts().is_pending(InterruptLevel::VBlank));

        machine.restore(&snapshot).expect("restore");
        let restored = machine.inspect().expect("inspect");
        assert_eq!(restored.state_hash, expected);
        assert_eq!(restored.tick.raw(), snapshot.tick().raw());
        assert_eq!(restored.registers["irq_mask"], 7);
        assert_eq!(restored.registers["irq_pending_level"], 6);
        assert_eq!(restored.registers["irq_deliverable"], 0);
        assert_eq!(
            restored.registers["vdp_status"] & u64::from(STATUS_VINT_PENDING),
            u64::from(STATUS_VINT_PENDING),
            "VINT pending is recomputed on restore, before any step"
        );
        assert!(machine.interrupts().is_pending(InterruptLevel::VBlank));
        assert!(machine.interrupts().is_pending(InterruptLevel::HBlank));

        // The restored machine stays deterministic: more masked steps acknowledge
        // nothing and do not lose the retained VBlank request.
        for _ in 0..100 {
            machine
                .step(InputFrame::NEUTRAL)
                .expect("post-restore masked step");
        }
        assert_eq!(register(&machine, "d6"), 0);
        assert!(machine.interrupts().is_pending(InterruptLevel::VBlank));
        assert_eq!(
            register(&machine, "vdp_status") & u64::from(STATUS_VINT_PENDING),
            u64::from(STATUS_VINT_PENDING)
        );
    }

    #[test]
    fn pending_vblank_state_survives_snapshot_restore() {
        let mut machine = interrupt_machine(3);
        step_until(
            &mut machine,
            |machine| machine.interrupts().is_pending(InterruptLevel::VBlank),
            "VBlank pending",
        );
        assert!(machine.interrupts().is_pending(InterruptLevel::VBlank));
        let snapshot = machine.snapshot().expect("snapshot");
        let expected = machine.inspect().expect("inspect").state_hash;
        assert_eq!(
            machine.vdp().status() & STATUS_VINT_PENDING,
            STATUS_VINT_PENDING
        );

        machine.step(InputFrame::NEUTRAL).expect("deliver VBlank");
        let delivered = machine.inspect().expect("inspect");
        assert_eq!(delivered.registers["d6"], 1, "the VBlank handler ran");
        assert_eq!(delivered.registers["irq_mask"], 6, "handler entry IPM");
        assert!(!machine.interrupts().is_pending(InterruptLevel::VBlank));
        assert_eq!(machine.vdp().status() & STATUS_VINT_PENDING, 0);
        // Finish the handler so the delivered VBlank is observed end to end.
        machine.step(InputFrame::NEUTRAL).expect("handler MOVEQ");
        machine.step(InputFrame::NEUTRAL).expect("handler RTE");
        assert_eq!(machine.inspect().expect("inspect").registers["d6"], 1);

        machine.restore(&snapshot).expect("restore");
        let restored = machine.inspect().expect("inspect");
        assert_eq!(restored.state_hash, expected);
        assert_eq!(restored.tick.raw(), snapshot.tick().raw());
        assert_eq!(
            restored.registers["irq_pending_level"],
            u64::from(InterruptLevel::VBlank.level())
        );
        assert_eq!(
            restored.registers["irq_vector"],
            u64::from(VBLANK_VECTOR_ADDRESS)
        );
        assert_eq!(
            restored.registers["vdp_status"] & u64::from(STATUS_VINT_PENDING),
            u64::from(STATUS_VINT_PENDING)
        );
        assert!(machine.interrupts().is_pending(InterruptLevel::VBlank));
        machine.step(InputFrame::NEUTRAL).expect("replay VBlank");
        let replayed = machine.inspect().expect("inspect");
        assert!(!machine.interrupts().is_pending(InterruptLevel::VBlank));
        assert_eq!(replayed.registers["d6"], 1);
        assert_eq!(replayed.registers["irq_mask"], 6);
    }

    #[test]
    fn snapshots_round_trip_and_detect_configuration_mismatch() {
        let factory = factory();
        let mut machine = factory.create_machine().expect("machine");
        for _ in 0..6 {
            machine.step(InputFrame::NEUTRAL).expect("step");
        }
        let snapshot = machine.snapshot().expect("snapshot");
        let expected = machine.inspect().expect("inspect").state_hash;
        machine
            .step(InputFrame::NEUTRAL)
            .expect("step after snapshot");
        machine.restore(&snapshot).expect("restore");
        assert_eq!(machine.inspect().expect("inspect").state_hash, expected);

        let mut other_rom = demo_rom();
        other_rom[0x300] ^= 0xFF;
        let other = MegadriveFactory::new(other_rom).expect("other factory");
        let mut other_machine = other.create_machine().expect("other machine");
        assert!(other_machine.restore(&snapshot).is_err());

        let pal_factory =
            MegadriveFactory::with_scheduler_config(demo_rom(), SchedulerConfig::pal())
                .expect("PAL factory");
        let mut other_timing = pal_factory.create_machine().expect("timing machine");
        assert!(other_timing.restore(&snapshot).is_err());
    }

    #[test]
    fn old_and_malformed_payloads_are_rejected() {
        let factory = factory();
        let mut machine = factory.create_machine().expect("machine");
        for _ in 0..6 {
            machine.step(InputFrame::NEUTRAL).expect("step");
        }
        let snapshot = machine.snapshot().expect("snapshot");
        assert_eq!(snapshot.payload()[8..10], PAYLOAD_VERSION.to_le_bytes());

        // Every earlier version is rejected with a typed error, never
        // reinterpreted as v8 bytes.
        for (index, magic, version) in [
            (1_u16, *b"CFMD68K1", 1_u16),
            (2, *b"CFMD68K2", 2),
            (3, *b"CFMD68K3", 3),
            (4, *b"CFMD68K4", 4),
            (5, *b"CFMD68K5", 5),
            (6, *b"CFMD68K6", 6),
            (7, *b"CFMD68K7", 7),
        ] {
            let mut old_payload = snapshot.payload().to_vec();
            old_payload[0..8].copy_from_slice(&magic);
            old_payload[8..10].copy_from_slice(&version.to_le_bytes());
            let old = BackendSnapshot::new(
                snapshot.backend_id().clone(),
                snapshot.backend_version(),
                snapshot.format_version(),
                snapshot.tick(),
                snapshot.config_hash(),
                snapshot.state_hash(),
                old_payload,
            );
            // The magic is checked before the version field, so an old magic is
            // rejected on its own terms; either way the error is typed and
            // nothing is reinterpreted. The version byte is also exercised on
            // its own below.
            let rejected = machine.restore(&old);
            assert!(
                matches!(rejected, Err(BackendError::InvalidSnapshot(_))),
                "payload v{index} must be rejected with a typed error, got {rejected:?}"
            );
            // Same v7 magic with an old version field is rejected by the version
            // check rather than the magic check.
            let mut versioned = snapshot.payload().to_vec();
            versioned[8..10].copy_from_slice(&version.to_le_bytes());
            let versioned = BackendSnapshot::new(
                snapshot.backend_id().clone(),
                snapshot.backend_version(),
                snapshot.format_version(),
                snapshot.tick(),
                snapshot.config_hash(),
                snapshot.state_hash(),
                versioned,
            );
            let rejected = machine.restore(&versioned);
            assert!(
                matches!(&rejected, Err(BackendError::InvalidSnapshot(message))
                    if message.contains(&format!("version {version}"))
                        && message.contains(&format!("expected {PAYLOAD_VERSION}"))),
                "payload version {index} must be rejected by the version check: {rejected:?}"
            );
        }

        let mut bad_vdp = snapshot.payload().to_vec();
        let vdp_start = bad_vdp
            .windows(SNAPSHOT_MAGIC.len())
            .position(|window| window == SNAPSHOT_MAGIC)
            .expect("VDP snapshot magic");
        bad_vdp[vdp_start] ^= 0xFF;
        let corrupted = BackendSnapshot::new(
            snapshot.backend_id().clone(),
            snapshot.backend_version(),
            snapshot.format_version(),
            snapshot.tick(),
            snapshot.config_hash(),
            snapshot.state_hash(),
            bad_vdp,
        );
        assert!(machine.restore(&corrupted).is_err());

        let mut truncated = snapshot.payload().to_vec();
        truncated.truncate(truncated.len() - 8);
        let short = BackendSnapshot::new(
            snapshot.backend_id().clone(),
            snapshot.backend_version(),
            snapshot.format_version(),
            snapshot.tick(),
            snapshot.config_hash(),
            snapshot.state_hash(),
            truncated,
        );
        assert!(machine.restore(&short).is_err());

        // A rejected restore leaves the machine at its current tick.
        assert_eq!(machine.inspect().expect("inspect").tick.raw(), 336);
    }

    #[test]
    fn sessions_fork_and_restore_the_real_core() {
        let mut manager = SessionManager::new(Arc::new(factory()));
        let source = manager.create_session().expect("source");
        for _ in 0..6 {
            manager.step(source, InputFrame::NEUTRAL).expect("step");
        }
        let checkpoint = manager.snapshot(source).expect("checkpoint");
        let before = manager.inspect(source).expect("inspect");
        let fork = manager
            .fork_from_snapshot(source, checkpoint)
            .expect("fork");
        manager
            .write_memory(fork, u64::from(RAM_BASE), &[0xAA])
            .expect("write fork");
        manager.step(fork, InputFrame::NEUTRAL).expect("fork step");
        let fork_view = manager.inspect(fork).expect("fork inspect");
        let source_view = manager.inspect(source).expect("source inspect");
        assert_ne!(fork_view.machine.state_hash, before.machine.state_hash);
        assert_eq!(source_view.machine.state_hash, before.machine.state_hash);
        assert_eq!(fork_view.lineage.parent_session(), Some(source));

        manager.restore(fork, checkpoint).expect("restore fork");
        let restored = manager.inspect(fork).expect("restored");
        assert_eq!(restored.machine.state_hash, before.machine.state_hash);
    }

    #[test]
    fn sessions_fork_pending_interrupt_and_vdp_state() {
        let mut manager = SessionManager::new(Arc::new(factory()));
        let source = manager.create_session().expect("source");
        for _ in 0..64 {
            manager.step(source, InputFrame::NEUTRAL).expect("step");
        }
        let checkpoint = manager.snapshot(source).expect("checkpoint");
        let before = manager.inspect(source).expect("inspect");
        assert_eq!(before.machine.registers["vdp_auto_increment"], 2);
        assert_eq!(before.machine.registers["irq_mask"], 7);
        assert_eq!(before.machine.registers["irq_pending_level"], 4);
        assert_eq!(before.machine.registers["irq_deliverable"], 0);

        let fork = manager
            .fork_from_snapshot(source, checkpoint)
            .expect("fork");
        let fork_before = manager.inspect(fork).expect("fork inspect");
        assert_eq!(fork_before.machine.state_hash, before.machine.state_hash);
        assert_eq!(fork_before.machine.registers["irq_pending_level"], 4);

        manager.step(fork, InputFrame::NEUTRAL).expect("fork step");
        let fork_after = manager.inspect(fork).expect("fork inspect");
        let source_after = manager.inspect(source).expect("source inspect");
        assert_ne!(fork_after.machine.state_hash, before.machine.state_hash);
        assert_eq!(source_after.machine.state_hash, before.machine.state_hash);
        assert_eq!(
            fork_after.machine.registers["d7"], 0,
            "the forked machine is still masked, so no handler runs"
        );
        assert_eq!(fork_after.machine.registers["irq_pending_level"], 4);

        manager.restore(fork, checkpoint).expect("restore fork");
        let restored = manager.inspect(fork).expect("restored");
        assert_eq!(restored.machine.state_hash, before.machine.state_hash);
        assert_eq!(restored.machine.registers["irq_pending_level"], 4);
        assert_eq!(restored.machine.registers["d7"], 0);
    }

    #[test]
    fn sessions_fork_masked_request_then_deliver_after_mask_lowering() {
        let mut manager = SessionManager::new(Arc::new(interrupt_factory(3)));
        let source = manager.create_session().expect("source");
        for _ in 0..2_000_000 {
            if manager.inspect(source).expect("inspect").machine.registers["irq_pending_level"]
                == u64::from(InterruptLevel::HBlank.level())
            {
                break;
            }
            manager.step(source, InputFrame::NEUTRAL).expect("step");
        }
        let checkpoint = manager.snapshot(source).expect("checkpoint");
        let masked = manager.inspect(source).expect("inspect");
        assert_eq!(
            masked.machine.tick.raw(),
            masked.machine.registers["master_cycles"],
            "tick is the master-cycle counter"
        );
        assert_eq!(masked.machine.registers["irq_mask"], 7);
        assert_eq!(masked.machine.registers["irq_pending_level"], 4);
        assert_eq!(masked.machine.registers["irq_deliverable"], 0);
        assert_eq!(masked.machine.registers["d7"], 0);

        let fork = manager
            .fork_from_snapshot(source, checkpoint)
            .expect("fork");
        assert_eq!(
            manager.inspect(fork).expect("fork").machine.state_hash,
            masked.machine.state_hash
        );

        // The fork finishes its delay loop, lowers the mask and takes the
        // retained HBlank at the next instruction boundary.
        for _ in 0..1_000_000 {
            if manager.inspect(fork).expect("inspect").machine.registers["pc"] == 0x218 {
                break;
            }
            manager.step(fork, InputFrame::NEUTRAL).expect("lower mask");
        }
        let lowered = manager.inspect(fork).expect("lowered");
        assert_eq!(lowered.machine.registers["irq_mask"], 3);
        assert_eq!(lowered.machine.registers["irq_deliverable"], 1);
        assert_eq!(lowered.machine.registers["d7"], 0);
        manager
            .step(fork, InputFrame::NEUTRAL)
            .expect("deliver HBlank");
        let delivered = manager.inspect(fork).expect("delivered");
        assert_eq!(
            delivered.machine.registers["d7"], 1,
            "the HBlank handler ran"
        );
        assert_eq!(
            delivered.machine.registers["irq_mask"], 4,
            "handler entry loaded IPM 4"
        );
        assert_eq!(delivered.machine.registers["irq_pending_level"], 0);

        // The source session is untouched by the fork's progress.
        let source_view = manager.inspect(source).expect("source");
        assert_eq!(source_view.machine.state_hash, masked.machine.state_hash);
        assert_eq!(source_view.machine.registers["d7"], 0);

        manager.restore(fork, checkpoint).expect("restore fork");
        let restored = manager.inspect(fork).expect("restored");
        assert_eq!(restored.machine.state_hash, masked.machine.state_hash);
        assert_eq!(restored.machine.registers["irq_mask"], 7);
        assert_eq!(restored.machine.registers["irq_pending_level"], 4);
        assert_eq!(restored.machine.registers["irq_deliverable"], 0);
    }

    #[test]
    fn two_machines_converge_including_devices() {
        let factory = factory();
        let mut left = factory.create_machine().expect("left");
        let mut right = factory.create_machine().expect("right");
        for _ in 0..70 {
            left.step(InputFrame::NEUTRAL).expect("left step");
            right.step(InputFrame::NEUTRAL).expect("right step");
        }
        assert_eq!(
            left.inspect().expect("left").state_hash,
            right.inspect().expect("right").state_hash
        );
        assert_eq!(left.vdp().snapshot_bytes(), right.vdp().snapshot_bytes());
        assert_eq!(
            left.interrupts().snapshot_bytes(),
            right.interrupts().snapshot_bytes()
        );
        assert_eq!(
            left.snapshot().expect("left snapshot").payload(),
            right.snapshot().expect("right snapshot").payload()
        );
    }

    #[test]
    fn two_interrupt_machines_deliver_byte_identical_state() {
        let factory = interrupt_factory(3);
        let mut left = factory.create_machine().expect("left");
        let mut right = factory.create_machine().expect("right");
        step_until(
            &mut left,
            |machine| machine.interrupts().is_pending(InterruptLevel::VBlank),
            "VBlank pending",
        );
        step_until(
            &mut right,
            |machine| machine.interrupts().is_pending(InterruptLevel::VBlank),
            "VBlank pending",
        );
        left.step(InputFrame::NEUTRAL).expect("deliver left VBlank");
        right
            .step(InputFrame::NEUTRAL)
            .expect("deliver right VBlank");
        let left_view = left.inspect().expect("left");
        let right_view = right.inspect().expect("right");
        assert_eq!(left_view.state_hash, right_view.state_hash);
        assert_eq!(left.vdp().status(), right.vdp().status());
        assert!(
            left_view.registers["d6"] >= 1,
            "both machines must have delivered VBlank"
        );
        assert_eq!(left_view.registers["d6"], right_view.registers["d6"]);
        assert_eq!(left_view.registers["d7"], right_view.registers["d7"]);
        assert_eq!(
            left.interrupts().snapshot_bytes(),
            right.interrupts().snapshot_bytes()
        );
        assert_eq!(
            left.snapshot().expect("left snapshot").payload(),
            right.snapshot().expect("right snapshot").payload()
        );
    }

    #[test]
    fn invalid_rom_and_illegal_opcode_are_errors() {
        assert!(MegadriveFactory::new(vec![0; 4]).is_err());
        let mut rom = demo_rom();
        rom[0x200..0x202].copy_from_slice(&[0xFF, 0xFF]);
        let factory = MegadriveFactory::new(rom).expect("factory");
        let mut machine = factory.create_machine().expect("machine");
        assert!(machine.step(InputFrame::NEUTRAL).is_err());
    }

    #[test]
    fn debugger_writes_are_limited_to_ram() {
        let factory = factory();
        let mut machine = factory.create_machine().expect("machine");
        let protected = machine
            .write_memory(0x200, &[0])
            .expect_err("cartridge ROM is read-only");
        assert!(
            matches!(&protected, BackendError::Execution(message) if message.contains("cartridge ROM")),
            "ROM protection must stay an execution error, never bounds: {protected}"
        );
        machine
            .write_memory(u64::from(RAM_BASE), &[0x12, 0x34])
            .expect("ram write");
        assert_eq!(
            machine
                .read_memory(u64::from(RAM_BASE), 2)
                .expect("ram read"),
            vec![0x12, 0x34]
        );
        let last_ram = u64::from(RAM_BASE) + RAM_SIZE as u64 - 1;
        assert_eq!(
            machine
                .write_memory(last_ram, &[1, 2])
                .expect_err("one byte past work RAM"),
            BackendError::OutOfBounds {
                address: last_ram,
                length: 2,
                limit: u64::from(RAM_END),
            }
        );
    }

    #[test]
    fn protected_and_unmapped_writes_are_not_reported_as_out_of_bounds() {
        let mut machine = factory().create_machine().expect("machine");
        // The cartridge, the unmapped hole above the ROM window and the YM2612
        // window are all failures with no containing writable window, so none of
        // them may be reported as a bounds overrun.
        for address in [0x200, u64::from(ROM_END), 0x00A0_4004] {
            let error = machine
                .write_memory(address, &[0])
                .expect_err("write outside work RAM must fail");
            assert!(
                matches!(error, BackendError::Execution(_)),
                "an address that is not a range overrun must not be out of bounds: {error}"
            );
        }
        // The `0xE00000..=0xFEFFFF` range is the work RAM mirror, so a write
        // through it succeeds and lands on the same byte as the primary window.
        machine
            .write_memory(0x00E0_0004, &[0xAB])
            .expect("mirror write");
        assert_eq!(
            machine
                .read_memory(u64::from(RAM_BASE) + 4, 1)
                .expect("ram read"),
            vec![0xAB]
        );
        // A real overrun reports the containing window end, which is strictly
        // above the requested start address, so the limit explains the failure.
        let last_ram = u64::from(RAM_BASE) + RAM_SIZE as u64 - 1;
        let error = machine
            .write_memory(last_ram, &[1, 2])
            .expect_err("one byte past work RAM");
        assert_eq!(
            error,
            BackendError::OutOfBounds {
                address: last_ram,
                length: 2,
                limit: u64::from(RAM_END),
            }
        );
        assert!(
            matches!(error, BackendError::OutOfBounds { address, limit, .. } if address < limit),
            "a reported bounds error must have a limit above the requested start: {error}"
        );
    }

    #[test]
    fn debugger_reads_validate_the_range_before_allocating() {
        let machine = factory().create_machine().expect("machine");
        assert_eq!(
            machine.read_memory(0, 2).expect("rom read"),
            vec![0x01, 0x00]
        );
        // The last RAM byte is readable, but a range that would pass the end of
        // the 24-bit bus is a bounds failure at the first unserved byte
        // (`0x01000000` decodes to ROM, so it is not RAM).
        let last_ram = u64::from(RAM_BASE) + RAM_SIZE as u64 - 1;
        assert_eq!(
            machine.read_memory(last_ram, 1).expect("last ram byte"),
            vec![0]
        );
        assert_eq!(
            machine.read_memory(last_ram, 2),
            Err(BackendError::OutOfBounds {
                address: last_ram,
                length: 2,
                limit: u64::from(RAM_END),
            }),
            "a range past the end of the 24-bit bus is a bounds failure"
        );
        assert_eq!(
            machine
                .read_memory(u64::from(RAM_END), 1)
                .expect("rom alias"),
            vec![0x01]
        );
        // A cartridge overrun is classified against the real image end (the
        // window the bus serves), not the 4 MiB cartridge address span.
        let rom_len = demo_rom().len();
        assert_eq!(
            machine.read_memory(0, rom_len + 1),
            Err(BackendError::OutOfBounds {
                address: 0,
                length: rom_len + 1,
                limit: rom_len as u64,
            }),
            "the cartridge limit is the image length (R1b-F1)"
        );
        // An enormous length is rejected by range arithmetic before the backend
        // can ask the allocator for it; an allocation of this size would abort.
        assert_eq!(
            machine.read_memory(u64::from(RAM_BASE), usize::MAX),
            Err(BackendError::OutOfBounds {
                address: u64::from(RAM_BASE),
                length: usize::MAX,
                limit: u64::from(RAM_END),
            })
        );
        // An address outside the 32-bit bus and an unmapped hole inside it are
        // execution failures: no mapped window contains the start address, so
        // there is no honest containing-window limit to report.
        for (address, length) in [(u64::from(u32::MAX) + 1, 1), (0x00C0_0010, 1)] {
            let error = machine
                .read_memory(address, length)
                .expect_err("unmapped debugger read must fail");
            assert!(
                matches!(error, BackendError::Execution(_)),
                "an unmapped start address is not a range overrun: {error}"
            );
        }
        // The VDP port window is a device failure, never a bounds failure.
        assert!(matches!(
            machine.read_memory(0x00C0_0000, 1),
            Err(BackendError::Execution(_))
        ));
        // Zero-length reads still validate the start address: a representable
        // address (even a side-effecting one) is accepted, an out-of-bus
        // address is rejected.
        assert_eq!(
            machine
                .read_memory(0x00C0_0010, 0)
                .expect("empty unmapped read"),
            Vec::<u8>::new()
        );
        assert!(matches!(
            machine.read_memory(u64::from(u32::MAX) + 1, 0),
            Err(BackendError::Execution(_))
        ));
    }

    #[test]
    fn debugger_bus_failures_keep_their_classification() {
        let window = Some(u64::from(RAM_END));
        for error in [
            BusError::UnmappedRead { address: 0x200 },
            BusError::UnmappedWrite { address: 0x200 },
            BusError::AddressOverflow {
                address: 0x200,
                length: 2,
            },
        ] {
            assert_eq!(
                super::map_memory_error(error, 0x200, 1, window),
                BackendError::OutOfBounds {
                    address: 0x200,
                    length: 1,
                    limit: u64::from(RAM_END),
                }
            );
        }
        // Protection and alignment are execution failures, not bounds failures.
        for error in [
            BusError::WriteProtected { address: 0x200 },
            BusError::OddAddress { address: 0x201 },
        ] {
            assert!(
                matches!(
                    super::map_memory_error(error, 0x200, 1, window),
                    BackendError::Execution(_)
                ),
                "protection/alignment must not become OutOfBounds"
            );
        }
        // Without a containing mapped window even unmapped/overflow failures
        // stay execution errors instead of inventing a limit.
        for error in [
            BusError::UnmappedRead { address: 0x200 },
            BusError::UnmappedWrite { address: 0x200 },
            BusError::AddressOverflow {
                address: 0x200,
                length: 2,
            },
        ] {
            assert!(matches!(
                super::map_memory_error(error, 0x200, 1, None),
                BackendError::Execution(_)
            ));
        }
        let side_effecting = super::map_memory_error(
            BusError::SideEffectingRead {
                address: 0x00C0_0000,
            },
            0x00C0_0000,
            1,
            None,
        );
        assert!(
            matches!(&side_effecting, BackendError::Execution(message) if message.contains("side-effect")),
            "a device failure must stay an execution error: {side_effecting}"
        );
        assert!(matches!(
            super::map_memory_error(
                BusError::Vdp(VdpError::UnsupportedOffset { offset: 0x10 }),
                0x00C0_0010,
                1,
                None,
            ),
            BackendError::Execution(_)
        ));
    }

    #[test]
    fn trace_is_off_by_default_and_captures_backend_steps() {
        let mut machine = factory().create_machine().expect("machine");
        assert!(machine.trace().is_none(), "tracing is disabled by default");
        machine.enable_trace(16).expect("trace capacity 16");
        for _ in 0..3 {
            machine.step(InputFrame::NEUTRAL).expect("step");
        }
        let trace = machine.trace().expect("trace enabled");
        let records: Vec<(u64, u32, u16)> = trace
            .entries()
            .map(|entry| (entry.index(), entry.pc(), entry.opcode()))
            .collect();
        assert_eq!(
            records,
            vec![
                (0, 0x200, 0x7001), // MOVEQ #1,D0
                (1, 0x202, 0x7202), // MOVEQ #2,D1
                (2, 0x204, 0xD041), // ADD.W D1,D0
            ]
        );
        let taken = machine.take_trace().expect("trace was enabled");
        assert_eq!(taken.len(), 3);
        assert!(machine.trace().is_none());
    }

    #[test]
    fn trace_does_not_perturb_snapshot_state_or_config_hash() {
        let factory = factory();
        let mut traced = factory.create_machine().expect("traced machine");
        let mut untraced = factory.create_machine().expect("untraced machine");
        traced.enable_trace(64).expect("trace capacity 64");
        for _ in 0..16 {
            traced.step(InputFrame::NEUTRAL).expect("traced step");
            untraced.step(InputFrame::NEUTRAL).expect("untraced step");
        }
        assert_eq!(traced.trace().expect("trace").len(), 16);
        assert!(untraced.trace().is_none());

        let traced_snapshot = traced.snapshot().expect("traced snapshot");
        let untraced_snapshot = untraced.snapshot().expect("untraced snapshot");
        assert_eq!(traced_snapshot.payload(), untraced_snapshot.payload());
        assert_eq!(traced_snapshot.state_hash(), untraced_snapshot.state_hash());
        assert_eq!(
            traced_snapshot.config_hash(),
            untraced_snapshot.config_hash()
        );
        assert_eq!(traced_snapshot.config_hash(), factory.config_hash());
        assert_eq!(
            traced_snapshot.format_version(),
            chronofork_backend_api::BACKEND_SNAPSHOT_FORMAT_VERSION
        );
        assert_eq!(
            traced_snapshot.payload()[8..10],
            PAYLOAD_VERSION.to_le_bytes()
        );
        assert_eq!(
            traced.inspect().expect("traced inspect").state_hash,
            untraced.inspect().expect("untraced inspect").state_hash
        );

        // A snapshot can be restored into a machine with tracing enabled, and the
        // restore does not touch the active trace.
        machine_restore_and_trace_survives(&factory);
    }

    /// Restoring a snapshot must leave trace contents alone (debug state is not
    /// canonical state).
    fn machine_restore_and_trace_survives(factory: &MegadriveFactory) {
        let mut machine = factory.create_machine().expect("machine");
        machine.enable_trace(64).expect("trace capacity 64");
        for _ in 0..4 {
            machine.step(InputFrame::NEUTRAL).expect("step");
        }
        let snapshot = machine.snapshot().expect("snapshot");
        for _ in 0..4 {
            machine.step(InputFrame::NEUTRAL).expect("step");
        }
        let recorded_before_restore = machine.trace().expect("trace").len();
        machine.restore(&snapshot).expect("restore");
        let trace = machine.trace().expect("trace survives restore");
        assert_eq!(recorded_before_restore, 8);
        assert_eq!(trace.len(), 8, "restore must not rewind trace contents");
        assert_eq!(trace.next_index(), 8);
        assert_eq!(trace.get(7).map(|entry| entry.pc()), Some(0x218));
    }

    #[test]
    fn backend_trace_rejects_zero_capacity_with_a_typed_error() {
        let mut machine = factory().create_machine().expect("machine");
        assert_eq!(
            machine.enable_trace(0),
            Err(crate::trace::TraceError::ZeroCapacity)
        );
        assert!(machine.trace().is_none());
    }

    #[test]
    fn demo_rom_uses_real_absolute_long_load_encodings() {
        // Source mode/register 111/001 (absolute-long) is low byte 0x39. The demo
        // must use the real 68000 load encoding, not the absolute-word 0x..38.
        let rom = demo_rom();
        assert_eq!(&rom[0x20E..0x210], &[0x34, 0x39], "MOVE.W (0xFF0000).L,D2");
        assert_eq!(&rom[0x250..0x252], &[0x34, 0x39], "MOVE.W (0xC00000).L,D2");
        assert_eq!(&rom[0x25C..0x25E], &[0x36, 0x39], "MOVE.W (0xC00004).L,D3");

        // The logical program still executes: D2 gets the computed 3, then the
        // VDP read-back value 0x1234, and RAM keeps the expected words.
        let mut machine = factory().create_machine().expect("machine");
        for _ in 0..23 {
            machine.step(InputFrame::NEUTRAL).expect("step");
        }
        let state = machine.inspect().expect("inspect");
        assert_eq!(state.registers["d2"], 0x1234);
        assert_eq!(state.memory[0..2], [0x00, 0x03]);
        assert_eq!(state.memory[4..6], [0x12, 0x34]);
    }

    #[test]
    fn arbitrary_payloads_are_rejected_without_panicking() {
        let mut state = 0xdead_beef_cafe_f00d_u64;
        for length in 0..600 {
            let mut payload = Vec::with_capacity(length);
            for _ in 0..length {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                payload.push((state >> 24).to_le_bytes()[0]);
            }
            assert!(
                super::parse_payload(&payload).is_err(),
                "arbitrary {length}-byte payload must be rejected"
            );
        }
    }

    #[test]
    fn state_hash_hashes_the_payload_state_body() {
        let mut machine = factory().create_machine().expect("machine");
        for _ in 0..6 {
            machine.step(InputFrame::NEUTRAL).expect("step");
        }
        let snapshot = machine.snapshot().expect("snapshot");
        let payload = snapshot.payload();
        assert_eq!(
            &payload[..super::PAYLOAD_MAGIC.len()],
            &super::PAYLOAD_MAGIC
        );
        let version_offset = super::PAYLOAD_MAGIC.len();
        assert_eq!(
            u16::from_le_bytes([payload[version_offset], payload[version_offset + 1]]),
            PAYLOAD_VERSION
        );
        let body = &payload[version_offset + 2..];
        let mut hash_input = Vec::with_capacity(super::STATE_MAGIC.len() + body.len());
        hash_input.extend_from_slice(super::STATE_MAGIC);
        hash_input.extend_from_slice(body);
        assert_eq!(
            chronofork_core::hash_bytes(&hash_input),
            snapshot.state_hash(),
            "state hash must be hash(STATE_MAGIC + payload state body)"
        );
    }

    #[test]
    fn snapshot_payload_and_hashes_are_golden() {
        // Golden values for payload v8 / state hash v8 / configuration hash v8
        // (six steps of the demo ROM on the master-cycle timeline). Phase 8.3
        // moved the outer identity from v4 to v5; phase 8.4 task 14 moved it to
        // v6; the W1 bus phase moved it to v7 and the body gained the eight port
        // 1/2/3/EXT control register bytes at `0xA10008..=0xA1000F` (payload
        // length 206 251 -> 206 259); the W1Z read-cache phase moved it to v8 and
        // the body gained the VDP read buffer appended after VRAM
        // (`CFVDP003`, +2 bytes, 206 259 -> 206 261). The VDP advances on the
        // master-cycle timeline, so the six-step state includes a non-zero
        // line/dot position. The length and all three hashes below are the
        // current v8 golden; v1-v7 payloads are rejected before any body byte is
        // reinterpreted.
        let mut machine = factory().create_machine().expect("machine");
        for _ in 0..6 {
            machine.step(InputFrame::NEUTRAL).expect("step");
        }
        let snapshot = machine.snapshot().expect("snapshot");
        assert_eq!(snapshot.payload().len(), 206_261);
        // Re-pinned by the W1Z read-cache phase, with the cause of each moved
        // value recorded: the VDP snapshot grew by the read buffer
        // (`CFVDP003`, +2 bytes) so the payload length moved 206 259 -> 206 261 and
        // the payload hash moved by construction; the state hash moved because
        // `CFMDSTATE8` replaced `CFMDSTATE7` *and* the hashed body grew by those
        // two bytes; the configuration hash moved only with the `CFMDCONF8` magic
        // (the cartridge/ROM/video configuration itself is unchanged). The prior
        // v7 values were re-pinned by the W1 bus phase; the v6 values by the 8.4
        // repair round (R1-F2/R1-F3).
        assert_eq!(
            chronofork_core::hash_bytes(snapshot.payload()).to_hex(),
            "32f4a4bbd4e637b63ed5fd9872e1c566e21300480d527b6e5164993240b73cf5"
        );
        assert_eq!(
            snapshot.state_hash().to_hex(),
            "acbe068c368be56579845bea5a18e6bc76ccb4b42703b30474bcd6eb38bf6456"
        );
        assert_eq!(
            snapshot.config_hash().to_hex(),
            "f454b76b7dec1181fd9645a5351c99a33e4f21b636ea3ba2c40fb0406f7d5c16"
        );
    }

    /// Builds a machine whose battery SRAM starts from a caller-supplied image.
    ///
    /// The imported image is the host policy seam of [`Cartridge::import_sram`]; the
    /// factory is built here through the private [`MegadriveFactory::from_parts`], so
    /// no production constructor exists only for this fixture.
    fn imported_sram_machine(image: &[u8]) -> MegadriveBackend {
        let mut cartridge = Cartridge::parse(demo_rom()).expect("cartridge");
        cartridge.import_sram(image).expect("SRAM image");
        let video = cartridge.derived_video_standard();
        let factory = MegadriveFactory::from_parts(
            cartridge,
            TmssModel::Disabled,
            SchedulerConfig::new(video),
        )
        .expect("factory");
        factory.create_machine().expect("machine")
    }

    /// Advances a machine until both scheduler requests are pending at once.
    ///
    /// The scheduler retains at most one request per level, so that state is the
    /// widest payload the v7 format supports. The work is bounded: the widest state
    /// arrives within thousands of steps on every supported configuration.
    fn step_until_two_pending(machine: &mut MegadriveBackend) {
        for _ in 0..20_000 {
            if machine.interrupts().pending_len() >= 2 {
                return;
            }
            machine.step(InputFrame::NEUTRAL).expect("step");
        }
        unreachable!("the demo program must reach a two-pending state within 20 000 steps");
    }

    /// Swaps the state of two machines with the same configuration.
    ///
    /// One machine reaches the widest state and the payload is moved into the other;
    /// both keep the same configuration, so the restores are accepted.
    fn swap_states(left: &mut MegadriveBackend, right: &mut MegadriveBackend) {
        let left_state = left.snapshot().expect("snapshot");
        let right_state = right.snapshot().expect("snapshot");
        left.restore(&right_state).expect("restore left");
        right.restore(&left_state).expect("restore right");
    }

    #[test]
    fn the_payload_bound_is_the_two_pending_payload() {
        // The scheduler declares the exact maximum snapshot it can serialize; the
        // +0/+8/+16 assertions below prove the backend's bound matches it in bytes.
        assert_eq!(SCHEDULER_MAX_SNAPSHOT_LEN, 68);
        assert_eq!(SCHEDULER_PENDING_SLOT_LEN, 8);
        let mut machine = factory().create_machine().expect("machine");
        let fresh = machine.snapshot().expect("snapshot").payload().len();
        assert_eq!(
            machine.snapshot_payload_bound(),
            Some(SNAPSHOT_PAYLOAD_BOUND),
            "the backend declares the format-derived bound"
        );
        assert!(
            fresh < SNAPSHOT_PAYLOAD_BOUND,
            "the fresh payload is strictly below the bound"
        );

        // A single pending request adds exactly one tick to the scheduler snapshot
        // and nothing else: the payload grows by exactly the slot length.
        while !machine.interrupts().has_pending() {
            machine.step(InputFrame::NEUTRAL).expect("step");
        }
        assert_eq!(machine.interrupts().pending_len(), 1);
        let single = machine.snapshot().expect("snapshot").payload().len();
        assert_eq!(
            single,
            fresh + SCHEDULER_PENDING_SLOT_LEN,
            "one pending request adds exactly its tick"
        );

        // The two-pending state is exactly one further slot wide and reaches the
        // declared bound exactly.
        let mut both = factory().create_machine().expect("machine");
        step_until_two_pending(&mut both);
        let widest = both.snapshot().expect("snapshot").payload().len();
        assert_eq!(
            widest - single,
            SCHEDULER_PENDING_SLOT_LEN,
            "the second pending request adds exactly another tick"
        );
        assert_eq!(
            widest, SNAPSHOT_PAYLOAD_BOUND,
            "a two-pending state reaches the declared bound exactly"
        );
        // Structural equivalence: for the same machine state the payload differs
        // from the fresh one only by `8 * pending`, so the scheduler snapshot is the
        // only component whose length can change.
        assert_eq!(
            widest - fresh,
            SCHEDULER_PENDING_SLOT_LEN * both.interrupts().pending_len()
        );

        // A restored two-pending state reserializes to the same bounded width and
        // state hash, so the bound is not an artifact of the live machine.
        let mut restored = factory().create_machine().expect("machine");
        let widest_state = both.snapshot().expect("snapshot");
        swap_states(&mut both, &mut restored);
        let restored_snapshot = restored.snapshot().expect("snapshot");
        assert_eq!(
            restored_snapshot.payload().len(),
            widest,
            "a restored two-pending state keeps the widest payload"
        );
        assert_eq!(
            restored_snapshot.state_hash(),
            widest_state.state_hash(),
            "the restored state keeps its identity"
        );
        assert_eq!(
            restored.snapshot_payload_bound(),
            Some(SNAPSHOT_PAYLOAD_BOUND)
        );
    }

    /// Result of one bounded payload sweep.
    struct BoundSweep {
        /// Widest payload sampled.
        maximum: usize,
        /// Whether a two-pending state was reached (and therefore sampled).
        reached_two: bool,
    }

    /// Runs one sweep and asserts every sampled payload stays within the bound.
    ///
    /// The pending count is read every step, so the two-pending coverage is never
    /// missed; the full serialization is sampled with `sample_stride` and always on
    /// the step that reaches the two-pending state. The sweep stops there, because
    /// every wider sample costs a full 206 KiB serialization and the tightness proof
    /// is complete once the widest state has been measured.
    fn sweep_stays_within_payload_bound(
        label: &str,
        machine: &mut MegadriveBackend,
        steps: usize,
        sample_stride: usize,
    ) -> BoundSweep {
        let mut maximum = 0usize;
        let mut reached_two = false;
        for step in 0..steps {
            let two = machine.interrupts().pending_len() >= 2;
            if two {
                reached_two = true;
            }
            if two || step % sample_stride == 0 {
                let length = machine.snapshot().expect("snapshot").payload().len();
                assert!(
                    length <= SNAPSHOT_PAYLOAD_BOUND,
                    "{label}: a payload above the declared bound at step {step}: {length}"
                );
                maximum = maximum.max(length);
            }
            if reached_two {
                break;
            }
            machine.step(InputFrame::NEUTRAL).expect("step");
        }
        assert_eq!(
            machine.snapshot_payload_bound(),
            Some(SNAPSHOT_PAYLOAD_BOUND),
            "{label}: the declaration must not depend on the state"
        );
        BoundSweep {
            maximum,
            reached_two,
        }
    }

    #[test]
    fn the_payload_bound_holds_for_ntsc_pal_and_imported_sram() {
        let ntsc = factory();
        let mut ntsc_machine = ntsc.create_machine().expect("machine");
        let ntsc_sweep = sweep_stays_within_payload_bound("ntsc", &mut ntsc_machine, 12_000, 100);
        assert!(
            ntsc_sweep.reached_two,
            "ntsc: the run must reach a two-pending state"
        );
        assert_eq!(
            ntsc_sweep.maximum, SNAPSHOT_PAYLOAD_BOUND,
            "ntsc: the declared bound must be reached exactly"
        );

        let pal = MegadriveFactory::with_scheduler_config(demo_rom(), SchedulerConfig::pal())
            .expect("pal");
        assert_eq!(pal.video_standard(), VideoStandard::Pal);
        let mut pal_machine = pal.create_machine().expect("machine");
        let pal_sweep = sweep_stays_within_payload_bound("pal", &mut pal_machine, 12_000, 100);
        assert!(
            pal_sweep.reached_two,
            "pal: the run must reach a two-pending state"
        );
        assert_eq!(
            pal_sweep.maximum, SNAPSHOT_PAYLOAD_BOUND,
            "pal: the declared bound must be reached exactly"
        );

        // The imported 64 KiB battery SRAM is a fixed-length component: importing an
        // image changes the canonical bytes but not one payload length.
        let image: Vec<u8> = (0..SRAM_SIZE).map(|index| (index * 7) as u8).collect();
        let mut imported_machine = imported_sram_machine(&image);
        let empty_machine = ntsc.create_machine().expect("machine");
        assert_eq!(
            imported_machine
                .snapshot()
                .expect("snapshot")
                .payload()
                .len(),
            empty_machine.snapshot().expect("snapshot").payload().len(),
            "imported SRAM does not change the payload length"
        );
        assert_ne!(
            imported_machine.snapshot().expect("snapshot").payload(),
            empty_machine.snapshot().expect("snapshot").payload(),
            "the image really is canonical state"
        );
        let imported_sweep =
            sweep_stays_within_payload_bound("imported-sram", &mut imported_machine, 12_000, 100);
        assert!(
            imported_sweep.reached_two,
            "imported-sram: the run must reach a two-pending state"
        );
        assert_eq!(
            imported_sweep.maximum, SNAPSHOT_PAYLOAD_BOUND,
            "imported-sram: the declared bound must be reached exactly"
        );
    }

    #[test]
    fn io_control_v7_round_trip() {
        let factory = factory();
        let mut machine = factory.create_machine().expect("machine");
        for _ in 0..4 {
            machine.step(InputFrame::NEUTRAL).expect("step");
        }
        // Non-zero port 1/2 control bytes, so the round trip would also catch a
        // field that was serialized as zeros. The port 3/EXT half is the
        // ADR-0014 constant-zero stub, so those writes are discarded.
        for offset in 0..IO_CONTROL_STUB_OFFSET {
            machine
                .write_memory(
                    u64::from(IO_CONTROL_BASE) + offset as u64,
                    &[0x80 + offset as u8],
                )
                .expect("control write");
        }
        for offset in IO_CONTROL_STUB_OFFSET..IO_CONTROL_LEN {
            machine
                .write_memory(
                    u64::from(IO_CONTROL_BASE) + offset as u64,
                    &[0x80 + offset as u8],
                )
                .expect("stub write is discarded, not rejected");
        }
        let mut expected: Vec<u8> = (0..IO_CONTROL_LEN).map(|o| 0x80 + o as u8).collect();
        for byte in expected.iter_mut().skip(IO_CONTROL_STUB_OFFSET) {
            *byte = 0;
        }
        assert_eq!(
            machine
                .read_memory(u64::from(IO_CONTROL_BASE), IO_CONTROL_LEN)
                .expect("control read"),
            expected
        );

        let snapshot = machine.snapshot().expect("snapshot");
        assert_eq!(
            &snapshot.payload()[..PAYLOAD_MAGIC.len()],
            &PAYLOAD_MAGIC,
            "the snapshot must carry the v7 payload magic"
        );

        // Mutate the live registers, then restore: the latched bytes come back.
        machine
            .write_memory(u64::from(IO_CONTROL_BASE), &[0x00])
            .expect("mutate control");
        machine
            .write_memory(0x00FF_0000, &[0xEE])
            .expect("mutate ram");
        machine.restore(&snapshot).expect("restore");
        assert_eq!(
            machine
                .read_memory(u64::from(IO_CONTROL_BASE), IO_CONTROL_LEN)
                .expect("control read after restore"),
            expected
        );
        assert_eq!(
            machine
                .read_memory(0x00FF_0000, 1)
                .expect("ram read after restore"),
            vec![0x00]
        );
        assert_eq!(
            machine.snapshot().expect("re-snapshot").state_hash(),
            snapshot.state_hash(),
            "a restored control file must hash back to the snapshot"
        );

        // A fork from the snapshot keeps the control bytes and diverges only
        // after a mutation.
        let mut manager = SessionManager::new(Arc::new(factory));
        let source = manager.create_session().expect("source");
        for offset in 0..IO_CONTROL_STUB_OFFSET {
            manager
                .write_memory(
                    source,
                    u64::from(IO_CONTROL_BASE) + offset as u64,
                    &[0x40 + offset as u8],
                )
                .expect("session control write");
        }
        let checkpoint = manager.snapshot(source).expect("checkpoint");
        let fork = manager
            .fork_from_snapshot(source, checkpoint)
            .expect("fork");
        assert_eq!(
            manager
                .read_memory(fork, u64::from(IO_CONTROL_BASE), IO_CONTROL_LEN)
                .expect("fork control read"),
            vec![0x40, 0x41, 0x42, 0x43, 0x00, 0x00, 0x00, 0x00]
        );
        manager
            .write_memory(fork, u64::from(IO_CONTROL_BASE), &[0x01])
            .expect("fork mutation");
        assert_ne!(
            manager
                .inspect(fork)
                .expect("fork inspect")
                .machine
                .state_hash,
            manager
                .inspect(source)
                .expect("source inspect")
                .machine
                .state_hash
        );
        assert_eq!(
            manager
                .read_memory(source, u64::from(IO_CONTROL_BASE), 1)
                .expect("source control read"),
            vec![0x40],
            "the fork mutation must not touch the source"
        );
    }

    #[test]
    fn io_control_stub_discards_writes_and_stays_zero_across_a_snapshot() {
        // ADR-0014: the port 3/EXT control pairs (0xA1000C..=0xA1000F) are a
        // stub pinned to zero. A write is acknowledged and discarded on the
        // execution, debugger and control-port paths, the long read of the
        // whole file reads zero on those lanes, and a snapshot/restore keeps
        // them zero while the port 1/2 pairs keep their latched bytes.
        let factory = factory();
        let mut machine = factory.create_machine().expect("machine");
        for _ in 0..4 {
            machine.step(InputFrame::NEUTRAL).expect("step");
        }
        // The functional pairs latch.
        for offset in 0..4 {
            machine
                .write_memory(
                    u64::from(IO_CONTROL_BASE) + offset as u64,
                    &[0xA0 + offset as u8],
                )
                .expect("port 1/2 write");
        }
        // The stub half discards.
        for offset in 4..IO_CONTROL_LEN {
            machine
                .write_memory(
                    u64::from(IO_CONTROL_BASE) + offset as u64,
                    &[0x80 + offset as u8],
                )
                .expect("stub write is discarded, not rejected");
        }
        let expected = vec![0xA0, 0xA1, 0xA2, 0xA3, 0x00, 0x00, 0x00, 0x00];
        assert_eq!(
            machine
                .read_memory(u64::from(IO_CONTROL_BASE), IO_CONTROL_LEN)
                .expect("control read"),
            expected,
            "the stub bytes must read zero"
        );

        let snapshot = machine.snapshot().expect("snapshot");
        machine
            .write_memory(u64::from(IO_CONTROL_BASE), &[0xFF])
            .expect("mutate a functional byte");
        machine.restore(&snapshot).expect("restore");
        assert_eq!(
            machine
                .read_memory(u64::from(IO_CONTROL_BASE), IO_CONTROL_LEN)
                .expect("control read after restore"),
            expected
        );

        // A rejected range that overruns the control file by one byte is
        // atomic: neither the stub byte nor any functional byte moves, and the
        // state hash is unchanged (the R1-F3 regression at the backend).
        let hash_before = machine.snapshot().expect("snapshot").state_hash();
        assert!(
            machine
                .write_memory(
                    u64::from(IO_CONTROL_BASE) + IO_CONTROL_LEN as u64 - 1,
                    &[0x55, 0x66,]
                )
                .is_err(),
            "a write that overruns the control file must fail"
        );
        assert_eq!(
            machine
                .read_memory(u64::from(IO_CONTROL_BASE), IO_CONTROL_LEN)
                .expect("control read after rejection"),
            expected,
            "a rejected write must not store its first byte"
        );
        assert_eq!(
            machine.snapshot().expect("snapshot").state_hash(),
            hash_before,
            "a rejected write must not move the state hash"
        );
    }

    #[test]
    fn io_control_affects_the_state_hash() {
        let factory = factory();
        let mut base = factory.create_machine().expect("machine");
        for _ in 0..4 {
            base.step(InputFrame::NEUTRAL).expect("step");
        }
        let neutral = base.snapshot().expect("snapshot").state_hash();

        let mut latched = factory.create_machine().expect("machine");
        for _ in 0..4 {
            latched.step(InputFrame::NEUTRAL).expect("step");
        }
        // Nothing but a control register changes, so the control file is proof
        // of canonical state rather than decoration.
        latched
            .write_memory(u64::from(IO_CONTROL_BASE) + 3, &[0x5A])
            .expect("control write");
        let changed = latched.snapshot().expect("snapshot").state_hash();
        assert_ne!(
            neutral, changed,
            "the control file must enter the state hash"
        );
    }

    /// A valid cartridge whose program is a NOP fill, so a step's tick delta is
    /// exactly the instruction cost.
    fn nop_factory() -> MegadriveFactory {
        let mut rom = vec![0_u8; 0x400];
        for word in rom.chunks_exact_mut(2) {
            word.copy_from_slice(&[0x4E, 0x71]);
        }
        rom[0..4].copy_from_slice(&0x0100_0000_u32.to_be_bytes());
        rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());
        stamp_synthetic_header(&mut rom);
        MegadriveFactory::new(rom).expect("nop cartridge")
    }

    fn fill_vdp_fifo(machine: &mut MegadriveBackend) {
        use crate::vdp::{CONTROL_PORT_OFFSET, DATA_PORT_OFFSET};

        let vdp = machine.bus.vdp_mut();
        vdp.write16(CONTROL_PORT_OFFSET, 0x8F02)
            .expect("auto-increment");
        vdp.write16(CONTROL_PORT_OFFSET, 0x4000)
            .expect("command first word");
        vdp.write16(CONTROL_PORT_OFFSET, 0x0000)
            .expect("command second word");
        for word in 0..5_u16 {
            vdp.write16(DATA_PORT_OFFSET, word)
                .expect("queued data write");
        }
    }

    #[test]
    fn vdp_fifo_wait_cycles_are_added_to_the_instruction_tick() {
        let factory = nop_factory();
        let mut plain = factory.create_machine().expect("machine");
        let mut stalled = factory.create_machine().expect("machine");
        fill_vdp_fifo(&mut stalled);

        let plain_outcome = plain.step(InputFrame::NEUTRAL).expect("plain step");
        let stalled_outcome = stalled.step(InputFrame::NEUTRAL).expect("stalled step");
        assert_eq!(
            stalled_outcome.tick.raw(),
            plain_outcome.tick.raw() + 7,
            "one FIFO overflow slot adds one 68000 cycle (seven master cycles)"
        );

        // The same setup is deterministic: identical tick and state hash.
        let mut again = factory.create_machine().expect("machine");
        fill_vdp_fifo(&mut again);
        let again_outcome = again.step(InputFrame::NEUTRAL).expect("repeat step");
        assert_eq!(again_outcome.tick, stalled_outcome.tick);
        assert_eq!(again_outcome.state_hash, stalled_outcome.state_hash);
    }
}
