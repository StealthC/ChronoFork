# Third-Party Notices

ChronoFork's own source code is licensed under MIT (see `LICENSE`). This file
records third-party software that is compiled into or distributed with ChronoFork.

## Runtime and build dependencies

The bootstrap depends on the RustCrypto SHA-2 implementation and its transitive
dependencies. All are permissively licensed.

| Package | Version | License | Use |
|---|---:|---|---|
| `sha2` | 0.10.9 | MIT OR Apache-2.0 | SHA-256 state digests and snapshot checksums |
| `digest` | 0.10.7 | MIT OR Apache-2.0 | SHA-2 trait layer |
| `block-buffer` | 0.10.4 | MIT OR Apache-2.0 | Block buffering |
| `crypto-common` | 0.1.7 | MIT OR Apache-2.0 | Common cryptographic traits |
| `generic-array` | 0.14.7 | MIT | Fixed-size arrays |
| `typenum` | 1.20.1 | MIT OR Apache-2.0 | Type-level integers |
| `cpufeatures` | 0.2.17 | MIT OR Apache-2.0 | CPU feature detection |
| `cfg-if` | 1.0.5 | MIT OR Apache-2.0 | Conditional compilation |
| `version_check` | 0.9.5 | MIT/Apache-2.0 | Build-time version checks |
| `libc` | 0.2.191 | MIT OR Apache-2.0 | Platform bindings (transitive build dependency) |

The exact resolved versions are recorded in `Cargo.lock`. License texts are
available from the upstream crates and must be preserved in binary distributions.

## Development-only test tooling

These crates are compiled only for tests and the dependency-free benchmark
example. They are not part of the ChronoFork library, are not linked into the
`chronofork` binary and are not distributed with ChronoFork. Their exact
resolved versions and transitive tree are recorded in `Cargo.lock`; duplicates
are audited with `cargo tree -d`.

| Package | Version | License | Use |
|---|---:|---|---|
| `proptest` | 1.11.0 | MIT OR Apache-2.0 | Adversarial property tests for untrusted byte parsers (dev-dependency) |

`proptest`'s dev-only closure resolves 23 packages including itself and adds 21
entries to `Cargo.lock` (16 -> 37); `cargo tree -d` reports no duplicates. It is
declared with `default-features = false, features = ["std"]` to keep the
fork/timeout/runtime extras out of the test build; 13 new packages are compiled
for the x86_64-linux test target and the remainder are proc-macro/build or
target-specific `getrandom` backends. The size and maintenance cost are accepted
because adversarial decoder tests are required by the anti-slop policy (AS-09),
the crate is permissively licensed and maintained, and it is pinned in
`Cargo.lock` but never enters the normal build or distribution.

## Research references (not distributed)

The following projects are cloned only into the ignored `.research/upstream/`
directory for analysis. Their code is **not** part of ChronoFork and is **not**
distributed in this repository:

- RogerSanders/Exodus — MIT, copyright (c) 2024 Roger Sanders.
- paulomanrique/exodus-for-ais — MIT, copyright (c) 2024 Roger Sanders; port
  changes by the fork authors.
- TASEmulators/BizHawk — heterogeneous licensing; repository-level LICENSE warns
  of incompatible GPL combinations and unclear third-party provenance.

If code is ever incorporated, this file must be updated with the exact source
revision, copyright holder, license text, destination files and modifications.
See `docs/licensing/provenance.md`.

## Test fixtures

No upstream ROMs, firmware or binary test fixtures are bundled. The synthetic
backend's program is authored in this repository (`chronofork-test-backend`) and
is covered by the project license.
