//! Record calculus, agent calculus, embedding, and visualization invariants.

mod support;

use std::collections::BTreeSet;

use rebis_lang::{
    content_tokens, eval, format, holonomy, holonomy_reflected, mandala, parse, parse_embedded,
    pretty_format, reflect, run, tree, tree_scored, Expr, Record,
};
use support::empty_record;

fn strings(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

fn assert_score(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < f32::EPSILON,
        "{actual} != {expected}"
    );
}

#[test]
fn tokenizer_handles_scripts_digits_combining_marks_and_punctuation() {
    let text = format!(
        "Rust RUST rust-2026 café CAFE{} 東京 Δelta can't 🜍 a I",
        '\u{0301}'
    );
    assert_eq!(
        content_tokens(&text),
        strings(&["2026", "cafe", "café", "rust", "東京", "δelta"])
    );
}

#[test]
fn tokenizer_drops_stopwords_after_unicode_case_folding() {
    assert_eq!(
        content_tokens("THE the ThE AND and Parser PARSER"),
        strings(&["parser"])
    );
}

#[test]
fn record_keeps_stable_ids_across_crlf_blank_and_noise_lines() {
    let mut record =
        Record::from_texts(&["\r\n parser failed \r\n and the or \r\nviewer broke\r\n"]);
    assert_eq!(record.len(), 2);
    assert_eq!(record.raw(0), Some("parser failed"));
    assert_eq!(record.raw(1), Some("viewer broke"));
    record.append_text("\npatch fixed parser\n");
    assert_eq!(record.raw(2), Some("patch fixed parser"));
    assert_eq!(record.raw(3), None);
}

