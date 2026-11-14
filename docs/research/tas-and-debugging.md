# BizHawk TAS, Debugging, and Tracing Architecture

> Research note for the ChronoFork bootstrap. This is a pinned-source assessment of
> `TASEmulators/BizHawk`, not a specification for ChronoFork and not legal advice.
> No upstream source was copied; short quotations are used only where licensing or
> interface semantics require exact wording.

## Snapshot

| Field | Value |
| --- | --- |
| Repository | `TASEmulators/BizHawk` |
| Canonical URL | <https://github.com/TASEmulators/BizHawk> |
| Reviewed commit | `d148601a4629c2c230e41647d7c94eee794011c7` |
| Commit URL | <https://github.com/TASEmulators/BizHawk/tree/d148601a4629c2c230e41647d7c94eee794011c7> |
| Branch at review | `master` |
| Commit date | `2026-11-10T17:07:36-06:00` |
| Commit subject | `fix memory leak` |
| Tags at commit | none |
| Local clone | `.research/upstream/bizhawk` |
| Worktree state | clean (`git status --porcelain` empty) |
| Submodule state | 32 entries in `.gitmodules`; all uninitialized in this clone (`git submodule status` showed `-` prefixes) |
| Method | static source inspection, git metadata, grep/file counts; no upstream build or runtime execution |
| Scope | architecture, TAS/movie model, save states, frame control, input, Lua, callbacks, debugger, tracing, capabilities, tests, licensing boundaries |

**Evidence labels used below**

- **`[V]` verified**: read directly in the pinned tree. A citation like
  `src/…/File.cs:12-34` is relative to `.research/upstream/bizhawk/`.
- **`[T]` tree observation**: based on presence/absence of files or on
  `git`/`grep` output, not on executed code.
- **`[U]` unverified**: plausible but not checked here, or impossible to check
  because a submodule is absent. Reason is stated.
- **`[I]` inferred**: a conclusion drawn from several verified facts; not a
  single-line claim in the source.

---

## 1. Executive summary

1. **The transferable core is small and service-oriented.** BizHawk defines a
   minimal `IEmulator` (load identity, frame advance, frame counter, system ID,
   determinism flag) and pushes everything optional — save states, memory
   domains, debugger, tracing, disassembly, input polling, SaveRAM, video,
   audio — into optional `IEmulatorService` interfaces. Clients discover
   capabilities at runtime through `IEmulatorServiceProvider`
   (`src/BizHawk.Emulation.Common/Interfaces/IEmulator.cs:10-66`,
   `.../IEmulatorService.cs:8-20`). This is the most reusable architectural idea
   in the repository. `[V]`

2. **Capabilities are runtime types, not a declared contract.** There is no
   load-time capability descriptor. `CoreAttribute` carries name/author/flags,
   and `ServiceNotApplicableAttribute` / `FeatureNotImplementedAttribute` only
   annotate intent; the client probes for services and catches
   `NotImplementedException`. The “Core Features” tool then guesses method
   completeness by reflection (`CoreFeatureAnalysis.cs:16-91`). `[V]`

3. **Movie recording is an input-log + state-anchor model, not a general
   emulator script.** A movie is a header, an input log, optional savestate or
   SaveRAM anchor, comments/subtitles, and sync settings; `.tasproj` adds a
   greenzone (state cache), lag log, markers, branches, and undo history.
   `[V]` (`src/BizHawk.Client.Common/movie/interfaces/IMovie.cs:34-208`,
   `.../tasproj/TasMovie.IO.cs:12-49`)

4. **“Rerecording” is an emergent property of input-log truncation plus
   savestate invalidation.** There is no separate rerecord emulation mode.
   Rerecord counts increment when states are invalidated
   (`TasMovie.cs:120-144`) or when loading a state while a movie is attached
   (`Bk2Movie.InputLog.cs:97-100`), and recording-while-rewinding truncates the
   movie (`MainForm.cs:3021-3026`). `[V]`

5. **Frame stepping is frame-granular; instruction stepping is rare and
   partly disabled.** The run loop pauses between frames
   (`MainForm.cs:2845-3100`), and `IDebuggable.Step` exists only when a core
   implements it. In the sampled tree, NES, PSX (Octoshock), QuickNES, SNES
   (libsnes), and link cores report `CanStep == false`; N64 reports false with
   the comment that stepping is “implemented but disabled” pending mid-frame
   pausing (`N64.IDebuggable.cs:79-92`). BSNES and Apple II are counterexamples
   that do implement it. `[V]`

6. **Seeking is replay-based.** TAStudio’s `GoToFrame` loads the nearest
   cached state then lets the emulator advance to the target; backward seeks load
   a state, forward seeks replay (`TAStudio.Navigation.cs:7-58`,
   `MainForm.cs:3055-3062`). There is no arbitrary random-access seek API. `[V]`

7. **Lua is a client-side host layer, not an emulator contract.** Scripts run
   as NLua coroutines; `emu.frameadvance()` cooperatively yields to the host
   frame loop, and event libraries are registered only when their required
   emulator services exist (`LuaLibraries.cs:42-128`,
   `EventsLuaLibrary.cs:80-106`). The “sandbox” only sets the current directory
   and catches exceptions; it is not a security boundary
   (`LuaSandbox.cs:44-76`, `EnvironmentSandbox.cs:4-11`). `[V]`

8. **Memory callbacks are a per-core instrumentation hook, not a generic bus.**
   `IMemoryCallbackSystem` supports read/write/execute, address masks, named
   scopes, and even value override from a callback; the implementation warns
   cores to keep the empty-list path extremely fast
   (`IMemoryCallbackSystem.cs:18-85`, `MemoryCallbackSystem.cs:69-113`). The
   debugger implements breakpoints as memory callbacks. `[V]`

9. **Tracing is the weakest abstraction.** `ITraceable` exposes a single sink
   and stringly typed `TraceInfo` rows (`Disassembly`, `RegisterInfo`).
   `TraceBuffer` is a trivial holder; `CallbackBasedTraceBuffer` is a fallback
   built from memory callbacks + disassembly and explicitly uses only the first
   available callback scope (`src/BizHawk.Emulation.Common/Interfaces/Services/ITraceable.cs:3-38`,
   `CallbackBasedTraceBuffer.cs:5-17,47-63`). `[V]`

10. **Licensing prevents code reuse.** The repository licence says the
    BizHawk team’s original C# work is MIT, while the repository as a whole is a
    heterogeneous mix with “an illegal combination of several incompatible GPL
    licenses” (`LICENSE:1-7`). The README adds that “anything outside `/src`
    isn’t ours” (`README.md:469-471`). Several in-tree vendored components are
    GPLv2, LGPL, non-commercial, or MIT; 32 submodules are uninitialized and
    could not be inspected locally. For ChronoFork the safe course is
    **patterns and interfaces, not code**. `[V]`/`[U]`

11. **The test infrastructure is strong for library logic and weak for core
    conformance.** `dotnet test` runs three MSTest projects in the main solution,
    with fakes for the emulator, movie session, and state manager, plus extensive
    movie/undo/state-manager/Lua/callback tests. Test-ROM screenshot suites exist
    but are outside the main solution and CI, require prebuilt native cores and
    ROM/firmware assets, and therefore are not a generic backend acceptance
    suite (`src/BizHawk.Tests.*`, `.github/workflows/ci.yml:27-69`,
    `BizHawk.sln:38-42`). `[V]`/`[T]`

---

## 2. Repository, provenance, and licensing boundaries

### 2.1 What the pinned tree contains

The pinned tree is organized around a C# solution plus native/core source
directories and research submodules:

- `src/BizHawk.Emulation.Common` — core contracts and generic implementations.
- `src/BizHawk.Emulation.Cores` — core wrappers, CPUs, system-specific code.
- `src/BizHawk.Client.Common` — movies, TAStudio infrastructure, save states,
  input chain, Lua, external APIs, config.
- `src/BizHawk.Client.EmuHawk` — WinForms frontend, tools, debugger, trace
  logger, TAStudio UI.
- `src/BizHawk.Tests.*` — MSTest unit tests and test-ROM harnesses.
- Native/vendored trees at repository root: `waterbox/`, `libmupen64plus/`,
  `quicknes/`, `lynx/`, `psx/`, `wonderswan/`, `encore/`, `libHawk/`,
  `blip_buf/`, `ExternalProjects/`, `ExternalCoreProjects/`, `References/`,
  `Assets/dll/`.
- 32 git submodules (`.gitmodules:1-111`).

`[V]`/`[T]`

### 2.2 Original BizHawk-team code vs. third-party components

The root `LICENSE` is explicit about provenance (`LICENSE:1-7`):

- original work “chiefly in c# by the BizHawk team” is MIT;
- embedded submodules from other authors have their own licences, “clearly
  provided” in some cases and absent in others;
- BizHawk’s customizations to those submodules are MIT;
- compiled binary modules from other authors are included **without clear
  licences or origins**;
- the repository “as-is should be considered an illegal combination of several
  incompatible GPL licenses”;
- removing many C/C++ cores would leave “an almost pristine MIT-licensed work”.

The README gives the practical boundary for reusers: “anything outside `/src`
isn’t ours and we can’t give you permission to share, use, or sell it”
(`README.md:469-471`). Note the wording: `/src` is the team’s *frontend*
code; `/src/BizHawk.Emulation.Cores` contains C# wrappers around ported
upstream emulators, marked with `PortedCoreAttribute`
(`src/BizHawk.Emulation.Common/CoreAttribute.cs:23-42`). Those wrappers may be MIT as team-authored
code while the emulation logic they bind to is not.

Concrete provenance/licence evidence found in the tree:

