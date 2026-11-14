#![cfg_attr(test, allow(clippy::expect_used))]
//! A deterministic synthetic backend used to validate ChronoFork's contracts.
//!
//! **This is not a Sega Mega Drive / Genesis emulator.** It executes a tiny
//! documented bytecode over a 256-byte memory space. Its value is that its
//! state is simple enough to verify independently, which makes it a useful
//! fixture for snapshot, fork, replay and isolation tests.
//!
//! The backend is deliberately isolated from the generic session layer: the
//! session layer knows only the [`chronofork_backend_api`] contracts.

use std::collections::BTreeMap;
use std::sync::Arc;

use chronofork_backend_api::{
    BACKEND_SNAPSHOT_FORMAT_VERSION, Backend, BackendDescriptor, BackendError, BackendFactory,
    BackendId, BackendSnapshot, Capabilities, MachineInspection, StepOutcome,
};
use chronofork_core::{InputFrame, StateHash, Tick, hash_bytes};

/// Identifier of this synthetic backend.
pub const BACKEND_ID: &str = "chronofork.test.synthetic";

/// Version of the synthetic backend payload format.
pub const PAYLOAD_FORMAT_VERSION: u16 = 1;

/// Independent verifiable program used by the default factory.
pub const DEFAULT_PROGRAM: &[u8] = &[
    0x01, // INC_ACC
    0x02, 0x03, // ADD_IMM 3
    0x09, // APPLY_INPUT
    0x03, 0x20, // STORE_RAM 0x20
    0x06, 0x00, 0x00, // JMP 0x0000
];

/// An immutable synthetic program.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyntheticProgram {
    code: Vec<u8>,
    hash: StateHash,
}

impl SyntheticProgram {
    /// Creates a program from documented bytecode.
    pub fn new(code: Vec<u8>) -> Result<Self, BackendError> {
        if code.is_empty() {
            return Err(BackendError::InvalidConfiguration(
                "synthetic program must not be empty".to_owned(),
            ));
        }
        if code.len() > usize::from(u16::MAX) {
            return Err(BackendError::InvalidConfiguration(
                "synthetic program exceeds the 16-bit address space".to_owned(),
            ));
        }
        let hash = hash_bytes(&code);
        Ok(Self { code, hash })
    }

    /// Returns the program bytes.
    #[must_use]
    pub fn code(&self) -> &[u8] {
        &self.code
    }

    /// Returns a digest of the immutable program.
    #[must_use]
    pub const fn hash(&self) -> StateHash {
        self.hash
    }
}

impl Default for SyntheticProgram {
    fn default() -> Self {
        let code = DEFAULT_PROGRAM.to_vec();
        let hash = hash_bytes(&code);
        Self { code, hash }
    }
}

/// Factory for fresh synthetic machines.
#[derive(Clone, Debug)]
pub struct SyntheticBackendFactory {
    descriptor: BackendDescriptor,
    program: Arc<SyntheticProgram>,
}

impl SyntheticBackendFactory {
    /// Creates a factory for a specific program.
    #[must_use]
    pub fn new(program: SyntheticProgram) -> Self {
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
        Self {
            descriptor: BackendDescriptor::new(
                BackendId::new(BACKEND_ID),
                "ChronoFork Synthetic Test Backend (not a Mega Drive emulator)",
                env!("CARGO_PKG_VERSION"),
                capabilities,
                true,
            ),
            program: Arc::new(program),
        }
    }
}

impl Default for SyntheticBackendFactory {
    fn default() -> Self {
        Self::new(SyntheticProgram::default())
    }
}

impl BackendFactory for SyntheticBackendFactory {
    fn descriptor(&self) -> &BackendDescriptor {
        &self.descriptor
    }

    fn create(&self) -> Result<Box<dyn Backend>, BackendError> {
        Ok(Box::new(SyntheticBackend {
            descriptor: self.descriptor.clone(),
            program: Arc::clone(&self.program),
            pc: 0,
            acc: 0,
            ram: [0; 256],
            tick: Tick::ZERO,
            steps: 0,
            halted: false,
        }))
    }
}

/// One independent synthetic machine.
#[derive(Clone, Debug)]
pub struct SyntheticBackend {
    descriptor: BackendDescriptor,
    program: Arc<SyntheticProgram>,
    pc: u16,
    acc: u32,
    ram: [u8; 256],
    tick: Tick,
    steps: u64,
    halted: bool,
}

