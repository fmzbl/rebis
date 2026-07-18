//! Orchestration: a Rebis program run as a system of agents and subagents.
//!
//! The o-[]-o tree *is* the org chart. Every node is an agent; its operands
//! are its subagents; execution is depth-first with the record as the shared
//! blackboard:
//!
//! - an **atom** is a raw model prompt;
//! - a **square** runs its branch programs and then its bracketed mediator;
//! - an **arrow** routes actual answers between programs without firing itself.
//!
//! Cost bound: at most one model call per atom plus one per square — the
//! node count of the program is a hard ceiling on model calls.

use crate::eval::{eval, Oracle};
use crate::record::{Concept, Record};
use crate::syntax::{parse, Expr, ModuleName};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

type Functions = BTreeMap<String, (Vec<String>, Expr)>;

/// Structural calls are macros and may expand recursively. This budget keeps
/// an accidentally non-terminating expansion from exhausting the host stack.
pub const MAX_MACRO_EXPANSIONS: usize = 256;
/// Maximum number of distinct uncached modules loaded by one orchestration.
pub const MAX_MODULE_IMPORTS: usize = 64;
/// Maximum model calls made by one default orchestration.
pub const MAX_MODEL_CALLS: usize = 1_024;
/// Default bound on concurrent branch evaluation inside one square. Square
/// branches are semantically unordered; this caps how many evaluate at once
/// under [`orchestrate_parallel`]. `1` reproduces sequential evaluation.
pub const MAX_CONCURRENCY: usize = 4;

/// Host-configurable, deterministic orchestration resource limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeLimits {
    macro_expansions: usize,
    module_imports: usize,
    model_calls: usize,
    max_concurrency: usize,
}

impl RuntimeLimits {
    /// Reference production limits used by the convenience entry points.
    #[must_use]
    pub const fn standard() -> Self {
        Self {
            macro_expansions: MAX_MACRO_EXPANSIONS,
            module_imports: MAX_MODULE_IMPORTS,
            model_calls: MAX_MODEL_CALLS,
            max_concurrency: MAX_CONCURRENCY,
        }
    }

    /// Set the maximum number of macro expansions. Zero disables expansion.
    #[must_use]
    pub const fn with_macro_expansions(mut self, limit: usize) -> Self {
        self.macro_expansions = limit;
        self
    }

    /// Set the maximum number of distinct module loads. Zero disables imports.
    #[must_use]
    pub const fn with_module_imports(mut self, limit: usize) -> Self {
        self.module_imports = limit;
        self
    }

    /// Set the maximum number of model calls. Zero makes execution model-silent.
    #[must_use]
    pub const fn with_model_calls(mut self, limit: usize) -> Self {
        self.model_calls = limit;
        self
    }

    /// Bound concurrent branch evaluation within one square under
    /// [`orchestrate_parallel`]. `1` (or `0`) evaluates branches sequentially,
    /// byte-identical to the sequential entry points. The bound applies per
    /// square: nested squares each fan out up to this bound.
    #[must_use]
    pub const fn with_max_concurrency(mut self, limit: usize) -> Self {
        self.max_concurrency = if limit == 0 { 1 } else { limit };
        self
    }
}

impl Default for RuntimeLimits {
    fn default() -> Self {
        Self::standard()
    }
}

/// Host boundary used by `(# module)`.
///
/// The language core never accesses a filesystem or network. A host maps the
/// symbolic module name to Rebis source under its own policy. A host may make a
/// folder importable by resolving it to a definition-only program containing
/// imports of its children, for example `((# tools/a) (# tools/b))`.
pub trait ModuleResolver {
    /// Return module source, `None` when it does not exist, or a host failure.
    ///
    /// # Errors
    ///
    /// Returns a host-defined message when module storage cannot be accessed.
    fn resolve(&self, module: &ModuleName) -> Result<Option<String>, String>;
}

struct NoModules;

impl ModuleResolver for NoModules {
    fn resolve(&self, _module: &ModuleName) -> Result<Option<String>, String> {
        Ok(None)
    }
}

fn substitute(expr: &Expr, bindings: &BTreeMap<String, Expr>) -> Expr {
    match expr {
        Expr::Program(forms) => Expr::Program(
            forms
                .iter()
                .map(|form| substitute(form, bindings))
                .collect(),
        ),
        Expr::Symbol(name) => bindings.get(name).cloned().unwrap_or_else(|| expr.clone()),
        // A quote is an expansion boundary. Only explicit unquotes inside it
        // may see the macro's bindings.
        Expr::Prompt(_) | Expr::Quote(_) | Expr::Import { .. } => expr.clone(),
        Expr::Unquote(inner) => substitute(inner, bindings),
        Expr::Compose(items) => {
            Expr::Compose(items.iter().map(|e| substitute(e, bindings)).collect())
        }
        Expr::Concat(items) => {
            Expr::Concat(items.iter().map(|e| substitute(e, bindings)).collect())
        }
        Expr::Square { mediator, branches } => Expr::Square {
            mediator: Box::new(substitute(mediator, bindings)),
            branches: branches.iter().map(|e| substitute(e, bindings)).collect(),
        },
        Expr::Forward(a, b) => Expr::Forward(
            Box::new(substitute(a, bindings)),
            Box::new(substitute(b, bindings)),
        ),
        Expr::Backflow(a, b) => Expr::Backflow(
            Box::new(substitute(a, bindings)),
            Box::new(substitute(b, bindings)),
        ),
        Expr::Call { name, args } => {
            let callee = match bindings.get(name) {
                Some(Expr::Symbol(bound)) => bound.clone(),
                _ => name.clone(),
            };
            Expr::Call {
                name: callee,
                args: args.iter().map(|e| substitute(e, bindings)).collect(),
            }
        }
        Expr::Function { name, params, body } => {
            let inner = bindings
                .iter()
                .filter(|(key, _)| !params.contains(key))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            Expr::Function {
                name: name.clone(),
                params: params.clone(),
                body: Box::new(substitute(body, &inner)),
            }
        }
    }
}

/// Expand one implicitly quasiquoted macro template. At depth zero an
/// unquote is replaced by its structurally substituted expression. Nested
/// quotes remain syntax and require a corresponding nested unquote.
fn expand_quoted(expr: &Expr, bindings: &BTreeMap<String, Expr>, depth: usize) -> Expr {
    match expr {
        Expr::Program(forms) => Expr::Program(
            forms
                .iter()
                .map(|form| expand_quoted(form, bindings, depth))
                .collect(),
        ),
        Expr::Unquote(inner) if depth == 0 => substitute(inner, bindings),
        Expr::Unquote(inner) => Expr::Unquote(Box::new(expand_quoted(inner, bindings, depth - 1))),
        Expr::Quote(inner) => Expr::Quote(Box::new(expand_quoted(inner, bindings, depth + 1))),
        Expr::Prompt(_) | Expr::Symbol(_) | Expr::Import { .. } => expr.clone(),
        Expr::Compose(items) => Expr::Compose(
            items
                .iter()
                .map(|item| expand_quoted(item, bindings, depth))
                .collect(),
        ),
        Expr::Concat(items) => Expr::Concat(
            items
                .iter()
                .map(|item| expand_quoted(item, bindings, depth))
                .collect(),
        ),
        Expr::Square { mediator, branches } => Expr::Square {
            mediator: Box::new(expand_quoted(mediator, bindings, depth)),
            branches: branches
                .iter()
                .map(|branch| expand_quoted(branch, bindings, depth))
                .collect(),
        },
        Expr::Forward(a, b) => Expr::Forward(
            Box::new(expand_quoted(a, bindings, depth)),
            Box::new(expand_quoted(b, bindings, depth)),
        ),
        Expr::Backflow(a, b) => Expr::Backflow(
            Box::new(expand_quoted(a, bindings, depth)),
            Box::new(expand_quoted(b, bindings, depth)),
        ),
        Expr::Function { name, params, body } => Expr::Function {
            name: name.clone(),
            params: params.clone(),
            body: Box::new(expand_quoted(body, bindings, depth)),
        },
        Expr::Call { name, args } => Expr::Call {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| expand_quoted(arg, bindings, depth))
                .collect(),
        },
    }
}