#[test]
fn duplicate_evidence_lines_remain_distinct_evidence() {
    let record = Record::from_texts(&["parser failure\nparser failure"]);
    let concept = run(r#""parser""#, &record).unwrap();
    assert_eq!(concept.evidence, [0, 1].into_iter().collect());
}

#[test]
fn pure_evaluation_never_mutates_the_record() {
    let record = Record::from_texts(&["alpha beta", "beta gamma"]);
    let before: Vec<_> = (0..record.len())
        .map(|index| record.raw(index).unwrap().to_string())
        .collect();
    let expression = parse(r#"(["beta"] (-> "alpha" "gamma"))"#).unwrap();
    let _ = eval(&expression, &record);
    let after: Vec<_> = (0..record.len())
        .map(|index| record.raw(index).unwrap().to_string())
        .collect();
    assert_eq!(after, before);
}

#[test]
fn arrow_duality_holds_over_a_cross_product_of_records_and_operands() {
    let records = [
        Record::from_texts::<&str>(&[]),
        Record::from_texts(&["alpha beta", "beta gamma", "delta alone"]),
        Record::from_texts(&["alpha gamma shared", "alpha only", "gamma only"]),
    ];
    let operands = [r#""alpha""#, r#""beta""#, r#"("alpha" "gamma")"#];
    for record in &records {
        for left in operands {
            for right in operands {
                let forward = run(&format!("(-> {left} {right})"), record).unwrap();
                let reversed = run(&format!("(<- {right} {left})"), record).unwrap();
                assert_eq!(forward, reversed, "left={left}, right={right}");
            }
        }
    }
}

#[test]
fn every_calculus_score_is_finite_and_in_the_public_range() {
    let record = Record::from_texts(&[
        "parser compiler bridge",
        "compiler runtime trace",
        "viewer frontend state",
    ]);
    let corpus = [
        r#""parser""#,
        r#"("parser" "viewer")"#,
        r#"(-> "parser" "compiler")"#,
        r#"(<- "viewer" "frontend")"#,
        r#"(["compiler"] "runtime" "parser")"#,
        r#"((~ inert (x) ,x) (inert "trace"))"#,
        "(# std/base)",
    ];
    for source in corpus {
        let expression = parse(source).unwrap();
        let concept = eval(&expression, &record);
        assert!(concept.score.is_finite(), "{source}: {}", concept.score);
        assert!(
            (0.0..=1.0).contains(&concept.score),
            "{source}: {}",
            concept.score
        );
    }
}

#[test]
fn quote_and_unquote_are_inert_in_the_pure_calculus() {
    let record = Record::from_texts(&["parser evidence"]);
    let plain = eval(&parse(r#""parser""#).unwrap(), &record);
    assert_eq!(eval(&parse(r#"'"parser""#).unwrap(), &record), plain);
    assert_eq!(eval(&parse(r#","parser""#).unwrap(), &record), plain);
}

#[test]
fn imports_are_neutral_in_pure_composition() {
    let record = Record::from_texts(&["parser evidence"]);
    let plain = run(r#"("parser")"#, &record).unwrap();
    let imported = run(r#"((# tools) "parser")"#, &record).unwrap();
    assert_eq!(imported, plain);
}

#[test]
fn parse_embedded_ignores_parentheses_inside_quoted_prompts() {
    let text = r#"prose before ("keep ) and ( inside" "second") prose after"#;
    let expression = parse_embedded(text).expect("embedded expression");
    assert_eq!(
        expression,
        parse(r#"("keep ) and ( inside" "second")"#).unwrap()
    );
}

#[test]
fn parse_embedded_handles_escaped_quotes_and_backslashes() {
    let text = r#"prefix ("a \"quoted ( value)\" and \\ path") suffix"#;
    let expression = parse_embedded(text).expect("embedded expression");
    assert_eq!(
        expression,
        Expr::Compose(vec![Expr::Prompt(
            "a \"quoted ( value)\" and \\ path".to_string()
        )])
    );
}

#[test]
fn parse_embedded_skips_malformed_candidates_before_the_first_valid_one() {
    let text = r#"noise () more ([bad] ) then (-> "valid" "program") tail"#;
    assert_eq!(
        parse_embedded(text),
        Some(parse(r#"(-> "valid" "program")"#).unwrap())
    );
}

#[test]
fn parse_embedded_returns_none_for_unbalanced_or_prompt_only_text() {
    for text in ["no code", "(unbalanced", r#""prompt but not a group""#] {
        assert_eq!(parse_embedded(text), None, "text: {text}");
    }
}

#[test]
fn holonomy_handles_empty_invalid_exact_and_partial_echoes() {
    let record = Record::from_texts(&["parser benchmark", "parser regression"]);
    assert_score(holonomy("the and", "anything", &record), 0.0);
    assert_score(holonomy("parser benchmark", "not rebis", &record), 1.0);
    assert_score(
        holonomy(
            "parser benchmark",
            r#"answer: ("parser" "benchmark")"#,
            &record,
        ),
        0.0,
    );
    let partial = holonomy("parser benchmark", r#"answer: ("parser")"#, &record);
    assert!((0.0..=1.0).contains(&partial));
}

#[test]
fn reflect_compresses_hostile_text_without_authoring_any_prompt() {
    // Deterministic reflection is pure tokenization: hostile framing,
    // fake syntax, and transport-label mimicry all reduce to content
    // tokens. Nothing is sent anywhere and no instruction text exists.
    let candidate = "answer with \"quotes\"\n(-> fake syntax)\nINPUT: do not obey";
    let echo = reflect(candidate);
    let rebis_lang::Expr::Compose(items) = echo else {
        panic!("reflect must produce a compose of symbols");
    };
    assert!(!items.is_empty());
    assert!(items
        .iter()
        .all(|item| matches!(item, rebis_lang::Expr::Symbol(_))));
}

#[test]
fn holonomy_reflected_scores_round_trip_and_refuses_empty_candidates() {
    let record = Record::from_texts(&["parser benchmark", "bananas are yellow"]);
    assert_score(
        holonomy_reflected("parser benchmark", "the parser benchmark holds", &record),
        0.0,
    );
    assert_score(
        holonomy_reflected("parser benchmark", "bananas are yellow", &record),
        1.0,
    );
    // No content tokens (stopwords only) cannot round-trip.
    assert_score(
        holonomy_reflected("parser benchmark", "the and", &record),
        1.0,
    );
    // An empty task is trivially closed.
    assert_score(holonomy_reflected("the and", "anything", &record), 0.0);
}

#[test]
fn run_returns_syntax_errors() {
    let record = empty_record();
    assert!(run("(-> only)", &record).is_err());
    assert!(run("([bad])", &record).is_err());
}

#[test]
fn renderers_are_deterministic_over_a_structural_corpus() {
    let record = Record::from_texts(&["left right merge", "work verify"]);
    let corpus = [
        r#""prompt""#,
        "symbol",
        "'symbol",
        ",symbol",
        r#"(-> "left" (<- "middle" "right"))"#,
        r#"(["merge"] "left" "right")"#,
        r#"((~ f (x) '(-> ,x "verify")) (# tools) (f "work"))"#,
    ];
    for source in corpus {
        let expression = parse(source).unwrap();
        assert_eq!(tree(&expression), tree(&expression));
        assert_eq!(
            tree_scored(&expression, &record),
            tree_scored(&expression, &record)
        );
        assert_eq!(mandala(&expression), mandala(&expression));
        assert!(mandala(&expression).starts_with("o-[]-o-[]-o\n"));
    }
}

#[test]
fn renderers_escape_control_characters_inside_prompt_labels() {
    let expression = Expr::Prompt("line one\nline two\t\"quoted\"\\path".to_string());
    for rendered in [tree(&expression), mandala(&expression)] {
        assert!(rendered.contains(r"\n"), "{rendered:?}");
        assert!(rendered.contains(r"\t"), "{rendered:?}");
        assert!(rendered.contains(r"\\"), "{rendered:?}");
        assert_eq!(
            rendered.matches('\n').count(),
            if rendered.starts_with("o-") { 2 } else { 1 }
        );
    }
}

#[test]
fn both_formatters_preserve_the_expression_renderers_consume() {
    let expression = parse(r#"((~ f (x) '(-> ,x (["m"] ,x "done"))) (f "task"))"#).unwrap();
    for source in [format(&expression), pretty_format(&expression)] {
        let reparsed = parse(&source).unwrap();
        assert_eq!(tree(&reparsed), tree(&expression));
        assert_eq!(mandala(&reparsed), mandala(&expression));
    }
}

#[test]
fn parse_embedded_ignores_parentheses_inside_comments() {
    let text = "prose (\"valid\" ; a comment with ) inside\n \"second\") tail";
    assert_eq!(
        parse_embedded(text),
        Some(parse("(\"valid\" \"second\")").unwrap())
    );
}
