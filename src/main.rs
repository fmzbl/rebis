//! The `rebis` command: evaluate, visualize, check, and format programs.
//!
//! The record is read from stdin when piped: `cat notes.txt | rebis "<program>"`.
//! Without a record, expressions still parse and evaluate against emptiness
//! (scores degrade honestly; nothing crashes).

use std::io::{IsTerminal, Read};
use std::process::ExitCode;

use rebis_lang::{format, parse, tree, tree_scored, Record, REBIS_SIGIL};

const USAGE: &str = "\
rebis — an atomic language to program AI  o-[]-o

usage:
  rebis \"<program>\"          evaluate against the record on stdin
  rebis tree \"<program>\"     draw the o-[]-o tree (scored when stdin is piped)
  rebis check <file>         parse a .rebis file, report ok or the error
  rebis fmt <file>           print the canonical form of a .rebis file

the language:
  program := expr+ EOF
  expr := prompt | symbol | '(' form ')'
  form := '~' symbol '(' symbol* ')' expr | '#' module | '[' expr ']' expr+ | '$' expr+ | op expr expr+ | symbol expr* | expr+
  op   := '->' | '<-'

  expr expr  top-level forms share one program scope; no outer group required
  ([m] a b)  run branches a and b, then execute mediator m
  (~ f (x) body)  define function f with lexical parameter x (its variables)
  (# module) import a host-resolved definition-only module
  ($ a b)    compose one string from the inert text of a and b, then fire it
  (-> a b)   flow a into b: the part of b that a explains
  (<- a b)   refine a by b: the part of a that b explains
  (a b)      compose

example:
  printf 'late commits\\nshort sleep on friday\\n' | rebis \"([\\\"Summarize the evidence\\\"] \\\"commits\\\" \\\"sleep\\\")\"";

fn stdin_record() -> Option<Record> {
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        return None;
    }
    let mut text = String::new();
    stdin.lock().read_to_string(&mut text).ok()?;
    Some(Record::from_texts(&[text]))
}

fn eval_cmd(src: &str) -> ExitCode {
    let expr = match parse(src) {
        Ok(expr) => expr,
        Err(error) => {
            eprintln!("rebis: {error}");
            return ExitCode::FAILURE;
        }
    };
    let record = stdin_record().unwrap_or_else(|| Record::from_texts::<&str>(&[]));
    let concept = rebis_lang::eval(&expr, &record);
    println!("score    {:.3}", concept.score);
    println!(
        "terms    {}",
        concept.terms.iter().cloned().collect::<Vec<_>>().join(" ")
    );
    println!("evidence {} line(s)", concept.evidence.len());
    ExitCode::SUCCESS
}

fn tree_cmd(src: &str) -> ExitCode {
    let expr = match parse(src) {
        Ok(expr) => expr,
        Err(error) => {
            eprintln!("rebis: {error}");
            return ExitCode::FAILURE;
        }
    };
    println!("{REBIS_SIGIL}");
    match stdin_record() {
        Some(record) => print!("{}", tree_scored(&expr, &record)),
        None => print!("{}", tree(&expr)),
    }
    ExitCode::SUCCESS
}

fn read_file(path: &str) -> Result<String, ExitCode> {
    std::fs::read_to_string(path).map_err(|error| {
        eprintln!("rebis: could not read {path}: {error}");
        ExitCode::FAILURE
    })
}

fn check_cmd(path: &str) -> ExitCode {
    let source = match read_file(path) {
        Ok(source) => source,
        Err(code) => return code,
    };
    match parse(&source) {
        Ok(_) => {
            println!("ok");
            ExitCode::SUCCESS
        }
        Err(error) => {
            match error.offset {
                Some(offset) => eprintln!("rebis: {error} (byte {offset})"),
                None => eprintln!("rebis: {error}"),
            }
            ExitCode::FAILURE
        }
    }
}

fn fmt_cmd(path: &str) -> ExitCode {
    let source = match read_file(path) {
        Ok(source) => source,
        Err(code) => return code,
    };
    match parse(&source) {
        Ok(expr) => {
            println!("{}", format(&expr));
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("rebis: {error}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => {
            eprintln!("{USAGE}");
            ExitCode::FAILURE
        }
        [one] if one == "-h" || one == "--help" || one == "help" => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        [cmd, rest @ ..] if cmd == "tree" && !rest.is_empty() => tree_cmd(&rest.join(" ")),
        [cmd, path] if cmd == "check" => check_cmd(path),
        [cmd, path] if cmd == "fmt" => fmt_cmd(path),
        rest => eval_cmd(&rest.join(" ")),
    }
}
