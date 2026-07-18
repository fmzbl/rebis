//! Black-box command-line behavior and exit-status contracts.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_FILE: AtomicUsize = AtomicUsize::new(0);

fn rebis() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rebis"))
}

fn run_with_stdin(args: &[&str], stdin: &str) -> Output {
    let mut child = rebis()
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn temp_source(contents: &str) -> PathBuf {
    let id = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("rebis-test-{}-{id}.rebis", std::process::id()));
    fs::write(&path, contents).unwrap();
    path
}

#[test]
fn help_is_successful_and_documents_the_language_surface() {
    for argument in ["help", "-h", "--help"] {
        let output = rebis().arg(argument).output().unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("o-[]-o"));
        assert!(stdout.contains("'#' module"));
        assert!(stdout.contains("->"));
        assert!(stdout.contains("<-"));
    }
}

#[test]
fn no_arguments_prints_usage_to_stderr_and_fails() {
    let output = rebis().output().unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8(output.stderr).unwrap().contains("usage:"));
}

#[test]
fn evaluation_reads_multiline_record_from_stdin() {
    let output = run_with_stdin(&[r#"(-> "parser" "compiler")"#], "parser compiler\nnoise\n");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("score"));
    assert!(stdout.contains("terms"));
    assert!(stdout.contains("evidence 1 line(s)"));
    assert!(output.stderr.is_empty());
}

#[test]
fn split_shell_arguments_are_rejoined_into_one_program() {
    let output = run_with_stdin(&["(->", r#""alpha""#, r#""beta")"#], "alpha beta\n");
    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout)
        .unwrap()
        .contains("evidence 1 line(s)"));
}

#[test]
fn malformed_program_fails_without_writing_normal_output() {
    let output = run_with_stdin(&["(-> one)"], "");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("at least two operands"));
}

#[test]
fn tree_command_draws_the_sigil_and_scored_nodes() {
    let output = run_with_stdin(
        &["tree", r#"(["merge"] "left" "right")"#],
        "left right merge\n",
    );
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("o-[]-o\n"));
    assert!(stdout.contains("□ mediator square"));
    assert!(stdout.contains("line(s)"));
}

#[test]
fn check_reports_ok_for_valid_file() {
    let path = temp_source(r#"(-> "inspect" "report")"#);
    let output = rebis()
        .args(["check", path.to_str().unwrap()])
        .output()
        .unwrap();
    fs::remove_file(path).unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "ok\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn check_reports_byte_offset_for_invalid_file() {
    let path = temp_source("λ )");
    let output = rebis()
        .args(["check", path.to_str().unwrap()])
        .output()
        .unwrap();
    fs::remove_file(path).unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("byte 3"), "{stderr}");
}

#[test]
fn fmt_prints_canonical_source_without_modifying_the_file() {
    let original = "\n ( ->\n \"left\"   \"right\" ) \n";
    let path = temp_source(original);
    let output = rebis()
        .args(["fmt", path.to_str().unwrap()])
        .output()
        .unwrap();
    let after = fs::read_to_string(&path).unwrap();
    fs::remove_file(path).unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "(-> \"left\" \"right\")\n"
    );
    assert_eq!(after, original);
}

#[test]
fn check_fmt_and_unknown_file_fail_cleanly() {
    let path = std::env::temp_dir().join(format!("rebis-missing-{}.rebis", std::process::id()));
    let _ = fs::remove_file(&path);
    for command in ["check", "fmt"] {
        let output = rebis()
            .args([command, path.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8(output.stderr)
            .unwrap()
            .contains("could not read"));
    }
}
