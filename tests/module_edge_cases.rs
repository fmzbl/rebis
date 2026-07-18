//! Foundational `#` module graph, isolation, shadowing, and budget edge cases.

mod support;

use rebis_lang::{
    orchestrate_with_limits, orchestrate_with_runtime, parse, ExecutionEvent, ModuleName,
    RuntimeDiagnostic, RuntimeLimits,
};
use support::{empty_record, MemoryModules, ScriptedOracle};

fn run(
    source: &str,
    oracle: &ScriptedOracle,
    modules: &MemoryModules,
) -> rebis_lang::Orchestration {
    let mut record = empty_record();
    orchestrate_with_runtime(
        &parse(source).unwrap(),
        &mut record,
        oracle,
        modules,
        &mut |_| {},
    )
}

#[test]
fn direct_single_definition_module_is_valid() {
    let modules = MemoryModules::default().with("tool", r#"(~ run () "from module")"#);
    let oracle = ScriptedOracle::answers(&[Some("done")]);
    let result = run("((# tool) (run))", &oracle, &modules);
    assert_eq!(oracle.prompts(), ["from module"]);
    assert_eq!(result.output.as_deref(), Some("done"));
    assert!(result.diagnostics.is_empty());
}

#[test]
fn resolver_failure_is_distinct_from_a_missing_module() {
    let modules = MemoryModules::default().failing("remote", "permission denied");
    let result = run("(# remote)", &ScriptedOracle::default(), &modules);
    assert_eq!(
        result.diagnostics,
        [RuntimeDiagnostic::ModuleLoadFailure {
            module: ModuleName::try_from("remote").unwrap(),
            message: "permission denied".to_string(),
        }]
    );

    let missing = run(
        "(# absent)",
        &ScriptedOracle::default(),
        &MemoryModules::default(),
    );
    assert_eq!(
        missing.diagnostics,
        [RuntimeDiagnostic::ModuleNotFound {
            module: ModuleName::try_from("absent").unwrap(),
        }]
    );
}

#[test]
fn module_parse_errors_preserve_the_module_identity() {
    let modules = MemoryModules::default().with("broken", "(~ f (x) x");
    let result = run("(# broken)", &ScriptedOracle::default(), &modules);
    assert!(matches!(
        result.diagnostics.as_slice(),
        [RuntimeDiagnostic::InvalidModule { module, message }]
            if module.as_str() == "broken" && message.contains("exactly one body")
    ));
}

#[test]
fn every_executable_top_level_module_shape_is_rejected_without_firing() {
    for (name, module_source) in [
        ("prompt", r#""execute""#),
        ("symbol", "symbol"),
        ("call", "(call)"),
        ("arrow", r#"(-> "a" "b")"#),
        ("square", r#"(["m"] "b")"#),
        ("mixed", r#"((~ ok () "body") "execute")"#),
    ] {
        let modules = MemoryModules::default().with(name, module_source);
        let oracle = ScriptedOracle::answers(&[Some("must not be used")]);
        let result = run(&format!("(# {name})"), &oracle, &modules);
        assert!(oracle.prompts().is_empty(), "module {name}");
        assert!(matches!(
            result.diagnostics.as_slice(),
            [RuntimeDiagnostic::InvalidModule { module, .. }] if module.as_str() == name
        ));
    }
}

#[test]
fn diamond_reexports_load_each_distinct_module_once() {
    let modules = MemoryModules::default()
        .with("base", "(~ identity (x) ',x)")
        .with("left", "((# base) (~ left (x) '(identity ,x)))")
        .with("right", "((# base) (~ right (x) '(identity ,x)))")
        .with("top", "((# left) (# right))");
    let oracle = ScriptedOracle::answers(&[Some("done")]);
    let result = run(r#"((# top) (left "work"))"#, &oracle, &modules);

    assert_eq!(oracle.prompts(), ["work"]);
    assert!(result.diagnostics.is_empty());
    let requests = modules.requests();
    for name in ["top", "left", "right", "base"] {
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.as_str() == name)
                .count(),
            1,
            "requests: {requests:?}"
        );
    }
}

#[test]
fn repeated_root_import_uses_cache_but_emits_each_lexical_load() {
    let modules = MemoryModules::default().with("tool", "(~ tool () 'ready)");
    let result = run("((# tool) (# tool))", &ScriptedOracle::default(), &modules);
    assert_eq!(modules.requests(), ["tool"]);
    let loaded: Vec<_> = result
        .events
        .iter()
        .filter(|event| matches!(event, ExecutionEvent::ModuleLoaded { .. }))
        .collect();
    assert_eq!(loaded.len(), 2);
}

#[test]
fn imported_and_local_shadowing_obeys_source_order() {
    let modules = MemoryModules::default().with("values", r#"(~ value () "module")"#);

    let local_last = ScriptedOracle::answers(&[Some("done")]);
    let result = run(
        r#"((# values) (~ value () "local") (value))"#,
        &local_last,
        &modules,
    );
    assert_eq!(local_last.prompts(), ["local"]);
    assert!(result.diagnostics.is_empty());

    let import_last = ScriptedOracle::answers(&[Some("done")]);
    let result = run(
        r#"((~ value () "local") (# values) (value))"#,
        &import_last,
        &modules,
    );
    assert_eq!(import_last.prompts(), ["module"]);
    assert!(result.diagnostics.is_empty());
}

#[test]
fn module_internal_shadowing_obeys_source_order() {
    let modules = MemoryModules::default()
        .with("old", r#"(~ value () "old")"#)
        .with(
            "new",
            r#"((# old) (~ value () "new") (~ expose () '(value)))"#,
        );
    let oracle = ScriptedOracle::answers(&[Some("done")]);
    let result = run("((# new) (expose))", &oracle, &modules);
    assert_eq!(oracle.prompts(), ["new"]);
    assert!(result.diagnostics.is_empty());
}

#[test]
fn exact_module_budget_allows_chain_and_next_distinct_load_fails() {
    let modules = MemoryModules::default()
        .with("a", "(# b)")
        .with("b", "(~ b () 'ok)")
        .with("c", "(~ c () 'ok)");
    let oracle = ScriptedOracle::default();
    let mut record = empty_record();
    let result = orchestrate_with_limits(
        &parse("((# a) (# c))").unwrap(),
        &mut record,
        &oracle,
        &modules,
        RuntimeLimits::standard().with_module_imports(2),
        &mut |_| {},
    );
    assert_eq!(modules.requests(), ["a", "b"]);
    assert_eq!(
        result.diagnostics,
        [RuntimeDiagnostic::ModuleLimit { limit: 2 }]
    );
    assert!(result.events.iter().any(|event| matches!(
        event,
        ExecutionEvent::ModuleLoaded { module, definitions: 1 } if module.as_str() == "a"
    )));
}

#[test]
fn cached_imports_do_not_consume_additional_module_budget() {
    let modules = MemoryModules::default().with("only", "(~ work () 'ok)");
    let mut record = empty_record();
    let result = orchestrate_with_limits(
        &parse("((# only) (# only) (# only))").unwrap(),
        &mut record,
        &ScriptedOracle::default(),
        &modules,
        RuntimeLimits::standard().with_module_imports(1),
        &mut |_| {},
    );
    assert_eq!(modules.requests(), ["only"]);
    assert!(result.diagnostics.is_empty());
}

#[test]
fn self_cycle_and_deep_cycle_report_the_minimal_ordered_cycle() {
    let self_cycle = MemoryModules::default().with("self", "(# self)");
    let result = run("(# self)", &ScriptedOracle::default(), &self_cycle);
    assert!(matches!(
        result.diagnostics.as_slice(),
        [RuntimeDiagnostic::ImportCycle { modules }]
            if modules.iter().map(ModuleName::as_str).collect::<Vec<_>>() == ["self", "self"]
    ));

    let deep_cycle = MemoryModules::default()
        .with("entry", "(# a)")
        .with("a", "(# b)")
        .with("b", "(# a)");
    let result = run("(# entry)", &ScriptedOracle::default(), &deep_cycle);
    assert!(matches!(
        result.diagnostics.as_slice(),
        [RuntimeDiagnostic::ImportCycle { modules }]
            if modules.iter().map(ModuleName::as_str).collect::<Vec<_>>() == ["a", "b", "a"]
    ));
}

#[test]
fn failed_module_compilation_exposes_no_partial_definitions() {
    let modules =
        MemoryModules::default().with("partial", r#"((~ leaked () "must not run") (# missing))"#);
    let oracle = ScriptedOracle::answers(&[Some("unexpected")]);
    let result = run("((# partial) (leaked))", &oracle, &modules);
    assert!(oracle.prompts().is_empty());
    assert!(matches!(
        result.diagnostics.as_slice(),
        [
            RuntimeDiagnostic::ModuleNotFound { module },
            RuntimeDiagnostic::UndefinedMacro { name },
        ] if module.as_str() == "missing" && name == "leaked"
    ));
}

#[test]
fn nested_lexical_import_does_not_leak_to_its_parent_scope() {
    let modules = MemoryModules::default().with("private", r#"(~ secret () "inside")"#);
    let oracle = ScriptedOracle::answers(&[Some("inside answer")]);
    let result = run("(((# private) (secret)) (secret))", &oracle, &modules);
    assert_eq!(oracle.prompts(), ["inside"]);
    assert_eq!(
        result.diagnostics,
        [RuntimeDiagnostic::UndefinedMacro {
            name: "secret".to_string()
        }]
    );
}

#[test]
fn a_macro_can_expand_to_a_lexical_import() {
    let modules = MemoryModules::default().with("tools", r#"(~ tool () "loaded")"#);
    let oracle = ScriptedOracle::answers(&[Some("done")]);
    let result = run("((~ load () '(# tools)) (load) (tool))", &oracle, &modules);
    assert_eq!(modules.requests(), ["tools"]);
    assert_eq!(oracle.prompts(), ["loaded"]);
    assert_eq!(result.output.as_deref(), Some("done"));
}

#[test]
fn module_cache_is_scoped_to_one_orchestration() {
    let modules = MemoryModules::default().with("tool", "(~ work () 'ok)");
    for _ in 0..2 {
        let result = run("(# tool)", &ScriptedOracle::default(), &modules);
        assert!(result.diagnostics.is_empty());
    }
    assert_eq!(modules.requests(), ["tool", "tool"]);
}

#[test]
fn import_only_modules_reexport_transitively() {
    let modules = MemoryModules::default()
        .with("empty-prelude", "(# no-exports)")
        .with("no-exports", "(# leaf)")
        .with("leaf", "(~ leaf () 'ok)");
    let result = run("(# empty-prelude)", &ScriptedOracle::default(), &modules);
    assert!(result.diagnostics.is_empty());
    assert!(result
        .events
        .iter()
        .any(|event| matches!(event, ExecutionEvent::ModuleLoaded { definitions: 1, .. })));
}
