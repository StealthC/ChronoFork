//! ChronoFork's first real-hardware backend slice.
//!
//! It is a minimal 68000 + ROM/RAM + VDP-port machine with a deterministic
//! instruction-quantum interrupt scheduler, **not** a complete Mega Drive.
//! Rendering, audio, Z80, controllers, DMA, cartridge mappers and cycle-accurate
//! timing are not implemented. The backend advertises only what it actually
//! provides and uses an independently authored synthetic ROM in tests and the
//! demo.
//!
//! # Interrupt policy
//!
//! One executed instruction advances one scheduler quantum. Before each
//! instruction, **at most one** pending VBlank (level 6) or HBlank (level 4)
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
//! Differential runs that must remain interrupt-free can rely on the reset mask
//! 7 (a program that never writes `SR` takes no VDP interrupt), or construct
//! their machine with an explicit large scheduler quantum such as `1_000_000`
//! instructions per line. The default remains 64 instructions per line and is
//! never changed implicitly.

use std::sync::Arc;

use chronofork_backend_api::{
    BACKEND_SNAPSHOT_FORMAT_VERSION, Backend, BackendDescriptor, BackendError, BackendFactory,
    BackendId, BackendSnapshot, Capabilities, MachineInspection, StepOutcome,
};
use chronofork_core::{InputFrame, StateHash, Tick, hash_bytes};

use crate::bus::{MegadriveBus, RAM_BASE, RAM_SIZE, ROM_END};
use crate::cpu::{CpuError, M68000};
use crate::interrupts::{
    InterruptLevel, InterruptScheduler, SchedulerConfig, SchedulerError, VideoStandard,
};
use crate::trace::{InstructionTrace, TraceError};
use crate::vdp::{AUTO_INCREMENT_REGISTER, SNAPSHOT_LEN as VDP_SNAPSHOT_LEN, Vdp};

/// Stable backend identifier.
pub const BACKEND_ID: &str = "chronofork.megadrive.m68000";
/// Payload format version.
///
/// Version 2 adds the VDP and interrupt-scheduler snapshots after RAM. Version 1
/// payloads are rejected instead of being reinterpreted.
///
/// E2-15's VINT-pending status bit does not bump this version: the bit is derived
/// from the already-serialized and hashed scheduler pending state, not new
/// canonical state. A payload written before the bit was wired restores to the
/// same canonical machine state and recomputes the bit on restore.
pub const PAYLOAD_VERSION: u16 = 2;
/// 68000 autovector address used for a level-4 HBlank request.
pub const HBLANK_VECTOR_ADDRESS: u32 = 0x70;
/// 68000 autovector address used for a level-6 VBlank request.
pub const VBLANK_VECTOR_ADDRESS: u32 = 0x78;

const PAYLOAD_MAGIC: [u8; 8] = *b"CFMD68K2";
const STATE_MAGIC: &[u8; 10] = b"CFMDSTATE2";
const CONFIG_MAGIC: &[u8; 9] = b"CFMDCONF2";
const PAYLOAD_HEADER_LEN: usize = 8 + 2 + 32 + 32 + 4 + 2 + 1 + 8;

/// Factory for independent minimal Mega Drive machines.
#[derive(Clone, Debug)]
pub struct MegadriveFactory {
    descriptor: BackendDescriptor,
    rom: Arc<[u8]>,
    config_hash: StateHash,
    scheduler_config: SchedulerConfig,
}

impl MegadriveFactory {
    /// Creates a factory from an independently provided cartridge image using
    /// the default NTSC scheduler configuration.
    ///
    /// The image must contain at least the reset vectors and must not exceed the
    /// 68000 cartridge window. No ROM is bundled with this crate.
    pub fn new(rom: Vec<u8>) -> Result<Self, BackendError> {
        Self::with_scheduler_config(rom, SchedulerConfig::ntsc())
    }

    /// Creates a factory with an explicit interrupt-scheduler configuration.
    ///
    /// The configuration is part of the configuration hash, so a snapshot made
    /// with a different video standard or instruction quantum is rejected.
    pub fn with_scheduler_config(
        rom: Vec<u8>,
        scheduler_config: SchedulerConfig,
    ) -> Result<Self, BackendError> {
        if rom.len() < 8 {
            return Err(BackendError::InvalidConfiguration(
                "ROM must contain at least the reset vectors (8 bytes)".to_owned(),
            ));
        }
        let max_rom = usize::try_from(ROM_END)
            .map_err(|_| BackendError::InvalidConfiguration("ROM window overflow".to_owned()))?;
        if rom.len() > max_rom {
            return Err(BackendError::InvalidConfiguration(format!(
                "ROM is {} bytes, exceeding the 4 MiB cartridge window",
                rom.len()
            )));
        }
        let rom: Arc<[u8]> = Arc::from(rom.into_boxed_slice());
        let config_hash = config_hash_for(rom.as_ref(), scheduler_config);
        let capabilities = Capabilities {
            execution: true,
            save_states: true,
            memory_read: true,
            memory_write: true,
            register_inspection: true,
            instruction_stepping: true,
            state_cloning: true,
            deterministic_replay: true,
            ..Capabilities::none()
        };
        Ok(Self {
            descriptor: BackendDescriptor::new(
                BackendId::new(BACKEND_ID),
                "ChronoFork Mega Drive 68000 (VDP port + interrupt scheduler, no rendering/audio)",
                env!("CARGO_PKG_VERSION"),
                capabilities,
                false,
            ),
            rom,
            config_hash,
            scheduler_config,
        })
    }