| Component / path | Origin signal | Licence evidence | Status |
| --- | --- | --- | --- |
| `src/BizHawk.*` (C# frontend, common, core wrappers) | BizHawk team’s original work; ported wrappers use `PortedCoreAttribute` | Root `LICENSE:9-20` (MIT text for team’s original work); `README.md:469-471` | **MIT claim verified in text**, per-file provenance not audited |
| `src/BizHawk.Emulation.Cores/Consoles/Sony/PSX/Nymashock.cs` | Ported Mednafen core | `Nymashock.cs:11` `[PortedCore(CoreNames.Nymashock, "Mednafen Team", "1.32.1", …)]` | Wrapper origin verified; upstream licence not in tree `[U]` |
| `src/BizHawk.Emulation.Cores/Arcades/MAME/MAME.cs` | Ported MAME | `MAME.cs:19` `[PortedCore(CoreNames.MAME, "MAMEDev", "0.252", …)]` | Wrapper origin verified; MAME licence not in tree `[U]` |
| `src/BizHawk.Emulation.Cores/Consoles/Nintendo/SNES9X/Snes9x.cs` | Ported Snes9x | `Snes9x.cs:11` ported-core declaration | Wrapper origin verified; upstream licence not in tree `[U]` |
| `blip_buf/` | Third-party audio resampler | `blip_buf/license.txt:1-3` GNU LGPL v2.1 | Verified |
| `lynx/` | Handy/Lynx emulator source | `lynx/license.txt:1-14` zlib-style permissive (“as-is”, attribution, altered versions marked) | Verified |
| `waterbox/picodrive/` | PicoDrive | `waterbox/picodrive/COPYING:1-8` non-commercial redistribution terms | Verified |
| `waterbox/tic80/` | TIC-80 | `waterbox/tic80/LICENSE:1-3` MIT | Verified |
| `libmupen64plus/mupen64plus-core/LICENSES` | Mupen64Plus core | `LICENSES:1-4` GPLv2 | Verified |
| `libmupen64plus/mupen64plus-rsp-hle/LICENSES` | Mupen64Plus RSP HLE | `LICENSES:1-4` GPLv2 | Verified |
| `libmupen64plus/mupen64plus-video-rice/LICENSES` | Mupen64Plus video plugin | `LICENSES:1-4` GPLv2 | Verified |
| `libmupen64plus/mupen64plus-video-glide64mk2/LICENSES` | Glide64mk2 | `LICENSES:1-4` GPLv2 | Verified |
| `libmupen64plus/mupen64plus-video-glide64/COPYING` | Glide64 | `COPYING:1-3` GPLv2 | Verified |
| `ExternalProjects/NLua/LICENSE` | NLua fork | MIT text | Verified |
| `ExternalProjects/iso-parser/LICENSE.md` | ISO parser | MIT text | Verified |
| 32 submodules (`waterbox/snes9x`, `submodules/mgba`, `waterbox/melon/melonDS`, `waterbox/nyma/mednafen`, `waterbox/mame-arcade/mame`, `submodules/gambatte`, `submodules/sameboy/libsameboy`, `waterbox/stella/core`, `waterbox/dosbox/dosbox-x`, `ExternalCoreProjects/ppsspp/…`, `submodules/libdarm`, `submodules/libfwunpack`, `submodules/libemu83`, `submodules/libre-roms-ci`, `waterbox/musl`, `ExternalProjects/SDL2/SDL`, `ExternalProjects/librcheevos/rcheevos`, `encore/encore`, …) | Upstream projects, mostly TASEmulators forks | `.gitmodules:1-111` only | **Uninitialized locally; licences not inspected** `[U]` |
| `Assets/dll/`, `References/` | Prebuilt native/managed binaries, including emulator cores and NLua | Root `LICENSE:1` admits compiled binaries without clear licences/origins | Do not redistribute or treat as source `[V]` |

The submodule count is 32 `[submodule "…"]` entries in a 111-line
`.gitmodules` file. This is useful as a caution: the repository is not a single
licence even though it contains a large MIT-licensed C# frontend. `[T]`

### 2.3 What this means for ChronoFork

- **Do not copy BizHawk source, generated assets, native binaries, or test ROMs
  into ChronoFork.** Even where the file is MIT, provenance of a specific file
  must be verified; the root licence explicitly calls the whole a “minefield”.
  `[V]`
- **Patterns are safe to study and re-express independently**: minimal core
  contract, optional capability interfaces, service discovery, movie/state
  split, greenzone invalidation, callback scopes, trace sink. These are
  architectural ideas, not code. `[I]`
- **Do not link GPL/non-commercial cores into a permissively licensed product**
  without a licensing decision. In-tree evidence alone includes GPLv2
  (Mupen64Plus components, Glide64), LGPL (blip_buf), non-commercial
  (PicoDrive), and unknown submodule licences. `[V]`/`[U]`
- **The MIT claim is a useful citation for frontend architecture, not a blanket
  licence for the emulator collection.** `[I]`

### 2.4 Unverified licensing claims

- Exact licences of all 32 submodules. They were absent from the clone, so
  their `LICENSE`/`COPYING` files could not be read. `[U]`
- Which submodule commits correspond to the prebuilt binaries in
  `Assets/dll/`/`References/`, and whether those binaries carry notices. `[U]`
- Whether every file under `/src` is BizHawk-authored or contains copied
  third-party code beyond the declared `PortedCoreAttribute` markers. `[U]`
- Whether the project’s self-description (“illegal combination”) has been
  tested by anyone; this is a statement in the repository, not a legal
  conclusion. `[U]`

---

## 3. Multi-system and core abstraction

### 3.1 The minimal emulator contract

`IEmulator` is deliberately small (`IEmulator.cs:5-66`):

- `IEmulatorServiceProvider ServiceProvider`
- `ControllerDefinition ControllerDefinition`
- `bool FrameAdvance(IController controller, bool render, bool renderSound = true)`
- `int Frame`
- `string SystemId`
- `bool DeterministicEmulation`
- `void ResetCounters()`
- `IDisposable`

Important semantics from the comments:

- `FrameAdvance` runs “1 frame” and returns whether a new frame was produced;
  cores expect `SoundProvider.GetSamples()` after each advance, even when
  `renderSound == false` (`IEmulator.cs:27-38`).
- `Frame` must be incremented after emulation (`IEmulator.cs:40-44`).
- `DeterministicEmulation` is “a contract with the client”, but the client is
  responsible for managing the flag; a core may use non-deterministic
  speed hacks only when it is false (`IEmulator.cs:51-60`).
- `ResetCounters` resets frame/lag counters (`IEmulator.cs:62-65`).

This is a clean, backend-agnostic shape. A generic backend can map it to
“step one quantum, report the new tick, identify the machine, declare
determinism”.

### 3.2 Optional services and discovery

Optional capabilities are separate interfaces, all deriving from
`IEmulatorService` (`IEmulatorService.cs:3-20`). The shipped service set is:

`IBoardInfo`, `ICodeDataLogger`, `ICreateGameDBEntries`, `ICycleTiming`,
`IDebuggable`, `IDisassemblable`, `IDriveLight`, `IGLTextureProvider`,
`IInputPollable`, `ILinkable`, `IMemoryDomains`, `IRegionable`, `IRomInfo`,
`ISaveRam`, `ISettable`, `ISoundProvider`, `IStatable`, `ITraceable`,
`IVideoOffset`, `IVideoProvider`
(`src/BizHawk.Emulation.Common/Interfaces/Services/`). `[T]`

`IEmulatorServiceProvider` gives `HasService`, `GetService`, and
`AvailableServices` (`IEmulatorServiceProvider.cs:14-44`).
`BasicServiceProvider` is the standard implementation:

- auto-registers every `IEmulatorService` interface implemented by the core
  object, plus the concrete type and its base types;
- allows explicit `Register<T>` and `Unregister<T>`;
- returns null for services it does not have
  (`BasicServiceProvider.cs:14-74`).

`Unregister<T>` is not theoretical: the libretro core removes `IStatable` at
load time when the hosted libretro core reports `retro_serialize_size() == 0`
(`Libretro.cs:275-283`). Capability can therefore change after construction;
clients must re-check rather than cache the initial service set. `[V]`

`ServiceInjector` implements attribute-based injection for client-side tools
and Lua libraries (`ServiceInjector.cs:59-99`):

- properties marked `[RequiredService]` must resolve, or the target is
  unavailable;
- properties marked `[OptionalService]` may be null;
- `IsAvailable` checks only required services, so a tool can exist without an
  optional capability (`ServiceInjector.cs:93-99`).

`FeatureNotImplementedAttribute` is the convention for “this service exists but
this member does not”; by convention it throws `NotImplementedException`
(`ServiceAttributes.cs:5-13`). `ServiceNotApplicableAttribute` declares
services a core will never implement (`ServiceAttributes.cs:15-28`).
The `IsImplemented` helper only checks whether that attribute is absent
(`src/BizHawk.Emulation.Common/Extensions.cs:365-368`), so it is a declaration check, not a behavioural
proof. `[V]`

### 3.3 Capability declaration and introspection

`CoreAttribute` carries only `CoreName`, `Author`, `Released`, and
`SingleInstance` (`CoreAttribute.cs:3-21`). `PortedCoreAttribute` adds
`PortedUrl`/`PortedVersion` (`CoreAttribute.cs:23-42`). There is no
`Capabilities` object at load time. `[V]`

The frontend’s “Core Features” tool reconstructs capability status:

- it enumerates `ServiceProvider.AvailableServices`;
- it excludes declared `NotApplicableTypes`;
- it treats service interfaces not present as missing;
- it marks each method “complete” unless `[FeatureNotImplemented]` is present;
- exceptions during reflection are reported as incomplete
  (`CoreFeatureAnalysis.cs:16-91,147-160`, `Extensions.cs:365-368`).

This is a diagnostic UI, not a contract. The important pattern is “show
capabilities and degradation”, not “reflect over method bodies”. `[V]`/`[I]`

### 3.4 Core registry and load path

`CoreInventory` is a reflection-based registry (`CoreInventory.cs:14-191`):

- scans assemblies for non-abstract types implementing `IEmulator`;
- requires exactly one `CoreAttribute`;
- discovers one or more `CoreConstructorAttribute`s, each naming a system ID and
  an optional `CorePriority` (`CoreInventory.cs:194-238`);
- exposes `AllCores` as `system ID → list of cores`.

Load paths support two constructor shapes:

- modern `CoreLoadParameters<TSettings,TSyncS>` with `Comm`, `Game`, `Settings`,
  `SyncSettings`, `Roms`, `Discs`, and `DeterministicEmulationRequested`
  (`CoreLoadParameters.cs:34-64`);
- legacy parameter-name matching for `comm`, `game`, `rom`, `file`,
  `deterministic`, `settings`, `syncsettings`, `extension`
  (`CoreInventory.cs:111-125`).

`CoreComm` is the client-to-core channel: modal/OSD message callbacks, a
firmware/file provider, and optional `CorePreferencesFlags` such as waterbox
consistency checks (`CoreComms.cs:11-54`). `ICoreFileProvider` abstracts
firmware/BIOS lookup, libretro system paths, and per-system user directories
(`ICoreFileProvider.cs:6-35`).

This separation — registry/factory on the outside, explicit load parameters in
the middle, a narrow comm/file-provider channel for side effects — is highly
reusable. `[I]`

### 3.5 Settings and sync settings

`ISettable<TSettings,TSync>` distinguishes:

- normal settings that may change during a core instance’s life;
- sync settings that must not change while recording because they affect movie
  synchronisation (`ISettable.cs:8-56`).

Both are JSON-serializable objects. `SettingsAdapter` locates the single
`ISettable<,>` service at runtime and invokes its methods reflectively; it
treats `object` as “this half does not exist”
(`ISettable.cs:70-197`). Movie files and savestates store the sync-settings JSON
string and compare it for equality. `[V]`

This is a reusable pattern (separate deterministic configuration from
user-facing settings), but the opaque JSON string and equality check are a
BizHawk-specific weakness; see §5.4.

### 3.6 Input abstraction

`ControllerDefinition` is the input schema (`ControllerDefinition.cs:14-200`):

- a name;
- boolean buttons;
- axes with an `AxisSpec` range, neutral value, optional reversal and
  constraint such as circular axis pairing (`AxisSpec.cs:7-45`);
- ordered control groups (console buttons, then per-player) used for input-log
  encoding;
- haptic channels and category labels;
- an immutability lifecycle: build, `MakeImmutable()`, then enumerate
  (`ControllerDefinition.cs:22-32,114-145`);
- a `Merge` helper for combined/link cores (`ControllerDefinition.cs:186-199`).

`IController` is read-only from the core’s perspective: `Definition`,
`IsPressed`, `AxisValue`, haptic snapshot/strength
(`IController.cs:7-30`). `IInputAdapter` adds a mutable `Source` so input can
be layered (`IInputAdapter.cs:9-12`).

The client builds an input chain in `InputManager.SyncControls`
(`InputManager.cs:63-88`), described in the file header as:

```text
user -> Input -> ActiveController -> UDLR -> Sticky... -> MovieInputSourceAdapter
     -> MovieSession -> MovieOutputAdapter -> ControllerOutput -> Game
```
(`InputManager.cs:10-13`)

Adapters include bitwise AND/OR/XOR, UDLR cleanup, sticky hold/autofire,
override, and multitrack. `MultitrackAdapter` is especially relevant to
TAStudio: it lets an edit affect only a subset of controls while preserving the
rest from a backing controller (`MultitrackAdapter.cs:12-56`).

Movie input is encoded as a compact text mnemonic: `|` separates control groups,
booleans are single mnemonic characters or `.`, axes are fixed-width integers
followed by commas (`Bk2LogEntryGenerator.cs:41-70`). A `LogKey` records the
button order as human-readable documentation
(`Bk2LogEntryGenerator.cs:19-39`). `Bk2Controller` can set state from these
mnemonics (`Bk2Controller.cs:55-84`).

This is a good generic pattern: a declared, immutable input schema plus a
layered adapter chain. The concrete BK2 mnemonic encoding is BizHawk-specific
and should not be adopted as a storage format. `[V]`/`[I]`

### 3.7 Observed core capability differences

The following table is a sample, not a census. It is based on class
declarations, service registrations, and `CanStep` implementations in the
pinned tree.

| Core / path | Debug service | Instruction step | Trace service | Memory domains | State | Notable limitation |
| --- | --- | --- | --- | --- | --- | --- |
| NesHawk `Consoles/Nintendo/NES/NES.cs:11-13` | `IDebuggable` | `false` (`NES.IDebuggable.cs:12`) | `ITraceable` registered (`NES.cs:52-61`) | yes | `IStatable` via `StateSerializer` | No mid-frame stepping; debugger still has registers/memory callbacks/disassembly |
| BSNESv115+ `Consoles/Nintendo/BSNES/BsnesCore.cs:17` | `IDebuggable` | `true` for Into/Over/Out (`BsnesCore.IDebuggable.cs:59-70`) | `ITraceable` registered (`BsnesCore.cs:143-146`) | yes | yes | One of the few cores with real instruction stepping |
| BSNES v87 `Consoles/Nintendo/SNES/LibsnesCore.cs:22` | `IDebuggable` | `false` (`LibsnesCore.IDebuggable.cs:53-62`) | `ITraceable` exists | yes | yes | Step declared but not implemented |
| Octoshock PSX `Consoles/Sony/PSX/Octoshock.cs:30-33` | `IDebuggable` | `false` (`Octoshock.IDebuggable.cs:82-90`) | `ITraceable` via `Octoshock.ITraceable.cs` | yes | yes | Memory callbacks implemented; no stepping; cycles unavailable |
| Mupen64Plus N64 `Consoles/Nintendo/N64/N64.cs:9-16` | `IDebuggable` | `false`; comment says disabled pending mid-frame pausing (`N64.IDebuggable.cs:79-92`) | trace available | yes | yes | Shows the runtime limitation explicitly |
| MAME `Arcades/MAME/` | **no `IDebuggable` files** (`[T]` grep: 0 matches) | no | **no `ITraceable`** (`[T]`: 0 matches) | yes, registered in `MAME.MemoryDomains.cs:110` | `IStatable` exists | Save states exist but no generic debugger/trace integration |
| Libretro `Libretro/` | **no `IDebuggable`** (`[T]`: 0 matches) | no | **no `ITraceable`** (`[T]`: 0 matches) | conditionally registered | `IStatable` can be unregistered at load (`Libretro.cs:275-283`) | Capability depends on the hosted core |
| Link cores (`LinkedDebuggable.cs`) | aggregates linked `IDebuggable`s | `false` (`LinkedDebuggable.cs:50-57`) | per linked core | merged | per linked core | Registers prefixed `P1 `, `P2 `…; no stepping |

Implications:

- A generic backend must not assume that “debugger exists” means “can step” or
  “can trace”.
- Capability discovery must happen after load for dynamic cores such as
  libretro.
- The client is expected to degrade gracefully
  (`GenericDebugger.IToolForm.cs:36-99`, `CoreFeatureAnalysis.cs:147-160`).
- `ICycleTiming` is separate from `IDebuggable.TotalExecutedCycles`; NES only
  registers cycle timing for the subframe core and unregisters it otherwise
  (`NES.cs:73-77`). `[V]`

---

## 4. TAS movie model and rerecording

### 4.1 The movie abstraction

`IMovie` is a large interface covering identity, playback state, input-log
editing, anchors, persistence, and attachment to a core
(`src/BizHawk.Client.Common/movie/interfaces/IMovie.cs:34-208`). Key elements:

- **Modes**: `Inactive`, `Play`, `Record`, `Finished`
  (`IMovie.cs:9-30`); `Finished` means playback has passed the end but the
  movie is still loaded and the user resumes control (`IMovie.cs:148-151`).
- **Header/metadata**: filename, name, game name, frame count, duration, frame
  rate, subtitles, comments, system ID, rerecords, ROM hash, author, core,
  emulator version, firmware hash, board name, arbitrary header entries
  (`IBasicMovieInfo.cs:5-57`).
- **Sync settings**: a JSON string (`IMovie.cs:65-68`).
- **Anchors**: optional text or binary savestate, optional savestate
  framebuffer, optional SaveRAM; flags for “starts from savestate” and “starts
  from SaveRAM” (`IMovie.cs:70-81`).
- **Input editing**: append/poke/record frame, truncate, get input state,
  extract/check input log, write log (`IMovie.cs:96-183`).
- **Attachment lifetime**: attach a core once; “a movie can not and should not
  exist for more than the lifetime of the core” (`IMovie.cs:185-194`).

`MovieSession` owns the active movie and the input latching policy
(`MovieSession.cs`):

- on each frame before emulation, it latches input from the user or from the
  movie depending on mode; in record mode it records the current input at
  `Emulator.Frame` (`MovieSession.cs:79-106`);
- after emulation, if the movie is a `ITasMovie`, it calls
  `GreenzoneCurrentFrame()`; if playback reached the end it applies the
  configured end action (`MovieSession.cs:108-119,338-419`);
- on save/load state it writes/validates the movie input log
  (`MovieSession.cs:121-173`);
- `RunQueuedMovie` attaches the core, processes savestate/SaveRAM anchors, and
  starts recording or playback (`MovieSession.cs:223-247`);
- `Get()` chooses `TasMovie` for `.tasproj` and `Bk2Movie` otherwise
  (`MovieSession.cs:301-317`).

`[V]`

### 4.2 Containers: BK2 and TasProj

Both `.bk2` and `.tasproj` are zip containers written through
`ZipStateSaver`/`ZipStateLoader`. The common lump names are declared in
`BinaryStateLump.cs:8-67`:

- `BizState 1` / `BizVersion` — container and emulator version;
- `Core.bin` / `CoreText.txt` — savestate anchor;
- `Framebuffer.bmp` — screenshot anchor;
- `Input Log.txt` — input log;
- `MovieSaveRam.bin` — SaveRAM anchor;
- `Header.txt`, `Comments.txt`, `Subtitles.txt`, `SyncSettings.json`;
- TasProj extras: `LagLog.json`, `GreenZone`, `GreenZoneSettings.json`,
  `Markers.txt`, `ClientSettings.json`, `VerificationLog.txt`, `UserData.txt`,
  `Session.json`;
- branch extras: `Branches/CoreData.bin`, `Branches/InputLog.txt`,
  `Branches/FrameBuffer.bmp`, `Branches/CoreFrameBuffer.bmp`,
  `Branches/Header.json`, `Branches/Markers.txt`, `Branches/UserText.txt`.

`Bk2Movie.IO` writes the header, comments, subtitles, sync settings, input
log, and optional savestate/SaveRAM anchor (`Bk2Movie.IO.cs:84-120`), and reads
them back (`Bk2Movie.IO.cs:142-190`). `TasMovie.IO` adds lag log, markers,
client settings, verification log, branches, session, and the greenzone
(`TasMovie.IO.cs:12-49`). `ZipStateSaver` writes the zip version and emulator
version into every container (`ZipStateSaver.cs:30-38`). `[V]`

The input log itself is textual and wrapped in `[Input] … [/Input]`, with a
`LogKey:` line followed by one mnemonic row per frame
(`Bk2Movie.InputLog.cs:13-25`). `TimeLength` and `FrameRate` are derived from
the system’s frame rate, not stored per frame (`IBasicMovieInfo.cs:19-27`,
`PlatformFrameRates.cs`). `[V]`

### 4.3 Rerecording semantics

Rerecording is not a special emulation mode. It is the result of:

1. **Input-log truncation on rewind while recording.**
   `MainForm` truncates the movie to `Emulator.Frame` and increments
   `Rerecords` when rewinding while recording
   (`MainForm.cs:3021-3026`). `Bk2Movie.Truncate` removes input entries at or
   after the frame, with special handling for multitrack active inputs so the
   movie length may stay unchanged (`Bk2Movie.cs:107-125`).
2. **State invalidation on edits.**
   `TasMovie.InvalidateAfter` removes lag-log entries, invalidates the state
   manager after the frame, increments `Rerecords` only if a state was actually
   invalidated, and invokes `GreenzoneInvalidated(frame)`
   (`TasMovie.cs:120-144`).
3. **Savestate load into an attached movie.**
   `ExtractInputLog` increments rerecords when the movie input log is replaced
   from a savestate (`Bk2Movie.InputLog.cs:97-100`); `Bk2Movie.RecordFrame`
   truncates first in VBA-style load mode (`Bk2Movie.cs:94-105`).

The practical consequences are important:

- `Rerecords` is a heuristic counter attached to input/state operations, not a
  proof that emulation was re-run or that determinism held.
- `IMovie.IsCountingRerecords` exists to suppress the counter while loading
  (`IMovie.cs:48`, `Bk2Movie.InputLog.cs:144-149`).
- Editing a movie can leave the movie length unchanged (multitrack active-input
  semantics), so `FrameCount` and `InputLogLength` are documented as potentially
  different (`IMovie.cs:54-58`). `[V]`

### 4.4 Savestate timeline validation

When a savestate is loaded while a movie is active, BizHawk validates that the
state belongs to the same input timeline:

- `SavestateFile.Load` first checks the movie timeline **before** loading the
  core state (`SavestateFile.cs:190-198`);
- after loading the core state it calls `HandleLoadState`, which either switches
  to playback and latches the log or extracts the saved input log and switches
  to record (`SavestateFile.cs:213-221`, `MovieSession.cs:144-173`);
- `Bk2Movie.CheckTimeLines` compares each saved input line against the current
  movie up to the state frame, rejects states from a longer movie, and handles
  post-movie savestates via `Finished` mode
  (`Bk2Movie.InputLog.cs:105-183`).

This is a good pattern for a generic session: associate snapshots with an input
prefix digest/timeline, and validate before restoring. BizHawk implements it
with a text comparison of the whole input prefix. `[V]`/`[I]`

### 4.5 TAStudio layer

`.tasproj` is the TAStudio project format. On top of the BK2 fields it stores:

- **Greenzone**: a savestate cache (`GreenZone` lump) plus its settings JSON
  (`GreenZoneSettings.json`) and a lag log (`LagLog.json`)
  (`TasMovie.IO.cs:18-49`).
- **Markers**: named positions bound to input (`TasMovie.cs:25-26,77`).
- **Branches**: alternatives captured as core state + input log + framebuffer +
  markers + user text (`TasBranch.cs:12-44`), saved/loaded with indexed lump
  names (`TasBranch.cs:133-140`).
- **Session**: current frame and current branch (`TasSession.cs:3-20`).
- **Undo/redo**: a batched `IMovieChangeLog` with stable action IDs, merge,
  undo/redo, and bounded `MaxSteps` (`TasMovie.History.cs:8-92`).

`TasMovie.Attach` requires a core with both `IStatable` and `IInputPollable` and
creates/engages the state manager (`TasMovie.cs:29-58`). `GreenzoneCurrentFrame`
is called every frame by `MovieSession.HandleFrameAfter`, writes the lag flag,
and requests a state capture from the manager, forcing a capture for the last
edited frame when needed (`TasMovie.cs:189-195`, `MovieSession.cs:108-113`).
The manager may still decline a capture according to its policy.

The greenzone/state-cache machinery is the core of TAStudio’s responsiveness:
edits at or before the current frame cause invalidation from that frame onward;
TAStudio can then restore the nearest prior state and replay. `[V]`

### 4.6 Movie import/export boundary

BizHawk imports many legacy movie formats into BK2/TasProj:

- 17 concrete importer classes in
  `src/BizHawk.Client.Common/movie/import/` (BKM, DOOM LMP, DSM, FCM, FM2, FMV,
  GMV, Heretic LMP, Hexen LMP, LSMV, MC2, MMV, PJM, PXM, SMV, VBM, YMV), plus
  `IMovieImport` and the `MovieImport` dispatcher
  (`import/IMovieImport.cs:10-17`, `import/MovieImport.cs:11-31`). `[T]`
- Internally, `MovieConversionExtensions` converts `IMovie` → `ITasMovie` and
  `IMovie` → BK2 while preserving header entries, sync settings, comments,
  subtitles, and anchors (`MovieConversionExtensions.cs:8-48`). `[V]`
- No exporter to a foreign emulator format was found in the inspected tree.
  `[T]`

The reusable pattern is the **import boundary**: external formats are
normalised into the internal movie model, with ROM hashes and per-format
quirks handled by dedicated importers. The internal BK2/TasProj format itself
is BizHawk-specific and version-coupled.

---

## 5. Save-state management

### 5.1 The core state contract

`IStatable` is intentionally tiny (`src/BizHawk.Emulation.Common/Interfaces/Services/IStatable.cs:8-31`):

```csharp
bool AvoidRewind { get; }
void SaveStateBinary(BinaryWriter writer);
void LoadStateBinary(BinaryReader reader);
```

`AvoidRewind` lets a core say its states are too slow or otherwise unsuitable
for rewind. `ITextStatable` adds text save/load; `StatableExtensions` provides
hex fallback and `CloneSavestate()` (`IStatable.cs:26-93`). `StateSerializer` is
the generic implementation that delegates to a `Serializer` callback and can
run a `LoadStateCallback` after loading, e.g. to rebuild memory domains
(`StateSerializer.cs:8-53`). NES uses this pattern (`NES.cs:58-61`). `[V]`

### 5.2 Value vs. container

There are two layers:

1. **Core value**: an opaque byte blob produced by `IStatable`.
2. **Container metadata**: `SavestateFile` wraps the core blob with optional
   framebuffer, sync settings, movie input log, user data, lag log, and
   version tags (`SavestateFile.cs:11-59,61-137`).

The container is a zip with named lumps (`BinaryStateLump.cs:8-67`). On load,
`SavestateFile` checks:

- emulator version: exact string equality, unless a developer build; a mismatch
  is a confirm dialog, not a hard failure (`SavestateFile.cs:146-158`);
- core sync settings: exact string equality; a mismatch is a hard failure
  (`SavestateFile.cs:160-188`);
- movie input timeline: hard failure if not the same timeline
  (`SavestateFile.cs:190-198`);
- then it loads the core state and only afterwards applies movie input and
  framebuffer (`SavestateFile.cs:200-226`).

This ordering (validate metadata → load core → apply movie/anchor state) is a
sound pattern. The checks themselves are weak for a general system: version
strings and opaque JSON are not a schema, and “different version” is only a
warning. `[V]`/`[I]`

### 5.3 State managers and rewind

`IStateManager` abstracts a bounded state cache
(`src/BizHawk.Client.Common/movie/tasproj/IStateManager.cs:7-67`):

- `Capture(frame, IStatable, force)`;
- `HasState(frame)`;
- `InvalidateAfter(frame)`;
- `GetStateClosestToFrame(frame)` — must always return a state at or before the
  requested frame, guaranteed by holding frame 0;
- `Clear`, `Unreserve`, `Count`, `Last`;
- `SaveStateHistory` / `LoadStateHistory` for persistence;
- `Engage(frameZeroState)` before use.

Two implementations coexist:

- **`PagedStateManager`** — the current default per
  `MovieConfig.DefaultTasStateManagerSettings` (`MovieConfig.cs:23`); the
  settings converter returns `PagedSettings` for the default settings property
  when no type discriminator is present, while an untyped `.tasproj` setting
  falls back to Zwinder settings (`IStateManagerSettings.cs:31-43`). It uses
  4 KiB pages, a single memory budget, and allows out-of-order states
  (`PagedStateManager.cs:17-30,31-110`).
- **`ZwinderStateManager`** — a tiered manager with `current`, `recent`,
  `gapFiller`, and `reserved` states, eviction into an “ancient” tier, and a
  reserve callback for markers/branches (`ZwinderStateManager.cs:11-58`).
  `GetStateClosestToFrame` scans all tiers for the highest frame ≤ target
  (`ZwinderStateManager.cs:469-483`).

`ZwinderBuffer` is a ring buffer with no copies, arbitrary/changing state sizes,
dynamic capture frequency, optional compression, and optional out-of-order
states (`ZwinderBuffer.cs:9-66`, `src/BizHawk.Client.Common/rewind/ZwinderBuffer.cs:11-18`).
`IRewinder` is the frontend rewind abstraction (`IRewinder.cs:3-23`); `Zwinder`
uses a ring buffer and `ZeldaWinder` adds delta compression
(`Zwinder.cs:9-108`, `rewind/ZeldaWinder.cs`). The main form creates a rewinder
only when the core has `IStatable`, rewind is enabled, and either the core does
not `AvoidRewind` or “allow slow states” is set
(`MainForm.cs:1281-1293`). `[V]`

The tiering policy is a reusable idea: keep dense recent states, sparse older
states, always keep frame 0, and let markers/branches pin states. The specific
implementation is tied to TAStudio’s frame model and should be re-expressed as
an injectable snapshot-cache policy. `[I]`

### 5.4 Determinism and compatibility gaps

- `DeterministicEmulation` is an advisory core flag, not a verified invariant
  (`IEmulator.cs:51-60`).
- Snapshot compatibility is only “same emulator version string + same sync
  settings JSON” (`SavestateFile.cs:146-188`); there is no structured
  configuration hash, backend ID, or schema version in the core payload.
- Core state blobs are opaque; no cross-version migration is attempted. A
  version mismatch prompts the user, then may still load.
- `ZwinderStateManager` has a `GreenZone` version byte and rejects version 0
  (`ZwinderStateManager.cs:524-528`); corrupted state history is caught and
  replaced with a fresh manager (`TasMovie.IO.cs:145-174`).
- Branch data stores a raw core blob plus input log and framebuffer
  (`TasBranch.cs:30-34`), so branches inherit the same opacity.

For ChronoFork, this argues for a snapshot envelope that carries backend ID,
backend version, payload format version, tick, configuration digest, and state
digest — which the existing `chronofork-backend-api` already sketches — plus a
migration/compatibility policy that BizHawk does not have. `[V]`/`[I]`

---

## 6. Frame stepping and seeking

### 6.1 Run-loop anatomy

The core loop is `MainForm.StepRunLoop_Core` (`MainForm.cs:2845-3100`). In
execution order:

1. Decide whether to run a frame (`frameAdvance` hotkey, frame-inch, skip lag
   frames, rewind, pause state) (`MainForm.cs:2861-2914`).
2. Update tools before the frame (`Tools.UpdateToolsBefore()`), including Lua
   `onframestart` and trace-logger sink attachment
   (`MainForm.cs:2941-2948`, `ToolManager.cs:747-756`).
3. Capture a rewind state (`CaptureRewind`) (`MainForm.cs:2950`).
4. `MovieSession.HandleFrameBefore()` — latch user input or movie input, and
   record the frame if recording (`MainForm.cs:2976`, `MovieSession.cs:79-106`).
5. `Emulator.FrameAdvance(InputManager.ControllerOutput, render, renderSound)`
   (`MainForm.cs:3011-3012`).
6. `MovieSession.HandleFrameAfter(...)` — greenzone capture, movie-end action
   (`MainForm.cs:3014`, `MovieSession.cs:108-119`).
7. If rewinding while recording, truncate the movie and increment rerecords
   (`MainForm.cs:3021-3026`).
8. Update tools after the frame (`UpdateToolsAfter()`), including Lua
   `onframeend` and trace-logger flush (`MainForm.cs:3039-3046`,
   `ToolManager.cs:758-767`).
9. Stop turbo-seek when the target frame is reached, and pause if requested
   (`MainForm.cs:3055-3062`).

A notable special case: when a `PreFutureFrameCallback` is set, the client
saves a state, runs “future” frames with `HandleFrameBefore` +
`FrameAdvance` and no tool updates, then restores the saved state
(`MainForm.cs:3066-3086`). This is a save-state trick to answer “what would
happen next?” without permanently advancing. It also illustrates that the
runtime has no mid-frame stepping facility. `[V]`

### 6.2 Frame advance, pause, and turbo seek

- `MainForm.FrameAdvance(bool)` forces one core frame
  (`MainForm.cs:2835-2843`).
- `FrameInch` either advances one frame if paused or pauses if running
  (`MainForm.cs:2862-2873`).
- Frame-advance input sets `PauseEmulator()` on first trigger; holding it can
  turn into frame progress after `FrameProgressDelayMs`
  (`MainForm.cs:2875-2901`).
- Turbo seek is “unpause and run until `PauseOnFrame`”
  (`MainForm.cs:2919-2923,3055-3062`); `IsSeeking` is simply
  `PauseOnFrame.HasValue` (`MainForm.cs:1128-1141`).
- The external API exposes `DoFrameAdvance`, `DoFrameAdvanceAndUnpause`,
  `IsPaused`, `Pause`, `TogglePause`, `IsSeeking`, and `IsRewinding`
  (`IEmuClientApi.cs:50-80`, `EmuClientApi.cs:106-142`).

These are all host-control semantics, not backend contracts. A generic backend
only needs “step one quantum” and “snapshot/restore”; pause/turbo/seek belong to
the session layer. `[I]`

### 6.3 TAStudio seeking

TAStudio does not have a random-access seek. `GoToFrame(frame)`:

- returns immediately if already on the frame;
- pauses after seeking if it was paused;
- switches to play mode while seeking;
- gets the nearest prior state (“for framebuffer”), loads it if the target is
  behind the current frame, or if the nearest state is ahead of the current
  frame;
- unpauses and lets the main loop replay forward to the target;
- stops seeking when `Emulator.Frame >= SeekingTo`
  (`TAStudio.Navigation.cs:7-58`, `TAStudio.IToolForm.cs:109-118`).

`SeekingTo` is stored in `MainForm.PauseOnFrame` (`TAStudio.cs:57-67`).
`StopSeeking` restores recording mode if needed and pauses depending on
`_pauseAfterSeeking` (`TAStudio.ListView.cs:106-126`).

When the target is before the current frame, the greenzone state is loaded and
the emulator replays the input log from there. When the target is ahead, the
emulator runs forward. Thus seek cost is proportional to distance from the
nearest cached state, and “seek” semantics are not guaranteed to land exactly
at the frame without replay. `[V]`

### 6.4 Instruction-level stepping and mid-frame pausing

`IDebuggable` defines `CanStep(StepType)` and `Step(StepType)`; callers must
check `CanStep` and handle `NotImplementedException`
(`IDebuggable.cs:29-38`). The debugger probes this at startup and disables the
step buttons if unsupported (`GenericDebugger.IToolForm.cs:36-99`,
`GenericDebugger.cs:127-145`). Supported step types are `Into`, `Over`, `Out`.

Survey of `CanStep` in the pinned tree `[T]`:

- 27 lambda implementations return `false`.
- Only a handful implement `true` for all three step types, notably BSNESv115+
  (`BsnesCore.IDebuggable.cs:59-70`) and Apple II
  (`AppleII.IDebuggable.cs:92-103`).
- C64 delegates to the selected sub-core; the 6510/1541 implementations return
  true only when `DebuggerStep != null`
  (`Chip6510.IDebuggable.cs:12-23`, `Drive1541.IDebuggable.cs:53-64`).
- N64 explicitly disables stepping with the comment: “Implemented but disabled
  for now. Should be re-enabled once BizHawk supports mid-frame pausing.”
  (`N64.IDebuggable.cs:79-92`).

The root cause is architectural: the main loop advances whole frames and only
checks pause/seek state between them. There is no execution loop that can stop
at an arbitrary instruction boundary with a consistent host state. `[V]`/`[I]`

---

## 7. Lua scripting

### 7.1 Architecture and capability gating

Lua is entirely a client-side feature (`src/BizHawk.Client.Common/lua/`). The
host uses NLua. `LuaLibraries`:

- reflects over sealed `LuaLibraryBase` subclasses;
- constructs a library only if its `[RequiredService]` dependencies are
  satisfied by the current `IEmulatorServiceProvider`;
- otherwise enumerates its methods into the documentation table without
  registering them (`LuaLibraries.cs:72-103`);
- registers `[LuaMethod]` methods into a Lua table, and builds the API docs from
  attributes (`LuaLibraries.cs:21-40`).

This is a strong pattern: **capability-gated API exposure**. If the core lacks
`IDebuggable`, the memory-event library is still documented but not callable.
`LuaLibraryAttribute.Released` and `VersionInfo.DeveloperBuild` further gate
unreleased libraries (`LuaLibraries.cs:75-77`). `[V]`

Built-in libraries include `client`, `comm`, `emu`, `gameinfo`, `input`,
`joypad`, `memory`, `memorysavestate`, `movie`, `savestate`, `sqlite`,
`userdata`, `bit`, `events`, plus system-specific helpers (`lua/CommonLibs/`,
`lua/LuaHelperLibs/`). The UI-only libraries (`console`, `forms`, `gui`,
`TAStudio`) live in `src/BizHawk.Client.EmuHawk/tools/Lua/Libraries/`. `[T]`

### 7.2 Execution model

- Each script owns a `LuaThread` and a `LuaFile` run state
  (`LuaFile.cs:14-117`).
- Scripts are resumed by the host frame loop: `UpdateBefore` calls
  `onframestart`; `UpdateAfter` calls `onframeend` and resumes frame-waiting
  scripts (`LuaConsole.cs:603-621`).
- `emu.frameadvance()` sets a flag and calls `Thread.Yield()`; the host resumes
  the script on the next frame (`LuaLibraries.cs:435-476`).
- `emu.yield()` yields without requesting a frame advance
  (`LuaLibraries.cs:468-476`).
- The host refuses to run Lua from two host threads simultaneously
  (`LuaLibraries.cs:420-433`).
- Event functions are named and can be unregistered by GUID or name; removing
  the last event lets a script stop (`LuaFile.cs:56-60`,
  `LuaLibraries.cs:291-307`).

### 7.3 Mid-frame API restrictions

`ApiGroup` defines `YIELDING`, `BOOTING`, `STATES`, and
`PROHIBITED_MID_FRAME` (`ApiGroup.cs:3-24`). Memory/input callbacks register
their named functions with `ApiGroup.PROHIBITED_MID_FRAME`, and `Sandbox` ORs
that group into `ProhibitedApis` for the duration of the callback
(`LuaLibraries.cs:309-321`). Attempting `emu.frameadvance`, `savestate.load`,
or similar from inside such a callback throws
(`LuaLibraries.cs:457-475`, `SaveStateLuaLibrary.cs:15-17`,
`MemorySavestateLuaLibrary.cs:17-31`). This is a necessary safety property, and
a reusable pattern for any scripting host: **classify APIs by reentrancy
safety**.

### 7.4 Sandboxing

`LuaSandbox` sets the process current directory for the script, catches
exceptions, and restores the directory (`LuaSandbox.cs:44-84`).
`EnvironmentSandbox.Sandbox` is explicitly a stub that just invokes the
callback (`EnvironmentSandbox.cs:4-11`). No OS/FFI/module restriction was found
in the inspected code. Therefore the “sandbox” is a convenience, not a security
boundary; scripts run with the host process’s privileges. `[V]`/`[T]`

### 7.5 Lua ↔ memory callbacks ↔ movie/state

- `EventsLuaLibrary` bridges `event.on_bus_read/write/exec` (and older
  `onmemory*` aliases) to `IDebuggable.MemoryCallbacks`. It validates the
  requested scope and logs when the core lacks callback support
  (`EventsLuaLibrary.cs:36-61,120-160,210-295`).
- Callback parameters are `(addr, value, flags)`; a numeric return can override
  the value passed back to the core (`EventsLuaLibrary.cs:36-49`).
- `InputPollableCore.InputCallbacks` backs `event.oninputpoll` when the core
  supports it (`EventsLuaLibrary.cs:80-110`).
- The movie/state libraries expose playback and savestate operations but refuse
  them during callbacks (see §7.3).

### 7.6 Testing

Lua has a dedicated test harness: `LuaTestContext` constructs `LuaLibraries` on
a `FakeEmulator`, loads scripts from `lua/scripts`, drives frames manually, and
asserts log output/script state (`LuaTestContext.cs:11-97`). Tests cover basic
libraries, events, and the API surface (`src/BizHawk.Tests.Client.Common/lua/`).
This is a good model for testing a scripting layer against a deterministic test
backend. `[V]`

---

## 8. Memory callbacks, debugger, and tracing

### 8.1 Memory callbacks

`IMemoryCallbackSystem` is a per-core service exposed through `IDebuggable`
(`IMemoryCallbackSystem.cs:18-85`). Each callback has:

- `Type`: `Read`, `Write`, or `Execute`;
- `Name`;
- `Address` and `AddressMask` (optional; null means “any address”, except
  execute callbacks which require an address);
- `Scope` (a memory domain/CPU scope from `AvailableScopes`);
- a delegate `uint? (addr, value, flags)`; a non-null return overrides the value
  passed back to the core (`IMemoryCallbackSystem.cs:7-11,92-103`).

Semantics:

- Callbacks are called by the core through `CallMemoryCallbacks(addr, value,
  flags, scope)`; cores are expected to call the read/write/execute variants on
  every access (`IMemoryCallbackSystem.cs:20-58`).
- The API warns that cores may call the empty system on every access, so the
  empty path must be extremely fast (`IMemoryCallbackSystem.cs:20-24`).
- `MemoryCallbackSystem` keeps separate collections for reads/writes/executes,
  checks `_hasAny` first, filters by scope and address mask, and lets the last
  non-null override win (`MemoryCallbackSystem.cs:69-113`).
- `AvailableScopes` is declared per core; e.g. Octoshock uses a single
  `"System Bus"` scope even though it has no domain with that name
  (`Octoshock.IDebuggable.cs:79-80`).
- `ExecuteCallbacksAvailable` is not universal; clients must check it
  (`IMemoryCallbackSystem.cs:26`).

The generic implementation is reusable as a *pattern* (type/scope/address +
fast empty path), but the value-override feature is unusual for a debugger bus
and makes it an instrumentation mechanism, not a pure observer. `[V]`

### 8.2 Debugger

The debugger is `GenericDebugger` (`src/BizHawk.Client.EmuHawk/tools/Debugger/`).
Its architecture:

- `[RequiredService] IDebuggable`; `[OptionalService] IDisassemblable`;
  `[RequiredService] IMemoryDomains`
  (`GenericDebugger.IToolForm.cs:6-20`).
- On restart it probes capabilities: memory callbacks available?
  disassembler present? PC register readable? CPU settable? `CanStep` for each
  step type? It disables UI sections and shows tooltips when unavailable
  (`GenericDebugger.IToolForm.cs:36-99`, `GenericDebugger.cs:41-145`).
- Registers are read/written through `IDebuggable.GetCpuFlagsAndRegisters` /
  `SetCpuRegister`; the register panel builds its UI from the returned
  dictionary (`RegisterBoxControl.cs`). `[T]`
- Disassembly is rendered from `IDisassemblable.Disassemble(MemoryDomain, addr,
  out length)` (`src/BizHawk.Emulation.Common/Interfaces/Services/IDisassemblable.cs:11-33`).
- Breakpoints are implemented as memory callbacks. `Breakpoint` wraps an
  `IMemoryCallback` and activates/deactivates by adding/removing it from the
  core (`BreakpointList.cs:66-166`). `BreakpointControl` maps the breakpoint
  list to the callback system and triggers `UpdateForBreakpointHit`
  (`BreakpointControl.cs:56-70`).
- “Seek to PC” installs a temporary execute callback at the target address
  (`BreakpointControl.cs:162-176`).

This is a good reusable pattern: breakpoints/watchpoints are clients of a
generic callback bus, while CPU registers/disassembly are separate optional
services. The implementation is per-core and only as complete as the core’s
`IDebuggable` is. `[V]`

### 8.3 Tracing

`ITraceable` has a `Header` string and a single `Sink` of `ITraceSink`
(`src/BizHawk.Emulation.Common/Interfaces/Services/ITraceable.cs:3-38`). Each `TraceInfo` is a disassembly string plus a
register-info string (`ITraceable.cs:27-38`). The comments acknowledge the
single-sink limitation and suggest a sink can fan out to two sinks if needed
(`ITraceable.cs:19-24`).

Two implementations:

- `TraceBuffer` — trivial `ITraceable` holder
  (`src/BizHawk.Emulation.Common/Base Implementations/TraceBuffer.cs:5-16`).
- `CallbackBasedTraceBuffer` — an abstract fallback for cores that have memory
  callbacks, `IMemoryDomains`, and `IDisassemblable` but no native trace hook.
  It requires the core to provide `GetCpuFlagsAndRegisters`, adds an execute
  callback with `Address == null` (wildcard), and uses
  `AvailableScopes[0]` only; the comment flags multi-scope cores as an issue
  (`CallbackBasedTraceBuffer.cs:5-17,47-63`). It warns that this is
  significantly slower than a native implementation. `[V]`

`TraceLogger` attaches a `CallbackSink` to `Tracer.Sink` at `PreFrame`, either
accumulating rows for a window (bounded by `MaxLines`) or writing lines to a
log file (with optional file splitting at `FileSizeCap`)
(`TraceLogger.cs:119-193`). It reads `TraceInfo.Disassembly` and
`TraceInfo.RegisterInfo` directly (`TraceLogger.cs:105-117`). `[V]`

Separately, `ICodeDataLogger` supports code/data logging (CDL) with memory
domain maps and disassembly (`ICodeDataLogger.cs:10-75`). CDL is a
domain-specific instrumentation service, not part of `ITraceable`. `[T]`

Tracing weaknesses for a generic system:

- stringly typed rows rather than structured records;
- one sink, no backpressure, no bounded ring buffer in the interface;
- per-core availability and performance vary widely;
- fallback uses only the first callback scope;
- `Header` is a free-form string, not machine-readable schema. `[V]`/`[I]`

---

## 9. Test infrastructure

### 9.1 Unit tests in the main solution

The main solution includes three test projects (`BizHawk.sln:38-42`):

- `BizHawk.Tests.Common`
- `BizHawk.Tests.Emulation.Common`
- `BizHawk.Tests.Client.Common`

They target `net48` by default, with `TestProjTargetFrameworkOverride` used to
run on `net10.0` in CI (`TestProjects.props:1-16`,
`.github/workflows/ci.yml:27-69`). The runner is MSTest 3.9.3 with
`JunitXml.TestLogger`, `EnableMSTestRunner`, and a 5-minute session timeout
(`TestProjects.props:4,13-14`, `MainSln.runsettings:1-7`). Tests are invoked by
`dotnet test` through `Dist/.InvokeCLIOnMainSln.sh:8-14` and
`Dist/.BuildTestInConfigX.sh:1-10`; GitLab CI uses
`Dist/BuildTestRelease.sh` (`.gitlab-ci.yml:213-226`). `[V]`

Local counts at the pinned commit: 337 `[TestMethod]` occurrences across 43
files. `[T]`

The strongest part is the test-double design:

- `FakeEmulator` implements `IEmulator`, `IStatable`, and `IInputPollable` with
  two bool buttons and one axis; `FrameAdvance` increments `Frame`, fires input
  callbacks, and save/load is a tiny binary format
  (`src/BizHawk.Tests.Client.Common/Movie/FakeEmulator.cs:8-76`).
- `FakeMovieSession` and `FakeStateManager` provide minimal implementations for
  movie/state tests (`.../FakeMovieSession.cs:9-78`, `.../FakeStateManager.cs:9-34`).
- Tests cover BK2 multitrack semantics, TasMovie operation/undo behaviour,
  movie service parsing, state managers (`Paged`, `Zwinder`), Lua libraries and
  events, memory callback system, input manager, and config serialization
  stability (`src/BizHawk.Tests.Client.Common/`, `src/BizHawk.Tests.Emulation.Common/`).
  `[T]`

This is a useful model: a deterministic fake backend with a tiny state format,
plus tests for movie/state/undo semantics against the fake. `[I]`

### 9.2 Test-ROMs / screenshot suites

The `BizHawk.Tests.Testroms*` projects are separate from the main solution and
not run by `dotnet test BizHawk.sln` (no `Testroms` entry in `BizHawk.sln`;
no testroms job in the inspected CI files). `[T]`

They use a `DummyFrontend` that constructs a real core with a `CoreComm` and a
`SimpleGDIPDisplayManager`, manually advances frames, and grabs the video
buffer as a screenshot (`src/BizHawk.Tests.Testroms/DummyFrontend.cs:14-193`).
Tests compare screenshots to embedded reference PNGs with ImageMagick, support
known-failure lists and “unexpected success” detection, and write visual
artifacts on failure (`ImageUtils.cs:61-82`, `AcidTestroms.cs:92-120`,
`Testroms.props:1-21`). Missing ROMs are reported as inconclusive
(`TestUtils.cs:71-74`). `[V]`

This is a solid pattern for emulator conformance (deterministic frame counts +
golden screenshots), but it is not a generic-backend acceptance suite because:

- it needs real cores and native binaries;
- it needs copyrighted/free test ROMs and firmware assets;
- it is separate from the default test invocation;
- it tests rendering and some memory behaviour, not the movie/state/TAS
  contracts. `[V]`/`[I]`

### 9.3 Test gaps

- No shared conformance suite that every `IEmulator` implementation must pass.
- No automated determinism/replay test that hashes state after replaying an
  input log.
- No test that `FrameAdvance` is a pure function of `(state, input)` for each
  core.
- No cross-version snapshot compatibility tests.
- No headless frontend test in the main solution; the testroms harness is the
  closest analogue.
- Native/FFI cores and prebuilt binaries are largely outside unit-test
  coverage. `[I]`/`[T]`

---

## 10. Assessment for a generic backend interface

### 10.1 Reusable patterns

The table below is the core deliverable: which BizHawk patterns should inform a
generic backend contract, and which parts are BizHawk-specific.

| Pattern | BizHawk evidence | Reusable in a generic backend? | Caveat / adaptation |
| --- | --- | --- | --- |
| **Minimal execution contract + optional capability interfaces** | `IEmulator` vs. `IEmulatorService` (`IEmulator.cs:10-66`, `IEmulatorService.cs:8-20`) | **Yes, strongly** | Keep the base trait tiny; make save/restore, memory, debug, trace, input callbacks, audio/video separate optional capabilities. |
| **Runtime service discovery** | `IEmulatorServiceProvider`, `BasicServiceProvider` (`IEmulatorServiceProvider.cs:14-44`, `BasicServiceProvider.cs:14-74`) | **Partially** | In Rust, prefer explicit optional trait objects or a capability struct over a stringly/reflection service locator. A service locator is fine internally, but capability reporting must be explicit. |
| **Declared capability report** | `CoreAttribute`, `ServiceNotApplicable`, `FeatureNotImplemented`, `CoreFeatureAnalysis` (`CoreAttribute.cs:3-42`, `ServiceAttributes.cs:5-28`, `CoreFeatureAnalysis.cs:16-91`) | **Yes, but improve** | BizHawk infers capabilities at runtime. A generic backend should publish a static `Capabilities` value at load time and never infer “implemented” from method presence. |
| **Registry + explicit load parameters** | `CoreInventory`, `CoreConstructorAttribute`, `CoreLoadParameters<,>` (`CoreInventory.cs:14-191,227-258`, `CoreLoadParameters.cs:34-64`) | **Yes** | Keep system ID → factories mapping, priority, and an explicit load-parameters struct. Avoid reflection and legacy constructor heuristics. |
| **Narrow client↔core communication channel** | `CoreComm`, `ICoreFileProvider` (`CoreComms.cs:11-54`, `ICoreFileProvider.cs:6-35`) | **Yes** | Separate messages, firmware/file lookup, and optional host diagnostics from the core state machine. Inject an interface, not a global. |
| **Separate deterministic config from user settings** | `ISettable<TSettings,TSync>` (`ISettable.cs:8-56`) | **Yes** | Hash the deterministic config into snapshots; do not rely on opaque JSON string equality. |
| **Immutable input schema + layered adapters** | `ControllerDefinition`, `AxisSpec`, `IController`, `IInputAdapter` (`ControllerDefinition.cs:14-200`, `AxisSpec.cs:7-45`, `IController.cs:7-30`, `IInputAdapter.cs:9-12`) | **Yes** | Version the schema; support ports/axes explicitly; keep adapters out of the backend. The BK2 mnemonic encoding is not reusable. |
| **Input-log + state-anchor movie model** | `IMovie`, `MovieSession`, BK2/TasProj (`IMovie.cs:34-208`, `MovieSession.cs:79-119`) | **Yes** | Model a movie as input frames + optional snapshot anchor + metadata. Do not adopt BK2/TasProj as the storage format. |
| **Snapshot envelope with metadata** | `SavestateFile` + `BinaryStateLump` (`SavestateFile.cs:11-137,139-254`, `BinaryStateLump.cs:8-67`) | **Yes, strengthen** | Add backend ID/version, payload format version, tick, config digest, state digest; BizHawk has only emulator version + sync-settings string. |
| **Bounded state cache with nearest-state lookup** | `IStateManager`, `PagedStateManager`, `ZwinderStateManager` (`IStateManager.cs:7-67`, `PagedStateManager.cs:17-110`, `ZwinderStateManager.cs:11-66`) | **Yes** | Make eviction/tiering policy injectable; always keep tick 0; support pinning for markers/branches; persist/restore the cache if desired. |
| **Greenzone invalidation on edit** | `TasMovie.InvalidateAfter`, `GreenzoneInvalidated`, `FrameEdited` (`TasMovie.cs:120-144`, `TAStudio.ListView.cs:1064-1147`) | **Yes** | The pattern “an edit at tick N invalidates cached states after N, then restore nearest state and replay” is general. |
| **Rewind as a separate ring buffer** | `IRewinder`, `Zwinder`, `ZwinderBuffer` (`IRewinder.cs:3-23`, `Zwinder.cs:9-108`, `ZwinderBuffer.cs:9-66`) | **Yes** | Keep rewind policy separate from snapshot persistence; allow a core to opt out (`AvoidRewind` analogue). |
| **Frame-loop with explicit ordering** | `MainForm.StepRunLoop_Core` (`MainForm.cs:2845-3100`) | **Yes, conceptually** | The order “input latch → pre-frame hooks → step → post-frame hooks” is reusable; tool/UI hooks must live in the host, not the backend. |
| **Breakpoints/watchpoints as callbacks over a memory bus** | `IMemoryCallbackSystem`, `Breakpoint` (`IMemoryCallbackSystem.cs:18-85`, `BreakpointList.cs:66-166`) | **Yes** | Make the empty path cheap; define scope semantics; decide whether callbacks may mutate (BizHawk allows override — a generic debugger probably should not). |
| **Structured trace sink** | `ITraceable`, `TraceInfo`, `TraceLogger` (`ITraceable.cs:3-38`, `TraceLogger.cs:105-193`) | **Partially** | Adopt the sink idea, but emit structured records (tick, CPU, PC, opcode, registers, flags) and support bounded buffers, multiple consumers, and backpressure. |
| **Capability-gated scripting API** | `LuaLibraries` registration (`LuaLibraries.cs:72-103`) | **Yes** | Expose only APIs whose capabilities are present; document unavailable APIs; reject unsafe calls during callbacks. |
| **Deterministic test double + golden tests** | `FakeEmulator`, `DummyFrontend`, screenshot suites (`FakeEmulator.cs:8-76`, `DummyFrontend.cs:114-193`, `ImageUtils.cs:61-82`) | **Yes** | Build a fake backend and a conformance suite; keep golden-image tests separate from unit tests. |

### 10.2 BizHawk-specific limitations to avoid

| Limitation | Evidence | Consequence for a generic system |
| --- | --- | --- |
| **WinForms / .NET Framework coupling** | EmuHawk and DiscoHawk are `net48` + `UseWindowsForms` (`src/BizHawk.Client.EmuHawk/BizHawk.Client.EmuHawk.csproj:3,14`; same for DiscoHawk); `BatchRunner` is a GUI tool (`BatchRun.cs:12`). No headless entry point found. | A Linux-first/headless backend must not depend on the frontend. Keep the core contract UI-free; expose a library/CLI boundary. |
| **Capabilities are runtime/reflection-based** | `ServiceProvider` probes, `CoreFeatureAnalysis` reflection (`CoreFeatureAnalysis.cs:16-91`), `IsImplemented` only checks attributes (`Extensions.cs:365-368`). | Capabilities must be declared, versioned, and testable. Do not infer from type presence or method bodies. |
| **Dynamic services can disappear after load** | Libretro unregisters `IStatable` if `retro_serialize_size() == 0` (`Libretro.cs:275-283`). | Re-query capabilities after load; snapshot support may be conditional on the loaded content. |
| **Instruction stepping is rare and frame-bound** | N64 disabled pending mid-frame pause (`N64.IDebuggable.cs:79-92`); many cores return `CanStep == false`; loop pauses between frames (`MainForm.cs:2845-3100`). | Decide whether the backend quantum is a frame or an instruction. If frames, do not promise instruction stepping. If instructions, design the host loop for mid-quantum pause from the start. |
| **Seeking is replay, not random access** | `TAStudio.Navigation.cs:7-58`; `MainForm.cs:3055-3062`. | Expose “advance to tick from nearest snapshot” and make snapshot density a policy; do not promise O(1) seek. |
| **Movie format is BizHawk-specific and weakly versioned** | `HeaderKeys.cs:9-30`, `MainForm.Movie.cs:174-297` (per-core header special cases), `SavestateFile.cs:146-188`. | Use a versioned, backend-neutral movie/session format; never key compatibility on emulator version strings or opaque sync JSON. |
| **Opaque core snapshot payloads** | `IStatable` byte stream (`IStatable.cs:14-19`); `SavestateFile` only checks version/sync settings. | Add payload format version, backend ID, config digest, state digest; define migration and rejection policy. |
| **Rerecord count is a heuristic** | Counters increment on invalidation/state-load/rewind (`TasMovie.cs:120-144`, `Bk2Movie.InputLog.cs:97-100`, `MainForm.cs:3021-3026`). | Treat rerecord counts as user-facing metadata, never as a determinism proof. Provide separate replay/divergence diagnostics. |
| **Lua sandbox is not a security boundary** | `LuaSandbox.cs:44-84`, `EnvironmentSandbox.cs:4-11`. | If scripting is exposed to untrusted users/agents, isolate it (process/WASM) and restrict capabilities explicitly. |
| **Stringly typed tracing and single sink** | `ITraceable.cs:3-38`, `CallbackBasedTraceBuffer.cs:47-63`. | Use structured trace events, bounded buffers, and multiple consumers; make trace availability a capability. |
| **Memory callbacks can mutate emulated values** | `MemoryCallbackDelegate` returns `uint?`; non-null overrides (`IMemoryCallbackSystem.cs:7-11`, `MemoryCallbackSystem.cs:69-80`). | Separate observation from mutation. Debugger watchpoints should be read-only by default; mutation should be an explicit API. |
| **Tests do not prove core conformance or determinism** | Three unit-test projects in the solution; testroms separate; no replay-hash suite (`BizHawk.sln:38-42`, `src/BizHawk.Tests.Testroms*`). | Make a generic backend conformance suite (deterministic fake + real-backend smoke) part of acceptance; include replay/state-hash tests. |
| **Licensing heterogeneity** | `LICENSE:1-7`, `README.md:469-471`, `.gitmodules:1-111`. | Do not reuse code/binaries without a per-file/per-component licence audit; prefer independent implementations and permissive backends. |

### 10.3 Mapping to the existing ChronoFork backend API

The lead’s current `chronofork-backend-api` scaffold already reflects several of
the right lessons, and comparing it with BizHawk highlights the remaining
decisions:

| ChronoFork concept | BizHawk analogue | Assessment |
| --- | --- | --- |
| `Backend::step(&mut self, InputFrame) -> Result<StepOutcome>` (`crates/chronofork-backend-api/src/traits.rs:26-41`) | `IEmulator.FrameAdvance` (`IEmulator.cs:27-38`) | Good. BizHawk returns a bool “new frame”; ChronoFork’s `StepOutcome { tick, state_hash, halted }` is stronger because it reports the resulting tick and digest. |
| `Backend::snapshot` / `restore` (`traits.rs:33-38`) | `IStatable.SaveStateBinary/LoadStateBinary` (`IStatable.cs:14-19`) | Good. ChronoFork’s `BackendSnapshot` envelope (backend ID, version, format version, tick, config hash, state hash, payload) is stronger than BizHawk’s container. |
| `Backend::inspect` (`traits.rs:40-41`) | `IDebuggable.GetCpuFlagsAndRegisters` + `IDisassemblable` (`IDebuggable.cs:12-45`, `IDisassemblable.cs:11-33`) | Good start. BizHawk separates register/debug state, disassembly, and memory domains; ChronoFork may want optional `inspect_disassembly`/`trace` capabilities later. |
| `Backend::read_memory` / `write_memory` (`traits.rs:43-50`) | `IMemoryDomains` + `MemoryDomain` (`IMemoryDomains.cs:16-33`, `MemoryDomain.cs:14-40`) | Good. BizHawk’s domains have names, endianness, word size, writability, peek/poke, and a “System Bus”; ChronoFork’s flat address API is simpler but loses domain scoping. |
| `Capabilities` booleans (`descriptor.rs:33-87`) | `ServiceProvider.AvailableServices` + `FeatureNotImplemented` | Much better than BizHawk: explicit, load-time, testable. Keep it authoritative and fail closed. |
| `BackendFactory::create` (`traits.rs:57-68`) | `CoreInventory` + `CoreConstructor` (`CoreInventory.cs:14-191`) | Good. ChronoFork avoids reflection; a registry of factories is the right analogue. |
| `BackendDescriptor` (`descriptor.rs:90-148`) | `CoreAttribute` (`CoreAttribute.cs:3-21`) | Good. Consider adding provenance/licence metadata here for auditability. |
| No input schema beyond `u32` buttons (`crates/chronofork-core/src/input.rs:1-30`) | `ControllerDefinition` + `AxisSpec` (`ControllerDefinition.cs:14-200`) | BizHawk is richer (ports, axes, constraints). A generic backend will eventually need a versioned input schema; the current `u32` is fine for a first test backend but should not be presented as universal. |
| No watchpoints/breakpoints/tracing yet (`descriptor.rs:52-57`) | `IMemoryCallbackSystem`, `ITraceable` (`IMemoryCallbackSystem.cs:18-85`, `ITraceable.cs:3-38`) | BizHawk shows both the value and the pitfalls. Add them as capabilities with structured, read-only defaults. |
| No rewind/state-cache layer in the API | `IStateManager`, `IRewinder` (`IStateManager.cs:7-67`, `IRewinder.cs:3-23`) | Correct layer separation: state caching belongs to the session layer, not the backend trait. |

**Recommendation:** adopt BizHawk’s *separation* (tiny execution contract +
optional capability interfaces) and its *state-cache/invalidation* ideas, but
not its reflection-based discovery, stringly typed movie format, opaque
snapshot validation, or UI-coupled scripting. The existing ChronoFork
`Capabilities`/`BackendSnapshot` design is already closer to what a generic
backend needs than BizHawk’s runtime probes. `[I]`

---

## 11. Open questions

1. **Backend target and determinism.** Which emulator(s) will ChronoFork actually
   wrap first, and do their licences permit the intended distribution model?
   BizHawk’s tree shows GPLv2, LGPL, non-commercial, and unknown components; this
   must be decided before any code reuse. `[U]`
2. **Quantum: frame or instruction?** BizHawk is frame-quantized; instruction
   stepping is exceptional and N64’s is disabled pending mid-frame pausing.
   Should ChronoFork’s `step` be one frame, one instruction, or a
   backend-declared quantum? The answer determines whether `Capabilities`
   should include `instruction_stepping` as a first-class promise.
3. **Snapshot compatibility policy.** What happens when a snapshot’s backend
   version, payload format, or config hash mismatches? BizHawk warns on
   emulator-version mismatch and hard-fails only on sync-settings mismatch.
   ChronoFork needs an explicit accept/reject/migrate policy.
4. **State-cache policy ownership.** Should the session layer own a greenzone
   (dense recent / sparse old / pinned markers) as BizHawk does, or should the
   backend expose state-storage hints? BizHawk’s `IStateManager` is a good
   starting point, but it is coupled to frame numbers and a single `IStatable`
   byte stream.
5. **Input schema.** When will ChronoFork outgrow the `u32` input mask? BizHawk’s
   `ControllerDefinition` shows the value of ports, axes, neutral values, and
   constraints, but also shows how much format coupling follows from a mnemonic
   encoding.
6. **Debugger semantics.** Should memory callbacks be pure observers, or may
   they mutate emulated memory (BizHawk allows override)? Should breakpoints be
   conditional expressions, as ChronoFork’s capability stub suggests, and who
   evaluates them — host or backend?
7. **Tracing format.** What does a structured trace event need to contain for
   ChronoFork’s use cases (tick, CPU, PC, opcode, registers, flags, cycles)?
   BizHawk’s `TraceInfo` (two strings) is too weak to build tooling on.
8. **Scripting model.** Does ChronoFork need a Lua-compatible scripting layer,
   a different embedded language, or an external control API? BizHawk’s Lua
   layer is capable but UI-coupled and not sandboxed; the external API
   (`IEmuClientApi`) is a useful precedent for a host-control surface.
9. **Headless contract.** What is the equivalent of `CoreComm`/`ICoreFileProvider`
   for firmware, BIOS, and per-system paths in a headless Linux process? The
   testroms `EmbeddedFirmwareProvider` is a useful precedent
   (`DummyFrontend.cs:15-90`).
10. **Acceptance tests.** Should ChronoFork adopt a BizHawk-style golden
    screenshot suite, a replay/state-hash conformance suite, or both? The
    deterministic fake backend is necessary but not sufficient; real-backend
    smoke tests need licensing-safe ROMs.
11. **Licence provenance metadata.** Should `BackendDescriptor` carry licence
    and upstream URL fields so that any backend can be audited automatically?
    BizHawk’s `PortedCoreAttribute` is a minimal precedent
    (`CoreAttribute.cs:23-42`), but it does not carry a licence identifier.
12. **Cross-version state hashing.** BizHawk has no state hash in its save
    states. ChronoFork’s `BackendSnapshot` has one, but how is it computed and
    verified across backends/versions? A stable, documented hash is a
    prerequisite for divergence detection and replay proofs.

---

## Appendix A. Citation map

All paths below are relative to `.research/upstream/bizhawk/`.

**Repository / licensing**

- `LICENSE:1-20` — provenance, MIT claim, GPL-minefield statement.
- `README.md:467-471` — MIT for EmuHawk/DiscoHawk; “anything outside `/src`
  isn’t ours”.
- `contributing.md:322-327` — contribution licensing policy.
- `.gitmodules:1-111` — 32 submodule entries.
- `blip_buf/license.txt:1-3`; `lynx/license.txt:1-14`;
  `waterbox/picodrive/COPYING:1-8`; `waterbox/tic80/LICENSE:1-3`;
  `libmupen64plus/mupen64plus-core/LICENSES:1-4`;
  `libmupen64plus/mupen64plus-rsp-hle/LICENSES:1-4`;
  `libmupen64plus/mupen64plus-video-rice/LICENSES:1-4`;
  `libmupen64plus/mupen64plus-video-glide64mk2/LICENSES:1-4`;
  `libmupen64plus/mupen64plus-video-glide64/COPYING:1-3`;
  `ExternalProjects/NLua/LICENSE`; `ExternalProjects/iso-parser/LICENSE.md`.

**Core abstraction**

- `src/BizHawk.Emulation.Common/Interfaces/IEmulator.cs:10-66`
- `.../Interfaces/IEmulatorService.cs:8-20`
- `.../Interfaces/IEmulatorServiceProvider.cs:14-44`
- `.../Base Implementations/BasicServiceProvider.cs:14-74`
- `.../ServiceInjector.cs:59-99`
- `.../ServiceAttributes.cs:5-28`
- `.../CoreAttribute.cs:3-42`
- `.../Extensions.cs:74-77,85-233,365-368`
- `src/BizHawk.Emulation.Cores/CoreInventory.cs:14-191,194-258`
- `src/BizHawk.Emulation.Cores/CoreLoadParameters.cs:34-64`
- `src/BizHawk.Emulation.Common/CoreComms.cs:11-54`
- `src/BizHawk.Emulation.Common/Interfaces/ICoreFileProvider.cs:6-35`

**Input**

- `src/BizHawk.Emulation.Common/Base Implementations/ControllerDefinition.cs:14-200`
- `.../Base Implementations/Axes/AxisSpec.cs:7-45`
- `.../Interfaces/IController.cs:7-30`
- `src/BizHawk.Client.Common/inputAdapters/IInputAdapter.cs:9-12`
- `src/BizHawk.Client.Common/inputAdapters/InputManager.cs:10-13,63-88`
- `src/BizHawk.Client.Common/inputAdapters/MultitrackAdapter.cs:12-56`
- `src/BizHawk.Client.Common/movie/bk2/Bk2Controller.cs:55-84`
- `src/BizHawk.Client.Common/movie/bk2/Bk2LogEntryGenerator.cs:19-70`

**Movie / TAStudio**

- `src/BizHawk.Client.Common/movie/interfaces/IMovie.cs:9-30,34-208`
- `.../interfaces/IBasicMovieInfo.cs:5-57`
- `.../interfaces/ITasMovie.cs:6-78`
- `.../MovieSession.cs:79-119,121-173,223-247,301-317,324-336,338-419`
- `.../bk2/Bk2Movie.cs:19-23,25-28,83-164`
- `.../bk2/Bk2Movie.IO.cs:84-120,142-190`
- `.../bk2/Bk2Movie.InputLog.cs:13-25,34-103,105-183`
- `.../bk2/Bk2Movie.ModeApi.cs:5-23`
- `.../tasproj/TasMovie.cs:11-16,29-58,111-144,189-195`
- `.../tasproj/TasMovie.IO.cs:12-49,65-175`
- `.../tasproj/TasMovie.History.cs:8-92`
- `.../tasproj/TasBranch.cs:12-44,46-67,133-140`
- `.../tasproj/TasSession.cs:3-20`
- `.../MovieService.cs:8-27`
- `src/BizHawk.Client.Common/config/MovieConfig.cs:14-24`
- `src/BizHawk.Client.EmuHawk/tools/TAStudio/TAStudio.Navigation.cs:7-58`
- `.../TAStudio/TAStudio.IToolForm.cs:26-55,57-118`
- `.../TAStudio/TAStudio.ListView.cs:106-126,1064-1147`
- `.../TAStudio/TAStudio.cs:57-67,739-749,1362-1384`
- `src/BizHawk.Client.Common/movie/import/IMovieImport.cs:10-17`
- `src/BizHawk.Client.Common/movie/import/MovieImport.cs:11-31`
- `src/BizHawk.Client.Common/movie/MovieConversionExtensions.cs:8-48`

**Save states / rewind**

- `src/BizHawk.Emulation.Common/Interfaces/Services/IStatable.cs:8-93`
- `.../Base Implementations/StateSerializer.cs:8-53`
- `.../Interfaces/Services/IMemoryDomains.cs:16-33`
- `.../Base Implementations/MemoryDomain.cs:14-40`
- `src/BizHawk.Client.Common/savestates/SavestateFile.cs:11-137,139-254`
- `.../savestates/BinaryStateLump.cs:8-67`
- `.../savestates/ZipStateSaver.cs:9-120`
- `.../savestates/ZipStateLoader.cs:44-83`
- `.../movie/tasproj/IStateManager.cs:7-67`
- `.../movie/tasproj/IStateManagerSettings.cs:6-45`
- `.../movie/tasproj/ZwinderStateManager.cs:11-66,358-457,469-528`
- `.../tools/TAStudio/PagedStateManager.cs:17-110`
- `.../rewind/IRewinder.cs:3-23`
- `.../rewind/Zwinder.cs:9-108`
- `.../rewind/ZwinderBuffer.cs:9-66`
- `src/BizHawk.Client.Common/config/RewindConfig.cs:3-64`
- `src/BizHawk.Client.EmuHawk/MainForm.cs:1281-1293,4351-4460`

**Frame loop / seeking / stepping**

- `src/BizHawk.Client.EmuHawk/MainForm.cs:1128-1141,2835-3100,3021-3026,3055-3062,3066-3086`
- `.../MainForm.Movie.cs:174-297`
- `src/BizHawk.Client.EmuHawk/tools/ToolManager.cs:747-790`
- `src/BizHawk.Emulation.Common/Interfaces/Services/IDebuggable.cs:12-45`
- `src/BizHawk.Emulation.Common/Interfaces/Services/IDisassemblable.cs:11-33`
- `src/BizHawk.Emulation.Cores/Consoles/Nintendo/NES/NES.IDebuggable.cs:6-20`
- `.../Nintendo/BSNES/BsnesCore.IDebuggable.cs:59-70`
- `.../Nintendo/SNES/LibsnesCore.IDebuggable.cs:53-62`
- `.../Nintendo/N64/N64.IDebuggable.cs:79-92`
- `.../Sony/PSX/Octoshock.IDebuggable.cs:79-90`
- `.../Nintendo/QuickNES/QuickNES.IDebuggable.cs:29`
- `.../Computers/AppleII/AppleII.IDebuggable.cs:92-103`
- `.../Computers/Commodore64/MOS/Chip6510.IDebuggable.cs:12-23`
- `.../Computers/Commodore64/Serial/Drive1541.IDebuggable.cs:53-64`
- `.../Sega/GGHawkLink/GGHawkLink.IDebuggable.cs:135-144`
- `src/BizHawk.Emulation.Common/Base Implementations/LinkedDebuggable.cs:50-57`
- `src/BizHawk.Client.Common/Api/Interfaces/IEmuClientApi.cs:50-80`
- `src/BizHawk.Client.Common/Api/Classes/EmuClientApi.cs:106-142`
- `src/BizHawk.Client.EmuHawk/tools/Debugger/GenericDebugger.cs:41-145,175-205`
- `.../Debugger/GenericDebugger.IToolForm.cs:6-99`
- `.../Debugger/BreakpointControl.cs:56-70,162-176`
- `src/BizHawk.Client.Common/BreakpointList.cs:66-166`

**Lua / callbacks / tracing**

- `src/BizHawk.Client.Common/lua/ApiGroup.cs:3-24`
- `.../lua/LuaLibraries.cs:21-40,42-128,193-210,241-265,291-321,345-386,420-476`
- `.../lua/LuaFile.cs:14-117`
- `.../lua/LuaSandbox.cs:44-84`
- `.../lua/EnvironmentSandbox.cs:4-11`
- `.../lua/LuaHelperLibs/EventsLuaLibrary.cs:36-61,80-110,120-295`
- `.../lua/CommonLibs/SaveStateLuaLibrary.cs:15-17`
- `.../lua/CommonLibs/MemorySavestateLuaLibrary.cs:17-31`
- `src/BizHawk.Client.EmuHawk/tools/Lua/LuaConsole.cs:603-621`
- `src/BizHawk.Emulation.Common/Interfaces/IMemoryCallbackSystem.cs:7-128`
- `.../Base Implementations/MemoryCallbackSystem.cs:69-113`
- `.../Base Implementations/InputCallbackSystem.cs:9-57`
- `.../Interfaces/Services/ITraceable.cs:3-38`
- `.../Base Implementations/TraceBuffer.cs:5-16`
- `.../Base Implementations/CallbackBasedTraceBuffer.cs:5-17,47-63`
- `src/BizHawk.Client.EmuHawk/tools/TraceLogger.cs:105-193`
- `src/BizHawk.Emulation.Common/Interfaces/Services/ICodeDataLogger.cs:10-75`
- `src/BizHawk.Client.EmuHawk/CoreFeatureAnalysis.cs:16-91,147-160`

**Tests**

- `src/TestProjects.props:1-16`
- `src/MainSlnTest.props:1-20`
- `src/MainSln.runsettings:1-7`
- `BizHawk.sln:38-42`
- `Dist/.InvokeCLIOnMainSln.sh:8-14`
- `Dist/.BuildTestInConfigX.sh:1-10`
- `.github/workflows/ci.yml:27-69`
- `.gitlab-ci.yml:213-226`
- `src/BizHawk.Tests.Client.Common/Movie/FakeEmulator.cs:8-76`
- `.../Movie/FakeMovieSession.cs:9-78`
- `.../Movie/FakeStateManager.cs:9-34`
- `.../lua/LuaTestContext.cs:11-97`
- `src/BizHawk.Tests.Emulation.Common/Base Implementations/MemoryCallbackSystemTests.cs:25-140`
- `src/BizHawk.Tests.Testroms/DummyFrontend.cs:14-193`
- `src/BizHawk.Tests.Testroms/TestUtils.cs:28-83`
- `src/BizHawk.Tests.Testroms/ImageUtils.cs:61-82`
- `src/BizHawk.Tests.Testroms.GB/AcidTestroms.cs:92-120`
- `src/Testroms.props:1-21`

---

## Appendix B. Verification log

Commands run against the pinned clone (no network, no builds):

```text
git rev-parse HEAD
  -> d148601a4629c2c230e41647d7c94eee794011c7
git rev-parse --abbrev-ref HEAD
  -> master
git show -s --format=%cI HEAD
  -> 2026-11-10T17:07:36-06:00
git status --porcelain
  -> (empty)
git submodule status
  -> 32 entries, all prefixed "-" (uninitialized)
grep -c '\[submodule' .gitmodules
  -> 32
grep -rn '\[CoreAttribute\|\[PortedCore' src/BizHawk.Emulation.Cores --include=*.cs | wc -l
  -> 39 core declarations
grep -rn '\[TestMethod\]' src/BizHawk.Tests.* --include=*.cs | wc -l
  -> 337 test methods across 43 files
grep -rn 'IDebuggable' src/BizHawk.Emulation.Cores/Arcades/MAME --include=*.cs | wc -l
  -> 0
grep -rn 'ITraceable' src/BizHawk.Emulation.Cores/Arcades/MAME --include=*.cs | wc -l
  -> 0
grep -rn 'IDebuggable' src/BizHawk.Emulation.Cores/Libretro --include=*.cs | wc -l
  -> 0
```

No tests were executed and no native submodule was built; performance claims
and runtime behaviour are therefore source-level observations, not measurements.
