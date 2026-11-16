# Backend Model

## Purpose

A backend is an implementation of one emulated machine. It may be a Rust-native
core, a wrapper around an existing C/C++ core, a worker process, or a remote
service adapter. The platform must not assume which.

The contract lives in `crates/chronofork-backend-api/`. It deliberately contains
no session, storage, transport or GUI types.

## Capability guarantees

`Capabilities` reports what a backend actually guarantees:

| Capability | Meaning |
|---|---|
| `execution` | Can advance virtual time |
| `save_states` | Can produce and consume complete snapshots |
| `memory_read` | Side-effect-free debugger memory reads |
| `memory_write` | Debugger memory writes |
| `register_inspection` | CPU/device register inspection |
| `instruction_stepping` | Can advance by a documented quantum |
| `breakpoints` | Conditional code breakpoints (backend capability; host debuggers implement them over `inspect`) |
| `watchpoints` | Memory watchpoints |
| `debug_events` | Bounded typed bus/device/interrupt event observation through `DebugAccess` |
| `tracing` | Structured execution tracing |
| `state_cloning` | Snapshot can seed a fresh independent instance |
| `deterministic_replay` | Identical inputs from identical states converge |
| `rendering` | Produces video frames |
| `audio` | Produces audio samples |

A method existing on the trait is **not** a capability. A backend that cannot
guarantee a feature must report `false`; the host must not expose it as working.
Host-implemented features (for example, a comparison view built from two state
hashes) must be reported separately and must not be disguised as backend
capabilities.

The synthetic backend reports only what it implements and sets
`is_test_backend: true`. The CLI prints that flag and the actual capability values.

## Backend trait

```rust
pub trait Backend: Send {
    fn descriptor(&self) -> &BackendDescriptor;
    fn step(&mut self, input: InputFrame) -> Result<StepOutcome, BackendError>;
    fn snapshot(&self) -> Result<BackendSnapshot, BackendError>;
    fn restore(&mut self, snapshot: &BackendSnapshot) -> Result<(), BackendError>;
    fn inspect(&self) -> Result<MachineInspection, BackendError>;
    fn read_memory(&self, address: u64, length: usize) -> Result<Vec<u8>, BackendError>;
    fn write_memory(&mut self, address: u64, data: &[u8]) -> Result<(), BackendError>;
    fn debug_access(&mut self) -> Option<&mut dyn DebugAccess> { None }
}
```

`debug_access` is optional and defaulted. A backend that can observe its own bus
implements [`DebugAccess`] with a bounded event ring and matching watchpoint specs;
a backend that cannot returns `None` and must not advertise `watchpoints` or
`debug_events`. Debugger observation is non-canonical: it never enters snapshots,
state hashes or configuration hashes, and restore does not rewind it. See
[ADR-0010](../adr/0010-debugging-depth.md).

`BackendFactory` creates fresh, independent instances. A factory must never return
two handles to the same mutable machine.

## Snapshot contract

`BackendSnapshot` carries:

| Field | Purpose |
|---|---|
| `backend_id` | Family identity; prevents cross-backend restore |
| `backend_version` | Exact implementation version |
| `format_version` | Backend payload format version |
| `tick` | Emulated time at capture |
| `config_hash` | Digest of immutable configuration (for example program/ROM identity) |
| `state_hash` | Digest of canonical mutable state |
| `payload` | Opaque backend state |

The session layer adds `SnapshotId`, origin session, outer format version and a
SHA-256 checksum. Restore must validate identity, version, configuration hash and
payload integrity, and must reject rather than partially apply an incompatible
snapshot.

Current bootstrap: `chronofork-sessions` implements the outer envelope and the
synthetic backend implements the payload. The synthetic payload has its own magic
and version and is validated independently.

## Inspection versus bus access

`inspect`, `read_memory` and `write_memory` are debugger-oriented. They must not
trigger emulated-bus side effects that would not occur at the current execution
point. A backend that cannot make this distinction must report the limitation.

Not every internal hardware register is readable or writable through an emulated
bus. The future device-inspection API must expose a separate, typed view rather
than pretending a debugger write is a bus write.

## Real-backend requirements

A production-grade real Mega Drive backend must eventually demonstrate:

- Two independent instances in one host process **or** a documented process
  isolation model.
- Complete snapshot/restore with a versioned format and deterministic state hash.
- Deterministic execution given identical ROM, configuration and input sequence.
- A path to CPU, memory and device inspection without UI dependencies.
- A headless build and test path.
- A license compatible with distribution and a complete provenance record.
- Differential test evidence against a trusted reference.

Features not yet demonstrated must stay `false` in capabilities.

## Current real slice

`chronofork-megadrive` is the first real-hardware slice. It satisfies the backend
contract for a minimal 68000 + ROM/RAM + VDP-port machine with a deterministic
interrupt scheduler:

- 68000 reset vectors, a documented instruction subset, big-endian ROM/RAM bus.
- VDP status/control ports and a 64 KiB VRAM window at `0xC00000..=0xC0000F`, with
  typed errors for PSG/Z80/controller and other unmapped windows.
- An instruction-quantum VBlank/HBlank interrupt scheduler whose pending requests
  are delivered through the 68000 interrupt-entry API only when the SR mask admits
  them (level 7 always; every other level strictly above the interrupt-priority
  mask), VBlank before HBlank, with `RTE` restoring the interrupted raw `SR` and PC.
- Versioned snapshots, restore validation and canonical state hashing that include
  CPU, RAM, VDP and scheduler state.
- Independent instances and session fork/restore/isolation tests.
- Headless build/test path and no UI dependency.
- Explicit errors for unsupported opcodes and unmapped/odd-address accesses.
- MIT-licensed original code; no upstream source incorporated.

It does **not** yet satisfy a production Mega Drive backend: rendering, DMA, audio,
Z80, controllers, mapper/SRAM and cycle accuracy remain absent. `RTE`, SR-mask
interrupt gating and VDP VINT-pending/acknowledge status are implemented for the
modelled slice, with the remaining limits documented in
[Mega Drive 68000 slice](megadrive-slice.md): privilege is not enforced; only
HBlank (level 4) and VBlank (level 6) sources are modelled, with level 7 a predicate
rule rather than a wired source; no interrupt-timing equivalence is claimed; and VDP
interrupt-enable/acknowledge logic is absent. Differential oracle evidence now
exists for three synthetic ROMs (projected address streams, selected final state and
negative controls); see
[Oracle differential evidence](../research/oracle-differential-evidence.md). The VDP
port window and interrupt scheduler are partial slices, not a complete VDP or
hardware-timed interrupt controller. Capabilities report rendering/audio and the
generic `tracing` capability as `false`/absent; the bounded instruction trace is a
concrete-type debug facility, not a generic capability. See
[Mega Drive 68000 slice](megadrive-slice.md).

## Open questions

- How are ROM identity and regional hardware configuration represented in
  `config_hash`?
- Which debugger operations are guaranteed side-effect-free across devices?
- Should the backend contract expose structured tracing directly, or should the
  host derive traces from step/inspection events?
- What is the concurrency model for a future process-isolated backend?
