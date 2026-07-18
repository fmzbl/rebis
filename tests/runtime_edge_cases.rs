//! Orchestrator execution, limits, macro, routing, and event edge cases.

mod support;

use rebis_lang::{
    orchestrate, orchestrate_with_limits, orchestrate_with_observer, parse, ExecutionEvent,
    FlowDirection, RuntimeDiagnostic, RuntimeLimits,
};
use support::{empty_record, MemoryModules, ScriptedOracle};

#[test]
fn zero_model_budget_is_completely_model_silent() {
    let oracle = ScriptedOracle::answers(&[Some("must not be used")]);
    let mut record = empty_record();
    let result = orchestrate_with_limits(
        &parse(r#"("one" "two")"#).unwrap(),
        &mut record,
        &oracle,
        &MemoryModules::default(),
        RuntimeLimits::standard().with_model_calls(0),
        &mut |_| {},
    );

    assert!(oracle.prompts().is_empty());
    assert!(result.firings.is_empty());
    assert!(record.is_empty());
    assert_eq!(
        result.diagnostics,
        [RuntimeDiagnostic::ModelCallLimit { limit: 0 }]
    );
    assert_eq!(
        result.events,
        [ExecutionEvent::Diagnostic(
            RuntimeDiagnostic::ModelCallLimit { limit: 0 }
        )]
    );
}

#[test]
fn model_limit_is_reported_once_while_structural_execution_continues() {
    let oracle = ScriptedOracle::answers(&[Some("first")]);
    let mut record = empty_record();
    let result = orchestrate_with_limits(
        &parse(r#"(-> ("one" "two") "three")"#).unwrap(),
        &mut record,
        &oracle,
        &MemoryModules::default(),
        RuntimeLimits::standard().with_model_calls(1),
        &mut |_| {},
    );

    assert_eq!(oracle.prompts(), ["one"]);
    assert_eq!(result.firings.len(), 1);
    assert_eq!(result.diagnostics.len(), 1);
    assert!(result.events.iter().any(|event| matches!(
        event,
        ExecutionEvent::FlowRouted {
            direction: FlowDirection::Forward,
            value
        } if value == "RESULT 1:\nfirst"
    )));
}

#[test]
fn exact_macro_expansion_budget_succeeds_and_next_expansion_fails() {
    let source = r#"
      ((~ id (x) ',x)
       (id "first")
       (id "blocked"))
    "#;
    let oracle = ScriptedOracle::answers(&[Some("ok")]);
    let mut record = empty_record();
    let result = orchestrate_with_limits(
        &parse(source).unwrap(),
        &mut record,
        &oracle,
        &MemoryModules::default(),
        RuntimeLimits::standard().with_macro_expansions(1),
        &mut |_| {},
    );

    assert_eq!(oracle.prompts(), ["first"]);
    assert_eq!(
        result.diagnostics,
        [RuntimeDiagnostic::ExpansionLimit {
            name: "id".to_string(),
            limit: 1,
        }]
    );
    assert!(matches!(
        result.events.first(),
        Some(ExecutionEvent::MacroExpanded { name, remaining: 0 }) if name == "id"
    ));
}

#[test]
fn zero_macro_budget_prevents_body_side_effects() {
    let oracle = ScriptedOracle::answers(&[Some("must not fire")]);
    let mut record = empty_record();
    let result = orchestrate_with_limits(
        &parse(r#"((~ work () "side effect") (work))"#).unwrap(),
        &mut record,
        &oracle,
        &MemoryModules::default(),
        RuntimeLimits::standard().with_macro_expansions(0),
        &mut |_| {},
    );
    assert!(oracle.prompts().is_empty());
    assert_eq!(
        result.diagnostics,
        [RuntimeDiagnostic::ExpansionLimit {
            name: "work".to_string(),
            limit: 0,
        }]
    );
}

#[test]
fn every_provider_failure_has_start_diagnostic_finish_order() {
    let oracle = ScriptedOracle::results(vec![
        Err("timeout".to_string()),
        Err("connection reset".to_string()),
    ]);
    let mut record = empty_record();
    let result = orchestrate(
        &parse(r#"("first" "second")"#).unwrap(),
        &mut record,
        &oracle,
    );

    assert_eq!(result.firings.len(), 2);
    assert!(result.firings.iter().all(|firing| firing.answer.is_none()));
    assert!(record.is_empty());
    assert_eq!(result.events.len(), 6);
    for events in result.events.chunks_exact(3) {
        assert!(matches!(events[0], ExecutionEvent::PromptStarted { .. }));
        assert!(matches!(events[1], ExecutionEvent::Diagnostic(_)));
        assert!(matches!(events[2], ExecutionEvent::PromptFinished(_)));
    }
}

#[test]
fn declined_and_nothing_answers_are_traced_but_not_routed() {
    for first_answer in [None, Some("nothing"), Some("  NoThInG\n")] {
        let oracle = ScriptedOracle::answers(&[first_answer, Some("done")]);
        let mut record = empty_record();
        let result = orchestrate(
            &parse(r#"(-> "producer" "consumer")"#).unwrap(),
            &mut record,
            &oracle,
        );
        assert_eq!(oracle.prompts(), ["producer", "consumer\n\nINPUT:\n"]);
        assert_eq!(result.output.as_deref(), Some("done"));
        assert_eq!(record.len(), 1);
    }
}

#[test]
fn empty_answer_remains_a_real_structural_value() {
    let oracle = ScriptedOracle::answers(&[Some(""), Some("consumed")]);
    let mut record = empty_record();
    let result = orchestrate(
        &parse(r#"(-> "producer" "consumer")"#).unwrap(),
        &mut record,
        &oracle,
    );
    assert_eq!(
        oracle.prompts(),
        ["producer", "consumer\n\nINPUT:\nRESULT 1:\n"]
    );
    assert_eq!(result.output.as_deref(), Some("consumed"));
    assert_eq!(record.len(), 1);
}

#[test]
fn nested_forward_flow_routes_every_answer_from_the_producer_subgraph() {
    let oracle = ScriptedOracle::answers(&[Some("A"), Some("B"), Some("C")]);
    let mut record = empty_record();
    let result = orchestrate(
        &parse(r#"(-> (-> "a" "b") "c")"#).unwrap(),
        &mut record,
        &oracle,
    );
    assert_eq!(oracle.prompts()[0], "a");
    assert_eq!(oracle.prompts()[1], "b\n\nINPUT:\nRESULT 1:\nA");
    assert_eq!(
        oracle.prompts()[2],
        "c\n\nINPUT:\nRESULT 1:\nA\n\nRESULT 2:\nB"
    );
    assert_eq!(result.output.as_deref(), Some("C"));
}

#[test]
fn nested_backflow_executes_producers_from_right_to_left() {
    let oracle = ScriptedOracle::answers(&[Some("C"), Some("B"), Some("A")]);
    let mut record = empty_record();
    let result = orchestrate(
        &parse(r#"(<- "a" (<- "b" "c"))"#).unwrap(),
        &mut record,
        &oracle,
    );
    assert_eq!(oracle.prompts()[0], "c");
    assert_eq!(oracle.prompts()[1], "b\n\nINPUT:\nRESULT 1:\nC");
    assert_eq!(
        oracle.prompts()[2],
        "a\n\nINPUT:\nRESULT 1:\nC\n\nRESULT 2:\nB"
    );
    assert_eq!(result.output.as_deref(), Some("A"));
}

#[test]
fn ordinary_square_isolates_branch_reports_from_incoming_context() {
    let oracle = ScriptedOracle::answers(&[
        Some("upstream"),
        Some("left"),
        Some("right"),
        Some("merged"),
    ]);
    let mut record = empty_record();
    let result = orchestrate(
        &parse(r#"(-> "source" (["merge"] "left" "right"))"#).unwrap(),
        &mut record,
        &oracle,
    );
    let prompts = oracle.prompts();
    assert!(prompts[1].contains("RESULT 1:\nupstream"));
    assert!(prompts[2].contains("RESULT 1:\nupstream"));
    assert_eq!(
        prompts[3],
        "merge\n\nINPUT:\nRESULT 1:\nleft\n\nRESULT 2:\nright"
    );
    assert_eq!(result.output.as_deref(), Some("merged"));
}

#[test]
fn two_branch_call_mediator_is_lazy_but_other_squares_are_eager() {
    let lazy_source = r#"
      ((~ choose () "decision")
       ([(choose)] "yes branch" "no branch"))
    "#;
    let lazy_oracle = ScriptedOracle::answers(&[Some("yes"), Some("selected")]);
    let mut lazy_record = empty_record();
    let lazy = orchestrate(&parse(lazy_source).unwrap(), &mut lazy_record, &lazy_oracle);
    assert_eq!(lazy_oracle.prompts(), ["decision", "yes branch"]);
    assert!(lazy
        .events
        .contains(&ExecutionEvent::BranchSelected { decision: true }));

    let eager_source = r#"
      ((~ merge () "mediator")
       ([(merge)] "one" "two" "three"))
    "#;
    let eager_oracle = ScriptedOracle::answers(&[Some("1"), Some("2"), Some("3"), Some("merged")]);
    let mut eager_record = empty_record();
    let eager = orchestrate(
        &parse(eager_source).unwrap(),
        &mut eager_record,
        &eager_oracle,
    );
    assert_eq!(&eager_oracle.prompts()[..3], ["one", "two", "three"]);
    assert!(eager_oracle.prompts()[3].starts_with("mediator\n\nINPUT:"));
    assert!(!eager
        .events
        .iter()
        .any(|event| matches!(event, ExecutionEvent::BranchSelected { .. })));
}

#[test]
fn missing_conditional_value_reports_both_underlying_and_condition_errors() {
    let oracle = ScriptedOracle::default();
    let mut record = empty_record();
    let result = orchestrate(
        &parse(r#"([(undefined)] "yes" "no")"#).unwrap(),
        &mut record,
        &oracle,
    );
    assert_eq!(
        result.diagnostics,
        [
            RuntimeDiagnostic::UndefinedMacro {
                name: "undefined".to_string()
            },
            RuntimeDiagnostic::InvalidCondition { value: None },
        ]
    );
    assert!(oracle.prompts().is_empty());
}

#[test]
fn unquoted_macro_body_substitutes_a_whole_expression() {
    let oracle = ScriptedOracle::answers(&[Some("answer")]);
    let mut record = empty_record();
    let result = orchestrate(
        &parse(r#"((~ identity (x) x) (identity ("nested prompt")))"#).unwrap(),
        &mut record,
        &oracle,
    );
    assert_eq!(oracle.prompts(), ["nested prompt"]);
    assert_eq!(result.output.as_deref(), Some("answer"));
}

#[test]
fn a_non_symbol_higher_order_argument_expands_as_data_not_as_a_call_head() {
    let source = r#"
      ((~ apply (worker x) '(,worker ,x))
       (apply "not a macro name" "payload"))
    "#;
    let oracle = ScriptedOracle::answers(&[Some("first"), Some("second")]);
    let mut record = empty_record();
    let result = orchestrate(&parse(source).unwrap(), &mut record, &oracle);
    assert_eq!(oracle.prompts(), ["not a macro name", "payload"]);
    assert_eq!(result.output.as_deref(), Some("second"));
    assert!(result.diagnostics.is_empty());
}

#[test]
fn definitions_are_lexical_and_later_definitions_shadow_earlier_ones() {
    let source = r#"
      ((~ value () "outer-old")
       (~ value () "outer-new")
       ((~ value () "inner") (value))
       (value))
    "#;
    let oracle = ScriptedOracle::answers(&[Some("inner answer"), Some("outer answer")]);
    let mut record = empty_record();
    let result = orchestrate(&parse(source).unwrap(), &mut record, &oracle);
    assert_eq!(oracle.prompts(), ["inner", "outer-new"]);
    assert_eq!(result.output.as_deref(), Some("outer answer"));
}

#[test]
fn inner_definition_does_not_leak_to_a_later_sibling() {
    let source = r#"(((~ private () "inside") (private)) (private))"#;
    let oracle = ScriptedOracle::answers(&[Some("inside answer")]);
    let mut record = empty_record();
    let result = orchestrate(&parse(source).unwrap(), &mut record, &oracle);
    assert_eq!(oracle.prompts(), ["inside"]);
    assert_eq!(
        result.diagnostics,
        [RuntimeDiagnostic::UndefinedMacro {
            name: "private".to_string()
        }]
    );
}

#[test]
fn observer_event_stream_is_identical_to_the_retained_trace() {
    let source = r#"
      ((~ route (x) '(-> ,x "consume"))
       (route "produce"))
    "#;
    let oracle = ScriptedOracle::answers(&[Some("value"), Some("done")]);
    let mut record = empty_record();
    let mut observed = Vec::new();
    let result = orchestrate_with_observer(
        &parse(source).unwrap(),
        &mut record,
        &oracle,
        &mut |event| observed.push(event.clone()),
    );
    assert_eq!(observed, result.events);
    assert!(matches!(
        observed.as_slice(),
        [
            ExecutionEvent::MacroExpanded { .. },
            ExecutionEvent::PromptStarted { .. },
            ExecutionEvent::PromptFinished(_),
            ExecutionEvent::FlowRouted { .. },
            ExecutionEvent::PromptStarted { .. },
            ExecutionEvent::PromptFinished(_),
        ]
    ));
}

#[test]
fn every_diagnostic_is_mirrored_once_in_the_event_stream() {
    let oracle = ScriptedOracle::results(vec![Err("offline".to_string())]);
    let mut record = empty_record();
    let result = orchestrate(
        &parse(r#"("fails" (missing))"#).unwrap(),
        &mut record,
        &oracle,
    );
    let event_diagnostics: Vec<_> = result
        .events
        .iter()
        .filter_map(|event| match event {
            ExecutionEvent::Diagnostic(diagnostic) => Some(diagnostic.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(event_diagnostics, result.diagnostics);
}
