//! `chronofork gui`: the Stage 7 human-facing presentation surface.
//!
//! This module is the composition root of
//! [ADR-0013 §3.9](../../../docs/adr/0013-human-facing-gui.md#39-the-chronofork-gui-run-process-surface-normative)
//! and [architecture §6](../../../docs/architecture/gui.md#6-cli-composition). It owns
//! exactly the process-level responsibilities: flag parsing, the token file, the
//! principal, the read-only movie file, stdin/stdout/stderr, the `--html` write, the
//! `key=value` formatting of a block and the exit code.
//!
//! It implements **no** grammar, view, pane, panel, pacer or lease logic. Every line is
//! parsed and executed by [`CommandSession::execute`], every block is built by
//! `chronofork_gui::command`, and every canonical byte is produced by the M6 service
//! codec. The two [`OperationChannel`] implementations below are the only place this
//! surface speaks HTTP (remote mode) or dispatches into an in-process service (local
//! mode): the grammar never encodes a request and never reads a response.
//!
//! Output policy (ADR-0013 §3.9 items 1–5):
//!
//! - stdout carries data only — blocks, frame grids and the HTML document — and blocks
//!   are separated by exactly one blank line with no trailing blank line;
//! - stderr carries exactly one diagnostic line per failure: the canonical
//!   `chronofork.error.v1` envelope bytes for an operation failure, otherwise
//!   `chronofork: gui: <code>: <detail>`;
//! - the exit code is 0 for `quit` or a clean end of input and 1 for any failure, and a
//!   failing step emits nothing on stdout (fail-closed, the same rule as `tas studio`);
//! - neither a token nor a host path reaches stdout, stderr, a block, a frame or the HTML
//!   document.

use std::error::Error;
use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chronofork_backend_api::BackendFactory;
use chronofork_gui::command::{
    Block, BlockField, CommandConfig, CommandOutcome, CommandSession, QuitReason, Record,
};
use chronofork_gui::pacing::{ManualClock, PacerConfig};
use chronofork_gui::{
    ChannelFault, DEFAULT_FPS, DEFAULT_HEIGHT, DEFAULT_MAX_CATCH_UP_FRAMES,
    DEFAULT_TICKS_PER_FRAME, DEFAULT_WIDTH, GuiError, MAX_FPS, MAX_HEIGHT, MAX_WIDTH, MIN_FPS,
    MIN_HEIGHT, MIN_WIDTH, OperationChannel, RenderOptions, TimelineAnchor, TimelineView,
};
use chronofork_megadrive::{MegadriveFactory, ROM_END};
use chronofork_service::{
    AppService, Principal, ServiceConfig, ServiceLimits, TokenTable, parse_token_file,
};
use chronofork_sessions::{MAX_MOVIE_BYTES, Movie};
use chronofork_test_backend::SyntheticBackendFactory;
use chronofork_transport::{HttpClient, TransportError};

use crate::VERSION;
use crate::agent::{FailureReported, local_admin};

/// Runs one `chronofork gui` subcommand.
///
/// `chronofork gui --help` and `chronofork gui run --help` print usage to stdout and exit
/// 0; every other outcome is `run` or a typed failure with one stderr line and empty
/// stdout.
pub(crate) fn run(args: &[String]) -> Result<(), Box<dyn Error>> {
    match execute(args) {
        Ok(()) => Ok(()),
        Err(error) => fail(error),
    }
}

/// Dispatches one `chronofork gui` invocation, returning the first typed failure.
///
/// The help forms write through [`print_help`], which is a checked stdout write, so a
/// closed or unwritable stdout is a typed `channel` failure and never the standard
/// library's print panic.
fn execute(args: &[String]) -> Result<(), GuiError> {
    match args.first().map(String::as_str) {
        Some("--help" | "-h") => {
            if args.len() != 1 {
                return Err(GuiError::bad_value(
                    "gui --help",
                    "takes no other arguments",
                ));
            }
            print_help()
        }
        Some("run") => open_window(&args[1..]),
        None => Err(GuiError::bad_value("gui", "a subcommand is required: run")),
        Some(other) => Err(GuiError::unknown_command(&format!("gui {other}"))),
    }
}

