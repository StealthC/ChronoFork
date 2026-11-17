//! Language-level acceptance tests: lexer, parser, typing, arithmetic,
//! variables and control flow.

use std::sync::Arc;

use chronofork_script::{ScriptLimits, ScriptOptions, ScriptRun, ScriptStatus, run_source};
use chronofork_test_backend::SyntheticBackendFactory;

fn options() -> ScriptOptions {
    ScriptOptions::new(Arc::new(SyntheticBackendFactory::default()))
}

fn execute(source: &str) -> ScriptRun {
    run_source(source, options())
}

fn run_with(source: &str, limits: ScriptLimits) -> ScriptRun {
    run_source(source, options().with_limits(limits))
}

fn require_failure(run: &ScriptRun) -> Result<&chronofork_script::ScriptFailure, String> {
    run.failure()
        .ok_or_else(|| format!("expected a failed run, got status {}", run.status()))
}

fn assert_failure(run: &ScriptRun, code: &str, line: usize) -> Result<(), String> {
    assert_eq!(run.status(), ScriptStatus::Failed);
    let failure = require_failure(run)?;
    assert_eq!(failure.code(), code, "failure: {failure:?}");
    assert_eq!(failure.line(), Some(line), "failure: {failure:?}");
    Ok(())
}

fn assert_parse_failure(
    source: &str,
    code: &str,
    line: usize,
    column: usize,
) -> Result<(), String> {
    let run = execute(source);
    assert_eq!(run.status(), ScriptStatus::Failed);
    let failure = require_failure(&run)?;
    assert_eq!(failure.code(), code, "failure: {failure:?}");
    assert_eq!(failure.line(), Some(line), "failure: {failure:?}");
    assert_eq!(failure.column(), Some(column), "failure: {failure:?}");
    assert!(run.prints().is_empty());
    assert_eq!(run.stats().statements(), 0);
    Ok(())
}

fn prints(run: &ScriptRun) -> Vec<&str> {
    run.prints().iter().map(String::as_str).collect()
}

fn report_text(run: &ScriptRun) -> String {
    let mut text = String::new();
    for (key, value) in run.report_fields() {
        text.push_str(&key);
        text.push('=');
        text.push_str(&value);
        text.push('\n');
    }
    text
}

#[test]
fn lexer_and_parser_report_line_and_column() -> Result<(), String> {
    assert_parse_failure("print 1 @ 2\n", "parse.unknown_character", 1, 9)?;
    assert_parse_failure("print \"abc\n", "parse.unterminated_string", 1, 7)?;
    assert_parse_failure("print \"a\\q\"\n", "parse.invalid_escape", 1, 9)?;
    assert_parse_failure("let x 1\n", "parse.expected_token", 1, 7)?;
    assert_parse_failure("print len(\"a\"\n", "parse.unexpected_eof", 2, 1)?;
    assert_parse_failure("if true {\nprint 1\n", "parse.unexpected_eof", 3, 1)?;
    assert_parse_failure("print 1\n}\n", "parse.unexpected_token", 2, 1)?;
    assert_parse_failure("let x = 1 let y = 2\n", "parse.missing_newline", 1, 11)?;
    assert_parse_failure("print 1\nelse {\n}\n", "parse.stray_else", 2, 1)?;
    assert_parse_failure("let x =", "parse.unexpected_eof", 1, 8)?;
    assert_parse_failure(
        "let x = 9223372036854775808\n",
        "parse.invalid_number",
        1,
        9,
    )?;
    assert_parse_failure("print 0x\n", "parse.invalid_number", 1, 7)?;
    assert_parse_failure("", "parse.empty_program", 1, 1)?;
    assert_parse_failure("# comment only\n", "parse.empty_program", 1, 1)?;
    assert_parse_failure("\n\n\n", "parse.empty_program", 1, 1)?;

    let oversized = "print 1234\n";
    let limits = ScriptLimits {
        source_bytes: 10,
        ..ScriptLimits::default()
    };
    let run = run_with(oversized, limits);
    assert_eq!(run.status(), ScriptStatus::Failed);
    let failure = require_failure(&run)?;
    assert_eq!(failure.code(), "limit.source_bytes");
    assert_eq!(failure.line(), None);
    assert_eq!(failure.column(), None);
    assert_eq!(run.stats().statements(), 0);
    assert!(run.prints().is_empty());

    let blank_lines = execute("let x = 1\n\nprint x\n");
    assert_eq!(blank_lines.status(), ScriptStatus::Passed);
    assert_eq!(prints(&blank_lines), ["1"]);

    let nested = format!("print {}1{}\n", "(".repeat(70), ")".repeat(70));
    let nested = execute(&nested);
    assert_eq!(nested.status(), ScriptStatus::Failed);
    assert_eq!(require_failure(&nested)?.code(), "parse.nesting_too_deep");
    Ok(())
}

