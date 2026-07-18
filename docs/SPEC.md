# Rebis 0.0.1 language specification

```text
program := expr+ EOF
expr    := prompt | symbol | '\'' expr | ',' expr | '(' form ')'
prompt  := '"' characters '"'
form    := '~' symbol '(' symbol* ')' expr
         | '#' module
         | '<' expr '>' expr+
         | '$' expr+
         | '->' expr expr+ | '<-' expr expr+
         | symbol expr* | expr+
comment := ';' characters-to-end-of-line     (whitespace; literal in prompts)
```

Two or more top-level expressions form an implicit program sequence. This
sequence is not parenthesized: its definitions share program scope and its
executable forms run in source order. Parentheses retain their existing group,
call, arrow, square, definition, and import meanings.

A quoted string is the only form that fires a model. Escapes include `\\"`,
`\\\\`, `\\n`, `\\r`, and `\\t`; every other backslash escape is a syntax
error rather than an implicit rewrite. A bare atom is a symbol, never a prompt.
A `;` outside a quoted prompt begins a comment extending to the end of the
line; the lexer treats it exactly as whitespace (it also terminates a word).
Comments are not represented in the syntax tree: formatting emits canonical
source without them, and inside a quoted prompt `;` is ordinary prompt text.

`(~ name (parameter ...) body)` installs a macro abstraction in its containing
group. `(name argument ...)` expands it. Arguments remain unevaluated Rebis
syntax; application performs structural,
capture-avoiding substitution and never interpolates prompt text. Calls supply
exactly one argument per parameter. Parameter names within one definition must
be unique.

A quoted macro body emits syntax rather than executing it. Within that quote,
`,` splices an argument expression into the emitted tree:

```rebis
(
  (~ twice (work)
    '(-> ,work ,work))

  (twice "Improve this code."))
```

This expands to `(-> "Improve this code." "Improve this code.")`. Quote and
unquote operate on parsed syntax nodes; they never interpolate characters into
a raw prompt. An unquote outside a quoted macro template is inert and invalid
as an executable program.

Macros are higher-order. If a parameter occurs in call-head position, its
argument must be a bare macro symbol and becomes the callee. Thus
`(~ apply (worker x) '(,worker ,x))` makes `(apply inspect "parser")` expand to
`(inspect "parser")`. Rebis has named macros but no anonymous
abstractions, closures, currying, or partial application.

`(# module)` imports a definition-only module through the host's
`ModuleResolver`. A module contains one `~` definition, or a group containing
only `~` definitions and other `#` imports. Imports are lexical, cached per run,
and may re-export definitions from nested imports. Later definitions/imports in
the same group shadow earlier names. Module names are validated
`/`-separated paths such as `engineering` or `std/loops`; each segment contains
ASCII letters, numbers, `-`, or `_`.

The core performs no I/O. Missing modules, resolver failures, invalid module
bodies, cycles, and the 64-module import limit are typed runtime diagnostics.
This resolver boundary allows Kaos hypersigils, embedded standard-library
modules, and future package stores to share the same language semantics.

`([M] A B ...)` runs its branches in source order and supplies their answers to
mediator program `M` as labeled `RESULT n` blocks. `(-> A B)` routes answers
from `A` to `B`. `(<- B A)` expresses the same flow from the consumer side:
`(<- B A) ≡ (-> A B)`.

## String composition

`($ A B ...)` **interpolates** its operands into one string and yields that
string. It is pure text construction — nothing inside `$` fires or runs. Each
operand contributes text:

