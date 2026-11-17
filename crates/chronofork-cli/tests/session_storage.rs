#![allow(clippy::expect_used)]

//! Process-level acceptance tests for `chronofork session save/load/list/delete`.
//!
//! Every invocation spawns the built `chronofork` binary with `Command::output`, so
//! each call is a separate operating-system process: the saving process has exited
//! (and been reaped) before the loading process starts, and no in-memory handle is
//! shared. The command line, exit status and the two output streams are the whole
//! contract observed here; storage correctness is covered by the
//! `chronofork-sessions` suites, which own the repository.

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Output;

use chronofork_backend_api::{BackendId, BackendSnapshot};
use chronofork_core::{SessionId, SnapshotId, StateHash, Tick, hash_bytes};
use chronofork_sessions::{DurableLineage, Snapshot, SnapshotRepository};
use common::{path_str, run, temp_dir, write_rom};

/// One parsed `key=value` record block.
type Block = BTreeMap<String, String>;

fn describe(output: &Output) -> (String, String) {
    (
        String::from_utf8(output.stdout.clone()).expect("stdout is UTF-8"),
        String::from_utf8(output.stderr.clone()).expect("stderr is UTF-8"),
    )
}

/// Runs the CLI as a new process, requiring exit 0, empty stderr and data on stdout.
fn run_ok(args: &[&str]) -> String {
    let output = run(args);
    let (stdout, stderr) = describe(&output);
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

/// Runs the CLI as a new process, requiring exit 1, empty stdout and a diagnostic.
fn run_fail(args: &[&str]) -> String {
    let output = run(args);
    let (stdout, stderr) = describe(&output);
    assert_eq!(
        Some(1),
        output.status.code(),
        "typed failures exit 1: {args:?}\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.is_empty(),
        "typed failures must keep stdout data-only: {args:?}\n{stdout}"
    );
    assert!(
        stderr.starts_with("chronofork: "),
        "stderr must carry a `chronofork:` diagnostic: {args:?}\n{stderr}"
    );
    stderr
}

/// Builds the argv for one `session` subcommand.
fn session_args(parts: &[&str]) -> Vec<String> {
    let mut args = vec!["session".to_owned()];
    args.extend(parts.iter().map(|part| (*part).to_owned()));
    args
}

fn refs(args: &[String]) -> Vec<&str> {
    args.iter().map(String::as_str).collect()
}

/// Runs a `session` subcommand in a new process, requiring success.
fn session_ok(parts: &[&str]) -> String {
    let args = session_args(parts);
    run_ok(&refs(&args))
}

/// Runs a `session` subcommand in a new process, requiring a typed failure.
fn session_fail(parts: &[&str]) -> String {
    let args = session_args(parts);
    run_fail(&refs(&args))
}

/// Runs a `session` subcommand in a new process without assertions on its streams.
fn session_raw(parts: &[&str]) -> Output {
    let args = session_args(parts);
    run(&refs(&args))
}

/// Parses the documented `key=value` block format, rejecting malformed output.
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
    let mut current = Block::new();
    for line in stdout.lines() {
        if line.is_empty() {
            assert!(!current.is_empty(), "output must not contain empty blocks");
            blocks.push(std::mem::take(&mut current));
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .expect("every record line is `key=value`");
        assert!(!key.is_empty(), "record key must not be empty");
        assert!(
            current.insert(key.to_owned(), value.to_owned()).is_none(),
            "duplicate field `{key}` inside one record block"
        );
    }
    assert!(
        !current.is_empty(),
        "output must not end with a blank block"
    );
    blocks.push(current);
    blocks
}

fn field<'a>(block: &'a Block, key: &str) -> &'a str {
    block
        .get(key)
        .map(String::as_str)
        .expect("record block contains the field")
}

fn session_key(block: &Block) -> String {
    block
        .get("session_key")
        .expect("block has a session_key")
        .clone()
}

fn snapshot_key(block: &Block) -> String {
    block
        .get("snapshot_key")
        .expect("block has a snapshot_key")
        .clone()
}

/// Saves a new session and returns its session block and first snapshot block.
fn save_new_session(root: &Path, name: &str, steps: u64) -> (Block, Block) {
    let steps = steps.to_string();
    let saved = parse_blocks(&session_ok(&[
        "save",
        "--root",
        path_str(root),
        "--name",
        name,
        "--steps",
        &steps,
    ]));
    assert_eq!(saved.len(), 2, "save prints a session and a snapshot block");
    let mut saved = saved;
    let snapshot = saved.pop().expect("snapshot block");
    let session = saved.pop().expect("session block");
    (session, snapshot)
}

