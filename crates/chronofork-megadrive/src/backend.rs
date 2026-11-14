//! ChronoFork's first real-hardware backend slice.
//!
//! It is a minimal 68000 + ROM/RAM machine, **not** a complete Mega Drive. VDP,
//! audio, Z80, controllers, mapper hardware and interrupts are not implemented.
//! The backend advertises only what it actually provides and uses an
//! independently authored synthetic ROM in tests and the demo.

use std::sync::Arc;

use chronofork_backend_api::{
    BACKEND_SNAPSHOT_FORMAT_VERSION, Backend, BackendDescriptor, BackendError, BackendFactory,
    BackendId, BackendSnapshot, Capabilities, MachineInspection, StepOutcome,
};
use chronofork_core::{InputFrame, StateHash, Tick, hash_bytes};

use crate::bus::{MegadriveBus, RAM_BASE, RAM_SIZE, ROM_END};
use crate::cpu::{CpuError, M68000};

/// Stable backend identifier.
pub const BACKEND_ID: &str = "chronofork.megadrive.m68000";
/// Payload format version.
pub const PAYLOAD_VERSION: u16 = 1;
const PAYLOAD_MAGIC: [u8; 8] = *b"CFMD68K1";
const STATE_MAGIC: &[u8; 10] = b"CFMDSTATE1";

/// Factory for independent minimal Mega Drive machines.
#[derive(Clone, Debug)]
pub struct MegadriveFactory {
    descriptor: BackendDescriptor,
    rom: Arc<[u8]>,
    config_hash: StateHash,
}

impl MegadriveFactory {
    /// Creates a factory from an independently provided cartridge image.
    ///
    /// The image must contain at least the reset vectors and must not exceed the
    /// 68000 cartridge window. No ROM is bundled with this crate.
    pub fn new(rom: Vec<u8>) -> Result<Self, BackendError> {
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
        let config_hash = hash_bytes(rom.as_ref());
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
                "ChronoFork Mega Drive 68000 (minimal subset, no VDP/audio)",
                env!("CARGO_PKG_VERSION"),
                capabilities,
                false,
            ),
            rom,
            config_hash,
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
}

impl BackendFactory for MegadriveFactory {
    fn descriptor(&self) -> &BackendDescriptor {
        &self.descriptor
    }

    fn create(&self) -> Result<Box<dyn Backend>, BackendError> {
        let mut bus = MegadriveBus::new(Arc::clone(&self.rom));
        let mut cpu = M68000::new();
        cpu.reset(&mut bus).map_err(map_cpu)?;
        Ok(Box::new(MegadriveBackend {
            descriptor: self.descriptor.clone(),
            config_hash: self.config_hash,
            bus,
            cpu,
            tick: Tick::ZERO,
        }))
    }
}

/// One independent minimal Mega Drive machine.
#[derive(Debug)]
pub struct MegadriveBackend {
    descriptor: BackendDescriptor,
    config_hash: StateHash,
    bus: MegadriveBus,
    cpu: M68000,
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

    fn state_hash(&self) -> StateHash {
        let (d, a, pc, sr, halted) = self.cpu.snapshot_state();
        state_hash_for(&d, &a, pc, sr, halted, self.tick, self.bus.ram())
    }

    fn payload(&self) -> Vec<u8> {
        let (d, a, pc, sr, halted) = self.cpu.snapshot_state();
        let mut payload = Vec::with_capacity(8 + 2 + 32 + 32 + 4 + 2 + 1 + 8 + RAM_SIZE);
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
        payload
    }
}

impl Backend for MegadriveBackend {
    fn descriptor(&self) -> &BackendDescriptor {
        &self.descriptor
    }

    fn step(&mut self, _input: InputFrame) -> Result<StepOutcome, BackendError> {
        self.cpu.step(&mut self.bus).map_err(map_cpu)?;
        self.tick = self.tick.next().map_err(|_| BackendError::TickOverflow)?;
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
        let actual = state_hash_for(
            &parsed.d,
            &parsed.a,
            parsed.pc,
            parsed.sr,
            parsed.halted,
            parsed.tick,
            &parsed.ram,
        );
        if actual != snapshot.state_hash() {
            return Err(BackendError::InvalidSnapshot(format!(
                "state hash mismatch after restore: expected {}, got {actual}",
                snapshot.state_hash()
            )));
        }
        self.bus
            .restore_ram(parsed.ram)
            .map_err(|error| BackendError::InvalidSnapshot(error.to_string()))?;
        self.cpu
            .restore_state(parsed.d, parsed.a, parsed.pc, parsed.sr, parsed.halted);
        self.tick = parsed.tick;
        Ok(())
    }