- a **prompt** contributes its exact characters;
- a **symbol** contributes its name (after macro substitution, a bound symbol is
  the prompt it was bound to, so it contributes that prompt's characters);
- a **macro call** contributes its *expanded text* — it is not fired;
- a **nested composition** contributes its assembled text;
- any **other expression** is a program, not a string, so it contributes nothing
  and does not run (a computed value reaches a prompt through `->`, not `$`).

Composition is an operator, not in-string interpolation: it never inspects the
interior of a quoted string, so a `$` written inside a prompt is ordinary prompt
text. The assembled string is itself a prompt in the position the `$` sits, so
it fires there (a group child, a branch, an arrow producer) and stays inert as
an operand of an enclosing `$`. Interpolation adds no model calls.

There is no binding keyword: **variables are macro parameters.** A macro
`(~ f (a b) body)` binds `a` and `b` lexically in `body`, and a call `(f x y)`
supplies the values, which may be reused freely — woven as text by `$` — with no
extra model call per use. A **text constant** is a macro whose body is a prompt;
because `$` interpolates a macro's text without firing it, `(~ topic () "the
fall of Rome")` is usable directly inside `$` with no quoting.

## Quote

Quote (`'`) marks a macro body as an output **template**: the quoted syntax is
returned by expansion instead of executing during it, and `,` (unquote) splices
caller syntax into it. The expanded template then runs. This is `'`'s only role;
there is no separate "data string" — a text constant is an ordinary prompt-bodied
macro (see *String composition*).

Definitions and arrows do not invoke a model themselves. An arrow always has at
least two operands; a one-operand arrow is a syntax error rather than a no-op.
Hosts must bound calls, generated text, and time.
Repeated parameters can duplicate model firings, so expanded call count must be
treated as a host-controlled resource. The reference orchestrator permits at
most 256 macro expansions per run. The reference parser accepts at most 1 MiB
of source and 256 levels of structural or prefix nesting.

## Runtime diagnostics and events

Syntactically valid programs may still fail at runtime. The reference
orchestrator reports typed diagnostics for undefined macros, arity mismatches,
macro-expansion exhaustion, invalid conditional results, and host oracle
failures. These are never silently converted into successful `nothing` values.

Execution also emits typed events for prompt start/completion, routed arrow
values, mediator starts, conditional branch selection, macro expansion, and
diagnostics. `orchestrate_with_observer` delivers each event synchronously so a
host can render live progress. The returned `Orchestration` retains the same
events and diagnostics for reproducibility.

`RuntimeLimits` lets a host set macro-expansion, module-import, and model-call
budgets. The reference defaults are 256 expansions, 64 distinct module loads,
and 1,024 model calls. A call is rejected before crossing the `Oracle` boundary
when its budget is exhausted.

## Lazy conditional mediation and loops

A two-branch square whose mediator is a macro call is a lazy conditional:

```rebis
([(condition value)] when-yes when-no)
```

The mediator call runs first. Its final model answer must be exactly `yes` or
`no` (ignoring case and surrounding whitespace). Only the selected branch is
executed. A non-boolean or missing answer selects neither branch and emits an
`InvalidCondition` diagnostic. All other squares retain
the ordinary behavior: branches run first and their ordered answers are passed
to the mediator.

Because a `~` body may call its own name, lazy conditional mediation is enough
to express runtime loops without a recursion operator:

```rebis
(~ loop (value step stop)
  ([(stop value)]
    value
    (loop (step value) step stop)))
```

## Deterministic mediation

A square whose bracketed mediator is **pure symbols** — a symbol, or a compose
of symbols, containing at least one symbol and no other node — does not run
its mediator as a program and fires no model for mediation. The calculus
judges instead:

1. Branches run as usual and their accepted answers are collected in source
   order (absent answers and `nothing` are dropped).
2. The mediator's symbols, tokenized by the calculus, are the judged task.
   The idiomatic spelling is a single hyphenated symbol
   (`[verified-root-cause]`), since the tokenizer splits on non-alphanumeric
   characters; a multi-word head such as `[(check value)]` parses as a call
   and is therefore conditional or ordinary mediation, never deterministic.
3. Each accepted answer is reflected to its content tokens and transported
   back onto the task through a record built from the accepted answers'
   own lines. The result is the answer's holonomy in `0..=1`.
4. The lowest holonomy wins; ties resolve to source order. An answer at
   exactly `1.0` cannot round-trip and is refused. The winning answer is the
   square's result and a `MediatorResolved` event reports its 1-based index
   and holonomy percentage. When every answer is refused the square yields
   no output.

The runtime authors no prompt text anywhere: a model only ever receives
quoted program prompts, plus the documented `INPUT:` / `RESULT n:` transport
labels that carry routed answers.

## Concurrent branch evaluation

Ordering in Rebis is expressed by arrows, and only by arrows. The branches of
an ordinary square are therefore semantically unordered and mutually isolated:
each evaluates against a snapshot of the record with a branch-scoped
definition table, and no branch can observe a sibling's answers or
definitions. A runtime may evaluate them concurrently.

The reference runtime exposes this through `orchestrate_parallel`, bounded by
`RuntimeLimits::with_max_concurrency` (default 4, applied per square). The
orchestration's observable structure is independent of the schedule: firings,
events, diagnostics, and the mediator's `RESULT n` labels always merge in
branch source order, and a concurrency bound of `1` is byte-identical to
sequential evaluation. Model-call, expansion, and module budgets are shared
across concurrent branches.

Sequential forms stay sequential: arrow stages (each consumes the previous
answer), group children (source order), and the lazy conditional (condition
first, then only the selected branch).
