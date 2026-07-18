//! Public-API conformance and edge-case coverage for the Rebis language.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use rebis_lang::{
    content_tokens, format, mandala, orchestrate, orchestrate_with_limits,
    orchestrate_with_observer, orchestrate_with_runtime, parse, pretty_format, run, tree,
    tree_scored, ExecutionEvent, Expr, ModuleName, ModuleResolver, Oracle, Record,
    RuntimeDiagnostic, RuntimeLimits, MAX_SOURCE_BYTES, MAX_SYNTAX_DEPTH,
};

#[derive(Default)]
struct ScriptedOracle {
    answers: RefCell<VecDeque<Option<String>>>,
    prompts: RefCell<Vec<String>>,
}

impl ScriptedOracle {
    fn new(answers: &[Option<&str>]) -> Self {
        Self {
            answers: RefCell::new(
                answers
                    .iter()
                    .map(|answer| answer.map(str::to_string))
                    .collect(),
            ),
            prompts: RefCell::new(Vec::new()),
        }
    }

    fn prompts(&self) -> Vec<String> {
        self.prompts.borrow().clone()
    }
}

impl Oracle for ScriptedOracle {
    fn fire(&self, prompt: &str) -> Option<String> {
        self.prompts.borrow_mut().push(prompt.to_string());
        self.answers.borrow_mut().pop_front().flatten()
    }
}

struct FailedOracle;

impl Oracle for FailedOracle {
    fn fire(&self, _prompt: &str) -> Option<String> {
        None
    }

    fn try_fire(&self, _prompt: &str) -> Result<Option<String>, String> {
        Err("provider timed out".to_string())
    }
}

#[derive(Default)]
struct MemoryModules {
    sources: BTreeMap<String, String>,
    requests: RefCell<Vec<String>>,
}

impl MemoryModules {
    fn with(mut self, name: &str, source: &str) -> Self {
        self.sources.insert(name.to_string(), source.to_string());
        self
    }
}

impl ModuleResolver for MemoryModules {
    fn resolve(&self, module: &ModuleName) -> Result<Option<String>, String> {
        self.requests.borrow_mut().push(module.to_string());
        Ok(self.sources.get(module.as_str()).cloned())
    }
}

fn empty_record() -> Record {
    Record::from_texts::<&str>(&[])
}

fn strings(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

fn assert_score(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < f32::EPSILON,
        "{actual} != {expected}"
    );
}

// Syntax and formatting -------------------------------------------------------

#[test]
fn whitespace_is_semantically_irrelevant_outside_prompts() {
    let compact = parse("(-> \"inspect\" \"report\")").unwrap();
    let spacious = parse("\n (  ->\n\t\"inspect\"   \n \"report\"  ) \n").unwrap();
    assert_eq!(compact, spacious);
}

