//! Lexer, parser, formatter, and public syntax-type edge cases.

use rebis_lang::{
    format, parse, pretty_format, Expr, ModuleName, MAX_SOURCE_BYTES, MAX_SYNTAX_DEPTH,
};

fn assert_round_trip(source: &str) {
    let expression = parse(source).unwrap_or_else(|error| panic!("{source:?}: {error}"));
    let compact = format(&expression);
    assert_eq!(parse(&compact).unwrap(), expression, "compact: {compact}");
    let pretty = pretty_format(&expression);
    assert_eq!(parse(&pretty).unwrap(), expression, "pretty:\n{pretty}");
}

#[test]
fn module_names_accept_every_documented_segment_shape() {
    for name in [
        "a",
        "A0",
        "snake_case",
        "kebab-case",
        "std/loops",
        "org/team_2/review-tools",
    ] {
        let module = ModuleName::try_from(name).unwrap();
        assert_eq!(module.as_str(), name);
        assert_eq!(module.to_string(), name);
        assert_round_trip(&format!("(# {name})"));
    }
}

#[test]
fn module_names_reject_traversal_empty_segments_and_non_ascii_text() {
    for name in [
        "", ".", "..", "/root", "root/", "a//b", "a/./b", "a/../b", "a b", "a.b", "a\\b", "módulo",
        "模块", "#",
    ] {
        assert!(ModuleName::try_from(name).is_err(), "accepted {name:?}");
    }
}

#[test]
fn all_rust_whitespace_is_ignored_between_forms() {
    let expected = parse("(-> \"a\" \"b\")").unwrap();
    for whitespace in [' ', '\t', '\n', '\r', '\u{000B}', '\u{000C}', '\u{00A0}'] {
        let source = format!("({whitespace}->{whitespace}\"a\"{whitespace}\"b\"{whitespace})");
        assert_eq!(
            parse(&source).unwrap(),
            expected,
            "whitespace U+{:04X}",
            whitespace as u32
        );
    }
}

