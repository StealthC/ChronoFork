#![allow(clippy::expect_used)]

//! Process-level acceptance tests for the ADR-0010 section 3.4 `chronofork debug`
//! command group.
//!
//! Every case spawns the built `chronofork` binary, so the command line, the exit
//! status and the two output streams are the whole observable contract. The
//! cartridge is the independently authored synthetic ROM from `tests/common`
//! (and, for controller, VDP and interrupt-delivery events, the synthetic
//! fixtures exported by `chronofork-megadrive`). No commercial ROM or copyrighted
//! asset is used, and no test writes into the worktree.
//!
//! The tests assert byte-identical repeated output, exact record fields, exit
//! codes, empty failing stdout and the typed error matrix; no path, timestamp or
//! other host-dependent value may appear in a data field.

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chronofork_core::hash_bytes;
use chronofork_megadrive::{RAM_SIZE, VRAM_SIZE, controller_rom, demo_rom, interrupt_rom};

use common::{path_str, run, synthetic_rom, temp_dir, write_rom};

/// State hash of the shared synthetic fixture at tick 0.
const RESET_STATE_HASH: &str = "f204d37bb42c9aee3c6aeb048726c932d80af4d92008ed1153b3e151202c0061";
/// State hash of the shared synthetic fixture after five instructions.
const FIVE_STEP_STATE_HASH: &str =
    "d8c16a74e3112caefe13418fab7e8b63158e9e8e05b55ab6be9b039b9eec83bd";
/// State hash after the fixture's `MOVEQ #1/D0`, `MOVEQ #2/D1` and `ADD`.
const THREE_STEP_STATE_HASH: &str =
    "da1dafce1a52555969bd6c19fb3c9128922ec370a96483d6724beceaa0e52fc9";
/// State hash after the fixture's `MOVE.W D0,(0x00FF0000).L` write.
const RAM_WRITE_STATE_HASH: &str =
    "97a5d1dded40ef2196d849ee4e9d0e592647783422f53b8931338f89927651a9";
/// State hash after the fixture's first two instructions.
const TWO_STEP_STATE_HASH: &str =
    "5b2304aad147f57fd567dbbd198c6db5293286aa0b096b3c45a5ca98aaaffb40";
/// SHA-256 of an all-zero 64 KiB image: work RAM and VRAM at reset.
const ZERO_64K_SHA256: &str = "de2f256064a0af797747c2b97505dc0b9f3df0de4f489eac731c23ae9ca9cc31";

/// One parsed `record=` block with its `key=value` fields and any row body.
struct Block {
    fields: BTreeMap<String, String>,
    body: Vec<String>,
}

impl Block {
    /// Returns a required field value.
    fn field(&self, key: &str) -> &str {
        self.fields
            .get(key)
            .map(String::as_str)
            .expect("record block has the field")
    }
}

/// Runs the CLI and requires exit 0 with empty stderr, returning stdout.
fn run_ok(args: &[&str]) -> String {
    let output = run(args);
    let stdout = String::from_utf8(output.stdout.clone()).expect("stdout is UTF-8");
    let stderr = String::from_utf8(output.stderr.clone()).expect("stderr is UTF-8");
    assert!(
        output.status.success(),
        "command failed: {args:?}\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stderr.is_empty(),
        "successful command wrote to stderr: {args:?}\n{stderr}"
    );
    stdout
}

/// Runs the CLI and requires exit 1, empty stdout and one `chronofork:`
/// diagnostic, returning stderr.
fn run_fail(args: &[&str]) -> String {
    let output = run(args);
    let stdout = String::from_utf8(output.stdout.clone()).expect("stdout is UTF-8");
    let stderr = String::from_utf8(output.stderr.clone()).expect("stderr is UTF-8");
    assert_eq!(
        output.status.code(),
        Some(1),
        "expected exit code 1 for {args:?}\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.is_empty(),
        "a typed failure must print empty stdout: {args:?}\n{stdout}"
    );
    assert!(
        stderr.starts_with("chronofork: "),
        "diagnostics must start with `chronofork: `: {args:?}\n{stderr}"
    );
    stderr
}

/// Runs a command twice and returns the single, byte-identical output.
fn stable(args: &[&str]) -> String {
    let first = run_ok(args);
    let second = run_ok(args);
    assert_eq!(first, second, "output is not deterministic for {args:?}");
    first
}

/// Interprets one line as a `key=value` field.
///
/// Row bodies (memory dumps and disassembly rows) start with an uppercase
/// address and contain spaces, so their first `=` is not a field separator.
fn field_line(line: &str) -> Option<(&str, &str)> {
    let (key, value) = line.split_once('=')?;
    let valid = !key.is_empty()
        && key
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_');
    valid.then_some((key, value))
}

/// Parses the documented blank-line-separated record blocks.
fn parse_blocks(stdout: &str) -> Vec<Block> {
    assert!(
        !stdout.is_empty(),
        "expected at least one record block, got empty stdout"
    );
    assert!(
        stdout.ends_with('\n') && !stdout.ends_with("\n\n"),
        "blocks end with exactly one newline:\n{stdout}"
    );
    let mut blocks = Vec::new();
    for chunk in stdout.split("\n\n") {
        let mut lines = chunk.lines();
        let record = lines
            .next()
            .and_then(|line| line.strip_prefix("record="))
            .expect("every block starts with `record=`");
        let mut fields = BTreeMap::new();
        fields.insert("record".to_owned(), record.to_owned());
        let mut body = Vec::new();
        for line in lines {
            match field_line(line) {
                Some((key, value)) => {
                    assert!(
                        fields.insert(key.to_owned(), value.to_owned()).is_none(),
                        "duplicate field `{key}`:\n{stdout}"
                    );
                }
                None => body.push(line.to_owned()),
            }
        }
        blocks.push(Block { fields, body });
    }
    blocks
}

/// Parses exactly one block.
fn single_block(stdout: &str) -> Block {
    let mut blocks = parse_blocks(stdout);
    assert_eq!(blocks.len(), 1, "expected exactly one block:\n{stdout}");
    blocks.pop().expect("one block was parsed")
}

/// Writes an independently authored fixture ROM into `dir`.
fn write_named_rom(dir: &Path, name: &str, bytes: Vec<u8>) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("write fixture ROM");
    path
}

