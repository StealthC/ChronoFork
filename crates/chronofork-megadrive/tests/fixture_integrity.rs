#![allow(clippy::expect_used)]
// The parsers below fail with a diagnostic that names the offending ROM and
// field, which is what a fixture regression must report; this is test-only.
#![allow(clippy::panic)]

//! Integrity of the committed synthetic differential ROMs.
//!
//! `scripts/fixtures/*.bin` are checked in so a clean clone can run the
//! differential corpus without the pinned oracle. This test keeps them honest in
//! two independent ways that need no oracle, no network and no commercial data:
//!
//! 1. it re-runs the deterministic generator (`scripts/oracle-diff.py
//!    --dump-roms`, an offline mode that only builds the ROM bytes) and compares
//!    every generated image byte-for-byte with the committed file, checks the
//!    documented SHA-256 pin and checks that the three legacy ROMs still end in
//!    their documented marker self-loop;
//! 2. it re-derives each image's documented header facts with the crate's own
//!    [`Cartridge`] parser, so a fixture that is byte-identical but structurally
//!    broken still fails.
//!
//! Nothing here weakens an existing test: the file is additive and asserts
//! equality, never a skip. A fixture that drifts, a generator that changes
//! without the fixtures, or a stale documented marker fails the run.
//!
//! Python 3 is the generator's own runtime and therefore a prerequisite, exactly
//! as `cargo` is a prerequisite for running this suite. The test reports a
//! missing interpreter as a hard failure instead of passing by absence.

use std::path::{Path, PathBuf};
use std::process::Command;

use chronofork_megadrive::cartridge::{Cartridge, CartridgeSystem, Region};
use sha2::{Digest, Sha256};

/// First byte of the program region in every synthetic ROM.
const PROGRAM_AT: usize = 0x200;
/// The synthetic ROMs are one 4 KiB bank.
const ROM_LEN: usize = 0x1000;
/// Reset vector: initial stack pointer, identical on both comparison sides.
const RESET_SP: u32 = 0x00FF_FF00;
/// Reset vector: initial PC is the first word of the program region.
const RESET_PC: u32 = 0x0000_0200;
/// `SEGA MEGA DRIVE` signature at `0x100`.
const SYSTEM_SIGNATURE: &[u8] = b"SEGA MEGA DRIVE ";
/// Bytes at `0x110` in every synthetic ROM.
///
/// The generator's header writer puts the copyright line at `0x110` and the
/// domestic title at `0x120`; `"(C) 2026 CHRONOFORK   "` is 22 bytes, so the
/// title overwrites its last six bytes and only `"(C) 2026 CHRONOF"` survives.
/// The overlap is a cosmetic generator defect (see the W4.1 review note), not a
/// property of a real cartridge header, so the fixture test pins the bytes that
/// are actually generated: the surviving copyright prefix followed by the title.
const COPYRIGHT_PREFIX: &[u8] = b"(C) 2026 CHRONOF";
/// Serial at `0x180`.
const SERIAL: &[u8] = b"GM 00000000-00";
/// Marker self-loop encoding: `BRA.S *`.
const MARKER_LOOP: [u8; 2] = [0x60, 0xFE];

/// One committed fixture and every fact documented for it.
#[derive(Clone, Copy)]
struct Fixture {
    name: &'static str,
    /// SHA-256 of the committed bytes, mirrored from
    /// `docs/research/oracle-differential-evidence.md` and
    /// `docs/research/cpu-cycle-differential-evidence.md`.
    sha256: &'static str,
    /// Title stamped at `0x120` (domestic) and `0x150` (overseas).
    title: &'static str,
    /// Marker self-loop address quoted by the evidence pages and by every
    /// `--stop-pc` transcript. The legacy three shifted by `+2` when the shared
    /// register setup gained the `ADD.B D0,D0` that clears `X`; these are the
    /// generator's current addresses, not the pre-repair ones.
    marker: u32,
}