#[test]
fn prompts_preserve_recognized_escapes_unicode_and_structural_text() {
    let expression = parse(r#""line\n\t\r\"quote\"\\ λ 🜍 ()[]~,'# -> <-""#).unwrap();
    assert_eq!(
        expression,
        Expr::Prompt("line\n\t\r\"quote\"\\ λ 🜍 ()[]~,'# -> <-".to_string())
    );
    assert_round_trip(&format(&expression));
}

#[test]
fn unknown_and_unfinished_prompt_escapes_are_rejected() {
    for source in [r#""bad\qescape""#, r#""bad\0escape""#, r#""unfinished\"#] {
        let error = parse(source).expect_err(source);
        assert!(
            error.message.contains("escape"),
            "unexpected error for {source:?}: {error}"
        );
        assert!(error.offset.is_some());
    }
}

#[test]
fn escaped_quotes_do_not_end_a_prompt() {
    assert_eq!(
        parse(r#""before \"(still prompt)\" after""#).unwrap(),
        Expr::Prompt("before \"(still prompt)\" after".to_string())
    );
}

#[test]
fn arrow_tokens_are_recognized_without_surrounding_spaces() {
    assert_eq!(
        parse(r#"(->"left""right")"#).unwrap(),
        parse(r#"(-> "left" "right")"#).unwrap()
    );
    assert_eq!(
        parse(r#"(<-"left""right")"#).unwrap(),
        parse(r#"(<- "left" "right")"#).unwrap()
    );
    for source in ["(left->right)", "(left<-right)"] {
        assert!(
            parse(source).is_err(),
            "accepted embedded operator {source}"
        );
    }
}

#[test]
fn exact_parser_depth_boundaries_are_deterministic() {
    let accepted = format!(
        "{}\"core\"{}",
        "(".repeat(MAX_SYNTAX_DEPTH),
        ")".repeat(MAX_SYNTAX_DEPTH)
    );
    assert!(parse(&accepted).is_ok());

    let rejected = format!("{}x", "'".repeat(MAX_SYNTAX_DEPTH + 1));
    let error = parse(&rejected).unwrap_err();
    assert_eq!(error.message, "maximum syntax depth exceeded");
}

#[test]
fn exact_source_size_boundary_is_accepted_and_next_byte_is_rejected() {
    let accepted = "x".repeat(MAX_SOURCE_BYTES);
    assert!(parse(&accepted).is_ok());
    let rejected = format!("{accepted}x");
    let error = parse(&rejected).unwrap_err();
    assert_eq!(error.message, "maximum source size exceeded");
    assert_eq!(error.offset, Some(MAX_SOURCE_BYTES));
}

#[test]
fn every_unmatched_delimiter_reports_a_byte_location() {
    for source in [
        ")",
        "]",
        "(",
        "([x]",
        "([x",
        "((x)",
        "(~ f (x) x",
        "(~ f (x",
    ] {
        let error = parse(source).expect_err(source);
        assert!(error.offset.is_some(), "missing offset for {source:?}");
    }
}

#[test]
fn function_definition_shape_is_strict() {
    let cases = [
        "(~)",
        "(~ \"name\" () x)",
        "(~ f)",
        "(~ f x x)",
        "(~ f (x \"y\") x)",
        "(~ f (x x) x)",
        "(~ f (x) x y)",
        "(~ # () x)",
    ];
    for source in cases {
        assert!(
            parse(source).is_err(),
            "accepted malformed definition {source}"
        );
    }
}

#[test]
fn import_shape_requires_one_bare_valid_name() {
    for source in [
        "(#)",
        "(# one two)",
        r#"(# "quoted")"#,
        "(# (computed))",
        "(# a//b)",
        "(# ../escape)",
    ] {
        assert!(parse(source).is_err(), "accepted malformed import {source}");
    }
}

#[test]
fn square_and_arrow_arity_is_strict_at_every_nesting_level() {
    for source in [
        "([] x)",
        "([a b] x)",
        "([a])",
        "(->)",
        "(-> x)",
        "(<-)",
        "(<- x)",
        "((-> x))",
        "([(<- x)] y)",
    ] {
        assert!(
            parse(source).is_err(),
            "accepted malformed operator {source}"
        );
    }
}

#[test]
fn canonical_formatting_round_trips_a_structural_corpus() {
    let corpus = [
        r#""atom""#,
        "symbol",
        "'symbol",
        ",symbol",
        "('(''symbol ,symbol))",
        r#"("one")"#,
        r#"(-> "a" "b" "c" "d")"#,
        r#"(<- (-> "a" "b") (["m"] "x" "y"))"#,
        r#"([(<- "judge" "evidence")] (-> "a" "b") "c")"#,
        "(~ zero () 'symbol)",
        r#"(~ higher (worker value) '(,worker (-> ,value "done")))"#,
        r#"((~ f (x) '(-> ,x ,x)) (# std/base) (f "work"))"#,
        r#"([((~ choose () "yes?") (choose))] "yes" "no")"#,
    ];
    for source in corpus {
        assert_round_trip(source);
    }
}

#[test]
fn pretty_formatter_never_orphans_a_closing_parenthesis() {
    let expression =
        parse(r#"((~ pipeline (x) '(-> ,x (["merge"] ,x "review"))) (pipeline "task"))"#).unwrap();
    let pretty = pretty_format(&expression);
    assert!(pretty.lines().count() > 5);
    assert!(
        pretty.lines().all(|line| line.trim() != ")"),
        "orphan closing parenthesis:\n{pretty}"
    );
}

#[test]
fn repeated_format_parse_cycles_are_stable() {
    let mut expression =
        parse(r#"((~ route (x) '(-> ,x (<- "review" (["merge"] ,x "test")))) (route "fix"))"#)
            .unwrap();
    let canonical = format(&expression);
    for _ in 0..32 {
        expression = parse(&format(&expression)).unwrap();
        expression = parse(&pretty_format(&expression)).unwrap();
    }
    assert_eq!(format(&expression), canonical);
}

#[test]
fn deterministic_hostile_input_corpus_never_panics_or_loses_valid_syntax() {
    let alphabet = [
        '(', ')', '[', ']', '~', '#', '-', '>', '<', '\'', ',', '"', '\\', ' ', '\n', '\t', '\r',
        '\0', 'a', 'Z', '0', '_', '/', '.', 'λ', '界', '🜍', '\u{0301}',
    ];
    let mut state = 0x9e37_79b9usize;
    for case in 0..4_096 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let length = state % 96;
        let mut source = String::with_capacity(length);
        for _ in 0..length {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            source.push(alphabet[state % alphabet.len()]);
        }
        match parse(&source) {
            Ok(expression) => {
                assert_eq!(
                    parse(&format(&expression)).unwrap(),
                    expression,
                    "compact round trip failed for corpus case {case}: {source:?}"
                );
                assert_eq!(
                    parse(&pretty_format(&expression)).unwrap(),
                    expression,
                    "pretty round trip failed for corpus case {case}: {source:?}"
                );
            }
            Err(error) => assert!(
                error.offset.is_none_or(|offset| offset <= source.len()),
                "out-of-range offset in corpus case {case}: {source:?}: {error:?}"
            ),
        }
    }
}

// Comments --------------------------------------------------------------------

#[test]
fn semicolon_comments_run_to_end_of_line_and_are_whitespace() {
    let commented = r#"
; a full-line comment
(-> "reproduce" ; trailing comment with ) and ( and "quotes"
    "diagnose")  ; another
"#;
    assert_eq!(
        parse(commented).unwrap(),
        parse(r#"(-> "reproduce" "diagnose")"#).unwrap()
    );
}

#[test]
fn semicolon_terminates_a_word_like_whitespace() {
    assert_eq!(
        parse("(alpha;comment\n beta)").unwrap(),
        parse("(alpha beta)").unwrap()
    );
}

#[test]
fn semicolon_inside_a_prompt_is_prompt_text() {
    let expression = parse(r#""stay; this is not a comment""#).unwrap();
    assert_eq!(
        expression,
        Expr::Prompt("stay; this is not a comment".to_string())
    );
}

#[test]
fn semicolons_remain_text_in_multiline_prompts_after_escaped_quotes() {
    let expression = parse("\"first; line\nsecond \\\"quoted; text\\\"; end\"").unwrap();
    assert_eq!(
        expression,
        Expr::Prompt("first; line\nsecond \"quoted; text\"; end".to_string())
    );
}

#[test]
fn comment_at_end_of_source_without_newline_is_fine() {
    assert_eq!(
        parse("(\"work\") ; trailing").unwrap(),
        parse("(\"work\")").unwrap()
    );
}

#[test]
fn comment_only_source_is_an_empty_program_error() {
    assert!(parse("; nothing but a comment").is_err());
}

#[test]
fn commented_sources_still_round_trip_through_the_formatter() {
    // The formatter emits canonical source without comments; parsing its
    // output reproduces the same expression (comments are whitespace).
    let expression =
        parse("(~ twice (work) ; sequential re-application\n  '(-> ,work ,work))").unwrap();
    assert_round_trip("(~ twice (work) '(-> ,work ,work))");
    assert_eq!(
        expression,
        parse("(~ twice (work) '(-> ,work ,work))").unwrap()
    );
}