#[test]
fn debug_inspect_prints_stable_cpu_and_device_fields() {
    let dir = temp_dir("debug-inspect");
    let rom = write_rom(&dir);
    let rom_path = path_str(&rom);
    let stdout = stable(&["debug", "inspect", "--rom", rom_path, "--steps", "5"]);
    assert!(
        !stdout.contains(rom_path),
        "valid stdout must not contain host paths:\n{stdout}"
    );
    let block = single_block(&stdout);
    assert!(block.body.is_empty(), "inspect has no row body:\n{stdout}");
    let expected = [
        ("record", "debug-inspect"),
        ("tick", "224"),
        ("state_hash", FIVE_STEP_STATE_HASH),
        ("halted", "0"),
        ("steps", "5"),
        ("pc", "0x0000020E"),
        ("sr", "0x2700"),
        ("supervisor", "1"),
        ("trace", "0"),
        ("interrupt_mask", "7"),
        ("x", "0"),
        ("n", "0"),
        ("z", "0"),
        ("v", "0"),
        ("c", "0"),
        ("vdp_status", "0x0100"),
        ("vdp_vblank", "0"),
        ("vdp_vint_pending", "0"),
        ("vdp_address", "0x0000"),
        ("vdp_auto_increment", "0"),
        ("scheduler_standard", "ntsc"),
        ("scheduler_lines_per_frame", "262"),
        ("scheduler_active_lines", "224"),
        ("scheduler_frame", "0"),
        ("scheduler_line", "0"),
        ("scheduler_master_cycle_in_line", "224"),
    ];
    assert_eq!(
        block.fields.len(),
        expected.len(),
        "unexpected extra fields"
    );
    for (key, value) in expected {
        assert_eq!(block.field(key), value, "field `{key}`");
    }

    // At reset the view reports the reset PC and no completed quantum.
    let reset = single_block(&run_ok(&["debug", "inspect", "--rom", rom_path]));
    assert_eq!(reset.field("tick"), "0");
    assert_eq!(reset.field("steps"), "0");
    assert_eq!(reset.field("pc"), "0x00000200");
    assert_eq!(reset.field("scheduler_master_cycle_in_line"), "0");
    assert_eq!(reset.field("state_hash"), RESET_STATE_HASH);
}

#[test]
fn debug_memory_dump_is_exact_and_bounded() {
    let dir = temp_dir("debug-memory");
    let rom = write_rom(&dir);
    let rom_path = path_str(&rom);

    let stdout = stable(&[
        "debug",
        "memory",
        "--rom",
        rom_path,
        "--steps",
        "5",
        "--address",
        "0x00FF0000",
        "--length",
        "32",
    ]);
    let expected = "\
record=debug-memory\n\
address=0x00FF0000\n\
length=32\n\
state_hash=d8c16a74e3112caefe13418fab7e8b63158e9e8e05b55ab6be9b039b9eec83bd\n\
rows=2\n\
00FF0000  00 03 00 00 00 00 00 00 00 00 00 00 00 00 00 00 |................|\n\
00FF0010  00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 |................|\n";
    assert_eq!(stdout, expected);
    let block = single_block(&stdout);
    assert_eq!(block.body.len(), 2);
    assert_eq!(block.field("state_hash"), FIVE_STEP_STATE_HASH);

    // The dump's state hash equals a fresh inspection at the same steps: a
    // debugger read never mutates the canonical state.
    let inspect = single_block(&run_ok(&[
        "debug", "inspect", "--rom", rom_path, "--steps", "5",
    ]));
    assert_eq!(block.field("state_hash"), inspect.field("state_hash"));

    // Printable ASCII is rendered verbatim; control bytes become `.`.
    let image = single_block(&run_ok(&[
        "debug",
        "memory",
        "--rom",
        rom_path,
        "--address",
        "0x200",
        "--length",
        "16",
    ]));
    assert_eq!(
        image.body,
        vec!["00000200  70 01 72 02 D0 41 33 C0 00 FF 00 00 4E 71 4E 71 |p.r..A3.....NqNq|"]
    );

    // A zero length prints the header and no body row.
    let empty = stable(&[
        "debug",
        "memory",
        "--rom",
        rom_path,
        "--address",
        "0x00FF0000",
        "--length",
        "0",
    ]);
    let expected = format!(
        "record=debug-memory\naddress=0x00FF0000\nlength=0\nstate_hash={RESET_STATE_HASH}\nrows=0\n"
    );
    assert_eq!(empty, expected);
    assert!(single_block(&empty).body.is_empty());

    // Decimal and 0X-uppercase addresses are the same address.
    let decimal = single_block(&run_ok(&[
        "debug",
        "memory",
        "--rom",
        rom_path,
        "--address",
        "16711680",
        "--length",
        "4",
    ]));
    assert_eq!(decimal.field("address"), "0x00FF0000");

    // An odd start address is rejected even though the underlying bus could read it.
    let stderr = run_fail(&[
        "debug",
        "memory",
        "--rom",
        rom_path,
        "--address",
        "0x00FF0001",
        "--length",
        "4",
    ]);
    assert!(stderr.contains("is odd"), "{stderr}");

    // A request that starts inside work RAM and crosses its end is out of bounds.
    let stderr = run_fail(&[
        "debug",
        "memory",
        "--rom",
        rom_path,
        "--address",
        "0x00FFFFFE",
        "--length",
        "4",
    ]);
    assert!(stderr.contains("memory access out of bounds"), "{stderr}");

    // An unmapped hole is a typed failure, not an empty dump.
    let stderr = run_fail(&[
        "debug",
        "memory",
        "--rom",
        rom_path,
        "--address",
        "0x00200000",
        "--length",
        "4",
    ]);
    assert!(stderr.contains("outside any mapped window"), "{stderr}");

    // The dump length is bounded before any allocation.
    let stderr = run_fail(&[
        "debug",
        "memory",
        "--rom",
        rom_path,
        "--address",
        "0x00FF0000",
        "--length",
        "4097",
    ]);
    assert!(
        stderr.contains("`--length` must be between 0 and 4096"),
        "{stderr}"
    );
}