/// The three legacy ROMs that predate the committed fixture directory.
///
/// They are the ROMs the differential harness calls the "existing" corpus:
/// `rom_alu`, `rom_mem_vdp` and the E2-15 masked control `rom_masked`.
const LEGACY: [Fixture; 3] = [
    Fixture {
        name: "rom_alu",
        sha256: "aaa4b3d3dd866bb9dfb3c96de4dec903bd1b346dce24891bc20a874162c8996a",
        title: "CHRONOFORK DIFF ALU ROM",
        marker: 0x000354,
    },
    Fixture {
        name: "rom_mem_vdp",
        sha256: "21a6e4a611e4026cf2c70e2e66bb4694a2ddce16d80c4f65319a84e5e2f2fe25",
        title: "CHRONOFORK DIFF MEM VDP ROM",
        marker: 0x0002E2,
    },
    Fixture {
        name: "rom_masked",
        sha256: "f8cb96dc8455d96fe92fcd4870c75baffec958dba9d24a6613025bba8fc6ad44",
        title: "CHRONOFORK DIFF MASKED ROM",
        marker: 0x0002F0,
    },
];

/// Repository root, derived from this crate's manifest directory.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .to_path_buf()
}

/// Path of a committed fixture.
fn fixture_path(name: &str) -> PathBuf {
    repo_root()
        .join("scripts")
        .join("fixtures")
        .join(format!("{name}.bin"))
}

/// The generator's path, which is also the fixture test's only external tool.
fn generator_path() -> PathBuf {
    repo_root().join("scripts").join("oracle-diff.py")
}

/// One ROM as the generator's offline mode reports it.
struct GeneratedRom {
    name: String,
    marker: String,
    len: u64,
    sha256: String,
    bytes: Vec<u8>,
}

