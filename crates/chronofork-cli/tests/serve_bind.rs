//! Process-level test for the loopback-only bind refusal (ADR-0014 §1).
//!
//! `chronofork serve` must refuse a non-loopback `--bind` **before** it reads the token
//! file, composes the service, binds a socket or prints `record=serve`. The test proves
//! the ordering by giving the command a token file that does not exist: only the bind
//! refusal diagnostic is allowed to appear, so reaching the token stage would fail the
//! test instead of being mistaken for a refusal.
//!
//! The refusal is one stable `chronofork: <message>` stderr line and an empty stdout. A
//! loopback bind with the same missing token file still reaches the token stage, which
//! keeps the ordinary loopback path (including an ephemeral port) covered.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Absolute bound on a refusal; the command must not reach the serving loop.
const REFUSAL_TIMEOUT: Duration = Duration::from_secs(10);

/// Token file that is deliberately absent: a refused bind must never read it.
const MISSING_TOKEN_FILE: &str = "/nonexistent/chronofork-serve-bind-test/tokens.json";

/// Spawns `chronofork serve` for one `--bind` value.
fn spawn_serve(bind: &str) -> Option<Child> {
    Command::new(env!("CARGO_BIN_EXE_chronofork"))
        .args(["serve", "--token-file", MISSING_TOKEN_FILE, "--bind", bind])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()
}

/// Waits for the child to exit within the deadline and returns its output.
///
/// `None` means the child had to be killed (it reached the serving loop or failed to
/// report a status), so the caller fails the test instead of hanging.
fn wait_and_collect(mut child: Child) -> Option<(i32, String, String)> {
    let deadline = Instant::now() + REFUSAL_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let mut stdout = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    Some((status.code().unwrap_or(-1), stdout, stderr))
}

#[test]
fn a_non_loopback_bind_is_refused_before_the_token_file_is_read() {
    for bind in ["0.0.0.0:0", "[::]:0"] {
        let outcome = spawn_serve(bind).and_then(wait_and_collect);
        assert!(
            outcome.is_some(),
            "`--bind {bind}` must exit within {REFUSAL_TIMEOUT:?}, not keep serving"
        );
        let Some((code, stdout, stderr)) = outcome else {
            return;
        };
        assert_ne!(code, 0, "`--bind {bind}` must exit nonzero");
        assert!(
            stdout.is_empty(),
            "`--bind {bind}` must not print `record=serve` or any record, got {stdout:?}"
        );
        // One stable diagnostic line: the phase it names proves the refusal happened
        // before the token read, and the address is the requested one.
        let expected = format!(
            "chronofork: `--bind` refuses the non-loopback address `{bind}`; v1 serves \
             loopback only (ADR-0014)\n"
        );
        assert_eq!(
            stderr, expected,
            "`--bind {bind}` must print exactly the stable refusal diagnostic"
        );
    }
}

#[test]
fn a_loopback_bind_still_reaches_the_token_check() {
    let outcome = spawn_serve("127.0.0.1:0").and_then(wait_and_collect);
    assert!(
        outcome.is_some(),
        "a loopback bind with a missing token file must exit within {REFUSAL_TIMEOUT:?}"
    );
    let Some((code, stdout, stderr)) = outcome else {
        return;
    };
    assert_ne!(code, 0, "a missing token file must still refuse to start");
    assert!(
        stdout.is_empty(),
        "a refused startup prints no records: {stdout:?}"
    );
    assert!(
        stderr.starts_with("chronofork: "),
        "a local failure prints one `chronofork: ` line: {stderr:?}"
    );
    assert!(
        stderr.contains("failed to read token file"),
        "a loopback bind reaches the token read: {stderr:?}"
    );
    assert!(
        !stderr.contains("non-loopback"),
        "a loopback bind is not refused by the bind rule: {stderr:?}"
    );
}