/// Publishes one valid storage fixture whose envelope names a foreign backend.
///
/// The fixture uses only the canonical repository API: the CLI process under test
/// must reject it at rehydration through the single backend compatibility predicate.
fn write_foreign_snapshot(root: &Path) -> (String, String) {
    let repository = SnapshotRepository::initialize(root).expect("initialize fixture root");
    let session = repository
        .create_session(
            SessionId::from_raw(1),
            Some("foreign"),
            DurableLineage::root(),
        )
        .expect("create fixture session");
    let backend = BackendSnapshot::new(
        BackendId::new("chronofork.test.foreign"),
        "0.0.1",
        1,
        Tick::ZERO,
        StateHash::from_bytes([0_u8; 32]),
        StateHash::from_bytes([1_u8; 32]),
        Vec::new(),
    );
    let snapshot = Snapshot::new(SnapshotId::from_raw(1), SessionId::from_raw(1), backend);
    let entry = repository
        .save_snapshot(session.session_key, &snapshot, Some("foreign"))
        .expect("publish fixture snapshot");
    (
        session.session_key.to_string(),
        entry.snapshot_key.to_string(),
    )
}

/// The documented fixed layout path for a session metadata record.
fn session_metadata_path(root: &Path, session_key: &str) -> PathBuf {
    root.join("v1")
        .join("sessions")
        .join(format!("session-{session_key}"))
        .join("session.cfmeta")
}

/// The documented fixed layout path for a snapshot checkpoint.
fn checkpoint_path(root: &Path, session_key: &str, snapshot_key: &str) -> PathBuf {
    root.join("v1")
        .join("sessions")
        .join(format!("session-{session_key}"))
        .join("snapshots")
        .join(format!("snapshot-{snapshot_key}"))
        .join("checkpoint.cfsnap")
}

/// Content fingerprint of a storage tree: sorted relative paths plus file digests.
fn tree_fingerprint(root: &Path) -> Vec<(String, String)> {
    fn walk(root: &Path, directory: &Path, entries: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(directory).expect("read directory") {
            let entry = entry.expect("directory entry");
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .expect("entry is below the storage root");
            let name = relative.to_string_lossy().into_owned();
            if entry.file_type().expect("entry file type").is_dir() {
                entries.push((name, "<dir>".to_owned()));
                walk(root, &path, entries);
            } else {
                let bytes = std::fs::read(&path).expect("read stored file");
                entries.push((name, hash_bytes(&bytes).to_hex()));
            }
        }
    }
    let mut entries = Vec::new();
    walk(root, root, &mut entries);
    entries.sort();
    entries
}

#[test]
fn session_usage_requires_explicit_root() {
    let base = temp_dir("usage");
    // Every session command requires its own explicit root; there is no default root,
    // no environment-variable root and no implicit creation.
    for subcommand in ["save", "load", "list", "delete"] {
        let stderr = session_fail(&[subcommand]);
        assert!(
            stderr.contains("requires `--root <DIR>`"),
            "`session {subcommand}` must require --root:\n{stderr}"
        );
    }
    // load and delete additionally require an explicit durable target.
    let empty = base.join("empty");
    std::fs::create_dir(&empty).expect("create usable directory");
    for subcommand in ["load", "delete"] {
        let stderr = session_fail(&[subcommand, "--root", path_str(&empty)]);
        assert!(
            stderr.contains("requires `--session <KEY>`"),
            "`session {subcommand}` must require --session:\n{stderr}"
        );
    }
    // A missing value and unknown or misplaced flags are rejected, not ignored.
    let stderr = session_fail(&["save", "--root"]);
    assert!(stderr.contains("requires a value"), "{stderr}");
    let stderr = session_fail(&["save", "--root", path_str(&empty), "--yaml"]);
    assert!(
        stderr.contains("unknown `session save` option `--yaml`"),
        "{stderr}"
    );
    for parts in [
        vec!["save", "--root", path_str(&empty), "--snapshot", "1"],
        vec!["list", "--root", path_str(&empty), "--name", "x"],
        vec![
            "delete",
            "--root",
            path_str(&empty),
            "--session",
            "1",
            "--steps",
            "1",
        ],
    ] {
        let stderr = session_fail(&parts);
        assert!(
            stderr.contains("unknown `session"),
            "flags are not interchangeable across subcommands:\n{stderr}"
        );
    }
    // Rejected invocations never initialized the empty directory.
    assert_eq!(
        std::fs::read_dir(&empty)
            .expect("read empty directory")
            .count(),
        0,
        "usage errors must not create a storage root"
    );
    let stderr = session_fail(&["frobnicate"]);
    assert!(stderr.contains("unknown `session` subcommand"), "{stderr}");
}

