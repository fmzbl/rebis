//! The embedded standard library: build gate, namespace reservation, and
//! execution of every module under a scripted oracle.

use std::cell::RefCell;
use std::collections::VecDeque;

use rebis_lang::{
    orchestrate, orchestrate_with_runtime, parse, std_modules, ExecutionEvent, ModuleName,
    ModuleResolver, Oracle, Orchestration, Record, RuntimeDiagnostic,
};

#[derive(Default)]
struct Scripted {
    answers: RefCell<VecDeque<Option<String>>>,
    prompts: RefCell<Vec<String>>,
}

impl Scripted {
    fn new(answers: &[&str]) -> Self {
        Self {
            answers: RefCell::new(answers.iter().map(|a| Some((*a).to_string())).collect()),
            prompts: RefCell::new(Vec::new()),
        }
    }

    fn calls(&self) -> usize {
        self.prompts.borrow().len()
    }
}

impl Oracle for Scripted {
    fn fire(&self, prompt: &str) -> Option<String> {
        self.prompts.borrow_mut().push(prompt.to_string());
        self.answers.borrow_mut().pop_front().flatten()
    }
}

fn run(source: &str, answers: &[&str]) -> (Orchestration, Scripted) {
    let oracle = Scripted::new(answers);
    let mut record = Record::from_texts::<&str>(&[]);
    let result = orchestrate(&parse(source).expect("parse"), &mut record, &oracle);
    (result, oracle)
}

// ── build gate ────────────────────────────────────────────────────

#[test]
fn every_embedded_module_parses_and_imports_cleanly() {
    for (name, source) in std_modules() {
        parse(source).unwrap_or_else(|error| panic!("{name}: {error}"));
        let (result, _) = run(&format!("((# {name}) \"probe\")"), &["ok"]);
        assert!(
            result.diagnostics.is_empty(),
            "{name}: {:?}",
            result.diagnostics
        );
        assert!(result.events.iter().any(|event| matches!(
            event,
            ExecutionEvent::ModuleLoaded { module, .. } if module.as_str() == *name
        )));
    }
}

#[test]
fn the_inventory_is_fourteen_modules_and_fifty_one_macros() {
    assert_eq!(std_modules().len(), 14);
    let macros: usize = std_modules()
        .iter()
        .map(|(_, source)| source.matches("(~ ").count())
        .sum();
    assert_eq!(macros, 51);
}

#[test]
fn structural_modules_contain_no_prompt_text() {
    for (name, source) in std_modules() {
        if matches!(*name, "std/canon" | "std/shape") {
            continue;
        }
        assert!(
            !source.contains('"'),
            "{name} must stay prompt-free (principle 2.1)"
        );
    }
}

// ── namespace reservation ─────────────────────────────────────────

struct Hijacker;

impl ModuleResolver for Hijacker {
    fn resolve(&self, module: &ModuleName) -> Result<Option<String>, String> {
        // Claims EVERY name, including std/* — the library must win.
        let _ = module;
        Ok(Some("(~ hijacked (x) ',x)".to_string()))
    }
}