#[test]
fn debug_disasm_matches_supported_encodings() {
    let dir = temp_dir("debug-disasm");
    let rom = write_rom(&dir);
    let rom_path = path_str(&rom);

    let stdout = stable(&["debug", "disasm", "--rom", rom_path, "--count", "5"]);
    let expected = "\
record=debug-disasm\n\
address=0x00000200\n\
count=5\n\
instructions=5\n\
00000200  7001  MOVEQ #$01,D0\n\
00000202  7202  MOVEQ #$02,D1\n\
00000204  D041  ADD.W D1,D0\n\
00000206  33C0  MOVE.W D0,($00FF0000).L\n\
0000020C  4E71  NOP\n";
    assert_eq!(stdout, expected);
    let block = single_block(&stdout);
    assert_eq!(block.body.len(), 5);
    // The encoded lengths equal the PC deltas of execution: 2, 2, 2, 6, 2.
    let addresses: Vec<&str> = block
        .body
        .iter()
        .map(|row| row.split_whitespace().next().expect("row address"))
        .collect();
    assert_eq!(
        addresses,
        vec!["00000200", "00000202", "00000204", "00000206", "0000020C"]
    );

    // Work RAM is a supported window; unexecuted data prints as DC.W.
    let ram = single_block(&run_ok(&[
        "debug",
        "disasm",
        "--rom",
        rom_path,
        "--steps",
        "5",
        "--address",
        "0x00FF0000",
        "--count",
        "2",
    ]));
    assert_eq!(
        ram.body,
        vec!["00FF0000  0003  DC.W $0003", "00FF0002  0000  DC.W $0000"]
    );

    // An odd start address is rejected: instruction words are word-aligned.
    let stderr = run_fail(&[
        "debug",
        "disasm",
        "--rom",
        rom_path,
        "--address",
        "0x201",
        "--count",
        "1",
    ]);
    assert!(stderr.contains("is odd"), "{stderr}");

    // A truncated extension word is a typed error, not a partial row.
    let stderr = run_fail(&[
        "debug",
        "disasm",
        "--rom",
        rom_path,
        "--address",
        "0x3FE",
        "--count",
        "2",
    ]);
    assert!(
        stderr.contains("disassembly word at 0x000400 is unavailable"),
        "{stderr}"
    );

    // A device window is not disassemblable in this slice.
    let stderr = run_fail(&[
        "debug",
        "disasm",
        "--rom",
        rom_path,
        "--address",
        "0x00C00000",
        "--count",
        "1",
    ]);
    assert!(
        stderr.contains("is not mapped in the cartridge image or work RAM"),
        "{stderr}"
    );

    // The count is bounded by the disassembler limit.
    let stderr = run_fail(&["debug", "disasm", "--rom", rom_path, "--count", "0"]);
    assert!(stderr.contains("must be between 1 and 4096"), "{stderr}");
    let stderr = run_fail(&["debug", "disasm", "--rom", rom_path, "--count", "4097"]);
    assert!(stderr.contains("must be between 1 and 4096"), "{stderr}");
}

#[test]
fn debug_vdp_prints_registers_status_and_vram_digest() {
    let dir = temp_dir("debug-vdp");
    let rom = write_rom(&dir);
    let rom_path = path_str(&rom);

    let stdout = stable(&[
        "debug",
        "vdp",
        "--rom",
        rom_path,
        "--vram-address",
        "0",
        "--vram-length",
        "16",
    ]);
    let block = single_block(&stdout);
    assert_eq!(block.field("record"), "debug-vdp");
    assert_eq!(block.field("tick"), "0");
    assert_eq!(block.field("state_hash"), RESET_STATE_HASH);
    assert_eq!(block.field("vdp_status"), "0x0100");
    assert_eq!(block.field("vdp_vblank"), "0");
    assert_eq!(block.field("vdp_vint_pending"), "0");
    assert_eq!(block.field("vdp_address"), "0x0000");
    assert_eq!(block.field("vdp_auto_increment"), "0");
    for index in 0..24 {
        assert_eq!(
            block.field(&format!("vdp_register_{index:02}")),
            "0x00",
            "register {index} at reset"
        );
    }
    assert_eq!(block.field("vdp_vram_len"), VRAM_SIZE.to_string());
    let expected_vram = hash_bytes(&vec![0_u8; VRAM_SIZE]).to_string();
    assert_eq!(block.field("vdp_vram_sha256"), expected_vram);
    assert_eq!(block.field("vdp_vram_sha256"), ZERO_64K_SHA256);
    assert_eq!(block.field("vram_address"), "0x00000000");
    assert_eq!(block.field("vram_length"), "16");
    assert_eq!(block.field("vram_hex"), "00000000000000000000000000000000");
    assert!(block.body.is_empty(), "the VDP view has no row body");

    // The demo ROM programs register 0x0F and moves the VDP address, so the
    // block reflects real device state instead of reset constants.
    let demo = write_named_rom(&dir, "demo.rom", demo_rom());
    let advanced = single_block(&run_ok(&[
        "debug",
        "vdp",
        "--rom",
        path_str(&demo),
        "--steps",
        "23",
    ]));
    assert_eq!(advanced.field("tick"), "1862");
    assert_eq!(advanced.field("vdp_auto_increment"), "2");
    assert_eq!(advanced.field("vdp_register_15"), "0x02");
    assert_eq!(advanced.field("vdp_address"), "0x0002");

    // A VRAM window is bounds-checked against the 64 KiB image.
    let stderr = run_fail(&[
        "debug",
        "vdp",
        "--rom",
        rom_path,
        "--vram-address",
        "0xFFF0",
        "--vram-length",
        "4096",
    ]);
    assert!(
        stderr.contains("exceeds the 65536-byte VRAM image"),
        "{stderr}"
    );
    let stderr = run_fail(&["debug", "vdp", "--rom", rom_path, "--vram-length", "4"]);
    assert!(stderr.contains("must be given together"), "{stderr}");
}

