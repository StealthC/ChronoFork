# Exodus / exodus-for-ais: emulator architecture, provenance, and backend assessment

**Status:** research note for the ChronoFork bootstrap.
**Author:** architecture-researcher.
**Date:** 2026-11-13.
**Scope:** the pinned upstream tree and the pinned Linux-headless fork under `.research/upstream/`, plus the tests and adapter code they ship.

All paths below are relative to `.research/upstream/`. `exodus/` is `RogerSanders/Exodus` at `master`; `exodus-headless/` is `paulomanrique/exodus-for-ais` at `linux-headless`. Citations use `path:line` or `path:line-range`.

**Evidence convention used throughout**

- **[inspected]** — read from the pinned source in this workspace.
- **[executed]** — a command was actually run here, with its output recorded in Appendix A.
- **[fork-reported]** — stated by the fork's own documentation/tests but not reproduced here.
- **[inferred]** — a conclusion from source structure, not demonstrated by a test.
- **[unverified]** — cannot be established from this workspace or was not attempted.

---

## 0. Verdict in one page

Exodus is a serious, debugging-oriented Sega Mega Drive emulator built around a plugin architecture: a declarative module/device graph, typed device interfaces, optimistic parallel device execution with rollback, full machine save states, a breakpoint/watchpoint debugger, reflection-based device introspection, and an execution-guided disassembler called **Active Disassembly**.

The Linux-headless fork is not a rewrite. It is a careful adapter and portability patchset: a CMake build, a static device/extension registry replacing the Windows plugin scan, a host that implements the GUI interfaces without a GUI, a CLI probe, an MCP daemon, and a set of tests. The emulation cores are essentially byte-identical to upstream. That makes the fork unusually useful as a **reference oracle** and as a source of test fixtures and architectural patterns.

It is **not** a drop-in engine for ChronoFork:

1. There is **no stable C ABI**. The engine is C++17 virtual interfaces, `std::wstring`, RTTI, and a monolithic `System` class. The only `extern "C"` symbols are plugin metadata/allocator entry points, not an embedding API.
2. It is **multi-threaded by design**, with one worker thread per active device, optimistic per-timeslice execution, rollback on conflicting accesses, and additional render/DMA/audio threads. Reading device state while running is explicitly unsafe; the MCP daemon gates every read on a stopped state.
3. Its **rollback is not user-visible time travel**. It is an in-flight conflict-resolution mechanism. There is no input log, replay, movie, incremental snapshot, or deterministic-replay facility in the tree.
4. The **shipped daemon is single-session and single-instance** (one `System` per process). The core classes look instance-safe by inspection, but multi-instance operation is untested.
5. The **MCP surface is deliberately narrow**: 35 tools for load/step/read/debug/trace/disassemble. It has no save-state, controller-input, module-setting, screenshot, or multi-session tool.
6. Its **correctness gate is narrow but real**: a 68000 instruction trace over a two-frame boot path, compared byte-for-byte across MSVC/Windows and clang/Linux. It does **not** compare VDP, audio, Z80, or memory state. The cross-platform numbers are fork-reported and not reproduced here.

**Recommendation:** use the fork as an external **oracle and fixture generator** during bootstrap (subprocess MCP/probe, trace and disassembly comparisons). Do not make it the primary backend for ChronoFork's deterministic snapshot/rollback model. If in-process embedding becomes necessary, wrap only `HeadlessHost` + `System` + the static registries in a narrow C ABI, keep one instance per process initially, and own the scheduler/determinism layer in ChronoFork rather than inheriting Exodus's.

---

## 1. What was inspected, and what was not

### 1.1 Pinned revisions [executed]

| Repo | URL | Branch | Commit SHA | Commit date | HEAD subject |
|---|---|---|---|---|---|
| Upstream Exodus | `https://github.com/RogerSanders/Exodus` | `master` (also `origin/HEAD`) | `08f388f77040af28d16d44fdfbddb73252953161` | 2024-02-16 11:00:39 +1100 | `Added timing information to trace logs and improved speed when tracing active` |
| exodus-for-ais | `https://github.com/paulomanrique/exodus-for-ais` | `linux-headless` (also `origin/HEAD`) | `ec0056151f3ff1c334d09ee375abb30a68fdfec5` | 2026-08-13 07:16:08 -0300 | `docs: make the repository legible to the agents it is for` |

Both working trees were clean at inspection time. The research clones are ignored by ChronoFork (`/home/stealthc/projects/ChronoFork/.gitignore` contains `.research/` and `**/.research/`).

### 1.2 Method

- Enumerated the tree, build metadata, licenses, and source layout.
- Read the public interfaces (`IDevice`, `IDeviceContext`, `IBusInterface`, `IClockSource`, `IProcessor`, `IGenericAccess`, `ISystemGUIInterface`) and the concrete implementations behind them.
- Read the scheduler, rollback, savestate, debugger, Active Disassembly, headless host, MCP server, and tests.
- Diffed the two trees file by file to separate fork changes from upstream emulation code (`diff -rq`, byte comparison, per-file diff).
- Ran the pure-Python test ROM generator and the trace comparator from the fork. These do not require building the emulator and do not write into the research tree.

### 1.3 Not done, and why

- **No emulator build.** This workspace has `cmake`, `g++`, `pkg-config`, `python3`, `expat`, `zlib`, and `libjpeg`, but **no `clang++`, no `ninja`, and no `libpng`**. The fork's Linux build hard-requires clang++ and Ninja and links libpng (`exodus-headless/CMakeLists.txt:7-16`, `:47-53`). An out-of-tree build could avoid writing into the research clone, but the missing toolchain is the binding constraint, and modifying `.research/upstream/` is forbidden by this task.
- **No Windows/MSVC reproduction.** The fork's cross-compiler equivalence claim is therefore [fork-reported], not independently confirmed.
- **No real commercial ROM.** The only executable ROM evidence available is the fork's generated synthetic cartridge, which is intentional and license-clean.
- **No multi-instance experiment.** No test in the tree constructs two `System` objects; the assessment in §5.16 is [inferred].

---

## 2. Provenance and licensing

### 2.1 History: the fork is a squashed snapshot, not a git descendant

`exodus-headless` has exactly **one commit** (`git rev-list --count HEAD` = 1) and does not contain the upstream commit object. Its history therefore proves nothing about the upstream base. The upstream tree is also a single-commit import in this workspace.

What can be established is content identity:

- Upstream has **1,439 tracked files**.
- The fork contains **all 1,439** (none missing).
- **1,374 are byte-identical**.
- **65 differ**, all attributable to the port/adapter work below.

This is strong evidence that the fork is a snapshot of upstream `08f388f` (or a tree content-identical in those 1,374 files) plus changes. It is not cryptographic proof of the base commit; label it **[inferred]**.

### 2.2 What the fork actually changed

**Added (fork-only) trees:**

- `CMakeLists.txt` (17,552 bytes), `cmake/vcpkg-ports/expat/`, `vcpkg.json`, `_windows_build.bat`, `_windows_capture.bat`.
- `host/` — `HeadlessHost`, `HeadlessHostConfig`, `HeadlessViewManager`, `StaticDeviceRegistry`, `StaticExtensionRegistry`, `ExodusProbe`, and a POSIX path test.
- `mcp/` — JSON-RPC/MCP server, session, tool groups, generic-access bridge, disassembly graph, JSON/UTF-8 codecs, transport, tests.
- `tests/equivalence/`, `tests/hierarchicalstorage/`, `tests/rom/`.
- `skills/`, `AGENTS.md`, `README.md`, `_PortReports/`.
- `System/PlatformKeyCodes.h`, `Support Libraries/ImageInterface/BitmapTypes.h`.

**Modified upstream files (categories):**

| Category | Examples | Nature |
|---|---|---|
| Build/export | `Devices/*/interface.cpp`, `System/interface.cpp` | `EX_EXPORT` visibility macro for non-Windows; Win32 version-resource lookup guarded by `_WIN32`. No engine logic. |
| Thread portability | `Support Libraries/ThreadLib/*` | POSIX timestamps, locks, atomics, timers, best-effort priority. |
| File/stream | `Support Libraries/Stream/File.*`, `WAVFile.*`, `Stream.cpp` | POSIX file descriptors; `wchar_t` UTF-32 handling. |
| Audio | `Support Libraries/AudioStream/*` | Complete Win32 `waveOut` backend preserved; non-Windows silent null backend. |
| Paths | `Support Libraries/WindowsSupport/PathFunctions.*` | POSIX path semantics. |
| XML | `Support Libraries/HierarchicalStorage/*` | Raw bytes handed to Expat (which handles BOM/encoding) instead of decoding to `wstring` first; duplicate-attribute save fix; UTF-8/wide conversion. |
| Scheduler | `System/DeviceContext.cpp` | `Sleep(0)` → `std::this_thread::yield()` in dependency spin waits (4 sites). No scheduling logic change. |
| VDP | `Devices/315-5313/S315-5313_General.cpp`, `..._Rendering.cpp` | `0x4E+0` → `0x4E + 0` numeric-literal lexing fix. Same arithmetic; clang previously rejected the token. |
| Timed buffers | `ExodusSDK/TimedBuffers/RandomTimeAccessBuffer.inl` | Pointer-overload member access `.` → `->`, enum scoping. Affects a previously non-compiling specialization. |
| ROM loader | `Extensions/MDExtensions/MegaDriveROMLoader.*` | Menu/Win32 half compiled out; path-based load exposed; error reporting via event log. |
| Misc | `System/System.cpp` (3 lines: include), `Processor.h` (`<cstdint>`), `LogEntry.cpp`, `ExtensionInterface`, `MarshalSupport`, `Image`, `ZIP`, `Debug` | Portability and latent-bug fixes. |

**Emulation cores that are unchanged:** `Devices/M68000/M68000.cpp`, `Devices/Z80/Z80.cpp`, `ExodusSDK/Processor/Processor.cpp` (0 changed lines), `System/ExecutionManager.h` (1-line include), and almost all of `System/System.cpp` (3 lines). The M68000 and Z80 implementations are byte-identical to upstream apart from their plugin export files.

**Conclusion:** the fork is an **adapter + portability patchset**, not an emulation-behavior fork. That is good for using it as a reference, and it means fork-specific risk is concentrated in `host/`, `mcp/`, and platform shims rather than in the CPU/VDP/audio code.

### 2.3 Latent bugs the fork reports finding

The fork's equivalence README lists four defects exposed by clang/GCC that MSVC accepted (`exodus-headless/tests/equivalence/README.md:20-35`):

1. `Stream::ConvertUnicodeCodePointToUTF32` dropped the `0x10000` surrogate bias on the UTF-8 path. The diff shows the added UTF-32 recombination logic in `exodus-headless/Support Libraries/Stream/Stream.cpp` (STREAM_WCHAR_T_IS_UTF32 block). [inspected; fix not independently tested]
2. `HierarchicalStorageTree::SaveNode` wrote `BinaryDataPresent`/`SeparateBinaryData` twice on a load→save cycle, producing non-well-formed XML. The fix skips the generated attributes when saving (`exodus-headless/Support Libraries/HierarchicalStorage/HierarchicalStorageTree.cpp`, duplicate-attribute comment). [inspected]
3. `IMarshalSource`'s `std::array` specialization passed a template argument to a non-template function (`exodus-headless/Support Libraries/MarshalSupport/IMarshalSource.h`). [inspected]
4. `RandomTimeAccessBuffer`'s pointer overloads used `.` where they meant `->` (`exodus-headless/ExodusSDK/TimedBuffers/RandomTimeAccessBuffer.inl`). [inspected]

These are plausible and the fixes are localized, but only the fork's own reports validate them. Treat as [fork-reported] for correctness, [inspected] for existence.

### 2.4 License and third-party obligations

