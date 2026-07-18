# The Rebis standard library

The sources are the authority: fourteen
definition-only modules at [`src/std/`](../src/std), embedded in the crate via
`include_str!`, each documented inline with `;` comments — purpose, parameter
contract, and cost notes per macro. This document covers the design rules,
the resolution architecture, and what each module is for.

```rebis
(
  ; promptless best-of-three, straight from the library
  (# std/spread)
  (best-of-three tested-plan "Write the migration plan."))
```

## 1. What std is

- **Vocabulary, not power.** Everything in std is expressible by any user;
  std exists so the orchestration shapes everyone rebuilds have canonical,
  tested, documented names — complex agent systems read as a sentence of
  named strategies rather than pages of squares and arrows.
- **Same-everywhere.** `(# std/…)` resolves identically under every host,
  offline: every orchestration entry point consults the embedded library
  before the host resolver, and the `std/` namespace never falls through
  (an unknown std name is `ModuleNotFound`; a host module claiming a std
  name is provably ignored — see `tests/std_library.rs`).
- **The language stays closed.** No new syntax, no evaluator hooks. Each
  module is an ordinary definition-only module; `std/tournament` importing
  `std/spread` uses the same nested-import re-export as any hypersigil.
- **Folder import.** `(# std)` imports all fourteen modules in stable inventory
  order. It is ordinary nested-import expansion, so the usual cache, cycle,
  event, definition-shadowing, and module-budget rules still apply.

## 2. Design rules

1. **Structural modules are prompt-free.** Their expansions contain no quoted
   prompt text — workers, judges, and phrasings arrive as caller parameters.
   This extends the purity rule (the runtime writes no prompt text)
   one layer up. Exactly two modules are phrasal and exempt — `std/canon`
   (judgment protocol) and `std/shape` (answer contracts) — and prompt text
   may never appear anywhere else in std. Enforced by test:
   `structural_modules_contain_no_prompt_text`.
2. **Judge-neutral combinators.** Wherever a macro takes a `judge` (or
   `merge`, `reconcile`, …), a **symbol** yields deterministic mediation —
   promptless and free — and a **prompt** yields
   prompted mediation. One shape, both worlds.
3. **Structure over policy.** Thresholds, budgets, model choice, and retry
   policy stay host/program concerns.
4. **Costs are documented at the definition.** Structural substitution
   re-executes a spliced argument wherever it appears, so compact source
   multiplies model calls; every macro whose expansion fires a parameter
   more than once carries a `; Cost:` note in its source.

## 3. The inventory — 14 modules, 51 macros, three tiers

| Tier | Module | Macros | For |
|---|---|---|---|
| kernel | `std/flow` | `apply` `compose` `flip` `twice` `fan` `fan-three` | application, composition, heterogeneous fan-out |
| kernel | `std/spread` | `best-of-two` `-three` `-five` | verified best-of-k; promptless with a symbol judge |
| kernel | `std/map` | `map-two` `map-three` `zip` | static decomposition: one worker over many inputs, many workers over one |
| kernel | `std/gate` | `gate` `or-else` `audit` | the holonomy triangle as control flow: on-topic guard, tie-break fallback, independent re-derivation |
| kernel | `std/loops` | `loop` `stabilize` | judged iteration; convergence when refinement stops changing the answer |
| strategy | `std/evolve` | `evolve` `ascend` | judged hill-climbing — the self-improvement kernel, pure |
| strategy | `std/debate` | `debate` `panel-three` `cross-examine` `red-team` | adversarial structures |
| strategy | `std/dialectic` | `diamond` `reversible` `synthesis` `face` `anneal` | the reconciliation geometry; self-similar growth |
| strategy | `std/canon` | `yes-no` `agreed` `reconciled` `invert` `steelman` | **phrasal** — judgment protocol |
| strategy | `std/shape` | `final-only` `one-word` `stepwise` `with-evidence` | **phrasal** — answer contracts |
| search | `std/search` | `try-else` `route-two` `route-three` `dnc` `tot` | lazy fallback, lazy specialist routing, divide-and-conquer, tree-of-thoughts descent |
| search | `std/tournament` | `tournament-four` `playoff` `consensus-three` (+ re-exports `std/spread`) | bracket reduction; consensus-as-merge |
| search | `std/reflexion` | `reflexion` `exchange` `duel` | critique-and-retry; symmetric multi-round debate |
| search | `std/committee` | `chaired-panel` `quorum` `campaign` | criteria-fed panels, lazy agreement gates, plan–execute–review |