/// Builds the window and drives the line loop, returning the first typed failure.
///
/// The startup order is the documented one (`gui.md` §6.2): flags, then the remote token
/// file and client or the local backend factory and in-process service, then the
/// read-only movie, then [`CommandSession::open`] (which validates the frame bounds and
/// issues `capabilities` once), then stdin.
fn open_window(args: &[String]) -> Result<(), GuiError> {
    if args
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        if args.len() != 1 {
            return Err(GuiError::bad_value(
                "gui run --help",
                "takes no other arguments",
            ));
        }
        return print_help();
    }
    let options = GuiOptions::parse(args)?;
    let channel = build_channel(&options)?;
    let timeline = match options.movie.as_deref() {
        Some(path) => Some(load_timeline(path)?),
        None => None,
    };
    let config = CommandConfig {
        width: options.width,
        height: options.height,
        render: RenderOptions::with_ansi(options.ansi),
        pacer: PacerConfig::new(
            options.fps,
            DEFAULT_TICKS_PER_FRAME,
            DEFAULT_MAX_CATCH_UP_FRAMES,
        )?,
        timeline,
    };
    let mut session = CommandSession::open(channel, Arc::new(ManualClock::new()), config)?;
    drive(&mut session, &options)
}

/// One set of parsed `gui run` options.
///
/// Every option is parsed at most once and validated here or by [`CommandSession::open`];
/// no file, token or environment value is read while parsing.
#[derive(Clone, Debug, Eq, PartialEq)]
struct GuiOptions {
    /// `--remote <URL>`: remote mode base URL.
    remote: Option<String>,
    /// `--token-file <FILE>`: the bearer-token file of remote mode.
    token_file: Option<PathBuf>,
    /// `--rom <FILE>`: local-mode Mega Drive backend selection.
    rom: Option<PathBuf>,
    /// `--width <N>`: frame width in cells.
    width: u32,
    /// `--height <N>`: frame height in lines.
    height: u32,
    /// `--ansi`: emit SGR emphasis in frames.
    ansi: bool,
    /// `--html <OUT>`: write the HTML document instead of printing it.
    html_out: Option<PathBuf>,
    /// `--movie <FILE>`: read-only `.cfmovie` for the timeline panel.
    movie: Option<PathBuf>,
    /// `--fps <N>`: presentation cadence.
    fps: u32,
}

