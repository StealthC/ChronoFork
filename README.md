# ChronoFork

**Fork the past. Trace the cause.**

ChronoFork is an open-source, cross-platform emulation and reverse-engineering
platform. It treats emulated execution as a first-class, reproducible, forkable
timeline: sessions own independent machine state, checkpoints are versioned, and
branches can diverge without mutating each other.

> **Project status: bootstrap.** This repository currently ships a deterministic
> **synthetic test backend**, not a Sega Mega Drive / Genesis emulator. It proves
> the session, snapshot, fork, replay and isolation contracts that a real backend
> will plug into. No commercial ROMs or firmware are included.

## What works today

- A Rust 2024 Cargo workspace with five focused crates.
- A capability-based backend contract with explicitly reported guarantees.
- Independent sessions with create / step / pause / resume / reset / close.
- Versioned, checksummed snapshots with round-trip and corruption tests.
- Snapshot restore, replay verification and three-way forking.
- Session lineage, state digests, comparison and memory isolation tests.
- A CLI demonstration that runs the acceptance scenario end to end.
- 26 automated tests, plus `rustfmt`, `cargo check` and Clippy gates.

## Quick start

Requirements: Rust 1.85 or newer (tested with 1.100.0), Git, and a C linker.
No system emulator packages are required for the synthetic backend.

```sh
git clone <repository-url> ChronoFork
cd ChronoFork
cargo test --workspace
cargo run -p chronofork-cli -- demo
cargo run -p chronofork-cli -- backend describe
```

The demo prints the checkpoint, three independent forks, divergent state hashes,
source/memory isolation checks, replay verification, and `chronofork demo: PASS`.

## Workspace

| Crate | Responsibility |
|---|---|
| `chronofork-core` | IDs, virtual ticks, input, SHA-256 state digests |
| `chronofork-backend-api` | Capability, snapshot, inspection and backend traits |
| `chronofork-sessions` | Session lifecycle, checkpoints, lineage, fork/restore |
| `chronofork-test-backend` | Deterministic synthetic machine used for validation |
| `chronofork-cli` | Minimal executable demonstration of implemented features |

The session layer depends on the backend contract, never the reverse. The test
backend is explicitly labelled `test_backend: true` and must never be presented as
hardware emulation.

## Documentation

Start at [docs/README.md](docs/README.md). Key entry points:

- [Product vision](docs/vision/product-vision.md)
- [Architecture overview](docs/architecture/overview.md)
- [Session model](docs/architecture/session-model.md)
- [Backend model](docs/architecture/backend-model.md)
- [Backend strategy proposal](docs/architecture/backend-strategy.md)
- [EPIC-0001](docs/epics/EPIC-0001-foundation.md)
- [Roadmap](docs/roadmap/roadmap.md)
- [Testing strategy](docs/quality/testing-strategy.md)
- [License policy](docs/licensing/policy.md)

## Research references

ChronoFork is informed by, but not a reskin of, existing projects:

- [RogerSanders/Exodus](https://github.com/RogerSanders/Exodus) — MIT-licensed
  debugging-oriented Mega Drive / Genesis emulator.
- [paulomanrique/exodus-for-ais](https://github.com/paulomanrique/exodus-for-ais)
  — MIT-licensed Linux headless port with an MCP surface and a cycle-equivalence
  gate.
- [TASEmulators/BizHawk](https://github.com/TASEmulators/BizHawk) — multi-system
  emulator and TAS frontend with heterogeneous licensing. Its repository-level
  license explicitly warns that the repository as-is mixes incompatible licenses.

These repositories are research inputs only. They are cloned under the ignored
`.research/upstream/` directory and are not submodules or runtime dependencies.
See [provenance](docs/licensing/provenance.md) for exact revisions and the rules
for any future reuse.

## No ROMs or firmware

Do not commit commercial ROMs, BIOS/firmware images, game assets, or copyrighted
test data. Tests in this repository use an independently authored synthetic
program. See [AGENTS.md](AGENTS.md) and `.gitignore`.

## License

ChronoFork's original code is intended to be released under the MIT license. See
[LICENSE](LICENSE) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). The
project license is provisional until the upstream licensing assessment is reviewed;
no upstream source code has been incorporated into this bootstrap.

ChronoFork is not affiliated with or endorsed by Sega, the Exodus authors, the
BizHawk team, or any referenced emulator project.
