//! `chronofork tas edit`: strict, atomic rerecord and branch editing.
//!
//! This module is the process boundary of
//! [ADR-0008](../../../docs/adr/0008-rerecord-and-branch-editing.md) §7. It parses
//! the exact `tas edit` flags, guards the destination against identifying the
//! source file, reads the source read-only through the shared bounded reader,
//! delegates the functional edit (verified prefix copy plus deterministic suffix
//! re-simulation) to [`chronofork_sessions::edit_movie`], publishes the new movie
//! through the shared sibling-temporary-file + rename writer and prints one
//! stable `record=edit` block.
//!
//! The adapter never re-implements the movie codec, the edit algorithm, the
//! anchor rules or the publish helper. `--set-input` replaces only port 0 of the
//! recorded step and preserves ports 1-3, axes and keys; full-frame replacement is
//! library-only. The source movie and its file are never mutated: the output is a
//! new artifact whose parent is the source movie id, and an `--out` that resolves
//! to the source path or source inode is rejected before any temporary file is
//! created.

use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::path::{Component, Path, PathBuf};

use chronofork_core::{InputFrame, hash_bytes};
use chronofork_sessions::{
    AnchorPolicy, EditMeta, EditOutcome, MAX_NAME_BYTES, Movie, MovieEdit, edit_movie,
};

use crate::tas::{help_requested, read_movie_file, write_movie_file};
use crate::{
    CliError, VERSION, begin_record, option_value, parse_u64, set_once, synthetic_session_manager,
};

/// One explicit `--anchors` choice, before an interval is resolved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AnchorChoice {
    /// Retain source anchors before the earliest affected step (the default).
    KeepPrefix,
    /// Drop every source anchor.
    Drop,
    /// Replace every source anchor with a new table at `--anchor-interval`.
    Rebuild,
}

/// Parsed `tas edit` flags, before required-flag and combination validation.
#[derive(Debug, Default)]
struct EditOptions {
    movie: Option<PathBuf>,
    out: Option<PathBuf>,
    set_input: Vec<(u64, u32)>,
    truncate_after: Option<u64>,
    movie_id: Option<u64>,
    name: Option<String>,
    anchors: Option<AnchorChoice>,
    anchor_interval: Option<u64>,
}

/// Runs `chronofork tas edit`.
///
/// Parsing, source protection and request construction all complete before the
/// edit executes, and the output block is printed only after the new movie has
/// been published, so a typed failure never prints stdout data or leaves a
/// partial movie behind.
pub(crate) fn run(args: &[String]) -> Result<(), Box<dyn Error>> {
    if help_requested(args, "tas edit")? {
        print_help();
        return Ok(());
    }
    let options = parse_options(args)?;
    let movie_path = options
        .movie
        .as_deref()
        .ok_or_else(|| CliError("`tas edit` requires `--movie <FILE>`".to_owned()))?;
    let out_path = options
        .out
        .as_deref()
        .ok_or_else(|| CliError("`tas edit` requires `--out <FILE>`".to_owned()))?;
    let anchor_policy = resolve_anchor_policy(&options)?;
    let source_bytes = read_movie_file(movie_path)?;
    ensure_distinct_output(movie_path, out_path)?;
    let source = Movie::from_bytes(&source_bytes)?;
    let edit = build_edit(&source, &options)?;

    let mut meta = EditMeta::new();
    if let Some(movie_id) = options.movie_id {
        meta = meta.with_movie_id(movie_id);
    }
    if let Some(name) = &options.name {
        meta = meta.with_name(name.clone());
    }
    if let Some(policy) = anchor_policy {
        meta = meta.with_anchor_policy(policy);
    }

    let mut manager = synthetic_session_manager();
    let outcome = edit_movie(&source, &mut manager, &edit, &meta)?;
    let bytes = outcome.movie().to_bytes()?;
    write_movie_file(out_path, &bytes)?;

    let mut output = String::new();
    write_edit_block(&mut output, &outcome, &bytes, &source_bytes);
    print!("{output}");
    Ok(())
}

