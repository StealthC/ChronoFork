# Upstream Licensing and Provenance Assessment

Status: engineering assessment for planning. **This is not legal advice.** Every
"clear", "conditional" or "blocked" label below describes what the checked-out
evidence supports for engineering decisions, not a legal conclusion. Unclear
cases are listed as decisions for the user (see [Open questions](#open-questions-for-user-decision))
rather than silently resolved.

- Audit date: 2026-11-13
- Auditor: `quality-licensing` (ChronoFork bootstrap team)
- Scope: the three research checkouts under `.research/upstream/`
- Repository state: all three are shallow clones (`git rev-parse --is-shallow-repository`
  returns `true`, one commit of history each). They are research inputs only and are
  git-ignored by ChronoFork.

The durable policy and provenance records are `docs/licensing/policy.md` and
`docs/licensing/provenance.md`; this document supplies the upstream evidence and the
open decisions behind them.

## 1. Pinned revisions

These are the exact trees assessed. A conclusion here does not automatically apply to a
different commit.

| Project | Path | Commit | Commit date | Branch | Upstream |
|---|---|---|---|---|---|
| Exodus | `.research/upstream/exodus` | `08f388f77040af28d16d44fdfbddb73252953161` | 2024-02-16 | `master` | <https://github.com/RogerSanders/Exodus> |
| exodus-for-ais (Linux headless) | `.research/upstream/exodus-headless` | `ec0056151f3ff1c334d09ee375abb30a68fdfec5` | 2026-08-13 | `linux-headless` | <https://github.com/paulomanrique/exodus-for-ais> |
| BizHawk | `.research/upstream/bizhawk` | `d148601a4629c2c230e41647d7c94eee794011c7` | 2026-11-10 | `master` | <https://github.com/TASEmulators/BizHawk> |

Repository-level license identifiers as declared by the projects:

| Project | File | SHA-256 | Declared license |
|---|---|---|---|
| Exodus | `License.txt` | `be174adf3443ebe3169a1a9bddb19241fbb81b583f5db57ee5bf74fc0899cce9` | MIT ("Copyright (c) 2024 Roger Sanders") |
| exodus-for-ais | `License.txt` | `be174adf3443ebe3169a1a9bddb19241fbb81b583f5db57ee5bf74fc0899cce9` | MIT, byte-identical to Exodus |
| BizHawk | `LICENSE` | `6601cab56cb56875001906593a3adf94d52a0b82e6919fcbee81c31a48499121` | MIT for the BizHawk team's original work only; the file itself describes the repository as a heterogeneous, partly incompatible combination |

### 1.1 ChronoFork's own outbound licensing files (observed 2026-11-13)

The bootstrap work added outbound licensing files while this audit was running. They are
recorded here for consistency; this document does not edit them.

| File | Content observed | Consistency notes |
|---|---|---|
| `LICENSE` | MIT, "Copyright (c) 2026 ChronoFork contributors" | Matches `Cargo.toml` (`license = "MIT"`); no upstream code is claimed |
| `THIRD_PARTY_NOTICES.md` | 10 third-party Rust crates (`sha2` + transitives), all MIT/Apache-2.0; three research references explicitly marked **not distributed**; no ROM/firmware fixtures | The 10 packages and versions match `Cargo.lock` exactly; the research-reference statements are consistent with this assessment |
| `README.md` | States that the repository ships a deterministic synthetic test backend and a minimal 68000 + ROM/RAM Mega Drive backend, neither a complete emulator; 40 tests and no ROMs | Consistent with the code inventory: no upstream emulator source or assets are present in the workspace |

The cross-referenced `docs/licensing/policy.md` and `docs/licensing/provenance.md` were
created later in the same bootstrap day and record the same research-only posture
(MIT provisional, no upstream code incorporated, BizHawk not reusable, SHA-pinned
references). Keep those files, `THIRD_PARTY_NOTICES.md` and this assessment in sync; this
assessment is the detailed evidence behind them, not a duplicate policy.

## 2. Method and evidence limits

What was actually inspected:

- Every repository-level and named license/notice file found by an exhaustive file-name
  sweep, plus the license-bearing subdirectories listed in §4.
- The full tree shape and file inventories (tracked files, extensions, per-directory
  license presence).
- Per-file copyright/license headers: first five lines of every `.h`, `.cpp`, `.inl`
  (Exodus: 911 files) and every tracked `.cs` under BizHawk `src/` (2,416 files).
- Fork delta: a tree diff between the Exodus and exodus-for-ais checkouts (65 modified
  upstream files, plus added directories), and the fork's README/commit metadata.
- Build-integration files that name external dependencies (`Third/build instructions.txt`,
  `CMakeLists.txt`, `vcpkg.json`, `Directory.Packages.props`, `.gitmodules`).

Evidence limits that matter for the conclusions:

1. **Shallow clones.** Only the pinned commit is present. Tags, full authorship history,
   and prior license states cannot be checked. For exodus-for-ais, history is a single
   squashed commit by Paulo Manrique, so the upstream base commit and the exact extent of
   the fork cannot be verified from VCS metadata. The 65-file tree diff against the
   pinned Exodus commit is circumstantial evidence, not proof of lineage.
2. **BizHawk submodules are not initialized.** `git submodule status` lists 32 entries,
   all with the `-` prefix (absent), and their directories are empty. The pinned gitlink
   SHAs are recorded there, but the submodule sources and licenses are not in this
   checkout and were not assessed.
3. **Almost no per-file headers.** No Exodus source file carries a copyright/license
   header in its first five lines (0 of 911). Exactly one of BizHawk's 2,416 `src/**/*.cs`
   files does (`Computers/Commodore64/FFT.cs`, an Ooura FFT attribution). Elsewhere the
   only license evidence is the repository-level file plus scattered comments. That makes
   file-by-file provenance weaker than the repo-level license suggests.
4. **Comments are not proof.** Attribution comments ("based on", "logic copied from") are
   treated here as red flags that require review, not as verified provenance records.
5. **No web verification of third-party licenses at pinned SHAs** was performed except
   where noted (the MAME licensing history in Q4). License names for external,
   unvendored dependencies are taken from the projects' own declarations and standard
   knowledge; they should be re-checked at distribution time.

## 3. Bottom line by project

| Project | Assessment | Reuse posture |
|---|---|---|
| Exodus | Clear MIT at repository level, with one material provenance caveat in the YM2612 tables and no per-file headers. | **Conditional.** May be used as a reference and may be ported with the MIT notice retained. Do not copy the YM2612-derived tables without resolving Q4. |
| exodus-for-ais | MIT-informed derivative of Exodus. Fork additions are stated to be "under the same licence" in the README, but the copyright holder of the additions is not recorded in `License.txt`, and lineage is unverifiable from this checkout. | **Conditional.** Usable under the same notice obligations as Exodus, plus attribution of the fork's additions; confirm Q2/Q7. |
| BizHawk | Heterogeneous and partly copyleft/restricted. The repository's own `LICENSE` calls the combination "an illegal combination of several incompatible GPL licenses"; the README says anything outside `src/` is not the team's to license. Inspection additionally found GPL-derived code *inside* `src/`. | **Do not incorporate.** Reference for architecture, TAS/debug/trace concepts, and testing methodology only. Do not copy source, fixtures, or binaries. |

For ChronoFork's stated MIT-preferred distribution, the only upstream project that is
both relevant to this project and able to contribute copyrightable expression is Exodus
(or its headless fork), and only with notices and the YM2612 question resolved. BizHawk
must remain a conceptual reference.

## 4. Component-level assessment

Legend for **Reuse status**:

- **CLEAR** — permissive and evidenced; use with the stated notice.
- **CONDITIONAL** — usable only after the listed notice/provenance condition is met.
- **BLOCKED** — provenance or license conflict must be resolved before any copying.
- **DO NOT USE** — do not copy, link, redistribute, or derive from this component.
- **NOT ASSESSED** — not present in the checkout; cannot be evaluated here.

### 4.1 Exodus (`08f388f7…`)

| Project | Path | Copyright holder / provenance | License and evidence | Reuse status | Required notices | Open questions |
|---|---|---|---|---|---|---|
| Exodus | `License.txt` | Roger Sanders | MIT, explicit, 2024 | CLEAR | Reproduce the MIT copyright + permission notice in copies/substantial portions | — |
| Exodus | `Exodus/`, `ExodusSDK/`, `System/`, `Devices/`, `Extensions/`, `Support Libraries/` | Roger Sanders; original implementation from hardware documentation and research | MIT per repo-level `License.txt` only; no per-file headers | CONDITIONAL | If ported or translated, retain the MIT notice and identify modifications | Q1, Q5 |
| Exodus | `Data/Modules/*.xml`, `Exodus/Resources/`, `Devices/**/*.rc`, documentation XML | Roger Sanders (unless noted in-file) | Covered by the repo-level MIT license; no separate asset license | CONDITIONAL | Same MIT notice; asset provenance should be recorded if extracted | Q3 |
| Exodus | `Devices/YM2612/` — envelope/attenuation/detune tables and related algorithm comments | Roger Sanders; comments state the tables are "primarily based on the work of Jarek Burczynski in his OPN envelope generator implementation for the MAME core" (`Devices/YM2612/YM2612.cpp:9-11`; references at `YM2612.h:38-48`) | `License.txt` says MIT. The cited MAME contributions were explicitly placed under **GPL-2.0+** by MAME at Burczynski's request (MAME commit `ec5b9de`, 2016), and MAME's relicensing does not retroactively apply to older revisions | **BLOCKED** for direct copying of the tables/algorithms | If independently derived, document sources; if copied/translated, the MIT notice alone may be insufficient | Q4 |
| Exodus | `Devices/M68000/` | Roger Sanders; uses Motorola manuals, public research notes, and a BCD-behaviour note from flamewing (`ABCD.h`, `SBCD.h`, `NBCD.h`) | MIT repo-level; no third-party code headers found | CONDITIONAL | MIT notice if ported; credit research references in provenance notes | Q5 |
| Exodus | `Support Libraries/` (MarshalSupport, Stream, ThreadLib, etc.) | Roger Sanders | MIT repo-level; no third-party headers found | CONDITIONAL | MIT notice if ported | Q5 |
| Exodus | `Third/` (expat, zlib, libpng, libjpeg, libtiff, Catch, HTML Help) | Various third parties | **Not vendored.** `Third/` contains only Visual Studio project files and placeholders; `Third/build instructions.txt` names expat 2.1.0, zlib 1.2.8, libpng 1.6.12, libjpeg 9a, libtiff 4.0.9, Catch, and Microsoft HTML Help Workshop as downloads | CLEAR as build-time references; no third-party code is redistributed by this repository | Only if a future product links/distributes those libraries: retain their notices (Expat MIT, zlib license, libpng license, IJG, libtiff, BSL-1.0 for Catch) | Q6 |
| Exodus | `ExodusTests.sln`, `Support Libraries/MarshalSupport/Tests/` | Roger Sanders | MIT; tests only cover marshalling (unit/performance), not the emulator | CLEAR for methodology | MIT notice if tests are copied; prefer reimplementation | Q5 |
| Exodus | `.hgignore`, build props, solution files | Roger Sanders | MIT | CLEAR | MIT notice if copied | — |

### 4.2 exodus-for-ais (`ec005615…`)

The fork's delta against the pinned Exodus tree is: **65 modified upstream files** (port
work concentrated in `Support Libraries/ThreadLib` 11, `Support Libraries/Stream` 7,
`HierarchicalStorage`/`Debug` 4 each, plus `System/`, `Devices/`, `ExodusSDK/`, and
`Extensions/` changes), plus added `host/` (13 files), `mcp/` (30), `tests/` (9),
`skills/` (4), `cmake/` (3), `_PortReports/` (3), `CMakeLists.txt`, `README.md`,
`AGENTS.md`, `vcpkg.json`, and two Windows batch files. The tree diff is not a substitute
for history (see §2).

| Project | Path | Copyright holder / provenance | License and evidence | Reuse status | Required notices | Open questions |
|---|---|---|---|---|---|---|
| exodus-for-ais | `License.txt` | Roger Sanders | MIT; byte-identical to Exodus | CLEAR | Reproduce the MIT notice | — |
| exodus-for-ais | `README.md` §"Licence and credit" (`README.md:149-162`) | Fork author's statement; commit author Paulo Manrique | States Exodus is MIT and that the fork's additions are "under the same licence"; lists `host/`, `mcp/`, `tests/equivalence/`, skills/AGENTS, Windows build files and Linux port work | CONDITIONAL | Retain the upstream MIT notice **and** attribute the fork's additions; the fork author's copyright line is not in `License.txt` | Q2, Q7 |
| exodus-for-ais | 65 modified upstream files (portability and Linux port) | Roger Sanders + fork modifier | Derivative of MIT code; README says fork work is under the same license; no per-file change notices | CONDITIONAL | MIT notice for the upstream code; mark modifications and credit the fork | Q2, Q7 |
| exodus-for-ais | `host/`, `mcp/`, `tests/`, `skills/`, `AGENTS.md`, `CMakeLists.txt`, `_PortReports/` | Fork author/contributors | MIT per README statement; original code as far as inspection shows (JSON/codec/transport written in-tree, no third-party headers found) | CONDITIONAL | Same as above; retain fork attribution | Q2, Q8 |
| exodus-for-ais | `skills/` (three `megadrive-*` skills) | Fork author; commit message says they were previously in a private repository | README says added under the same license; no separate license file | CONDITIONAL | Attribute the author; confirm prior publication rights | Q8 |
| exodus-for-ais | `tests/rom/make_test_rom.py`, `tests/rom/rom_test.py`, generated ROM | Fork author | Original generated fixture, no ROM shipped (`AGENTS.md:103-108`) | CLEAR | MIT notice if copied | Q13 |
| exodus-for-ais | Build dependencies: `CMakeLists.txt`, `vcpkg.json`, `cmake/vcpkg-ports/expat` | Third parties: Expat, zlib, libpng, libjpeg/libjpeg-turbo | **Not vendored.** `find_package`/`pkg_check_modules` against system libraries; vcpkg baseline pins dependencies; local Expat port only changes build options and installs upstream `COPYING` | CLEAR as build-time references | If distributed binaries statically link them, include their notices (Expat MIT, zlib, libpng, IJG/libjpeg-turbo) | Q6 |
| exodus-for-ais | `Data/` modules and upstream assets | Roger Sanders / upstream | MIT via upstream | CONDITIONAL | MIT notice | Q3 |

### 4.3 BizHawk (`d148601a…`)

Global evidence:

- `LICENSE:1-7` distinguishes the team's MIT C# work, submodules with licenses, submodules
  without licenses, team customizations, and compiled binaries with neither license nor
  origin. `LICENSE:3` states the repository as-is "should be considered an illegal
  combination of several incompatible GPL licenses."
- `README.md:467-471`: "anything outside `/src` isn't ours and we can't give you
  permission to share, use, or sell it".
- 32 git submodules are declared in `.gitmodules`; all are absent from this checkout.
- Only 29 tracked files have a license/notice/author name (in `LICENSE`,
  `ExternalProjects/`, `libmupen64plus/`, `lynx/`, `blip_buf/`, `waterbox/`, and two font
  resources under `src/`); most bundled components have none.
- The global boundary is therefore **not** a clean MIT/third-party split. Even `src/`
  contains explicitly GPL-derived code.

| Project | Path | Copyright holder / provenance | License and evidence | Reuse status | Required notices | Open questions |
|---|---|---|---|---|---|---|
| BizHawk | `LICENSE`, BizHawk team's original frontend/framework work | BizHawk team | MIT, explicit; applies only where identified | DO NOT USE (no incorporation planned) | None if no code/text is copied; if a snippet is used, its notice applies | Q9 |
| BizHawk | `src/` generally (BizHawk C# framework, tests) | BizHawk team and many named contributors | MIT claimed for the team's work; **no per-file headers**; contains third-party-derived files listed below | DO NOT USE; reference only | None if reference only | Q9, Q10 |
| BizHawk | `src/BizHawk.Emulation.Cores/Consoles/Nintendo/NES/Boards/NSFBoard.cs:3`, `.../UNIF/UNIF_UNL-AX5705.cs:5` and other NES files | BizHawk team; comments say derived/copied from FCEUX (GPLv2) | MIT claim conflicts with GPL-derived provenance; no FCEUX license file in `src/` | **DO NOT USE** | N/A | Q10 |
| BizHawk | `src/BizHawk.Emulation.DiscSystem/Internal/Algorithms/GPL_ECM.cs:1-6` | Comment: "stolen somewhat from neill corlett's code… almost certainly some kind of GPL"; lineage via Mednafen | Self-labelled GPL; code is inside `#if false` but is still distributed in source | **DO NOT USE** | N/A | Q10 |
| BizHawk | `src/BizHawk.Emulation.DiscSystem/Internal/Algorithms/ECM.cs`, `.../DiscFormats/CHD_format.cs` (CRC table from MAME, BSD-3 at the cited commit) | BizHawk team; MAME for the noted table | Active ECM code is described as independently written with comparisons to MAME/Corlett; the CHD CRC table has an explicit MAME BSD-3 provenance comment | DO NOT USE without file-level audit | N/A | Q10 |
| BizHawk | `src/BizHawk.Emulation.Cores/Computers/SinclairSpectrum/.../NECUPD765.*`, `.../Fairchild/ChannelF/Cart/VesCartBase.cs:33` | BizHawk team; VesCart section explicitly from MAME BSD-3 (Fabio Priuli) | Mixed: datasheet-based plus BSD-3 snippet | DO NOT USE | N/A | Q10 |
| BizHawk | `src/BizHawk.Emulation.Cores/CPUs/Z80A/Z80A.cs:627-691`, `.../Atari/jaguar/JaguarDisassembler.cs:23`, `.../Nintendo/BSNES/BsnesCore.cs:130` | BizHawk team; comments cite Fuse (GPLv2+), Virtual Jaguar (GPL), bsnes (GPLv3) | Mixed; comments only, no license headers | **DO NOT USE** | N/A | Q10 |
| BizHawk | `src/BizHawk.Emulation.Cores/Computers/Commodore64/FFT.cs` | Takuya Ooura; C# port by J.D. Purcell | Custom permissive notice: use/copy/modify for any purpose without fee; no full license text or warranty disclaimer | DO NOT USE | N/A | Q10 |
| BizHawk | `src/BizHawk.Client.Common/Resources/fceux.license.txt`, `gens.license.txt` | feos (FontStruction) | CC-BY 3.0 for fonts | DO NOT USE | Attribution if ever used | Q10 |
| BizHawk | `libmupen64plus/*` (core, RSP HLE, Glide64, Glide64mk2, Rice) | Mupen64Plus/Mupen64 authors and contributors | GPLv2 per local `LICENSES`/`COPYING`; some components have their own docs | **DO NOT USE** | N/A | Q11 |
| BizHawk | `waterbox/picodrive/` | PicoDrive authors | Restrictive license (`COPYING`): redistribution may not be sold or used in a commercial product; modified redistribution must include complete source. Not an OSI-approved permissive license | **DO NOT USE**; incompatible with an unrestricted MIT-preferred distribution | N/A | Q11 |
| BizHawk | `blip_buf/` | blargg / Blip_Buffer authors | LGPL-2.1 per local `license.txt` | **DO NOT USE** (unless independent reimplementation) | N/A | Q11 |
| BizHawk | `lynx/` | K. Wilkins | zlib-style permissive with attribution + altered-source marking requirements | DO NOT USE (reference only) | N/A | Q11 |
| BizHawk | `waterbox/tic80/` (+ vendored moonscript, wasm3, wren) | Vadim Grigoruk et al. | MIT per local `LICENSE`; vendor licenses present | DO NOT USE | N/A | Q11 |
| BizHawk | `waterbox/ares64/ares/` | ares team, Near et al. | ISC-style permissive per local `LICENSE`; third-party sljit license present | DO NOT USE | N/A | Q11 |
| BizHawk | `psx/`, `wonderswan/`, `quicknes/`, `libHawk/`, `encore/`, `ExternalCoreProjects/`, `ExternalToolProjects/` | Various (Mednafen-derived, QuickNES, Virtu, ppsspp wrapper, etc.) | No local license file for several of these components; Mednafen lineage implies GPL for at least `psx/` and `wonderswan/` but the checkout does not prove it | NOT ASSESSED / DO NOT USE | N/A | Q11 |
| BizHawk | 32 submodules (`.gitmodules`) | Various (SDL, libusb, mgba, SameBoy, Gambatte, MAME, mednafen, ppsspp, etc.) | **Absent** (uninitialized); pinned gitlink SHAs known, licenses unknown here | NOT ASSESSED; do not incorporate | N/A | Q11 |
| BizHawk | `ExternalProjects/NLua`, `iso-parser`, `PcxFileTypePlugin.HawkQuantizer`, `libchd-rs-capi`, `FlooohChips` | Various | NLua MIT, iso-parser MIT, Pcx has `License.md`, others not assessed in depth | DO NOT USE | N/A | Q11 |
| BizHawk | `Assets/` (Lua scripts, shaders, palettes, fonts), `Assets/dll/` prebuilt binaries | Many authors; no comprehensive license inventory | Mixed/unclear; `Assets/Shaders/shaders-readme.txt` is technical, not a license; some Lua files mention licenses, most have no header | **DO NOT USE / do not redistribute** | N/A | Q10, Q11 |
| BizHawk | `Dist/` helper binaries (`7za.exe`, `ILMerge.exe`, `upx.exe`, `zip.exe`, `unzip.exe`) | Third parties; no license files or origin statements in tree | Not verified; treat as mixed third-party tooling | **DO NOT USE** | N/A | Q11 |
| BizHawk | `References/` prebuilt DLLs | BizHawk team + NLua/ISOParser/FlatBuffers | Mixed; no notices alongside binaries | DO NOT USE | N/A | Q11 |
| BizHawk | `Directory.Packages.props` (50 pinned NuGet packages) | Third parties | External, not vendored; licenses not audited here and vary by package | CLEAR as external references only; do not adopt the stack | Not applicable unless adopted | Q9 |
| BizHawk | `src/BizHawk.Tests.Client.Common/data/dearchive/m3_scy_change.gb` and its archives | Mealybug Tearoom Tests (Matt Currie) | MIT per `data/dearchive/readme.md`; provenance and source link recorded; SHA-1 asserted in test | DO NOT COPY; methodology is reusable | If ever used, include the MIT notice + provenance | Q13 |
| BizHawk | Test ROMs/firmware referenced by `src/BizHawk.Tests.Testroms*` | Various test-ROM authors; firmware is user-supplied | Not committed; downloaded from `libre-roms-ci` CI artifacts or copied manually; firmware explicitly user-supplied | DO NOT COPY/REDISTRIBUTE; adopt the external-artifact pattern | If adopting any suite, verify its own license at source | Q13 |

## 5. What can and cannot be safely incorporated

**Reusable under MIT with notice (after Q4 is resolved):**

- Original Exodus source and data as a reference for behaviour, and — if the project
  chooses translation over clean-room implementation — translated code, provided the
  Roger Sanders MIT notice is retained and modifications are recorded.
- exodus-for-ais additions, provided the fork's authorship/licence statement is retained
  (Q2) and lineage is confirmed (Q7).
- Ideas, formats, and interface concepts (e.g. module XML shape, reflection model, trace
  log fields) are not restricted by the MIT license in the same way as expression, but
  re-expressing them in Rust still requires care: paraphrase and reimplement rather than
  transliterate files. This is an engineering distinction, not a legal safe harbour.

**Not safe to incorporate without a separate decision:**

- The YM2612 envelope/detune tables and algorithms in `Devices/YM2612` (Q4).
- Any BizHawk code inside or outside `src/` (GPL-derived code, unlicensed submodules,
  mixed assets).
- Any BizHawk bundled binary, asset, shader, Lua script, test ROM, or firmware.
- Any third-party code reached only through a package manager unless its license is
  checked at the pinned version (Q6).
- Upstream test fixtures, traces, screenshots, ROMs, BIOS images, and extracted assets.

**Translation does not remove copyright.** Renaming identifiers, changing a language,
splitting functions, or porting C++/C# to Rust creates a derivative work when it follows
the original's expression. MIT still requires the copyright and permission notice for
copies or substantial portions. A genuinely independent implementation from specifications
and hardware documentation is a different provenance story, but it must be documented as
such: keep design notes showing what was read, avoid having the source open while writing
corresponding code where practical, and never describe a port as "clean-room" if it was
not.

## 6. Notice obligations if Exodus/fork code is used

If ChronoFork distributes translated copies or substantial portions of Exodus or
exodus-for-ais code, the minimum engineering checklist is:

1. Ship the full MIT text and copyright line for Roger Sanders exactly as in
   `License.txt`.
2. Attribute the headless fork's additions (Paulo Manrique / exodus-for-ais) and state
   that ChronoFork modified/translated them, unless the user decides to avoid fork code
   entirely.
3. Keep the existing `THIRD_PARTY_NOTICES.md` current: it must list every upstream
   component, its copyright holder, license, source URL and pinned revision.
4. Add SPDX headers (or a machine-readable manifest) to files that contain translated or
   adapted upstream expression, with the upstream file/revision noted.
5. Do not ship upstream ROMs, BIOS images, savestates, screenshots of copyrighted games,
   or the upstream test fixtures. ChronoFork's own fixtures follow the testing strategy's
   fixture policy.
6. If Expat/zlib/libpng/libjpeg are ever linked into a distributed binary, include their
   notices as well (Q6).

## 7. Open questions for user decision

| ID | Question | Why it matters | Engineering default if the user does not decide |
|---|---|---|---|
| Q1 | Is MIT the outbound license for all crates and docs, and what notice file layout is required? (`Cargo.toml` declares `license = "MIT"`; the repository now also has `LICENSE` and `THIRD_PARTY_NOTICES.md`.) | Determines the notice obligations for any upstream translation and the repository's distribution posture | Keep MIT; keep `LICENSE` + `THIRD_PARTY_NOTICES.md` in sync with this assessment |
| Q2 | May ChronoFork rely on exodus-for-ais additions under the README's "same licence" statement even though the fork author's copyright is not in `License.txt`? Should we obtain explicit confirmation? | The MIT notice must name the copyright holder; the fork's additions are otherwise unattributed | Treat as conditional; prefer Exodus-only provenance until confirmed |
| Q3 | Are Exodus data files, icons, module XML and documentation covered by the repo-level MIT license, or do any have separate terms? | Assets are copied as easily as code and are not separately licensed in the tree | Treat as MIT-covered but record each asset's provenance if imported |
| Q4 | How should ChronoFork handle the YM2612 tables that Exodus says are based on Jarek Burczynski's MAME implementation, given MAME's GPL-2.0+ marking? Is audio in scope for the first backend? | This is the one material conflict inside otherwise-MIT Exodus code; direct copying could impose GPL obligations or require removal | Do not copy the tables; if audio is needed, derive independently from documentation or use a permissively licensed implementation as an oracle, with its own notice; get legal review |
| Q5 | Will ChronoFork port/translate Exodus code, or reimplement behaviour from specifications? | Determines whether MIT notices and derivative-work records are needed at all | Prefer clean-room reimplementation with an explicit provenance log; if porting, follow §6 |
| Q6 | What is the distribution/linking model for any C/C++ dependencies (system libraries vs static bundles)? | Static bundling triggers notice obligations for Expat/zlib/libpng/libjpeg; dynamic linking may not, depending on interpretation | Avoid bundling; if bundling, include notices |
| Q7 | Can the exodus-for-ais fork's upstream base be confirmed (full history or upstream diff), and is the fork's license grant valid for all added files? | The checkout is a single squashed commit; we cannot prove the base or the grant | Ask the fork author for the base commit and an explicit MIT grant/notice for additions |
| Q8 | Were the `megadrive-*` skills previously published elsewhere under different terms? | Commit message says they came from a private repository; "vendored here" may not mean authored here | Treat as fork code under MIT; confirm before copying skill text |
| Q9 | Confirm BizHawk is reference-only: no source, no binaries, no assets, no fixtures, no test archives. | BizHawk's own LICENSE warns of incompatible GPL combinations and unlicensed binaries | Enforce a no-copy rule in review and in the provenance manifest |
| Q10 | If any BizHawk `src/` idea is ever reused, which files require a line-by-line license audit? | `src/` contains FCEUX-derived, GPL-labelled, MAME-BSD, Ooura and other third-party code despite the MIT claim | Keep the no-copy rule; if that changes, audit the specific files first |
| Q11 | Do we need a submodule license inventory for BizHawk even though we will not use it? | 32 submodules are uninitialized and several are GPL/LGPL/restricted; a future decision could be based on a false assumption | Record as "not assessed"; do not rely on any BizHawk component |
| Q12 | When is legal review required (before public release, before binary distribution, before accepting contributions)? Which jurisdiction? | This document is an engineering assessment, not legal advice | Get review before the first public binary release and before any upstream code translation is merged |
| Q13 | Are third-party test ROMs acceptable as fixtures if permissively licensed and recorded in a manifest, or should ChronoFork use only generated programs? | Affects test coverage and redistribution risk | Generated programs first; permissive third-party ROMs only with license + provenance + hash, and only after review |
| Q14 | How should upstream names/trademarks (Exodus, BizHawk, Sega Mega Drive/Genesis) be used in docs and package metadata? | Compatibility claims and names can raise trademark issues separate from copyright | Use nominative references only ("compatible with…"), no logos, no implied endorsement; confirm before release |

## 8. Evidence appendix

### 8.1 Commands and observations

| Observation | Command / source |
|---|---|
| Pinned commits and dates | `git -C <repo> log -1 --format='%H %ci %s'` |
| Shallow clones, one commit | `git rev-parse --is-shallow-repository`; `git rev-list --count HEAD` |
| License files | exhaustive `find`/`git ls-files` name sweep for `licen*`, `copying*`, `notice*`, `authors*` |
| Header scarcity | first five lines of 911 Exodus C/C++ files and 2,416 BizHawk `src/**/*.cs` files; 0 and 1 matches respectively |
| Fork delta | `diff -rq --exclude=.git exodus exodus-headless`: 65 modified files, 14 added top-level entries/directories |
| BizHawk submodules | `git submodule status`: 32 entries, all absent (`-` prefix) |
| BizHawk license inventory | 29 tracked license/notice/author files; 6,808 tracked files; 2,577 tracked `.cs` |
| Charged fixtures | `git ls-files` filtered for ROM/archive extensions (excluding Markdown false positives): only the MIT Mealybug GB fixture and its archives, one helper-scripts tarball, and `interop.bin` |

### 8.2 Key quoted evidence (short excerpts only)

- Exodus `License.txt:1-3`: MIT License, "Copyright (c) 2024 Roger Sanders".
- exodus-for-ais `README.md:151-162`: "Exodus is by Roger Sanders and is MIT
  licensed… Added by this fork, under the same licence:" followed by the fork's added
  directories. The same file says no ROMs/BIOS/disc images ship.
- Exodus `Devices/YM2612/YM2612.cpp:9-11`: tables "primarily based on the work of Jarek
  Burczynski in his OPN envelope generator implementation for the MAME core".
- BizHawk `LICENSE:1-7`: original C# work MIT; embedded submodules with and without
  licenses; "illegal combination of several incompatible GPL licenses"; readers with a
  keen interest should "assume it is a minefield".
- BizHawk `README.md:471`: "anything outside `/src` isn't ours and we can't give you
  permission to share, use, or sell it".
- BizHawk `src/.../NSFBoard.cs:3`: "NSF ROM and general approaches are heavily derived
  from FCEUX"; `UNIF_UNL-AX5705.cs:5`: "Logic copied from FCEUX".
- BizHawk `src/.../GPL_ECM.cs:2-5`: "WARNING GPL CODE … stolen somewhat from neill
  corlett's code… originally it came from mednafen".
- BizHawk `src/BizHawk.Tests.Client.Common/data/dearchive/readme.md`: fixture is from
  the MIT-licensed Mealybug Tearoom Tests, with source link and SHA-1 sanity test.
- MAME's own documentation: the project as a whole is GPL-2.0+, with many BSD-3 files;
  Burczynski's contributions were marked GPL-2.0+ at his request (MAME commit
  `ec5b9de`, 2016). Source: <https://docs.mamedev.org/license.html> and
  <https://github.com/mamedev/mame/commit/ec5b9deadee9a2f95e0691f7246a8f4663ba3ba9>.

### 8.3 What was not done

- No legal analysis, no counsel consulted, no license compatibility opinion.
- No full read of all 6,808 BizHawk files, all 32 submodules, or all NuGet package
  licenses.
- No verification of exodus-for-ais' upstream base commit or authorship history.
- No attempt to obtain permission from any upstream author.

Those gaps are intentional: this document exists to make the engineering risks visible
and to route unclear cases to the user, not to replace a legal review.