impl GuiOptions {
    /// Parses and validates the documented `gui run` options.
    ///
    /// An unknown, repeated or malformed option is a typed `bad_value`; the mutually
    /// exclusive and required combinations are checked after the loop, so the diagnostic
    /// names the option that has to change.
    fn parse(args: &[String]) -> Result<Self, GuiError> {
        let mut remote: Option<String> = None;
        let mut token_file: Option<PathBuf> = None;
        let mut rom: Option<PathBuf> = None;
        let mut width: Option<u32> = None;
        let mut height: Option<u32> = None;
        let mut ansi = false;
        let mut html_out: Option<PathBuf> = None;
        let mut movie: Option<PathBuf> = None;
        let mut fps: Option<u32> = None;

        let mut index = 0;
        while index < args.len() {
            let flag = args[index].clone();
            match flag.as_str() {
                "--remote" => set_once(
                    &mut remote,
                    option_value(args, &mut index, "--remote")?.to_owned(),
                    "--remote",
                )?,
                "--token-file" => set_once(
                    &mut token_file,
                    PathBuf::from(option_value(args, &mut index, "--token-file")?),
                    "--token-file",
                )?,
                "--rom" => set_once(
                    &mut rom,
                    PathBuf::from(option_value(args, &mut index, "--rom")?),
                    "--rom",
                )?,
                "--width" => set_once(
                    &mut width,
                    parse_u32(option_value(args, &mut index, "--width")?, "--width")?,
                    "--width",
                )?,
                "--height" => set_once(
                    &mut height,
                    parse_u32(option_value(args, &mut index, "--height")?, "--height")?,
                    "--height",
                )?,
                "--fps" => set_once(
                    &mut fps,
                    parse_u32(option_value(args, &mut index, "--fps")?, "--fps")?,
                    "--fps",
                )?,
                "--html" => set_once(
                    &mut html_out,
                    PathBuf::from(option_value(args, &mut index, "--html")?),
                    "--html",
                )?,
                "--movie" => set_once(
                    &mut movie,
                    PathBuf::from(option_value(args, &mut index, "--movie")?),
                    "--movie",
                )?,
                "--ansi" => {
                    if ansi {
                        return Err(GuiError::bad_value("--ansi", "was given more than once"));
                    }
                    ansi = true;
                }
                other => {
                    return Err(GuiError::bad_value(
                        "gui run option",
                        &format!("`{other}` is not a documented option"),
                    ));
                }
            }
            index += 1;
        }

        if remote.is_some() && rom.is_some() {
            return Err(GuiError::bad_value(
                "--rom",
                "cannot be combined with `--remote`",
            ));
        }
        if remote.is_some() && token_file.is_none() {
            return Err(GuiError::bad_value(
                "--token-file",
                "is required with `--remote`",
            ));
        }
        if remote.is_none() && token_file.is_some() {
            return Err(GuiError::bad_value(
                "--token-file",
                "requires `--remote`; local mode reads no token file",
            ));
        }

        Ok(Self {
            remote,
            token_file,
            rom,
            width: width.unwrap_or(DEFAULT_WIDTH),
            height: height.unwrap_or(DEFAULT_HEIGHT),
            ansi,
            html_out,
            movie,
            fps: fps.unwrap_or(DEFAULT_FPS),
        })
    }
}

/// Whose `--remote` was given, returns its URL; otherwise it errors.
fn set_once<T>(slot: &mut Option<T>, value: T, flag: &'static str) -> Result<(), GuiError> {
    if slot.is_some() {
        return Err(GuiError::bad_value(flag, "was given more than once"));
    }
    *slot = Some(value);
    Ok(())
}

/// Returns the value of the option at `index` and advances past it.
fn option_value<'a>(
    args: &'a [String],
    index: &mut usize,
    flag: &'static str,
) -> Result<&'a str, GuiError> {
    *index += 1;
    args.get(*index)
        .map(String::as_str)
        .ok_or_else(|| GuiError::bad_value(flag, "requires a value"))
}

/// Parses one unsigned option value, reporting the flag that carried it.
fn parse_u32(value: &str, flag: &'static str) -> Result<u32, GuiError> {
    value.parse().map_err(|_| {
        GuiError::bad_value(flag, &format!("expects a decimal integer, got `{value}`"))
    })
}

/// Creates the one operation channel of this process.
///
/// Remote mode composes the M6 typed client over the explicit `--remote` URL and the
/// bearer token of the token file's first principal; local mode composes the backend
/// factory into an in-process [`AppService`] with the documented local administrator
/// principal and no store and no artifacts. No other module of the CLI dispatches an
/// operation for this surface.
fn build_channel(options: &GuiOptions) -> Result<Arc<dyn OperationChannel>, GuiError> {
    match (options.remote.as_deref(), options.token_file.as_deref()) {
        (Some(url), Some(path)) => {
            let table = read_token_table(path)?;
            let token = table
                .credentials()
                .next()
                .map(|(_, token)| token.to_owned())
                .ok_or_else(|| {
                    GuiError::bad_value("--token-file", "the token file has no principal")
                })?;
            let client = HttpClient::new(url, token);
            Ok(Arc::new(RemoteChannel { client }))
        }
        (Some(_), None) => Err(GuiError::bad_value(
            "--token-file",
            "is required with `--remote`",
        )),
        (None, _) => {
            let factory = backend_factory(options.rom.as_deref())?;
            let service = AppService::new(ServiceConfig {
                factory,
                storage_root: None,
                artifacts_root: None,
                limits: ServiceLimits::default(),
            })
            .map_err(|_| GuiError::channel("the in-process service could not be created"))?;
            Ok(Arc::new(LocalChannel {
                service,
                principal: local_admin(),
            }))
        }
    }
}