/// Parses the exact `tas edit` flags.
///
/// Positional arguments, foreign flags and repeated single-valued flags are
/// rejected; `--set-input` is the only repeatable flag and its values are parsed
/// as `<INDEX=MASK>` decimal pairs. An empty value is rejected for every flag.
fn parse_options(args: &[String]) -> Result<EditOptions, CliError> {
    let mut parsed = EditOptions::default();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].clone();
        match flag.as_str() {
            "--movie" => set_once(
                &mut parsed.movie,
                PathBuf::from(non_empty_value(args, &mut index, &flag)?),
                &flag,
            )?,
            "--out" => set_once(
                &mut parsed.out,
                PathBuf::from(non_empty_value(args, &mut index, &flag)?),
                &flag,
            )?,
            "--set-input" => {
                parsed
                    .set_input
                    .push(parse_set_input(non_empty_value(args, &mut index, &flag)?)?);
            }
            "--truncate-after" => set_once(
                &mut parsed.truncate_after,
                parse_u64(non_empty_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--movie-id" => set_once(
                &mut parsed.movie_id,
                parse_u64(non_empty_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            "--name" => set_once(
                &mut parsed.name,
                non_empty_value(args, &mut index, &flag)?.to_owned(),
                &flag,
            )?,
            "--anchors" => set_once(
                &mut parsed.anchors,
                parse_anchor_choice(non_empty_value(args, &mut index, &flag)?)?,
                &flag,
            )?,
            "--anchor-interval" => set_once(
                &mut parsed.anchor_interval,
                parse_u64(non_empty_value(args, &mut index, &flag)?, &flag)?,
                &flag,
            )?,
            other => {
                return Err(CliError(format!(
                    "unknown `tas edit` option `{other}`; expected --movie <FILE> --out <FILE> \
                     --set-input <INDEX=MASK> --truncate-after <INDEX> --movie-id <U64> \
                     --name <NAME> --anchors <keep-prefix|drop|rebuild> --anchor-interval <N>"
                )));
            }
        }
        index += 1;
    }
    Ok(parsed)
}

/// Requires a non-empty value for a single-valued flag.
fn non_empty_value<'a>(
    args: &'a [String],
    index: &mut usize,
    flag: &str,
) -> Result<&'a str, CliError> {
    let value = option_value(args, index, flag)?;
    if value.is_empty() {
        return Err(CliError(format!("`{flag}` requires a non-empty value")));
    }
    Ok(value)
}

/// Parses one repeatable `--set-input <INDEX=MASK>` value.
///
/// The index is a zero-based decimal `u64` and the mask a decimal `u32`; the
/// index is range-checked against the decoded source before it is used.
fn parse_set_input(value: &str) -> Result<(u64, u32), CliError> {
    let Some((index, mask)) = value.split_once('=') else {
        return Err(CliError(format!(
            "`--set-input` expects `<INDEX=MASK>`, got `{value}`"
        )));
    };
    let index: u64 = index.parse().map_err(|_| {
        CliError(format!(
            "`--set-input` expects a zero-based decimal step index and a decimal u32 mask as \
             `<INDEX=MASK>`, got `{value}`"
        ))
    })?;
    let mask: u32 = mask.parse().map_err(|_| {
        CliError(format!(
            "`--set-input` expects a zero-based decimal step index and a decimal u32 mask as \
             `<INDEX=MASK>`, got `{value}`"
        ))
    })?;
    Ok((index, mask))
}

/// Parses one `--anchors <keep-prefix|drop|rebuild>` value.
fn parse_anchor_choice(value: &str) -> Result<AnchorChoice, CliError> {
    match value {
        "keep-prefix" => Ok(AnchorChoice::KeepPrefix),
        "drop" => Ok(AnchorChoice::Drop),
        "rebuild" => Ok(AnchorChoice::Rebuild),
        other => Err(CliError(format!(
            "`--anchors` expects `keep-prefix`, `drop` or `rebuild`, got `{other}`"
        ))),
    }
}

