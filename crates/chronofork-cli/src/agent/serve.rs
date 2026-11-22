//! `chronofork serve`: host the HTTP/MCP daemon over one in-process service.
//!
//! The command composes a backend factory (synthetic by default, the Mega Drive slice
//! with `--rom <FILE>`), validates the token file through the service, prints the
//! effective configuration as `key=value` records and then waits for the server to
//! stop. It implements no operation and no authentication rule.

use std::error::Error;
use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chronofork_backend_api::BackendFactory;
use chronofork_megadrive::MegadriveFactory;
use chronofork_service::{AppService, ServiceConfig, ServiceLimits};
use chronofork_test_backend::SyntheticBackendFactory;
use chronofork_transport::{ServerConfig, serve as serve_transport};

use super::read_token_table;
use crate::{CliError, VERSION, ensure, option_value, parse_u64, parse_usize, set_once};

/// Default bind address: loopback with an ephemeral port.
const DEFAULT_BIND: &str = "127.0.0.1:0";
/// Default fixed worker count (the transport's documented default).
const DEFAULT_WORKERS: usize = 4;
/// Per-connection read and write timeout handed to the transport.
const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// Options for `chronofork serve`.
#[derive(Debug)]
struct ServeOptions {
    token_file: PathBuf,
    bind: SocketAddr,
    store_root: Option<PathBuf>,
    artifacts: Option<PathBuf>,
    rom: Option<PathBuf>,
    workers: usize,
    max_requests: Option<u64>,
}

/// Runs `chronofork serve`.
pub(crate) fn run(args: &[String]) -> Result<(), Box<dyn Error>> {
    let Some(options) = parse_serve_options(args)? else {
        print_serve_help();
        return Ok(());
    };

    let limits = ServiceLimits::default();

    // Refuse to start on an invalid or empty token file: a daemon that authenticates
    // nobody would answer 401 to every authenticated route. The read is bounded by the
    // same request limit the transport enforces, so an oversized file is rejected
    // before it is buffered.
    let tokens = read_token_table(&options.token_file, limits.max_request_bytes)?;
    let factory = backend_factory(options.rom.as_deref())?;
    // The process identity is durable beside the configured root, and only a `serve`
    // start mints one: the counter is read, incremented and written back here, so a
    // restarted daemon never reuses an epoch and a request key minted under the previous
    // one is an `unknown_outcome` instead of a re-execution (ADR-0016 §2).
    let identity_root = options
        .store_root
        .clone()
        .unwrap_or_else(|| options.token_file.clone());
    let service = Arc::new(AppService::new_durable(
        ServiceConfig {
            factory,
            storage_root: options.store_root.clone(),
            artifacts_root: options.artifacts.clone(),
            limits,
        },
        &identity_root,
    )?);
    let handle = serve_transport(
        ServerConfig {
            bind: options.bind,
            workers: options.workers,
            max_requests: options.max_requests,
            io_timeout: IO_TIMEOUT,
            max_request_bytes: limits.max_request_bytes,
            tokens,
            // The ADR bounds: the pool reserves one worker for the control path, so the
            // long-run admission is `workers - 1` globally and one per principal by
            // default. An operator may raise both through the service; the daemon keeps
            // the conservative defaults.
            max_long_runs_global: None,
            max_long_runs_per_principal: None,
        },
        Arc::clone(&service),
    )?;

    // The record names the backend family this process was started with; the exact
    // backend identifier is reported by the service `capabilities` operation.
    let backend = if options.rom.is_some() {
        "megadrive"
    } else {
        "synthetic"
    };
    let local_addr = handle.local_addr();
    println!("record=serve");
    println!("bind={local_addr}");
    println!("backend={backend}");
    println!("store={}", enabled(options.store_root.is_some()));
    println!("artifacts={}", enabled(options.artifacts.is_some()));
    io::stdout().flush()?;
    if !local_addr.ip().is_loopback() {
        eprintln!(
            "chronofork: warning: `{local_addr}` is not a loopback address; the daemon has no TLS and uses coarse static tokens"
        );
    }

    // Serving ends when `--max-requests` is reached or the process is terminated.
    // `wait` joins without requesting shutdown, so the daemon actually serves first.
    handle.wait()?;
    Ok(())
}