#[test]
fn debug_interrupts_prints_pending_issue_tick_and_counters() {
    let dir = temp_dir("debug-interrupts");
    let rom = write_rom(&dir);
    let rom_path = path_str(&rom);

    let idle = single_block(&run_ok(&["debug", "interrupts", "--rom", rom_path]));
    assert_eq!(idle.field("tick"), "0");
    assert_eq!(idle.field("pending"), "0");
    assert_eq!(idle.field("counter_0_level"), "4");
    assert_eq!(idle.field("counter_0_issued"), "0");
    assert_eq!(idle.field("counter_0_coalesced"), "0");
    assert_eq!(idle.field("counter_1_level"), "6");
    assert_eq!(idle.field("counter_1_issued"), "0");
    assert_eq!(idle.field("counter_1_coalesced"), "0");

    // The fixture never writes SR, so the reset mask 7 keeps the HBlank request
    // pending at the end of the first active line on the master-cycle timeline.
    let stdout = stable(&["debug", "interrupts", "--rom", rom_path, "--steps", "120"]);
    let block = single_block(&stdout);
    let expected = [
        ("record", "debug-interrupts"),
        ("tick", "3444"),
        (
            "state_hash",
            "2f6de1cb49ec437e5beb64529006485398d05536b7875b6bafba5c13b245075d",
        ),
        ("video", "ntsc"),
        ("lines_per_frame", "262"),
        ("active_lines", "224"),
        ("frame", "0"),
        ("line", "1"),
        ("master_cycle_in_line", "24"),
        ("pending", "1"),
        ("pending_0_level", "4"),
        ("pending_0_issued_tick", "3420"),
        ("pending_0_deliverable", "0"),
        ("counter_0_level", "4"),
        ("counter_0_issued", "1"),
        ("counter_0_coalesced", "0"),
        ("counter_1_level", "6"),
        ("counter_1_issued", "0"),
        ("counter_1_coalesced", "0"),
    ];
    assert_eq!(
        block.fields.len(),
        expected.len(),
        "unexpected extra fields"
    );
    for (key, value) in expected {
        assert_eq!(block.field(key), value, "field `{key}`");
    }

    // The interrupt ROM lowers the mask, so the retained HBlank is delivered and
    // acknowledged: the request counter advanced and nothing is pending.
    let interrupt = write_named_rom(&dir, "interrupt.rom", interrupt_rom(3));
    let delivered = single_block(&run_ok(&[
        "debug",
        "interrupts",
        "--rom",
        path_str(&interrupt),
        "--steps",
        "120",
    ]));
    assert_eq!(delivered.field("tick"), "8694");
    assert_eq!(delivered.field("pending"), "0");
    assert_eq!(delivered.field("counter_0_issued"), "2");
    assert_eq!(delivered.field("counter_0_coalesced"), "1");
    assert_eq!(delivered.field("master_cycle_in_line"), "1854");
}

#[test]
fn debug_bus_and_cpu_view_are_stable() {
    let dir = temp_dir("debug-bus");
    let rom = write_rom(&dir);
    let rom_path = path_str(&rom);

    let stdout = stable(&["debug", "bus", "--rom", rom_path]);
    let block = single_block(&stdout);
    assert_eq!(block.field("record"), "debug-bus");
    assert_eq!(block.field("rom_len"), "1024");
    assert_eq!(
        block.field("rom_sha256"),
        hash_bytes(&synthetic_rom()).to_string()
    );
    assert_eq!(block.field("ram_len"), RAM_SIZE.to_string());
    assert_eq!(block.field("ram_sha256"), ZERO_64K_SHA256);
    let windows = [
        ("rom", "0x00000000", "0x00400000", "0"),
        ("controller", "0x00A10000", "0x00A10020", "0"),
        ("vdp", "0x00C00000", "0x00C00010", "1"),
        ("ram", "0x00FF0000", "0x01000000", "1"),
    ];
    for (index, (name, start, end, writable)) in windows.iter().enumerate() {
        assert_eq!(block.field(&format!("window_{index}_name")), *name);
        assert_eq!(block.field(&format!("window_{index}_start")), *start);
        assert_eq!(block.field(&format!("window_{index}_end")), *end);
        assert_eq!(block.field(&format!("window_{index}_writable")), *writable);
    }
    assert_eq!(block.fields.len(), 5 + 4 * 4, "unexpected extra fields");
    assert!(block.body.is_empty(), "the bus view has no row body");

    // The CPU view's PC and the disassembly at the default address describe the
    // same next instruction.
    let inspect = single_block(&run_ok(&["debug", "inspect", "--rom", rom_path]));
    let pc = inspect.field("pc");
    assert_eq!(pc, "0x00000200");
    let disasm = single_block(&run_ok(&[
        "debug", "disasm", "--rom", rom_path, "--count", "1",
    ]));
    assert_eq!(disasm.field("address"), pc);
    assert_eq!(disasm.body, vec!["00000200  7001  MOVEQ #$01,D0"]);

    // The bus layout is state-independent and rejects an instruction budget.
    let stderr = run_fail(&["debug", "bus", "--rom", rom_path, "--steps", "1"]);
    assert!(stderr.contains("does not accept `--steps`"), "{stderr}");
}

