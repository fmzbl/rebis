#![forbid(unsafe_code)]
//! Rebis (`o-[]-o`) — an atomic language to program AI.
//!
//! Quoted strings are raw prompts; bare atoms are Lisp-like symbols:
//!
//! ```text
//! program := expr+ EOF
//! expr := prompt | symbol | '\'' expr | ',' expr | '(' form ')'
//! form := '~' symbol '(' symbol* ')' expr
//!       | '#' module | '[' expr ']' expr+
//!       | '$' expr+
//!       | op expr expr+ | symbol expr* | expr+
//! op   := '->' | '<-'
//! ```
//!
//! A program may contain multiple top-level forms, like a Lisp source file;
//! they share one lexical definition scope without requiring an outer group.
//! The program *is* the symbol: `([m] a b)` is the mediator square, with
//! executable mediator program `m` between its branches and result. `->` and
//! `<-` route actual agent answers in their written direction. A bare group
//! composes. There is no other syntax: no keywords or numbers.
//!
//! Two languages merged, dividing the work as algebra and calculus do. The
//! **algebra** is the three forms themselves. The **calculus** is the
//! evaluation: every expression is evaluated against a [`Record`] — meaning
//! is a quantity computed from evidence, never string equality:
//!
//! - a **quoted prompt** is sent to the model exactly (and resolves to
//!   matching evidence in the optional deterministic record calculus);
//! - a **macro abstraction** `(~ name (parameter ...) body)` receives raw
//!   Rebis syntax; `'` quotes an output template and `,` splices syntax into
//!   it. A macro may call itself, so lazy conditional squares can express
//!   loops without a dedicated recursion form;
//! - a **module import** `(# name)` asks a host [`ModuleResolver`] for a
//!   definition-only compilation unit; the core remains I/O-free;
//! - a **group** composes concepts and raises its children one
//!   abstraction level; singleton groups are therefore significant;
//! - the **square** `([m] a b)` runs its branches, then runs `m` over their
//!   ordered model results. In deterministic record evaluation it exposes
//!   a compatibility collision view over mediator and branch concepts;
//!   record's co-occurrence graph and returns their *common ground*: shared
//!   evidence, scored by overlap. In agent evaluation this is the generative
//!   site — the model speaks into the square, never into a score;
//! - the **arrows** `(-> a b)` / `(<- a b)` route actual model results from
//!   `a` to `b`, or from `b` back to `a`. The optional record evaluator keeps
//!   the corresponding deterministic refinement interpretation.
//! - the **composition** `($ a b ...)` interpolates its operands to one string
//!   — a prompt's characters, a symbol's name, a macro's expanded text, a nested
//!   composition — and yields that string. It is pure text construction: nothing
//!   inside `$` fires or runs, and the assembled string fires only where it
//!   sits, like any literal. It is the one operator over the language's
//!   fundamental value; variables are macro parameters.
//!
//! Rebis owns composition and judgment. A host supplies the record and, for
//! agent programs, an [`Oracle`]. Nothing here chooses a vendor or performs
//! I/O.

mod agents;
mod eval;
mod record;
mod stdlib;
mod syntax;
mod tree;

pub use agents::{
    orchestrate, orchestrate_parallel, orchestrate_with_limits, orchestrate_with_observer,
    orchestrate_with_runtime, ExecutionEvent, Firing, FlowDirection, ModuleResolver, Orchestration,
    RuntimeDiagnostic, RuntimeLimits, MAX_CONCURRENCY, MAX_MACRO_EXPANSIONS, MAX_MODEL_CALLS,
    MAX_MODULE_IMPORTS,
};
pub use eval::{eval, holonomy, holonomy_reflected, parse_embedded, reflect, run, Oracle};
pub use record::{content_tokens, Concept, Record};
pub use stdlib::std_modules;
pub use syntax::{
    format, parse, pretty_format, tokens, Error, Expr, InvalidModuleName, ModuleName, Token,
    TokenKind, MAX_SOURCE_BYTES, MAX_SYNTAX_DEPTH,
};
pub use tree::{mandala, tree, tree_scored};

/// The visual identity of the language: two circles and the square between.
pub const REBIS_SIGIL: &str = "o-[]-o";