#[test]
fn save_then_load_in_a_new_process_round_trips() {
    let root = temp_dir("round-trip");
    let saved = parse_blocks(&session_ok(&[
        "save",
        "--root",
        path_str(&root),
        "--name",
        "baseline",
        "--steps",
        "7",
    ]));
    assert_eq!(saved.len(), 2);
    let session = &saved[0];
    let snapshot = &saved[1];
    assert_eq!(field(session, "record"), "session");
    assert_eq!(field(session, "name"), "baseline");
    let session_key = session_key(session);
    assert_eq!(field(snapshot, "record"), "snapshot");
    assert_eq!(field(snapshot, "session_key"), session_key.as_str());
    assert_eq!(field(snapshot, "name"), "baseline");
    assert_eq!(field(snapshot, "tick"), "7");
    assert_eq!(field(snapshot, "creation_sequence"), "1");
    assert_eq!(field(snapshot, "parent_session_key"), "-");
    assert_eq!(field(snapshot, "origin_snapshot_key"), "-");
    let snapshot_key = snapshot_key(snapshot);
    let state_hash = field(snapshot, "state_hash").to_owned();
    assert_eq!(state_hash.len(), 64, "state_hash is a SHA-256 digest");
    assert!(
        state_hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "state_hash is lowercase hexadecimal: {state_hash}"
    );

    // `run` reaped the saving process before this second process started.
    let loaded = parse_blocks(&session_ok(&[
        "load",
        "--root",
        path_str(&root),
        "--session",
        session_key.as_str(),
    ]));
    assert_eq!(loaded.len(), 1, "load prints one snapshot block");
    let loaded = &loaded[0];
    assert_eq!(field(loaded, "record"), "snapshot");
    assert_eq!(field(loaded, "session_key"), session_key.as_str());
    assert_eq!(field(loaded, "snapshot_key"), snapshot_key.as_str());
    assert_eq!(field(loaded, "name"), "baseline");
    assert_eq!(field(loaded, "tick"), "7");
    assert_eq!(field(loaded, "state_hash"), state_hash.as_str());
    assert_eq!(field(loaded, "creation_sequence"), "1");
}

#[test]
fn identical_inputs_reproduce_the_same_state_hash_across_processes() {
    let first_root = temp_dir("determinism-a");
    let second_root = temp_dir("determinism-b");
    let save = |root: &Path, input: &str| {
        let saved = parse_blocks(&session_ok(&[
            "save",
            "--root",
            path_str(root),
            "--name",
            "same",
            "--steps",
            "6",
            "--input",
            input,
        ]));
        let snapshot = saved.into_iter().nth(1).expect("snapshot block");
        (
            field(&snapshot, "tick").to_owned(),
            field(&snapshot, "state_hash").to_owned(),
        )
    };
    let first = save(&first_root, "42");
    let second = save(&second_root, "42");
    assert_eq!(first, second, "same inputs reproduce the same state hash");
    let different = save(&first_root, "43");
    assert_eq!(first.0, different.0, "input does not change the tick");
    assert_ne!(
        first.1, different.1,
        "input is part of the canonical state, so the hash changes"
    );
}

#[test]
fn save_name_is_visible_after_new_process() {
    let root = temp_dir("name");
    let name = "baseline run=1";
    let saved = parse_blocks(&session_ok(&[
        "save",
        "--root",
        path_str(&root),
        "--name",
        name,
        "--steps",
        "1",
    ]));
    let named_key = session_key(&saved[0]);
    assert_eq!(field(&saved[0], "name"), name);
    assert_eq!(field(&saved[1], "name"), name);

    let listed = parse_blocks(&session_ok(&[
        "list",
        "--root",
        path_str(&root),
        "--session",
        named_key.as_str(),
    ]));
    assert_eq!(listed.len(), 2);
    assert_eq!(field(&listed[0], "name"), name);
    assert_eq!(field(&listed[1], "name"), name);
    let loaded = parse_blocks(&session_ok(&[
        "load",
        "--root",
        path_str(&root),
        "--session",
        named_key.as_str(),
    ]));
    assert_eq!(field(&loaded[0], "name"), name);

    // An unnamed save is recorded as an empty `name=` label, and still creates a new
    // durable session instead of overwriting the named one.
    let unnamed = parse_blocks(&session_ok(&["save", "--root", path_str(&root)]));
    assert_eq!(field(&unnamed[0], "name"), "");
    assert_eq!(field(&unnamed[1], "name"), "");
    assert_ne!(session_key(&unnamed[0]), named_key);

    // Name rules are the service catalog's validated `name` field, reported through
    // the CLI. Moving validation to the shared service boundary changed the wording
    // from the storage layer's `InvalidName` message to this typed rejection.
    let stderr = session_fail(&["save", "--root", path_str(&root), "--name", "bad\nname"]);
    assert!(
        stderr.contains("`name` must not contain control characters"),
        "{stderr}"
    );
}