#[test]
fn debug_run_breakpoint_and_condition_stop_before_the_instruction() {
    let dir = temp_dir("debug-run-breakpoint");
    let rom = write_rom(&dir);
    let rom_path = path_str(&rom);

    // No trigger: the budget is reached at the next instruction boundary.
    let limited = single_block(&stable(&[
        "debug", "run", "--rom", rom_path, "--steps", "5",
    ]));
    assert_eq!(limited.field("status"), "stopped");
    assert_eq!(limited.field("reason"), "step-limit");
    assert_eq!(limited.field("tick"), "224");
    assert_eq!(limited.field("state_hash"), FIVE_STEP_STATE_HASH);
    assert_eq!(limited.field("pc"), "0x0000020E");
    assert_eq!(limited.field("steps"), "5");
    assert_eq!(limited.field("halted"), "0");
    assert_eq!(limited.field("breakpoint_id"), "-");
    assert_eq!(limited.field("eventpoint_id"), "-");
    assert_eq!(limited.field("events_matched"), "0");
    assert_eq!(limited.field("events_dropped"), "0");
    assert_eq!(limited.field("log_entries"), "0");
    assert_eq!(limited.field("log_dropped"), "0");
    assert_eq!(limited.field("watchpoint_ids"), "-");

    // A PC breakpoint stops before `MOVE.W`, so only three instructions ran and
    // the reported tick/state are the pre-instruction boundary.
    let breakpoint = single_block(&run_ok(&[
        "debug",
        "run",
        "--rom",
        rom_path,
        "--steps",
        "5",
        "--breakpoint",
        "0x206",
    ]));
    assert_eq!(breakpoint.field("reason"), "breakpoint");
    assert_eq!(breakpoint.field("breakpoint_id"), "1");
    assert_eq!(breakpoint.field("tick"), "84");
    assert_eq!(breakpoint.field("steps"), "3");
    assert_eq!(breakpoint.field("pc"), "0x00000206");
    assert_eq!(breakpoint.field("state_hash"), THREE_STEP_STATE_HASH);

    // Decimal and 0X-uppercase addresses select the same location.
    for location in ["518", "0X206"] {
        let block = single_block(&run_ok(&[
            "debug",
            "run",
            "--rom",
            rom_path,
            "--steps",
            "5",
            "--breakpoint",
            location,
        ]));
        assert_eq!(block.field("reason"), "breakpoint", "location `{location}`");
        assert_eq!(block.field("pc"), "0x00000206", "location `{location}`");
        assert_eq!(block.field("steps"), "3", "location `{location}`");
    }

    // An inclusive range stops at the first PC inside it.
    let range = single_block(&run_ok(&[
        "debug",
        "run",
        "--rom",
        rom_path,
        "--steps",
        "5",
        "--breakpoint",
        "0x204-0x206",
    ]));
    assert_eq!(range.field("reason"), "breakpoint");
    assert_eq!(range.field("tick"), "56");
    assert_eq!(range.field("steps"), "2");
    assert_eq!(range.field("pc"), "0x00000204");
    assert_eq!(range.field("state_hash"), TWO_STEP_STATE_HASH);

    // A register condition is ANDed with the location; hex values are accepted.
    let condition = single_block(&run_ok(&[
        "debug",
        "run",
        "--rom",
        rom_path,
        "--steps",
        "5",
        "--breakpoint",
        "0x206:reg:d0==0x3",
    ]));
    assert_eq!(condition.field("reason"), "breakpoint");
    assert_eq!(condition.field("pc"), "0x00000206");
    assert_eq!(condition.field("tick"), "84");

    // A condition that never holds reaches the step limit instead of stopping.
    let miss = single_block(&run_ok(&[
        "debug",
        "run",
        "--rom",
        rom_path,
        "--steps",
        "5",
        "--breakpoint",
        "0x206:reg:d0==7",
    ]));
    assert_eq!(miss.field("reason"), "step-limit");
    assert_eq!(miss.field("steps"), "5");

    // A big-endian memory condition sees the value the fixture stores.
    let memory = single_block(&run_ok(&[
        "debug",
        "run",
        "--rom",
        rom_path,
        "--steps",
        "5",
        "--breakpoint",
        "0x20E:mem:0x00FF0000:2==3",
    ]));
    assert_eq!(memory.field("reason"), "breakpoint");
    assert_eq!(memory.field("pc"), "0x0000020E");
    assert_eq!(memory.field("tick"), "224");
    let miss = single_block(&run_ok(&[
        "debug",
        "run",
        "--rom",
        rom_path,
        "--steps",
        "5",
        "--breakpoint",
        "0x20E:mem:0x00FF0000:2==4",
    ]));
    assert_eq!(miss.field("reason"), "step-limit");
}