- **Exodus license:** MIT, `Copyright (c) 2024 Roger Sanders` (`exodus/License.txt:1-3`). Permissive: use, copy, modify, merge, publish, distribute, sublicense, sell, with the copyright/permission notice preserved. No copyleft.
- **Fork license:** `License.txt` is byte-identical to upstream (verified with `diff`). The fork README says additions are under the same licence (`exodus-headless/README.md:149-160`). The fork's added files carry no separate copyright header, so the safest compliance position is to keep `License.txt` intact and credit both Roger Sanders and the fork.
- **Third-party code:** none is vendored. `exodus/Third/` contains only MSBuild project/metadata files, no library sources. Linux links system `expat`, `zlib`, `libpng`, `libjpeg` via pkg-config (`exodus-headless/CMakeLists.txt:47-53`); Windows uses vcpkg (`exodus-headless/vcpkg.json`). Those libraries carry their own permissive licenses (Expat MIT, zlib zlib-style, libpng libpng-style, libjpeg BSD/IJG-style), which must be reviewed and reproduced when distributing binaries. [License names are general knowledge; the exact obligations were not reviewed here — **verify before shipping**.]
- **ROMs/BIOS:** none shipped. The Mega Drive module can reference an embedded Boot ROM file (`exodus/Data/Modules/MegaDriveTestSystem.xml` points at a Genesis TMSS BIOS filename), but no such file exists in the tree. The test suite generates its own 32 KiB cartridge (`exodus-headless/tests/rom/make_test_rom.py`).
- **No GPL/AGPL code or notice found** in the inspected core, host, MCP, or tests. This is an inspection result, not a legal audit.

---

## 3. Repository and build shape

### 3.1 Size and layout [inspected]

| Metric | Upstream | Fork |
|---|---:|---:|
| C/C++ lines (`*.c`, `*.cpp`, `*.h`, `*.hpp`) | 118,745 | not separately counted; adds host/MCP/test code |
| C++ source files (`*.cpp`) | 218 | 244 |
| Headers (`*.h`) | 564 | 581 |
| MSBuild `.vcxproj` | 52 | 52 |
| `CMakeLists.txt` | absent | 17.5 KB |
| Fork-added host/MCP/test lines | — | ~12,900 (including Python) |

Layout (both trees):

- `Devices/` — M68000, Z80, 315-5313 VDP, YM2612, SN76489, MD1600IO (A10000, bus arbiter, controllers), Memory (ROM/RAM/SharedRAM/TimedBufferInt).
- `ExodusSDK/` — `Device`, `DeviceInterface`, `Processor`, `GenericAccess`, `TimedBuffers`, `SystemInterface`, `ExtensionInterface`, `Extension`.
- `System/` — `System` (the monolith), `BusInterface`, `DeviceContext`, `ExecutionManager`, `ClockSource`, module loader.
- `Extensions/` — UI/menu extensions plus `MDExtensions/MegaDriveROMLoader`; `ProcessorMenus` holds the Active Disassembly *view*, while the analysis engine lives in `ExodusSDK/Processor`.
- `Exodus/` — Windows GUI host (`ExodusInterface`, views, menus, `main.cpp`).
- `Support Libraries/` — 16 small libraries (Marshal, Stream, HierarchicalStorage, Image, ZIP, ThreadLib, AudioStream, WindowsSupport, etc.) plus their documentation.
- `Data/Modules/` — declarative hardware definitions, 6 files.
- `Third/` — MSBuild metadata for third-party libs only.

### 3.2 Build systems

- **Upstream:** Visual Studio/MSBuild only (`Exodus.sln`, `ExodusTests.sln`, `ThirdPartyLibraries.sln`, 52 `.vcxproj`). No CMake, no Makefile, no Linux path. Win32 types leak into SDK headers (`WindowsSupport` packages), so native Linux compilation is not possible without the fork's patches.
- **Fork:** CMake static-library build. Each upstream `.vcxproj`'s `ClCompile` items become an `exodus_static_library` target (`exodus-headless/CMakeLists.txt:108-135`). The helper sets include roots (`Support Libraries`, `ExodusSDK`, `Devices`), C++17, `_STATICLIB`, and debug/release defines.
- **Hard gates:** on non-Windows, CMake refuses generators other than Ninja and compilers other than clang (`exodus-headless/CMakeLists.txt:7-16`). This is a deliberate portability/equivalence choice, not a technical necessity.
- **Dependencies:** pkg-config `expat`, `zlib`, `libpng`, `libjpeg`; libtiff is intentionally disabled with a generated `tiffio.h` shim that always fails, preserving the existing "TIFF not loaded" path (`CMakeLists.txt:67-107`).
- **Targets:** `exodus-probe` (CLI) and `exodus-mcpd` (MCP daemon), both linking the same core static libraries and host sources (`CMakeLists.txt:402-431`, `:446-481`). The static registry lists 23 device classes and one extension (`exodus-headless/host/StaticDeviceRegistry.cpp:99-137`, `host/StaticExtensionRegistry.cpp:73-91`).
- **Windows:** Visual Studio 2022 + vcpkg static triplet, with a local Expat port that builds `wchar_t` support (`_windows_build.bat`, `cmake/vcpkg-ports/expat`).

### 3.3 Buildability in this workspace [executed]

`cmake 3.28.3`, `g++`, `pkg-config`, `python3`, `expat 2.6.1`, `zlib 1.3`, `libjpeg 2.1.5` are present. `ninja`, `clang++`, and `libpng` are **missing**. The fork cannot be configured or built here without installing them; I did not install anything. This is the main reason all emulator-behavior claims below are inspection-based or fork-reported.

---

## 4. Runtime architecture

### 4.1 The device model

The central abstraction is `IDevice` (`exodus/ExodusSDK/DeviceInterface/IDevice.h:31-159`). It combines:

- **Lifecycle:** `BindToSystemInterface`, `BindToDeviceContext`, `Construct`, `BuildDevice`, `ValidateDevice`, `Initialize`, `BeginExecution`, `SuspendExecution`.
- **Typed references:** `AddReference` overloads for `IDevice`, `IExtension`, `IBusInterface`, `IClockSource` (plus removals).
- **Execution:** `ExecuteStep`, `ExecuteTimeslice`, `ExecuteTimesliceTimingPointStep`, `GetNextTimingPointInDeviceTime`, `ExecuteRollback`, `ExecuteCommit`, `GetUpdateMethod`, and notification hooks (`NotifyUpcomingTimeslice`, `NotifyBeforeExecuteCalled`, `NotifyAfterExecuteCalled`, `NotifyAfterExecuteStepFinishedTimeslice`).
- **State:** `SaveState`/`LoadState`, `SavePersistentState`/`LoadPersistentState`, `SaveSettingsState`/`LoadSettingsState`, `SaveDebuggerState`/`LoadDebuggerState`, `GetScreenshot`.
- **Electrical model:** CE-line state, `ReadInterface`/`WriteInterface`, `ReadPort`/`WritePort`, transparent variants, line state/`AdvanceToLineState`, clock-source rates, input key codes.

`IDeviceContext` (`exodus/System/DeviceContext.h:40-140`) owns the per-device worker thread, timeslice progress, suspend/transient-execution state, device dependencies, and callbacks into the system (`WriteLogEvent`, `FlagStopSystem`, `StopSystem`, `RunSystem`, `ExecuteDeviceStep`).

`Device` (`exodus/ExodusSDK/Device/Device.h:1-160`) implements names/module ID and holds the system/device-context pointers. `Processor` adds generic access, breakpoints, watchpoints, call stack, tracing, and Active Disassembly (`exodus/ExodusSDK/Processor/Processor.h:116-282`).

The **module XML** is the composition root: it declares device instances, buses, line groups, clocks and dividers, connectors, references, CE-line mappings, dependencies, and settings. The Mega Drive module (`exodus/Data/Modules/Sega Mega Drive 1600.xml`, 447 lines) is the canonical example.

### 4.2 Scheduling and synchronization

`IDevice::UpdateMethod` has three values: `None`, `Step`, `Timeslice` (`exodus/ExodusSDK/DeviceInterface/IDevice.inl:4-9`). On the Mega Drive, the 68000 and Z80 are `Step` devices, the VDP/YM2612/PSG are `Timeslice` devices, and memory/IO/bus-arbiter devices are passive (`None`). `DeviceContext::ActiveDevice()` returns true only for non-`None` devices (`exodus/System/DeviceContext.inl:208-211`).

**Free-running loop** (`exodus/System/System.cpp:2374-2445`): a detached `SystemExecuteThread` repeatedly calls `ExecuteSystemStepInternal(20000000.0)` — a 20 ms maximum timeslice — and throttles only at the host layer.

**One internal step** (`System.cpp:2253-2372`):

1. `ExecutionManager::GetNextTimingPoint` asks every device for its next timing point and picks the earliest one below the maximum timeslice (`exodus/System/ExecutionManager.inl:259-286`). `Step` devices return `-1` by default (`ExodusSDK/Device/Device.cpp:172-175`); the VDP returns the time to the next VSYNC (`Devices/315-5313/S315-5313_General.cpp:1027-1071`).
2. `ExecutionManager::ExecuteTimeslice` starts **all active devices concurrently**, one worker thread each, for the same timeslice; it then waits for all of them (`ExecutionManager.inl:173-198`; thread start at `:289-305`).
3. Devices that detect an out-of-order access call `SetSystemRollback` (`System.cpp:2457-2502`). Trigger sites include the 68000/Z80 line checks (`Devices/M68000/M68000.cpp:1436,1468,1770`; `Devices/Z80/Z80.cpp:1031,1066,1210`), VDP port accesses (`Devices/315-5313/S315-5313_Ports.cpp:121,686,1026,1282`), YM2612/PSG (`Devices/YM2612/YM2612.cpp:660,2307,2329`; `Devices/SN76489/SN76489.cpp:266,718`), bus arbiter/IO (`Devices/MD1600IO/MDBusArbiter.cpp:1215,1287`; `A10000.cpp:447,510,880,973`), and shared RAM (`Devices/Memory/SharedRAM.cpp:95,139`).
4. The system keeps the **earliest** rollback target, rolls every device back to its last committed state, shrinks the timeslice, and retries. On success it calls `Commit()` at the end of the step (`System.cpp:2366-2369`). Committed state is what `ExecuteCommit` snapshots inside each device.
5. `System::ExecuteSystemStep(targetTime)` calls `StopSystem()` on entry and loops internal steps until the target time (`System.cpp:2227-2251`). Stepping is therefore synchronous and always begins/ends halted.

**Dependencies:** module XML declares `Device.SetDependentDevice` edges. The Mega Drive wires Z80↔68000 and both→VDP (`Sega Mega Drive 1600.xml:103-112`). `DeviceContext` spin-waits with `std::this_thread::yield()` until the dependency's progress catches up (fork-modified `System/DeviceContext.cpp:547-550, 669-697, 857-860`; upstream used `Sleep(0)`). There is also a shared-execution-thread spinoff path for tightly coupled devices.

**Where determinism comes from, and where it does not:**

- Deterministic ingredients: fixed device array order; earliest-timing-point tie-break by index; IEEE double arithmetic; no wall-clock in emulation (except optional throttling); frame time fixed at `1e9 / 59.922751` ns in the MCP session (`exodus-headless/mcp/Session.cpp:19`) and probe.
- Non-deterministic risks: parallel device threads, `volatile` rather than `std::atomic` flags (`System.h:550-580`), rollback selection that keeps the first flag when two conflicts report equal target times, render/audio threads whose lag can stall the executor, and known `pendingRenderOperationCount` bugs (`Devices/315-5313/S315-5313_Rendering.cpp:163-173`). The render-thread comment explicitly says this has broken PSG/YM2612 and can make the execute thread wait forever.
- The fork's equivalence gate is empirical evidence for one 68000 boot path, not a proof for all device interleavings. Treat determinism as **[fork-reported for the tested path; unverified generally]**.

