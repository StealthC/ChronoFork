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
    BackendId, BackendSnapshot, BatchSignals, Capabilities, MachineInspection, MachineStatus,
    StepOutcome, validate_envelope,
};
use chronofork_core::codec::{CodecError, Reader, Writer};
use chronofork_core::{InputFrame, StateHash, Tick, hash_bytes};

/// Identifier of this synthetic backend.
pub const BACKEND_ID: &str = "chronofork.test.synthetic";

/// Version of the synthetic backend payload format.
pub const PAYLOAD_FORMAT_VERSION: u16 = 1;

/// Size of the synthetic machine's RAM, in bytes (fixed by the payload format).
pub const RAM_BYTES: usize = 256;

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
    ram: [u8; RAM_BYTES],
    tick: Tick,
    steps: u64,
    halted: bool,
}

impl SyntheticBackend {
    /// Serializes the canonical mutable state exactly once.
    ///
    /// Both [`Self::state_hash`] and the snapshot payload derive from these
    /// bytes, so adding a state field requires one edit and cannot silently
    /// escape the state hash.
    fn state_body(&self) -> Vec<u8> {
        let mut body = Writer::with_capacity(2 + 4 + 8 + 8 + 1 + self.ram.len());
        body.write_u16_le(self.pc);
        body.write_u32_le(self.acc);
        body.write_u64_le(self.tick.raw());
        body.write_u64_le(self.steps);
        body.write_u8(u8::from(self.halted));
        body.write_bytes(&self.ram);
        body.into_bytes()
    }

    /// The one register-map writer for status and inspection.
    fn register_maps(&self) -> (BTreeMap<String, u64>, BTreeMap<String, u64>) {
        let mut cpu = BTreeMap::new();
        cpu.insert("pc".to_owned(), u64::from(self.pc));
        cpu.insert("acc".to_owned(), u64::from(self.acc));
        let mut device = BTreeMap::new();
        device.insert("tick".to_owned(), self.tick.raw());
        device.insert("steps".to_owned(), self.steps);
        device.insert("halted".to_owned(), u64::from(self.halted));
        (cpu, device)
    }