#[test]
fn debug_run_watchpoint_and_eventpoint_stop_after_the_instruction() {
    let dir = temp_dir("debug-run-events");
    let rom = write_rom(&dir);
    let rom_path = path_str(&rom);

    // A write watchpoint matches both bytes of the fixture's word write and stops
    // after the writing instruction.
    let stdout = stable(&[
        "debug",
        "run",
        "--rom",
        rom_path,
        "--steps",
        "5",
        "--watchpoint",
        "0x00FF0000-0x00FF0001:w",
        "--show-events",
        "12",
    ]);
    let blocks = parse_blocks(&stdout);
    assert_eq!(blocks.len(), 5, "run report plus four event blocks");
    let run = &blocks[0];
    assert_eq!(run.field("reason"), "watchpoint");
    assert_eq!(run.field("watchpoint_ids"), "1");
    assert_eq!(run.field("steps"), "4");
    assert_eq!(run.field("tick"), "196");
    assert_eq!(run.field("pc"), "0x0000020C");
    assert_eq!(run.field("state_hash"), RAM_WRITE_STATE_HASH);
    assert_eq!(run.field("events_matched"), "0");
    assert_eq!(run.field("events_retained"), "4");
    let kinds: Vec<&str> = blocks[1..]
        .iter()
        .map(|block| block.field("kind"))
        .collect();
    assert_eq!(
        kinds,
        vec!["bus-write", "watchpoint", "bus-write", "watchpoint"]
    );
    for (index, block) in blocks[1..].iter().enumerate() {
        assert_eq!(block.field("record"), "debug-event");
        assert_eq!(block.field("matched"), "0");
        assert_eq!(block.field("tick"), "84", "pre-step tick of the write");
        assert_eq!(block.field("width"), "1");
        assert_eq!(block.field("access"), "write");
        let expected_address = if index < 2 {
            "0x00FF0000"
        } else {
            "0x00FF0001"
        };
        assert_eq!(block.field("address"), expected_address);
        let expected_value = if index < 2 {
            "0x00000000"
        } else {
            "0x00000003"
        };
        assert_eq!(block.field("value"), expected_value);
    }
    assert_eq!(blocks[2].field("watchpoint_id"), "1");
    assert_eq!(blocks[4].field("watchpoint_id"), "1");

    // `rw` installs a read and a write watchpoint; the write spec has host id 2.
    let rw = single_block(&run_ok(&[
        "debug",
        "run",
        "--rom",
        rom_path,
        "--steps",
        "5",
        "--watchpoint",
        "0x00FF0000:rw",
    ]));
    assert_eq!(rw.field("reason"), "watchpoint");
    assert_eq!(rw.field("watchpoint_ids"), "2");
    assert_eq!(rw.field("events_retained"), "3");

    // A read watchpoint on the demo ROM stops after the RAM reload, and the
    // matching events are reads of the written bytes.
    let demo = write_named_rom(&dir, "demo.rom", demo_rom());
    let blocks = parse_blocks(&run_ok(&[
        "debug",
        "run",
        "--rom",
        path_str(&demo),
        "--steps",
        "20",
        "--watchpoint",
        "0x00FF0000-0x00FF0001:r",
        "--show-events",
        "10",
    ]));
    let read = &blocks[0];
    assert_eq!(read.field("reason"), "watchpoint");
    assert_eq!(read.field("watchpoint_ids"), "1");
    assert_eq!(read.field("tick"), "336");
    assert_eq!(read.field("steps"), "6");
    assert_eq!(read.field("pc"), "0x00000214");
    assert_eq!(read.field("events_retained"), "4");

    // A bus-write event breakpoint stops after the writing instruction and keeps
    // both matched events even without `--show-events`.
    let blocks = parse_blocks(&run_ok(&[
        "debug",
        "run",
        "--rom",
        rom_path,
        "--steps",
        "5",
        "--eventpoint",
        "bus-write:0x00FF0000-0x00FF0001",
    ]));
    assert_eq!(blocks.len(), 3, "run report plus both matched events");
    let run = &blocks[0];
    assert_eq!(run.field("reason"), "event");
    assert_eq!(run.field("eventpoint_id"), "1");
    assert_eq!(run.field("tick"), "196");
    assert_eq!(run.field("steps"), "4");
    assert_eq!(run.field("events_matched"), "2");
    assert_eq!(run.field("events_retained"), "2");
    for (index, block) in blocks[1..].iter().enumerate() {
        assert_eq!(block.field("kind"), "bus-write");
        assert_eq!(block.field("matched"), "1");
        assert_eq!(block.field("tick"), "84");
        let expected_address = if index == 0 {
            "0x00FF0000"
        } else {
            "0x00FF0001"
        };
        assert_eq!(block.field("address"), expected_address);
    }

    // A controller-read filter stops on the fixture's first controller read.
    let controller = write_named_rom(&dir, "controller.rom", controller_rom());
    let blocks = parse_blocks(&run_ok(&[
        "debug",
        "run",
        "--rom",
        path_str(&controller),
        "--steps",
        "20",
        "--eventpoint",
        "controller:1",
    ]));
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].field("reason"), "event");
    assert_eq!(blocks[0].field("tick"), "224");
    assert_eq!(blocks[0].field("steps"), "5");
    assert_eq!(blocks[0].field("events_matched"), "1");
    let event = &blocks[1];
    assert_eq!(event.field("kind"), "controller-read");
    assert_eq!(event.field("port"), "1");
    assert_eq!(event.field("value"), "0x0000007F");
    assert_eq!(event.field("width"), "1");
    assert_eq!(event.field("access"), "read");
    assert_eq!(event.field("tick"), "112");
    assert_eq!(event.field("matched"), "1");

    // A VDP-write filter stops on the first data-port write of the demo ROM.
    let blocks = parse_blocks(&run_ok(&[
        "debug",
        "run",
        "--rom",
        path_str(&demo),
        "--steps",
        "20",
        "--eventpoint",
        "vdp:write",
    ]));
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].field("reason"), "event");
    assert_eq!(blocks[0].field("tick"), "504");
    assert_eq!(blocks[0].field("steps"), "8");
    let event = &blocks[1];
    assert_eq!(event.field("kind"), "vdp-port");
    assert_eq!(event.field("offset"), "0x04");
    assert_eq!(event.field("value"), "0x00008F02");
    assert_eq!(event.field("width"), "2");
    assert_eq!(event.field("access"), "write");
    assert_eq!(event.field("tick"), "392");
    assert_eq!(event.field("matched"), "1");

    // An interrupt-request filter stops on the masked HBlank request.
    let blocks = parse_blocks(&run_ok(&[
        "debug",
        "run",
        "--rom",
        rom_path,
        "--steps",
        "200",
        "--eventpoint",
        "interrupt:requested:4",
    ]));
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].field("reason"), "event");
    assert_eq!(blocks[0].field("tick"), "3444");
    assert_eq!(blocks[0].field("steps"), "120");
    assert_eq!(blocks[0].field("events_retained"), "3");
    let event = &blocks[1];
    assert_eq!(event.field("kind"), "interrupt-requested");
    assert_eq!(event.field("level"), "4");
    assert_eq!(event.field("phase"), "requested");
    assert_eq!(event.field("tick"), "3416");

    // Delivery and acknowledgement filters stop after the delivering instruction;
    // the delivered and acknowledged events share that step.
    let interrupt = write_named_rom(&dir, "interrupt.rom", interrupt_rom(3));
    let blocks = parse_blocks(&run_ok(&[
        "debug",
        "run",
        "--rom",
        path_str(&interrupt),
        "--steps",
        "200",
        "--eventpoint",
        "interrupt:delivered:4",
    ]));
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].field("reason"), "event");
    assert_eq!(blocks[0].field("tick"), "7406");
    assert_eq!(blocks[0].field("steps"), "105");
    assert_eq!(blocks[0].field("events_matched"), "1");
    assert_eq!(blocks[1].field("kind"), "interrupt-delivered");
    assert_eq!(blocks[1].field("level"), "4");
    assert_eq!(blocks[1].field("phase"), "delivered");
    assert_eq!(blocks[1].field("tick"), "7378");
    assert_eq!(blocks[1].field("matched"), "1");

    let blocks = parse_blocks(&run_ok(&[
        "debug",
        "run",
        "--rom",
        path_str(&interrupt),
        "--steps",
        "200",
        "--eventpoint",
        "interrupt:acknowledged",
    ]));
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].field("tick"), "7406");
    assert_eq!(blocks[1].field("kind"), "interrupt-acknowledged");
    assert_eq!(blocks[1].field("level"), "4");
    assert_eq!(blocks[1].field("phase"), "acknowledged");
    assert_eq!(blocks[1].field("matched"), "1");

    // A controller filter without a port accepts either hardware port.
    let blocks = parse_blocks(&run_ok(&[
        "debug",
        "run",
        "--rom",
        path_str(&controller),
        "--steps",
        "20",
        "--eventpoint",
        "controller:any",
    ]));
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].field("reason"), "event");
    assert_eq!(blocks[0].field("eventpoint_id"), "1");
    assert_eq!(blocks[1].field("kind"), "controller-read");
}