Mechanics the search tier leans on, all ordinary language semantics:

- **Laziness is the conditional.** `try-else`, `route-*`, `dnc`, `quorum`
  derive from the language's only lazy form — the unselected branch never
  expands, so routing runs one specialist and backtracking pays for the
  fallback only on rejection.
- **INPUT flows through arrows into squares.** `chaired-panel`'s chair
  answer reaches every panelist branch as input — verified by test.
- **Recursion is bounded by judgment plus budgets.** `dnc` and `tot` are
  exponential-shaped by design; the simple/stop judges terminate them and
  the expansion/model-call budgets are the hard backstop.

Composition example (validated):

```rebis
(
  (# std/evolve) (# std/canon)
  (~ improve (v) (-> v "Rewrite this plan to be more testable."))
  (~ done (v) (yes-no (-> v "Is every step independently verifiable?")))
  (ascend "Draft: migrate the billing tables." improve testable-plan done))
```

## 4. Resolution architecture (as implemented)

`src/stdlib.rs`: a `(name, source)` table via `include_str!`, wrapped around
the host resolver at every entry point —

```rust
match embedded(name) {
    Some(source) => Ok(Some(source)),
    None if is_std(name) => Ok(None),   // reserved: ModuleNotFound, no fall-through
    None => self.0.resolve(module),     // everything else is host policy
}
```

`embedded("std")` synthesizes a definition-only group containing one import
for every inventory entry. Exact leaf names still resolve directly.

- `std_modules()` exposes the `(name, source)` list for host tooling
  (completion, search, docs).
- Embedded resolutions count against the module budget and are cached like
  any module.
- kaos refuses `/sigil save std/...` so no dead files shadow-in-appearance.

## 5. Scope and collision semantics

Registration is **source-order, last wins** (`Functions::extend`): a user `~`
after `(# std/flow)` shadows std's `twice`; an import after a user definition
overwrites it. Simple and lexical; kept. Future work: a
`DefinitionShadowed { name, module }` event so silent shadowing is visible in
the trace.

## 6. Versioning

std is pinned to the crate version; CHANGELOG entries under `### std` record
macro additions and phrasing changes. Renames/removals are semver-major.
Phrasal edits (`std/canon`, `std/shape`) are semver-minor but changelogged
individually — they alter what models receive.

## 7. Test matrix (tests/std_library.rs)

Build gate (every module parses + imports cleanly, inventory counts pinned,
prompt-free rule enforced); namespace (std beats a hijacking host resolver;
unknown std names never fall through); execution under scripted oracles for
every tier — promptless best-of-k call counts, gate refusal and or-else
tie-break, loop termination and re-execution costs, deterministic `evolve`,
`debate`+`canon` composition, shape contracts, routing laziness (the
unselected specialist provably never fires), the tournament bracket (four
attempts, three deterministic mediations, zero judge calls), reflexion's
critique-as-input retry, and chaired-panel criteria delivery to every branch.

## 8. Known limits

- **No dynamic decomposition.** The language cannot destructure a model
  answer into branches; `std/map` parts are static syntax, and `dnc` splits
  by caller *strategy*, not by model output.
- **No meta-prompting.** Answers route as INPUT; they can never become
  prompt text. A self-rewriting-prompt loop is not expressible, by design.
- **No `race`/`first`.** Cancellation doesn't exist in the runtime; a
  combinator pretending otherwise would lie.
- **Deterministic judges are topical gates,** not correctness oracles;
  mechanical fitness (tests, compilers) remains host territory.
- **Arity families instead of numerals** (`best-of-five`, `map-three`);
  larger tiers only on demonstrated need.
