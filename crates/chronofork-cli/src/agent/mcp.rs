//! `chronofork mcp`: an MCP Streamable HTTP client for a running daemon.
//!
//! `tools/call` result text is exactly the service envelope the HTTP surface returns, so
//! the CLI prints it unchanged; `isError:true` is a typed failure.

use std::error::Error;
use std::path::PathBuf;

use chronofork_service::ServiceLimits;
use chronofork_service::json::{JsonValue, to_canonical_bytes};
use chronofork_transport::McpClient;

use super::{
    FailureReported, parse_input_json, read_token_table, select_token, transport_failure,
    write_canonical_line, write_stderr_line,
};
use crate::{CliError, VERSION, ensure, option_value, set_once};

/// The `chronofork mcp` subcommands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum McpKind {
    Tools,
    Call,
}

impl McpKind {
    const fn name(self) -> &'static str {
        match self {
            Self::Tools => "tools",
            Self::Call => "call",
        }
    }

    const fn expects(self) -> &'static str {
        match self {
            Self::Tools => "--url <URL> --token-file <FILE> [--principal <NAME>]",
            Self::Call => {
                "--url <URL> --token-file <FILE> [--principal <NAME>] --tool <NAME> \
                 [--arguments <JSON>]"
            }
        }
    }
}

/// Parsed `mcp` flags.
#[derive(Debug, Default)]
struct McpOptions {
    url: Option<String>,
    token_file: Option<PathBuf>,
    principal: Option<String>,
    tool: Option<String>,
    arguments: Option<String>,
}

/// Runs `chronofork mcp`.
pub(crate) fn run(args: &[String]) -> Result<(), Box<dyn Error>> {
    let (kind, rest) = match args.first().map(String::as_str) {
        None => {
            return Err(CliError("`mcp` requires a subcommand: tools or call".to_owned()).into());
        }
        Some("--help" | "-h") => {
            ensure(args.len() == 1, "`mcp --help` takes no other arguments")?;
            print_mcp_help();
            return Ok(());
        }
        Some("tools") => (McpKind::Tools, &args[1..]),
        Some("call") => (McpKind::Call, &args[1..]),
        Some(other) => {
            return Err(CliError(format!(
                "unknown `mcp` subcommand `{other}`; expected tools or call"
            ))
            .into());
        }
    };
    let options = parse_mcp_options(kind, rest)?;
    let url = options
        .url
        .clone()
        .ok_or_else(|| CliError(format!("`mcp {}` requires `--url <URL>`", kind.name())))?;
    let token_file = options.token_file.clone().ok_or_else(|| {
        CliError(format!(
            "`mcp {}` requires `--token-file <FILE>`",
            kind.name()
        ))
    })?;
    let table = read_token_table(&token_file, ServiceLimits::default().max_request_bytes)?;
    let token = select_token(&table, options.principal.as_deref(), &token_file)?;
    let client = McpClient::new(url, token);
    let limits = ServiceLimits::default();

    match kind {
        McpKind::Tools => {
            let result = client.list_tools().map_err(transport_failure)?;
            let bytes = to_canonical_bytes(&result, &limits)?;
            write_canonical_line(&bytes)?;
        }
        McpKind::Call => {
            let tool = options
                .tool
                .clone()
                .ok_or_else(|| CliError("`mcp call` requires `--tool <NAME>`".to_owned()))?;
            let arguments = match options.arguments.as_deref() {
                Some(text) => parse_input_json(text.as_bytes(), "`--arguments`")?,
                None => JsonValue::Object(Default::default()),
            };
            let outcome = client
                .call_tool(&tool, &arguments)
                .map_err(transport_failure)?;
            if outcome.is_error {
                write_stderr_line(outcome.text.as_bytes());
                return Err(Box::new(FailureReported));
            }
            write_canonical_line(outcome.text.as_bytes())?;
        }
    }
    Ok(())
}

fn parse_mcp_options(kind: McpKind, args: &[String]) -> Result<McpOptions, Box<dyn Error>> {
    let mut options = McpOptions::default();
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
            "--tool" if kind == McpKind::Call => set_once(
                &mut options.tool,
                option_value(args, &mut index, &flag)?.to_owned(),
                &flag,
            )?,
            "--arguments" if kind == McpKind::Call => set_once(
                &mut options.arguments,
                option_value(args, &mut index, &flag)?.to_owned(),
                &flag,
            )?,
            other => {
                return Err(CliError(format!(
                    "unknown `mcp {}` option `{other}`; expected {}",
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

fn print_mcp_help() {
    println!("chronofork {VERSION} — mcp");
    println!();
    println!("An MCP client over Streamable HTTP (protocol 2025-03-26) for a running");
    println!("`chronofork serve` daemon. `mcp call` prints the tool result text, which");
    println!("is the canonical service envelope the HTTP surface would return.");
    println!();
    println!("Usage:");
    println!("  chronofork mcp tools --url <URL> --token-file <FILE> [--principal <NAME>]");
    println!("  chronofork mcp call  --url <URL> --token-file <FILE> [--principal <NAME>]");
    println!("                       --tool <NAME> [--arguments <JSON>]");
    println!();
    println!("Options:");
    println!("  --url <URL>        Daemon base URL such as http://127.0.0.1:8080");
    println!("  --token-file <FILE> Required `chronofork.tokens.v1` bearer-token file");
    println!("  --principal <NAME> Token to use; default: the first principal in the file");
    println!("  --tool <NAME>      Tool name for `mcp call` (see `mcp tools`)");
    println!("  --arguments <JSON> Canonical JSON arguments object (default {{}})");
    println!("  --help             Show this help");
    println!();
    println!("Output and exit codes:");
    println!("  Success: the canonical result text plus one newline on stdout.");
    println!("  `tools/call` with `isError:true`: the error envelope as one stderr line,");
    println!("  empty stdout and exit 1. Transport and JSON-RPC failures exit 1 with a");
    println!("  single `chronofork:` stderr line.");
}