#[test]
fn debug_run_logpoint_records_without_stopping() {
    let dir = temp_dir("debug-run-logpoint");
    let rom = write_rom(&dir);
    let rom_path = path_str(&rom);

    let stdout = stable(&[
        "debug",
        "run",
        "--rom",
        rom_path,
        "--steps",
        "5",
        "--logpoint",
        "0x200:start",
        "--logpoint",
        "0x204",
    ]);
    let blocks = parse_blocks(&stdout);
    assert_eq!(blocks.len(), 3, "run report plus two log blocks");
    let run = &blocks[0];
    assert_eq!(run.field("reason"), "step-limit");
    assert_eq!(run.field("steps"), "5");
    assert_eq!(run.field("tick"), "224");
    assert_eq!(run.field("log_entries"), "2");
    assert_eq!(run.field("log_dropped"), "0");

    let first = &blocks[1];
    assert_eq!(first.field("record"), "debug-log");
    assert_eq!(first.field("logpoint_id"), "1");
    assert_eq!(first.field("tick"), "0");
    assert_eq!(first.field("pc"), "0x00000200");
    assert_eq!(first.field("hits"), "1");
    assert_eq!(first.field("message"), "start");

    let second = &blocks[2];
    assert_eq!(second.field("logpoint_id"), "2");
    assert_eq!(second.field("tick"), "56");
    assert_eq!(second.field("pc"), "0x00000204");
    assert_eq!(second.field("hits"), "1");
    assert_eq!(second.field("message"), "hit");

    // A logpoint that never matches records nothing and still reaches the limit.
    let miss = single_block(&run_ok(&[
        "debug",
        "run",
        "--rom",
        rom_path,
        "--steps",
        "5",
        "--logpoint",
        "0x300",
    ]));
    assert_eq!(miss.field("reason"), "step-limit");
    assert_eq!(miss.field("log_entries"), "0");
}

#[cfg(unix)]
#[test]
fn debug_unreadable_rom_is_a_typed_failure() {
    use std::os::unix::fs::PermissionsExt;

    let dir = temp_dir("debug-unreadable");
    let rom = write_rom(&dir);
    let mode_off = std::fs::Permissions::from_mode(0o000);
    std::fs::set_permissions(&rom, mode_off).expect("remove read permission");
    // A privileged process (for example CI running as root) can read any file;
    // the premise of this test is unavailable there, so restore and stop.
    let read_denied = std::fs::File::open(&rom).is_err();
    if !read_denied {
        std::fs::set_permissions(&rom, std::fs::Permissions::from_mode(0o600))
            .expect("restore mode");
        return;
    }
    let stderr = run_fail(&["debug", "inspect", "--rom", path_str(&rom)]);
    std::fs::set_permissions(&rom, std::fs::Permissions::from_mode(0o600)).expect("restore mode");
    assert!(stderr.contains("failed to open ROM"), "{stderr}");
}