    /// Returns the immutable cartridge image.
    #[must_use]
    pub fn rom(&self) -> &[u8] {
        &self.rom
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
    /// concrete type available for VDP and scheduler inspection.
    pub fn create_machine(&self) -> Result<MegadriveBackend, BackendError> {
        let mut bus = MegadriveBus::new(Arc::clone(&self.rom));
        let mut cpu = M68000::new();
        cpu.reset(&mut bus).map_err(map_cpu)?;
        Ok(MegadriveBackend {
            descriptor: self.descriptor.clone(),
            config_hash: self.config_hash,
            bus,
            cpu,
            scheduler: InterruptScheduler::new(self.scheduler_config),
            tick: Tick::ZERO,
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
}

impl MegadriveBackend {
    /// Returns the current tick.
    #[must_use]
    pub const fn tick(&self) -> Tick {
        self.tick
    }

    /// Returns the current 68000.
    #[must_use]
    pub const fn cpu(&self) -> &M68000 {
        &self.cpu
    }

    /// Returns the current VDP state.
    #[must_use]
    pub const fn vdp(&self) -> &Vdp {
        self.bus.vdp()
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

    fn state_hash(&self) -> StateHash {
        let (d, a, pc, sr, halted) = self.cpu.snapshot_state();
        let vdp = self.bus.vdp().snapshot_bytes();
        let scheduler = self.scheduler.snapshot_bytes();
        state_hash_for(&StateView {
            d: &d,
            a: &a,
            pc,
            sr,
            halted,
            tick: self.tick,
            ram: self.bus.ram(),
            vdp: &vdp,
            scheduler: &scheduler,
        })
    }

    fn payload(&self) -> Vec<u8> {
        let (d, a, pc, sr, halted) = self.cpu.snapshot_state();
        let vdp = self.bus.vdp().snapshot_bytes();
        let scheduler = self.scheduler.snapshot_bytes();
        let mut payload =
            Vec::with_capacity(PAYLOAD_HEADER_LEN + RAM_SIZE + vdp.len() + scheduler.len());
        payload.extend_from_slice(&PAYLOAD_MAGIC);
        payload.extend_from_slice(&PAYLOAD_VERSION.to_le_bytes());
        for value in d {
            payload.extend_from_slice(&value.to_be_bytes());
        }
        for value in a {
            payload.extend_from_slice(&value.to_be_bytes());
        }
        payload.extend_from_slice(&pc.to_be_bytes());
        payload.extend_from_slice(&sr.to_be_bytes());
        payload.push(u8::from(halted));
        payload.extend_from_slice(&self.tick.raw().to_be_bytes());
        payload.extend_from_slice(self.bus.ram());
        payload.extend_from_slice(&vdp);
        payload.extend_from_slice(&scheduler);
        payload
    }

    fn sync_vdp_vblank(&mut self) {
        let vblank = self.scheduler.line_in_frame() >= self.scheduler.config().active_lines();
        self.bus.vdp_mut().set_vblank(vblank);
    }

    /// Recomputes the derived VINT-pending status bit from the canonical
    /// scheduler state.
    ///
    /// The latch is set while a VBlank (level 6) request is pending, whether or
    /// not the 68000 `SR` mask currently admits it, and clears only when delivery
    /// acknowledges that request. It is derived state, so this must run after
    /// every mutation of pending state and after a restore.
    fn sync_vdp_vint_pending(&mut self) {
        let pending = self.scheduler.is_pending(InterruptLevel::VBlank);
        self.bus.vdp_mut().set_vint_pending(pending);
    }

    fn sync_vdp_status(&mut self) {
        self.sync_vdp_vblank();
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
        let Some(level) = self.scheduler.next_deliverable(self.cpu.sr()) else {
            return Ok(());
        };
        self.cpu
            .interrupt(level.level(), vector_address(level), &mut self.bus)
            .map_err(map_cpu)?;
        // Acknowledge exactly the delivered level. `acknowledge_next` would be
        // wrong here: it selects the highest-priority pending request, which can
        // be a different (masked) level.
        let acknowledged = self.scheduler.acknowledge(level);
        debug_assert!(acknowledged, "a deliverable level must have been pending");
        self.sync_vdp_vint_pending();
        Ok(())
    }
}

impl Backend for MegadriveBackend {
    fn descriptor(&self) -> &BackendDescriptor {
        &self.descriptor
    }

    fn step(&mut self, _input: InputFrame) -> Result<StepOutcome, BackendError> {
        // Take at most one interrupt at the instruction boundary, then execute
        // one instruction. This models the 68000 taking one request between
        // instructions instead of nesting several handlers in a single tick.
        self.deliver_pending_interrupt()?;
        self.cpu.step(&mut self.bus).map_err(map_cpu)?;
        self.tick = self.tick.next().map_err(|_| BackendError::TickOverflow)?;
        self.scheduler.tick().map_err(map_scheduler)?;
        self.sync_vdp_status();
        Ok(StepOutcome {
            tick: self.tick,
            state_hash: self.state_hash(),
            halted: self.cpu.halted(),
        })
    }

    fn snapshot(&self) -> Result<BackendSnapshot, BackendError> {
        Ok(BackendSnapshot::new(
            self.descriptor.id().clone(),
            self.descriptor.version(),
            BACKEND_SNAPSHOT_FORMAT_VERSION,
            self.tick,
            self.config_hash,
            self.state_hash(),
            self.payload(),
        ))
    }

    fn restore(&mut self, snapshot: &BackendSnapshot) -> Result<(), BackendError> {
        if snapshot.backend_id() != self.descriptor.id() {
            return Err(BackendError::IncompatibleSnapshot {
                field: "backend_id",
                expected: self.descriptor.id().to_string(),
                found: snapshot.backend_id().to_string(),
            });
        }
        if snapshot.backend_version() != self.descriptor.version() {
            return Err(BackendError::IncompatibleSnapshot {
                field: "backend_version",
                expected: self.descriptor.version().to_owned(),
                found: snapshot.backend_version().to_owned(),
            });
        }
        if snapshot.format_version() != BACKEND_SNAPSHOT_FORMAT_VERSION {
            return Err(BackendError::IncompatibleSnapshot {
                field: "backend_format_version",
                expected: BACKEND_SNAPSHOT_FORMAT_VERSION.to_string(),
                found: snapshot.format_version().to_string(),
            });
        }
        if snapshot.config_hash() != self.config_hash {
            return Err(BackendError::IncompatibleSnapshot {
                field: "config_hash",
                expected: self.config_hash.to_string(),
                found: snapshot.config_hash().to_string(),
            });
        }
        let parsed = parse_payload(snapshot.payload())?;
        let actual = state_hash_for(&StateView {
            d: &parsed.d,
            a: &parsed.a,
            pc: parsed.pc,
            sr: parsed.sr,
            halted: parsed.halted,
            tick: parsed.tick,
            ram: parsed.ram,
            vdp: parsed.vdp,
            scheduler: parsed.scheduler,
        });
        if actual != snapshot.state_hash() {
            return Err(BackendError::InvalidSnapshot(format!(
                "state hash mismatch after restore: expected {}, got {actual}",
                snapshot.state_hash()
            )));
        }

        // Validate and build the replacement state before committing it, so a
        // rejected restore leaves the machine untouched.
        let mut bus = self.bus.clone();
        bus.restore_ram(parsed.ram.to_vec())
            .map_err(|error| BackendError::InvalidSnapshot(error.to_string()))?;
        bus.vdp_mut().restore_bytes(parsed.vdp).map_err(|error| {
            BackendError::InvalidSnapshot(format!("invalid VDP state: {error}"))
        })?;
        let mut scheduler = self.scheduler.clone();
        scheduler.restore_bytes(parsed.scheduler).map_err(|error| {
            BackendError::InvalidSnapshot(format!("invalid interrupt state: {error}"))
        })?;
        self.bus = bus;
        self.scheduler = scheduler;
        self.cpu
            .restore_state(parsed.d, parsed.a, parsed.pc, parsed.sr, parsed.halted);
        self.tick = parsed.tick;
        // The VINT-pending status bit is derived from the restored scheduler
        // state, so recompute it before anything can inspect the machine. The
        // canonical VBlank bit (bit 3) is serialized and was validated against
        // the snapshot state hash, so it is not reinterpreted here.
        self.sync_vdp_vint_pending();
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
        registers.insert("scheduler_frame".to_owned(), self.scheduler.frame());
        registers.insert(
            "scheduler_line".to_owned(),
            u64::from(self.scheduler.line_in_frame()),
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
        self.bus
            .read_memory(address, length)
            .map_err(|error| BackendError::Execution(error.to_string()))
    }

    fn write_memory(&mut self, address: u64, data: &[u8]) -> Result<(), BackendError> {
        self.bus
            .write_memory(address, data)
            .map_err(|error| BackendError::Execution(error.to_string()))
    }
}

fn vector_address(level: InterruptLevel) -> u32 {
    match level {
        InterruptLevel::HBlank => HBLANK_VECTOR_ADDRESS,
        InterruptLevel::VBlank => VBLANK_VECTOR_ADDRESS,
    }
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

fn config_hash_for(rom: &[u8], scheduler_config: SchedulerConfig) -> StateHash {
    let mut bytes = Vec::with_capacity(CONFIG_MAGIC.len() + rom.len() + 5);
    bytes.extend_from_slice(CONFIG_MAGIC);
    bytes.extend_from_slice(rom);
    bytes.push(match scheduler_config.standard() {
        VideoStandard::Ntsc => 0,
        VideoStandard::Pal => 1,
    });
    bytes.extend_from_slice(&scheduler_config.instructions_per_line().to_be_bytes());
    hash_bytes(&bytes)
}

/// The complete mutable state that a state hash covers.
struct StateView<'a> {
    d: &'a [u32; 8],
    a: &'a [u32; 8],
    pc: u32,
    sr: u16,
    halted: bool,
    tick: Tick,
    ram: &'a [u8],
    vdp: &'a [u8],
    scheduler: &'a [u8],
}

fn state_hash_for(state: &StateView<'_>) -> StateHash {
    let mut bytes = Vec::with_capacity(
        STATE_MAGIC.len()
            + 32
            + 32
            + 4
            + 2
            + 1
            + 8
            + state.ram.len()
            + state.vdp.len()
            + state.scheduler.len(),
    );
    bytes.extend_from_slice(STATE_MAGIC);
    for value in state.d {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    for value in state.a {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    bytes.extend_from_slice(&state.pc.to_be_bytes());
    bytes.extend_from_slice(&state.sr.to_be_bytes());
    bytes.push(u8::from(state.halted));
    bytes.extend_from_slice(&state.tick.raw().to_be_bytes());
    bytes.extend_from_slice(state.ram);
    bytes.extend_from_slice(state.vdp);
    bytes.extend_from_slice(state.scheduler);
    hash_bytes(&bytes)
}

#[derive(Debug)]
struct ParsedState<'a> {
    d: [u32; 8],
    a: [u32; 8],
    pc: u32,
    sr: u16,
    halted: bool,
    tick: Tick,
    ram: &'a [u8],
    vdp: &'a [u8],
    scheduler: &'a [u8],
}

fn parse_payload(payload: &[u8]) -> Result<ParsedState<'_>, BackendError> {
    let mut reader = Reader::new(payload);
    if reader.read_array::<8>()? != PAYLOAD_MAGIC {
        return Err(BackendError::InvalidSnapshot(
            "Mega Drive payload magic mismatch".to_owned(),
        ));
    }
    let version = reader.read_u16()?;
    if version != PAYLOAD_VERSION {
        return Err(BackendError::InvalidSnapshot(format!(
            "Mega Drive payload version {version}, expected {PAYLOAD_VERSION}"
        )));
    }
    let mut d = [0_u32; 8];
    for value in &mut d {
        *value = u32::from_be_bytes(reader.read_array::<4>()?);
    }
    let mut a = [0_u32; 8];
    for value in &mut a {
        *value = u32::from_be_bytes(reader.read_array::<4>()?);
    }
    let pc = u32::from_be_bytes(reader.read_array::<4>()?);
    let sr = u16::from_be_bytes(reader.read_array::<2>()?);
    let halted = reader.read_u8()? != 0;
    let tick = Tick::from_raw(u64::from_be_bytes(reader.read_array::<8>()?));
    let ram = reader.read_exact(RAM_SIZE)?;
    let vdp = reader.read_exact(VDP_SNAPSHOT_LEN)?;
    let remaining = reader.remaining();
    let scheduler = reader.read_exact(remaining)?;
    Ok(ParsedState {
        d,
        a,
        pc,
        sr,
        halted,
        tick,
        ram,
        vdp,
        scheduler,
    })
}

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    const fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.position)
    }

    fn read_exact(&mut self, length: usize) -> Result<&'a [u8], BackendError> {
        let end = self.position.checked_add(length).ok_or_else(|| {
            BackendError::InvalidSnapshot("payload length arithmetic overflow".to_owned())
        })?;
        if end > self.bytes.len() {
            return Err(BackendError::InvalidSnapshot(
                "Mega Drive payload is truncated".to_owned(),
            ));
        }
        let slice = &self.bytes[self.position..end];
        self.position = end;
        Ok(slice)
    }

    fn read_array<const N: usize>(&mut self) -> Result<[u8; N], BackendError> {
        let slice = self.read_exact(N)?;
        let mut output = [0_u8; N];
        output.copy_from_slice(slice);
        Ok(output)
    }

    fn read_u8(&mut self) -> Result<u8, BackendError> {
        Ok(self.read_array::<1>()?[0])
    }

    fn read_u16(&mut self) -> Result<u16, BackendError> {
        Ok(u16::from_le_bytes(self.read_array::<2>()?))
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
    rom
}

/// Builds an independently authored synthetic ROM that exercises mask-gated
/// interrupt delivery and `RTE`.
///
/// The main program at `0x200`:
///
/// 1. explicitly masks both VDP interrupt levels with `MOVE.W #$2700,SR`;
/// 2. idles for six `NOP`s, so with a small scheduler quantum (for example four
///    instructions per line) an HBlank request is raised and retained while it is
///    masked;
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
/// after which a retained request can be delivered. With a quantum of four
/// instructions per line, the first HBlank is raised while the mask is still 7,
/// the lowering instruction executes at instruction 8, and the retained request
/// is delivered at instruction 9.
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
        0x4E, 0x71, // NOP
        0x4E, 0x71, // NOP
        0x4E, 0x71, // NOP
        0x4E, 0x71, // NOP
        0x4E, 0x71, // NOP
        0x4E, 0x71, // NOP
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
    rom
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chronofork_backend_api::{Backend, BackendFactory, BackendSnapshot};
    use chronofork_core::InputFrame;
    use chronofork_sessions::SessionManager;

    use super::{
        HBLANK_VECTOR_ADDRESS, MegadriveBackend, MegadriveFactory, PAYLOAD_VERSION,
        VBLANK_VECTOR_ADDRESS, demo_rom, interrupt_rom,
    };
    use crate::bus::RAM_BASE;
    use crate::interrupts::{InterruptLevel, InterruptScheduler, SchedulerConfig, VideoStandard};
    use crate::vdp::{SNAPSHOT_MAGIC, STATUS_FIFO_EMPTY, STATUS_VBLANK, STATUS_VINT_PENDING};

    fn factory() -> MegadriveFactory {
        MegadriveFactory::new(demo_rom()).expect("valid demo ROM")
    }

    fn fast_factory() -> MegadriveFactory {
        let config = SchedulerConfig::new(VideoStandard::Ntsc, 4).expect("positive quantum");
        MegadriveFactory::with_scheduler_config(demo_rom(), config).expect("valid demo ROM")
    }

    /// Factory for the mask-gating synthetic ROM with a four-instruction line.
    fn interrupt_factory(mask: u8) -> MegadriveFactory {
        let config = SchedulerConfig::new(VideoStandard::Ntsc, 4).expect("positive quantum");
        MegadriveFactory::with_scheduler_config(interrupt_rom(mask), config)
            .expect("valid interrupt ROM")
    }

    fn interrupt_machine(mask: u8) -> MegadriveBackend {
        interrupt_factory(mask)
            .create_machine()
            .expect("interrupt machine")
    }

    /// Steps until the scheduler tick reaches `tick`.
    fn step_to_tick(machine: &mut MegadriveBackend, tick: u64) {
        while machine.tick().raw() < tick {
            machine.step(InputFrame::NEUTRAL).expect("step to tick");
        }
        assert_eq!(
            machine.tick().raw(),
            tick,
            "stepped past the requested tick"
        );
    }

    fn register(machine: &MegadriveBackend, name: &str) -> u64 {
        machine.inspect().expect("inspect").registers[name]
    }

    #[test]
    fn reset_vectors_and_demo_program_execute() {
        let factory = factory();
        assert!(!factory.descriptor().capabilities().rendering);
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
        assert_eq!(state.tick.raw(), 6);
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

        // MOVE.W #$2700,SR, six NOPs, then MOVE.W #$2300,SR.
        assert_eq!(&rom[0x200..0x204], &[0x46, 0xFC, 0x27, 0x00]);
        for offset in [0x204, 0x206, 0x208, 0x20A, 0x20C, 0x20E] {
            assert_eq!(
                &rom[offset..offset + 2],
                &[0x4E, 0x71],
                "NOP at {offset:#X}"
            );
        }
        assert_eq!(&rom[0x210..0x214], &[0x46, 0xFC, 0x23, 0x00]);
        assert_eq!(&rom[0x214..0x216], &[0x70, 0x00]); // MOVEQ #0,D0
        assert_eq!(
            &rom[0x216..0x21C],
            &[0x3A, 0x39, 0x00, 0xC0, 0x00, 0x04] // MOVE.W (0x00C00004).L,D5
        );
        assert_eq!(&rom[0x21C..0x21E], &[0x60, 0xF8]); // BRA.S status read

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
        assert_eq!(&interrupt_rom(7)[0x210..0x214], &[0x46, 0xFC, 0x27, 0x00]);
        assert_eq!(&interrupt_rom(0)[0x210..0x214], &[0x46, 0xFC, 0x20, 0x00]);
    }

    #[test]
    fn masked_hblank_is_retained_until_the_program_lowers_the_mask() {
        let mut machine = interrupt_machine(3);

        step_to_tick(&mut machine, 4);
        let masked = machine.inspect().expect("inspect");
        assert_eq!(masked.registers["pc"], 0x20A);
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

        // Seven quanta run with the request pending and the mask still at 7.
        step_to_tick(&mut machine, 7);
        let still_masked = machine.inspect().expect("inspect");
        assert_eq!(still_masked.registers["pc"], 0x210);
        assert_eq!(still_masked.registers["irq_mask"], 7);
        assert_eq!(still_masked.registers["irq_pending_level"], 4);
        assert_eq!(still_masked.registers["irq_deliverable"], 0);
        assert_eq!(still_masked.registers["d7"], 0);
        assert_eq!(still_masked.registers["d6"], 0);
        assert!(machine.interrupts().is_pending(InterruptLevel::HBlank));
        assert_eq!(
            machine.interrupts().requests_issued(InterruptLevel::HBlank),
            1
        );
        assert_eq!(
            machine
                .interrupts()
                .requests_coalesced(InterruptLevel::HBlank),
            0
        );

        // Instruction 8 lowers the mask with MOVE.W #$2300,SR. Delivery is only
        // evaluated at the *next* instruction boundary, so the request is still
        // pending — but it is now deliverable.
        step_to_tick(&mut machine, 8);
        let lowered = machine.inspect().expect("inspect");
        assert_eq!(lowered.registers["irq_mask"], 3);
        assert_eq!(lowered.registers["irq_pending_level"], 4);
        assert_eq!(lowered.registers["irq_deliverable"], 1);
        assert_eq!(lowered.registers["d7"], 0);
        assert!(machine.interrupts().is_pending(InterruptLevel::HBlank));
        assert_eq!(
            machine.interrupts().requests_issued(InterruptLevel::HBlank),
            2,
            "the line completed by instruction 8 raised a second HBlank"
        );
        assert_eq!(
            machine
                .interrupts()
                .requests_coalesced(InterruptLevel::HBlank),
            1,
            "the second masked HBlank coalesces into the retained request"
        );

        // Instruction 9 delivers exactly the retained HBlank request and runs the
        // handler's first instruction. Exception entry loads the accepted level 4
        // into the IPM, so the handler runs at mask 4 even though the program that
        // lowered its mask to 3 is interrupted.
        machine.step(InputFrame::NEUTRAL).expect("deliver HBlank");
        let delivered = machine.inspect().expect("inspect");
        assert_eq!(delivered.tick.raw(), 9);
        assert_eq!(delivered.registers["d7"], 1, "the handler counter ran");
        assert_eq!(delivered.registers["pc"], 0x2C2);
        assert_eq!(
            delivered.registers["irq_mask"], 4,
            "handler entry loaded IPM 4"
        );
        assert_eq!(machine.cpu().interrupt_priority_mask(), 4);
        assert_eq!(delivered.registers["irq_pending_level"], 0);
        assert_eq!(delivered.registers["irq_deliverable"], 0);
        assert!(!machine.interrupts().is_pending(InterruptLevel::HBlank));
        assert_eq!(
            machine.interrupts().requests_issued(InterruptLevel::HBlank),
            2,
            "delivery acknowledges the request, it does not reissue it"
        );

        // The frame holds the pre-interrupt raw SR and PC, which RTE must restore.
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
        assert_eq!(saved_pc, 0x214);
        assert_eq!(
            delivered.memory[frame..frame + 6],
            [0x23, 0x00, 0x00, 0x00, 0x02, 0x14]
        );

        // The handler changes CCR (MOVEQ #-1,D3 sets N) before RTE.
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
        assert_eq!(returned.tick.raw(), 11);
        assert_eq!(
            returned.registers["pc"], 0x214,
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
        let mut machine = interrupt_machine(3);
        step_to_tick(&mut machine, 896);
        let pending = machine.inspect().expect("inspect");
        assert_eq!(pending.registers["irq_mask"], 3);
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
        let d7_before_vblank = pending.registers["d7"];

        // VBlank (level 6) is delivered before the pending HBlank (level 4).
        // Exception entry loads IPM 6, so the pending HBlank cannot nest.
        machine.step(InputFrame::NEUTRAL).expect("deliver VBlank");
        let delivered = machine.inspect().expect("inspect");
        assert_eq!(delivered.tick.raw(), 897);
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
        assert_eq!(
            machine.interrupts().requests_issued(InterruptLevel::VBlank),
            1
        );
        assert!(
            !machine.interrupts().is_pending(InterruptLevel::VBlank),
            "the delivered VBlank request is acknowledged exactly once"
        );
        assert!(
            machine.interrupts().is_pending(InterruptLevel::HBlank),
            "acknowledging VBlank must retain the pending HBlank"
        );
        assert_eq!(
            delivered.registers["irq_pending_level"],
            u64::from(InterruptLevel::HBlank.level())
        );
        assert_eq!(
            delivered.registers["irq_vector"],
            u64::from(HBLANK_VECTOR_ADDRESS)
        );
        assert_eq!(
            delivered.registers["vdp_status"] & u64::from(STATUS_VINT_PENDING),
            0,
            "VINT pending clears when the VBlank request is acknowledged"
        );
        assert_eq!(
            delivered.registers["vdp_status"] & u64::from(STATUS_VBLANK),
            u64::from(STATUS_VBLANK),
            "the video VBlank flag is independent of the interrupt latch"
        );

        // The next instruction boundary must not deliver the pending HBlank while
        // the handler runs at IPM 6: no nested HBlank handler, one frame only.
        machine
            .step(InputFrame::NEUTRAL)
            .expect("handler MOVEQ, no nested HBlank");
        let in_handler = machine.inspect().expect("inspect");
        assert_eq!(in_handler.registers["d3"], 0xFFFF_FFFF);
        assert_eq!(in_handler.registers["pc"], 0x2E4);
        assert_eq!(machine.cpu().interrupt_priority_mask(), 6);
        assert_eq!(
            in_handler.registers["a7"], 0x00FF_FFFA,
            "only the VBlank frame is on the stack"
        );
        assert!(
            machine.interrupts().is_pending(InterruptLevel::HBlank),
            "the HBlank request must remain pending while IPM is 6"
        );
        assert_eq!(in_handler.registers["irq_pending_level"], 4);
        assert_eq!(
            in_handler.registers["d7"], d7_before_vblank,
            "no nested HBlank handler ran"
        );

        machine.step(InputFrame::NEUTRAL).expect("VBlank RTE");
        let returned = machine.inspect().expect("inspect");
        assert_eq!(returned.tick.raw(), 899);
        assert_eq!(
            returned.registers["pc"], pre_pc,
            "RTE returns to the interrupted PC"
        );
        assert_eq!(machine.cpu().sr(), pre_sr, "RTE restores the saved raw SR");
        assert_eq!(returned.registers["irq_mask"], 3);
        assert!(
            machine
                .cpu()
                .accepts_interrupt_level(InterruptLevel::HBlank.level()),
            "RTE restores mask 3, so HBlank is deliverable again"
        );
        assert_eq!(returned.registers["d6"], 1);
        assert_eq!(
            returned.registers["a7"], 0x0100_0000,
            "RTE pops the six-byte frame"
        );
        assert!(machine.interrupts().is_pending(InterruptLevel::HBlank));

        // The retained HBlank is delivered exactly once after RTE restored mask 3.
        let d7_before = returned.registers["d7"];
        machine
            .step(InputFrame::NEUTRAL)
            .expect("deliver retained HBlank");
        let hblank = machine.inspect().expect("inspect");
        assert_eq!(
            hblank.registers["d7"],
            d7_before + 1,
            "the retained HBlank is delivered exactly once"
        );
        assert_eq!(hblank.registers["pc"], 0x2C2);
        assert_eq!(
            hblank.registers["irq_mask"], 4,
            "HBlank handler runs at IPM 4"
        );
        assert!(!machine.interrupts().is_pending(InterruptLevel::VBlank));
        assert_eq!(
            hblank.registers["vdp_status"] & u64::from(STATUS_VINT_PENDING),
            0
        );
        machine
            .step(InputFrame::NEUTRAL)
            .expect("HBlank handler MOVEQ");
        machine.step(InputFrame::NEUTRAL).expect("HBlank RTE");
        assert_eq!(
            machine.inspect().expect("inspect").registers["a7"],
            0x0100_0000
        );
    }

    #[test]
    fn vblank_handler_ipm_six_blocks_hblank_reentry_until_rte() {
        let mut machine = interrupt_machine(3);
        step_to_tick(&mut machine, 896);
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
        step_to_tick(&mut machine, 898);
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
        step_to_tick(&mut machine, 2100);
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
        step_to_tick(&mut machine, 898);
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

    #[test]
    fn acknowledging_one_level_never_clears_the_other() {
        // The backend can only take VBlank first (68000 priority), so the reverse
        // acknowledgement order is exercised through the same scheduler primitive
        // the backend acknowledges with.
        let config = SchedulerConfig::new(VideoStandard::Ntsc, 4).expect("positive quantum");

        let mut vblank_first = InterruptScheduler::new(config);
        vblank_first.advance(896).expect("advance to VBlank");
        assert!(vblank_first.is_pending(InterruptLevel::VBlank));
        assert!(vblank_first.is_pending(InterruptLevel::HBlank));
        assert!(vblank_first.acknowledge(InterruptLevel::VBlank));
        assert!(!vblank_first.is_pending(InterruptLevel::VBlank));
        assert!(vblank_first.is_pending(InterruptLevel::HBlank));

        let mut hblank_first = InterruptScheduler::new(config);
        hblank_first.advance(896).expect("advance to VBlank");
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
        assert_eq!(snapshot.tick().raw(), 64);

        let vdp_bytes = machine.vdp().snapshot_bytes();
        let scheduler_bytes = machine.interrupts().snapshot_bytes();
        assert!(
            snapshot
                .payload()
                .windows(vdp_bytes.len())
                .any(|window| window == vdp_bytes.as_slice())
        );
        assert!(snapshot.payload().ends_with(&scheduler_bytes));
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
        assert_eq!(restored.tick.raw(), 64);
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
        step_to_tick(&mut machine, 898);
        let snapshot = machine.snapshot().expect("snapshot");
        let expected = machine.inspect().expect("inspect").state_hash;
        assert_eq!(snapshot.state_hash(), expected);
        assert_eq!(snapshot.payload()[8..10], PAYLOAD_VERSION.to_le_bytes());
        assert!(
            snapshot
                .payload()
                .ends_with(&machine.interrupts().snapshot_bytes())
        );

        // A masked step keeps the request and never runs a handler.
        machine.step(InputFrame::NEUTRAL).expect("masked step");
        assert_eq!(register(&machine, "d6"), 0);
        assert!(machine.interrupts().is_pending(InterruptLevel::VBlank));

        machine.restore(&snapshot).expect("restore");
        let restored = machine.inspect().expect("inspect");
        assert_eq!(restored.state_hash, expected);
        assert_eq!(restored.tick.raw(), 898);
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
        step_to_tick(&mut machine, 896);
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
        assert_eq!(restored.tick.raw(), 896);
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

        let mut other_timing = fast_factory().create_machine().expect("timing machine");
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

        let mut old_payload = snapshot.payload().to_vec();
        old_payload[0..8].copy_from_slice(b"CFMD68K1");
        old_payload[8..10].copy_from_slice(&1_u16.to_le_bytes());
        let old = BackendSnapshot::new(
            snapshot.backend_id().clone(),
            snapshot.backend_version(),
            snapshot.format_version(),
            snapshot.tick(),
            snapshot.config_hash(),
            snapshot.state_hash(),
            old_payload,
        );
        assert!(machine.restore(&old).is_err());

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
        assert_eq!(machine.inspect().expect("inspect").tick.raw(), 6);
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
        for _ in 0..7 {
            manager.step(source, InputFrame::NEUTRAL).expect("step");
        }
        let checkpoint = manager.snapshot(source).expect("checkpoint");
        let masked = manager.inspect(source).expect("inspect");
        assert_eq!(masked.machine.tick.raw(), 7);
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

        // The fork lowers the mask with instruction 8 and takes the retained
        // HBlank at instruction 9.
        manager.step(fork, InputFrame::NEUTRAL).expect("lower mask");
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
        step_to_tick(&mut left, 950);
        step_to_tick(&mut right, 950);
        let left_view = left.inspect().expect("left");
        let right_view = right.inspect().expect("right");
        assert_eq!(left_view.state_hash, right_view.state_hash);
        assert_eq!(left.vdp().status(), right.vdp().status());
        assert!(
            left_view.registers["d6"] >= 1,
            "both machines must have delivered VBlank by tick 950"
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
        assert!(machine.write_memory(0x200, &[0]).is_err());
        machine
            .write_memory(u64::from(RAM_BASE), &[0x12, 0x34])
            .expect("ram write");
        assert_eq!(
            machine
                .read_memory(u64::from(RAM_BASE), 2)
                .expect("ram read"),
            vec![0x12, 0x34]
        );
        assert!(machine.read_memory(0x00C0_0000, 1).is_err());
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
}