fn expand_macro(body: &Expr, bindings: &BTreeMap<String, Expr>) -> Expr {
    fn normalize(expr: Expr) -> Expr {
        match expr {
            Expr::Compose(items) => {
                let mut items: Vec<_> = items.into_iter().map(normalize).collect();
                if matches!(items.first(), Some(Expr::Symbol(_))) {
                    let Expr::Symbol(name) = items.remove(0) else {
                        unreachable!()
                    };
                    Expr::Call { name, args: items }
                } else {
                    Expr::Compose(items)
                }
            }
            Expr::Concat(items) => Expr::Concat(items.into_iter().map(normalize).collect()),
            Expr::Square { mediator, branches } => Expr::Square {
                mediator: Box::new(normalize(*mediator)),
                branches: branches.into_iter().map(normalize).collect(),
            },
            Expr::Forward(a, b) => Expr::Forward(Box::new(normalize(*a)), Box::new(normalize(*b))),
            Expr::Backflow(a, b) => {
                Expr::Backflow(Box::new(normalize(*a)), Box::new(normalize(*b)))
            }
            Expr::Function { name, params, body } => Expr::Function {
                name,
                params,
                body: Box::new(normalize(*body)),
            },
            Expr::Call { name, args } => Expr::Call {
                name,
                args: args.into_iter().map(normalize).collect(),
            },
            Expr::Quote(inner) => Expr::Quote(Box::new(normalize(*inner))),
            Expr::Unquote(inner) => Expr::Unquote(Box::new(normalize(*inner))),
            Expr::Import { .. } => expr,
            leaf => leaf,
        }
    }

    let expanded = match body {
        Expr::Quote(template) => expand_quoted(template, bindings, 0),
        _ => substitute(body, bindings),
    };
    normalize(expanded)
}

/// One agent firing in the orchestration trace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Firing {
    /// Which quoted prompt fired.
    pub agent: String,
    /// The architected prompt it received.
    pub prompt: String,
    /// The answer, or None when the oracle declined.
    pub answer: Option<String>,
    /// Parenthesis depth at which this agent executes.
    pub abstraction: usize,
}

/// Direction in which an arrow routes a produced value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowDirection {
    /// `(-> producer consumer)`.
    Forward,
    /// `(<- consumer producer)`.
    Backflow,
}

/// A typed runtime problem discovered while orchestrating valid Rebis syntax.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeDiagnostic {
    /// A call names no macro visible in its lexical group.
    UndefinedMacro {
        /// The unresolved name.
        name: String,
    },
    /// A macro received a different number of arguments than it declares.
    ArityMismatch {
        /// Macro name.
        name: String,
        /// Declared parameter count.
        expected: usize,
        /// Supplied argument count.
        actual: usize,
    },
    /// Recursive structural expansion exhausted the deterministic safety budget.
    ExpansionLimit {
        /// Macro being expanded when the limit was reached.
        name: String,
        /// Configured maximum number of expansions.
        limit: usize,
    },
    /// A conditional mediator returned neither `yes` nor `no`.
    InvalidCondition {
        /// Returned text, or `None` when the condition declined to answer.
        value: Option<String>,
    },
    /// The host model adapter failed before producing a response.
    OracleFailure {
        /// Host-provided failure description.
        message: String,
    },
    /// No module source exists for an imported name.
    ModuleNotFound {
        /// Missing module.
        module: ModuleName,
    },
    /// The host failed while resolving module storage.
    ModuleLoadFailure {
        /// Requested module.
        module: ModuleName,
        /// Host-provided failure description.
        message: String,
    },
    /// A module did not parse or contained executable top-level forms.
    InvalidModule {
        /// Invalid module.
        module: ModuleName,
        /// Parse or structure diagnostic.
        message: String,
    },
    /// Nested imports formed a cycle.
    ImportCycle {
        /// Ordered cycle ending with the repeated module.
        modules: Vec<ModuleName>,
    },
    /// A program exceeded the bounded number of distinct module loads.
    ModuleLimit {
        /// Configured module limit.
        limit: usize,
    },
    /// A program attempted more model calls than its host budget permits.
    ModelCallLimit {
        /// Configured call limit.
        limit: usize,
    },
}

impl fmt::Display for RuntimeDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UndefinedMacro { name } => write!(formatter, "undefined macro `{name}`"),
            Self::ArityMismatch {
                name,
                expected,
                actual,
            } => write!(
                formatter,
                "macro `{name}` expected {expected} argument(s), received {actual}"
            ),
            Self::ExpansionLimit { name, limit } => write!(
                formatter,
                "macro expansion limit ({limit}) reached while expanding `{name}`"
            ),
            Self::InvalidCondition { value: Some(value) } => write!(
                formatter,
                "conditional mediator must return `yes` or `no`, received {value:?}"
            ),
            Self::InvalidCondition { value: None } => formatter
                .write_str("conditional mediator must return `yes` or `no`, but returned no value"),
            Self::OracleFailure { message } => write!(formatter, "oracle failure: {message}"),
            Self::ModuleNotFound { module } => write!(formatter, "module `{module}` was not found"),
            Self::ModuleLoadFailure { module, message } => {
                write!(formatter, "could not load module `{module}`: {message}")
            }
            Self::InvalidModule { module, message } => {
                write!(formatter, "invalid module `{module}`: {message}")
            }
            Self::ImportCycle { modules } => write!(
                formatter,
                "module import cycle: {}",
                modules
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" -> ")
            ),
            Self::ModuleLimit { limit } => {
                write!(formatter, "module import limit ({limit}) reached")
            }
            Self::ModelCallLimit { limit } => {
                write!(formatter, "model call limit ({limit}) reached")
            }
        }
    }
}

impl std::error::Error for RuntimeDiagnostic {}

