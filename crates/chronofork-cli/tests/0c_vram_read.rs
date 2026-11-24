//! W1Z — the `0x0C` 8-bit VRAM read end to end through the public backend.
//!
//! The VDP read buffer is canonical state: it is the value the next data-port
//! read returns, and the pinned core derives a `0x0C` result's upper half from it
//! (`.research/0c/oracle-notes.md` §1, reconciled in ADR-0017). This binary
//! composes a concrete Mega Drive machine and drives a synthetic guest that
//!
//! 1. writes a destination sentinel to work RAM,
//! 2. seeds VRAM with nonzero words through the data port,
//! 3. performs a real `0x0C` read of a nonzero VRAM byte and stores it,
//! 4. and performs a second `0x0C` read of a different byte and stores that too.
//!
//! The tests then prove through the public API only that the guest's first read
//! executed (nonzero stored result), that the checkpoint carries a nonzero
//! canonical read buffer, and that a machine restored from that checkpoint
//! executes the **following** read with the same result as the original. A
//! committed negative control replaces both data-port reads with NOPs and fails
//! the same assertions, so the proof cannot pass with the feature exercised away.

use chronofork_backend_api::Backend;
use chronofork_core::InputFrame;
use chronofork_megadrive::vdp::{SNAPSHOT_LEN, SNAPSHOT_MAGIC};
use chronofork_megadrive::{MegadriveFactory, PAYLOAD_MAGIC, PAYLOAD_VERSION, demo_rom};

// The workspace denies `clippy::expect_used`/`unwrap_used` in tests too, so a
// failed precondition panics through `panic!`, which the workspace keeps allowed.
macro_rules! require_ok {
    ($value:expr, $message:expr) => {
        match $value {
            Ok(value) => value,
            Err(error) => panic!("{}: {error:?}", $message),
        }
    };
}

/// Unwraps an `Option`, panicking with a contextual message when it is absent.
macro_rules! require_some {
    ($value:expr, $message:expr) => {
        match $value {
            Some(value) => value,
            None => panic!("{}: value is absent", $message),
        }
    };
}

/// Fixed payload length for payload v8 (fresh state, no pending request).
///
/// W1Z appended the 2-byte VDP read buffer to the embedded `CFVDP003` snapshot:
/// the v7 value was 206 259 (`crates/chronofork-megadrive/src/backend.rs`).
const PAYLOAD_V8_LEN: usize = 206_261;

/// Work-RAM address of the destination sentinel the guest writes first.
const SENTINEL_ADDRESS: u64 = 0x00FF_0000;
const SENTINEL: [u8; 4] = [0x00, 0x00, 0xDE, 0xAD];
/// First read result (VRAM byte `0xAB` at address 0 with the zeroed seed).
const FIRST_READ_ADDRESS: u64 = 0x00FF_0004;
const FIRST_READ_WORD: [u8; 2] = [0x00, 0xAB];
/// Following read result (VRAM byte `0x34` at address 2 with the zeroed seed).
const SECOND_READ_ADDRESS: u64 = 0x00FF_0008;
const SECOND_READ_WORD: [u8; 2] = [0x00, 0x34];

/// Encodes a 68000 word stream into big-endian bytes.
fn words(words: &[u16]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_be_bytes()).collect()
}