    fn state_hash(&self) -> StateHash {
        hash_state_body(&self.state_body())
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

    /// Applies a schema-2 input frame to the accumulator and RAM.
    ///
    /// Port 0 is the v1 effect and always runs: its low byte is added to the
    /// accumulator and the low byte of the result is stored at `ram[0x10]`.
    /// Non-neutral ports 1 to 3 then add their low byte in ascending order and
    /// store the low byte of the result at `ram[0x10 + port]`. Non-neutral axes
    /// add their two's-complement 16-bit value in ascending order and store the
    /// low byte of the result at `ram[0x14 + axis]`. A non-neutral keyboard
    /// bitmap adds its eight bytes least-significant first and stores each byte
    /// at `ram[0x18 + index]`.
    ///
    /// A neutral extra field neither adds to the accumulator nor writes RAM, so
    /// a v1 frame executes exactly the pre-schema-2 path and keeps its state
    /// hash. The work is constant and ordered, with no host time or randomness.
    fn apply_input(&mut self, input: InputFrame) {
        let ports = input.ports();
        self.acc = self.acc.wrapping_add(u32::from((ports[0] & 0xff) as u8));
        self.ram[0x10] = (self.acc & 0xff) as u8;
        for (index, port) in ports.iter().enumerate().skip(1) {
            if *port != 0 {
                self.acc = self.acc.wrapping_add(u32::from((*port & 0xff) as u8));
                self.ram[0x10 + index] = (self.acc & 0xff) as u8;
            }
        }
        for (index, axis) in input.axes().iter().enumerate() {
            if *axis != 0 {
                self.acc = self.acc.wrapping_add(u32::from(*axis as u16));
                self.ram[0x14 + index] = (self.acc & 0xff) as u8;
            }
        }
        if input.keys() != 0 {
            for (index, byte) in input.keys().to_le_bytes().iter().enumerate() {
                self.acc = self.acc.wrapping_add(u32::from(*byte));
                self.ram[0x18 + index] = *byte;
            }
        }
    }

    fn apply_snapshot_payload(
        &mut self,
        payload: &[u8],
        expected_hash: StateHash,
    ) -> Result<(), BackendError> {
        let mut reader = Reader::new(payload);
        let magic = reader.read_array::<4>().map_err(payload_error)?;
        if magic != *b"CFTB" {
            return Err(BackendError::InvalidSnapshot(
                "synthetic backend payload magic mismatch".to_owned(),
            ));
        }
        let version = reader.read_u16_le().map_err(payload_error)?;
        if version != PAYLOAD_FORMAT_VERSION {
            return Err(BackendError::InvalidSnapshot(format!(
                "synthetic backend payload version {version}, expected {PAYLOAD_FORMAT_VERSION}"
            )));
        }
        let pc = reader.read_u16_le().map_err(payload_error)?;
        let acc = reader.read_u32_le().map_err(payload_error)?;
        let tick = Tick::from_raw(reader.read_u64_le().map_err(payload_error)?);
        let steps = reader.read_u64_le().map_err(payload_error)?;
        let halted = reader.read_u8().map_err(payload_error)? != 0;
        let ram = reader.read_array::<256>().map_err(payload_error)?;
        reader.finish().map_err(payload_error)?;
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

/// Magic prefix of the synthetic state-hash input.
const STATE_MAGIC: [u8; 10] = *b"CFTBSTATE1";
/// Magic prefix of the synthetic snapshot payload.
const PAYLOAD_MAGIC: [u8; 4] = *b"CFTB";

/// Prefixes a canonical state body with the payload magic and format version.
fn payload_from_body(body: &[u8]) -> Vec<u8> {
    let mut payload = Writer::with_capacity(PAYLOAD_MAGIC.len() + 2 + body.len());
    payload.write_bytes(&PAYLOAD_MAGIC);
    payload.write_u16_le(PAYLOAD_FORMAT_VERSION);
    payload.write_bytes(body);
    payload.into_bytes()
}

/// Prefixes a canonical state body with the state-hash magic.
fn hash_state_body(body: &[u8]) -> StateHash {
    let mut state = Writer::with_capacity(STATE_MAGIC.len() + body.len());
    state.write_bytes(&STATE_MAGIC);
    state.write_bytes(body);
    hash_bytes(&state.into_bytes())
}

impl Backend for SyntheticBackend {
    fn descriptor(&self) -> &BackendDescriptor {
        &self.descriptor
    }

    fn state_hash(&self) -> Result<StateHash, BackendError> {
        // The private inherent method is the single canonical digest: it hashes the
        // one `state_body` that also feeds the snapshot payload. This impl exists only
        // to expose it through the trait (inherent method resolution picks it here).
        Ok(SyntheticBackend::state_hash(self))
    }

    fn inspection_memory_window(&self) -> (u64, usize) {
        (0, RAM_BYTES)
    }

    fn batch_signals(&self) -> BatchSignals {
        // The synthetic machine has a program counter but no frame timeline.
        BatchSignals {
            tick: Some(self.tick),
            pc: Some(u32::from(self.pc)),
            frame: None,
            halted: self.halted,
        }
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
            0x09 => self.apply_input(input),
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
            // W2-3 target shape: ordinary execution carries no digest at all. The
            // verified path computes the canonical digest once through `state_hash()`.
            halted: self.halted,
            verified: false,
        })
    }

    fn snapshot(&self) -> Result<BackendSnapshot, BackendError> {
        // One canonical body feeds both the payload and the state hash, so a
        // snapshot costs one full-state serialization.
        let body = self.state_body();
        Ok(BackendSnapshot::new(
            self.descriptor.id().clone(),
            self.descriptor.version(),
            BACKEND_SNAPSHOT_FORMAT_VERSION,
            self.tick,
            self.program.hash(),
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
            self.program.hash(),
        )?;
        self.apply_snapshot_payload(snapshot.payload(), snapshot.state_hash())
    }

    fn status(&self) -> Result<MachineStatus, BackendError> {
        let (cpu_registers, device_registers) = self.register_maps();
        Ok(MachineStatus {
            tick: self.tick,
            halted: self.halted,
            cpu_registers,
            device_registers,
            frame_index: 0,
        })
    }

    fn inspect(&self) -> Result<MachineInspection, BackendError> {
        let (cpu_registers, device_registers) = self.register_maps();
        Ok(MachineInspection {
            tick: self.tick,
            halted: self.halted,
            frame_index: 0,
            cpu_registers,
            device_registers,
            memory: None,
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

    /// The synthetic payload size is fixed by its format, so it is an exact bound.
    ///
    /// `payload_from_body` prefixes the state body with the payload magic and the format
    /// version, and the body is the fixed-width register block plus the 256-byte RAM, so
    /// every snapshot this backend produces has exactly
    /// [`SYNTHETIC_PAYLOAD_BYTES`] payload bytes.
    fn snapshot_payload_bound(&self) -> Option<usize> {
        Some(SYNTHETIC_PAYLOAD_BYTES)
    }
}

/// Exact payload size of every synthetic snapshot (see [`SyntheticBackend`]).
pub const SYNTHETIC_PAYLOAD_BYTES: usize = PAYLOAD_MAGIC.len() + 2 + 2 + 4 + 8 + 8 + 1 + RAM_BYTES;

/// Translates a shared codec failure into the backend snapshot error contract.
fn payload_error(error: CodecError) -> BackendError {
    match error {
        CodecError::Truncated { .. } => {
            BackendError::InvalidSnapshot("synthetic backend payload is truncated".to_owned())
        }
        CodecError::LengthOverflow => {
            BackendError::InvalidSnapshot("payload length arithmetic overflow".to_owned())
        }
        CodecError::TrailingBytes { count } => BackendError::InvalidSnapshot(format!(
            "synthetic backend payload has {count} trailing bytes"
        )),
        other => {
            BackendError::InvalidSnapshot(format!("invalid synthetic backend payload: {other}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use chronofork_backend_api::{BackendFactory, BackendSnapshot};
    use chronofork_core::{InputFrame, Tick, hash_bytes};

    use super::{
        DEFAULT_PROGRAM, PAYLOAD_FORMAT_VERSION, SyntheticBackendFactory, SyntheticProgram,
    };

    /// Program that executes one `APPLY_INPUT` and then halts.
    const INPUT_ONLY_PROGRAM: &[u8] = &[0x09, 0xff];

    fn input_only_factory() -> SyntheticBackendFactory {
        SyntheticBackendFactory::new(
            SyntheticProgram::new(INPUT_ONLY_PROGRAM.to_vec()).expect("input program"),
        )
    }

    #[test]
    fn synthetic_backend_reports_no_debug_observation() {
        // This backend has no bus to observe, so it must keep the defaulted
        // `Backend::debug_access` and advertise neither watchpoints nor debug
        // events (ADR-0010).
        let factory = SyntheticBackendFactory::default();
        let capabilities = factory.capabilities();
        assert!(!capabilities.watchpoints);
        assert!(!capabilities.debug_events);
        assert!(!capabilities.breakpoints);
        let mut machine = factory.create().expect("machine");
        assert!(
            machine.debug_access().is_none(),
            "the synthetic backend must not expose a debug observation surface"
        );
    }

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
            left.state_hash().expect("left inspect digest"),
            right.state_hash().expect("right inspect digest")
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
            left.state_hash().expect("left inspect digest"),
            right.state_hash().expect("right inspect digest")
        );
    }

    #[test]
    fn apply_input_ignores_buttons_above_the_low_byte() {
        // `APPLY_INPUT` adds the low eight bits of the button mask only, so the
        // extraction is total and identical for every high-bit pattern.
        let program = SyntheticProgram::new(vec![0x09, 0xff]).expect("program");
        let factory = SyntheticBackendFactory::new(program);
        let mut low = factory.create().expect("low machine");
        let mut high = factory.create().expect("high machine");
        let low_outcome = low.step(InputFrame::new(0x02)).expect("low input");
        let high_outcome = high.step(InputFrame::new(0x1_0002)).expect("high input");
        assert_eq!(low_outcome, high_outcome);
        assert_eq!(
            low.state_hash().expect("inspect digest"),
            high.state_hash().expect("inspect digest")
        );
    }

    #[test]
    fn v1_frames_keep_the_pre_schema2_state_hash() {
        // Golden value computed independently from the documented canonical
        // state body before the schema-2 extension: pc=1, acc=2, tick=1,
        // steps=1, halted=0, ram[0x10]=2, all other RAM bytes zero.
        const GOLDEN: &str = "9daa3442b67f7770adf027fca2cc26b14182c985aa47db9b67b54ceb836791f3";
        let factory = input_only_factory();
        let mut v1 = factory.create().expect("v1 machine");
        let mut schema2 = factory.create().expect("schema2 machine");
        let v1_outcome = v1.step(InputFrame::new(2)).expect("v1 step");
        let schema2_outcome = schema2
            .step(InputFrame::with_devices([2, 0, 0, 0], [0; 4], 0))
            .expect("schema2 step");
        assert_eq!(v1_outcome, schema2_outcome);
        // W2-2: the ordinary outcome carries no digest, so the golden asks the
        // canonical source explicitly; the value must not change.
        assert_eq!(v1.state_hash().expect("canonical digest").to_hex(), GOLDEN);
        assert_eq!(
            v1.snapshot().expect("v1 snapshot").payload(),
            schema2.snapshot().expect("schema2 snapshot").payload()
        );
    }

    #[test]
    fn v1_mask_equivalence_holds_for_every_low_byte() {
        let factory = input_only_factory();
        for mask in 0..=u8::MAX {
            let mut v1 = factory.create().expect("v1 machine");
            let mut schema2 = factory.create().expect("schema2 machine");
            let v1_outcome = v1.step(InputFrame::new(u32::from(mask))).expect("v1 step");
            let schema2_outcome = schema2
                .step(InputFrame::with_devices(
                    [u32::from(mask), 0, 0, 0],
                    [0; 4],
                    0,
                ))
                .expect("schema2 step");
            assert_eq!(v1_outcome, schema2_outcome, "mask {mask:#04x}");
            assert_eq!(
                v1.snapshot().expect("v1 snapshot").payload(),
                schema2.snapshot().expect("schema2 snapshot").payload(),
                "mask {mask:#04x}"
            );
        }
    }

    #[test]
    fn schema2_ports_are_consumed_in_ascending_order() {
        let factory = input_only_factory();
        let mut machine = factory.create().expect("machine");
        let input = InputFrame::with_devices([0x12, 0x34, 0x56, 0x78], [0; 4], 0);
        machine.step(input).expect("step");
        assert_eq!(
            machine.read_memory(0x10, 0x10).expect("memory"),
            vec![0x12, 0x46, 0x9c, 0x14, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,]
        );
        assert_eq!(
            machine.inspect().expect("inspect").registers()["acc"],
            0x114
        );
    }

    #[test]
    fn schema2_axes_are_consumed_as_twos_complement_values() {
        let factory = input_only_factory();
        let mut machine = factory.create().expect("machine");
        let input = InputFrame::with_devices([0; 4], [1, -1, i16::MIN, i16::MAX], 0);
        machine.step(input).expect("step");
        assert_eq!(
            machine.read_memory(0x14, 4).expect("axes"),
            vec![1, 0, 0, 0xff]
        );
        assert_eq!(
            machine.inspect().expect("inspect").registers()["acc"],
            131_071
        );
    }

    #[test]
    fn schema2_keyboard_bytes_are_consumed_least_significant_first() {
        let factory = input_only_factory();
        let mut machine = factory.create().expect("machine");
        let keys = 0x0807_0605_0403_0201_u64;
        machine
            .step(InputFrame::with_devices([0; 4], [0; 4], keys))
            .expect("step");
        assert_eq!(
            machine.read_memory(0x18, 8).expect("keys"),
            vec![1, 2, 3, 4, 5, 6, 7, 8]
        );
        assert_eq!(machine.inspect().expect("inspect").registers()["acc"], 36);
    }

    #[test]
    fn neutral_device_fields_have_no_state_effect() {
        let factory = SyntheticBackendFactory::default();
        let mut v1 = factory.create().expect("v1 machine");
        let mut neutral_extras = factory.create().expect("neutral machine");
        for mask in [0, 1, 3, 0x80] {
            let v1_outcome = v1.step(InputFrame::new(mask)).expect("v1 step");
            let neutral_outcome = neutral_extras
                .step(InputFrame::with_devices([mask, 0, 0, 0], [0; 4], 0))
                .expect("neutral step");
            assert_eq!(v1_outcome, neutral_outcome);
        }
        assert_eq!(
            v1.snapshot().expect("v1 snapshot").payload(),
            neutral_extras
                .snapshot()
                .expect("neutral snapshot")
                .payload()
        );
    }

    #[test]
    fn multi_device_sequences_replay_deterministically() {
        let factory = SyntheticBackendFactory::default();
        let mut left = factory.create().expect("left machine");
        let mut right = factory.create().expect("right machine");
        let frames = [
            InputFrame::with_devices([1, 2, 3, 4], [1, -1, 2, -2], 0x0102_0304_0506_0708),
            InputFrame::with_devices([0xff, 0, 0, 0x8000_0000], [i16::MIN, 0, 0, i16::MAX], 1),
        ];
        for _ in 0..8 {
            for frame in frames {
                let left_outcome = left.step(frame).expect("left step");
                let right_outcome = right.step(frame).expect("right step");
                assert_eq!(left_outcome, right_outcome);
                assert_eq!(
                    left.snapshot().expect("left snapshot").payload(),
                    right.snapshot().expect("right snapshot").payload()
                );
            }
        }
    }

    #[test]
    fn schema2_consumption_keeps_the_canonical_payload_layout() {
        // The state body stays exactly pc, acc, tick, steps, halted and ram.
        const BODY_LEN: usize = 2 + 4 + 8 + 8 + 1 + 256;
        let factory = SyntheticBackendFactory::default();
        assert_eq!(PAYLOAD_FORMAT_VERSION, 1);
        let mut v1 = factory.create().expect("v1 machine");
        let mut multi = factory.create().expect("multi machine");
        v1.step(InputFrame::new(2)).expect("v1 step");
        multi
            .step(InputFrame::with_devices([2, 3, 4, 5], [6, 7, 8, 9], 0x0a0b))
            .expect("multi step");
        for machine in [&v1, &multi] {
            let snapshot = machine.snapshot().expect("snapshot");
            let payload = snapshot.payload();
            assert_eq!(payload.len(), super::PAYLOAD_MAGIC.len() + 2 + BODY_LEN);
            assert_eq!(
                &payload[..super::PAYLOAD_MAGIC.len()],
                &super::PAYLOAD_MAGIC
            );
            let version = &payload[super::PAYLOAD_MAGIC.len()..super::PAYLOAD_MAGIC.len() + 2];
            assert_eq!(
                u16::from_le_bytes([version[0], version[1]]),
                PAYLOAD_FORMAT_VERSION
            );
            assert_eq!(snapshot.config_hash(), hash_bytes(DEFAULT_PROGRAM));
        }
    }

    #[test]
    fn snapshot_restore_round_trips_exact_state() {
        let factory = SyntheticBackendFactory::default();
        let mut machine = factory.create().expect("machine");
        for _ in 0..11 {
            machine.step(InputFrame::new(3)).expect("step");
        }
        let snapshot = machine.snapshot().expect("snapshot");
        let expected = machine.state_hash().expect("inspect digest");
        for _ in 0..5 {
            machine
                .step(InputFrame::new(9))
                .expect("step after snapshot");
        }
        machine.restore(&snapshot).expect("restore");
        assert_eq!(machine.state_hash().expect("inspect digest"), expected);
    }

    #[test]
    fn state_hash_is_derived_from_the_payload_state_body() {
        let factory = SyntheticBackendFactory::default();
        let mut machine = factory.create().expect("machine");
        for _ in 0..11 {
            machine.step(InputFrame::new(3)).expect("step");
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
            PAYLOAD_FORMAT_VERSION
        );
        let body = &payload[version_offset + 2..];
        let mut hash_input = Vec::with_capacity(super::STATE_MAGIC.len() + body.len());
        hash_input.extend_from_slice(&super::STATE_MAGIC);
        hash_input.extend_from_slice(body);
        assert_eq!(
            chronofork_core::hash_bytes(&hash_input),
            snapshot.state_hash(),
            "state hash must be hash(STATE_MAGIC + payload state body)"
        );
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

    #[test]
    fn arbitrary_payloads_are_rejected_without_panicking() {
        let factory = SyntheticBackendFactory::default();
        let mut machine = factory.create().expect("machine");
        let base = machine.snapshot().expect("snapshot");
        let mut state = 0x0123_4567_89ab_cdef_u64;
        for length in 0..400 {
            let mut payload = Vec::with_capacity(length);
            for _ in 0..length {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                payload.push((state >> 24).to_le_bytes()[0]);
            }
            let snapshot = BackendSnapshot::new(
                base.backend_id().clone(),
                base.backend_version(),
                base.format_version(),
                base.tick(),
                base.config_hash(),
                base.state_hash(),
                payload,
            );
            assert!(
                machine.restore(&snapshot).is_err(),
                "arbitrary {length}-byte payload must be rejected"
            );
        }
        assert_eq!(machine.inspect().expect("inspect").tick, Tick::ZERO);
    }
}