/// Resolves the optional explicit anchor policy and its interval.
///
/// Without any anchor flag the library applies its default `keep-prefix` rule and
/// a v1 source remains valid; any explicit flag is passed through so the library
/// rejects it on a v1 source. `--anchor-interval` is valid only with `--anchors
/// rebuild`, and the interval is converted to `usize` before the edit runs.
fn resolve_anchor_policy(options: &EditOptions) -> Result<Option<AnchorPolicy>, CliError> {
    let no_interval =
        || CliError("`--anchor-interval` is valid only with `--anchors rebuild`".to_owned());
    match (options.anchors, options.anchor_interval) {
        (None, None) => Ok(None),
        (Some(AnchorChoice::Rebuild), None) => Err(CliError(
            "`--anchors rebuild` requires `--anchor-interval <N>`".to_owned(),
        )),
        (Some(AnchorChoice::KeepPrefix), None) => Ok(Some(AnchorPolicy::KeepPrefix)),
        (Some(AnchorChoice::Drop), None) => Ok(Some(AnchorPolicy::Drop)),
        (Some(AnchorChoice::Rebuild), Some(interval)) => {
            let interval = usize::try_from(interval).map_err(|_| {
                CliError(format!(
                    "`--anchor-interval` is too large for this platform: {interval}"
                ))
            })?;
            Ok(Some(AnchorPolicy::Rebuild { interval }))
        }
        (Some(AnchorChoice::KeepPrefix | AnchorChoice::Drop), Some(_)) | (None, Some(_)) => {
            Err(no_interval())
        }
    }
}

/// Builds the library edit request from the parsed flags.
///
/// Each edited step replaces only port 0 of its recorded [`InputFrame`], so ports
/// 1-3, axes and keys are preserved exactly as recorded. An out-of-range index is
/// rejected here because the recorded frame must be read before the request is
/// built; duplicate indices and truncation rules stay owned by the library.
fn build_edit(source: &Movie, options: &EditOptions) -> Result<MovieEdit, Box<dyn Error>> {
    let step_count = source.step_count();
    let mut edit = MovieEdit::new();
    for &(index, mask) in &options.set_input {
        let recorded = usize::try_from(index)
            .ok()
            .and_then(|index| source.steps().get(index))
            .ok_or_else(|| {
                CliError(format!(
                    "`--set-input` step index {index} is out of range for a movie with \
                     {step_count} step(s)"
                ))
            })?;
        edit = edit
            .with_input(index, replace_port_zero(recorded.input(), mask))
            .map_err(|error| CliError(error.to_string()))?;
    }
    if let Some(index) = options.truncate_after {
        edit = edit.with_truncate_after(index);
    }
    Ok(edit)
}

/// Replaces only port 0 of a recorded frame, preserving every other device field.
fn replace_port_zero(recorded: InputFrame, mask: u32) -> InputFrame {
    let mut ports = recorded.ports();
    ports[0] = mask;
    InputFrame::with_devices(ports, recorded.axes(), recorded.keys())
}

/// Rejects an `--out` that can identify the source file.
///
/// The two paths are compared after becoming absolute and being lexically
/// normalized (`--out` is never resolved through symlinks). On Unix an existing
/// destination that shares the source's device and inode (a hard link or a
/// symlink-resolved alias) is rejected too, so the source inode can never be
/// unlinked or rewritten. Both checks run before the edit executes and before any
/// temporary file is created.
fn ensure_distinct_output(source: &Path, out: &Path) -> Result<(), CliError> {
    if out.file_name().is_none() {
        return Err(CliError(format!(
            "`--out` must name a file, not a directory: `{}`",
            out.display()
        )));
    }
    if lexical_absolute(source)? == lexical_absolute(out)? {
        return Err(aliases_source(source, out));
    }
    let Ok(out_metadata) = fs::metadata(out) else {
        return Ok(());
    };
    if out_metadata.is_dir() {
        return Err(CliError(format!(
            "`--out` must name a file, not a directory: `{}`",
            out.display()
        )));
    }
    let source_metadata = fs::metadata(source).map_err(|error| {
        CliError(format!(
            "failed to inspect movie `{}`: {error}",
            source.display()
        ))
    })?;
    if same_file(&source_metadata, &out_metadata) {
        return Err(aliases_source(source, out));
    }
    Ok(())
}

/// Builds the stable `OutputAliasesSource` diagnostic.
fn aliases_source(source: &Path, out: &Path) -> CliError {
    CliError(format!(
        "OutputAliasesSource: `--out` `{}` identifies the source movie `{}`; the source is \
         never overwritten",
        out.display(),
        source.display()
    ))
}

/// Returns an absolute path with `.`/`..` components removed lexically.
///
/// The path is not resolved through the filesystem, so a symbolic link is not
/// followed and a lexically equal `--out` is rejected even before it exists.
fn lexical_absolute(path: &Path) -> Result<PathBuf, CliError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| CliError(format!("failed to resolve the current directory: {error}")))?
            .join(path)
    };
    Ok(lexically_normalize(&absolute))
}

