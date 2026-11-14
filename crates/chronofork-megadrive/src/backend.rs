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
//! instruction, at most one pending VBlank (level 6) or HBlank (level 4) request
//! is delivered through [`M68000::interrupt`] using the 68000 autovector address
//! (`0x78` or `0x70`). VBlank is delivered first when both levels are pending.
//! The 68000 `SR` interrupt mask is exposed for inspection but does **not** gate
//! delivery yet: no instruction in the current subset can change the mask, and
//! the CPU entry API leaves masking to the caller. The VDP VBlank status flag is
//! driven from the scheduler's scanline position.
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
//! Because the current core does not gate interrupts with the `SR` mask (E2-15),
//! differential runs that the oracle keeps interrupt-free must construct their
//! machine with an explicit large scheduler quantum such as `1_000_000`
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

    fn deliver_pending_interrupt(&mut self) -> Result<(), BackendError> {
        if let Some(level) = self.scheduler.next_pending() {
            self.cpu
                .interrupt(vector_address(level), &mut self.bus)
                .map_err(map_cpu)?;
            self.scheduler.acknowledge(level);
        }
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
        self.sync_vdp_vblank();
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
/// tiny handlers that increment `D7`/`D6` and loop. The ROM uses no copyrighted
/// data and is not a game.
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chronofork_backend_api::{Backend, BackendFactory, BackendSnapshot};
    use chronofork_core::InputFrame;
    use chronofork_sessions::SessionManager;

    use super::{
        HBLANK_VECTOR_ADDRESS, MegadriveFactory, PAYLOAD_VERSION, VBLANK_VECTOR_ADDRESS, demo_rom,
    };
    use crate::bus::RAM_BASE;
    use crate::interrupts::{InterruptLevel, SchedulerConfig, VideoStandard};
    use crate::vdp::{SNAPSHOT_MAGIC, STATUS_FIFO_EMPTY, STATUS_VBLANK};

    fn factory() -> MegadriveFactory {
        MegadriveFactory::new(demo_rom()).expect("valid demo ROM")
    }

    fn fast_factory() -> MegadriveFactory {
        let config = SchedulerConfig::new(VideoStandard::Ntsc, 4).expect("positive quantum");
        MegadriveFactory::with_scheduler_config(demo_rom(), config).expect("valid demo ROM")
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
    fn interrupt_scheduler_delivers_requests_to_the_cpu() {
        let factory = fast_factory();
        let mut machine = factory.create_machine().expect("machine");
        for _ in 0..4 {
            machine.step(InputFrame::NEUTRAL).expect("step");
        }
        let pending = machine.inspect().expect("inspect");
        assert_eq!(pending.tick.raw(), 4);
        assert_eq!(
            pending.registers["irq_pending_level"],
            u64::from(InterruptLevel::HBlank.level())
        );
        assert_eq!(
            pending.registers["irq_vector"],
            u64::from(HBLANK_VECTOR_ADDRESS)
        );
        assert!(machine.interrupts().is_pending(InterruptLevel::HBlank));
        assert_eq!(
            machine.interrupts().requests_issued(InterruptLevel::HBlank),
            1
        );

        machine.step(InputFrame::NEUTRAL).expect("interrupt entry");
        let delivered = machine.inspect().expect("inspect");
        assert_eq!(delivered.tick.raw(), 5);
        assert_eq!(delivered.registers["d7"], 1);
        assert_eq!(delivered.registers["pc"], 0x2C2);
        assert_eq!(delivered.registers["irq_pending_level"], 0);
        assert!(!machine.interrupts().is_pending(InterruptLevel::HBlank));

        let vblank_tick = u64::from(VideoStandard::Ntsc.active_lines()) * 4;
        for _ in 5..vblank_tick {
            machine.step(InputFrame::NEUTRAL).expect("step to VBlank");
        }
        let vblank = machine.inspect().expect("inspect");
        assert_eq!(vblank.tick.raw(), vblank_tick);
        assert_eq!(
            vblank.registers["irq_pending_level"],
            u64::from(InterruptLevel::VBlank.level())
        );
        assert_eq!(
            vblank.registers["irq_vector"],
            u64::from(VBLANK_VECTOR_ADDRESS)
        );
        assert_eq!(
            vblank.registers["vdp_status"] & u64::from(STATUS_VBLANK),
            u64::from(STATUS_VBLANK)
        );
        assert!(machine.interrupts().is_pending(InterruptLevel::VBlank));
        assert!(machine.interrupts().is_pending(InterruptLevel::HBlank));

        // VBlank (level 6) is delivered before the coalesced HBlank (level 4).
        machine.step(InputFrame::NEUTRAL).expect("VBlank entry");
        let vblank_delivered = machine.inspect().expect("inspect");
        assert_eq!(vblank_delivered.registers["d6"], 1);
        assert_eq!(
            vblank_delivered.registers["irq_pending_level"],
            u64::from(InterruptLevel::HBlank.level())
        );
        machine.step(InputFrame::NEUTRAL).expect("HBlank entry");
        let after = machine.inspect().expect("inspect");
        assert_eq!(after.registers["irq_pending_level"], 0);
        assert!(after.registers["d7"] > 1);
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
        assert!(machine.interrupts().is_pending(InterruptLevel::HBlank));

        machine.step(InputFrame::NEUTRAL).expect("delivery");
        assert_eq!(machine.inspect().expect("inspect").registers["d7"], 1);
        machine.restore(&snapshot).expect("restore");
        let restored = machine.inspect().expect("inspect");
        assert_eq!(restored.state_hash, expected);
        assert_eq!(restored.tick.raw(), 64);
        assert_eq!(
            restored.registers["irq_pending_level"],
            u64::from(InterruptLevel::HBlank.level())
        );
        assert_eq!(machine.vdp().register(15), Some(2));
        assert_eq!(&machine.vdp().vram()[0..2], &[0x12, 0x34]);
        assert!(machine.interrupts().is_pending(InterruptLevel::HBlank));

        // The restored pending request is delivered deterministically.
        machine.step(InputFrame::NEUTRAL).expect("replay");
        assert_eq!(machine.inspect().expect("inspect").registers["d7"], 1);
    }

    #[test]
    fn pending_vblank_state_survives_snapshot_restore() {
        let mut machine = fast_factory().create_machine().expect("machine");
        for _ in 0..896 {
            machine.step(InputFrame::NEUTRAL).expect("step");
        }
        assert!(machine.interrupts().is_pending(InterruptLevel::VBlank));
        let snapshot = machine.snapshot().expect("snapshot");
        machine.step(InputFrame::NEUTRAL).expect("deliver VBlank");
        assert_eq!(machine.inspect().expect("inspect").registers["d6"], 1);

        machine.restore(&snapshot).expect("restore");
        let restored = machine.inspect().expect("inspect");
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
            restored.registers["vdp_status"] & u64::from(STATUS_VBLANK),
            u64::from(STATUS_VBLANK)
        );
        assert!(machine.interrupts().is_pending(InterruptLevel::VBlank));
        machine.step(InputFrame::NEUTRAL).expect("replay VBlank");
        assert_eq!(machine.inspect().expect("inspect").registers["d6"], 1);
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
        assert_eq!(before.machine.registers["irq_pending_level"], 4);

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
        assert_eq!(fork_after.machine.registers["d7"], 1);

        manager.restore(fork, checkpoint).expect("restore fork");
        let restored = manager.inspect(fork).expect("restored");
        assert_eq!(restored.machine.state_hash, before.machine.state_hash);
        assert_eq!(restored.machine.registers["irq_pending_level"], 4);
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
