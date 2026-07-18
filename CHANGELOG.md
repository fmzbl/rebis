# Changelog

All notable changes to Rebis are documented here. The project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

- **String composition `($ A B …)`.** The one operator over the language's
  fundamental value. It *interpolates* its operands into one string and yields
  that string — pure text construction, nothing inside `$` fires or runs. An
  operand contributes text: a prompt its characters, a symbol its bound value, a
  macro its expanded text (not fired), a nested `$` its assembled text; any other
  (a program) contributes nothing. The assembled string is a prompt in the
  position the `$` sits, so it fires there, once. It never peeks inside a quoted
  string: a `$` in a prompt (`"it cost $100"`) stays literal. Interpolation adds
  no model calls. **Variables are macro parameters** — no binding keyword — and a
  text constant is a macro whose body is a prompt, usable in `$` without quoting.
- **Mediator delimiters are `<` and `>`.** A mediator square is written
  `([M] A B …)` (was `([M] …)`); the identity sigil is now `o-[]-o`. The arrows
  `<-`/`->` are unchanged and coexist with the mediator delimiters.

## [0.0.1] - 2026-07-16

Initial open source release.

The language: quoted strings are the only model prompts; bare atoms are
symbols; `->` and `<-` route real answers; `([M] A B …)` runs branches and
mediates their results; `~` defines structural macros with quote/unquote and
higher-order calls; `(# module)` imports definition-only modules; `;` starts
a line comment.

What ships in this release:

- **Lisp-style source files.** Multiple top-level forms need no redundant outer
  group. Definitions share the implicit program scope, and canonical and pretty
  formatting preserve that program boundary.

- **A pure core.** The runtime writes no prompt text of its own. A model
  only ever receives quoted program prompts plus the documented `INPUT:` /
  `RESULT n:` labels that carry routed answers.
- **Deterministic mediation.** A square whose mediator is pure symbols
  (`([verified-root-cause] A B)`) judges its branches without any model
  call: each answer is reduced to its content tokens and scored by how well
  it round-trips onto the mediator's tokens. The best answer wins.
- **Concurrent squares.** Square branches are unordered and isolated, so
  `orchestrate_parallel` evaluates them in parallel, bounded by
  `RuntimeLimits::with_max_concurrency`. Results, events, and labels always
  merge in source order; a bound of 1 is byte-identical to sequential runs.
- **The embedded standard library.** Fourteen definition-only modules
  (51 macros) under the reserved `std/` namespace, compiled into the crate
  and documented inline: flow, spread, map, gate, loops, evolve, debate,
  dialectic, search, tournament, reflexion, committee, and the two phrasal
  modules canon and shape. Structural modules contain no prompt text
  (enforced by test). `(# std)` imports the whole folder, while an exact leaf
  import remains available. `std_modules()` lists the sources for host tooling.
- **A deterministic record calculus** (`eval`, `run`, `reflect`,
  `holonomy_reflected`) for scoring text against evidence without a model.
- **Typed diagnostics and live events** for every runtime problem and
  execution step, with hard budgets for expansions, module loads, model
  calls, and concurrency.
- **Zero dependencies.** The crate is std-only and builds offline; hosts
  supply the model behind the one-method `Oracle` trait.