/// Removes `.` and resolves `..` without touching the filesystem.
fn lexically_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if normalized.file_name().is_some() {
                    normalized.pop();
                } else {
                    normalized.push(component.as_os_str());
                }
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

/// Returns whether two existing files are the same filesystem object.
#[cfg(unix)]
fn same_file(source: &fs::Metadata, out: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    source.dev() == out.dev() && source.ino() == out.ino()
}

/// Platforms without an inode identity check fall back to the lexical rule.
#[cfg(not(unix))]
fn same_file(_source: &fs::Metadata, _out: &fs::Metadata) -> bool {
    false
}

/// Writes the fixed `record=edit` block in the ADR-0008 field order.
fn write_edit_block(output: &mut String, outcome: &EditOutcome, bytes: &[u8], source_bytes: &[u8]) {
    let movie = outcome.movie();
    let report = outcome.report();
    begin_record(output, "edit");
    let _ = writeln!(output, "movie_version={}", movie.format_version());
    let _ = writeln!(output, "input_schema={}", movie.input_schema_version());
    let _ = writeln!(output, "source_movie_id={}", report.source_movie_id);
    let _ = writeln!(output, "movie_id={}", movie.meta().movie_id());
    let _ = writeln!(output, "parent_movie_id={}", report.parent_movie_id);
    let _ = writeln!(output, "name={}", movie.meta().name().unwrap_or(""));
    let _ = writeln!(output, "start_tick={}", movie.start_tick().raw());
    let _ = writeln!(output, "end_tick={}", movie.end_tick().raw());
    let _ = writeln!(output, "steps={}", movie.step_count());
    let _ = writeln!(output, "edits={}", report.edits);
    let _ = writeln!(
        output,
        "truncate_after={}",
        report
            .truncate_after
            .map_or_else(String::new, |index| index.to_string())
    );
    let _ = writeln!(output, "prefix_steps={}", report.prefix_steps);
    let _ = writeln!(output, "resimulated_steps={}", report.resimulated_steps);
    let _ = writeln!(output, "prefix_verified=1");
    let _ = writeln!(
        output,
        "anchor_policy={}",
        anchor_policy_name(report.anchor_policy)
    );
    let _ = writeln!(output, "anchors_retained={}", report.anchors_retained);
    let _ = writeln!(output, "anchors_dropped={}", report.anchors_dropped);
    let _ = writeln!(output, "anchors_rebuilt={}", report.anchors_rebuilt);
    let _ = writeln!(output, "final_state_hash={}", movie.final_state_hash());
    let _ = writeln!(output, "movie_sha256={}", hash_bytes(bytes));
    let _ = writeln!(output, "source_movie_sha256={}", hash_bytes(source_bytes));
}

/// Returns the CLI spelling of an anchor policy.
const fn anchor_policy_name(policy: AnchorPolicy) -> &'static str {
    match policy {
        AnchorPolicy::KeepPrefix => "keep-prefix",
        AnchorPolicy::Drop => "drop",
        AnchorPolicy::Rebuild { .. } => "rebuild",
    }
}