#[test]
fn whitespace_and_structural_characters_inside_prompts_are_raw_text() {
    let expr = parse(r#""  keep ( -> [ ] ~ ' , and  spaces  ""#).unwrap();
    assert_eq!(
        expr,
        Expr::Prompt("  keep ( -> [ ] ~ ' , and  spaces  ".into())
    );
}

#[test]
fn prompt_escapes_and_unicode_round_trip_through_both_formatters() {
    let expr = Expr::Prompt("línea one\n\t\"quoted\" \\ λ 🜍\r".into());
    for rendered in [format(&expr), pretty_format(&expr)] {
        assert_eq!(parse(&rendered).unwrap(), expr, "rendered as {rendered:?}");
    }
}

#[test]
fn arrow_chains_are_left_associative() {
    let parsed = parse("(-> a b c d)").unwrap();
    let expected = Expr::Forward(
        Box::new(Expr::Forward(
            Box::new(Expr::Forward(
                Box::new(Expr::Symbol("a".into())),
                Box::new(Expr::Symbol("b".into())),
            )),
            Box::new(Expr::Symbol("c".into())),
        )),
        Box::new(Expr::Symbol("d".into())),
    );
    assert_eq!(parsed, expected);
}

#[test]
fn singleton_group_is_preserved_as_an_abstraction_boundary() {
    assert_eq!(
        parse("((\"higher\"))").unwrap(),
        Expr::Compose(vec![Expr::Compose(vec![Expr::Prompt("higher".into())])])
    );
}

#[test]
fn complex_program_round_trips_through_compact_and_pretty_source() {
    let source = r#"
      (
        (~ apply (worker value) '(,worker ,value))
        (~ pipeline (value) '(-> ,value (["mediate"] ,value "review")))
        (apply pipeline "inspect"))
    "#;
    let expr = parse(source).unwrap();
    assert_eq!(parse(&format(&expr)).unwrap(), expr);
    let pretty = pretty_format(&expr);
    assert!(pretty.lines().count() >= 8);
    assert!(!pretty.lines().any(|line| line.trim() == ")"));
    assert_eq!(parse(&pretty).unwrap(), expr);
}

#[test]
fn deeply_nested_valid_program_round_trips_without_losing_depth() {
    let depth = 96;
    let source = format!("{}\"core\"{}", "(".repeat(depth), ")".repeat(depth));
    let expr = parse(&source).unwrap();
    assert_eq!(parse(&format(&expr)).unwrap(), expr);
    assert_eq!(parse(&pretty_format(&expr)).unwrap(), expr);
}

#[test]
fn parser_rejects_resource_exhaustion_before_recursive_descent() {
    let too_deep = format!(
        "{}\"core\"{}",
        "(".repeat(MAX_SYNTAX_DEPTH + 1),
        ")".repeat(MAX_SYNTAX_DEPTH + 1)
    );
    assert!(parse(&too_deep)
        .unwrap_err()
        .message
        .contains("maximum syntax depth"));

    let too_many_quotes = format!("{}\"core\"", "'".repeat(MAX_SYNTAX_DEPTH + 1));
    assert!(parse(&too_many_quotes)
        .unwrap_err()
        .message
        .contains("maximum syntax depth"));

    let too_large = "x".repeat(MAX_SOURCE_BYTES + 1);
    assert!(parse(&too_large)
        .unwrap_err()
        .message
        .contains("maximum source size"));
}

#[test]
fn malformed_programs_report_a_location_and_specific_diagnostic() {
    let cases = [
        ("", "empty expression"),
        ("()", "empty group"),
        ("(a", "unbalanced `(`"),
        ("([a]", "unbalanced `(`"),
        ("([] a)", "exactly one"),
        ("([a b] c)", "exactly one"),
        ("([a])", "at least one branch"),
        ("(->)", "at least two operands"),
        ("(-> a)", "at least two operands"),
        ("(~ f x x)", "parameters must be a list"),
        ("(~ f (\"x\") x)", "parameters must be symbols"),
        ("(~ f (x) x y)", "exactly one body"),
        ("(~ f (x x) x)", "duplicate function parameter"),
        ("(~ # (x) x)", "reserved for module imports"),
        ("\"unterminated", "unterminated quoted prompt"),
        ("'", "quote needs an expression"),
        (",", "unquote needs an expression"),
    ];
    for (source, expected) in cases {
        let error = parse(source).expect_err(source);
        assert!(
            error.message.contains(expected),
            "{source:?}: expected {expected:?}, got {error}"
        );
        assert!(error.offset.is_some(), "{source:?} has no error offset");
    }
}

#[test]
fn hash_import_is_structural_and_module_names_are_validated() {
    let imported = parse("(# std/loops)").unwrap();
    assert_eq!(format(&imported), "(# std/loops)");
    assert_eq!(pretty_format(&imported), "(# std/loops)");
    for invalid in ["(#)", "(# one two)", "(# ../escape)", "(# /root)"] {
        assert!(parse(invalid).is_err(), "accepted invalid import {invalid}");
    }
}

#[test]
fn imported_hypersigil_definitions_are_available_lexically() {
    let modules =
        MemoryModules::default().with("tools", "((~ inspect (target) '(-> ,target \"verify\")))");
    let oracle = ScriptedOracle::new(&[Some("analysis"), Some("verified")]);
    let mut record = empty_record();
    let mut events = Vec::new();
    let result = orchestrate_with_runtime(
        &parse("((# tools) (inspect \"parser\"))").unwrap(),
        &mut record,
        &oracle,
        &modules,
        &mut |event| events.push(event.clone()),
    );

    assert_eq!(oracle.prompts()[0], "parser");
    assert!(oracle.prompts()[1].starts_with("verify\n\nINPUT:"));
    assert_eq!(result.output.as_deref(), Some("verified"));
    assert!(result.diagnostics.is_empty());
    assert!(events.iter().any(|event| matches!(
        event,
        ExecutionEvent::ModuleLoaded { module, definitions: 1 }
            if module.as_str() == "tools"
    )));
}

#[test]
fn modules_can_reexport_nested_imports_and_are_cached_per_run() {
    let modules = MemoryModules::default()
        .with("lib/base", "(~ identity (value) ',value)")
        .with("lib/prelude", "((# lib/base) (~ twice (x) '(-> ,x ,x)))");
    let oracle = ScriptedOracle::new(&[Some("first"), Some("second")]);
    let mut record = empty_record();
    let result = orchestrate_with_runtime(
        &parse("((# lib/prelude) (# lib/prelude) (identity \"ready\"))").unwrap(),
        &mut record,
        &oracle,
        &modules,
        &mut |_| {},
    );

    assert_eq!(result.output.as_deref(), Some("first"));
    assert!(result.diagnostics.is_empty());
    let requests = modules.requests.borrow();
    assert_eq!(
        requests
            .iter()
            .filter(|name| *name == "lib/prelude")
            .count(),
        1
    );
    assert_eq!(
        requests.iter().filter(|name| *name == "lib/base").count(),
        1
    );
}

#[test]
fn module_graph_failures_are_typed_and_side_effect_free() {
    let cases = [
        (
            MemoryModules::default(),
            "((# missing) \"must still be explicit\")",
            "module `missing` was not found",
        ),
        (
            MemoryModules::default().with("bad", "(\"executable\")"),
            "(# bad)",
            "modules contain only",
        ),
        (
            MemoryModules::default()
                .with("a", "(# b)")
                .with("b", "(# a)"),
            "(# a)",
            "module import cycle: a -> b -> a",
        ),
    ];
    for (modules, source, expected) in cases {
        let oracle = ScriptedOracle::default();
        let mut record = empty_record();
        let result = orchestrate_with_runtime(
            &parse(source).unwrap(),
            &mut record,
            &oracle,
            &modules,
            &mut |_| {},
        );
        assert!(result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.to_string().contains(expected)));
        if !source.contains("must still") {
            assert!(oracle.prompts().is_empty());
        }
    }
}

#[test]
fn utf8_error_offsets_are_byte_offsets() {
    let error = parse("λ )").unwrap_err();
    assert_eq!(error.offset, Some("λ ".len()));
}

// Record and deterministic calculus -----------------------------------------

#[test]
fn tokenization_lowercases_deduplicates_and_drops_noise() {
    assert_eq!(
        content_tokens("THE Parser, parser! Δelta x 42 and café."),
        strings(&["42", "café", "parser", "δelta"])
    );
}

#[test]
fn record_ignores_empty_lines_and_preserves_trimmed_raw_evidence() {
    let mut record = Record::from_texts(&[" \n the and or \n  Parser failed here  \n"]);
    assert_eq!(record.len(), 1);
    assert_eq!(record.raw(0), Some("Parser failed here"));
    assert!(record.line(0).unwrap().contains("parser"));
    assert_eq!(record.raw(1), None);
    assert_eq!(record.line(1), None);

    record.append_text("\nPatch fixed parser\n");
    assert_eq!(record.len(), 2);
    assert_eq!(record.raw(1), Some("Patch fixed parser"));
}

#[test]
fn composition_unions_terms_and_evidence() {
    let record = Record::from_texts(&["parser regression", "viewer regression", "unrelated note"]);
    let concept = run("(\"parser\" \"viewer\")", &record).unwrap();
    assert_eq!(concept.terms, strings(&["parser", "viewer"]));
    assert_eq!(concept.evidence, [0, 1].into_iter().collect());
    assert_score(concept.score, 1.0);
}

#[test]
fn forward_and_reversed_backflow_obey_the_same_law() {
    let record = Record::from_texts(&[
        "parser compiler shared",
        "compiler backend",
        "viewer frontend",
    ]);
    let forward = run("(-> \"parser\" \"compiler\")", &record).unwrap();
    let backflow = run("(<- \"compiler\" \"parser\")", &record).unwrap();
    assert_eq!(forward, backflow);
}

#[test]
fn arrows_fail_soft_with_an_empty_record() {
    let record = empty_record();
    let related = run("(-> \"parser\" \"parser\")", &record).unwrap();
    let unrelated = run("(-> \"parser\" \"viewer\")", &record).unwrap();
    assert_score(related.score, 1.0);
    assert_score(unrelated.score, 0.0);
    assert!(related.evidence.is_empty());
}

#[test]
fn square_finds_shared_record_ground_and_scores_the_overlap() {
    let record = Record::from_texts(&[
        "parser benchmark shared",
        "parser isolated",
        "benchmark isolated",
    ]);
    let concept = run("([\"parser\"] \"benchmark\")", &record).unwrap();
    assert!(concept.evidence.contains(&0));
    assert!(concept.score > 0.0 && concept.score <= 1.0);
    assert!(concept.terms.contains("parser") || concept.terms.contains("benchmark"));
}

#[test]
fn unresolved_square_preserves_both_sides_with_zero_score() {
    let concept = run("([\"alpha\"] \"omega\")", &empty_record()).unwrap();
    assert_eq!(concept.terms, strings(&["alpha", "omega"]));
    assert!(concept.evidence.is_empty());
    assert_score(concept.score, 0.0);
}

// Deterministic mediation ----------------------------------------------------

#[test]
fn pure_symbol_mediator_judges_without_firing_a_model_for_mediation() {
    // Two prompts answer; the symbol mediator judges deterministically.
    // Exactly two model calls happen — none for the mediator.
    let oracle = ScriptedOracle::new(&[
        Some("the parser benchmark improved"),
        Some("bananas are yellow"),
    ]);
    let mut record = empty_record();
    let result = orchestrate(
        &parse(r#"([parser-benchmark] "measure the parser" "off topic")"#).unwrap(),
        &mut record,
        &oracle,
    );
    assert_eq!(oracle.prompts().len(), 2);
    assert_eq!(
        result.output.as_deref(),
        Some("the parser benchmark improved")
    );
    assert!(result
        .events
        .iter()
        .any(|event| matches!(event, ExecutionEvent::MediatorResolved { result: 1, .. })));
}

#[test]
fn pure_symbol_mediator_yields_nothing_when_no_answer_round_trips() {
    let oracle = ScriptedOracle::new(&[Some("bananas are yellow"), Some("the sky is blue")]);
    let mut record = empty_record();
    let result = orchestrate(
        &parse(r#"([parser-benchmark] "one" "two")"#).unwrap(),
        &mut record,
        &oracle,
    );
    assert_eq!(result.output, None);
    assert!(!result
        .events
        .iter()
        .any(|event| matches!(event, ExecutionEvent::MediatorResolved { .. })));
}

#[test]
fn prompt_mediators_still_mediate_and_conditionals_stay_lazy() {
    // A prompted mediator is unaffected by deterministic mediation.
    let oracle = ScriptedOracle::new(&[Some("branch answer"), Some("mediated verdict")]);
    let mut record = empty_record();
    let result = orchestrate(
        &parse(r#"(["combine"] "branch")"#).unwrap(),
        &mut record,
        &oracle,
    );
    assert_eq!(result.output.as_deref(), Some("mediated verdict"));
}

// Full orchestration ---------------------------------------------------------

#[test]
fn prompt_is_fired_verbatim_and_answer_becomes_output_and_evidence() {
    let prompt = "Fix (parser) -> safely [today]";
    let oracle = ScriptedOracle::new(&[Some("parser fix evidence")]);
    let mut record = empty_record();
    let result = orchestrate(&Expr::Prompt(prompt.into()), &mut record, &oracle);
    assert_eq!(oracle.prompts(), [prompt]);
    assert_eq!(result.output.as_deref(), Some("parser fix evidence"));
    assert_eq!(result.firings[0].prompt, prompt);
    assert_eq!(record.len(), 1);
}

#[test]
fn nothing_is_traced_but_not_returned_or_recorded() {
    let oracle = ScriptedOracle::new(&[Some(" NOTHING\n")]);
    let mut record = empty_record();
    let result = orchestrate(&parse("\"question\"").unwrap(), &mut record, &oracle);
    assert_eq!(result.output, None);
    assert_eq!(result.firings[0].answer.as_deref(), Some(" NOTHING\n"));
    assert!(record.is_empty());
}

#[test]
fn provider_failure_is_not_misreported_as_a_model_decline() {
    let mut record = empty_record();
    let result = orchestrate(&parse("\"question\"").unwrap(), &mut record, &FailedOracle);
    assert_eq!(result.output, None);
    assert_eq!(result.firings.len(), 1);
    assert_eq!(
        result.diagnostics,
        [RuntimeDiagnostic::OracleFailure {
            message: "provider timed out".into(),
        }]
    );
}

#[test]
fn host_model_call_budget_is_enforced_before_the_provider_boundary() {
    let oracle = ScriptedOracle::new(&[Some("first"), Some("must not be used")]);
    let modules = MemoryModules::default();
    let mut record = empty_record();
    let result = orchestrate_with_limits(
        &parse("(\"one\" \"two\")").unwrap(),
        &mut record,
        &oracle,
        &modules,
        RuntimeLimits::standard().with_model_calls(1),
        &mut |_| {},
    );

    assert_eq!(oracle.prompts(), ["one"]);
    assert_eq!(result.firings.len(), 1);
    assert_eq!(
        result.diagnostics,
        [RuntimeDiagnostic::ModelCallLimit { limit: 1 }]
    );
}

#[test]
fn host_can_disable_module_loading_with_a_zero_budget() {
    let oracle = ScriptedOracle::default();
    let modules = MemoryModules::default().with("tools", "(~ tool (x) ',x)");
    let mut record = empty_record();
    let result = orchestrate_with_limits(
        &parse("(# tools)").unwrap(),
        &mut record,
        &oracle,
        &modules,
        RuntimeLimits::standard().with_module_imports(0),
        &mut |_| {},
    );

    assert!(modules.requests.borrow().is_empty());
    assert_eq!(
        result.diagnostics,
        [RuntimeDiagnostic::ModuleLimit { limit: 0 }]
    );
}

#[test]
fn forward_arrow_routes_only_prior_non_nothing_answers() {
    let oracle = ScriptedOracle::new(&[
        Some("first answer"),
        Some("nothing"),
        Some("consumer answer"),
    ]);
    let mut record = empty_record();
    let expr = parse("(-> (\"first\" \"second\") \"consumer\")").unwrap();
    let result = orchestrate(&expr, &mut record, &oracle);
    assert_eq!(
        oracle.prompts(),
        [
            "first",
            "second",
            "consumer\n\nINPUT:\nRESULT 1:\nfirst answer"
        ]
    );
    assert_eq!(result.output.as_deref(), Some("consumer answer"));
}

#[test]
fn backflow_executes_right_first_and_routes_into_left() {
    let oracle = ScriptedOracle::new(&[Some("right value"), Some("left result")]);
    let mut record = empty_record();
    let result = orchestrate(
        &parse("(<- \"left consumer\" \"right producer\")").unwrap(),
        &mut record,
        &oracle,
    );
    assert_eq!(
        oracle.prompts(),
        [
            "right producer",
            "left consumer\n\nINPUT:\nRESULT 1:\nright value"
        ]
    );
    assert_eq!(result.output.as_deref(), Some("left result"));
}

#[test]
fn square_runs_all_branches_in_order_then_mediator() {
    let oracle = ScriptedOracle::new(&[Some("A"), None, Some("C"), Some("synthesis")]);
    let mut record = empty_record();
    let result = orchestrate(
        &parse("([\"merge\"] \"one\" \"two\" \"three\")").unwrap(),
        &mut record,
        &oracle,
    );
    assert_eq!(
        oracle.prompts(),
        [
            "one",
            "two",
            "three",
            "merge\n\nINPUT:\nRESULT 1:\nA\n\nRESULT 2:\nC"
        ]
    );
    assert_eq!(result.output.as_deref(), Some("synthesis"));
}

#[test]
fn nested_groups_raise_abstraction_without_changing_raw_prompt() {
    let oracle = ScriptedOracle::new(&[Some("a"), Some("b"), Some("c")]);
    let mut record = empty_record();
    let result = orchestrate(
        &parse("(\"level one\" (\"level two\" (\"level three\")))").unwrap(),
        &mut record,
        &oracle,
    );
    let levels: Vec<_> = result
        .firings
        .iter()
        .map(|firing| (firing.prompt.as_str(), firing.abstraction))
        .collect();
    assert_eq!(
        levels,
        [("level one", 1), ("level two", 2), ("level three", 3)]
    );
}

#[test]
fn final_executable_item_is_the_value_of_a_group() {
    let oracle = ScriptedOracle::new(&[Some("first"), Some("last")]);
    let mut record = empty_record();
    let result = orchestrate(
        &parse("(\"discarded structural value\" \"returned value\")").unwrap(),
        &mut record,
        &oracle,
    );
    assert_eq!(result.output.as_deref(), Some("last"));
}

#[test]
fn macro_arguments_are_structural_and_prompts_are_not_textually_interpolated() {
    let source = r#"
      ((~ wrap (value) '(-> ,value "literal value stays literal"))
       (wrap "argument prompt"))
    "#;
    let oracle = ScriptedOracle::new(&[Some("argument answer"), Some("done")]);
    let mut record = empty_record();
    let _ = orchestrate(&parse(source).unwrap(), &mut record, &oracle);
    assert_eq!(
        oracle.prompts(),
        [
            "argument prompt",
            "literal value stays literal\n\nINPUT:\nRESULT 1:\nargument answer"
        ]
    );
}

#[test]
fn top_level_definitions_scope_over_later_forms_without_an_outer_group() {
    let source = r#"
      (~ investigate (topic)
        (-> topic "Investigate this topic in depth"))

      (investigate "fibonacci")
    "#;
    let oracle = ScriptedOracle::new(&[Some("seed"), Some("detailed report")]);
    let mut record = empty_record();
    let result = orchestrate(&parse(source).unwrap(), &mut record, &oracle);

    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    assert_eq!(result.output.as_deref(), Some("detailed report"));
    assert_eq!(
        oracle.prompts(),
        [
            "fibonacci",
            "Investigate this topic in depth\n\nINPUT:\nRESULT 1:\nseed"
        ]
    );
}

#[test]
fn inner_macro_parameter_shadows_outer_binding() {
    let source = r#"
      ((~ outer (x)
          '((~ inner (x) ',x)
            (inner "inner value")))
       (outer "outer value"))
    "#;
    let oracle = ScriptedOracle::new(&[Some("ok")]);
    let mut record = empty_record();
    let _ = orchestrate(&parse(source).unwrap(), &mut record, &oracle);
    assert_eq!(oracle.prompts(), ["inner value"]);
}

#[test]
fn higher_order_macro_can_receive_a_macro_as_an_argument() {
    let source = r#"
      ((~ apply (worker value) '(,worker ,value))
       (~ worker (value) '(-> ,value "verify"))
       (apply worker "implement"))
    "#;
    let oracle = ScriptedOracle::new(&[Some("patch"), Some("verified")]);
    let mut record = empty_record();
    let result = orchestrate(&parse(source).unwrap(), &mut record, &oracle);
    assert_eq!(oracle.prompts()[0], "implement");
    assert!(oracle.prompts()[1].starts_with("verify\n\nINPUT:"));
    assert_eq!(result.output.as_deref(), Some("verified"));
}

#[test]
fn unknown_and_wrong_arity_calls_are_typed_diagnostics() {
    let cases = [
        (
            "(missing \"must not fire\")",
            RuntimeDiagnostic::UndefinedMacro {
                name: "missing".into(),
            },
        ),
        (
            "((~ one (x) ,x) (one \"a\" \"must not fire\"))",
            RuntimeDiagnostic::ArityMismatch {
                name: "one".into(),
                expected: 1,
                actual: 2,
            },
        ),
    ];
    for (source, expected) in cases {
        let oracle = ScriptedOracle::new(&[Some("unexpected")]);
        let mut record = empty_record();
        let result = orchestrate(&parse(source).unwrap(), &mut record, &oracle);
        assert!(oracle.prompts().is_empty(), "source: {source}");
        assert!(result.firings.is_empty());
        assert_eq!(result.output, None);
        assert_eq!(result.diagnostics, [expected]);
    }
}

#[test]
fn runaway_self_recursive_macro_stops_at_the_expansion_budget() {
    let oracle = ScriptedOracle::default();
    let mut record = empty_record();
    let result = orchestrate(
        &parse("((~ forever () '(forever)) (forever))").unwrap(),
        &mut record,
        &oracle,
    );
    assert!(result.firings.is_empty());
    assert!(oracle.prompts().is_empty());
    assert_eq!(result.output, None);
    assert_eq!(
        result.diagnostics,
        [RuntimeDiagnostic::ExpansionLimit {
            name: "forever".into(),
            limit: 256,
        }]
    );
}

#[test]
fn conditional_square_is_lazy_case_insensitive_and_returns_selected_branch() {
    for (decision, selected, rejected) in [
        (" YES ", "yes branch", "no branch"),
        ("no", "no branch", "yes branch"),
    ] {
        let oracle = ScriptedOracle::new(&[Some(decision), Some("selected output")]);
        let mut record = empty_record();
        let source = r#"
          ((~ decide () "choose")
           ([(decide)] "yes branch" "no branch"))
        "#;
        let result = orchestrate(&parse(source).unwrap(), &mut record, &oracle);
        assert_eq!(oracle.prompts(), ["choose", selected]);
        assert!(!oracle.prompts().iter().any(|prompt| prompt == rejected));
        assert_eq!(result.output.as_deref(), Some("selected output"));
    }
}

#[test]
fn invalid_conditional_decision_runs_neither_branch() {
    let oracle = ScriptedOracle::new(&[Some("maybe"), Some("must not be used")]);
    let mut record = empty_record();
    let source = r#"
      ((~ decide () "choose")
       ([(decide)] "yes branch" "no branch"))
    "#;
    let result = orchestrate(&parse(source).unwrap(), &mut record, &oracle);
    assert_eq!(oracle.prompts(), ["choose"]);
    assert_eq!(result.output, None);
    assert_eq!(
        result.diagnostics,
        [RuntimeDiagnostic::InvalidCondition {
            value: Some("maybe".into()),
        }]
    );
}

#[test]
fn observer_receives_live_events_before_the_final_result_exists() {
    let oracle = ScriptedOracle::new(&[Some("source"), Some("result")]);
    let mut record = empty_record();
    let mut observed = Vec::new();
    let result = orchestrate_with_observer(
        &parse("(-> \"produce\" \"consume\")").unwrap(),
        &mut record,
        &oracle,
        &mut |event| observed.push(event.clone()),
    );

    assert!(matches!(observed[0], ExecutionEvent::PromptStarted { .. }));
    assert!(observed
        .iter()
        .any(|event| matches!(event, ExecutionEvent::FlowRouted { .. })));
    assert_eq!(observed, result.events);
    assert!(result.diagnostics.is_empty());
}

// Visualization contracts ---------------------------------------------------

#[test]
fn every_renderer_handles_all_structural_forms() {
    let expr = parse(
        r#"((# tools) (~ route (x) '(-> ,x "done")) (["merge"] (route "left") (<- "a" "b")))"#,
    )
    .unwrap();
    let record = Record::from_texts(&["left merge", "done merge", "a b"]);
    let syntax_tree = tree(&expr);
    let scored = tree_scored(&expr, &record);
    let circuit = mandala(&expr);

    assert!(syntax_tree.contains("λ function route"));
    assert!(syntax_tree.contains("⇲ import tools"));
    assert!(syntax_tree.contains("□ mediator square"));
    assert!(syntax_tree.contains("→ forward"));
    assert!(syntax_tree.contains("← backflow"));
    assert!(scored.lines().all(|line| line.contains("line(s)")));
    assert!(circuit.starts_with("o-[]-o-[]-o\n"));
    assert!(circuit.contains("~[route(x)]"));
    assert!(circuit.contains("⇲[tools]"));
    assert!(circuit.contains("[M:"));
}