#[test]
fn save_session_continue_and_save_appends_snapshot() {
    let root = temp_dir("continue");
    let (session, first) = save_new_session(&root, "first", 4);
    let session_key = session_key(&session);
    let first_snapshot = snapshot_key(&first);
    let first_hash = field(&first, "state_hash").to_owned();
    let first_checkpoint = checkpoint_path(&root, &session_key, &first_snapshot);
    let first_bytes = std::fs::read(&first_checkpoint).expect("read first checkpoint");

    // P2 continues the committed session in a new process and appends one snapshot.
    let appended = parse_blocks(&session_ok(&[
        "save",
        "--root",
        path_str(&root),
        "--session",
        session_key.as_str(),
        "--name",
        "second",
        "--steps",
        "3",
    ]));
    assert_eq!(appended.len(), 2);
    assert_eq!(field(&appended[0], "session_key"), session_key.as_str());
    assert_eq!(
        field(&appended[0], "name"),
        "first",
        "continue-and-save keeps the session record"
    );
    let second = &appended[1];
    assert_eq!(field(second, "session_key"), session_key.as_str());
    assert_eq!(field(second, "name"), "second");
    assert_ne!(field(second, "snapshot_key"), first_snapshot.as_str());
    assert_eq!(field(second, "creation_sequence"), "2");
    assert_eq!(field(second, "tick"), "7");
    assert_ne!(field(second, "state_hash"), first_hash.as_str());
    let second_snapshot = snapshot_key(second);
    assert_eq!(
        std::fs::read(&first_checkpoint).expect("re-read first checkpoint"),
        first_bytes,
        "appending must not rewrite an existing checkpoint"
    );

    // P3 re-reads both snapshots with increasing `creation_sequence` values.
    let listed = parse_blocks(&session_ok(&[
        "list",
        "--root",
        path_str(&root),
        "--session",
        session_key.as_str(),
    ]));
    assert_eq!(listed.len(), 3);
    assert_eq!(field(&listed[0], "record"), "session");
    assert_eq!(field(&listed[1], "snapshot_key"), first_snapshot.as_str());
    assert_eq!(field(&listed[1], "creation_sequence"), "1");
    assert_eq!(field(&listed[2], "snapshot_key"), second_snapshot.as_str());
    assert_eq!(field(&listed[2], "creation_sequence"), "2");
    let whole_root = parse_blocks(&session_ok(&["list", "--root", path_str(&root)]));
    assert_eq!(whole_root.len(), 3, "continue-and-save adds no session");
    assert_eq!(
        whole_root
            .iter()
            .filter(|block| field(block, "record") == "session")
            .count(),
        1,
        "the appended snapshot belongs to the existing session"
    );

    // `load` defaults to the latest committed snapshot; `--snapshot` re-reads the old
    // checkpoint bytes without writing anything.
    let latest = parse_blocks(&session_ok(&[
        "load",
        "--root",
        path_str(&root),
        "--session",
        session_key.as_str(),
    ]));
    assert_eq!(field(&latest[0], "snapshot_key"), second_snapshot.as_str());
    assert_eq!(field(&latest[0], "tick"), "7");
    let original = parse_blocks(&session_ok(&[
        "load",
        "--root",
        path_str(&root),
        "--session",
        session_key.as_str(),
        "--snapshot",
        first_snapshot.as_str(),
    ]));
    assert_eq!(field(&original[0], "snapshot_key"), first_snapshot.as_str());
    assert_eq!(field(&original[0], "creation_sequence"), "1");
    assert_eq!(field(&original[0], "tick"), "4");
    assert_eq!(field(&original[0], "state_hash"), first_hash.as_str());
    assert_eq!(
        std::fs::read(&first_checkpoint).expect("re-read first checkpoint"),
        first_bytes,
        "a read must not change a committed checkpoint"
    );
}

#[test]
fn continue_and_save_contends_with_a_process_holding_the_writer_lock() {
    let root = temp_dir("continue-contended");
    let (session, first) = save_new_session(&root, "first", 2);
    let session_key_text = session_key(&session);
    let durable_key: u64 = session_key_text.parse().expect("durable session key");
    let first_snapshot_text = snapshot_key(&first);
    let first_path = checkpoint_path(&root, &session_key_text, &first_snapshot_text);
    let first_bytes = std::fs::read(&first_path).expect("read first checkpoint");
    let snapshots = root
        .join("v1")
        .join("sessions")
        .join(format!("session-{session_key_text}"))
        .join("snapshots");
    let snapshot_directories = |path: &Path| {
        std::fs::read_dir(path)
            .expect("read snapshots directory")
            .count()
    };
    let snapshots_before = snapshot_directories(&snapshots);

    // The test harness is a real operating-system process separate from the CLI
    // child. It holds the actual per-session writer lock through the repository API,
    // so the contended CLI process must fail before it can read the base checkpoint.
    let holder = SnapshotRepository::open(&root).expect("open root for the lock holder");
    let held = holder
        .acquire_writer_lock(durable_key)
        .expect("hold the real writer lock");
    let stderr = session_fail(&[
        "save",
        "--root",
        path_str(&root),
        "--session",
        &session_key_text,
        "--steps",
        "1",
    ]);
    assert!(
        stderr.contains("writer lock is held by owner"),
        "the contended process reports typed writer contention:\n{stderr}"
    );
    assert!(
        stderr.contains(&format!("durable session {durable_key}")),
        "the contention diagnostic names the durable session:\n{stderr}"
    );
    assert!(
        !stderr.contains("corrupt") && !stderr.contains("invalid"),
        "contention is not reported as corruption or invalid data:\n{stderr}"
    );

    // Nothing was read-and-published: no snapshot directory, no key or creation
    // sequence reservation, and the existing checkpoint is byte-identical.
    assert_eq!(
        snapshot_directories(&snapshots),
        snapshots_before,
        "a contended continue-and-save creates no snapshot"
    );
    assert!(
        !snapshots.join("snapshot-2").exists(),
        "the contended process must not reserve or create snapshot 2"
    );
    assert_eq!(
        holder
            .session(durable_key)
            .expect("session record")
            .next_snapshot_key,
        2,
        "the contended process must not reserve a snapshot key"
    );
    assert_eq!(
        holder
            .latest_snapshot(durable_key)
            .expect("latest snapshot")
            .expect("the base snapshot")
            .snapshot_key,
        first_snapshot_text.parse().expect("first snapshot key"),
        "the base snapshot is unchanged"
    );
    assert_eq!(
        std::fs::read(&first_path).expect("re-read first checkpoint"),
        first_bytes,
        "the first checkpoint bytes are unchanged"
    );

    // After the holder releases, one normal continue-and-save appends exactly one
    // snapshot from the same base, with a greater creation sequence.
    holder.release_writer_lock(&held).expect("release the lock");
    let appended = parse_blocks(&session_ok(&[
        "save",
        "--root",
        path_str(&root),
        "--session",
        &session_key_text,
        "--steps",
        "3",
    ]));
    assert_eq!(appended.len(), 2);
    let second = &appended[1];
    assert_eq!(field(second, "snapshot_key"), "2");
    assert_eq!(field(second, "creation_sequence"), "2");
    assert_eq!(field(second, "tick"), "5");
    assert_eq!(
        std::fs::read(&first_path).expect("re-read first checkpoint after append"),
        first_bytes,
        "continuing after contention must not rewrite the prior checkpoint"
    );
    assert_eq!(snapshot_directories(&snapshots), snapshots_before + 1);
}