/// An observable event emitted synchronously while a Rebis program executes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecutionEvent {
    /// A prompt is about to be handed to the host model.
    PromptStarted {
        /// Fully routed prompt, including any input value.
        prompt: String,
        /// Parenthesis abstraction depth.
        abstraction: usize,
    },
    /// A model call completed and was added to the firing trace.
    PromptFinished(Firing),
    /// An arrow routed the collected non-`nothing` results of one side.
    FlowRouted {
        /// Arrow direction.
        direction: FlowDirection,
        /// Exact value supplied to the consumer.
        value: String,
    },
    /// All ordinary square branches completed and mediation is starting.
    MediatorStarted {
        /// Number of branch programs mediated.
        branches: usize,
    },
    /// A lazy two-way square selected exactly one branch.
    BranchSelected {
        /// `true` selects the first branch; `false` selects the second.
        decision: bool,
    },
    /// A pure-symbol mediator judged its branches deterministically — no
    /// model call. The branch whose answer best round-trips onto the
    /// mediator's tokens becomes the square's result.
    MediatorResolved {
        /// 1-based index of the winning accepted answer, in source order.
        result: usize,
        /// The winner's holonomy as a percentage `0..=100`: `0` is a
        /// perfect round-trip, `100` cannot round-trip at all.
        holonomy: u8,
    },
    /// A macro expanded successfully.
    MacroExpanded {
        /// Expanded macro name.
        name: String,
        /// Remaining expansion budget.
        remaining: usize,
    },
    /// A definition-only module was resolved and added to lexical scope.
    ModuleLoaded {
        /// Imported module name.
        module: ModuleName,
        /// Number of macros made available, including re-exported imports.
        definitions: usize,
    },
    /// A typed runtime problem occurred.
    Diagnostic(RuntimeDiagnostic),
}

#[cfg(test)]
mod current_tests {
    use super::*;
    use crate::syntax::parse;
    use std::cell::RefCell;

    struct Scripted {
        answers: RefCell<Vec<String>>,
        prompts: RefCell<Vec<String>>,
    }
    impl Oracle for Scripted {
        fn fire(&self, prompt: &str) -> Option<String> {
            self.prompts.borrow_mut().push(prompt.to_string());
            let mut answers = self.answers.borrow_mut();
            (!answers.is_empty()).then(|| answers.remove(0))
        }
    }

    #[test]
    fn square_runs_branches_then_its_embedded_mediator() {
        let oracle = Scripted {
            answers: RefCell::new(vec!["inspection".into(), "trace".into(), "final".into()]),
            prompts: RefCell::new(Vec::new()),
        };
        let expr = parse("([\"Combine the reports\"] \"Inspect the code\" \"Trace the failure\")")
            .unwrap();
        let mut record = Record::from_texts::<&str>(&[]);
        let out = orchestrate(&expr, &mut record, &oracle);
        let prompts = oracle.prompts.borrow();
        assert_eq!(prompts[0], "Inspect the code");
        assert_eq!(prompts[1], "Trace the failure");
        assert!(prompts[2].starts_with("Combine the reports\n\nINPUT:\n"));
        assert!(prompts[2].contains("RESULT 1:\ninspection"));
        assert!(prompts[2].contains("RESULT 2:\ntrace"));
        assert_eq!(out.firings.len(), 3);
    }

    #[test]
    fn arrows_route_actual_answers_in_their_written_direction() {
        let oracle = Scripted {
            answers: RefCell::new(vec!["found the cause".into(), "wrote the patch".into()]),
            prompts: RefCell::new(Vec::new()),
        };
        let expr = parse("(-> \"Find the cause\" \"Write the patch\")").unwrap();
        let mut record = Record::from_texts::<&str>(&[]);
        let _ = orchestrate(&expr, &mut record, &oracle);
        let prompts = oracle.prompts.borrow();
        assert_eq!(prompts[0], "Find the cause");
        assert!(prompts[1].starts_with("Write the patch\n\nINPUT:\n"));
        assert!(prompts[1].contains("RESULT 1:\nfound the cause"));
    }

    #[test]
    fn named_functions_substitute_prompt_arguments_structurally() {
        let oracle = Scripted {
            answers: RefCell::new(vec!["analysis".into(), "report".into()]),
            prompts: RefCell::new(Vec::new()),
        };
        let expr = parse(
            "((~ inspect (target) (-> target \"Write report\")) (inspect \"Inspect parser\"))",
        )
        .unwrap();
        let mut record = Record::from_texts::<&str>(&[]);
        let _ = orchestrate(&expr, &mut record, &oracle);
        let prompts = oracle.prompts.borrow();
        assert_eq!(prompts[0], "Inspect parser");
        assert!(prompts[1].starts_with("Write report\n\nINPUT:\n"));
    }

    #[test]
    fn functions_can_receive_named_functions_as_arguments() {
        let oracle = Scripted {
            answers: RefCell::new(vec!["inspection".into(), "report".into()]),
            prompts: RefCell::new(Vec::new()),
        };
        let expr = parse(
            "((~ apply (worker target) (worker target)) \
              (~ inspect (target) (-> target \"Write report\")) \
              (apply inspect \"Inspect parser\"))",
        )
        .unwrap();
        let mut record = Record::from_texts::<&str>(&[]);
        let _ = orchestrate(&expr, &mut record, &oracle);
        let prompts = oracle.prompts.borrow();
        assert_eq!(prompts[0], "Inspect parser");
        assert!(prompts[1].starts_with("Write report\n\nINPUT:\n"));
        assert!(prompts[1].contains("RESULT 1:\ninspection"));
    }

    #[test]
    fn quoted_macro_templates_splice_unquoted_arguments() {
        let oracle = Scripted {
            answers: RefCell::new(vec!["first".into(), "second".into()]),
            prompts: RefCell::new(Vec::new()),
        };
        let expr =
            parse("((~ twice (work) '(-> ,work ,work)) (twice \"Improve this code\"))").unwrap();
        let mut record = Record::from_texts::<&str>(&[]);
        let _ = orchestrate(&expr, &mut record, &oracle);
        assert_eq!(
            oracle.prompts.borrow().as_slice(),
            [
                "Improve this code",
                "Improve this code\n\nINPUT:\nRESULT 1:\nfirst"
            ]
        );
    }

    #[test]
    fn unquote_can_generate_a_call_head() {
        let oracle = Scripted {
            answers: RefCell::new(vec!["done".into()]),
            prompts: RefCell::new(Vec::new()),
        };
        let expr = parse(
            "((~ apply (worker value) '(,worker ,value)) \
              (~ inspect (value) ',value) \
              (apply inspect \"Inspect parser\"))",
        )
        .unwrap();
        let mut record = Record::from_texts::<&str>(&[]);
        let _ = orchestrate(&expr, &mut record, &oracle);
        assert_eq!(oracle.prompts.borrow().as_slice(), ["Inspect parser"]);
    }

    #[test]
    fn function_definitions_do_not_leak_out_of_their_group() {
        let oracle = Scripted {
            answers: RefCell::new(Vec::new()),
            prompts: RefCell::new(Vec::new()),
        };
        let expr = parse("(((~ hidden (x) x)) (hidden \"must not fire\"))").unwrap();
        let mut record = Record::from_texts::<&str>(&[]);
        let _ = orchestrate(&expr, &mut record, &oracle);
        assert!(oracle.prompts.borrow().is_empty());
    }

    #[test]
    fn call_mediated_two_way_square_is_lazy() {
        let oracle = Scripted {
            answers: RefCell::new(vec!["yes".into(), "selected".into()]),
            prompts: RefCell::new(Vec::new()),
        };
        let expr = parse(
            "((~ choose () \"Answer exactly yes or no\") \
              ([(choose)] \"yes branch\" \"no branch\"))",
        )
        .unwrap();
        let mut record = Record::from_texts::<&str>(&[]);
        let _ = orchestrate(&expr, &mut record, &oracle);
        assert_eq!(
            oracle.prompts.borrow().as_slice(),
            ["Answer exactly yes or no", "yes branch"]
        );
    }

