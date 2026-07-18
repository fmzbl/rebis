//! Concurrent square-branch evaluation: determinism, isolation, and budgets.
//!
//! Square branches are semantically unordered and mutually isolated, so
//! `orchestrate_parallel` may evaluate them concurrently. These tests pin the
//! contract: source-order structure regardless of completion order,
//! byte-identical traces at `max_concurrency: 1`, branch-scoped definitions,
//! an untouched lazy conditional, and one shared model-call budget.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use rebis_lang::{
    orchestrate_parallel, orchestrate_with_limits, parse, ExecutionEvent, ModuleName,
    ModuleResolver, Oracle, Orchestration, Record, RuntimeLimits,
};

/// Thread-safe scripted oracle: answers keyed by prompt head, with an
/// optional per-prompt delay schedule so completion order can be forced to
/// differ from source order.
struct KeyedOracle {
    answers: BTreeMap<&'static str, &'static str>,
    delays: BTreeMap<&'static str, u64>,
    calls: AtomicUsize,
    live: AtomicUsize,
    peak: AtomicUsize,
    prompts: Mutex<Vec<String>>,
}

impl KeyedOracle {
    fn new(answers: &[(&'static str, &'static str)]) -> Self {
        Self {
            answers: answers.iter().copied().collect(),
            delays: BTreeMap::new(),
            calls: AtomicUsize::new(0),
            live: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            prompts: Mutex::new(Vec::new()),
        }
    }

    fn with_delays(mut self, delays: &[(&'static str, u64)]) -> Self {
        self.delays = delays.iter().copied().collect();
        self
    }

    fn lookup(&self, prompt: &str) -> Option<&'static str> {
        let head = prompt.lines().next().unwrap_or_default();
        self.answers.get(head).copied()
    }
}

impl Oracle for KeyedOracle {
    fn fire(&self, prompt: &str) -> Option<String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let live = self.live.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(live, Ordering::SeqCst);
        self.prompts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(prompt.to_string());
        let head = prompt.lines().next().unwrap_or_default();
        if let Some(ms) = self.delays.get(head) {
            std::thread::sleep(Duration::from_millis(*ms));
        }
        let answer = self.lookup(prompt).map(str::to_string);
        self.live.fetch_sub(1, Ordering::SeqCst);
        answer
    }
}

struct NoModules;

impl ModuleResolver for NoModules {
    fn resolve(&self, _module: &ModuleName) -> Result<Option<String>, String> {
        Ok(None)
    }
}

fn shape(run: &Orchestration) -> (Vec<String>, Vec<String>, Option<String>) {
    let firings = run
        .firings
        .iter()
        .map(|firing| {
            format!(
                "{} =] {}",
                firing.prompt.replace('\n', "\\n"),
                firing.answer.as_deref().unwrap_or("-")
            )
        })
        .collect();
    let events = run
        .events
        .iter()
        .map(|event| format!("{event:?}"))
        .collect();
    (firings, events, run.output.clone())
}

const SQUARE: &str = r#"(["judge"] "alpha" "beta" "gamma")"#;

fn square_oracle() -> KeyedOracle {
    KeyedOracle::new(&[
        ("alpha", "answer-a"),
        ("beta", "answer-b"),
        ("gamma", "answer-c"),
        ("judge", "verdict"),
    ])
}

#[test]
fn parallel_square_matches_sequential_structure_exactly() {
    let expr = parse(SQUARE).expect("parse");
    let limits = RuntimeLimits::standard();

    let sequential = {
        let oracle = square_oracle();
        let mut record = Record::from_texts::<&str>(&[]);
        orchestrate_with_limits(&expr, &mut record, &oracle, &NoModules, limits, &mut |_| {})
    };
    // Delays reverse the completion order: gamma finishes first, alpha last.
    let oracle = square_oracle().with_delays(&[("alpha", 60), ("beta", 30), ("gamma", 1)]);
    let mut record = Record::from_texts::<&str>(&[]);
    let parallel =
        orchestrate_parallel(&expr, &mut record, &oracle, &NoModules, limits, &mut |_| {});

    assert_eq!(shape(&parallel), shape(&sequential));
    // The judge must see RESULT 1..3 in source order, not completion order.
    let judge = parallel
        .firings
        .iter()
        .find(|firing| firing.prompt.starts_with("judge"))
        .expect("judge fired");
    let expected =
        "judge\n\nINPUT:\nRESULT 1:\nanswer-a\n\nRESULT 2:\nanswer-b\n\nRESULT 3:\nanswer-c";
    assert_eq!(judge.prompt, expected);
}

#[test]
fn branches_actually_overlap_and_respect_the_concurrency_bound() {
    let expr = parse(SQUARE).expect("parse");
    let oracle = square_oracle().with_delays(&[("alpha", 40), ("beta", 40), ("gamma", 40)]);
    let mut record = Record::from_texts::<&str>(&[]);
    let limits = RuntimeLimits::standard().with_max_concurrency(2);
    let run = orchestrate_parallel(&expr, &mut record, &oracle, &NoModules, limits, &mut |_| {});
    assert_eq!(run.diagnostics, vec![]);
    let peak = oracle.peak.load(Ordering::SeqCst);
    assert!(peak >= 2, "expected overlapping branch calls, peak {peak}");
    assert!(peak <= 2, "bound exceeded: peak {peak}");
}