/// Re-runs the deterministic ROM generator through its offline `--dump-roms` mode.
///
/// The mode builds every synthetic image and prints one JSON object per line; it
/// never imports the oracle, builds the CLI or writes outside stdout, so this is
/// a byte-level re-execution of the generator, not a second implementation of it.
fn generate_roms() -> Vec<GeneratedRom> {
    let generator = generator_path();
    assert!(
        generator.is_file(),
        "the ROM generator is missing at {}",
        generator.display()
    );
    let output = Command::new("python3")
        .arg(&generator)
        .arg("--dump-roms")
        .output()
        .unwrap_or_else(|error| {
            panic!(
                "cannot run python3 for {}: {error}. The synthetic ROM generator is Python, \
                 and this test refuses to pass without re-running it.",
                generator.display()
            )
        });
    assert!(
        output.status.success(),
        "{} --dump-roms failed with {}: {}",
        generator.display(),
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("the generator prints UTF-8 JSON lines");
    let roms: Vec<GeneratedRom> = stdout.lines().map(parse_generated_rom).collect();
    // W4c added the twelfth ROM (``rom_exception``, the TRAP/RTE coverage ROM);
    // the W1 boot phase added the thirteenth (``rom_branch16``, the taken 16-bit
    // branch and DBcc base coverage). Both counts and the generator's corpus-size
    // guard move together.
    assert_eq!(
        roms.len(),
        13,
        "the differential corpus should still hold thirteen synthetic ROMs"
    );
    for rom in &roms {
        assert_eq!(
            rom.len as usize,
            rom.bytes.len(),
            "{}: the generator reported {} bytes but emitted {}",
            rom.name,
            rom.len,
            rom.bytes.len()
        );
        assert_eq!(
            sha256_hex(&rom.bytes),
            rom.sha256,
            "{}: the generator's own digest does not match its bytes",
            rom.name
        );
    }
    for fixture in LEGACY {
        assert!(
            roms.iter().any(|rom| rom.name == fixture.name),
            "the generator no longer builds the legacy ROM {}",
            fixture.name
        );
    }
    roms
}

/// A short, printable form of a generated JSON line for diagnostics.
///
/// A full line carries the whole 4 KiB ROM as hexadecimal, which would bury the
/// actual failure in the test output.
fn digest_line(line: &str) -> String {
    const KEEP: usize = 120;
    if line.len() <= KEEP {
        return line.to_owned();
    }
    let head = line.get(..KEEP).unwrap_or(line);
    format!("{head}... ({} bytes total)", line.len())
}

/// Parses one `--dump-roms` JSON line.
///
/// The mode is machine-only and emits exactly `name`, `marker`, `len`, `sha256`
/// and `bytes`, so a tiny reader keeps the test dependency-free instead of
/// adding a JSON crate for five fixed fields. Every field is required.
fn parse_generated_rom(line: &str) -> GeneratedRom {
    assert!(
        !line.trim().is_empty(),
        "the generator emitted a blank line"
    );
    // The needle includes the key's opening quote and the colon, so a key can
    // never match inside a quoted hex payload; the value is trimmed before the
    // opening quote is required, because the generator emits `": "` separators.
    let string_field = |key: &str, what: &str| -> String {
        let needle = format!("\"{key}\":");
        let start = line.find(&needle).unwrap_or_else(|| {
            panic!(
                "the generated line has no {key} ({what}): {}",
                digest_line(line)
            )
        }) + needle.len();
        let rest = line
            .get(start..)
            .expect("field start is in range")
            .trim_start();
        let value = rest
            .strip_prefix('"')
            .unwrap_or_else(|| panic!("the {key} value is not a string: {}", digest_line(line)));
        let end = value
            .find('"')
            .unwrap_or_else(|| panic!("the {key} value is not terminated: {}", digest_line(line)));
        value.get(..end).expect("value is in range").to_owned()
    };
    let number_field = |key: &str, what: &str| -> u64 {
        let needle = format!("\"{key}\":");
        let start = line.find(&needle).unwrap_or_else(|| {
            panic!(
                "the generated line has no {key} ({what}): {}",
                digest_line(line)
            )
        }) + needle.len();
        let rest = line
            .get(start..)
            .expect("field start is in range")
            .trim_start();
        let end = rest
            .find(|character: char| !character.is_ascii_digit())
            .unwrap_or(rest.len());
        let digits = rest.get(..end).expect("digits are in range");
        digits
            .parse()
            .unwrap_or_else(|error| panic!("the {key} value {digits:?} is not a number: {error}"))
    };
    let bytes_hex = string_field("bytes", "the ROM bytes as hexadecimal");
    assert_eq!(bytes_hex.len() % 2, 0, "hex payload has an odd length");
    let bytes = (0..bytes_hex.len() / 2)
        .map(|index| {
            let pair = bytes_hex
                .get(index * 2..index * 2 + 2)
                .expect("hex pair is in range");
            u8::from_str_radix(pair, 16)
                .unwrap_or_else(|error| panic!("{pair} is not a hex byte: {error}"))
        })
        .collect();
    GeneratedRom {
        name: string_field("name", "the ROM identity"),
        marker: string_field("marker", "the marker self-loop address"),
        len: number_field("len", "the ROM length in bytes"),
        sha256: string_field("sha256", "the generated digest"),
        bytes,
    }
}

/// Lowercase hex SHA-256 of `bytes`.
fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

/// Text of a fixed-width ASCII field, with the generator's space padding removed.
fn ascii_field(rom: &[u8], offset: usize, length: usize) -> String {
    let field = rom.get(offset..offset + length).expect("field is in range");
    let text: String = field
        .iter()
        .map(|byte| {
            if byte.is_ascii_graphic() || *byte == b' ' {
                char::from(*byte)
            } else {
                panic!("field at 0x{offset:X} is not printable ASCII")
            }
        })
        .collect();
    text.trim_end().to_owned()
}

/// `u32` big-endian at `offset`.
fn read_u32(rom: &[u8], offset: usize) -> u32 {
    let field = rom.get(offset..offset + 4).expect("word is in range");
    u32::from_be_bytes(field.try_into().expect("four bytes"))
}

/// Word-aligned offsets of every `BRA.S *` self-loop at or after `from`.
///
/// The synthetic ROMs use `BRA.S *` both for the marker that ends the measured
/// program and for the unreachable fail stubs and masked-interrupt handlers, so
/// the marker's identity is "the last one", not "the only one".
fn self_loop_offsets(rom: &[u8], from: usize) -> Vec<usize> {
    let tail = rom.get(from..).expect("the scan start is inside the image");
    tail.chunks_exact(2)
        .enumerate()
        .filter(|(_, word)| *word == MARKER_LOOP)
        .map(|(index, _)| from + index * 2)
        .collect()
}

/// Re-derives the documented header and reset facts of one fixture.
///
/// These are the properties the pinned oracle's loader reads and the ones the
/// evidence pages quote, parsed here by the crate's own header model.
fn assert_documented_image(fixture: Fixture, rom: &[u8]) {
    assert_eq!(
        rom.len(),
        ROM_LEN,
        "{}: unexpected fixture length",
        fixture.name
    );
    assert_eq!(
        read_u32(rom, 0x000),
        RESET_SP,
        "{}: reset stack pointer",
        fixture.name
    );
    assert_eq!(read_u32(rom, 0x004), RESET_PC, "{}: reset PC", fixture.name);
    // A byte-range assertion derived from the expected slice, so the range can
    // never disagree with the constant's own length.
    let has_bytes_at = |offset: usize, expected: &[u8], what: &str| {
        assert_eq!(
            rom.get(offset..offset + expected.len()),
            Some(expected),
            "{}: {what} at 0x{offset:X}",
            fixture.name
        );
    };
    has_bytes_at(0x100, SYSTEM_SIGNATURE, "system signature");
    // 0x110..0x120 is the copyright prefix the title has not overwritten; the
    // title itself starts at 0x120 and is asserted below.
    has_bytes_at(0x110, COPYRIGHT_PREFIX, "copyright prefix");
    assert_eq!(
        ascii_field(rom, 0x120, 0x30),
        fixture.title,
        "{}: domestic name",
        fixture.name
    );
    assert_eq!(
        ascii_field(rom, 0x150, 0x30),
        fixture.title,
        "{}: overseas name",
        fixture.name
    );
    assert_eq!(
        ascii_field(rom, 0x180, 0x0E),
        String::from_utf8_lossy(SERIAL),
        "{}: serial",
        fixture.name
    );
    assert_eq!(
        read_u32(rom, 0x1A4),
        ROM_LEN as u32 - 1,
        "{}: ROM end",
        fixture.name
    );
    assert_eq!(
        ascii_field(rom, 0x1F0, 0x10),
        "U",
        "{}: region (export/NTSC, as the oracle loader defaults)",
        fixture.name
    );

    // The marker self-loop must be the documented address, not merely present:
    // a shifted marker would still pass a "program ends in BRA.S *" scan.
    let marker = fixture.marker as usize;
    assert!(
        marker % 2 == 0 && (PROGRAM_AT + 2..ROM_LEN).contains(&marker),
        "{}: marker 0x{marker:06X} is not an aligned address inside the program region",
        fixture.name
    );
    assert_eq!(
        rom.get(marker..marker + 2),
        Some(MARKER_LOOP.as_slice()),
        "{}: the documented marker 0x{marker:06X} is not a BRA.S * self-loop",
        fixture.name
    );
    // The marker is the last self-loop in the image. The measured program ends in
    // `BRA.S *`, and the address-comparison window excludes it: a shifted marker
    // would still pass a "contains a self-loop" scan, but not "the highest
    // self-loop", because every fail stub and branch target sits below it.
    let last_self_loop = self_loop_offsets(rom, PROGRAM_AT + 2).last().copied();
    assert_eq!(
        last_self_loop,
        Some(marker),
        "{}: the documented marker 0x{marker:06X} must be the last BRA.S * self-loop in the image",
        fixture.name
    );

    // The crate's own parser must accept the image and agree field for field.
    let header = Cartridge::parse(rom.to_vec())
        .unwrap_or_else(|error| {
            panic!("{}: the cartridge header is invalid: {error}", fixture.name)
        })
        .header()
        .clone();
    assert_eq!(
        header.system,
        CartridgeSystem::MegaDrive,
        "{}",
        fixture.name
    );
    assert_eq!(header.domestic_name, fixture.title, "{}", fixture.name);
    assert_eq!(header.overseas_name, fixture.title, "{}", fixture.name);
    assert_eq!(header.serial, "GM 00000000-00", "{}", fixture.name);
    assert_eq!(
        header.region,
        Region::parse("U").expect("U is a valid region"),
        "{}",
        fixture.name
    );
    assert_eq!(header.rom_start, 0, "{}", fixture.name);
    assert_eq!(header.rom_end, ROM_LEN as u32 - 1, "{}", fixture.name);
    assert!(
        header.sram.is_none(),
        "{}: no SRAM is declared",
        fixture.name
    );
    assert_eq!(
        header.checksum_stored, 0,
        "{}: the synthetic header declares the documented zero checksum",
        fixture.name
    );
}

/// Every committed fixture is exactly what the generator builds, and reads back
/// as its documented synthetic image.
#[test]
fn committed_fixtures_match_the_generator_byte_for_byte() {
    let generated = generate_roms();

    // The committed set must match the generated set exactly: a fixture without
    // a generator entry is dead weight, and a generator ROM without a committed
    // file would make the corpus silently incomplete in a clean clone.
    let mut committed: Vec<std::ffi::OsString> =
        std::fs::read_dir(repo_root().join("scripts").join("fixtures"))
            .expect("the fixture directory exists")
            .map(|entry| entry.expect("readable directory entry").file_name())
            .filter(|name| name.to_string_lossy().ends_with(".bin"))
            .collect();
    committed.sort();
    let mut expected: Vec<std::ffi::OsString> = generated
        .iter()
        .map(|rom| std::ffi::OsString::from(format!("{}.bin", rom.name)))
        .collect();
    expected.sort();
    assert_eq!(
        committed, expected,
        "the committed fixture set must equal the generator's ROM set"
    );

    for rom in &generated {
        let path = fixture_path(&rom.name);
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|error| panic!("{}: cannot read the fixture: {error}", path.display()));
        assert_eq!(
            bytes,
            rom.bytes,
            "{}: the committed fixture diverged from the generator (committed {} bytes, generated {} bytes, first difference at 0x{:X})",
            rom.name,
            bytes.len(),
            rom.bytes.len(),
            bytes
                .iter()
                .zip(&rom.bytes)
                .position(|(committed, generated)| committed != generated)
                .unwrap_or(bytes.len().min(rom.bytes.len()))
        );
    }

    for fixture in LEGACY {
        let rom =
            std::fs::read(fixture_path(fixture.name)).expect("the legacy fixture is committed");
        assert_eq!(
            sha256_hex(&rom),
            fixture.sha256,
            "{}: committed SHA-256 pin",
            fixture.name
        );
        let generated = generated
            .iter()
            .find(|generated| generated.name == fixture.name)
            .expect("the legacy ROM is generated");
        assert_eq!(
            generated.marker,
            format!("0x{:06X}", fixture.marker),
            "{}: the documented marker must be the generator's marker",
            fixture.name
        );
        assert_documented_image(fixture, &rom);
    }
}