impl SyntheticBackend {
    fn state_hash(&self) -> StateHash {
        let mut state = Vec::with_capacity(285);
        state.extend_from_slice(b"CFTBSTATE1");
        state.extend_from_slice(&self.pc.to_le_bytes());
        state.extend_from_slice(&self.acc.to_le_bytes());
        state.extend_from_slice(&self.tick.raw().to_le_bytes());
        state.extend_from_slice(&self.steps.to_le_bytes());
        state.push(u8::from(self.halted));
        state.extend_from_slice(&self.ram);
        hash_bytes(&state)
    }

    fn snapshot_payload(&self) -> Vec<u8> {
        let mut payload = Vec::with_capacity(285);
        payload.extend_from_slice(b"CFTB");
        payload.extend_from_slice(&PAYLOAD_FORMAT_VERSION.to_le_bytes());
        payload.extend_from_slice(&self.pc.to_le_bytes());
        payload.extend_from_slice(&self.acc.to_le_bytes());
        payload.extend_from_slice(&self.tick.raw().to_le_bytes());
        payload.extend_from_slice(&self.steps.to_le_bytes());
        payload.push(u8::from(self.halted));
        payload.extend_from_slice(&self.ram);
        payload
    }

    fn fetch_u8(&mut self) -> Result<u8, BackendError> {
        let index = usize::from(self.pc);
        let value = *self.program.code().get(index).ok_or_else(|| {
            BackendError::InvalidConfiguration("program counter out of range".to_owned())
        })?;
        self.pc = self.pc.checked_add(1).ok_or_else(|| {
            BackendError::InvalidConfiguration("program counter overflow".to_owned())
        })?;
        Ok(value)
    }

    fn apply_snapshot_payload(
        &mut self,
        payload: &[u8],
        expected_hash: StateHash,
    ) -> Result<(), BackendError> {
        let mut reader = PayloadReader::new(payload);
        let magic = reader.read_array::<4>()?;
        if magic != *b"CFTB" {
            return Err(BackendError::InvalidSnapshot(
                "synthetic backend payload magic mismatch".to_owned(),
            ));
        }
        let version = reader.read_u16()?;
        if version != PAYLOAD_FORMAT_VERSION {
            return Err(BackendError::InvalidSnapshot(format!(
                "synthetic backend payload version {version}, expected {PAYLOAD_FORMAT_VERSION}"
            )));
        }
        let pc = reader.read_u16()?;
        let acc = reader.read_u32()?;
        let tick = Tick::from_raw(reader.read_u64()?);
        let steps = reader.read_u64()?;
        let halted = reader.read_u8()? != 0;
        let ram = reader.read_array::<256>()?;
        if reader.remaining() != 0 {
            return Err(BackendError::InvalidSnapshot(format!(
                "synthetic backend payload has {} trailing bytes",
                reader.remaining()
            )));
        }
        let restored = Self {
            descriptor: self.descriptor.clone(),
            program: Arc::clone(&self.program),
            pc,
            acc,
            ram,
            tick,
            steps,
            halted,
        };
        let actual_hash = restored.state_hash();
        if actual_hash != expected_hash {
            return Err(BackendError::InvalidSnapshot(format!(
                "state hash mismatch after restore: expected {expected_hash}, got {actual_hash}"
            )));
        }
        self.pc = restored.pc;
        self.acc = restored.acc;
        self.ram = restored.ram;
        self.tick = restored.tick;
        self.steps = restored.steps;
        self.halted = restored.halted;
        Ok(())
    }
}

impl Backend for SyntheticBackend {
    fn descriptor(&self) -> &BackendDescriptor {
        &self.descriptor
    }