#[test]
fn list_is_deterministic_and_ordered() {
    let root = temp_dir("list-order");
    let (first_session, first_snapshot) = save_new_session(&root, "a", 1);
    let (second_session, _) = save_new_session(&root, "b", 2);
    let first_key = session_key(&first_session);
    let second_key = session_key(&second_session);
    let appended = parse_blocks(&session_ok(&[
        "save",
        "--root",
        path_str(&root),
        "--session",
        first_key.as_str(),
        "--name",
        "a2",
        "--steps",
        "3",
    ]));
    let appended_snapshot = snapshot_key(&appended[1]);

    let before = tree_fingerprint(&root);
    let first_list = session_ok(&["list", "--root", path_str(&root)]);
    let second_list = session_ok(&["list", "--root", path_str(&root)]);
    assert_eq!(
        first_list, second_list,
        "list must be byte-identical across processes"
    );
    assert!(
        !first_list.contains("\n\n\n"),
        "blocks are separated by exactly one blank line:\n{first_list}"
    );
    assert_eq!(
        tree_fingerprint(&root),
        before,
        "list must not write storage"
    );

    let blocks = parse_blocks(&first_list);
    assert_eq!(blocks.len(), 5, "two sessions plus three snapshots");
    assert_eq!(field(&blocks[0], "record"), "session");
    assert_eq!(field(&blocks[0], "session_key"), first_key.as_str());
    assert_eq!(
        field(&blocks[1], "snapshot_key"),
        snapshot_key(&first_snapshot).as_str()
    );
    assert_eq!(field(&blocks[1], "creation_sequence"), "1");
    assert_eq!(
        field(&blocks[2], "snapshot_key"),
        appended_snapshot.as_str()
    );
    assert_eq!(field(&blocks[2], "creation_sequence"), "2");
    assert_eq!(field(&blocks[3], "session_key"), second_key.as_str());
    assert_eq!(field(&blocks[4], "session_key"), second_key.as_str());
    let first_number: u64 = first_key.parse().expect("session key is decimal");
    let second_number: u64 = second_key.parse().expect("session key is decimal");
    assert!(
        first_number < second_number,
        "sessions are ordered by durable session key"
    );

    // A single-session listing is the same ordered subset.
    let single = parse_blocks(&session_ok(&[
        "list",
        "--root",
        path_str(&root),
        "--session",
        first_key.as_str(),
    ]));
    assert_eq!(single.len(), 3);
    assert_eq!(field(&single[0], "session_key"), first_key.as_str());
    assert_eq!(
        field(&single[1], "snapshot_key"),
        snapshot_key(&first_snapshot).as_str()
    );
    assert_eq!(
        field(&single[2], "snapshot_key"),
        appended_snapshot.as_str()
    );
}

#[test]
fn load_does_not_create_a_missing_root() {
    let base = temp_dir("missing-root");
    for subcommand in ["load", "list", "delete"] {
        let root = base.join(format!("{subcommand}-root"));
        let stderr = session_fail(&[subcommand, "--root", path_str(&root), "--session", "1"]);
        assert!(
            stderr.contains("storage root is not initialized"),
            "`session {subcommand}` must report the typed NotInitialized error:\n{stderr}"
        );
        assert!(
            !root.exists(),
            "`session {subcommand}` must not create a missing root"
        );
    }
    // An existing directory without the marker is not a root either, and stays empty.
    let empty = base.join("empty");
    std::fs::create_dir(&empty).expect("create empty directory");
    let stderr = session_fail(&["load", "--root", path_str(&empty), "--session", "1"]);
    assert!(
        stderr.contains("storage root is not initialized"),
        "{stderr}"
    );
    assert_eq!(
        std::fs::read_dir(&empty)
            .expect("read empty directory")
            .count(),
        0,
        "load must not initialize the directory"
    );
}