/// Parses `serve` arguments, returning `Ok(None)` when help was requested.
fn parse_serve_options(args: &[String]) -> Result<Option<ServeOptions>, Box<dyn Error>> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        ensure(args.len() == 1, "`serve --help` takes no other arguments")?;
        return Ok(None);
    }
    let mut token_file: Option<PathBuf> = None;
    let mut bind: Option<SocketAddr> = None;
    let mut store_root: Option<PathBuf> = None;
    let mut artifacts: Option<PathBuf> = None;
    let mut rom: Option<PathBuf> = None;
    let mut workers: Option<usize> = None;
    let mut max_requests: Option<u64> = None;

    let mut index = 0;
    while index < args.len() {
        let flag = args[index].clone();
        match flag.as_str() {
            "--token-file" => set_once(
                &mut token_file,
                PathBuf::from(option_value(args, &mut index, &flag)?),
                &flag,
            )?,
            "--bind" => {
                let value = option_value(args, &mut index, &flag)?;
                let address = value.parse().map_err(|_| {
                    CliError(format!(
                        "`{flag}` expects an address such as {DEFAULT_BIND}, got `{value}`"
                    ))
                })?;
                set_once(&mut bind, address, &flag)?;
            }
            "--store-root" => set_once(
                &mut store_root,
                PathBuf::from(option_value(args, &mut index, &flag)?),
                &flag,
            )?,
            "--artifacts" => set_once(
                &mut artifacts,
                PathBuf::from(option_value(args, &mut index, &flag)?),
                &flag,
            )?,
            "--rom" => set_once(
                &mut rom,
                PathBuf::from(option_value(args, &mut index, &flag)?),
                &flag,
            )?,
            "--workers" => {
                let value = parse_usize(option_value(args, &mut index, &flag)?, &flag)?;
                // ADR-0016 §1/F4: the pool reserves one worker for the control path, so
                // a single-worker daemon could not serve `cancel`/`list`/`progress` while
                // a long run is in flight. The CLI refuses it with the same bound the
                // transport enforces.
                if value < 2 {
                    return Err(CliError(
                        "`--workers` must be at least 2 (one worker is reserved for the \
                         control path)"
                            .to_owned(),
                    )
                    .into());
                }
                set_once(&mut workers, value, &flag)?;
            }
            "--max-requests" => {
                let value = parse_u64(option_value(args, &mut index, &flag)?, &flag)?;
                if value == 0 {
                    return Err(CliError("`--max-requests` must be at least 1".to_owned()).into());
                }
                set_once(&mut max_requests, value, &flag)?;
            }
            other => {
                return Err(CliError(format!(
                    "unknown `serve` option `{other}`; see `chronofork serve --help`"
                ))
                .into());
            }
        }
        index += 1;
    }

    let token_file =
        token_file.ok_or_else(|| CliError("`serve` requires `--token-file <FILE>`".to_owned()))?;
    Ok(Some(ServeOptions {
        token_file,
        bind: bind.unwrap_or_else(|| {
            DEFAULT_BIND
                .parse()
                .unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], 0)))
        }),
        store_root,
        artifacts,
        rom,
        workers: workers.unwrap_or(DEFAULT_WORKERS),
        max_requests,
    }))
}

/// Resolves the backend factory named by `--rom`.
fn backend_factory(rom: Option<&Path>) -> Result<Arc<dyn BackendFactory>, Box<dyn Error>> {
    match rom {
        Some(path) => {
            let bytes = std::fs::read(path).map_err(|error| {
                CliError(format!("failed to read ROM `{}`: {error}", path.display()))
            })?;
            Ok(Arc::new(MegadriveFactory::new(bytes)?))
        }
        None => Ok(Arc::new(SyntheticBackendFactory::default())),
    }
}

/// Renders a configuration toggle for the `serve` records.
fn enabled(configured: bool) -> &'static str {
    if configured { "enabled" } else { "disabled" }
}

fn print_serve_help() {
    println!("chronofork {VERSION} — serve");
    println!();
    println!("Hosts the Stage 6 HTTP and MCP endpoints over one in-process service.");
    println!();
    println!("Usage:");
    println!("  chronofork serve --token-file <FILE> [options]");
    println!();
    println!("Options:");
    println!("  --token-file <FILE>   Required `chronofork.tokens.v1` bearer-token file");
    println!("  --bind <ADDR>         Listen address (default {DEFAULT_BIND}); the effective");
    println!("                        address is printed as `bind=`");
    println!("  --store-root <DIR>    Enable the durable store operations on this root");
    println!("  --artifacts <DIR>     Persist experiment artifacts under this directory");
    println!("  --rom <FILE>          Serve the Mega Drive slice instead of the synthetic");
    println!("                        test backend");
    println!(
        "  --workers <N>         Fixed connection workers (default {DEFAULT_WORKERS}, minimum 2)"
    );
    println!("  --max-requests <N>    Stop after N served requests (bounded process tests)");
    println!("  --help                Show this help");
    println!();
    println!("Output:");
    println!("  `record=serve` and `bind=`, `backend=`, `store=`, `artifacts=` records.");
    println!("  The token file is read with the same byte bound as the request limit; an");
    println!("  oversized file is rejected with one clear stderr diagnostic.");
    println!("  An invalid or empty token file refuses to start (exit 1, empty stdout).");
    println!("  A non-loopback bind additionally prints a warning to stderr.");
    println!();
    println!("The daemon serves until it is terminated or `--max-requests` is reached.");
}