    #[test]
    fn macros_can_express_a_runtime_loop_by_self_calling() {
        let oracle = Scripted {
            answers: RefCell::new(vec![
                "original".into(),
                "no".into(),
                "original".into(),
                "improved".into(),
                "yes".into(),
                "original".into(),
                "improved".into(),
            ]),
            prompts: RefCell::new(Vec::new()),
        };
        let expr = parse(
            "((~ step (value) (-> value \"Improve once\")) \
              (~ done (value) (-> value \"Answer exactly yes or no\")) \
              (~ loop (value work stop) \
                ([(stop value)] value (loop (work value) work stop))) \
              (loop \"original program\" step done))",
        )
        .unwrap();
        let mut record = Record::from_texts::<&str>(&[]);
        let _ = orchestrate(&expr, &mut record, &oracle);
        let prompts = oracle.prompts.borrow();
        assert_eq!(
            prompts
                .iter()
                .filter(|prompt| prompt.starts_with("Answer exactly yes or no"))
                .count(),
            2
        );
        assert_eq!(
            prompts
                .iter()
                .filter(|prompt| prompt.starts_with("Improve once"))
                .count(),
            2
        );
    }

    #[test]
    fn composition_assembles_operand_text_into_one_prompt() {
        let oracle = Scripted {
            answers: RefCell::new(vec!["done".into()]),
            prompts: RefCell::new(Vec::new()),
        };
        // A macro parameter woven into a `$` composition contributes its bound
        // text, not a separate model call: the whole square fires once.
        let expr = parse(
            "((~ case (self rival) ($ \"the case that \" self \" beats \" rival)) \
              (case \"Jay-Z\" \"Kanye\"))",
        )
        .unwrap();
        let mut record = Record::from_texts::<&str>(&[]);
        let _ = orchestrate(&expr, &mut record, &oracle);
        assert_eq!(
            oracle.prompts.borrow().as_slice(),
            ["the case that Jay-Z beats Kanye"]
        );
    }

    #[test]
    fn composition_interpolates_a_macros_text_without_firing_it() {
        let oracle = Scripted {
            answers: RefCell::new(vec!["done".into()]),
            prompts: RefCell::new(Vec::new()),
        };
        // `$` never executes an operand: `(color)` is expanded to its text and
        // woven in, it does NOT fire. Only the one assembled string reaches the
        // model — the whole point of `$` being interpolation, not execution.
        let expr = parse("((~ color () \"blue\") ($ \"the color is \" (color)))").unwrap();
        let mut record = Record::from_texts::<&str>(&[]);
        let _ = orchestrate(&expr, &mut record, &oracle);
        assert_eq!(oracle.prompts.borrow().as_slice(), ["the color is blue"]);
    }

    #[test]
    fn nested_composition_assembles_without_extra_calls() {
        let oracle = Scripted {
            answers: RefCell::new(vec!["done".into()]),
            prompts: RefCell::new(Vec::new()),
        };
        let expr = parse("($ \"a\" ($ \"b\" \"c\"))").unwrap();
        let mut record = Record::from_texts::<&str>(&[]);
        let _ = orchestrate(&expr, &mut record, &oracle);
        assert_eq!(oracle.prompts.borrow().as_slice(), ["abc"]);
    }

    #[test]
    fn a_program_operand_in_composition_contributes_nothing_and_does_not_run() {
        let oracle = Scripted {
            answers: RefCell::new(vec!["done".into()]),
            prompts: RefCell::new(Vec::new()),
        };
        // `$` is a string constructor: an operand that is a program (here an
        // arrow) is not a string, so it contributes nothing and — crucially —
        // is never executed. No error, no stray firing; the arrow's inner
        // prompts do not run. A computed value reaches a prompt through `->`.
        let expr = parse("($ \"answer: \" (-> \"think\" \"conclude\"))").unwrap();
        let mut record = Record::from_texts::<&str>(&[]);
        let _ = orchestrate(&expr, &mut record, &oracle);
        // Only the assembled string fires; "think"/"conclude" never run.
        assert_eq!(oracle.prompts.borrow().as_slice(), ["answer: "]);
    }

    #[test]
    fn a_quoted_program_is_a_running_template() {
        let oracle = Scripted {
            answers: RefCell::new(vec!["first".into(), "second".into()]),
            prompts: RefCell::new(Vec::new()),
        };
        // `'` is unchanged: a quoted program is a macro template that expands
        // and runs when the macro is called in execution position, so both
        // prompts fire.
        let expr = parse("((~ twice (work) '(-> ,work ,work)) (twice \"Improve this\"))").unwrap();
        let mut record = Record::from_texts::<&str>(&[]);
        let _ = orchestrate(&expr, &mut record, &oracle);
        let prompts = oracle.prompts.borrow();
        assert_eq!(prompts[0], "Improve this");
        assert!(prompts[1].starts_with("Improve this\n\nINPUT:\nRESULT 1:\nfirst"));
    }

    #[test]
    fn orchestration_exposes_the_structurally_returned_answer() {
        let oracle = Scripted {
            answers: RefCell::new(vec!["source".into(), "forward result".into()]),
            prompts: RefCell::new(Vec::new()),
        };
        let expr = parse("(-> \"source\" \"consumer\")").unwrap();
        let mut record = Record::from_texts::<&str>(&[]);
        let result = orchestrate(&expr, &mut record, &oracle);
        assert_eq!(result.output.as_deref(), Some("forward result"));
    }

    #[test]
    fn mediator_answer_is_the_square_output() {
        let oracle = Scripted {
            answers: RefCell::new(vec!["left".into(), "right".into(), "synthesis".into()]),
            prompts: RefCell::new(Vec::new()),
        };
        let expr = parse("([\"combine\"] \"left\" \"right\")").unwrap();
        let mut record = Record::from_texts::<&str>(&[]);
        let result = orchestrate(&expr, &mut record, &oracle);
        assert_eq!(result.output.as_deref(), Some("synthesis"));
    }
}

/// The result of running a program through the agent system: the final
/// concept plus the complete firing trace.
#[derive(Debug)]
pub struct Orchestration {
    /// What the program evaluated to after all agents ran.
    pub concept: Concept,
    /// The textual value returned by the program's structural output path.
    pub output: Option<String>,
    /// Every model call, in execution order.
    pub firings: Vec<Firing>,
    /// Every typed execution transition, in the order it occurred.
    pub events: Vec<ExecutionEvent>,
    /// Runtime problems that prevented part of the program from executing.
    pub diagnostics: Vec<RuntimeDiagnostic>,
}

struct NodeResult {
    concept: Concept,
    output: Option<String>,
}

fn silent(concept: Concept) -> NodeResult {
    NodeResult {
        concept,
        output: None,
    }
}

/// Orchestration budgets, shared across every engine of one run. Atomics so
/// square branches evaluating concurrently draw from the same allowances.
struct Budgets {
    expansions: AtomicUsize,
    modules: AtomicUsize,
    model_calls: AtomicUsize,
    model_limit_reported: AtomicBool,
}

impl Budgets {
    fn new(limits: RuntimeLimits) -> Self {
        Self {
            expansions: AtomicUsize::new(limits.macro_expansions),
            modules: AtomicUsize::new(limits.module_imports),
            model_calls: AtomicUsize::new(limits.model_calls),
            model_limit_reported: AtomicBool::new(false),
        }
    }
}

