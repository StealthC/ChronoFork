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
| `breakpoints` | Conditional code breakpoints |
| `watchpoints` | Memory watchpoints |
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
}
```

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

Any future real Mega Drive backend must demonstrate:

- Two independent instances in one host process **or** a documented process
  isolation model.
- Complete snapshot/restore with a versioned format and deterministic state hash.
- Deterministic execution given identical ROM, configuration and input sequence.
- A path to CPU, memory and device inspection without UI dependencies.
- A headless build and test path.
- A license compatible with distribution and a complete provenance record.
- Differential test evidence against a trusted reference.

Features not yet demonstrated must stay `false` in capabilities.

## Open questions

- How are ROM identity and regional hardware configuration represented in
  `config_hash`?
- Which debugger operations are guaranteed side-effect-free across devices?
- Should the backend contract expose structured tracing directly, or should the
  host derive traces from step/inspection events?
- What is the concurrency model for a future process-isolated backend?