/// One operation channel over an in-process service under one principal.
///
/// The service still validates, authorises and limits every request: the local principal
/// is the documented local trust boundary, not a bypass of an operation rule.
struct LocalChannel {
    /// The in-process service.
    service: AppService,
    /// The principal the local process acts as.
    principal: Principal,
}

impl OperationChannel for LocalChannel {
    fn call(&self, request: &[u8]) -> Result<Vec<u8>, ChannelFault> {
        match self.service.dispatch(&self.principal, request) {
            Ok(envelope) => Ok(envelope),
            Err(error) => {
                let limits = ServiceLimits::default();
                match error.to_canonical_bytes(&limits) {
                    Ok(body) => Err(ChannelFault::with_envelope(&error.to_string(), body)),
                    Err(_) => Err(ChannelFault::new(
                        "the service failure could not be encoded canonically",
                    )),
                }
            }
        }
    }
}

/// One operation channel over the M6 typed HTTP client.
///
/// A non-2xx answer becomes the single stderr line of the process only when its body is
/// exactly the canonical `chronofork.error.v1` document; any other body (proxy HTML, a
/// multi-line text page, binary bytes) becomes a bounded local diagnostic that quotes
/// nothing, so a peer outside ChronoFork cannot break the one-line rule.
struct RemoteChannel {
    /// The typed client, already bound to the URL and the bearer token.
    client: HttpClient,
}

impl OperationChannel for RemoteChannel {
    fn call(&self, request: &[u8]) -> Result<Vec<u8>, ChannelFault> {
        self.client.call_raw(request).map_err(|error| match error {
            TransportError::Http { status, body } => match canonical_envelope(&body) {
                Some(envelope) => ChannelFault::with_envelope(
                    &format!("peer returned HTTP status {status}"),
                    envelope,
                ),
                // A proxy or a foreign server can answer a non-2xx status with HTML, a
                // multi-line page or binary bytes. Printing that verbatim would break the
                // one-stderr-line contract, so only the canonical document is forwarded and
                // everything else becomes a local diagnostic that quotes nothing.
                None => ChannelFault::new(&format!(
                    "peer returned HTTP status {status} without a canonical error envelope"
                )),
            },
            other => ChannelFault::new(&other.to_string()),
        })
    }
}

/// Returns the peer body when it is exactly the canonical `chronofork.error.v1` envelope.
///
/// The body is accepted only when the service codec parses it, it carries the documented
/// shape (`schema` plus `error` with `code`, `message` and `retryable`), and re-encoding the
/// parsed document reproduces the same bytes. Re-encoding is the canonicality test: any
/// extra field, reordered key, stray whitespace or trailing byte makes the round trip
/// differ, so a body that merely *looks* like JSON cannot reach stderr verbatim.
fn canonical_envelope(body: &[u8]) -> Option<Vec<u8>> {
    let limits = ServiceLimits::default();
    let document = chronofork_service::json::parse(body, &limits).ok()?;
    let root = document.as_object()?;
    if root.len() != 2 || root.get("schema")?.as_str()? != "chronofork.error.v1" {
        return None;
    }
    let error = root.get("error")?.as_object()?;
    if error.len() != 3 {
        return None;
    }
    error.get("code")?.as_str()?;
    error.get("message")?.as_str()?;
    error.get("retryable")?.as_bool()?;
    let canonical = chronofork_service::json::to_canonical_bytes(&document, &limits).ok()?;
    (canonical == body).then_some(canonical)
}