#[test]
fn type_and_name_errors_are_typed() -> Result<(), String> {
    let cases = [
        ("if 1 {\nprint 1\n}\n", "type", 1),
        ("while \"x\" {\n}\n", "type", 1),
        ("print !1\n", "type", 1),
        ("print true && 1\n", "type", 1),
        ("print false || 1\n", "type", 1),
        ("repeat true {\n}\n", "type", 1),
        ("print 1 == true\n", "type", 1),
        ("print true < false\n", "type", 1),
        ("print \"a\" - \"b\"\n", "type", 1),
        ("print missing\n", "unknown_variable", 1),
        ("set y = 1\n", "unknown_variable", 1),
        ("let x = 1\nlet x = 2\n", "duplicate_variable", 2),
        ("print nosuch()\n", "unknown_builtin", 1),
        ("print len()\n", "arity", 1),
        ("print len(1)\n", "type", 1),
        ("assert 1\n", "type", 1),
        ("print fail()\n", "arity", 1),
    ];
    for (source, code, line) in cases {
        let run = execute(source);
        assert_failure(&run, code, line)
            .map_err(|error| format!("source {source:?} expected {code}: {error}"))?;
    }

    let accepted = execute(
        "print 1 + 2 * 3\n\
         print \"a\" + \"b\"\n\
         print 1 < 2\n\
         print true == false\n\
         print 1 != 2\n\
         assert 2 >= 2\n\
         let t = true\n\
         print !t\n\
         print t || false\n\
         print t && true\n",
    );
    assert_eq!(accepted.status(), ScriptStatus::Passed);
    assert_eq!(
        prints(&accepted),
        ["7", "ab", "true", "false", "true", "false", "true", "true"]
    );
    assert_eq!(accepted.stats().assertions(), 1);

    let short_circuit = execute(
        "print false && fail(\"right evaluated\")\n\
         print true || fail(\"right evaluated\")\n",
    );
    assert_eq!(short_circuit.status(), ScriptStatus::Passed);
    assert_eq!(prints(&short_circuit), ["false", "true"]);
    Ok(())
}

#[test]
fn arithmetic_is_checked_i64() -> Result<(), String> {
    let overflow = [
        "print 9223372036854775807 + 1\n",
        "print -9223372036854775807 - 2\n",
        "print 9223372036854775807 * 2\n",
        "print -(-9223372036854775807 - 1)\n",
        "print (-9223372036854775807 - 1) / -1\n",
        "print (-9223372036854775807 - 1) % -1\n",
    ];
    for source in overflow {
        let run = execute(source);
        assert_failure(&run, "arithmetic_overflow", 1)
            .map_err(|error| format!("source {source:?}: {error}"))?;
    }

    let zero = ["print 1 / 0\n", "print 1 % 0\n"];
    for source in zero {
        let run = execute(source);
        assert_failure(&run, "division_by_zero", 1)
            .map_err(|error| format!("source {source:?}: {error}"))?;
    }

    let mixed = execute("print 1 + \"a\"\n");
    assert_failure(&mixed, "type", 1)?;

    let long_text = format!(
        "print \"{}\" + \"{}\"\n",
        "a".repeat(3000),
        "b".repeat(3000)
    );
    let too_long = execute(&long_text);
    assert_failure(&too_long, "text_too_long", 1)?;

    let utf8 = execute("print \"héllo\" + \"🌍\"\n");
    assert_eq!(utf8.status(), ScriptStatus::Passed);
    assert_eq!(prints(&utf8), ["héllo🌍"]);

    let truncation = execute("print -7 / 2\nprint -7 % 2\nprint 7 / -2\nprint 7 % -2\n");
    assert_eq!(truncation.status(), ScriptStatus::Passed);
    assert_eq!(prints(&truncation), ["-3", "-1", "-3", "1"]);
    Ok(())
}