**Performance/observability hazards:**

- `System::SetSystemRollback` unconditionally formats and writes a debug line to `std::wcout` (`System.cpp:2459-2467`), even in release. Every rollback costs stream formatting and pollutes stdout. The MCP daemon redirects file descriptor 1 to stderr precisely because of this class of output (`mcp/Transport.h:1-40`; `mcp/Transport.cpp:41-59`).
- The free-running loop can execute up to 20 ms of emulated time per internal step, and rollbacks can rewind within that window. Conflict-heavy code can therefore be expensive.
- Each active device has a worker thread; the VDP adds a render thread and a DMA thread; YM2612 and PSG add render threads; Windows audio adds `waveOut` threads. A Mega Drive instance is roughly a ten-thread process before any host threads.

### 4.3 Module graph: buses, clocks, lines, connectors

The module file is a small hardware description language:

- **Devices:** VDP, 68000, RAM16Variable, YM2612, SN76489, Z80 RAM, Z80, A10000 IO, MDBusArbiter, Boot ROM, and four `TimedBufferIntDevice` memories (VRAM, CRAM, VSRAM, sprite cache).
- **Buses:** `M68kBus` 24-bit address/16-bit data, `Z80Bus` 16/8, `ControlPortBus`.
- **Clocks:** `MCLK` 53.2034 MHz (PAL; NTSC value noted in comment), `VCLK` = MCLK/7, `ZCLK` = MCLK/15, with device clock references.
- **CE lines:** LDS/UDS, R/W, AS, FC, ROM/BootROM, CAS/RAS/OE, Z80 ZRD/ZWR/ZRAM/SOUND, mapped as inputs/outputs with bit counts.
- **Connectors:** cartridge, expansion, three control ports, with exported buses/clocks/system settings.
- **Settings:** Region (U/E/J/JPAL), TMSS, Soft Reset, each with declared options and defaults.
- **Dependencies:** explicit device execution-order coupling.

This is one of the most reusable ideas in the codebase: **hardware topology is data, not code**. The `System` loader validates the graph, maps addresses/lines/clocks, and creates device contexts. It also gives the fork a clean way to load a cartridge: `MegaDriveROMLoader` parses the ROM header and *generates a module* that plugs into the cartridge port (`exodus-headless/Extensions/MDExtensions/MegaDriveROMLoader.cpp:218-360`).

### 4.4 Host boundary and headless adapter

`System` takes an `IGUIExtensionInterface&` in its constructor (`exodus/System/System.cpp:19-20`). That interface is the entire host contract: module loading, preferences, file dialogs, path handling, view manager, main-window handle, assembly loading, and the system event log.

The fork implements it in `host/HeadlessHost`:

- `HeadlessHost` implements `IGUIExtensionInterface`; `HeadlessViewManager` accepts and discards every view request; `GetMainWindowHandle()` returns null; file dialogs fail; preferences live in an in-memory map; paths resolve against a configured base path (`exodus-headless/host/HeadlessHost.h:1-130`; `HeadlessViewManager.h:1-40`; `HeadlessHostConfig.h:1-80`).
- `StaticDeviceRegistry` replaces the Windows plugin DLL scan: 23 device classes are linked directly and registered with a sentinel assembly handle (`host/StaticDeviceRegistry.cpp:99-137`).
- `StaticExtensionRegistry` registers `MegaDriveROMLoader` as a **persistent global extension**, which the Windows build does not do. This is the one deliberate behavioural difference, and it is why `session.load_rom` works after only loading `Sega Mega Drive 1600.xml` (`host/StaticExtensionRegistry.cpp:73-110`).
- `HeadlessHost::BuildConnectorMappings` silently picks the first compatible free connector when a module imports one; the Windows host would ask the user. Ambiguity is reported through the event log (`host/HeadlessHost.cpp:149-220`).
- `System` construction order is forced: build the host, construct the `System` with it, then `BindToSystem`.

**Initialization trap:** a module load raises `System`'s private `_initialize` flag. The flag is cleared in only one place: the top of `System::ExecuteDeviceStep`, which reacts by re-initializing every device. `System::ExecuteSystemStep` does **not** test the flag. The MCP session works around this by explicitly calling `System::Initialize()`, then performing one device step to burn the flag, then initializing again (`exodus-headless/mcp/Session.cpp:140-160`; `Session.cpp:180-205`). `exodus-probe` avoids the problem by only using `ExecuteSystemStep` and calling `Initialize()` explicitly (`host/ExodusProbe.cpp:360-380`). This is a fragile integration seam, not a documented API.

---

## 5. Subsystem assessments

Each subsection answers the same dimensions: what/where, dependencies, isolatability, multi-instance safety, state serialization, Rust callability, headless suitability, license, and how correctness is validated.

### 5.1 Motorola 68000 (`M68000`)

| Dimension | Assessment |
|---|---|
| **What it implements** | A 68000 interpreter: 84 opcode classes registered into a 16-bit decode table (65,536 entries), effective-address decoding, exception processing (group 0/1/2, interrupt autovector, trace, illegal, privilege, bus error), bus arbitration lines, clock input, and instruction-level tracing/disassembly. |
| **Where** | `Devices/M68000/` — `M68000.h` (known inaccuracies at top), `M68000.cpp` (constructor and 84 registrations at `:11` and `:71-275`; `ExecuteStep` at `:643`; trace/breakpoint at `:934-935`; Active Disassembly recording at `:985`; `ExecuteRollback`/`Commit` at `:1029`/`:1090`; `GetOpcodeInfo` at `:1262`; `LoadState`/`SaveState` at `:2861`/`:2995`), `M68000Instruction.*`, 84 per-opcode headers, `EffectiveAddress.*`, `ExecuteTime.*`, `OpcodeTable`. |
| **Dependencies** | `Processor` base (device, generic access, breakpoints, trace, Active Disassembly), `Data`, `IBusInterface`, `IClockSource`, `ThreadLib`. |
| **Isolatability** | High in principle: it is a static library with explicit bus/clock interfaces and no GUI dependency. A fake `IBusInterface` + `IDeviceContext` could host it. There is no standalone CPU harness or per-opcode test suite in the tree. |
| **Multi-instance safety** | Per-object state; no mutable globals or singletons found. Two 68000 instances in one `System` would need a module definition that declares two, which the stock modules do not. |
| **State serialization** | Full machine state in `SaveState`: A0-A7, D0-D7, SSP/USP/PC/SR, last read bus data, processor state, prefetch state, group-0 exception capture, pending line-access buffer, last timeslice length, line states, interrupt level. `LoadState` is the inverse. Debugger state is separate. |
| **Rust callability** | No C ABI. C++ virtual class with `std::wstring` names and callback-style interfaces. A Rust binding would need a C++ shim and C++ toolchain; cannot be `bindgen`-only because of virtual methods and non-POD types. |
| **Headless suitability** | Excellent. The probe and MCP both drive it without a display. Trace capture is a first-class API. |
| **License** | MIT (inherited). |
| **How correctness is validated** | (a) The fork's differential 68000 trace over 18,941 instructions across MSVC/Windows and clang/Linux [fork-reported]; (b) the synthetic test ROM's predicted instruction stream and exact address set (`tests/rom/rom_test.py:124-200`); (c) the MCP session's execute-a-MOVEQ-from-RAM test (`mcp/tests/session_test.py`, “execute an instruction from RAM”). No exhaustive opcode/flag conformance suite, no randomized differential test against a reference, no hardware oracle. |

**Important accuracy caveat:** the core executes each opcode as a single indivisible step and performs all bus accesses at one logical time. It does not model the 68000 prefetch pipeline. The header documents these as known inaccuracies, including delayed group-0 exceptions, bus grant only between instructions, unimplemented prefetch, approximate DIVS/DIVU timing, TAS behaviour, and A7 alignment questions (`Devices/M68000/M68000.h:1-25`). This is enough for analysis and software compatibility, not for hardware-cycle-exact bus conformance.

**Performance design worth borrowing:** `ExecuteStep` clones the decoded opcode into a preallocated `_opcodeBuffer` sized to the largest opcode class (`M68000.cpp:210-216`, `:966`), avoiding per-instruction heap allocation. Disassembly/`GetOpcodeInfo` uses a heap clone, but only on slow paths.

### 5.2 Zilog Z80 (`Z80`)

| Dimension | Assessment |
|---|---|
| **What it implements** | A Z80 interpreter with 37 opcode classes across three 8-bit tables (base, CB, ED). Used as the Mega Drive sound CPU, with a banked 68k↔Z80 bus and interrupt lines. |
| **Where** | `Devices/Z80/` — `Z80.h` (known inaccuracies at top), `Z80.cpp` (constructor/table sizes at `:9`; `ExecuteStep` at `:516`; trace/breakpoint at `:709-710`; `GetOpcodeInfo` at `:1322`; `LoadState`/`SaveState` at `:1612`/`:1737`), `Z80Instruction.*`, per-opcode headers, `EffectiveAddress.*`, `ExecuteTime.*`. |
| **Dependencies** | `Processor` base, `Data`, `IBusInterface` (Z80 bus), `IClockSource`. |
| **Isolatability** | High in principle; same caveats as the 68000. No standalone tests. |
| **Multi-instance safety** | Per-object state; no globals found. |
| **State serialization** | Full register/flag/alt-register/interrupt state plus line-access buffer in `SaveState`; inverse in `LoadState`. |
| **Rust callability** | No C ABI; same shim requirement as the 68000. |
| **Headless suitability** | Good. Driven indirectly on every Mega Drive run; the MCP test asserts two processors are present and uses the 68000 for most work. |
| **License** | MIT. |
| **How correctness is validated** | Only indirect system-level runs. The ROM fixture disables interrupts and runs 68000 code; it does not exercise the Z80. There is no Z80 trace or state test. |

**Documented gaps:** no interrupt modes 0/2, no port-based I/O, many undocumented/ED opcodes missing, and inaccurate Y/X flags for `BIT` (`Devices/Z80/Z80.h:1-25`). Active Disassembly is explicitly unsupported for the Z80 (`Processor::ActiveDisassemblySupported` returns false at `ExodusSDK/Processor/Processor.cpp:1137-1140`; only the 68000 overrides it to true at `M68000.cpp:2381-2384`).

### 5.3 Sega 315-5313 VDP