/// Prints the `tas edit` help and honesty text.
pub(crate) fn print_help() {
    println!("chronofork {VERSION} — tas edit");
    println!();
    println!("Creates a new branch movie from an existing recorded movie by functional");
    println!("input editing and optional truncation. Editing is a deterministic suffix");
    println!("re-simulation from the source's embedded checkpoint, not an in-place edit and");
    println!("not an O(1) operation: the unchanged prefix is executed and verified against");
    println!("its recorded expectations, every edited and later step is re-simulated, and");
    println!("every later expectation is recomputed. Editing a late step costs almost a");
    println!("full replay.");
    println!();
    println!("The source movie is opened read-only and never modified: the output is a new");
    println!("artifact whose `parent_movie_id` is the source `movie_id`. `--out` may not");
    println!("identify the source by normalized path or, on Unix, by device/inode (hard link");
    println!("or symlink alias). The new movie is written to a sibling temporary file and");
    println!("renamed on success, so a failed edit leaves no partial output.");
    println!();
    println!("Format v1 stays v1, format v2 stays v2; there is no migration or upgrade. This");
    println!("CLI edits only port 0 of a step and preserves ports 1-3, axes and keys as");
    println!("recorded; full-frame replacement is library-only. A v1 source cannot carry an");
    println!("anchor table, so any explicit `--anchors` flag is rejected for v1.");
    println!();
    println!("`--anchors` defaults to `keep-prefix`: source anchors before the earliest");
    println!("edited or truncated step are retained verbatim, later anchors are dropped.");
    println!("`drop` removes every anchor and `rebuild` captures a new table every");
    println!("`--anchor-interval` output steps. The embedded checkpoint is always copied");
    println!("unchanged and is never an anchor. No rerecord counter, edit history or new");
    println!("movie format is stored: `edits`, `truncate_after`, `prefix_steps` and");
    println!("`resimulated_steps` describe this invocation only.");
    println!();
    println!("This is synthetic-backend scope; the Mega Drive slice ignores input. Deferred:");
    println!("real-hardware input TAS, in-place editing, timeline insert/delete/reorder,");
    println!("scripted input files, persisted rerecord counters, import/export adapters");
    println!("(such as BizHawk BK2), mixing retained and rebuilt anchors, CLI editing of");
    println!("ports 1-3/axes/keys, and HTTP/MCP/GUI/daemon. This is one increment of M3,");
    println!("which is not complete.");
    println!();
    println!("Usage:");
    println!("  chronofork tas edit --movie <FILE> --out <FILE>");
    println!("                      --set-input <INDEX=MASK>...   or --truncate-after <INDEX>");
    println!("                      [--movie-id <U64>] [--name <NAME>]");
    println!("                      [--anchors <keep-prefix|drop|rebuild>]");
    println!("                      [--anchor-interval <N>]");
    println!("  chronofork tas edit --help");
    println!();
    println!("  --movie <FILE>     Required source movie, opened read-only and never modified");
    println!("  --out <FILE>       Required new movie file, published atomically on success;");
    println!("                     it must not identify the source file");
    println!("  --set-input <INDEX=MASK>  Repeatable. INDEX is a zero-based decimal step index");
    println!("                     below the source step count; MASK is a decimal u32 that");
    println!("                     replaces only port 0 of that step. Duplicate indices are");
    println!("                     rejected. At least one --set-input or --truncate-after is");
    println!("                     required; a metadata-only copy is rejected");
    println!("  --truncate-after <INDEX>  Zero-based decimal index; retain steps 0..=INDEX and");
    println!("                     drop every later step. At least one step must be dropped,");
    println!("                     and every edited index must be <= INDEX");
    println!("  --movie-id <U64>   Non-zero output movie id (default: source id + 1); it must");
    println!("                     differ from the source id");
    println!("  --name <NAME>      Optional output label, at most {MAX_NAME_BYTES} UTF-8 bytes,");
    println!("                     non-empty and free of control characters; never inherited");
    println!("                     Command-line arguments must be valid UTF-8: a non-UTF-8");
    println!("                     value is a typed rejection before any file is read or written");
    println!("  --anchors <keep-prefix|drop|rebuild>  Anchor policy (default keep-prefix);");
    println!("                     explicit flags are rejected on a format v1 source");
    println!("  --anchor-interval <N>  Positive decimal anchor interval, valid only with");
    println!("                     `--anchors rebuild`; 1 <= N <= output step count and the");
    println!("                     predicted anchor count must stay within the format limit");
    println!();
    println!("A successful edit prints exactly one block, in this order, and exits 0:");
    println!("  record=edit");
    println!("  movie_version input_schema source_movie_id movie_id parent_movie_id name");
    println!("  start_tick end_tick steps edits truncate_after prefix_steps resimulated_steps");
    println!("  prefix_verified=1 anchor_policy anchors_retained anchors_dropped");
    println!("  anchors_rebuilt final_state_hash movie_sha256 source_movie_sha256");
    println!("`truncate_after` is empty when the request does not truncate; `prefix_verified=1`");
    println!("means every copied prefix step matched its source record, never that a dropped");
    println!("suffix ran. On any validation, decode, execution, anchor, I/O or write failure");
    println!("nothing is printed on stdout, `chronofork: <error>` goes to stderr and the exit");
    println!("code is 1.");
}