#[test]
fn save_initializes_only_an_empty_root() {
    let base = temp_dir("save-init");
    // A non-empty directory without a marker is refused, never adopted or cleaned.
    // The shared synthetic ROM stands in for unrelated project data: a directory that
    // holds a cartridge image is not a storage root.
    let foreign = base.join("foreign");
    std::fs::create_dir(&foreign).expect("create foreign directory");
    let rom = write_rom(&foreign);
    let stderr = session_fail(&["save", "--root", path_str(&foreign)]);
    assert!(stderr.contains("storage root is not empty"), "{stderr}");
    assert!(rom.is_file(), "foreign data is preserved");
    assert!(
        !foreign.join("chronofork.store").exists(),
        "a refused root is not initialized"
    );

    // Continuing a session requires an existing root and never creates one.
    let missing = base.join("missing");
    let stderr = session_fail(&["save", "--root", path_str(&missing), "--session", "1"]);
    assert!(
        stderr.contains("storage root is not initialized"),
        "{stderr}"
    );
    assert!(
        !missing.exists(),
        "a failed continue must not create a root"
    );

    // An empty directory is initialized, and every save without --session creates a
    // new durable session instead of overwriting the previous one.
    let root = base.join("root");
    std::fs::create_dir(&root).expect("create empty root");
    let first = parse_blocks(&session_ok(&[
        "save",
        "--root",
        path_str(&root),
        "--steps",
        "1",
    ]));
    assert!(root.join("chronofork.store").is_file());
    let second = parse_blocks(&session_ok(&[
        "save",
        "--root",
        path_str(&root),
        "--steps",
        "2",
    ]));
    assert_eq!(field(&first[0], "session_key"), "1");
    assert_eq!(
        field(&second[0], "session_key"),
        "2",
        "save without --session never overwrites a durable session"
    );
    let listed = parse_blocks(&session_ok(&["list", "--root", path_str(&root)]));
    assert_eq!(listed.len(), 4, "two sessions plus their snapshots");
}

#[test]
fn unknown_session_exits_one_with_typed_diagnostic() {
    let root = temp_dir("unknown-session");
    session_ok(&["save", "--root", path_str(&root), "--steps", "1"]);
    for subcommand in ["load", "list", "delete", "save"] {
        let stderr = session_fail(&[subcommand, "--root", path_str(&root), "--session", "9999"]);
        assert!(
            stderr.contains("unknown durable session key: 9999"),
            "`session {subcommand}` must report the typed unknown session:\n{stderr}"
        );
    }
    // Canonical key parsing is part of the surface: `0` and leading zeroes are usage
    // errors, not durable targets, and an out-of-range key is rejected rather than
    // truncated.
    for key in ["0", "007"] {
        let stderr = session_fail(&["load", "--root", path_str(&root), "--session", key]);
        assert!(
            stderr.contains("canonical positive decimal key"),
            "`--session {key}` is not a canonical durable key:\n{stderr}"
        );
    }
    let stderr = session_fail(&[
        "load",
        "--root",
        path_str(&root),
        "--session",
        "18446744073709551616",
    ]);
    assert!(stderr.contains("is too large"), "{stderr}");
}

#[test]
fn unknown_snapshot_exits_one_with_typed_diagnostic() {
    let root = temp_dir("unknown-snapshot");
    let (session, snapshot) = save_new_session(&root, "base", 2);
    let session_key = session_key(&session);
    let snapshot_key = snapshot_key(&snapshot);
    let before = tree_fingerprint(&root);
    let stderr = session_fail(&[
        "load",
        "--root",
        path_str(&root),
        "--session",
        session_key.as_str(),
        "--snapshot",
        "9999",
    ]);
    assert!(
        stderr.contains(&format!(
            "unknown durable snapshot key 9999 for durable session {session_key}"
        )),
        "{stderr}"
    );
    let stderr = session_fail(&[
        "delete",
        "--root",
        path_str(&root),
        "--session",
        session_key.as_str(),
        "--snapshot",
        "9999",
    ]);
    assert!(
        stderr.contains("unknown durable snapshot key 9999"),
        "{stderr}"
    );
    assert_eq!(
        tree_fingerprint(&root),
        before,
        "failed reads/deletes must not modify the store"
    );
    // The known snapshot is untouched and still loadable.
    let loaded = parse_blocks(&session_ok(&[
        "load",
        "--root",
        path_str(&root),
        "--session",
        session_key.as_str(),
        "--snapshot",
        snapshot_key.as_str(),
    ]));
    assert_eq!(field(&loaded[0], "snapshot_key"), snapshot_key.as_str());
}