| Dimension | Assessment |
|---|---|
| **What it implements** | Mega Drive VDP: H/V counters, MCLK/pixel-clock conversion, H32/H40 and V28/V30 modes, PAL/NTSC, interlace, VRAM/CRAM/VSRAM via timed buffers, register writes timestamped within a timeslice, sprite cache, FIFO/access slots, 68k→VRAM DMA (with a worker thread), fill/copy DMA, layer A/B/window/sprite rendering, priority, shadow/highlight, line buffer, render thread, screenshot. |
| **Where** | `Devices/315-5313/` — `S315-5313_General.cpp` (`GetUpdateMethod` at `:737`, `NotifyUpcomingTimeslice` at `:749`, `ExecuteTimeslice` at `:934`, `GetNextTimingPointInDeviceTime` at `:1027`, `ExecuteRollback`/`Commit` at `:1074`/`:1273`, `GetScreenshot` at `:3205`), `S315-5313_Ports.cpp` (register/port semantics, DMA, rollback triggers), `S315-5313_Timing.cpp` (H/V counter advance, access slots, MCLK conversion), `S315-5313_Rendering.cpp` (render thread at `:149`; digital process at `:365`; analog process at `:1028`; sprite list at `:1880`; priority at `:2158`). |
| **Dependencies** | `Device`, `TimedBuffers`, memory devices (VRAM/CRAM/VSRAM/sprite cache), `IBusInterface`, `IClockSource`, `Image`/PNG for screenshots, render/DMA threads. |
| **Isolatability** | Low-to-medium. It needs four timed memories, clocks, a bus, and a host; the render pipeline assumes image buffers. No standalone harness. |
| **Multi-instance safety** | Per-object state; each instance spawns its own render/DMA threads. The `pendingRenderOperationCount` FIXME (`Rendering.cpp:163-173`) is per-instance but indicates a known liveness hazard. |
| **State serialization** | Heavy. `SaveState` writes registers, H/V counters, render state, DMA state, and each timed buffer's timeslice/write lists; it also produces a screenshot for ZIP savestates. This is one of the largest state serializers in the tree. |
| **Rust callability** | No C ABI. Screenshot and timed-buffer access are C++-only. MCP exposes only generic reflection paths and no framebuffer. |
| **Headless suitability** | Medium. It runs headlessly, but still renders in a background thread even though the host discards views. A `Settings/Disable Rendering` reflection flag exists (`S315-5313_General.cpp:212,246,4049,4729`), which a headless client could set to reduce work. MCP does not set it. |
| **License** | MIT. |
| **How correctness is validated** | Indirectly by the 68000 trace (VDP interrupts/ports affect 68000 timing), and by a smoke boot. There is no golden-frame/VRAM test, no register conformance suite, and no visual comparison. Active Disassembly tests exercise VDP port reads only as side effects. |

**Documented gaps:** Mode 4 (SMS compatibility), vertical-scroll inhibit, left-column blank, 128 KiB VRAM, test register, sprite collision/overflow flags, and immediate H-mode changes are unimplemented or inaccurate (`Devices/315-5313/S315_5313.h:1-20`). Rendering is line-oriented and uses a separate thread; it is not a cycle-by-cycle rasterizer.

### 5.4 Yamaha YM2612 (OPN2)

| Dimension | Assessment |
|---|---|
| **What it implements** | Six-channel FM synthesis with per-operator envelopes, LFO, timers, DAC, SSG-EG, per-channel/operator output flags, register interface, and a 48 kHz render pipeline. |
| **Where** | `Devices/YM2612/` — `YM2612.h` (known inaccuracies at top), `YM2612.cpp` (`Initialize` at `:392`; render thread start at `:686`; `ExecuteTimeslice` at `:745`; `ExecuteRollback`/`Commit` at `:761`/`:809`; render thread at `:876`; audio buffer submission around `:1454`), `IYM2612.*`, `interface.cpp`. |
| **Dependencies** | `Device`, `TimedBuffers`, `AudioStream`, `Stream::WAVFile` (optional logging), clock source, bus. |
| **Isolatability** | Medium. Can be instantiated without a GUI, but needs a clock, bus, audio stream, and timed buffers. No standalone tests. |
| **Multi-instance safety** | Per-object state and threads. On Windows, multiple instances ultimately share the `waveOut` device; on POSIX the audio backend is a no-op, so isolation is easier. |
| **State serialization** | Full register/operator/channel/timer/DAC state plus timed buffers and render bookkeeping in `SaveState`; inverse in `LoadState`. |
| **Rust callability** | No C ABI; PCM output is not exposed through MCP. |
| **Headless suitability** | Poor-to-medium for analysis: it runs and consumes CPU synthesizing audio even though the POSIX backend discards every buffer (`Support Libraries/AudioStream/AudioStream.cpp:571-675`). There is no global mute/disable setting. Device reflection exposes register state, so state inspection works. |
| **License** | MIT. |
| **How correctness is validated** | No audio golden files, no spectrum/sample comparisons, no tests in the tree. Only indirect execution during boot traces. |

**Documented gaps:** render is time-step rather than cycle-accurate; CSM overflow uses timeslice rounding; busy flag is unimplemented; DAC handling is approximate; the test register is partly unimplemented (`Devices/YM2612/YM2612.h:1-30`). Do not treat this as a sample-exact reference.

### 5.5 TI SN76489 PSG

| Dimension | Assessment |
|---|---|
| **What it implements** | Four channels (three tone, one noise), 10-bit tone dividers, 4-bit attenuation, noise LFSR, per-channel state, 48 kHz render pipeline. Header claims sample-accurate generated output and faithful known behaviour. |
| **Where** | `Devices/SN76489/` — `SN76489.h`, `SN76489.cpp` (render thread start at `:290`; `ExecuteTimeslice` at `:335`; render thread at `:396`; buffer submission around `:537`), `ISN76489.*`. |
| **Dependencies** | `Device`, `TimedBuffers`, `AudioStream`, clock source, bus. |
| **Isolatability** | Medium; same audio-stream dependency as the YM2612. No standalone tests. |
| **Multi-instance safety** | Per-object state/threads; same Windows audio caveat. |
| **State serialization** | Register/channel/noise state and timed-buffer state in `SaveState`. |
| **Rust callability** | No C ABI; no PCM surface in MCP. |
| **Headless suitability** | Same as YM2612: computes into a silent sink. |
| **License** | MIT. |
| **How correctness is validated** | None directly. Header documents open questions about attenuation-to-linear-power conversion and output filtering; those filters are not implemented. |

### 5.6 Memory, bus arbitration, and IO

| Dimension | Assessment |
|---|---|
| **What it implements** | ROM/RAM 8/16/32 and variable-size devices; `SharedRAM` for 68k/Z80 shared memory; `TimedBufferIntDevice` for VRAM/CRAM/VSRAM/sprite cache; `MDBusArbiter` (Z80 bus request/grant, bank register, 68k↔Z80 access); `A10000` IO (version register, controller ports, TMSS); `MDControl3`/`MDControl6` controllers. |
| **Where** | `Devices/Memory/` (`ROM*`, `RAM*`, `SharedRAM.*`, `TimedBufferInt*`), `Devices/MD1600IO/` (`MDBusArbiter.*`, `A10000.*`, `MDControl3/6.*`), bus mapping in `System/BusInterface.cpp` (3,003 lines) and module XML. |
| **Dependencies** | `Device`, `IBusInterface`, CE lines, clock sources, `TimedBuffers`. |
| **Isolatability** | Low for the arbiter/IO (deeply coupled to lines and both processors); high for individual ROM/RAM classes. |
| **Multi-instance safety** | Per-object; `SharedRAM` has explicit cross-device authorship tracking and rollback triggers (`SharedRAM.cpp:95,139`). |
| **State serialization** | Every device implements `SaveState`/`LoadState`; shared RAM saves contents and author/timeslice metadata; timed buffers save write lists. |
| **Rust callability** | No C ABI. |
| **Headless suitability** | Good; no display dependency. Controller input exists in the core (`System.h:195-196`) but is not exposed by MCP. |
| **License** | MIT. |
| **How correctness is validated** | The generated ROM proves cartridge mapping, ROM/RAM reads/writes, and the reset vector path. The MCP test proves a write to `0xFF0000` reaches work RAM and that a write to ROM is discarded. Bus-arbiter/TMSS/controller behaviour is untested. |

### 5.7 Device abstraction and scheduling: assessment

| Dimension | Assessment |
|---|---|
| **What it implements** | A plugin-style device contract with lifecycle, execution, bus/line/clock, reflection, and serialization; a per-device worker-thread execution manager; a deterministic-by-construction timing-point selection; optimistic parallel execution with rollback; dependency-lockstep edges. |
| **Where** | `ExodusSDK/DeviceInterface/*`, `System/DeviceContext.*`, `System/ExecutionManager.*`, `System/BusInterface.*`, `System/ClockSource.*`, `System/System.cpp`. |
| **Dependencies** | C++17, RTTI, `std::thread`/mutex/condition variables, `ThreadLib`, `MarshalSupport`. |
| **Isolatability** | The interfaces are clean, but `System` is required to construct and wire devices. There is no "library-only" entry point that takes a device graph and returns an engine. |
| **Multi-instance safety** | Structurally plausible (§5.16); not tested. |
| **State serialization** | Per-device `SaveState`/`LoadState`; debugger state separate; `System` saves module relationships and device order. |
| **Rust callability** | No C ABI; `System/interface.cpp:5-13` exposes C++ `GetSystem(IGUIExtensionInterface&)` / `DeleteSystem(...)`, the closest thing to a factory. Not usable from Rust without a shim. |
| **Headless suitability** | Good once `HeadlessHost` is in place. The host contract is wide (preferences, paths, dialogs, views), so a new host must implement all of it. |
| **License** | MIT. |
| **How correctness is validated** | The fork's MCP/session tests exercise stepping, run/stop gating, breakpoints, traces, and reflection end-to-end. No scheduler stress test, no TSAN, no deterministic repeat-under-load test, no multi-instance test. |

### 5.8 Save states and rollback

| Dimension | Assessment |
|---|---|
| **What it implements** | Full-machine snapshots in XML or ZIP (`save.xml` + binary buffers + optional `screenshot.png`), including module relationships and per-device state. A separate "debugger state" mode saves breakpoints, watchpoints, trace settings, and Active Disassembly data. |
| **Where** | `System/System.cpp` (`LoadState` at `:74`; `SaveState` at `:351`; `FileType` enum in `ExodusSDK/SystemInterface/ISystemGUIInterface.inl:13-17`), per-device `SaveState`/`LoadState`, `Support Libraries/HierarchicalStorage/*`, `Support Libraries/ZIP/*`, `Support Libraries/Image/*`. |
| **Dependencies** | `HierarchicalStorage` (Expat), `ZIP` (zlib), `Image` (libpng/libjpeg), `Stream`, every device's serializer. |
| **Isolatability** | The XML layer is isolatable (there is a standalone round-trip test). The full savestate is not: it needs the whole device graph. |
| **Multi-instance safety** | Per-`System` file path and tree; no global serializer state found. |
| **State serialization** | XML or ZIP. Metadata includes creation date/time and a debugger-state flag (`System.cpp:361-370`), so byte-identical snapshots are not expected. Binary buffers are externalized as separate ZIP entries. There is no explicit format version field beyond the tree shape. |
| **Rust callability** | The files are parseable by Rust as XML/ZIP, but the semantics are large and undocumented outside the C++ serializers. No stable schema; safest is to add a C++/MCP save-state tool rather than reimplement the format. |
| **Headless suitability** | The core supports it; **MCP does not expose it**. There is no `save_state`/`load_state` tool in the 35-tool surface. |
| **License** | MIT. |
| **How correctness is validated** | Only the `HierarchicalStorage` XML round-trip test (`tests/hierarchicalstorage/RoundTripTest.cpp`, 682 lines) and the latent duplicate-attribute fix. There is no full savestate round-trip test, no cross-build compatibility test, and no state-hash test. |

**Critical distinction:** `ExecuteRollback`/`ExecuteCommit` are **not** a user-facing rollback/time-travel feature. They restore the last committed device state when an out-of-order access is detected inside a timeslice. There is no input log, replay, movie format, or per-frame snapshot ring in the tree (`grep` for `movie`, `replay`, `input record`, `lua`, `python` found nothing in the core/host/extensions).

### 5.9 Debugger