/// Claim one unit from a budget. Returns the new remaining count, or `None`
/// when the budget is exhausted.
fn claim(budget: &AtomicUsize) -> Option<usize> {
    budget
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
        .ok()
        .map(|previous| previous - 1)
}

/// The oracle seam, split by thread-shareability. Sequential entry points
/// accept any [`Oracle`]; concurrent branch evaluation additionally requires
/// `Sync`, so the capability is carried in the reference itself.
#[derive(Clone, Copy)]
enum OracleRef<'a> {
    Sequential(&'a dyn Oracle),
    Shareable(&'a (dyn Oracle + Sync)),
}

impl<'a> OracleRef<'a> {
    fn as_dyn(self) -> &'a dyn Oracle {
        match self {
            Self::Sequential(oracle) => oracle,
            Self::Shareable(oracle) => oracle,
        }
    }
}

/// The module-resolver seam, split the same way as [`OracleRef`].
#[derive(Clone, Copy)]
enum ResolverRef<'a> {
    Sequential(&'a dyn ModuleResolver),
    Shareable(&'a (dyn ModuleResolver + Sync)),
}

impl<'a> ResolverRef<'a> {
    fn as_dyn(self) -> &'a dyn ModuleResolver {
        match self {
            Self::Sequential(modules) => modules,
            Self::Shareable(modules) => modules,
        }
    }
}

/// Whether a mediator is **pure symbols** — `Symbol` and `Compose` nodes
/// only, with at least one symbol. Quoted prompts describe work; symbols
/// describe structure — so a mediator of pure symbols asks for no work and
/// judges its branches deterministically instead of firing a model.
fn pure_symbols(expr: &Expr) -> bool {
    fn walk(expr: &Expr, symbols: &mut usize) -> bool {
        match expr {
            Expr::Symbol(_) => {
                *symbols += 1;
                true
            }
            Expr::Program(items) | Expr::Compose(items) => {
                items.iter().all(|item| walk(item, symbols))
            }
            _ => false,
        }
    }
    let mut symbols = 0;
    walk(expr, &mut symbols) && symbols > 0
}

/// The judged task of a pure-symbol mediator: its symbols, joined as text so
/// the calculus's tokenizer defines the wanted terms.
fn symbol_text(expr: &Expr) -> String {
    fn collect(expr: &Expr, out: &mut Vec<String>) {
        match expr {
            Expr::Symbol(word) => out.push(word.clone()),
            Expr::Program(items) | Expr::Compose(items) => {
                for item in items {
                    collect(item, out);
                }
            }
            _ => {}
        }
    }
    let mut words = Vec::new();
    collect(expr, &mut words);
    words.join(" ")
}

/// Everything one branch thread produced, merged back in source order.
struct BranchOutcome {
    trace: Vec<Firing>,
    events: Vec<ExecutionEvent>,
    diagnostics: Vec<RuntimeDiagnostic>,
    module_cache: BTreeMap<ModuleName, Functions>,
}

struct Engine<'a> {
    record: &'a mut Record,
    oracle: OracleRef<'a>,
    modules: ResolverRef<'a>,
    observer: &'a mut dyn FnMut(&ExecutionEvent),
    trace: Vec<Firing>,
    events: Vec<ExecutionEvent>,
    diagnostics: Vec<RuntimeDiagnostic>,
    budgets: &'a Budgets,
    expansion_limit: usize,
    module_limit: usize,
    model_call_limit: usize,
    max_concurrency: usize,
    module_cache: BTreeMap<ModuleName, Functions>,
    module_stack: Vec<ModuleName>,
}