#[test]
fn let_set_and_variables_are_flat() -> Result<(), String> {
    let program = "let a = 1\n\
                   if a == 1 {\n\
                   let b = 2\n\
                   set a = b + 3\n\
                   }\n\
                   print b\n\
                   print a\n\
                   let c = 0\n\
                   repeat 2 {\n\
                   set c = c + 1\n\
                   }\n\
                   print c\n\
                   let t = \"x\"\n\
                   set t = t + \"y\"\n\
                   print t\n";
    let first = execute(program);
    assert_eq!(first.status(), ScriptStatus::Passed);
    assert_eq!(prints(&first), ["2", "5", "2", "xy"]);
    assert_eq!(first.stats().variables(), 4);

    let second = execute(program);
    assert_eq!(report_text(&first), report_text(&second));
    assert_eq!(first.prints(), second.prints());

    assert_failure(&execute("let x = 1\nlet x = 2\n"), "duplicate_variable", 2)?;
    assert_failure(&execute("set x = 1\n"), "unknown_variable", 1)?;

    let limits = ScriptLimits {
        variables: 2,
        ..ScriptLimits::default()
    };
    let limited = run_with("let a = 1\nlet b = 2\nlet c = 3\n", limits);
    assert_failure(&limited, "limit.variables", 3)?;
    assert_eq!(limited.stats().variables(), 2);
    Ok(())
}

#[test]
fn if_repeat_and_while_are_deterministic() -> Result<(), String> {
    let branches = execute(
        "let x = 2\n\
         if x == 1 {\n\
         print \"one\"\n\
         } else if x == 2 {\n\
         print \"two\"\n\
         } else {\n\
         print \"other\"\n\
         }\n",
    );
    assert_eq!(branches.status(), ScriptStatus::Passed);
    assert_eq!(prints(&branches), ["two"]);

    let repeat_zero = execute("repeat 0 {\nprint 1\n}\nprint \"done\"\n");
    assert_eq!(prints(&repeat_zero), ["done"]);

    let counted = execute(
        "let n = 0\n\
         repeat 3 {\n\
         set n = n + 1\n\
         }\n\
         while n < 6 {\n\
         set n = n + 1\n\
         }\n\
         print n\n",
    );
    assert_eq!(counted.status(), ScriptStatus::Passed);
    assert_eq!(prints(&counted), ["6"]);

    let nested = execute(
        "let x = 0\n\
         repeat 2 {\n\
         repeat 3 {\n\
         set x = x + 1\n\
         }\n\
         }\n\
         assert x == 6\n\
         print x\n",
    );
    assert_eq!(nested.status(), ScriptStatus::Passed);
    assert_eq!(prints(&nested), ["6"]);
    assert_eq!(nested.stats().assertions(), 1);

    let negative = execute("repeat -1 {\n}\n");
    assert_failure(&negative, "invalid_loop_count", 1)?;

    let bounded = run_with(
        "repeat 3 {\n}\n",
        ScriptLimits {
            loop_iterations: 2,
            ..ScriptLimits::default()
        },
    );
    assert_failure(&bounded, "limit.loop_iterations", 1)?;

    let endless = run_with(
        "while true {\n}\n",
        ScriptLimits {
            operations: 50,
            loop_iterations: 1000,
            ..ScriptLimits::default()
        },
    );
    assert_failure(&endless, "limit.operations", 1)?;

    let first = report_text(&counted);
    let second = execute(
        "let n = 0\n\
         repeat 3 {\n\
         set n = n + 1\n\
         }\n\
         while n < 6 {\n\
         set n = n + 1\n\
         }\n\
         print n\n",
    );
    assert_eq!(first, report_text(&second));
    Ok(())
}

#[test]
fn hexadecimal_integer_literals_are_documented() -> Result<(), String> {
    // ADR-0011 §3.5 documents `0x`/`0X` as the only non-decimal integer form.
    let run = execute("let x = 0x10\nlet y = 0Xff\nassert x + y == 271\nprint x\nprint y\n");
    assert_eq!(run.status(), ScriptStatus::Passed);
    assert_eq!(prints(&run), ["16", "255"]);

    // Both decimal and hexadecimal literals must fit i64.
    assert_parse_failure("let x = 0x\n", "parse.invalid_number", 1, 9)?;
    assert_parse_failure("let x = 0x8000000000000000\n", "parse.invalid_number", 1, 9)?;
    assert_parse_failure(
        "let x = 9223372036854775808\n",
        "parse.invalid_number",
        1,
        9,
    )?;
    Ok(())
}