#[test]
fn load_steps_advance_only_in_memory() {
    let root = temp_dir("load-steps");
    let (session, snapshot) = save_new_session(&root, "base", 2);
    let session_key = session_key(&session);
    let snapshot_key = snapshot_key(&snapshot);
    let stored_hash = field(&snapshot, "state_hash").to_owned();
    let before = tree_fingerprint(&root);

    let advanced = session_ok(&[
        "load",
        "--root",
        path_str(&root),
        "--session",
        session_key.as_str(),
        "--steps",
        "5",
        "--input",
        "9",
    ]);
    let advanced_blocks = parse_blocks(&advanced);
    assert_eq!(advanced_blocks.len(), 1);
    assert_eq!(field(&advanced_blocks[0], "tick"), "7");
    assert_eq!(
        field(&advanced_blocks[0], "snapshot_key"),
        snapshot_key.as_str()
    );
    assert_ne!(
        field(&advanced_blocks[0], "state_hash"),
        stored_hash.as_str()
    );

    // Replaying the same in-memory advance is deterministic across processes.
    let again = session_ok(&[
        "load",
        "--root",
        path_str(&root),
        "--session",
        session_key.as_str(),
        "--steps",
        "5",
        "--input",
        "9",
    ]);
    assert_eq!(again, advanced);

    // A later process still sees the committed checkpoint and nothing extra.
    let stored = parse_blocks(&session_ok(&[
        "load",
        "--root",
        path_str(&root),
        "--session",
        session_key.as_str(),
    ]));
    assert_eq!(field(&stored[0], "tick"), "2");
    assert_eq!(field(&stored[0], "state_hash"), stored_hash.as_str());
    let listed = parse_blocks(&session_ok(&[
        "list",
        "--root",
        path_str(&root),
        "--session",
        session_key.as_str(),
    ]));
    assert_eq!(
        listed.len(),
        2,
        "the in-memory advance persisted no snapshot"
    );
    assert_eq!(
        tree_fingerprint(&root),
        before,
        "load must not write storage"
    );
}

#[test]
fn delete_requires_an_explicit_target_and_never_cascades() {
    let root = temp_dir("delete");
    let (session, snapshot) = save_new_session(&root, "keep", 3);
    let session_key = session_key(&session);
    let snapshot_key = snapshot_key(&snapshot);

    // Deleting a session that still has snapshots is rejected; nothing is removed.
    let stderr = session_fail(&[
        "delete",
        "--root",
        path_str(&root),
        "--session",
        session_key.as_str(),
    ]);
    assert!(
        stderr.contains("still contains snapshots; delete them explicitly first"),
        "{stderr}"
    );
    let listed = parse_blocks(&session_ok(&[
        "list",
        "--root",
        path_str(&root),
        "--session",
        session_key.as_str(),
    ]));
    assert_eq!(listed.len(), 2, "a rejected delete removes nothing");

    // Deleting exactly one snapshot leaves the session and its other records intact.
    let deleted = parse_blocks(&session_ok(&[
        "delete",
        "--root",
        path_str(&root),
        "--session",
        session_key.as_str(),
        "--snapshot",
        snapshot_key.as_str(),
    ]));
    assert_eq!(deleted.len(), 1);
    assert_eq!(field(&deleted[0], "record"), "deleted");
    assert_eq!(field(&deleted[0], "session_key"), session_key.as_str());
    assert_eq!(field(&deleted[0], "snapshot_key"), snapshot_key.as_str());
    let remaining = parse_blocks(&session_ok(&[
        "list",
        "--root",
        path_str(&root),
        "--session",
        session_key.as_str(),
    ]));
    assert_eq!(remaining.len(), 1, "delete removes only the named snapshot");
    assert_eq!(field(&remaining[0], "record"), "session");
    assert_eq!(field(&remaining[0], "name"), "keep");

    // Deleting the now-empty session is explicit and reports only the session key.
    let deleted = parse_blocks(&session_ok(&[
        "delete",
        "--root",
        path_str(&root),
        "--session",
        session_key.as_str(),
    ]));
    assert_eq!(deleted.len(), 1);
    assert_eq!(field(&deleted[0], "record"), "deleted");
    assert_eq!(field(&deleted[0], "session_key"), session_key.as_str());
    assert!(
        !deleted[0].contains_key("snapshot_key"),
        "a session deletion has no snapshot_key field"
    );
    let empty = session_ok(&["list", "--root", path_str(&root)]);
    assert!(empty.is_empty(), "the deleted session is gone: {empty}");
    let stderr = session_fail(&[
        "delete",
        "--root",
        path_str(&root),
        "--session",
        session_key.as_str(),
    ]);
    assert!(stderr.contains("unknown durable session key"), "{stderr}");
}