#[test]
fn std_names_resolve_from_the_crate_not_the_host() {
    let oracle = Scripted::new(&["ok"]);
    let mut record = Record::from_texts::<&str>(&[]);
    let result = orchestrate_with_runtime(
        &parse(r#"((# std/flow) (twice "probe"))"#).unwrap(),
        &mut record,
        &oracle,
        &Hijacker,
        &mut |_| {},
    );
    // Host's hijack would leave `twice` undefined; embedded std defines it.
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
}

#[test]
fn importing_std_loads_the_whole_standard_library_folder() {
    let (result, oracle) = run("((# std) (twice \"probe\"))", &["one", "two"]);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    assert_eq!(oracle.calls(), 2);
    assert!(result.events.iter().any(|event| matches!(
        event,
        ExecutionEvent::ModuleLoaded { module, definitions }
            if module.as_str() == "std" && *definitions == 51
    )));
}

#[test]
fn unknown_std_names_never_fall_through_to_the_host() {
    let oracle = Scripted::new(&[]);
    let mut record = Record::from_texts::<&str>(&[]);
    let result = orchestrate_with_runtime(
        &parse("((# std/absent) \"probe\")").unwrap(),
        &mut record,
        &oracle,
        &Hijacker,
        &mut |_| {},
    );
    assert!(result
        .diagnostics
        .iter()
        .any(|d| matches!(d, RuntimeDiagnostic::ModuleNotFound { module } if module.as_str() == "std/absent")));
}

// ── execution: kernel tier ────────────────────────────────────────

#[test]
fn flow_compose_chains_strategies() {
    let source = r#"(
      (# std/flow)
      (~ shout (v) '(-> ,v "upper"))
      (compose twice shout "start"))"#;
    let (result, oracle) = run(source, &["a", "b", "c", "d"]);
    assert!(result.diagnostics.is_empty());
    // shout = 2 calls (value + upper); twice routes it through itself: +2.
    assert_eq!(oracle.calls(), 4);
}

#[test]
fn spread_best_of_three_with_symbol_judge_is_promptless() {
    let source =
        r#"((# std/spread) (best-of-three parser-benchmark "measure the parser benchmark"))"#;
    let (result, oracle) = run(
        source,
        &[
            "the parser benchmark improved",
            "bananas",
            "parser benchmark holds steady",
        ],
    );
    assert_eq!(oracle.calls(), 3, "k calls, zero judging calls");
    assert_eq!(
        result.output.as_deref(),
        Some("the parser benchmark improved")
    );
    assert!(result
        .events
        .iter()
        .any(|e| matches!(e, ExecutionEvent::MediatorResolved { result: 1, .. })));
}

#[test]
fn gate_refuses_off_topic_and_or_else_prefers_the_primary() {
    let gate = r#"((# std/gate) (gate parser-benchmark "answer"))"#;
    let (result, _) = run(gate, &["bananas are yellow"]);
    assert_eq!(result.output, None, "off-topic answer is refused");

    let or_else = r#"((# std/gate) (or-else parser-benchmark "primary" "fallback"))"#;
    let (result, _) = run(
        or_else,
        &["parser benchmark first", "parser benchmark second"],
    );
    assert_eq!(
        result.output.as_deref(),
        Some("parser benchmark first"),
        "ties prefer source order"
    );
}

#[test]
fn loops_terminate_when_the_stop_macro_answers_yes() {
    let source = r#"(
      (# std/loops)
      (~ step (v) '(-> ,v "improve"))
      (~ done (v) '(-> ,v "done? answer exactly yes or no"))
      (loop "seed" step done))"#;
    // Structural substitution re-executes spliced arguments: round 1 checks
    // done("seed") (2 calls, no); round 2 checks done((step "seed")) — the
    // step runs inside the check (3 calls, yes) — and the selected branch
    // re-executes (step "seed") (2 calls).
    let (result, _) = run(source, &["s1", "no", "s2", "i1", "yes", "s3", "final"]);
    assert!(result
        .events
        .iter()
        .any(|e| matches!(e, ExecutionEvent::BranchSelected { .. })));
    assert!(result.diagnostics.is_empty());
}

// ── execution: strategy tier ──────────────────────────────────────

#[test]
fn evolve_keeps_the_better_candidate_deterministically() {
    let source = r#"(
      (# std/evolve)
      (~ improve (v) '(-> ,v "improve this"))
      (evolve tested-plan improve "write the plan"))"#;
    let (result, oracle) = run(
        source,
        &["a rough draft", "the draft", "a tested plan with stages"],
    );
    assert_eq!(oracle.calls(), 3);
    assert_eq!(result.output.as_deref(), Some("a tested plan with stages"));
}

#[test]
fn debate_and_canon_compose() {
    let source = r#"(
      (# std/debate) (# std/canon)
      (~ pro (c) '(-> ,c "defend the claim"))
      (debate "Pick the stronger argument" pro steelman "the gate holds"))"#;
    let (result, oracle) = run(source, &["c1", "defense", "c2", "opposite", "the verdict"]);
    assert!(result.diagnostics.is_empty());
    assert_eq!(result.output.as_deref(), Some("the verdict"));
    assert_eq!(oracle.calls(), 5);
}

#[test]
fn shape_wrappers_append_their_contract() {
    let source = r#"((# std/shape) (one-word "name the color"))"#;
    let (_, oracle) = run(source, &["blue-ish", "blue"]);
    let prompts = oracle.prompts.borrow();
    assert!(prompts[1].starts_with("Answer with exactly one word."));
}

// ── execution: search & control tier ─────────────────────────────

#[test]
fn route_two_is_lazy_and_only_the_chosen_specialist_runs() {
    let source = r#"(
      (# std/search)
      (~ kind-a (t) '(-> ,t "is it kind A? answer exactly yes or no"))
      (~ a (t) '(-> ,t "handle as A"))
      (~ b (t) '(-> ,t "handle as B"))
      (route-two kind-a a b "the task"))"#;
    let (result, oracle) = run(source, &["t", "no", "t2", "handled by B"]);
    assert!(result.diagnostics.is_empty());
    let prompts = oracle.prompts.borrow();
    assert!(
        !prompts.iter().any(|p| p.starts_with("handle as A")),
        "unselected specialist must never fire"
    );
    assert!(prompts.iter().any(|p| p.starts_with("handle as B")));
}

#[test]
fn tournament_reexports_spread_and_runs_the_bracket() {
    let source =
        r#"((# std/tournament) (tournament-four quality-check "attempt the quality check"))"#;
    let (result, oracle) = run(
        source,
        &[
            "quality check attempt one",
            "off topic",
            "quality check attempt three",
            "noise",
        ],
    );
    assert_eq!(oracle.calls(), 4, "four attempts, zero judge calls");
    assert!(result.diagnostics.is_empty());
    // Three deterministic mediations: two semifinals and the final.
    let resolved = result
        .events
        .iter()
        .filter(|e| matches!(e, ExecutionEvent::MediatorResolved { .. }))
        .count();
    assert_eq!(resolved, 3);
}

#[test]
fn reflexion_retries_with_the_critique_as_input() {
    let source = r#"(
      (# std/reflexion)
      (~ worker (t) '(-> ,t "attempt the task"))
      (~ critic (a) '(-> ,a "critique the attempt"))
      (reflexion worker critic "the task"))"#;
    // The critic's argument is spliced syntax, so the worker executes once
    // inside the critique flow (2 calls) + critique (1) + the retry (2).
    let (result, oracle) = run(
        source,
        &["t", "attempt one", "too vague", "t", "attempt two"],
    );
    assert!(result.diagnostics.is_empty());
    let prompts = oracle.prompts.borrow();
    // The retry's task stage receives the critique as INPUT.
    let retry_task = &prompts[3];
    assert!(retry_task.starts_with("the task"));
    assert!(retry_task.contains("too vague"));
    assert_eq!(result.output.as_deref(), Some("attempt two"));
}

#[test]
fn chaired_panel_routes_the_chairs_criteria_into_every_branch() {
    let source = r#"(
      (# std/committee)
      (~ chair (t) '(-> ,t "set the judging criteria"))
      (~ p1 (t) '(-> ,t "answer as engineer"))
      (~ p2 (t) '(-> ,t "answer as operator"))
      (~ p3 (t) '(-> ,t "answer as customer"))
      (chaired-panel chair "Weigh the three answers" p1 p2 p3 "the question"))"#;
    let (result, oracle) = run(
        source,
        &["q", "THE CRITERIA", "q", "e", "q", "o", "q", "c", "verdict"],
    );
    assert!(result.diagnostics.is_empty());
    let prompts = oracle.prompts.borrow();
    // The criteria enter each branch at its first stage (the branch's own
    // task prompt receives the arrow's routed value as INPUT) — three
    // panelists, three deliveries beyond the chair's own firings.
    let deliveries = prompts
        .iter()
        .filter(|p| p.starts_with("the question") && p.contains("THE CRITERIA"))
        .count();
    assert_eq!(deliveries, 3, "criteria must reach every panelist branch");
    for role in [
        "answer as engineer",
        "answer as operator",
        "answer as customer",
    ] {
        assert!(
            prompts.iter().any(|p| p.starts_with(role)),
            "{role} never fired"
        );
    }
    assert_eq!(result.output.as_deref(), Some("verdict"));
}