impl Engine<'_> {
    fn emit(&mut self, event: ExecutionEvent) {
        (self.observer)(&event);
        self.events.push(event);
    }

    fn diagnose(&mut self, diagnostic: RuntimeDiagnostic) {
        self.emit(ExecutionEvent::Diagnostic(diagnostic.clone()));
        self.diagnostics.push(diagnostic);
    }

    fn answers_since(&self, start: usize) -> String {
        self.trace[start..]
            .iter()
            .filter_map(|firing| firing.answer.as_deref())
            .filter(|answer| !answer.trim().eq_ignore_ascii_case("nothing"))
            .enumerate()
            .map(|(i, answer)| format!("RESULT {}:\n{}", i + 1, answer))
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    fn scout(&mut self, word: &str, abstraction: usize, input: Option<&str>) -> Option<String> {
        if claim(&self.budgets.model_calls).is_none() {
            if !self
                .budgets
                .model_limit_reported
                .swap(true, Ordering::SeqCst)
            {
                self.diagnose(RuntimeDiagnostic::ModelCallLimit {
                    limit: self.model_call_limit,
                });
            }
            return None;
        }
        let prompt = input.map_or_else(
            || word.to_string(),
            |value| format!("{word}\n\nINPUT:\n{value}"),
        );
        self.emit(ExecutionEvent::PromptStarted {
            prompt: prompt.clone(),
            abstraction,
        });
        let answer = match self.oracle.as_dyn().try_fire(&prompt) {
            Ok(answer) => answer,
            Err(message) => {
                self.diagnose(RuntimeDiagnostic::OracleFailure { message });
                None
            }
        };
        if let Some(text) = &answer {
            if !text.trim().eq_ignore_ascii_case("nothing") {
                self.record.append_text(text);
            }
        }
        let firing = Firing {
            agent: format!("○ prompt {word} · abstraction {abstraction}"),
            prompt,
            answer: answer.clone(),
            abstraction,
        };
        self.emit(ExecutionEvent::PromptFinished(firing.clone()));
        self.trace.push(firing);
        answer.filter(|text| !text.trim().eq_ignore_ascii_case("nothing"))
    }

    fn resolve_module(&mut self, module: &ModuleName) -> Result<Functions, RuntimeDiagnostic> {
        if let Some(functions) = self.module_cache.get(module) {
            return Ok(functions.clone());
        }
        if let Some(start) = self
            .module_stack
            .iter()
            .position(|candidate| candidate == module)
        {
            let mut modules = self.module_stack[start..].to_vec();
            modules.push(module.clone());
            return Err(RuntimeDiagnostic::ImportCycle { modules });
        }
        if claim(&self.budgets.modules).is_none() {
            return Err(RuntimeDiagnostic::ModuleLimit {
                limit: self.module_limit,
            });
        }
        self.module_stack.push(module.clone());
        let result = self.compile_module(module);
        self.module_stack.pop();
        if let Ok(functions) = &result {
            self.module_cache.insert(module.clone(), functions.clone());
        }
        result
    }

    fn compile_module(&mut self, module: &ModuleName) -> Result<Functions, RuntimeDiagnostic> {
        let source = self
            .modules
            .as_dyn()
            .resolve(module)
            .map_err(|message| RuntimeDiagnostic::ModuleLoadFailure {
                module: module.clone(),
                message,
            })?
            .ok_or_else(|| RuntimeDiagnostic::ModuleNotFound {
                module: module.clone(),
            })?;
        let expr = parse(&source).map_err(|error| RuntimeDiagnostic::InvalidModule {
            module: module.clone(),
            message: error.to_string(),
        })?;
        let items: &[Expr] = match &expr {
            Expr::Program(items) | Expr::Compose(items) => items,
            Expr::Function { .. } | Expr::Import { .. } => std::slice::from_ref(&expr),
            _ => {
                return Err(RuntimeDiagnostic::InvalidModule {
                    module: module.clone(),
                    message: "modules contain only top-level `~` definitions and `#` imports"
                        .to_string(),
                })
            }
        };
        let mut exports = Functions::new();
        for item in items {
            match item {
                Expr::Function { name, params, body } => {
                    exports.insert(name.clone(), (params.clone(), body.as_ref().clone()));
                }
                Expr::Import { module } => {
                    exports.extend(self.resolve_module(module)?);
                }
                _ => {
                    return Err(RuntimeDiagnostic::InvalidModule {
                        module: module.clone(),
                        message: "modules contain only top-level `~` definitions and `#` imports"
                            .to_string(),
                    })
                }
            }
        }
        Ok(exports)
    }

    fn import_into_scope(&mut self, module: &ModuleName, functions: &mut Functions) {
        match self.resolve_module(module) {
            Ok(imported) => {
                let definitions = imported.len();
                functions.extend(imported);
                self.emit(ExecutionEvent::ModuleLoaded {
                    module: module.clone(),
                    definitions,
                });
            }
            Err(diagnostic) => self.diagnose(diagnostic),
        }
    }

    fn run_compose(
        &mut self,
        expr: &Expr,
        items: &[Expr],
        abstraction: usize,
        input: Option<&str>,
        functions: &Functions,
    ) -> NodeResult {
        let mut local = functions.clone();
        for item in items {
            match item {
                Expr::Function { name, params, body } => {
                    local.insert(name.clone(), (params.clone(), body.as_ref().clone()));
                }
                Expr::Import { module } => self.import_into_scope(module, &mut local),
                _ => {}
            }
        }
        let mut last = silent(eval(expr, self.record));
        for item in items {
            if !matches!(item, Expr::Function { .. } | Expr::Import { .. }) {
                last = self.run_node(item, abstraction + 1, input, &mut local);
            }
        }
        last
    }

    fn run_conditional(
        &mut self,
        expr: &Expr,
        mediator: &Expr,
        branches: &[Expr],
        abstraction: usize,
        input: Option<&str>,
        functions: &mut Functions,
    ) -> NodeResult {
        let condition = self.run_node(mediator, abstraction + 1, input, functions);
        let decision = condition.output.as_deref().map(str::trim);
        let selected = match decision {
            Some(value) if value.eq_ignore_ascii_case("yes") => Some(true),
            Some(value) if value.eq_ignore_ascii_case("no") => Some(false),
            _ => None,
        };
        if let Some(decision) = selected {
            self.emit(ExecutionEvent::BranchSelected { decision });
            return self.run_node(
                &branches[usize::from(!decision)],
                abstraction + 1,
                input,
                functions,
            );
        }
        self.diagnose(RuntimeDiagnostic::InvalidCondition {
            value: condition.output,
        });
        silent(eval(expr, self.record))
    }

    /// Non-`nothing` answers fired since `start`, individually, in source order.
    fn accepted_since(&self, start: usize) -> Vec<String> {
        self.trace[start..]
            .iter()
            .filter_map(|firing| firing.answer.as_deref())
            .filter(|answer| !answer.trim().eq_ignore_ascii_case("nothing"))
            .map(str::to_string)
            .collect()
    }

    /// Deterministic mediation: judge accepted branch answers by round-trip
    /// holonomy against the mediator's own tokens — the calculus is the
    /// judge, and no model fires. The lowest holonomy wins (ties resolve to
    /// source order); an answer that cannot round-trip at all (`1.0`) is
    /// refused, and if every answer is refused the square yields nothing.
    fn resolve_mediation(&mut self, mediator: &Expr, answers: &[String]) -> Option<String> {
        let task = symbol_text(mediator);
        let record = Record::from_texts(answers);
        let mut best: Option<(f32, usize)> = None;
        for (index, answer) in answers.iter().enumerate() {
            let h = crate::eval::holonomy_reflected(&task, answer, &record);
            if h < 1.0 && best.as_ref().map_or(true, |(b, _)| h < *b) {
                best = Some((h, index));
            }
        }
        best.map(|(holonomy, index)| {
            // Scores are coarse evidence ratios; the percentage is exact
            // enough for the trace and keeps the event `Eq`.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let percent = (holonomy * 100.0).round() as u8;
            self.emit(ExecutionEvent::MediatorResolved {
                result: index + 1,
                holonomy: percent,
            });
            answers[index].clone()
        })
    }

    fn run_square(
        &mut self,
        mediator: &Expr,
        branches: &[Expr],
        abstraction: usize,
        input: Option<&str>,
        functions: &mut Functions,
    ) -> NodeResult {
        let start = self.trace.len();
        // Square branches are semantically unordered and mutually isolated:
        // each evaluates against a branch-scoped definition table, and — under
        // a shareable oracle — concurrently against a snapshot of the record.
        // Answers, events, and diagnostics always merge in source order.
        match (self.oracle, self.modules) {
            (OracleRef::Shareable(oracle), ResolverRef::Shareable(modules))
                if self.max_concurrency > 1 && branches.len() > 1 =>
            {
                self.run_branches_concurrently(
                    oracle,
                    modules,
                    branches,
                    abstraction,
                    input,
                    functions,
                );
            }
            _ => {
                for branch in branches {
                    let mut scoped = functions.clone();
                    self.run_node(branch, abstraction + 1, input, &mut scoped);
                }
            }
        }
        self.emit(ExecutionEvent::MediatorStarted {
            branches: branches.len(),
        });
        // A mediator of pure symbols judges deterministically — the calculus
        // scores each accepted answer's round-trip onto the mediator's tokens
        // and no model fires. Any other mediator is a program that runs with
        // the ordered RESULT blocks as its input.
        if pure_symbols(mediator) {
            let answers = self.accepted_since(start);
            let output = self.resolve_mediation(mediator, &answers);
            return NodeResult {
                concept: eval(mediator, self.record),
                output,
            };
        }
        let reports = self.answers_since(start);
        self.run_node(mediator, abstraction + 1, Some(&reports), functions)
    }

    /// Evaluate square branches concurrently, bounded by `max_concurrency`
    /// per wave. Each branch gets a snapshot of the record, a branch-scoped
    /// definition table, and buffered trace/events; everything merges back in
    /// source order, so the orchestration is deterministic in structure
    /// regardless of completion order.
    fn run_branches_concurrently(
        &mut self,
        oracle: &(dyn Oracle + Sync),
        modules: &(dyn ModuleResolver + Sync),
        branches: &[Expr],
        abstraction: usize,
        input: Option<&str>,
        functions: &Functions,
    ) {
        let mut outcomes: Vec<BranchOutcome> = Vec::with_capacity(branches.len());
        for wave in branches.chunks(self.max_concurrency) {
            let wave_outcomes = std::thread::scope(|scope| {
                let handles: Vec<_> = wave
                    .iter()
                    .map(|branch| {
                        let mut branch_record = self.record.clone();
                        let mut branch_functions = functions.clone();
                        let branch_cache = self.module_cache.clone();
                        let branch_stack = self.module_stack.clone();
                        let budgets = self.budgets;
                        let expansion_limit = self.expansion_limit;
                        let module_limit = self.module_limit;
                        let model_call_limit = self.model_call_limit;
                        let max_concurrency = self.max_concurrency;
                        scope.spawn(move || {
                            let mut silent_observer = |_: &ExecutionEvent| {};
                            let mut engine = Engine {
                                record: &mut branch_record,
                                oracle: OracleRef::Shareable(oracle),
                                modules: ResolverRef::Shareable(modules),
                                observer: &mut silent_observer,
                                trace: Vec::new(),
                                events: Vec::new(),
                                diagnostics: Vec::new(),
                                budgets,
                                expansion_limit,
                                module_limit,
                                model_call_limit,
                                max_concurrency,
                                module_cache: branch_cache,
                                module_stack: branch_stack,
                            };
                            engine.run_node(branch, abstraction + 1, input, &mut branch_functions);
                            BranchOutcome {
                                trace: engine.trace,
                                events: engine.events,
                                diagnostics: engine.diagnostics,
                                module_cache: engine.module_cache,
                            }
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|handle| match handle.join() {
                        Ok(outcome) => outcome,
                        Err(panic) => std::panic::resume_unwind(panic),
                    })
                    .collect::<Vec<_>>()
            });
            outcomes.extend(wave_outcomes);
        }
        for outcome in outcomes {
            // Replay the branch's accepted answers into the shared record —
            // the same appends the sequential path makes at firing time.
            for firing in &outcome.trace {
                if let Some(answer) = &firing.answer {
                    if !answer.trim().eq_ignore_ascii_case("nothing") {
                        self.record.append_text(answer);
                    }
                }
            }
            for event in outcome.events {
                self.emit(event);
            }
            self.trace.extend(outcome.trace);
            self.diagnostics.extend(outcome.diagnostics);
            self.module_cache.extend(outcome.module_cache);
        }
    }

    fn run_flow(
        &mut self,
        producer: &Expr,
        consumer: &Expr,
        direction: FlowDirection,
        abstraction: usize,
        input: Option<&str>,
        functions: &mut Functions,
    ) -> NodeResult {
        let start = self.trace.len();
        self.run_node(producer, abstraction, input, functions);
        let routed = self.answers_since(start);
        self.emit(ExecutionEvent::FlowRouted {
            direction,
            value: routed.clone(),
        });
        self.run_node(consumer, abstraction, Some(&routed), functions)
    }

    /// Expand one macro call structurally: look it up, check arity, claim the
    /// expansion budget, and substitute its arguments. Returns the expanded body
    /// (no model call — expansion is pure), or `None` after diagnosing an
    /// undefined name, an arity mismatch, or an exhausted budget.
    fn expand_call(&mut self, name: &str, args: &[Expr], functions: &Functions) -> Option<Expr> {
        let Some((params, body)) = functions.get(name).cloned() else {
            self.diagnose(RuntimeDiagnostic::UndefinedMacro {
                name: name.to_string(),
            });
            return None;
        };
        if params.len() != args.len() {
            self.diagnose(RuntimeDiagnostic::ArityMismatch {
                name: name.to_string(),
                expected: params.len(),
                actual: args.len(),
            });
            return None;
        }
        let Some(remaining) = claim(&self.budgets.expansions) else {
            self.diagnose(RuntimeDiagnostic::ExpansionLimit {
                name: name.to_string(),
                limit: self.expansion_limit,
            });
            return None;
        };
        let bindings = params.into_iter().zip(args.iter().cloned()).collect();
        let expanded = expand_macro(&body, &bindings);
        self.emit(ExecutionEvent::MacroExpanded {
            name: name.to_string(),
            remaining,
        });
        Some(expanded)
    }

    fn run_call(
        &mut self,
        expr: &Expr,
        name: &str,
        args: &[Expr],
        abstraction: usize,
        input: Option<&str>,
        functions: &mut Functions,
    ) -> NodeResult {
        match self.expand_call(name, args, functions) {
            Some(expanded) => self.run_node(&expanded, abstraction, input, functions),
            None => silent(eval(expr, self.record)),
        }
    }

    /// Interpolate a `$` operand to its text. This is pure string construction:
    /// it never fires a model and never runs a subprogram.
    ///
    /// - a prompt or symbol contributes its characters / name;
    /// - a nested `$` contributes its assembled text;
    /// - a macro call is *expanded* (structural, no firing) and its expansion
    ///   is interpolated in turn, so a text macro contributes its text;
    /// - anything else is a program, not a string, so it contributes nothing —
    ///   `$` builds text, and a computed value reaches a prompt through `->`.
    fn interpolate(&mut self, expr: &Expr, functions: &Functions) -> String {
        match expr {
            Expr::Prompt(text) => text.clone(),
            Expr::Symbol(name) => name.clone(),
            Expr::Concat(items) => items
                .iter()
                .map(|item| self.interpolate(item, functions))
                .collect(),
            Expr::Call { name, args } => match self.expand_call(name, args, functions) {
                Some(expanded) => self.interpolate(&expanded, functions),
                None => String::new(),
            },
            _ => String::new(),
        }
    }

    fn run_node(
        &mut self,
        expr: &Expr,
        abstraction: usize,
        input: Option<&str>,
        functions: &mut Functions,
    ) -> NodeResult {
        match expr {
            Expr::Prompt(word) => {
                let output = self.scout(word, abstraction, input);
                NodeResult {
                    concept: eval(expr, self.record),
                    output,
                }
            }
            Expr::Symbol(_) | Expr::Quote(_) | Expr::Unquote(_) => silent(eval(expr, self.record)),
            Expr::Program(items) | Expr::Compose(items) => {
                self.run_compose(expr, items, abstraction, input, functions)
            }
            Expr::Concat(items) => {
                // `$` interpolates its operands to one string — pure text
                // construction, nothing inside it fires. The assembled string is
                // itself a prompt in this (execution) position, so it fires once,
                // exactly like a written literal would.
                let word: String = items
                    .iter()
                    .map(|item| self.interpolate(item, functions))
                    .collect();
                let output = self.scout(&word, abstraction, input);
                NodeResult {
                    concept: eval(expr, self.record),
                    output,
                }
            }
            Expr::Square { mediator, branches }
                if branches.len() == 2 && matches!(mediator.as_ref(), Expr::Call { .. }) =>
            {
                self.run_conditional(expr, mediator, branches, abstraction, input, functions)
            }
            Expr::Square { mediator, branches } => {
                self.run_square(mediator, branches, abstraction, input, functions)
            }
            Expr::Forward(a, b) => {
                self.run_flow(a, b, FlowDirection::Forward, abstraction, input, functions)
            }
            Expr::Backflow(a, b) => {
                self.run_flow(b, a, FlowDirection::Backflow, abstraction, input, functions)
            }
            Expr::Function { name, params, body } => {
                functions.insert(name.clone(), (params.clone(), body.as_ref().clone()));
                silent(eval(expr, self.record))
            }
            Expr::Call { name, args } => {
                self.run_call(expr, name, args, abstraction, input, functions)
            }
            Expr::Import { module } => {
                self.import_into_scope(module, functions);
                silent(eval(expr, self.record))
            }
        }
    }
}

/// Run a program while synchronously observing typed execution events.
///
/// The observer is called before each model request and immediately after every
/// execution transition, making this entry point suitable for streaming hosts.
#[must_use]
pub fn orchestrate_with_observer<O, F>(
    expr: &Expr,
    record: &mut Record,
    oracle: &O,
    observer: &mut F,
) -> Orchestration
where
    O: Oracle,
    F: FnMut(&ExecutionEvent),
{
    orchestrate_with_runtime(expr, record, oracle, &NoModules, observer)
}

/// Run a program with both a host module resolver and a live event observer.
///
/// This is the foundational host entry point used for saved hypersigils today
/// alongside the embedded standard library (which resolves `std/*` first).
#[must_use]
pub fn orchestrate_with_runtime<O, R, F>(
    expr: &Expr,
    record: &mut Record,
    oracle: &O,
    modules: &R,
    observer: &mut F,
) -> Orchestration
where
    O: Oracle,
    R: ModuleResolver,
    F: FnMut(&ExecutionEvent),
{
    orchestrate_with_limits(
        expr,
        record,
        oracle,
        modules,
        RuntimeLimits::standard(),
        observer,
    )
}

/// Run with explicit host budgets, module resolution, and live observation.
#[must_use]
pub fn orchestrate_with_limits<O, R, F>(
    expr: &Expr,
    record: &mut Record,
    oracle: &O,
    modules: &R,
    limits: RuntimeLimits,
    observer: &mut F,
) -> Orchestration
where
    O: Oracle,
    R: ModuleResolver,
    F: FnMut(&ExecutionEvent),
{
    let budgets = Budgets::new(limits);
    // The embedded standard library resolves `std/*` before the host and
    // reserves the namespace; everything else stays host policy.
    let modules = crate::stdlib::WithStd(modules);
    run_orchestration(
        expr,
        record,
        OracleRef::Sequential(oracle),
        ResolverRef::Sequential(&modules),
        // Sequential seams cannot cross threads; concurrency stays off.
        1,
        &budgets,
        limits,
        observer,
    )
}

/// Run with explicit budgets and **concurrent square-branch evaluation**.
///
/// Square branches are semantically unordered and mutually isolated, so under
/// a thread-shareable oracle and module resolver they may evaluate in
/// parallel, bounded by [`RuntimeLimits::with_max_concurrency`]. Arrows, the
/// lazy conditional square, and group children remain sequential — ordering in
/// Rebis is expressed by arrows, and only arrows. Answers, events, firings,
/// and diagnostics are merged in source order, so the orchestration's
/// structure is deterministic regardless of completion order; a
/// `max_concurrency` of `1` is byte-identical to [`orchestrate_with_limits`].
///
/// The observer runs on the orchestrating thread only: branch events buffer
/// and replay in source order at each square's join.
#[must_use]
pub fn orchestrate_parallel<O, R, F>(
    expr: &Expr,
    record: &mut Record,
    oracle: &O,
    modules: &R,
    limits: RuntimeLimits,
    observer: &mut F,
) -> Orchestration
where
    O: Oracle + Sync,
    R: ModuleResolver + Sync,
    F: FnMut(&ExecutionEvent),
{
    let budgets = Budgets::new(limits);
    let modules = crate::stdlib::WithStdSync(modules);
    run_orchestration(
        expr,
        record,
        OracleRef::Shareable(oracle),
        ResolverRef::Shareable(&modules),
        limits.max_concurrency.max(1),
        &budgets,
        limits,
        observer,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_orchestration(
    expr: &Expr,
    record: &mut Record,
    oracle: OracleRef<'_>,
    modules: ResolverRef<'_>,
    max_concurrency: usize,
    budgets: &Budgets,
    limits: RuntimeLimits,
    observer: &mut dyn FnMut(&ExecutionEvent),
) -> Orchestration {
    let mut engine = Engine {
        record,
        oracle,
        modules,
        observer,
        trace: Vec::new(),
        events: Vec::new(),
        diagnostics: Vec::new(),
        budgets,
        expansion_limit: limits.macro_expansions,
        module_limit: limits.module_imports,
        model_call_limit: limits.model_calls,
        max_concurrency,
        module_cache: BTreeMap::new(),
        module_stack: Vec::new(),
    };
    let result = engine.run_node(expr, 0, None, &mut Functions::new());
    Orchestration {
        concept: result.concept,
        output: result.output,
        firings: engine.trace,
        events: engine.events,
        diagnostics: engine.diagnostics,
    }
}

/// Run a program through the agent system: every quoted prompt a subagent,
/// every square a mediator agent, and every arrow an answer-flow edge.
#[must_use]
pub fn orchestrate<O: Oracle>(expr: &Expr, record: &mut Record, oracle: &O) -> Orchestration {
    orchestrate_with_observer(expr, record, oracle, &mut |_| {})
}

#[cfg(any())]
mod tests {
    use super::*;
    use crate::syntax::parse;

    struct Scripted(std::cell::RefCell<Vec<String>>, std::cell::RefCell<usize>);
    impl Scripted {
        fn new(answers: &[&str]) -> Scripted {
            Scripted(
                std::cell::RefCell::new(answers.iter().map(|s| s.to_string()).collect()),
                std::cell::RefCell::new(0),
            )
        }
        fn calls(&self) -> usize {
            *self.1.borrow()
        }
    }
    impl Oracle for Scripted {
        fn fire(&self, _prompt: &str) -> Option<String> {
            *self.1.borrow_mut() += 1;
            let mut a = self.0.borrow_mut();
            if a.is_empty() {
                None
            } else {
                Some(a.remove(0))
            }
        }
    }

    #[test]
    fn every_atom_is_a_scout_and_every_square_a_mediator() {
        let mut record = Record::from_texts(&["commits on the parser", "sleep synced"]);
        let oracle = Scripted::new(&[
            "commits touched the parser",             // scout commits
            "sleep was short before the parser work", // scout sleep
            "late commits often follow poor sleep",   // mediator, if needed
        ]);
        let expr = parse("([] commits sleep)").unwrap();
        let out = orchestrate(&expr, &mut record, &oracle);
        assert!(
            out.firings.len() >= 2,
            "both scouts fired: {:?}",
            out.firings
        );
        assert!(out.firings[0].agent.starts_with("○ atom"));
        assert!(
            out.concept.score > 0.0,
            "the mediated square holds: {:?}",
            out.concept
        );
    }

    #[test]
    fn atoms_are_raw_prompts_and_groups_raise_abstraction() {
        let mut record = Record::from_texts(&["the tokenizer regression landed"]);
        let oracle = Scripted::new(&["nothing"]);
        let expr = parse("(hello hello (bye))").unwrap();
        let out = orchestrate(&expr, &mut record, &oracle);
        assert_eq!(out.firings.len(), 3);
        assert_eq!(out.firings[0].prompt, "hello");
        assert_eq!(out.firings[1].prompt, "hello");
        assert_eq!(out.firings[2].prompt, "bye");
        assert_eq!(out.firings[0].abstraction, 1);
        assert_eq!(out.firings[1].abstraction, 1);
        assert_eq!(out.firings[2].abstraction, 2);
    }

    #[test]
    fn arrows_never_fire_and_node_count_bounds_the_calls() {
        let mut record = Record::from_texts(&["a b together", "c alone"]);
        let oracle = Scripted::new(&["a note", "b note", "c note", "d note", "extra", "extra"]);
        let expr = parse("(<- ([] a b) c)").unwrap();
        let out = orchestrate(&expr, &mut record, &oracle);
        // 3 atoms + at most 1 square; the backflow judge adds zero
        assert!(
            oracle.calls() <= 4,
            "calls {} exceed the node bound",
            oracle.calls()
        );
        let judges = out
            .firings
            .iter()
            .filter(|f| f.agent.contains("judge"))
            .count();
        assert_eq!(judges, 0, "arrows are deterministic");
    }
}