| Dimension | Assessment |
|---|---|
| **What it implements** | Per-processor breakpoints with location conditions (`=`, `>`, `<`, range), masks, enable flags, log/break actions, hit counters, and break intervals; watchpoints with read/write and data conditions; call stack with step-over/step-out; transient breakpoints; counted-loop detection; trace logging. |
| **Where** | `ExodusSDK/Processor/Processor.cpp` (`CheckExecutionInternal` at `:500`; `TriggerBreakpoint` at `:561`; `BreakpointCallback` at `:596`; watchpoint checks at `:703`/`:739`; `RecordTraceInternal` at `:1081`), `Breakpoint.*`, `Watchpoint.*`, `IProcessor.h`, `Extensions/ProcessorMenus/*` (GUI views). |
| **Dependencies** | `Processor` base, `DeviceContext` callbacks, `System` rollback/stop machinery, `HierarchicalStorage` for state. |
| **Isolatability** | Low outside a running `System`; the debugger is embedded in the processor base and stops the system through the scheduler. |
| **Multi-instance safety** | Per-processor lists/mutexes; handles are per-session in MCP. |
| **State serialization** | `SaveDebuggerState`/`LoadDebuggerState` include breakpoints, watchpoints, trace settings, call stack, and Active Disassembly data. Normal savestates exclude debugger state. |
| **Rust callability** | MCP tools only (`breakpoint.*`, `watchpoint.*`, `trace.*`). No C ABI. |
| **Headless suitability** | Good: breakpoints stop a free-running system; stepping is synchronous. The daemon does not push events, so a client polls `run.status`. |
| **License** | MIT. |
| **How correctness is validated** | MCP session tests cover create/list/delete, range conditions, non-breaking hit counts, deleted/forged handle rejection, trace ring behavior, and trace entry fields. Watchpoints are created/listed/deleted but not fired in the test. |

**Mechanics worth noting:** a breakpoint does not simply set a flag. `TriggerBreakpoint` calls `SetSystemRollback` with a callback at the current timeslice progress; the scheduler rolls the machine back to that exact point, then the callback increments the hit counter and calls `FlagStopSystem`. This is why breakpoints land at instruction boundaries even under the optimistic parallel scheduler (`Processor.cpp:561-605`).

**Gaps:** watchpoints are not exposed through a `GenericAccess` page (there is a TODO at `Processor.cpp:126`; only breakpoints get a page). `BreakpointDeleteAll` is reachable through `device.execute_command`, so MCP re-verifies handles against the processor's own list before dereferencing (`mcp/Session.h:130-150`, `Session.cpp:599-690`).

### 5.10 Active Disassembly

| Dimension | Assessment |
|---|---|
| **What it implements** | Execution-guided disassembly. While recording, every executed instruction is recorded as code with its size; data reads from `MOVEM`/`MOVEP` are recorded as data; indexed `JMP`/`JSR` targets are recorded as offsets and possible branch tables. Analysis then merges observations, detects offset arrays, validates suspected jump tables, predicts boundaries, explores code paths, generates labels, and exports ASM/text/IDC/JSON. |
| **Where** | Engine: `ExodusSDK/Processor/Processor.cpp` (`AddDisassemblyAddressInfoCode` at `:1675`; data at `:1738`; offset at `:1819`; possible branch table at `:1873`; `PerformActiveDisassemblyAnalysis` at `:1978`; array detection at `:2519`; jump-table validation at `:3040`; predicted entries at `:3102`; ASM export at `:3605`; text export at `:4037`; IDC export at `:4105`). Recorder hooks: `Devices/M68000/M68000.cpp:985`, `Devices/M68000/JMP.h:95,107`, `JSR.h:101,115`, `MOVEM.h:307`, `MOVEP.h:110`. Fork JSON: `mcp/DisassemblyGraph.*`. GUI: `Extensions/ProcessorMenus/ActiveDisassemblyView*`. |
| **Dependencies** | `Processor` base, opcode decoding/disassembly, memory-space reads, `Stream::File` (exports), `Mutex`/debug lock. |
| **Isolatability** | Medium. It needs a processor with opcode decode and memory access; it is not usable without a device instance. The JSON graph builder additionally needs the SDK's text export. |
| **Multi-instance safety** | Per-processor analysis state; guarded by `_debugMutex`. Only the 68000 supports it in this tree. |
| **State serialization** | Active Disassembly data is saved in **debugger state**, not normal state, as a custom binary block: address-info entries, jump tables, arrays, options, and labels (`Processor.cpp:4364-4550`). |
| **Rust callability** | Best path is the MCP JSON export (`schema: "exodus.disassembly/1"`) or the text/ASM files. The in-memory analysis type is private and has no `IProcessor` accessor. |
| **Headless suitability** | High: this is the fork's reason for existing. It works without a GUI and is exposed through `disasm.*`. |
| **License** | MIT. |
| **How correctness is validated** | The synthetic ROM test is strong: it asserts every executed instruction is recovered as code and nothing else; exactly one data array at `0x300`, byte-size, 4 entries; the jump table at `0x280`, 4 word entries, relative, base `0x280`, targets `[0x400,0x420,0x440,0x460]`, all classified as code; reset vectors resolve to SSP/PC (`tests/rom/rom_test.py:304-390`). The MCP session test cross-checks JSON region counts against the SDK's own counters and explicitly pins the zero-ROM coverage gap. |

**Limits and risks:**

- Coverage is only what executed. A two-frame run sees the boot path, not the game.
- Only the 68000 supports it; Z80 `disasm.*` fails with an explanatory error.
- The engine's analysis model is private. The fork's JSON graph is built by **exporting text to a temporary file and parsing it back**, because the fork was not permitted to change `ExodusSDK` (`mcp/DisassemblyGraph.h:1-100`, `DisassemblyGraph.cpp:134-220`). That is a documented fragility: the parser is tolerant, counts skipped lines, and reports them, but it is not a direct model read.
- Offset-code vs offset-data is collapsed in the text export; the graph recovers jump-table vs pointer-table classification by resolving targets against classified code/data (`DisassemblyGraph.h:40-100`). This is evidence-based but heuristic.
- No user scripting, no symbol/source integration, no manual code/data override API.

### 5.11 Device introspection (`IGenericAccess`)

| Dimension | Assessment |
|---|---|
| **What it implements** | A reflection system for device state: pages → groups/collections/selection lists → data entries and commands. Values carry a type (`bool`, `int`, `uint`, `float`, `double`, `string`, file/folder path), display mode, min/max, read-only/locking flags, and descriptions. Addressable by data ID + optional data context. |
| **Where** | `ExodusSDK/GenericAccess/*`; per-device pages: M68000 `Registers` and `Breakpoints` (`Devices/M68000/M68000.cpp:219-269`; `Processor.cpp:126-155`), VDP `Settings`/`Debug`/`Layer removal` (`S315-5313_General.cpp:212-279`), YM2612 `Audio Logging` (`YM2612.cpp:333-390`), PSG registers/settings (`SN76489.cpp:69-84`). MCP bridge: `mcp/GenericAccessBridge.*`. |
| **Dependencies** | `Device`/`Processor` base, `GenericAccess` containers, `MarshalSupport`. |
| **Isolatability** | Medium: it is part of each device; the MCP bridge is independent but needs a running device. |
| **Multi-instance safety** | Per-device page objects; contexts are device-owned and re-resolved on each call; no caching across calls (`GenericAccessBridge.h:60-85`). |
| **State serialization** | Reflection state is derived from device state; settings/debugger state have separate serializers. |
| **Rust callability** | The MCP JSON projection is the practical interface. The C++ reflection types are virtual/marshal-heavy and not ABI-stable. |
| **Headless suitability** | Excellent concept: a new device is exposed without daemon changes. The MCP bridge walks the tree and addresses entries by path. |
| **License** | MIT. |
| **How correctness is validated** | MCP session tests: `device.describe` returns pages/entries with types; `A0`-`A7` remain eight distinct paths despite sharing one data ID; read/write by path round-trips strings and numbers; unknown paths error; VDP reflection exists despite being a non-processor (`session_test.py` device section). |

**Path semantics:** `Registers/User Registers/Address Registers/A0`; matched case-sensitively; tree depth capped at 24 to avoid pathological trees (`GenericAccessBridge.cpp:16-20`). Paths are stable while a device is loaded but are not a versioned public schema. Writes can perturb state; there is no dry-run.

### 5.12 Build system and Linux portability

| Dimension | Assessment |
|---|---|
| **What it implements** | Upstream: MSBuild only. Fork: CMake static build, POSIX shims, static registration, two executables, Windows vcpkg path, and a set of portability fixes. |
| **Where** | Upstream `*.sln`/`*.vcxproj`, `Build/PropertySheets/*`; fork `CMakeLists.txt`, `cmake/vcpkg-ports/expat`, `host/Static*Registry`, `Support Libraries/ThreadLib/*`, `Stream/File.*`, `AudioStream/*`, `WindowsSupport/PathFunctions.*`, `HierarchicalStorage/*`, `Stream/Stream.cpp`, `MarshalSupport/IMarshalSource.h`, `TimedBuffers/RandomTimeAccessBuffer.inl`. |
| **Dependencies** | CMake ≥3.20, Ninja, clang++, C++17, pthreads, expat, zlib, libpng, libjpeg; Windows: MSVC 2022 + vcpkg. |
| **Isolatability** | Each support library and device is a static library, but the executables pull almost everything. There is no shared/`cdylib` build. |
| **Multi-instance safety** | Build artifacts are per-executable; no global mutable build state. |
| **State serialization** | N/A. |
| **Rust callability** | Static libraries are linkable by a C++ shim, not directly by Rust. No `cdylib`, no C headers for the engine. |
| **Headless suitability** | The fork's whole purpose. Linux is the tested platform; Windows is for reference capture. |
| **License** | MIT for the port; third-party licenses separate. |
| **How correctness is validated** | `_PortReports/*.md` document the porting decisions and prior validation; the fork reports a successful clang build and equivalence run. Not reproduced here. |

**Remaining portability caveats:** `std::wstring`/`wchar_t` assumptions remain throughout (Linux `wchar_t` is 32-bit; Windows is 16-bit); RTTI is required; there is no exception use, so error propagation is `bool` + event log; `PlatformKeyCodes.h` is a small mapping shim for key codes; no ARM64/macOS verification; the fork's clang-only gate excludes GCC even though some test scripts use `g++`.

### 5.13 Headless execution and CLI

| Dimension | Assessment |
|---|---|
| **What it implements** | `exodus-probe`: one-shot module/ROM load, N-frame run, optional trace dump, event-log report, exit codes. `exodus-mcpd`: long-lived stdio MCP server with a single session and explicit run/stop gating. |
| **Where** | `host/ExodusProbe.cpp` (usage at `:30-55`; main at `:250-520`), `mcp/Main.cpp`, `host/HeadlessHost*`, `host/HeadlessViewManager*`. |
| **Dependencies** | The full core plus host and (for MCP) JSON/transport/session/tool code. |
| **Isolatability** | Each executable is self-contained; they share source lists rather than a common library (`CMakeLists.txt:402-481`). |
| **Multi-instance safety** | One `Session`/`System` per daemon. Multiple daemons are separate processes. |
| **State serialization** | Not exposed by the CLI/MCP; core savestates exist but are unreachable from these surfaces. |
| **Rust callability** | The CLI is trivially callable as a subprocess; the MCP protocol is the richest surface. |
| **Headless suitability** | Good: no window, no audio device, no dialogs. Still thread-heavy and still renders/audio-synthesizes in the background. |
| **License** | MIT. |
| **How correctness is validated** | The probe is used as the equivalence capture tool and in the ROM test stages 1–2. The daemon is exercised by `mcp/tests/session_test.py` and the ROM test stage 3. |

**CLI surface (probe):** `--rom <file>`, `--frames <n>` (default 10), `--frame-ns <n>` (default `1e9/59.922751`), `--base-path <dir>`, `--throttle`, `--no-mkdir`, `--trace-dump <f>`, `--trace-device <instance name>`, `--trace-length <n>` (default 1,000,000), `--help` (`ExodusProbe.cpp:30-55`). Default tracing uses the **instance** name (`Main 68000`), not the class name; wrong names yield an empty trace.

**Daemon surface (mcpd):** `--base-path <dir>`, `--module <path>` (load at startup, mostly for tests), `--mkdir`, `--list-tools` (render schemas without constructing the emulator), `--help` (`mcp/Main.cpp:20-50`).

**Operational caveats:**

