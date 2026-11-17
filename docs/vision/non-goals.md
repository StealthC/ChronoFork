# Non-Goals

These are deliberate boundaries, not missing features that will be silently added.

## Product non-goals

- **Not a ROM or firmware distribution site.** ChronoFork will not host or bundle
  commercial games, BIOS images or unlicensed test data.
- **Not a reskin of another emulator.** Upstream projects may inform architecture,
  but ChronoFork must have its own identity, contracts and documentation. Origin of
  reused code must always be disclosed.
- **Not a GUI-first emulator.** A GUI exists as an optional presentation layer, but
  emulation and sessions must work headless and must never depend on a display owning
  canonical state.
- **Not a single-backend product.** The host must not reduce every backend to the
  least capable one, nor expose unsupported capabilities.
- **Not an AI-only tool.** Agent-native interfaces are a feature, not the core
  architecture. The repository must remain usable by humans and other agents.
- **Not a cloud service in the bootstrap.** No remote execution, account system,
  telemetry or network dependency is required to run the platform.

## Engineering non-goals for the bootstrap

- Implementing a complete 68000, Z80, VDP, YM2612 or PSG.
- Claiming cycle accuracy or game compatibility without differential tests.
- Optimizing snapshot size or fork speed before correctness is established.
- Introducing a database, message broker, HTTP server or plugin runtime before a
  concrete use case and approval exist.
- Treating `unsafe` as a shortcut. Unsafe code is forbidden by workspace lint.
- Copying an upstream test suite wholesale without verifying redistribution rights.
- Hiding uncertainty in documentation. Unresolved items are recorded as open
  questions.

## Licensing non-goals

- Incorporating BizHawk code whose provenance or license is unclear.
- Assuming translation to Rust removes copyright obligations.
- Assuming process isolation resolves license incompatibility.
- Using attribution as a substitute for satisfying a license.
- Changing the project license silently.
