# Provenance

This document records the origin of code and research inputs. It must be updated
before any upstream code is incorporated.

## ChronoFork original code

| Field | Value |
|---|---|
| Project | ChronoFork |
| Original authors | ChronoFork contributors |
| License | MIT (provisional; see `LICENSE`) |
| Bootstrap date | 2026-11-13 |
| Incorporated upstream code | **None** |
| Test fixture | Independently authored synthetic bytecode in `chronofork-test-backend` |

The implementation was written for this project from the architecture requirements.
It does not copy source from Exodus, exodus-for-ais or BizHawk.

## Research-only references

These repositories are cloned under `.research/upstream/` for analysis. They are
ignored by Git, are not submodules, and are not distributed with ChronoFork.

| Project | URL | Branch | Commit | Commit date | License | Status |
|---|---|---|---|---|---|---|
| Exodus | https://github.com/RogerSanders/Exodus | `master` | `08f388f77040af28d16d44fdfbddb73252953161` | 2024-02-16 | MIT | Research only |
| exodus-for-ais | https://github.com/paulomanrique/exodus-for-ais | `linux-headless` | `ec0056151f3ff1c334d09ee375abb30a68fdfec5` | 2026-08-13 | MIT (root) | Research only |
| BizHawk | https://github.com/TASEmulators/BizHawk | `master` | `d148601a4629c2c230e41647d7c94eee794011c7` | 2026-11-10 | Heterogeneous; root LICENSE warns of incompatible GPL combinations | Research only |

### Exodus

- Copyright: (c) 2024 Roger Sanders, per `License.txt`.
- Evidence inspected: `License.txt`, `Exodus.sln`, `Devices/`, `System/`,
  `ExodusSDK/`, `Tests/` solution and `_Documentation/`.
- No files copied.

### exodus-for-ais

- Copyright: (c) 2024 Roger Sanders in the inherited `License.txt`; fork changes by
  the fork authors. The root license does not separately enumerate fork authorship.
- Evidence inspected: `README.md`, `AGENTS.md`, `CMakeLists.txt`, `License.txt`,
  `mcp/`, `host/`, `tests/equivalence/`, `tests/rom/`.
- No files copied.
- Open question: whether every added file is covered by the root MIT text; confirm
  before reuse.

### BizHawk

- Root `LICENSE` states original C# work is MIT, but also states the repository
  contains submodules with other licenses, submodules without clearly provided
  licenses, and binaries without clearly indicated origin. It explicitly calls the
  repository as-is an "illegal combination of several incompatible GPL licenses."
- Evidence inspected: `LICENSE`, `.gitmodules`, `README.md`, `src/`,
  `ExternalProjects/`, `ExternalCoreProjects/`.
- Submodules were **not** cloned (`--no-recurse-submodules`), so this audit does not
  cover their contents.
- No files copied. Do not incorporate BizHawk code without a component-level legal
  audit and explicit project-owner approval.

## Dependency provenance

| Component | Version | License | Use |
|---|---:|---|---|
| `sha2` | 0.10.9 | MIT OR Apache-2.0 | SHA-256 |
| `digest` | 0.10.7 | MIT OR Apache-2.0 | SHA-2 traits |
| `block-buffer` | 0.10.4 | MIT OR Apache-2.0 | Block buffering |
| `crypto-common` | 0.1.7 | MIT OR Apache-2.0 | Crypto traits |
| `generic-array` | 0.14.7 | MIT | Fixed arrays |
| `typenum` | 1.20.1 | MIT OR Apache-2.0 | Type-level integers |
| `cpufeatures` | 0.2.17 | MIT OR Apache-2.0 | CPU features |
| `cfg-if` | 1.0.5 | MIT OR Apache-2.0 | Conditional compilation |
| `version_check` | 0.9.5 | MIT/Apache-2.0 | Build checks |
| `libc` | 0.2.191 | MIT OR Apache-2.0 | Transitive build dependency |

Exact resolved versions are in `Cargo.lock`. See `THIRD_PARTY_NOTICES.md`.

## Incorporation template

Before merging any upstream code, record:

```text
Origin project:
Repository URL:
Exact revision:
Original file(s):
Copyright holder:
License:
Required notices:
Destination file(s):
Reuse type: verbatim | adapted | ported | independently implemented
Description of modifications:
Verification tests:
Reviewer/date:
```

## Open questions

- Does the exodus-for-ais fork have a complete per-file copyright inventory?
- Are the Exodus SDK third-party directories under MIT or separate licenses?
- Which BizHawk components (if any) could be safely studied for architecture
  without copying expression? (Research only; no reuse approved.)
- What is the final project license after the assessment is reviewed?