#[test]
fn debug_command_rejects_invalid_input_with_typed_errors() {
    let dir = temp_dir("debug-errors");
    let rom = write_rom(&dir);
    let rom_path = path_str(&rom);
    let missing = dir.join("missing.rom");
    let missing_path = path_str(&missing);
    let empty = dir.join("empty.rom");
    std::fs::write(&empty, []).expect("write empty ROM");
    let empty_path = path_str(&empty);
    let large = dir.join("large.rom");
    std::fs::File::create(&large)
        .expect("create sparse ROM")
        .set_len(u64::from(chronofork_megadrive::ROM_END) + 1)
        .expect("size sparse ROM");
    let large_path = path_str(&large);

    // Dispatch, required flags and the ROM read path.
    let stderr = run_fail(&["debug"]);
    assert!(stderr.contains("requires a subcommand"), "{stderr}");
    let stderr = run_fail(&["debug", "wat"]);
    assert!(stderr.contains("unknown `debug` subcommand"), "{stderr}");
    let stderr = run_fail(&["debug", "inspect"]);
    assert!(stderr.contains("requires `--rom <FILE>`"), "{stderr}");
    let stderr = run_fail(&["debug", "inspect", "--rom", missing_path]);
    assert!(stderr.contains("failed to open ROM"), "{stderr}");
    let stderr = run_fail(&["debug", "inspect", "--rom", path_str(&dir)]);
    assert!(stderr.contains("is not a regular file"), "{stderr}");
    let stderr = run_fail(&["debug", "inspect", "--rom", empty_path]);
    assert!(stderr.contains("512-byte header"), "{stderr}");
    let stderr = run_fail(&["debug", "inspect", "--rom", large_path]);
    assert!(stderr.contains("4 MiB"), "{stderr}");
    let stderr = run_fail(&["debug", "inspect", "--rom", ""]);
    assert!(stderr.contains("requires a non-empty value"), "{stderr}");
    let stderr = run_fail(&[
        "debug",
        "memory",
        "--rom",
        rom_path,
        "--address",
        "0x100000000",
        "--length",
        "4",
    ]);
    assert!(
        stderr.contains("exceeds the 32-bit address space"),
        "{stderr}"
    );
    let stderr = run_fail(&[
        "debug",
        "memory",
        "--rom",
        rom_path,
        "--address",
        "0x00FF0000",
        "--length",
        "zz",
    ]);
    assert!(
        stderr.contains("expects a decimal or `0x` hexadecimal number"),
        "{stderr}"
    );
    let stderr = run_fail(&["debug", "inspect", "--rom", rom_path, "--bogus"]);
    assert!(
        stderr.contains("unknown `debug inspect` option `--bogus`"),
        "{stderr}"
    );

    // `debug run` budget and trigger grammar.
    let stderr = run_fail(&["debug", "run", "--rom", rom_path]);
    assert!(stderr.contains("requires `--steps <N>`"), "{stderr}");
    let stderr = run_fail(&["debug", "run", "--rom", rom_path, "--steps", "0"]);
    assert!(stderr.contains("must be at least 1"), "{stderr}");
    let stderr = run_fail(&["debug", "run", "--rom", rom_path, "--steps", "1000001"]);
    assert!(stderr.contains("must be at most 1000000"), "{stderr}");
    let stderr = run_fail(&[
        "debug",
        "run",
        "--rom",
        rom_path,
        "--steps",
        "1",
        "--breakpoint",
        "0x200:reg:d0=3",
    ]);
    assert!(stderr.contains("must be `<name><op><value>`"), "{stderr}");
    let stderr = run_fail(&[
        "debug",
        "run",
        "--rom",
        rom_path,
        "--steps",
        "1",
        "--breakpoint",
        "0x200:nope",
    ]);
    assert!(
        stderr.contains("must start with `reg:` or `mem:`"),
        "{stderr}"
    );
    let stderr = run_fail(&[
        "debug",
        "run",
        "--rom",
        rom_path,
        "--steps",
        "1",
        "--breakpoint",
        "0x200:reg:zz==3",
    ]);
    assert!(stderr.contains("unknown register zz"), "{stderr}");
    let stderr = run_fail(&[
        "debug",
        "run",
        "--rom",
        rom_path,
        "--steps",
        "1",
        "--breakpoint",
        "0x200:mem:0x00FF0000:9==3",
    ]);
    assert!(stderr.contains("outside 1..=8 bytes"), "{stderr}");

    // Watchpoint grammar.
    let stderr = run_fail(&[
        "debug",
        "run",
        "--rom",
        rom_path,
        "--steps",
        "1",
        "--watchpoint",
        "0x00FF0000:x",
    ]);
    assert!(stderr.contains("must be `r`, `w` or `rw`"), "{stderr}");
    let stderr = run_fail(&[
        "debug",
        "run",
        "--rom",
        rom_path,
        "--steps",
        "1",
        "--watchpoint",
        "0x00FF0000",
    ]);
    assert!(
        stderr.contains("expects `<ADDR|START-END>:<r|w|rw>`"),
        "{stderr}"
    );
    let stderr = run_fail(&[
        "debug",
        "run",
        "--rom",
        rom_path,
        "--steps",
        "1",
        "--watchpoint",
        "0x00FF0002-0x00FF0000:w",
    ]);
    assert!(stderr.contains("is greater than end"), "{stderr}");

    // Eventpoint grammar.
    let stderr = run_fail(&[
        "debug",
        "run",
        "--rom",
        rom_path,
        "--steps",
        "1",
        "--eventpoint",
        "interrupt:maybe",
    ]);
    assert!(
        stderr.contains("must be `requested`, `delivered` or `acknowledged`"),
        "{stderr}"
    );
    let stderr = run_fail(&[
        "debug",
        "run",
        "--rom",
        rom_path,
        "--steps",
        "1",
        "--eventpoint",
        "vdp:sideways",
    ]);
    assert!(stderr.contains("must be `read` or `write`"), "{stderr}");
    let stderr = run_fail(&[
        "debug",
        "run",
        "--rom",
        rom_path,
        "--steps",
        "1",
        "--eventpoint",
        "controller:3",
    ]);
    assert!(stderr.contains("must be `1`, `2` or `any`"), "{stderr}");
    let stderr = run_fail(&[
        "debug",
        "run",
        "--rom",
        rom_path,
        "--steps",
        "1",
        "--eventpoint",
        "bus-write:0x200-0x100",
    ]);
    assert!(stderr.contains("is greater than end"), "{stderr}");
    let stderr = run_fail(&[
        "debug",
        "run",
        "--rom",
        rom_path,
        "--steps",
        "1",
        "--eventpoint",
        "nope:1",
    ]);
    assert!(
        stderr.contains("must be `interrupt:<phase>[:<level>]`"),
        "{stderr}"
    );

    // Logpoint message and event-ring bounds.
    let stderr = run_fail(&[
        "debug",
        "run",
        "--rom",
        rom_path,
        "--steps",
        "1",
        "--logpoint",
        "0x200:",
    ]);
    assert!(stderr.contains("must not be empty"), "{stderr}");
    let stderr = run_fail(&[
        "debug",
        "run",
        "--rom",
        rom_path,
        "--steps",
        "1",
        "--logpoint",
        "zz",
    ]);
    assert!(
        stderr.contains("expects a decimal or `0x` hexadecimal number"),
        "{stderr}"
    );
    let stderr = run_fail(&[
        "debug",
        "run",
        "--rom",
        rom_path,
        "--steps",
        "1",
        "--max-events",
        "0",
    ]);
    assert!(stderr.contains("between 1 and 65536"), "{stderr}");
    let stderr = run_fail(&[
        "debug",
        "run",
        "--rom",
        rom_path,
        "--steps",
        "1",
        "--max-events",
        "65537",
    ]);
    assert!(stderr.contains("between 1 and 65536"), "{stderr}");
    let stderr = run_fail(&[
        "debug",
        "run",
        "--rom",
        rom_path,
        "--steps",
        "1",
        "--show-events",
        "65537",
    ]);
    assert!(stderr.contains("between 0 and 65536"), "{stderr}");

    // A ROM whose reset vector cannot execute is a typed runtime failure, not a
    // partial report.
    let mut broken = synthetic_rom();
    broken[4..8].copy_from_slice(&0x00A1_0000_u32.to_be_bytes());
    let broken = write_named_rom(&dir, "broken-vector.rom", broken);
    let stderr = run_fail(&["debug", "run", "--rom", path_str(&broken), "--steps", "1"]);
    assert!(stderr.contains("unmapped read"), "{stderr}");
    let stderr = run_fail(&[
        "debug",
        "inspect",
        "--rom",
        path_str(&broken),
        "--steps",
        "1",
    ]);
    assert!(
        stderr.contains("execution failed after 0 instruction(s)"),
        "{stderr}"
    );

    // A repeated `--rom` is rejected before any read.
    let stderr = run_fail(&["debug", "inspect", "--rom", rom_path, "--rom", rom_path]);
    assert!(
        stderr.contains("`--rom` was given more than once"),
        "{stderr}"
    );

    // A full trigger table is a typed failure, not a silent drop.
    let mut args: Vec<String> = vec![
        "debug".to_owned(),
        "run".to_owned(),
        "--rom".to_owned(),
        rom_path.to_owned(),
        "--steps".to_owned(),
        "1".to_owned(),
    ];
    for _ in 0..65 {
        args.push("--breakpoint".to_owned());
        args.push("0x200".to_owned());
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let stderr = run_fail(&refs);
    assert!(stderr.contains("the breakpoint table is full"), "{stderr}");

    // Help lists the implemented surface and the main help dispatches to it.
    let help = run_ok(&["debug", "--help"]);
    for expected in [
        "debug run",
        "debug inspect",
        "debug memory",
        "debug disasm",
        "debug vdp",
        "debug interrupts",
        "debug bus",
        "--breakpoint",
        "--watchpoint",
        "--eventpoint",
        "--logpoint",
        "--max-events",
        "--show-events",
        "--address",
        "--length",
        "--count",
        "--vram-address",
        "--vram-length",
        "debug-run",
        "debug-event",
        "debug-log",
    ] {
        assert!(
            help.contains(expected),
            "debug help is missing `{expected}`:\n{help}"
        );
    }
    assert!(
        !help.contains("--video") && !help.contains("--instructions-per-line"),
        "debug help must not advertise scheduler flags:\n{help}"
    );
    let main_help = run_ok(&["help"]);
    assert!(
        main_help.contains("chronofork debug run --rom <FILE> --steps <N>"),
        "main help must list the debug group:\n{main_help}"
    );
    assert!(
        main_help.contains("chronofork debug bus --rom <FILE>"),
        "main help must list the bus view:\n{main_help}"
    );
}