/// Resolves the local backend factory named by `--rom`.
fn backend_factory(rom: Option<&Path>) -> Result<Arc<dyn BackendFactory>, GuiError> {
    match rom {
        Some(path) => {
            let limit = usize::try_from(ROM_END).unwrap_or(usize::MAX);
            let bytes = read_bounded(path, limit)
                .map_err(|_| GuiError::bad_value("--rom", "the ROM file could not be read"))?;
            if bytes.len() > limit {
                return Err(GuiError::limit_exceeded(
                    "ROM bytes",
                    u64::try_from(limit).unwrap_or(u64::MAX),
                ));
            }
            let factory = MegadriveFactory::new(bytes)
                .map_err(|error| GuiError::bad_value("--rom", &error.to_string()))?;
            Ok(Arc::new(factory))
        }
        None => Ok(Arc::new(SyntheticBackendFactory::default())),
    }
}

/// Reads the bounded `chronofork.tokens.v1` table and never echoes a token.
fn read_token_table(path: &Path) -> Result<TokenTable, GuiError> {
    let limit = ServiceLimits::default().max_request_bytes;
    let bytes = read_bounded(path, limit)
        .map_err(|_| GuiError::bad_value("--token-file", "the token file could not be read"))?;
    if bytes.len() > limit {
        return Err(GuiError::limit_exceeded(
            "token file bytes",
            u64::try_from(limit).unwrap_or(u64::MAX),
        ));
    }
    parse_token_file(&bytes).map_err(|_| {
        GuiError::bad_value(
            "--token-file",
            "the token file is not a valid `chronofork.tokens.v1` table",
        )
    })
}

/// Reads at most `limit + 1` bytes, so an oversized input is rejected before buffering.
///
/// This helper intentionally does not reuse the bounded readers of `crate::agent`: those
/// are private to the Stage 6 module tree (which is the frozen M6 snapshot surface for this
/// milestone) and they report failures as `chronofork: <message>` with a host path, while
/// this surface must return a typed [`GuiError`] code per input (`bad_value`,
/// `limit_exceeded`, `movie`) with a path-free and token-free detail. Sharing the reader
/// would mean widening the Stage 6 visibility and threading a second error mapping through
/// it; the two call sites here are the token file and the movie, so the small duplication is
/// the deliberate choice over coupling a frozen module to the GUI's error contract.
fn read_bounded(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    let file = File::open(path)?;
    let mut buffer = Vec::new();
    file.take(u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1))
        .read_to_end(&mut buffer)?;
    Ok(buffer)
}

/// Projects one `.cfmovie` read read-only into the plain [`TimelineView`] of the panel.
///
/// The existing bounded loader owns decoding, checksum verification and validation; this
/// function only reads the file (never writes, moves or truncates it) and projects four
/// documented values. Every failure is a typed `movie` error, and no diagnostic carries a
/// host path or a movie byte.
fn load_timeline(path: &Path) -> Result<TimelineView, GuiError> {
    let bytes = read_bounded(path, MAX_MOVIE_BYTES)
        .map_err(|_| GuiError::movie("the movie file could not be read"))?;
    if bytes.len() > MAX_MOVIE_BYTES {
        return Err(GuiError::limit_exceeded(
            "movie bytes",
            u64::try_from(MAX_MOVIE_BYTES).unwrap_or(u64::MAX),
        ));
    }
    let movie = Movie::from_bytes(&bytes).map_err(|error| GuiError::movie(&error.to_string()))?;
    let anchors = movie
        .anchors()
        .iter()
        .map(|anchor| {
            TimelineAnchor::new(
                u64::try_from(anchor.step_index()).unwrap_or(u64::MAX),
                anchor.tick().raw(),
            )
        })
        .collect();
    TimelineView::new(
        u64::from(movie.format_version()),
        u64::try_from(movie.step_count()).unwrap_or(u64::MAX),
        movie.end_tick().raw(),
        anchors,
    )
}