/// The guest program, one instruction stream shared by both controls.
///
/// With `read_enabled == false` the two `MOVE.W $00C00000,D0` instructions (six
/// bytes each) are replaced by three NOPs, so the program keeps its length and
/// every other store still executes; only the data-port read disappears.
fn guest_program(read_enabled: bool) -> Vec<u8> {
    let read = |enabled: bool| {
        if enabled {
            words(&[0x3039, 0x00C0, 0x0000]) // move.w $00C00000, d0
        } else {
            words(&[0x4E71, 0x4E71, 0x4E71]) // three NOPs, same six bytes
        }
    };
    let mut program = words(&[
        0x23FC, 0x0000, 0xDEAD, 0x00FF, 0x0000, // move.l #$0000DEAD, $00FF0000
        // Control words are written one word at a time: a long write to the
        // control port sends two words, and the second would half-latch a new
        // command.
        0x33FC, 0x8F00, 0x00C0, 0x0004, // move.w #$8F00, $00C00004  (register 15 = 0)
        0x33FC, 0x4000, 0x00C0, 0x0004, // command word 1: VRAM write at 0
        0x33FC, 0x0000, 0x00C0, 0x0004, // command word 2
        0x33FC, 0x00AB, 0x00C0, 0x0000, // move.w #$00AB, $00C00000 -> VRAM[1] = 0xAB
        0x33FC, 0x4002, 0x00C0, 0x0004, // command word 1: VRAM write at 2
        0x33FC, 0x0000, 0x00C0, 0x0004, // command word 2
        0x33FC, 0x1234, 0x00C0, 0x0000, // move.w #$1234, $00C00000 -> VRAM[3] = 0x34
        0x33FC, 0x0000, 0x00C0, 0x0004, // command word 1: 0x0C read at 0
        0x33FC, 0x0030, 0x00C0, 0x0004, // command word 2 (CD5:CD2 = 0b0011)
    ]);
    program.extend_from_slice(&read(read_enabled));
    program.extend_from_slice(&words(&[0x33C0, 0x00FF, 0x0004])); // move.w d0, $00FF0004
    program.extend_from_slice(&words(&[
        0x33FC, 0x0002, 0x00C0, 0x0004, // command word 1: 0x0C read at 2
        0x33FC, 0x0030, 0x00C0, 0x0004, // command word 2 (CD5:CD2 = 0b0011)
    ]));
    program.extend_from_slice(&read(read_enabled));
    program.extend_from_slice(&words(&[0x33C0, 0x00FF, 0x0008])); // move.w d0, $00FF0008
    program.extend_from_slice(&words(&[0x60FE])); // bra.s *
    program
}

/// Builds a synthetic cartridge carrying `guest_program`.
fn guest_rom(read_enabled: bool) -> Vec<u8> {
    let program = guest_program(read_enabled);
    let mut rom = vec![0_u8; 0x400];
    for word in rom.chunks_exact_mut(2) {
        word.copy_from_slice(&[0x4E, 0x71]); // NOP fill
    }
    rom[0..4].copy_from_slice(&0x0100_0000_u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x0000_0200_u32.to_be_bytes());
    rom[0x70..0x74].copy_from_slice(&0x0000_0300_u32.to_be_bytes());
    rom[0x78..0x7C].copy_from_slice(&0x0000_0320_u32.to_be_bytes());
    rom[0x200..0x200 + program.len()].copy_from_slice(&program);
    assert!(
        chronofork_megadrive::test_support::stamp_header(&mut rom, "JUE"),
        "the synthetic guest ROM holds a Mega Drive header"
    );
    rom
}

/// The canonical read buffer carried by a payload's embedded `CFVDP003` snapshot.
fn cached_buffer(payload: &[u8]) -> u16 {
    let start = require_some!(
        payload
            .windows(SNAPSHOT_MAGIC.len())
            .position(|window| window == SNAPSHOT_MAGIC),
        "the payload embeds a CFVDP003 snapshot"
    );
    let tail = start + SNAPSHOT_LEN - 2;
    u16::from_le_bytes([payload[tail], payload[tail + 1]])
}

fn read2(machine: &dyn Backend, address: u64) -> [u8; 2] {
    let bytes = require_ok!(machine.read_memory(address, 2), "work ram read");
    [bytes[0], bytes[1]]
}

/// Steps until the two bytes at `address` are nonzero, or the limit is reached.
fn step_until_written(machine: &mut dyn Backend, address: u64, limit: usize) {
    for _ in 0..limit {
        if read2(machine, address) != [0x00, 0x00] {
            return;
        }
        require_ok!(machine.step(InputFrame::NEUTRAL), "step");
    }
    require_some!(
        None::<()>,
        "the guest did not write the expected destination within the step limit"
    );
}

#[test]
fn a_v8_payload_is_the_identity_and_length_with_the_read_buffer() {
    let factory = require_ok!(MegadriveFactory::new(demo_rom()), "factory");
    let machine = require_ok!(factory.create_machine(), "machine");
    let snapshot = require_ok!(machine.snapshot(), "snapshot");
    assert_eq!(PAYLOAD_VERSION, 8);
    assert_eq!(&PAYLOAD_MAGIC, b"CFMD68K8");
    assert_eq!(
        snapshot.payload().len(),
        PAYLOAD_V8_LEN,
        "payload v8 carries the 2-byte VDP read buffer"
    );
    assert_eq!(snapshot.payload()[8..10], PAYLOAD_VERSION.to_le_bytes());
}