#[test]
fn max_concurrency_one_is_byte_identical_to_sequential() {
    let expr = parse(SQUARE).expect("parse");
    let limits = RuntimeLimits::standard().with_max_concurrency(1);
    let sequential = {
        let oracle = square_oracle();
        let mut record = Record::from_texts::<&str>(&[]);
        orchestrate_with_limits(
            &expr,
            &mut record,
            &oracle,
            &NoModules,
            RuntimeLimits::standard(),
            &mut |_| {},
        )
    };
    let oracle = square_oracle();
    let mut record = Record::from_texts::<&str>(&[]);
    let parallel =
        orchestrate_parallel(&expr, &mut record, &oracle, &NoModules, limits, &mut |_| {});
    assert_eq!(shape(&parallel), shape(&sequential));
}

#[test]
fn definitions_are_scoped_to_their_branch() {
    // Branch 1 defines a macro; branch 2 calls it. Under branch isolation the
    // second branch must not see the first branch's definition, regardless of
    // scheduling. The failed call surfaces as an UndefinedMacro diagnostic.
    let source = r#"(["judge"]
      ((~ local (x) (-> ,x "refine")) "alpha")
      (local "beta"))"#;
    let expr = parse(source).expect("parse");
    let oracle = KeyedOracle::new(&[("alpha", "answer-a"), ("judge", "verdict")]);
    let mut record = Record::from_texts::<&str>(&[]);
    let run = orchestrate_parallel(
        &expr,
        &mut record,
        &oracle,
        &NoModules,
        RuntimeLimits::standard(),
        &mut |_| {},
    );
    assert!(
        run.diagnostics
            .iter()
            .any(|d| format!("{d:?}").contains("UndefinedMacro")),
        "expected UndefinedMacro, got {:?}",
        run.diagnostics
    );
}

#[test]
fn conditional_squares_stay_lazy_and_sequential() {
    // The two-branch square with a macro-call mediator must still evaluate
    // the condition first and only the selected branch — no concurrent
    // expansion of the untaken branch.
    let source = r#"(
      (~ check (v) (-> v "condition"))
      ([(check "seed")] "kept" "dropped"))"#;
    let expr = parse(source).expect("parse");
    let oracle = KeyedOracle::new(&[
        ("seed", "seed-out"),
        ("condition", "no"),
        ("kept", "kept-out"),
        ("dropped", "dropped-out"),
    ]);
    let mut record = Record::from_texts::<&str>(&[]);
    let run = orchestrate_parallel(
        &expr,
        &mut record,
        &oracle,
        &NoModules,
        RuntimeLimits::standard(),
        &mut |_| {},
    );
    let fired: Vec<&str> = run
        .firings
        .iter()
        .map(|firing| firing.prompt.lines().next().unwrap_or_default())
        .collect();
    assert!(fired.contains(&"dropped"), "no-branch selected: {fired:?}");
    assert!(
        !fired.contains(&"kept"),
        "unselected branch fired: {fired:?}"
    );
}

#[test]
fn model_call_budget_is_shared_across_branches() {
    let expr = parse(SQUARE).expect("parse");
    let oracle = square_oracle();
    let mut record = Record::from_texts::<&str>(&[]);
    let limits = RuntimeLimits::standard().with_model_calls(2);
    let run = orchestrate_parallel(&expr, &mut record, &oracle, &NoModules, limits, &mut |_| {});
    assert_eq!(oracle.calls.load(Ordering::SeqCst), 2, "budget exceeded");
    assert!(
        run.diagnostics
            .iter()
            .any(|d| format!("{d:?}").contains("ModelCallLimit")),
        "expected ModelCallLimit, got {:?}",
        run.diagnostics
    );
}

#[test]
fn observer_receives_branch_events_in_source_order() {
    let expr = parse(SQUARE).expect("parse");
    let oracle = square_oracle().with_delays(&[("alpha", 50), ("gamma", 1)]);
    let mut record = Record::from_texts::<&str>(&[]);
    let mut seen: Vec<String> = Vec::new();
    let run = orchestrate_parallel(
        &expr,
        &mut record,
        &oracle,
        &NoModules,
        RuntimeLimits::standard(),
        &mut |event: &ExecutionEvent| {
            if let ExecutionEvent::PromptStarted { prompt, .. } = event {
                seen.push(prompt.lines().next().unwrap_or_default().to_string());
            }
        },
    );
    assert_eq!(run.diagnostics, vec![]);
    let interesting: Vec<&str> = seen
        .iter()
        .map(String::as_str)
        .filter(|p| ["alpha", "beta", "gamma"].contains(p))
        .collect();
    assert_eq!(interesting, vec!["alpha", "beta", "gamma"]);
}