/// Drives the documented line protocol until `quit` or end of input.
///
/// One line is read, executed and written at a time; a failure returns before the failing
/// step writes anything, so stdout only ever holds complete blocks, grids and documents.
fn drive(session: &mut CommandSession, options: &GuiOptions) -> Result<(), GuiError> {
    let stdin = io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let stdout = io::stdout();
    let mut output = stdout.lock();
    let mut wrote_any = false;
    loop {
        let Some(line) = read_line(&mut reader)? else {
            let block = session.quit(QuitReason::Eof);
            emit(&block_text(&block), &mut output, &mut wrote_any)?;
            return Ok(());
        };
        let Some(outcome) = session.execute(&line)? else {
            continue;
        };
        let quit = matches!(&outcome, CommandOutcome::Block(block) if block.record == Record::Quit);
        emit_outcome(&outcome, options, &mut output, &mut wrote_any)?;
        if quit {
            return Ok(());
        }
    }
}

/// Reads one bounded, UTF-8 command line; `Ok(None)` at end of input.
///
/// The bound is the service's canonical request limit: a line is the presentation input of
/// one operation, and a longer one could only ever be rejected downstream.
fn read_line<R: BufRead>(reader: &mut R) -> Result<Option<String>, GuiError> {
    let limit = ServiceLimits::default().max_request_bytes;
    let mut buffer = Vec::new();
    let read = reader
        .take(u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(2))
        .read_until(b'\n', &mut buffer)
        .map_err(|_| GuiError::bad_value("stdin", "a command line could not be read"))?;
    if read == 0 {
        return Ok(None);
    }
    let content = buffer.strip_suffix(b"\n").unwrap_or(buffer.as_slice());
    if content.len() > limit {
        return Err(GuiError::limit_exceeded(
            "command line bytes",
            u64::try_from(limit).unwrap_or(u64::MAX),
        ));
    }
    let text = std::str::from_utf8(&buffer)
        .map_err(|_| GuiError::bad_value("stdin", "a command line is not valid UTF-8"))?;
    Ok(Some(text.to_owned()))
}

/// Writes the outcome of one executed line.
///
/// A block is formatted in the crate's documented field order; a grid is written verbatim
/// (its trailing newline is part of it); the HTML document is written to `--html` and
/// described by its block, or printed as the document when `--html` is absent.
fn emit_outcome(
    outcome: &CommandOutcome,
    options: &GuiOptions,
    output: &mut impl Write,
    wrote_any: &mut bool,
) -> Result<(), GuiError> {
    match outcome {
        CommandOutcome::Block(block) => emit(&block_text(block), output, wrote_any),
        CommandOutcome::Grid(grid) => emit(grid, output, wrote_any),
        CommandOutcome::Html { document, block } => match options.html_out.as_deref() {
            Some(path) => {
                std::fs::write(path, document.as_bytes()).map_err(|_| {
                    GuiError::bad_value("--html", "the HTML document could not be written")
                })?;
                emit(&block_text(block), output, wrote_any)
            }
            None => emit(document, output, wrote_any),
        },
    }
}

/// Writes one chunk, separating it from a previous block by exactly one blank line.
fn emit(chunk: &str, output: &mut impl Write, wrote_any: &mut bool) -> Result<(), GuiError> {
    if *wrote_any {
        output.write_all(b"\n").map_err(output_failure)?;
    }
    output.write_all(chunk.as_bytes()).map_err(output_failure)?;
    output.flush().map_err(output_failure)?;
    *wrote_any = true;
    Ok(())
}

/// Maps an output-stream failure onto the closed protocol codes.
///
/// A stream that can no longer be written is the presentation channel to the terminal
/// being gone. The closed set of ADR-0013 §3.9 has no I/O code and none may be invented,
/// so the failure is reported under `channel`, with a diagnostic that carries no path.
fn output_failure(_: io::Error) -> GuiError {
    GuiError::channel("the output stream could not be written")
}