#[test]
fn corrupt_storage_metadata_exits_one_with_typed_diagnostic() {
    let root = temp_dir("corrupt-metadata");
    let (session, _) = save_new_session(&root, "base", 1);
    let session_key = session_key(&session);
    let path = session_metadata_path(&root, &session_key);
    let mut bytes = std::fs::read(&path).expect("read session metadata");
    bytes[0] ^= 0xff;
    std::fs::write(&path, &bytes).expect("tamper with session metadata");

    let stderr = session_fail(&[
        "load",
        "--root",
        path_str(&root),
        "--session",
        session_key.as_str(),
    ]);
    assert!(
        stderr.contains("corrupt storage metadata"),
        "a corrupt record is a typed failure:\n{stderr}"
    );
    let stderr = session_fail(&[
        "list",
        "--root",
        path_str(&root),
        "--session",
        session_key.as_str(),
    ]);
    assert!(stderr.contains("corrupt storage metadata"), "{stderr}");

    // A list still succeeds and reports the corrupt entry as an anomaly on stderr
    // instead of printing it or silently skipping it.
    let output = session_raw(&["list", "--root", path_str(&root)]);
    let (stdout, stderr) = describe(&output);
    assert!(
        output.status.success(),
        "list reports anomalies without failing:\n{stdout}\n{stderr}"
    );
    assert!(
        stdout.is_empty(),
        "no committed entry remains to list: {stdout}"
    );
    assert!(stderr.contains("storage anomaly"), "{stderr}");
    assert!(stderr.contains("corrupt storage metadata"), "{stderr}");
}

#[test]
fn tampered_checkpoint_is_rejected_by_the_storage_binding() {
    let root = temp_dir("tampered-checkpoint");
    let (session, snapshot) = save_new_session(&root, "base", 4);
    let session_key = session_key(&session);
    let snapshot_key = snapshot_key(&snapshot);
    let path = checkpoint_path(&root, &session_key, &snapshot_key);
    let mut bytes = std::fs::read(&path).expect("read checkpoint");
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0xff;
    std::fs::write(&path, &bytes).expect("tamper with checkpoint");

    let stderr = session_fail(&[
        "load",
        "--root",
        path_str(&root),
        "--session",
        session_key.as_str(),
        "--snapshot",
        snapshot_key.as_str(),
    ]);
    assert!(
        stderr.contains("checkpoint SHA-256 does not match metadata"),
        "the metadata binding rejects a tampered checkpoint:\n{stderr}"
    );
    // Only the snapshot is damaged: the session record still lists, and the tampered
    // snapshot is reported as an anomaly instead of being printed as committed data.
    let output = session_raw(&[
        "list",
        "--root",
        path_str(&root),
        "--session",
        session_key.as_str(),
    ]);
    let (stdout, stderr) = describe(&output);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    let listed = parse_blocks(&stdout);
    assert_eq!(listed.len(), 1, "only the session block is committed data");
    assert_eq!(field(&listed[0], "record"), "session");
    assert!(stderr.contains("storage anomaly"), "{stderr}");
}

#[test]
fn incompatible_snapshot_backend_exits_one_with_typed_diagnostic() {
    let root = temp_dir("incompatible-backend");
    let (session_key, snapshot_key) = write_foreign_snapshot(&root);
    let before = tree_fingerprint(&root);
    // Both rehydration paths go through the single backend compatibility predicate.
    for subcommand in ["load", "save"] {
        let stderr = session_fail(&[
            subcommand,
            "--root",
            path_str(&root),
            "--session",
            session_key.as_str(),
        ]);
        assert!(
            stderr.contains("incompatible snapshot"),
            "`session {subcommand}` must report the typed incompatibility:\n{stderr}"
        );
        assert!(stderr.contains("backend_id"), "{stderr}");
    }
    assert_eq!(
        tree_fingerprint(&root),
        before,
        "a rejected rehydration must not write storage"
    );
    // Listing is a storage read: the foreign snapshot is committed data and lists
    // without asserting backend compatibility, which belongs to the session restore
    // path.
    let listed = parse_blocks(&session_ok(&[
        "list",
        "--root",
        path_str(&root),
        "--session",
        session_key.as_str(),
    ]));
    assert_eq!(listed.len(), 2);
    assert_eq!(field(&listed[1], "snapshot_key"), snapshot_key.as_str());
}

#[test]
fn unsupported_layout_version_exits_one_with_typed_diagnostic() {
    let root = temp_dir("layout-version");
    session_ok(&["save", "--root", path_str(&root), "--steps", "1"]);
    let marker = root.join("chronofork.store");
    let mut bytes = std::fs::read(&marker).expect("read storage marker");
    assert_eq!(
        &bytes[..8],
        b"CFSTORE1",
        "marker magic is the documented one"
    );
    bytes[8..10].copy_from_slice(&2_u16.to_le_bytes());
    std::fs::write(&marker, &bytes).expect("tamper with layout version");

    for subcommand in ["load", "list", "save"] {
        let stderr = session_fail(&[subcommand, "--root", path_str(&root), "--session", "1"]);
        assert!(
            stderr.contains("unsupported storage layout version 2, expected 1"),
            "`session {subcommand}` rejects an incompatible layout:\n{stderr}"
        );
    }
}

#[test]
fn foreign_store_marker_is_rejected() {
    let root = temp_dir("foreign-marker");
    std::fs::write(root.join("chronofork.store"), b"NOTSTORE\x01\x00")
        .expect("write foreign marker");
    let stderr = session_fail(&["load", "--root", path_str(&root), "--session", "1"]);
    assert!(
        stderr.contains("not a ChronoFork storage root"),
        "a foreign marker is a typed failure:\n{stderr}"
    );
}