- `run.step_frames` is synchronous and capped at 100,000 frames; a single request can block for a long time (`mcp/Session.cpp:21-27,289-311`). The client cannot interrupt it except by killing the process.
- A free-running session stops at a breakpoint but does not notify the client; the client polls `run.status`.
- The daemon claims stdout before constructing the core because device code writes diagnostics to `std::wcout` (`mcp/Main.cpp:1-20`; `Transport.cpp:41-59`). Any Rust host that embeds the core must reproduce this redirection or patch the logging.

### 5.14 MCP/CLI tool surface

**Protocol:** JSON-RPC 2.0 over newline-delimited JSON on stdio; MCP protocol version `2025-06-18`; server `exodus-mcpd` version `0.1.0`; methods `initialize`, `notifications/initialized`, `tools/list`, `tools/call`, `ping`, `notifications/cancelled`; batch arrays supported and processed sequentially (`mcp/Server.cpp:19-21,113-169,172-400`; `mcp/Transport.h`). It implements tools only — no resources, prompts, sampling, or notifications from server to client.

**Tool inventory (35, verified by registration count):**

| Family | Tools | Count |
|---|---|---:|
| `session.*` | `load_module`, `load_rom`, `info`, `reset`, `shutdown` | 5 |
| `run.*` | `start`, `stop`, `status`, `step_frames`, `step_device` | 5 |
| `cpu.*` | `list`, `registers`, `read_memory`, `write_memory`, `disassemble` | 5 |
| `device.*` | `describe`, `read`, `write`, `execute_command` | 4 |
| `breakpoint.*` | `create`, `list`, `delete` | 3 |
| `watchpoint.*` | `create`, `list`, `delete` | 3 |
| `trace.*` | `enable`, `disable`, `get`, `clear` | 4 |
| `disasm.*` | `configure`, `start`, `stop`, `analyze`, `export`, `status` | 6 |

**Design strengths:**

- **Run-state gating:** every state-reading tool calls `RequireStopped`; while free-running it returns an error naming `run.stop` rather than silently stopping (`mcp/Session.h:40-90`, `Session.cpp:271-287`; 17 call sites across tool files). This is the correct response to a scheduler that can hand back torn state.
- **Opaque handles with validation:** device handles are `dev-N`, breakpoints `bp-N`, watchpoints `wp-N`; pointers are never serialized. Breakpoint handles are re-verified against the processor's own list because `device.execute_command` can delete them behind the daemon's back (`mcp/Session.h:130-180`, `Session.cpp:599-690`).
- **Argument validation:** undeclared arguments are rejected with the accepted list and a near-miss suggestion; addresses accept numbers or `0x` strings; unknown devices/paths are explicit errors (`mcp/Server.cpp:352-400`; `ToolRegistry.h` argument helpers).
- **Errors split correctly:** protocol mistakes are JSON-RPC errors; tool-level failures are normal results with `isError` and an attached event log (`Server.h:1-40`, `Server.cpp:380-420`).
- **Device-agnostic introspection:** the 4 `device.*` tools are generated from `IGenericAccess`, not per device. The same tool reads VDP, YM2612, PSG, Z80, and cart mapper state.

**Gaps (important for ChronoFork):**

- **No save-state tool.** Core `System::SaveState`/`LoadState` are unreachable.
- **No input tool.** There is no way to press Start/A/B/C via MCP, even though `System::HandleInputKeyDown/Up` exists (`System.h:195-196`).
- **No module-setting tool.** Region, TMSS, and Soft Reset can only be chosen by module XML or ROM header autodetection (`ISystemGUIInterface.h:65-71` has the API; no tool uses it).
- **No screenshot/framebuffer tool.** `S315_5313::GetScreenshot` exists but is only used inside ZIP savestates.
- **No multi-session or multi-instance.** One `Session` owns one `System` (`Session.h:202-205`), and the MCP server owns one `Session`.
- **No async/notification.** Long steps block; breakpoint stops are discovered by polling.
- **No audio capture.** YM2612/PSG WAV logging settings exist in reflection but are not exposed as tools.
- **Frame length is fixed at NTSC** (`1e9/59.922751`) even if the module/ROM selects a PAL region. `run.step_frames` advances by that constant; region changes affect VDP timing, so “one frame” may not equal one video frame on PAL. [inspected; behavioral impact unverified]
- **Step limits are generous but not interruptible** (`100,000` frames, `1,000,000` device steps).

### 5.15 Equivalence and trace tests

| Dimension | Assessment |
|---|---|
| **What it implements** | A self-describing trace dump format; an independent Python comparator; a capture path through `exodus-probe`; a cross-platform gate; unit/integration tests for JSON, UTF-8, paths, XML, ROM loading, MCP, and Active Disassembly. |
| **Where** | `tests/equivalence/README.md`, `compare_traces.py`; `tests/rom/{make_test_rom.py,rom_test.py,build_and_run.sh}`; `tests/hierarchicalstorage/*`; `host/tests/*`; `mcp/tests/*`. |
| **Dependencies** | Python 3 for most harnesses; built binaries for the end-to-end tests; Expat for the XML test; clang++/g++ for unit tests. |
| **Isolatability** | The comparator and ROM generator are standalone; the MCP session test needs a built daemon; the XML test compiles only the storage layer. |
| **Multi-instance safety** | Tests run one process at a time; the ROM/MCP tests spawn a daemon subprocess. |
| **State serialization** | The trace dump is tab-separated text with cycle/address/opcode/args/time; comments are ignored. The comparator enforces exact cycle/address/opcode/args and a `1e-9` time tolerance, with an option to ignore disassembly formatting. |
| **Rust callability** | The trace format and JSON export are easy for Rust to consume; the tests themselves are Python/C++. |
| **Headless suitability** | Designed for headless. |
| **License** | MIT (tests included). |
| **How correctness is validated** | The fork reports byte-identical MSVC/clang traces over 18,941 68000 instructions (same SHA-256), and a negative control where one record is corrupted and caught at the right divergence point (`tests/equivalence/README.md:90-120`). I reproduced the comparator's accept/reject behavior on synthetic traces (§Appendix A), but not the cross-compiler run. |

**What the equivalence gate actually covers:**

- Only the **68000 instruction stream**: cycle count, address, disassembly text, and cumulative time.
- It does **not** compare Z80 state, VDP registers/VRAM, audio samples, memory contents, or interrupt timing directly. Since the 68000's timing is affected by VDP interrupts and bus contention, the test is a meaningful indirect smoke test, but not a machine-equivalence proof.
- It covers a **two-frame boot path with no commercial ROM** (zero-filled cartridge space). The fork's own session test documents that offset/data/label/jump-table analysis paths are unexercised without a ROM and deliberately pins that gap (`mcp/tests/session_test.py`, “COVERAGE GAP” comment).
- The Windows side requires a separate machine/toolchain; CI cannot reproduce it on Linux alone.

**Test inventory and rough coverage:**

| Suite | Size | Covers |
|---|---:|---|
| `mcp/tests/JsonTest.cpp` | 236 lines | Parser/serializer, integer round-trips, escapes, ordering, malformed input rejection. |
| `mcp/tests/TextCodecTest.cpp` | 226 lines | UTF-8 ↔ wide strings, supplementary code points, lone surrogates, malformed bytes. |
| `mcp/tests/session_test.py` | 1,010 lines, 186 assertion call sites | Handshake, tools/list, guards, module load, device resolution, registers, stepping, run/stop gating, memory, execute-from-RAM, breakpoints, watchpoints, trace, reflection, Active Disassembly, JSON/ASM/text/IDC export, reset/shutdown/reload. |
| `tests/rom/rom_test.py` + `make_test_rom.py` | 407 + 266 lines | Module load, ROM load, predicted instruction stream, exact address set, BSR/RTS, jump-table dispatch, MCP load, RAM results, disassembly, Active Disassembly over a real data/jump table. |
| `tests/hierarchicalstorage/RoundTripTest.cpp` | 682 lines | XML tree round-trip with non-ASCII names, astral code points, reserved characters, binary inline/separate buffers. |
| `host/tests/PathSemanticsTest.cpp` | 133 lines | POSIX separators, relative/absolute, split/rebuild, drive-letter/UNC non-special-casing. |
| `tests/equivalence/compare_traces.py` | 174 lines | Strict prefix comparison, divergence point, length mismatch, tolerance, `--ignore-disassembly`. |

**Not covered:** Z80-specific tests, VDP golden frames/VRAM, audio golden samples, savestate round-trip, multi-instance, opcode conformance/flags, randomized differential fuzzing, TSAN/race detection, long-run determinism, input/region/TMSS matrix, MCP disconnect/backpressure, real commercial ROM analysis quality.

### 5.16 Multiple independent engine instances

**What the code suggests [inferred]:**

- Core state is per-object. A heuristic scan for mutable namespace-scope variables found none in the engine sources (`ExodusSDK`, `System`, `Devices`, `Extensions`, `Exodus`, `Support Libraries`); there are no singletons (`GetInstance`/`Singleton` searches found nothing), no `thread_local` state, and no static engine objects. The only file-scope statics are plugin allocator functions and sentinel assembly tokens.
- `System` owns its device/module/extension maps; `DeviceContext` owns its threads; each device owns its state; `HeadlessHost` owns its preferences and view manager; `Session` owns one `HeadlessHost` and one `System`.
- `GetSystem(IGUIExtensionInterface&)` / `DeleteSystem(...)` (`exodus/System/interface.cpp:5-13`) is a C++ factory that could create multiple `System` objects, each with its own host.

**Why it is still unproven:**

- No test constructs two `System`/`Session` objects in one process.
- Process-wide hazards: the MCP `Transport` claims stdout and redirects FD 1 (must be singular); Windows `waveOut` is a process-wide audio device (the POSIX backend is a null sink); `Debug/CrashReport` and assertion handling are process-wide; path preferences resolve against the current working directory at init; detached threads and device render/DMA threads multiply per instance; static library global constructors may exist in third-party code.
- Thread count and memory scale linearly per instance; a Mega Drive instance is already ~10 threads.
- The fork's own docs say “the server holds one emulated system at a time” (`AGENTS.md:22`), and the daemon has one `Session`.

**Practical conclusion:** treat multi-instance as **structurally plausible but unsupported and untested**. The safe bootstrap pattern is **process-per-instance**; a future in-process multi-instance design must add a two-instance smoke test, isolate stdout/logging/audio, and verify no static/global state leaks across instances.

---

## 6. Reusable patterns for ChronoFork

These are the ideas worth copying (as designs, not code):

1. **Declarative hardware graph.** Module XML/JSON with devices, buses, clocks, lines, connectors, dependencies, and settings. ChronoFork can describe test machines without compiling new C++.
2. **Narrow lifecycle contract.** `Construct → Build → Validate → Initialize` plus typed references and a device context. Error returns are booleans; failures are reported through an event log.
3. **Timed buffers as the concurrency primitive.** Timestamped writes plus committed/latest views let devices reconcile out-of-order accesses without a global lock. This is the most interesting design in the codebase.
4. **Optimistic timeslice + rollback + commit.** Run devices in parallel, detect conflicts, rewind to the earliest conflict, retry. It keeps the common case fast and the rare case correct.
5. **Explicit run/stop state with “state is readable only when stopped.”** The MCP layer turns a thread-safety hazard into a documented state machine and refuses reads while running rather than returning torn data.
6. **Reflection-based device introspection.** Pages → typed entries/commands addressed by stable paths. A new device becomes scriptable without a new RPC method.
7. **Execution-guided disassembly with observed vs predicted provenance.** The distinction is carried through the analysis and export schema; consumers can weigh evidence.
8. **Self-describing trace dumps + an independent comparator.** The comparator consumes files, so the reference side can be captured on another OS/toolchain. It stops at the first divergence rather than emitting consequential noise.
9. **Generated ROM fixtures with predicted behavior.** A checked-in generator produces a license-clean cartridge whose expected instruction stream and data/jump tables are known by construction. This is a better test asset than a commercial ROM.
10. **Schema-tagged JSON exports.** Flat address-sorted regions, numeric addresses with `_hex` companions, totals, and parse diagnostics. Diffable between runs.
11. **Static registries for headless builds.** Replace plugin discovery with explicit registration lists while keeping per-library boundaries; the Windows plugin path remains intact.
12. **Separate machine state from debugger state.** Breakpoints/traces/analysis survive or are excluded independently of the emulated machine.
13. **Path-abstracted host interface.** One `IGUIExtensionInterface` lets the same core run under a Win32 GUI, a CLI, and a daemon. A future ChronoFork host can implement the same seam.
14. **Negative controls in tests.** Corrupt a trace record and prove the gate fails; verify forged/deleted handles fail; pin known coverage gaps so they cannot silently expand.