/// Formats one block: `record=<name>` then one `key=value` line per field, in order.
///
/// The field values and names are already restricted to printable ASCII without spaces by
/// `chronofork_gui::command`, so a block can never break the line protocol.
fn block_text(block: &Block) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "record={}", block.record.as_str());
    for field in &block.fields {
        match field {
            BlockField::Uint(name, value) => {
                let _ = writeln!(text, "{name}={value}");
            }
            BlockField::Bool(name, value) => {
                let _ = writeln!(text, "{name}={value}");
            }
            BlockField::Text(name, value) => {
                let _ = writeln!(text, "{name}={value}");
            }
            BlockField::OptionalUint(name, Some(value)) => {
                let _ = writeln!(text, "{name}={value}");
            }
            BlockField::OptionalUint(name, None) => {
                let _ = writeln!(text, "{name}=none");
            }
        }
    }
    text
}

/// Writes the one stderr line of a failure and returns its failure marker.
fn fail(error: GuiError) -> Result<(), Box<dyn Error>> {
    report(&error);
    Err(Box::new(FailureReported))
}

/// Writes the single stderr diagnostic of one failure.
///
/// An operation failure prints the canonical `chronofork.error.v1` bytes verbatim;
/// everything else prints the documented `chronofork: gui: <code>: <detail>` line.
fn report(error: &GuiError) {
    let stderr = io::stderr();
    let mut output = stderr.lock();
    let written = match error.envelope() {
        Some(bytes) => output
            .write_all(bytes)
            .and_then(|()| output.write_all(b"\n")),
        None => writeln!(output, "chronofork: gui: {error}"),
    };
    let _ = written;
    let _ = output.flush();
}

/// Writes the `chronofork gui` usage, options, line grammar and output contract.
///
/// The help is one checked lock of stdout, not a series of `println!` calls: with a
/// closed or unwritable stdout the write fails as the same typed `channel` failure as
/// the data path, so the process prints one stderr line and exits 1 instead of panicking
/// inside the standard library's print machinery.
fn print_help() -> Result<(), GuiError> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    write_help(&mut output).map_err(output_failure)?;
    output.flush().map_err(output_failure)
}