    fn inspect(&self) -> Result<MachineInspection, BackendError> {
        Ok(MachineInspection {
            tick: self.tick,
            state_hash: self.state_hash(),
            halted: self.cpu.halted(),
            registers: self.cpu.registers(),
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

fn map_cpu(error: CpuError) -> BackendError {
    match error {
        CpuError::Halted => BackendError::Halted,
        other => BackendError::Execution(other.to_string()),
    }
}

fn state_hash_for(
    d: &[u32; 8],
    a: &[u32; 8],
    pc: u32,
    sr: u16,
    halted: bool,
    tick: Tick,
    ram: &[u8],
) -> StateHash {
    let mut bytes = Vec::with_capacity(10 + 32 + 32 + 4 + 2 + 1 + 8 + ram.len());
    bytes.extend_from_slice(STATE_MAGIC);
    for value in d {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    for value in a {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    bytes.extend_from_slice(&pc.to_be_bytes());
    bytes.extend_from_slice(&sr.to_be_bytes());
    bytes.push(u8::from(halted));
    bytes.extend_from_slice(&tick.raw().to_be_bytes());
    bytes.extend_from_slice(ram);
    hash_bytes(&bytes)
}

#[derive(Debug)]
struct ParsedState {
    d: [u32; 8],
    a: [u32; 8],
    pc: u32,
    sr: u16,
    halted: bool,
    tick: Tick,
    ram: Vec<u8>,
}

fn parse_payload(payload: &[u8]) -> Result<ParsedState, BackendError> {
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
    let ram = reader.read_exact(RAM_SIZE)?.to_vec();
    if reader.remaining() != 0 {
        return Err(BackendError::InvalidSnapshot(format!(
            "Mega Drive payload has {} trailing bytes",
            reader.remaining()
        )));
    }
    Ok(ParsedState {
        d,
        a,
        pc,
        sr,
        halted,
        tick,
        ram,
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
/// The program computes `1 + 2`, stores the word at work-RAM address 0x00FF0000,
/// reloads it into `D2`, then loops. It is not a game and uses no copyrighted
/// data.
#[must_use]
pub fn demo_rom() -> Vec<u8> {
    let mut rom = vec![0_u8; 0x400];
    for word in rom.chunks_exact_mut(2) {
        word.copy_from_slice(&[0x4E, 0x71]); // NOP fill
    }
    rom[0..4].copy_from_slice(&0x00FF_0000_u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());
    rom[0x200..0x216].copy_from_slice(&[
        0x70, 0x01, // MOVEQ #1,D0
        0x72, 0x02, // MOVEQ #2,D1
        0xD0, 0x41, // ADD.W D1,D0
        0x33, 0xC0, // MOVE.W D0,(0x00FF0000).L
        0x00, 0xFF, 0x00, 0x00, // absolute-long address
        0x74, 0x00, // MOVEQ #0,D2
        0x34, 0x38, // MOVE.W (0x00FF0000).L,D2
        0x00, 0xFF, 0x00, 0x00, // absolute-long address
        0x60, 0xFE, // BRA.S -2
    ]);
    rom
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chronofork_backend_api::BackendFactory;
    use chronofork_core::InputFrame;
    use chronofork_sessions::SessionManager;

    use super::{MegadriveFactory, demo_rom};
    use crate::bus::RAM_BASE;

    fn factory() -> MegadriveFactory {
        MegadriveFactory::new(demo_rom()).expect("valid demo ROM")
    }

    #[test]
    fn reset_vectors_and_demo_program_execute() {
        let factory = factory();
        assert!(!factory.descriptor().capabilities().rendering);
        let mut machine = factory.create().expect("machine");
        let initial = machine.inspect().expect("inspect");
        assert_eq!(initial.registers["pc"], 0x200);
        assert_eq!(initial.registers["a7"], 0x00FF_0000);
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
    fn snapshots_round_trip_and_detect_configuration_mismatch() {
        let factory = factory();
        let mut machine = factory.create().expect("machine");
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
        let mut other_machine = other.create().expect("other machine");
        assert!(other_machine.restore(&snapshot).is_err());
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
    fn two_machines_converge_on_the_same_program() {
        let factory = factory();
        let mut left = factory.create().expect("left");
        let mut right = factory.create().expect("right");
        for _ in 0..8 {
            left.step(InputFrame::NEUTRAL).expect("left step");
            right.step(InputFrame::NEUTRAL).expect("right step");
        }
        assert_eq!(
            left.inspect().expect("left").state_hash,
            right.inspect().expect("right").state_hash
        );
    }

    #[test]
    fn invalid_rom_and_illegal_opcode_are_errors() {
        assert!(MegadriveFactory::new(vec![0; 4]).is_err());
        let mut rom = demo_rom();
        rom[0x200..0x202].copy_from_slice(&[0xFF, 0xFF]);
        let factory = MegadriveFactory::new(rom).expect("factory");
        let mut machine = factory.create().expect("machine");
        assert!(machine.step(InputFrame::NEUTRAL).is_err());
    }

    #[test]
    fn debugger_writes_are_limited_to_ram() {
        let factory = factory();
        let mut machine = factory.create().expect("machine");
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
    }
}
