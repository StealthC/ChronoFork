# Upstream Repository Inventory

## Method

Repositories were cloned on 2026-11-13 into the ignored `.research/upstream/`
directory using shallow, single-branch clones and **no submodules**:

```sh
git clone --depth 1 --single-branch --branch master \
  https://github.com/RogerSanders/Exodus .research/upstream/exodus
git clone --depth 1 --single-branch --branch linux-headless \
  https://github.com/paulomanrique/exodus-for-ais .research/upstream/exodus-headless
git clone --depth 1 --single-branch --no-recurse-submodules --branch master \
  https://github.com/TASEmulators/BizHawk .research/upstream/bizhawk
```

Exact revisions:

| Repository | Branch | Commit |
|---|---|---|
| Exodus | `master` | `08f388f77040af28d16d44fdfbddb73252953161` |
| exodus-for-ais | `linux-headless` | `ec0056151f3ff1c334d09ee375abb30a68fdfec5` |
| BizHawk | `master` | `d148601a4629c2c230e41647d7c94eee794011c7` |

They are research inputs only, not dependencies, submodules or tracked content.

## Exodus

- URL: https://github.com/RogerSanders/Exodus
- License: MIT, copyright (c) 2024 Roger Sanders (`License.txt`).
- Size: ~18 MiB shallow clone.
- Build: Visual Studio solutions (`Exodus.sln`, `ExodusTests.sln`,
  `ThirdPartyLibraries.sln`), property sheets under `Build/`.
- Main areas: `Devices/` (68000, Z80, VDP, YM2612, PSG, memory, MD I/O),
  `System/`, `Exodus/`, `ExodusSDK/`, `Support Libraries/`, `Third Party/`,
  `_Documentation/`.
- Test areas: `MarshalSupport/Tests`, `Tests` property sheets, documentation
  interfaces. The repository is oriented to interactive Windows debugging.
- State: device-level `SaveState`/`LoadState` APIs exist (for example
  `Devices/YM2612/YM2612.h`, `Devices/MD1600IO/*`, and
  `ExodusSDK/DeviceInterface/_Documentation/IDevice/Methods.SaveState.xml`).
- Initial assessment: strong architectural reference for device abstraction,
  state serialization and debugging. Not directly buildable on Linux in its
  original form.

## exodus-for-ais (Linux headless port)

- URL: https://github.com/paulomanrique/exodus-for-ais
- Branch: `linux-headless`
- License: root `License.txt` is MIT, copyright (c) 2024 Roger Sanders. Fork
  additions are not separately enumerated; confirm coverage before reuse.
- Size: ~19 MiB shallow clone.
- Build: CMake (`CMakeLists.txt`), with explicit Linux requirements:
  - Ninja generator (`FATAL_ERROR` if not Ninja);
  - clang/clang++ (`FATAL_ERROR` if not Clang);
  - pkg-config modules `expat`, `zlib`, `libpng`, `libjpeg`.
- Headless host: `host/HeadlessHost.*`, `host/ExodusProbe.cpp`,
  `host/Static*Registry.*`.
- MCP server: `mcp/Main.cpp`, `mcp/Session.*`, `mcp/Server.*`, `mcp/Tools*.cpp`,
  JSON/transport/handle-table code. The README advertises 35 tools across
  session/run/cpu/device/breakpoint/watchpoint/trace/disasm families.
- Session model: `AGENTS.md` states: "The server holds one emulated system at a
  time." `Session` has load module/ROM, reset, step frames/device, start/stop,
  device index, breakpoints/watchpoints and trace/event log. No save/load-state
  MCP tool was found.
- Tests:
  - `tests/equivalence/compare_traces.py` and README documenting a trace-diff
    gate.
  - `tests/rom/make_test_rom.py`, `rom_test.py` (synthetic ROM generation).
  - `mcp/tests/session_test.py` (real MCP subprocess; requires a ROM).
  - `mcp/tests/JsonTest.cpp`, `TextCodecTest.cpp`.
  - `host/tests/PathSemanticsTest.cpp`, `tests/hierarchicalstorage/RoundTripTest.cpp`.
- Equivalence claim: README says an MSVC Windows trace and a clang Linux trace were
  byte-identical over 18,941 68000 instructions, with SHA-256
  `a89a94669a3d5cd42662c516870626c9ead67a7b92335ac50ee208b7afd3821d`, and that
  corrupting one record is detected.
  **This bootstrap did not reproduce the claim.** The pinned headless port was later
  built for E2-12 and exercised with independently authored synthetic ROMs (see
  [Oracle differential evidence](oracle-differential-evidence.md)), but the upstream
  18,941-record equivalence gate itself was not rerun and no real ROM is present.
  Treat the claim as an upstream claim with source evidence, not as ChronoFork test
  evidence.
- Initial assessment: valuable real-hardware oracle and debugging surface; not
  directly suitable as ChronoFork's canonical backend because of single-system MCP
  scope and missing state transfer.

## BizHawk

- URL: https://github.com/TASEmulators/BizHawk
- License: heterogeneous. Root `LICENSE` states that original C# work is MIT, but
  that the repository includes submodules with other licenses, submodules without
  clear licenses, and binaries of unclear origin, and explicitly calls the
  repository as-is an "illegal combination of several incompatible GPL licenses."
- Size: ~397 MiB shallow clone **without submodules**.
- Build: .NET solution (`BizHawk.sln`), `Directory.Packages.props`, `Common.props`,
  Nix (`default.nix`), CI workflows in `.github/` and `.gitlab-ci.yml`.
- Architecture areas: `src/BizHawk.Emulation.Common` (core interfaces),
  `src/BizHawk.Emulation.Cores` (systems/CPUs), `src/BizHawk.Client.Common`
  (movies, savestates, rewind, Lua, input, APIs), `src/BizHawk.Client.EmuHawk`.
- TAS/debug areas: `src/BizHawk.Client.Common/movie`, `savestates`, `rewind`,
  `lua`, `tools`; `Assets/Lua/UnitTests`; `src/BizHawk.Tests.*`.
- Submodules: `.gitmodules` lists 30+ external cores/dependencies (mupen64plus,
  Genesis-Plus-GX, SNES9x, mGBA, SameBoy, Dolphin/PPSSPP, etc.). They were not
  cloned, so their licenses and code were not inspected.
- Initial assessment: strong reference for multi-system capability abstraction, TAS
  movies, savestates and debugging UX. Not a reuse source for production without a
  component-level legal audit; default position is no code reuse.

## What was not verified

- No upstream project was built or executed in this environment.
- The exodus-headless equivalence gate was not rerun.
- The MCP session test was not run (requires a real ROM).
- BizHawk submodules and compiled binaries were not audited.
- No claim is made about game compatibility or cycle accuracy.

These gaps are deliberate and recorded rather than hidden.