---

## 7. Hard constraints (what ChronoFork must design around)

Ordered roughly by impact.

1. **No stable C ABI / no embedding library.** The engine is C++17 virtual interfaces with `std::wstring`, RTTI, static libraries, and a monolithic `System`. Only plugin metadata entry points are `extern "C"`. Any Rust integration needs a C++ shim and a pinned toolchain, or the process boundary.
2. **Multi-threaded scheduler with non-atomic flags.** One worker thread per active device, plus render/DMA/audio threads; `volatile` flags and dependency spin waits. Determinism is empirical and contention-sensitive. State reads are only safe while stopped.
3. **No user-facing rollback/replay.** Internal rollback is intra-timeslice conflict resolution. There is no input log, movie, or per-frame snapshot ring to build time travel on.
4. **Save states are full XML/ZIP snapshots**, with creation timestamps and no explicit format version. No MCP surface. Cross-build compatibility is unverified.
5. **Build requires clang + Ninja + libpng** on Linux; Windows needs MSVC + vcpkg. Not buildable in this workspace as provisioned. The fork is clang-only by policy, even where GCC might work.
6. **Initialization flag trap.** `System::Initialize` does not clear the module-load initialize flag; only `ExecuteDeviceStep` does. The MCP session burns the flag with a dummy device step. Any new host must handle this or writes will be silently reverted.
7. **Unconditional rollback logging to `std::wcout`.** Every rollback formats and prints; the daemon redirects FD 1 to stderr. A Rust host must redirect or patch logging, and rollback-heavy workloads can be noisy/slow.
8. **Instruction-level, not bus-cycle-level, CPU cores.** The 68000 executes opcodes indivisibly and lacks prefetch-pipeline modeling; the Z80 lacks modes 0/2, port I/O, and many ED opcodes. Not a hardware-conformance oracle.
9. **VDP/audio accuracy limits.** VDP lacks several documented features; YM2612 rendering is time-step, not cycle-accurate; PSG filtering is not implemented. Not a bit-exact multimedia oracle.
10. **Active Disassembly is 68000-only, coverage-limited, and private.** Its JSON export is built by parsing the SDK's text export. It is excellent for executed-path analysis, not a complete static disassembler.
11. **MCP gaps:** no save/load state, no controller input, no module settings (region/TMSS/soft reset), no screenshot, no audio capture, no multi-session, no async notifications, blocking steps.
12. **Frame pacing is hard-coded to NTSC** in both MCP and probe; PAL-region runs still advance by the NTSC frame constant.
13. **Single-session daemon.** One `System` per process; multi-instance is untested and has process-wide hazards.
14. **Diagnostics own stdout.** Device/core code writes to `std::wcout`; the daemon must claim stdout before constructing the core. Any embedding must do the same.
15. **Path/encoding complexity.** POSIX path semantics, UTF-8/UTF-16/UTF-32 conversions, wide filenames, ZIP-embedded files (`archive.zip|rom.bin`), and module-relative paths. The fork has tests, but the surface is large.
16. **License hygiene.** MIT is permissive, but third-party binaries and ROM/BIOS data have separate obligations. No ROMs ship; tests generate their own.

---

## 8. Backend-strategy implications

### 8.1 Options

| Option | What it means | Pros | Cons | Bootstrap fit |
|---|---|---|---|---|
| **A. External oracle** | Rust orchestrator spawns `exodus-mcpd` or `exodus-probe`; talks JSON-RPC / reads trace files | No ABI risk; uses the fork's tested surface; process isolation; easy to pin a commit | Per-call process/stdio overhead; no snapshot/input; one session per process; must parse errors/traces | **Best now** |
| **B. In-process C++ shim** | C++ wrapper around `HeadlessHost` + `System` + registries, exposed as a C ABI to Rust | Access to savestates, full debugger, device reflection, multiple instances (if proven); lower latency | Must own C++ toolchain, ABI, threading, logging, init-flag workaround; maintenance of a fork; multi-instance unproven | Spike only |
| **C. Rust port/rewrite** | Reimplement 68000/Z80/VDP/audio in Rust | Full control of determinism, snapshots, ABI | Very high cost and risk; would need its own oracle | Too large for bootstrap |
| **D. Hybrid** | Rust owns scheduler/state/replay; Exodus supplies golden traces, fixture ROMs, disassembly graphs; optional A shim for queries | Determinism and API are ChronoFork's; Exodus is a reference, not a dependency | Requires owning the core; integration is test-time only | **Recommended target** |

### 8.2 Recommendation

For the bootstrap, adopt **D with A**:

- Use `exodus-probe`/`exodus-mcpd` as an **external reference** to generate golden traces and disassembly graphs, and to answer “what does Exodus do here?” questions.
- Build ChronoFork's backend around its own deterministic scheduler/state model. Do not inherit Exodus's thread/rollback design as the product's execution model.
- Keep the C++ shim (B) behind a feature flag as a research spike, not as the production path. If it is pursued, wrap only `HeadlessHost` + `System` + the two static registries, expose a minimal opaque-handle C API, and serialize all access.
- Treat the MCP daemon as a **test harness** and **oracle**, not as the ChronoFork runtime.

### 8.3 If ChronoFork embeds the C++ core, it must own

1. **Instance lifecycle:** one host + one system per instance; no shared `Transport`; process-per-instance fallback.
2. **Determinism:** a single-threaded or explicitly ordered scheduler path if possible; otherwise a deterministic rollback tie-break and a stress test across CPU counts/runs.
3. **Snapshot format:** ChronoFork's own versioned state hash and snapshot envelope; use Exodus savestates only as an opaque reference or convert them.
4. **Input and replay:** record normalized input events; Exodus has no movie/replay format.
5. **Error surface:** boolean returns + event log, no exceptions; map event logs into structured errors.
6. **Logging:** redirect/patch `std::wcout`; suppress rollback spam; never let core diagnostics reach a protocol channel.
7. **Initialization:** consume the `_initialize` flag deterministically after module/ROM load.
8. **Multi-instance isolation:** test two instances; isolate stdout, audio, crash handling, and CWD-relative paths.

### 8.4 Implications for the deterministic test backend and acceptance tests

