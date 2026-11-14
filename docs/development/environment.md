# Development Environment

Verified on 2026-11-13. Update this document when the environment changes.

## Host and WSL

| Item | Value |
|---|---|
| Host OS | Windows (WSL2) |
| Distro | Ubuntu 24.04.4 LTS (Noble) |
| Kernel | `6.18.58.1-microsoft-standard-WSL2` |
| Architecture | `x86_64` |
| WSL workspace | `/home/stealthc/projects/ChronoFork` (native ext4, `/dev/sdd`) |
| CPU | 8 logical CPUs |
| RAM | 30 GiB visible, 8 GiB swap |
| Disk | ~814 GiB free at bootstrap |
| Network | GitHub reachable (HTTP/2 200) |

The workspace is on the native Linux filesystem, not `/mnt/c`. This avoids
cross-OS permission and performance surprises. Do not move the checkout to a
Windows-mounted path without testing build and file semantics.

## Toolchain

| Tool | Version | Status |
|---|---|---|
| Rust | 1.100.0 (2026-11-09) | Installed user-local via rustup |
| Cargo | 1.100.0 | Installed |
| rustfmt | 1.11.0-stable | Installed |
| clippy | 0.1.100 | Installed |
| Git | 2.43.0 | Present |
| CMake | 3.28.3 | Present |
| GCC | 13.3.0 | Present |
| G++ | 13.3.0 | Present |
| make | Present | Present |
| Python | 3.12.3 | Present |
| `pkg-config` | Present | Present |
| jq | Present | Present |
| Ninja | Missing | Required by exodus-headless Linux build |
| clang/clang++ | Missing | Required by exodus-headless Linux build |
| Node.js | Missing | Not required for the Rust workspace |

Rust was installed with:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup-init.sh
sh /tmp/rustup-init.sh -y --profile minimal -c rustfmt -c clippy
. "$HOME/.cargo/env"
```

No `sudo`, OS packages or global Harness configuration were changed. The user
approved the user-local rustup installation.

## What changed

- Initialized a Git repository on `main`; no remote configured.
- Added `.gitignore` before any clone.
- Added `.editorconfig`, `Cargo.toml`, `rust-toolchain.toml`, `rustfmt.toml`.
- Installed Rust stable user-local in `~/.cargo` and `~/.rustup`.
- Cloned three reference repositories under ignored `.research/upstream/`.
- Created the ignored `.planning/` working memory.

## Reproducibility

- `rust-toolchain.toml` pins the stable channel with rustfmt and clippy.
- `Cargo.lock` should be committed for this binary/workspace project.
- The synthetic backend has no external runtime dependencies.
- `sha2` is the only direct third-party crate; its exact version is locked.
- No environment variables are required for tests.
- No network access is required after dependencies are fetched.

## Limitations affecting reproducibility

1. **Native Windows was not tested.** Passing under WSL does not prove native
   Windows compatibility.
2. **The headless Exodus port was not built.** Ninja, clang and a real ROM are
   missing; its equivalence claim was not reproduced.
3. **No CI provider is configured.** The local gate is authoritative for this
   bootstrap; provider-specific integration needs approval.
4. **No GPU/audio device was exercised.** The bootstrap is headless and synthetic.
5. **BizHawk submodules were not cloned**, so their licenses/code were not audited.

## Approval-required environment work

The following are known future needs but were **not** performed:

- Installing Ninja and clang (system packages) to build exodus-headless.
- Installing `libexpat`, `zlib`, `libpng`, `libjpeg` development packages.
- Installing Node.js for future web/transport tooling.
- Enabling a CI provider or creating a remote repository.

Request approval with rationale before making any of these changes.