    fn step(&mut self, input: InputFrame) -> Result<StepOutcome, BackendError> {
        if self.halted {
            return Err(BackendError::Halted);
        }
        let opcode = self.fetch_u8()?;
        match opcode {
            0x00 => {}
            0x01 => {
                self.acc = self.acc.wrapping_add(1);
            }
            0x02 => {
                let operand = self.fetch_u8()?;
                self.acc = self.acc.wrapping_add(u32::from(operand));
            }
            0x03 => {
                let address = usize::from(self.fetch_u8()?);
                let value = (self.acc & 0xff).to_le_bytes()[0];
                self.ram[address] = value;
            }
            0x04 => {
                let address = usize::from(self.fetch_u8()?);
                self.acc = u32::from(self.ram[address]);
            }
            0x05 => {
                let operand = self.fetch_u8()?;
                self.acc ^= u32::from(operand);
            }
            0x06 => {
                let low = self.fetch_u8()?;
                let high = self.fetch_u8()?;
                self.pc = u16::from_le_bytes([low, high]);
            }
            0x09 => {
                let input_byte = u8::try_from(input.buttons() & 0xff).map_err(|_| {
                    BackendError::Internal("input byte conversion failed".to_owned())
                })?;
                self.acc = self.acc.wrapping_add(u32::from(input_byte));
                self.ram[0x10] = (self.acc & 0xff).to_le_bytes()[0];
            }
            0xff => {
                self.halted = true;
            }
            other => {
                return Err(BackendError::InvalidConfiguration(format!(
                    "unknown synthetic opcode 0x{other:02x} at pc {}",
                    self.pc.saturating_sub(1)
                )));
            }
        }
        self.tick = self.tick.next()?;
        self.steps = self
            .steps
            .checked_add(1)
            .ok_or(BackendError::TickOverflow)?;
        Ok(StepOutcome {
            tick: self.tick,
            state_hash: self.state_hash(),
            halted: self.halted,
        })
    }

    fn snapshot(&self) -> Result<BackendSnapshot, BackendError> {
        Ok(BackendSnapshot::new(
            self.descriptor.id().clone(),
            self.descriptor.version(),
            BACKEND_SNAPSHOT_FORMAT_VERSION,
            self.tick,
            self.program.hash(),
            self.state_hash(),
            self.snapshot_payload(),
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
        if snapshot.config_hash() != self.program.hash() {
            return Err(BackendError::IncompatibleSnapshot {
                field: "config_hash",
                expected: self.program.hash().to_string(),
                found: snapshot.config_hash().to_string(),
            });
        }
        self.apply_snapshot_payload(snapshot.payload(), snapshot.state_hash())
    }

    fn inspect(&self) -> Result<MachineInspection, BackendError> {
        let mut registers = BTreeMap::new();
        registers.insert("pc".to_owned(), u64::from(self.pc));
        registers.insert("acc".to_owned(), u64::from(self.acc));
        registers.insert("tick".to_owned(), self.tick.raw());
        registers.insert("steps".to_owned(), self.steps);
        registers.insert("halted".to_owned(), u64::from(self.halted));
        Ok(MachineInspection {
            tick: self.tick,
            state_hash: self.state_hash(),
            halted: self.halted,
            registers,
            memory_base: 0,
            memory: self.ram.to_vec(),
        })
    }

    fn read_memory(&self, address: u64, length: usize) -> Result<Vec<u8>, BackendError> {
        let start = usize::try_from(address).map_err(|_| BackendError::OutOfBounds {
            address,
            length,
            limit: 256,
        })?;
        let end = start.checked_add(length).ok_or(BackendError::OutOfBounds {
            address,
            length,
            limit: 256,
        })?;
        if end > self.ram.len() {
            return Err(BackendError::OutOfBounds {
                address,
                length,
                limit: 256,
            });
        }
        Ok(self.ram[start..end].to_vec())
    }

    fn write_memory(&mut self, address: u64, data: &[u8]) -> Result<(), BackendError> {
        let start = usize::try_from(address).map_err(|_| BackendError::OutOfBounds {
            address,
            length: data.len(),
            limit: 256,
        })?;
        let end = start
            .checked_add(data.len())
            .ok_or(BackendError::OutOfBounds {
                address,
                length: data.len(),
                limit: 256,
            })?;
        if end > self.ram.len() {
            return Err(BackendError::OutOfBounds {
                address,
                length: data.len(),
                limit: 256,
            });
        }
        self.ram[start..end].copy_from_slice(data);
        Ok(())
    }
}

struct PayloadReader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> PayloadReader<'a> {
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
                "synthetic backend payload is truncated".to_owned(),
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

    fn read_u32(&mut self) -> Result<u32, BackendError> {
        Ok(u32::from_le_bytes(self.read_array::<4>()?))
    }

    fn read_u64(&mut self) -> Result<u64, BackendError> {
        Ok(u64::from_le_bytes(self.read_array::<8>()?))
    }
}

#[cfg(test)]
mod tests {
    use chronofork_backend_api::BackendFactory;
    use chronofork_core::InputFrame;