- Use Exodus to generate **golden 68000 traces** and **Active Disassembly graphs** for a small set of generated ROMs; compare ChronoFork's outputs at the instruction/address level.
- Build **fixture ROMs from source** (the fork's generator is a good model): reset vectors, a known instruction stream, a data table, a relative jump table, a subroutine, a RAM result word, and a deliberate trap handler.
- Add a **negative control** to every differential gate (corrupt one record; assert failure at the right point).
- Test **state hashing** excluding timestamps and other non-semantic fields; round-trip save/load; cross-build stability.
- Test **determinism** across repeated runs, different CPU counts/affinities, and thread sanitizer builds.
- Test **MCP-like surfaces** for argument validation, opaque handles, stale/forged handles, run/stop gating, and error-vs-protocol separation.
- Keep **license/ROM hygiene** tests: no ROM/BIOS blobs, generated fixtures only, preserved notices.

---

## 9. Testing strategy assessment (condensed)

**Strong existing ideas worth adopting:**

- Independent comparator (file-based, not linked into the emulator).
- First-divergence reporting with context; stop rather than cascade.
- Self-describing dumps with comments and explicit field rules.
- Generated ROM with predicted instruction stream and exact address set.
- Cross-checking JSON export counts against the SDK's own counters.
- Pinning known coverage gaps so they fail when coverage changes.
- Opaque handles with stale/forged-handle tests.
- Run/stop gating tests that assert refusal, not silent behavior.

**Gaps ChronoFork should close:**

| Gap | Why it matters | Suggested test |
|---|---|---|
| No Z80 tests | Sound CPU is a large untested surface | Z80 trace + register/flag conformance ROMs; interrupt modes 1 (implemented) and bus requests |
| No VDP golden state | Rendering/register behavior unverified | Frame hash/VRAM/CRAM dumps for deterministic test ROMs; H/V counter and DMA tests |
| No audio golden samples | YM2612/PSG accuracy unknown | Deterministic register-write sequences + PCM hash/tolerance |
| No savestate round-trip | Snapshot compatibility unknown | Save→load→step→trace equality; cross-build load |
| No multi-instance test | Parallel exploration depends on it | Two `System`s in one process; isolated traces; thread/CPU stress |
| No determinism-under-load test | Parallel rollback is the core risk | N repeated runs, CPU affinity/count variation, TSAN |
| No opcode conformance suite | CPU flags/timing are the foundation | Generated opcode ROMs + expected CCR/cycles; randomized differential vs another core |
| No real-ROM active-disassembly evaluation | Synthetic ROM is friendly | A license-clean homebrew ROM with indirect jumps, compressed data, multiple tables |
| No MCP fuzzing | Hand-rolled JSON parser is a trust boundary | Property/fuzz tests for parse/serialize; malformed batch/ID/UTF-8 |
| No region/TMSS matrix | Boot behavior varies | Run the same ROM under U/E/J and TMSS on/off |
| No performance budget | Headless analysis can be slow | Frame-time benchmarks, rollback counters, thread counts |

---

## Appendix A — checks actually performed here [executed]

### A.1 Repository and provenance

```text
exodus:          HEAD 08f388f77040af28d16d44fdfbddb73252953161  master
exodus-headless: HEAD ec0056151f3ff1c334d09ee375abb30a68fdfec5  linux-headless
upstream tracked files: 1439
identical in fork:      1374
differing:              65
missing in fork:        0
fork commit count:      1
```

`git status --porcelain` was empty in both research trees before and after inspection.

### A.2 Toolchain probe

```text
cmake 3.28.3
g++ present
pkg-config present
python3 present
ninja: MISSING
clang++: MISSING
expat 2.6.1, zlib 1.3, libjpeg 2.1.5
libpng: MISSING
```

### A.3 ROM generator determinism

Ran `python3 tests/rom/make_test_rom.py` twice into a temp directory:

```text
8626f7fe3d4e33823b300bb75d3e94d72c5163087b5e5eb6adef34d3fd9b25fc  a.bin
8626f7fe3d4e33823b300bb75d3e94d72c5163087b5e5eb6adef34d3fd9b25fc  b.bin
size 32768
ssp 00FFFE00 pc 00000200
header b'SEGA MEGA DRIVE '
region b'U'
jump table [384, 416, 448, 480]   # 0x180,0x1A0,0x1C0,0x1E0 relative to 0x280
data table first 4 [1, 2, 3, 4]
```

### A.4 Comparator accept/reject

Synthetic two-record trace dumps:

```text
PASSED: 2 records identical (cycle, address, disassembly, time within 1e-09)
same_exit=0

EXECUTION DIVERGED at record 2
  reference: cycle=102 address=0x204 LEA $00000280, A0
  candidate: cycle=102 address=0x206 LEA $00000280, A0
  last 1 matching records before the divergence:
    cycle=100 address=0x200 MOVE #$2700, SR
FAILED: 1 problem(s) across 2 reference records
bad_exit=1
```

This confirms the comparator's accept/reject behavior and first-divergence reporting, but not the fork's cross-compiler claim.

### A.5 What was *not* executed

- No CMake configure/build (missing clang/Ninja/libpng; the fork's scripts also default to in-tree build directories, and the research tree is read-only for this task).
- No `exodus-probe` or `exodus-mcpd` invocation.
- No `mcp/tests/session_test.py`, `tests/rom/rom_test.py`, `tests/hierarchicalstorage`, or `host/tests` run (all require built binaries or would create build directories inside the research tree).
- No Windows/MSVC reproduction.
- No multi-instance or TSAN experiment.

---

## Appendix B — key files for follow-up reading

| Topic | Files |
|---|---|
| Device/execution contract | `exodus/ExodusSDK/DeviceInterface/IDevice.h`, `IDevice.inl`, `System/DeviceContext.h`, `System/ExecutionManager.h` |
| Scheduler | `exodus/System/ExecutionManager.inl`, `System/System.cpp:2227-2502`, `System/DeviceContext.cpp` |
| Bus/clock/line model | `exodus/System/BusInterface.cpp`, `System/ClockSource.h`, `ExodusSDK/DeviceInterface/IBusInterface.h` |
| Module graph | `exodus/Data/Modules/Sega Mega Drive 1600.xml` |
| 68000 | `exodus/Devices/M68000/M68000.{h,cpp}`, `M68000Instruction.*`, `ExecuteTime.*`, `EffectiveAddress.*` |
| Z80 | `exodus/Devices/Z80/Z80.{h,cpp}`, `Z80Instruction.*` |
| VDP | `exodus/Devices/315-5313/S315-5313_{General,Ports,Rendering,Timing}.cpp` |
| Audio | `exodus/Devices/YM2612/YM2612.cpp`, `Devices/SN76489/SN76489.cpp`, `Support Libraries/AudioStream/*` |
| Save states | `exodus/System/System.cpp:74-470`, `Support Libraries/HierarchicalStorage/*` |
| Debugger | `exodus/ExodusSDK/Processor/Processor.cpp:500-800`, `Breakpoint.*`, `Watchpoint.*` |
| Active Disassembly | `exodus/ExodusSDK/Processor/Processor.cpp:1675-2200,2519-3200,3605-4120`, `mcp/DisassemblyGraph.*` |
| Reflection | `exodus/ExodusSDK/GenericAccess/*`, `mcp/GenericAccessBridge.*` |
| Headless host | `exodus-headless/host/*` |
| MCP | `exodus-headless/mcp/*` |
| Tests | `exodus-headless/tests/*`, `exodus-headless/mcp/tests/*`, `exodus-headless/host/tests/*` |
| Port reports | `exodus-headless/_PortReports/*.md` |

---

## 10. Evidence table

Confidence: **H** = directly inspected in source or executed here; **M** = inferred from multiple source facts; **L** = fork-reported or unverified.

| # | Claim | Evidence | Confidence |
|---:|---|---|---|
| 1 | Upstream is `RogerSanders/Exodus` `master` at `08f388f77040af28d16d44fdfbddb73252953161`, 2024-02-16 | `git remote -v`, `git rev-parse HEAD`, `git log -1` | H |
| 2 | Fork is `paulomanrique/exodus-for-ais` `linux-headless` at `ec0056151f3ff1c334d09ee375abb30a68fdfec5`, 2026-08-13 | same commands | H |
| 3 | Fork history is a single squashed commit | `git rev-list --count HEAD` = 1; upstream object absent | H |
| 4 | 1,374 of 1,439 upstream tracked files are byte-identical in the fork; 65 differ; none missing | per-file byte comparison | H |
| 5 | Fork is an adapter/portability patchset, not an emulation rewrite | `Processor.cpp` 0 changed lines; M68000/Z80 cores identical; System.cpp 3 lines; DeviceContext only `Sleep(0)`→`yield`; VDP only literal lexing | H |
| 6 | License is MIT © 2024 Roger Sanders; fork preserves it unchanged | `exodus/License.txt:1-3`; `diff` identical | H |
| 7 | No third-party source is vendored; Linux uses system expat/zlib/libpng/libjpeg, Windows uses vcpkg | `Third/` contains only build metadata; `CMakeLists.txt:47-53`; `vcpkg.json` | H |
| 8 | No ROMs/BIOS ship; tests generate a 32 KiB cartridge | tree scan; `tests/rom/make_test_rom.py` | H |
| 9 | Device contract is `IDevice` with lifecycle, execution, bus/line/clock, reflection, serialization | `ExodusSDK/DeviceInterface/IDevice.h:31-159` | H |
| 10 | Mega Drive graph is declared in a 447-line XML module with devices/buses/clocks/lines/connectors/settings/dependencies | `Data/Modules/Sega Mega Drive 1600.xml` | H |
| 11 | Active devices run in parallel per timeslice; scheduler picks earliest timing point; rollback on conflict; commit at end | `ExecutionManager.inl:173-198,259-286`; `System.cpp:2253-2372`; `System.cpp:2457-2502` | H |
| 12 | `Step` devices return no external timing point; VDP returns next VSYNC | `Device.cpp:172-175`; `S315-5313_General.cpp:1027-1071` | H |
| 13 | Breakpoints stop via rollback to the current instruction, then `FlagStopSystem` | `Processor.cpp:561-605`; `System.cpp:2457-2502` | H |
| 14 | The 68000 is instruction-level, documented as lacking prefetch/exact bus timing | `M68000.h:1-25`; `M68000.cpp:643-1027` | H |
| 15 | The 68000 uses an 84-class, 16-bit decode table and placement-clone allocation | `M68000.cpp:71-275,966`; `M68000.h:311` | H |
| 16 | The Z80 has 37 classes across three 8-bit tables and documented missing modes/I/O | `Z80.cpp:9,516`; `Z80.h:1-25` | H |
| 17 | The VDP uses timed buffers, a render thread, and a DMA thread; has documented accuracy gaps | `S315-5313_General.cpp:737-1071`, `:527-548`; `S315_5313.h:1-20`; `Rendering.cpp:149` | H |
| 18 | YM2612/PSG render at 48 kHz to `AudioStream`; POSIX backend discards buffers | `YM2612.cpp:686,876,1454`; `SN76489.cpp:290,396,537`; `AudioStream.cpp:571-675` | H |
| 19 | Savestates are XML or ZIP full snapshots with creation timestamps and optional screenshot; debugger state is separate | `System.cpp:74-140,351-470`; `ISystemGUIInterface.inl:13-17`; `Processor.cpp:4329-4621` | H |
| 20 | Internal rollback is not user-facing replay; no movie/input log exists | rollback code is intra-timeslice; grep found no replay/movie/scripting | H |
| 21 | Active Disassembly is execution-guided, 68000-only, and exports ASM/text/IDC/JSON | `Processor.cpp:1675-1873,1978-2200,3605-4120`; `M68000.cpp:2381`; `DisassemblyGraph.h` | H |
| 22 | The JSON graph is built by parsing the SDK text export, not reading the model | `mcp/DisassemblyGraph.h:40-100`; `DisassemblyGraph.cpp:134-220` | H |
| 23 | MCP exposes exactly 35 tools across 8 families | `grep registry.Add` count and names in `mcp/Tools*.cpp` | H |
| 24 | MCP has no save-state, input, module-setting, or screenshot tool | full tool-name enumeration; `ISystemGUIInterface.h:65-71`; `System.h:195-196` | H |
| 25 | The daemon owns stdout before constructing the core and redirects FD 1 to stderr | `Transport.cpp:41-59`; `Main.cpp` comment | H |
| 26 | `run.step_frames` is synchronous, capped at 100,000, and uses a fixed NTSC frame length | `Session.cpp:19-27,289-311` | H |
| 27 | MCP session test has 1,010 lines and 186 assertion call sites; ROM test predicts an exact trace and jump-table targets | `mcp/tests/session_test.py`; `tests/rom/rom_test.py:124-200,304-390` | H |
| 28 | The test ROM generator is deterministic and the comparator rejects divergence | executed twice; same SHA-256; synthetic comparator exit 0/1 | H |
| 29 | Cross-compiler 18,941-record equivalence is real | fork README claims same SHA-256; **not reproduced here** | L |
| 30 | Multi-instance is structurally plausible but untested | no globals/singletons found; one `Session`/`System` in daemon; no two-instance test | M |
| 31 | The build cannot be reproduced in this workspace | `clang++`, `ninja`, `libpng` missing; CMake hard-requires clang/Ninja/libpng | H |
| 32 | Determinism under contention is not guaranteed by construction | parallel threads; `volatile` flags; rollback tie-break; render-thread liveness FIXME | M |
| 33 | Region/TMSS/soft-reset cannot be set over MCP | no tool; settings API exists only in C++ | H |
| 34 | PAL frame stepping is inconsistent with the fixed NTSC frame constant | `Session.cpp:19` vs module Region setting | M |
| 35 | The fork's portability fixes are localized and plausible | diff of 65 files; `_PortReports/*.md`; four latent-bug descriptions | H/M |

---

## 11. Open questions

1. **Exact upstream base.** Can the fork author confirm the upstream commit/tag the snapshot was taken from? Content identity is strong but not a cryptographic ancestry proof.
2. **Build reproducibility.** Does the fork build cleanly from a fresh Linux environment with only the documented dependencies? Which clang version is the reference (the workspace lacks it)?
3. **Determinism envelope.** Do repeated runs remain byte-identical under CPU-count changes, CPU affinity, load, and TSAN? Does the gate hold for ROMs that exercise VDP DMA, Z80 interrupts, and audio?
4. **Race safety.** Are the `volatile` flags and render-thread lag waits actually safe, or do they rely on x86 strong ordering? The VDP render-thread FIXME suggests known liveness bugs.
5. **Multi-instance.** Can two `System` objects coexist in one process without shared-state interference (stdout, audio, crash handler, CWD)? Is a two-instance test planned?
6. **Savestate compatibility.** Are savestates portable across compilers/OSes/builds? Is the XML/ZIP format versioned? Can a saved state survive a module change?
7. **PAL frame stepping.** Should `run.step_frames` derive frame length from the active region/settings rather than the NTSC constant? What does the fork intend?
8. **Module settings over MCP.** Is there a reason region/TMSS/soft reset are not exposed, or is it simply unimplemented? ChronoFork test ROMs will need region control.
9. **Input recording.** Is there an unreleased branch with input/movie support, or would a new host need to record `HandleInputKeyDown/Up` events itself?
10. **Real-ROM Active Disassembly quality.** How well do jump-table/array/label heuristics perform on commercial code with compression, pointer tables, and self-modifying code? Only a synthetic ROM is tested here.
11. **JSON graph fragility.** How often does the text-export parser skip lines or misclassify tables on large analyses? The parser reports counts, but no corpus test exists.
12. **Performance.** What is the headless speed (× realtime) for a real ROM with rendering/audio enabled vs `Disable Rendering`? No benchmark is in the tree.
13. **Embedding seam.** Was `GetSystem(IGUIExtensionInterface&)`/`DeleteSystem` intended as a stable factory? Is a C ABI acceptable upstream, or must ChronoFork maintain a shim fork?
14. **Third-party distribution.** Which exact licenses/notices must ship with static Windows binaries (vcpkg) and dynamically linked Linux binaries?
15. **MCP robustness.** How does the daemon behave on client disconnect mid-response, oversized messages, very large batches, or a step that exceeds the client timeout?
16. **macOS/ARM.** Do the `wchar_t`/endianness/path assumptions hold on macOS and ARM64? No evidence in the tree.

---