#[test]
fn a_guest_read_is_checkpointed_and_a_restored_machine_executes_the_next_read() {
    let factory = require_ok!(MegadriveFactory::new(guest_rom(true)), "factory");
    let mut machine = require_ok!(factory.create_machine(), "machine");
    step_until_written(&mut machine, FIRST_READ_ADDRESS, 400);

    assert_eq!(
        require_ok!(machine.read_memory(SENTINEL_ADDRESS, 4), "sentinel"),
        SENTINEL,
        "the guest wrote its destination sentinel"
    );
    assert_eq!(
        read2(&machine, FIRST_READ_ADDRESS),
        FIRST_READ_WORD,
        "the guest's first 0x0C read stored VRAM[0 ^ 1] = 0xAB with a zeroed seed"
    );

    let checkpoint = require_ok!(machine.snapshot(), "checkpoint");
    assert!(
        (PAYLOAD_V8_LEN..=PAYLOAD_V8_LEN + 16).contains(&checkpoint.payload().len()),
        "the checkpoint is a v8 payload: {}",
        checkpoint.payload().len()
    );
    assert_eq!(
        cached_buffer(checkpoint.payload()),
        0x00AB,
        "the checkpoint carries the nonzero canonical read buffer"
    );

    // A machine restored from the checkpoint must keep that buffer byte for byte
    // and then execute the following read itself.
    let mut restored = require_ok!(factory.create_machine(), "restored machine");
    require_ok!(restored.restore(&checkpoint), "restore checkpoint");
    let restored_checkpoint = require_ok!(restored.snapshot(), "restored checkpoint");
    assert_eq!(
        restored_checkpoint.payload(),
        checkpoint.payload(),
        "the nonzero read buffer and every other canonical field round-trip"
    );
    assert_eq!(cached_buffer(restored_checkpoint.payload()), 0x00AB);

    step_until_written(&mut restored, SECOND_READ_ADDRESS, 400);
    assert_eq!(
        read2(&restored, SECOND_READ_ADDRESS),
        SECOND_READ_WORD,
        "the restored machine executed the following 0x0C read of VRAM[2 ^ 1] = 0x34"
    );
    // The original machine reaches the same second read, so the restored result is
    // not a coincidence of a different path.
    step_until_written(&mut machine, SECOND_READ_ADDRESS, 400);
    assert_eq!(read2(&machine, SECOND_READ_ADDRESS), SECOND_READ_WORD);
    assert_eq!(
        cached_buffer(require_ok!(machine.snapshot(), "original").payload()),
        0x0034,
        "the following read replaced the buffer with the second target's byte"
    );
}

#[test]
fn the_no_read_control_fails_the_same_assertions() {
    // The committed negative control: the same program with both data-port reads
    // replaced by NOPs still writes the sentinel and both destinations, so a test
    // that reads the destinations without checking them against the real values
    // would pass. It must not.
    let factory = require_ok!(MegadriveFactory::new(guest_rom(false)), "factory");
    let mut machine = require_ok!(factory.create_machine(), "machine");
    for _ in 0..400 {
        require_ok!(machine.step(InputFrame::NEUTRAL), "step");
    }
    assert_eq!(
        require_ok!(machine.read_memory(SENTINEL_ADDRESS, 4), "sentinel"),
        SENTINEL,
        "the control program still runs to its sentinel and stores"
    );
    assert_ne!(
        read2(&machine, FIRST_READ_ADDRESS),
        FIRST_READ_WORD,
        "without the data-port read the first destination is not 0x00AB"
    );
    assert_eq!(read2(&machine, FIRST_READ_ADDRESS), [0x00, 0x00]);
    let checkpoint = require_ok!(machine.snapshot(), "checkpoint");
    assert_ne!(
        cached_buffer(checkpoint.payload()),
        0x00AB,
        "without a read the canonical buffer stays zero"
    );
    assert_eq!(cached_buffer(checkpoint.payload()), 0x0000);
}
