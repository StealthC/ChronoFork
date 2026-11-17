//! `chronofork api`: a typed HTTP client for a running `chronofork serve` daemon.
//!
//! The CLI adds no meaning to a response: the daemon's canonical bytes are written
//! unchanged, and a typed remote failure is reported by its canonical error envelope.

use std::error::Error;
use std::io;
use std::path::{Path, PathBuf};

use chronofork_service::ServiceLimits;
use chronofork_service::json::{JsonValue, to_canonical_bytes};
use chronofork_transport::HttpClient;

use super::{
    parse_input_json, read_bounded, read_bounded_file, read_token_table, select_token,
    transport_failure, write_canonical_line,
};
use crate::{CliError, VERSION, ensure, option_value, set_once};

/// Parses `--input`/`--input-file` into one canonical JSON value.
fn read_input(options: &ApiOptions) -> Result<JsonValue, Box<dyn Error>> {
    let limit = ServiceLimits::default().max_request_bytes;
    match (options.input.as_deref(), options.input_file.as_deref()) {
        (Some(_), Some(_)) => {
            Err(CliError("`--input` and `--input-file` are mutually exclusive".to_owned()).into())
        }
        (Some(text), None) => parse_input_json(text.as_bytes(), "`--input`"),
        (None, Some(path)) => {
            let bytes = if path == Path::new("-") {
                read_bounded(io::stdin().lock(), limit, "input stream", "-")?
            } else {
                read_bounded_file(path, limit, "input file")?
            };
            parse_input_json(&bytes, "`--input-file`")
        }
        (None, None) => Ok(JsonValue::Object(Default::default())),
    }
}

/// The `chronofork api` subcommands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ApiKind {
    Health,
    Tools,
    Call,
}

impl ApiKind {
    const fn name(self) -> &'static str {
        match self {
            Self::Health => "health",
            Self::Tools => "tools",
            Self::Call => "call",
        }
    }

    const fn expects(self) -> &'static str {
        match self {
            Self::Health | Self::Tools => "--url <URL> --token-file <FILE> [--principal <NAME>]",
            Self::Call => {
                "--url <URL> --token-file <FILE> [--principal <NAME>] --op <NAME> \
                 [--input <JSON> | --input-file <FILE|->]"
            }
        }
    }
}

/// Parsed `api` flags.
#[derive(Debug, Default)]
struct ApiOptions {
    url: Option<String>,
    token_file: Option<PathBuf>,
    principal: Option<String>,
    op: Option<String>,
    input: Option<String>,
    input_file: Option<PathBuf>,
}

/// Runs `chronofork api`.
pub(crate) fn run(args: &[String]) -> Result<(), Box<dyn Error>> {
    let (kind, rest) = match args.first().map(String::as_str) {
        None => {
            return Err(
                CliError("`api` requires a subcommand: health, tools or call".to_owned()).into(),
            );
        }
        Some("--help" | "-h") => {
            ensure(args.len() == 1, "`api --help` takes no other arguments")?;
            print_api_help();
            return Ok(());
        }
        Some("health") => (ApiKind::Health, &args[1..]),
        Some("tools") => (ApiKind::Tools, &args[1..]),
        Some("call") => (ApiKind::Call, &args[1..]),
        Some(other) => {
            return Err(CliError(format!(
                "unknown `api` subcommand `{other}`; expected health, tools or call"
            ))
            .into());
        }
    };
    let options = parse_api_options(kind, rest)?;
    let url = options
        .url
        .clone()
        .ok_or_else(|| CliError(format!("`api {}` requires `--url <URL>`", kind.name())))?;
    let token_file = options.token_file.clone().ok_or_else(|| {
        CliError(format!(
            "`api {}` requires `--token-file <FILE>`",
            kind.name()
        ))
    })?;
    let table = read_token_table(&token_file, ServiceLimits::default().max_request_bytes)?;
    let token = select_token(&table, options.principal.as_deref(), &token_file)?;
    let client = HttpClient::new(url, token);

    let body = match kind {
        ApiKind::Health => client.health(),
        ApiKind::Tools => client.tools(),
        ApiKind::Call => {
            let op = options
                .op
                .clone()
                .ok_or_else(|| CliError("`api call` requires `--op <NAME>`".to_owned()))?;
            let input = read_input(&options)?;
            let limits = ServiceLimits::default();
            let request = JsonValue::object([("op", JsonValue::string(op)), ("input", input)]);
            let bytes = to_canonical_bytes(&request, &limits)?;
            client.call_raw(&bytes)
        }
    };
    let body = body.map_err(transport_failure)?;
    write_canonical_line(&body)?;
    Ok(())
}

fn parse_api_options(kind: ApiKind, args: &[String]) -> Result<ApiOptions, Box<dyn Error>> {
    let mut options = ApiOptions::default();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].clone();
        match flag.as_str() {
            "--url" => set_once(
                &mut options.url,
                option_value(args, &mut index, &flag)?.to_owned(),
                &flag,
            )?,
            "--token-file" => set_once(
                &mut options.token_file,
                PathBuf::from(option_value(args, &mut index, &flag)?),
                &flag,
            )?,
            "--principal" => set_once(
                &mut options.principal,
                option_value(args, &mut index, &flag)?.to_owned(),
                &flag,
            )?,
            "--op" if kind == ApiKind::Call => set_once(
                &mut options.op,
                option_value(args, &mut index, &flag)?.to_owned(),
                &flag,
            )?,
            "--input" if kind == ApiKind::Call => set_once(
                &mut options.input,
                option_value(args, &mut index, &flag)?.to_owned(),
                &flag,
            )?,
            "--input-file" if kind == ApiKind::Call => set_once(
                &mut options.input_file,
                PathBuf::from(option_value(args, &mut index, &flag)?),
                &flag,
            )?,
            other => {
                return Err(CliError(format!(
                    "unknown `api {}` option `{other}`; expected {}",
                    kind.name(),
                    kind.expects()
                ))
                .into());
            }
        }
        index += 1;
    }
    Ok(options)
}

fn print_api_help() {
    println!("chronofork {VERSION} — api");
    println!();
    println!("A typed HTTP client for a running `chronofork serve` daemon.");
    println!("The CLI adds no meaning to a response: the daemon's canonical bytes are");
    println!("written unchanged.");
    println!();
    println!("Usage:");
    println!("  chronofork api health --url <URL> --token-file <FILE> [--principal <NAME>]");
    println!("  chronofork api tools  --url <URL> --token-file <FILE> [--principal <NAME>]");
    println!("  chronofork api call   --url <URL> --token-file <FILE> [--principal <NAME>]");
    println!("                        --op <NAME> [--input <JSON> | --input-file <FILE|->]");
    println!();
    println!("Options:");
    println!("  --url <URL>        Daemon base URL such as http://127.0.0.1:8080");
    println!("  --token-file <FILE> Required `chronofork.tokens.v1` bearer-token file");
    println!("  --principal <NAME> Token to use; default: the first principal in the file");
    println!("  --op <NAME>        Operation name for `api call` (see `api tools`)");
    println!("  --input <JSON>     Canonical JSON input object (default {{}})");
    println!("  --input-file <F>   Read the input from a file, or `-` for stdin");
    println!("  --help             Show this help");
    println!();
    println!("Output and exit codes:");
    println!("  Success: the canonical response bytes plus one newline on stdout.");
    println!("  A non-2xx answer: the canonical error envelope as one stderr line, empty");
    println!("  stdout and exit 1. Usage, token-file, URL and connection failures also exit");
    println!("  1 with a single `chronofork:` stderr line. stdout is success-only.");
}