/// The marker addresses the pre-repair evidence pages quoted for the legacy
/// three: `0x000352`, `0x0002E0`, `0x0002EE` in
/// `docs/research/oracle-differential-evidence.md`.
const STALE_MARKERS: [u32; 3] = [0x000352, 0x0002E0, 0x0002EE];

/// The committed marker is the generator's marker and the repair is exactly the
/// two bytes the shared `ADD.B D0,D0` register setup introduced.
///
/// This is the regression gate for the `+2` marker repair: the committed bytes
/// must carry the self-loop at the documented address, and the pre-repair
/// address must be the one that is two bytes too early. A future change that
/// moves an instruction without updating the documented marker, or that
/// "corrects" the docs back to the stale values, fails here.
#[test]
fn legacy_markers_are_the_generator_markers_and_shifted_by_two() {
    for (fixture, stale) in LEGACY.iter().zip(STALE_MARKERS) {
        let marker = fixture.marker;
        assert_eq!(
            marker,
            stale + 2,
            "{}: the documented marker 0x{marker:06X} must be the pre-repair 0x{stale:06X} plus two",
            fixture.name
        );
        let rom =
            std::fs::read(fixture_path(fixture.name)).expect("the legacy fixture is committed");
        assert_eq!(
            rom.get(marker as usize..marker as usize + 2),
            Some(MARKER_LOOP.as_slice()),
            "{}: the documented marker 0x{marker:06X} must be the BRA.S * self-loop",
            fixture.name
        );
        assert_ne!(
            rom.get(stale as usize..stale as usize + 2),
            Some(MARKER_LOOP.as_slice()),
            "{}: the stale marker 0x{stale:06X} must no longer start a self-loop",
            fixture.name
        );
    }
}