    use super::{DEFAULT_PROGRAM, SyntheticBackendFactory, SyntheticProgram};

    #[test]
    fn identical_machines_converge_on_identical_inputs() {
        let factory = SyntheticBackendFactory::default();
        let mut left = factory.create().expect("left machine");
        let mut right = factory.create().expect("right machine");
        for step in 0..32_u32 {
            let input = InputFrame::new(step % 4);
            let left_outcome = left.step(input).expect("left step");
            let right_outcome = right.step(input).expect("right step");
            assert_eq!(left_outcome, right_outcome);
        }
        assert_eq!(
            left.inspect().expect("left inspect").state_hash,
            right.inspect().expect("right inspect").state_hash
        );
    }

    #[test]
    fn different_inputs_diverge() {
        let factory = SyntheticBackendFactory::default();
        let mut left = factory.create().expect("left machine");
        let mut right = factory.create().expect("right machine");
        // INC_ACC, ADD_IMM, then APPLY_INPUT: inject on the third instruction.
        for _ in 0..2 {
            left.step(InputFrame::NEUTRAL).expect("left neutral step");
            right.step(InputFrame::NEUTRAL).expect("right neutral step");
        }
        left.step(InputFrame::new(1)).expect("left input step");
        right.step(InputFrame::new(2)).expect("right input step");
        assert_ne!(
            left.inspect().expect("left inspect").state_hash,
            right.inspect().expect("right inspect").state_hash
        );
    }

    #[test]
    fn snapshot_restore_round_trips_exact_state() {
        let factory = SyntheticBackendFactory::default();
        let mut machine = factory.create().expect("machine");
        for _ in 0..11 {
            machine.step(InputFrame::new(3)).expect("step");
        }
        let snapshot = machine.snapshot().expect("snapshot");
        let expected = machine.inspect().expect("inspect").state_hash;
        for _ in 0..5 {
            machine
                .step(InputFrame::new(9))
                .expect("step after snapshot");
        }
        machine.restore(&snapshot).expect("restore");
        assert_eq!(machine.inspect().expect("inspect").state_hash, expected);
    }

    #[test]
    fn snapshot_from_other_program_is_rejected() {
        let first = SyntheticBackendFactory::new(
            SyntheticProgram::new(DEFAULT_PROGRAM.to_vec()).expect("program"),
        );
        let second = SyntheticBackendFactory::new(
            SyntheticProgram::new(vec![0x01, 0x06, 0x00, 0x00]).expect("program"),
        );
        let source = first.create().expect("source");
        let snapshot = source.snapshot().expect("snapshot");
        let mut target = second.create().expect("target");
        assert!(target.restore(&snapshot).is_err());
    }

    #[test]
    fn halted_machine_rejects_further_steps() {
        let factory =
            SyntheticBackendFactory::new(SyntheticProgram::new(vec![0xff]).expect("halt program"));
        let mut machine = factory.create().expect("machine");
        let outcome = machine.step(InputFrame::NEUTRAL).expect("halt step");
        assert!(outcome.halted);
        assert!(machine.step(InputFrame::NEUTRAL).is_err());
    }

    #[test]
    fn memory_access_is_bounds_checked() {
        let factory = SyntheticBackendFactory::default();
        let mut machine = factory.create().expect("machine");
        assert!(machine.read_memory(255, 2).is_err());
        assert!(machine.write_memory(255, &[1, 2]).is_err());
        machine.write_memory(255, &[0xaa]).expect("last byte");
        assert_eq!(machine.read_memory(255, 1).expect("read"), vec![0xaa]);
    }

    #[test]
    fn unknown_opcode_is_an_error() {
        let factory =
            SyntheticBackendFactory::new(SyntheticProgram::new(vec![0x42]).expect("program"));
        let mut machine = factory.create().expect("machine");
        assert!(machine.step(InputFrame::NEUTRAL).is_err());
    }
}
