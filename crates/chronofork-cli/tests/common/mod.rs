//! Shared support for the `chronofork-cli` integration tests.
//!
//! The synthetic cartridge is the single generated ROM fixture behind the CLI
//! test binaries: `demo.rs` checks command wiring with it and `trace.rs` checks
//! the documented trace and final-state formats against it. It is generated in
//! code and contains no copyrighted data, so there is no fixture artifact to
//! track.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Builds a tiny synthetic cartridge: reset vectors plus a five-instruction
/// program that computes `1 + 2`, stores the word at `0x00FF0000` and continues
/// with NOPs.
pub(crate) fn synthetic_rom() -> Vec<u8> {
    let mut rom = vec![0_u8; 0x400];
    for word in rom.chunks_exact_mut(2) {
        word.copy_from_slice(&[0x4E, 0x71]); // NOP fill
    }
    rom[0..4].copy_from_slice(&0x0100_0000_u32.to_be_bytes()); // top of work RAM
    rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());
    rom[0x70..0x74].copy_from_slice(&0x0000_0300_u32.to_be_bytes()); // HBlank autovector
    rom[0x78..0x7C].copy_from_slice(&0x0000_0320_u32.to_be_bytes()); // VBlank autovector
    rom[0x200..0x20E].copy_from_slice(&[
        0x70, 0x01, // MOVEQ #1,D0
        0x72, 0x02, // MOVEQ #2,D1
        0xD0, 0x41, // ADD.W D1,D0
        0x33, 0xC0, 0x00, 0xFF, 0x00, 0x00, // MOVE.W D0,(0x00FF0000).L
        0x4E, 0x71, // NOP
    ]);
    rom
}

/// Creates (and clears) a per-test temporary directory for CLI artifacts.
pub(crate) fn temp_dir(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("chronofork-cli-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Writes the shared synthetic ROM into `dir` and returns its path.
pub(crate) fn write_rom(dir: &Path) -> PathBuf {
    let path = dir.join("synthetic.rom");
    std::fs::write(&path, synthetic_rom()).expect("write synthetic ROM");
    path
}

/// Runs the built CLI binary with `args`.
pub(crate) fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_chronofork"))
        .args(args)
        .output()
        .expect("chronofork runs")
}

/// Returns a UTF-8 path string for CLI arguments.
pub(crate) fn path_str(path: &Path) -> &str {
    path.to_str().expect("temporary path is UTF-8")
}