/// Writes the documented help text to one writer.
fn write_help(output: &mut impl Write) -> io::Result<()> {
    writeln!(output, "chronofork {VERSION} — gui")?;
    writeln!(output)?;
    writeln!(
        output,
        "The Stage 7 human-facing terminal and HTML presentation of a ChronoFork"
    )?;
    writeln!(
        output,
        "service. The GUI never owns canonical emulated state: it reads canonical views"
    )?;
    writeln!(
        output,
        "through the M6 operation surface and mutates only through `create` and `run`."
    )?;
    writeln!(
        output,
        "There is no rendering and no audio: both are reported as unavailable."
    )?;
    writeln!(output)?;
    writeln!(output, "Usage:")?;
    writeln!(output, "  chronofork gui --help")?;
    writeln!(output, "  chronofork gui run [OPTIONS]")?;
    writeln!(output)?;
    writeln!(
        output,
        "Options (each at most once; an unknown, repeated or malformed option exits 1"
    )?;
    writeln!(output, "with empty stdout):")?;
    writeln!(
        output,
        "  --remote <URL>       Remote mode: attach to a running `chronofork serve` at this"
    )?;
    writeln!(
        output,
        "                       base URL; requires `--token-file`, excludes `--rom`"
    )?;
    writeln!(
        output,
        "  --token-file <FILE>  `chronofork.tokens.v1` bearer-token file for remote mode"
    )?;
    writeln!(
        output,
        "                       (first principal; never echoed)"
    )?;
    writeln!(
        output,
        "  --rom <FILE>         Local mode backend: the minimal Mega Drive slice instead of"
    )?;
    writeln!(
        output,
        "                       the default synthetic test backend"
    )?;
    writeln!(
        output,
        "  --width <N>          Frame width in cells (default {DEFAULT_WIDTH}, \
         {MIN_WIDTH}..={MAX_WIDTH})"
    )?;
    writeln!(
        output,
        "  --height <N>         Frame height in lines (default {DEFAULT_HEIGHT}, \
         {MIN_HEIGHT}..={MAX_HEIGHT})"
    )?;
    writeln!(
        output,
        "  --ansi               Emit ANSI frames (SGR emphasis only)"
    )?;
    writeln!(
        output,
        "  --html <OUT>         Write the HTML document to OUT when `html` runs instead of"
    )?;
    writeln!(output, "                       printing it")?;
    writeln!(
        output,
        "  --movie <FILE>       Read-only `.cfmovie` used by the timeline panel"
    )?;
    writeln!(
        output,
        "  --fps <N>            Presentation cadence (default {DEFAULT_FPS}, \
         {MIN_FPS}..={MAX_FPS})"
    )?;
    writeln!(output, "  --help, -h           Show this help")?;
    writeln!(output)?;
    writeln!(
        output,
        "Commands read one per stdin line, tokenized on ASCII whitespace; empty lines are"
    )?;
    writeln!(
        output,
        "ignored. Only `create` reaches `session.create` and only `step`, `run` and `pulse`"
    )?;
    writeln!(
        output,
        "reach `session.run`; everything else observes or changes presentation state:"
    )?;
    writeln!(
        output,
        "  create               session.create {{}}; prints the create block"
    )?;
    writeln!(
        output,
        "  list                 session.list {{}}; prints the list block"
    )?;
    writeln!(
        output,
        "  attach <SESSION>     reads the session and attaches a pane"
    )?;
    writeln!(output, "  detach               detaches the focused pane")?;
    writeln!(output, "  focus <PANE>         focuses an attached pane")?;
    writeln!(output, "  layout <single|mosaic>")?;
    writeln!(output, "                       sets the layout")?;
    writeln!(output, "  panel <NAME> <on|off>")?;
    writeln!(
        output,
        "                       sets panel visibility; NAME is status, lineage,"
    )?;
    writeln!(
        output,
        "                       capabilities, registers, memory, inspectors, timeline or"
    )?;
    writeln!(output, "                       events")?;
    writeln!(
        output,
        "  step [<N>]           control: session.run with N steps (default 1)"
    )?;
    writeln!(
        output,
        "  run <N>              control: session.run with N steps"
    )?;
    writeln!(
        output,
        "  pulse <FRAMES>       control: advance FRAMES presentation frames"
    )?;
    writeln!(
        output,
        "  pause                toggles the presentation pause flag (not a canonical pause)"
    )?;
    writeln!(output, "  handoff <human|agent>")?;
    writeln!(
        output,
        "                       transfers the cooperative controller lease"
    )?;
    writeln!(
        output,
        "  frame                prints the current frame grid"
    )?;
    writeln!(
        output,
        "  html                 prints, or writes with --html, the HTML document"
    )?;
    writeln!(
        output,
        "  quit                 prints the quit block and exits 0; EOF behaves the same"
    )?;
    writeln!(output)?;
    writeln!(output, "Output and exit codes:")?;
    writeln!(
        output,
        "  stdout carries data only: one block per command (`record=<name>` then"
    )?;
    writeln!(
        output,
        "  `key=value` lines, blocks separated by exactly one blank line), a frame grid of"
    )?;
    writeln!(
        output,
        "  exactly `--height` lines of `--width` printable ASCII columns, or the HTML"
    )?;
    writeln!(
        output,
        "  document. stderr carries one diagnostic line per failure."
    )?;
    writeln!(
        output,
        "  Exit 0 on `quit` or a clean end of input; exit 1 on any failure, with the"
    )?;
    writeln!(
        output,
        "  failing step emitting nothing on stdout. An operation failure prints the"
    )?;
    writeln!(
        output,
        "  canonical `chronofork.error.v1` envelope; every other failure prints one"
    )?;
    writeln!(output, "  `chronofork: gui: <code>: <detail>` line.")?;
    Ok(())
}
