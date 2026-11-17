# ROM Fixtures

Acceptance fixtures for the functional Mega Drive milestone (M8). Commercial ROMs are
**never committed or redistributed**: this page records the names, regions and hashes so a
missing fixture is detectable and a test can skip with an explicit reason. The ROM bytes live
only under the ignored `.research/roms/` directory of the developer's checkout.

This follows the fixture-manifest method (M8) in the [testing strategy](testing-strategy.md):
source, revision, license and SHA-256 recorded; missing artifacts skip with a reason; no
commercial ROM is ever vendored.

## Policy

- **Never commit a ROM, BIOS, firmware or game asset.** `.research/` and `*.bin` are ignored.
- The primary checkout holds the copies; **worktrees reference them by absolute path**:
  `<repo>/.research/roms/` (for example `$HOME/projects/ChronoFork/.research/roms/`). Do not
  duplicate the files per worktree.
- **Verify before a run:** `cd .research/roms && sha256sum -c SHA256SUMS`.
- A test that needs a fixture must **skip with an explicit reason** when it is absent. CI
  never downloads, caches or bundles a commercial ROM.
- Hashes are recorded; the bytes are not. The dumps below match the No-Intro database by
  CRC-32 and MD5 and are not modified, trimmed or interleaved.

## Fixtures

| Fixture | No-Intro name | System header | Region | Serial | SHA-256 |
|---|---|---|---|---|---|
| Kid Chameleon | Kid Chameleon (USA, Europe) | SEGA MEGA DRIVE | USA, Europe (header `JUE`) | `GM MK-1010 -00` | `bc08b0188fb715ae469806843a8418d4937d2098b66d04227668865b3d4d5baa` |
| Chakan | Chakan (USA, Europe) | SEGA GENESIS | USA, Europe (header `U`) | `GM MK-1047 -00` | `c4fb268b911c4226d6c7e2ec7bfbe6669b691a14c0ba006c294cad79571f02be` |

### Kid Chameleon (USA, Europe)

- **Game:** *Kid Chameleon*, Sega Mega Drive, developed by Sega Technical Institute,
  published by Sega; US release 1992-03 (catalog 1010), JP release 1992-05-29 as
  *Chameleon Kid* (a different dump, not this fixture).
- **Header:** `SEGA MEGA DRIVE`, `(C)SEGA 1991 DEC`, domestic and overseas title
  `KID CHAMELEON`, serial `GM MK-1010 -00`, region code `JUE`, I/O support `J`.
- **Size:** 1,048,576 bytes (1 MiB). **ROM end:** `0x000FFFFF`. **RAM:** `0xFF0000`–`0xFFFFFF`.
- **Checksum:** stored and computed `0x24F4` (verified). **SRAM:** none. **Device:** 3-button
  controller. **Players:** 1–2. **Sound driver:** GEMS.
- **Hashes:** CRC-32 `ce36e6cc`, MD5 `756e603b6cf9b3ebbb1b0f79d9447e43`,
  SHA-1 `28b904000b2863b6760531807760b571f1a5fc1d`,
  SHA-256 `bc08b0188fb715ae469806843a8418d4937d2098b66d04227668865b3d4d5baa`.

### Chakan (USA, Europe)

- **Game:** *Chakan* (also known as *Chakan: The Forever Man*), Sega Mega Drive, developed by
  Extended Play and Recreational Brainware, published by Sega (US/EU) and Tec Toy (BR); US
  release 1992-11 (catalog 1047).
- **Header:** `SEGA GENESIS`, `(C)SEGA 1992.OCT`, domestic and overseas title `CHAKAN`,
  serial `GM MK-1047 -00`, region code `U`, I/O support `J`.
- **Size:** 1,048,576 bytes (1 MiB). **ROM end:** `0x000FFFFF`. **RAM:** `0xFF0000`–`0xFFFFFF`.
- **Checksum:** stored and computed `0x4BBD` (verified). **SRAM:** none. **Device:** 3-button
  controller. **Players:** 1. **Sound driver:** GEMS.
- **Hashes:** CRC-32 `046a48de`, MD5 `8844a97b65994de94cf398cf4a3217c3`,
  SHA-1 `7eae088f0b15e4bcd9a6a38849df5a20446be548`,
  SHA-256 `c4fb268b911c4226d6c7e2ec7bfbe6669b691a14c0ba006c294cad79571f02be`.

## Acceptance scenarios (M8)

The two fixtures drive three escalating scenarios, in this order:

1. **Boot** — reset vectors, TMSS, header/region parse and the game's main loop reached
   headlessly; deterministic state hash after a fixed number of cycles.
2. **Intro** — run the opening sequence **with no input**. Both titles have an
   intro/attract sequence with graphics and music, so this exercises VDP rendering, DMA and
   the sound driver before gameplay. Capture deterministic frames and audio and pin their
   hashes.
3. **TAS movie** — replay a full-game input movie and verify convergence, first-divergence
   absence and completion. Movies are ignored local artifacts (`*.cfmovie`), never the ROM.

Both games use the GEMS sound driver, which makes the intro scenario a useful YM2612/PSG
validation target. Fixture-dependent tests must skip with an explicit reason when the ROM is
absent.

## Adding a fixture

1. Obtain a clean, No-Intro-matching dump; do not patch, trim or byte-swap it.
2. Copy it to `.research/roms/` using the No-Intro file name and append its SHA-256 to
   `.research/roms/SHA256SUMS`.
3. Add a row and a detail block here with the header facts, hashes and the scenarios it
   covers.
4. Never commit the ROM. Verify the copy with `sha256sum -c SHA256SUMS`.

## Sources

- No-Intro DAT (libretro-database mirror), matched by CRC-32 and MD5 for both dumps.
- [GameDB-Genesis](https://github.com/niemasd/GameDB-Genesis) metadata (`games/<crc>/…`):
  titles, region support, serial, release date, publisher, header checksum.
- Sega Retro: [Kid Chameleon](https://segaretro.org/Kid_Chameleon) and
  [Chakan (Mega Drive)](https://segaretro.org/Chakan_(Mega_Drive)) for release dates,
  publishers, developers and the GEMS sound driver.
- The ROM headers themselves, parsed from the verified dumps.
