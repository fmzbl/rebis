# Rebis symbol reference

Every symbol in the language, what it means, how it renders, and how the
symbols combine into patterns. The narrative manual is [`GUIDE.md`](GUIDE.md);
the formal grammar and semantics are in [`SPEC.md`](SPEC.md); host
integration is in [`HOSTS.md`](HOSTS.md). With Kaos, the reference host, any
example can be tried directly:

```bash
kaos rebis run --dry '<program>'   # execute without a model (answers are "nothing")
kaos rebis tree '<program>'        # structural AST
kaos rebis mandala '<program>'     # whiteboard o-[]-o projection
kaos rebis run <file-or-program>   # live, with the bound model
```

Complete programs below run as-is; snippets that reference undefined names
(`draft`, `branch-a`, …) illustrate shape and want surrounding definitions.

The full token set: `(` `)` `[` `]` `~` `#` `'` `,` `->` `<-` `;` and the
double quote. `;` starts a line comment (whitespace to the language; literal
inside quoted prompts). There are no infix operators.

---

## The symbols

### `"…"` — prompt

The only form that fires a model. Everything inside the quotes belongs to the
prompt — spaces, punctuation, casing. Escaped quotes, backslashes, `\n`, `\t`,
and `\r` are supported.

```rebis
"Fix parser.rs: preserve UTF-8, brackets, and quoted text!"
```

When an arrow or mediator supplies data, the runtime appends an `INPUT:`
section to the effective prompt. The quoted source text is **never**
interpolated or rewritten — a macro parameter named `target` does not alter
the string `"describe target"`.

Renders as `○` in the tree, `o "prompt"` in the mandala.

### bare word — symbol

A Lisp-like atom. Never sent to a model. Symbols name macro parameters, macro
heads, and modules; inside a macro body a parameter symbol is substituted with
the caller's syntax.

```rebis
target
inspect
std/loops
```

Renders as `◇`.

### `( )` — group

A structural and abstraction boundary. Children execute one abstraction level
above their containing node (the firing trace shows this as `abstraction N`),
**in source order** — a group is the sequential composition form. But a group
is not a pipeline: children run independently, and if answers must flow, use
an arrow.

```rebis
(
  "Inspect the parser"
  "Inspect the tokenizer"
  "Inspect the tests")
```

Definitions (`~`, `#`) inside a group are registered before the group's
executable forms run, so order of definition versus use does not matter within
one group. Renders as `◌ group`.

### `->` — forward arrow

Routes actual answers left to right. Each stage's accepted answer becomes the
next stage's `INPUT:`. The arrow itself never calls a model — only the prompts
at its stages do. The value of the whole expression is the **last** stage (the
consumer).

```rebis
(->
  "Reproduce the bug"
  "Trace the failing execution path"
  "Write a root-cause report")
```

Renders as `→`.

### `<-` — backflow arrow

The same routing with the consumer written first. The governing equivalence:

```text
(-> A B) = (<- B A)
```

The right side runs first and flows into the left; the value is the **left**
operand. Use it to pin the deliverable's phrasing up front and hang the
grounding work behind it — the shape downstream mediators judge is then the
goal you wrote, not whatever a pipeline's tail happened to say.

```rebis
(<-
  "Write a root-cause report"
  "Trace the failing execution path")
```

Renders as `←`.

### `([M] A B …)` — mediator square

Convergence. `M` is **executable Rebis code**, not a special string, and the
remaining expressions are branches. Execution order:

1. every branch runs, in source order;
2. absent answers and the refusal `nothing` are dropped;
3. accepted answers are labeled `RESULT 1`, `RESULT 2`, … in execution order;
4. `M` executes with those results as its `INPUT:`;
5. the square's value is `M`'s result.

There is no hidden mediator prompt. `M` may be a single prompt or a whole
program:

```rebis
([(->
    "Compare every branch result"
    (<- "Resolve contradictions"
        "Check each claim against the supplied evidence")
    "Write the final decision")]
  "Inspect the code"
  "Trace the failure"
  "Run the tests")
```

Squares nest freely. The `RESULT n` labels preserve ordering but are not
variables reachable from source. Renders as `□ mediator square`; the mandala
shows `[M: code]`.

Branches are semantically **unordered and mutually isolated** — no branch can
observe a sibling's answers or definitions. Under `orchestrate_parallel`
(which Kaos uses) they evaluate concurrently, bounded by
`with_max_concurrency`; results, events, and `RESULT n` labels always merge in
source order, so the structure is deterministic regardless of schedule. Groups
and arrows stay sequential — in Rebis, ordering is expressed by arrows.

### `([symbol] A B …)` — deterministic mediation

A mediator of **pure symbols** fires no model: the calculus judges the
branches. Each accepted answer is reflected to its content tokens and
round-tripped onto the mediator's tokens through the answers' own record;
the best-closing answer becomes the square's value (ties → source order;
an answer that cannot round-trip is refused; all refused → no output).

```rebis
([verified-root-cause]
  "Trace the failure backward from the symptom"
  "Inspect recent changes for the earliest wrong state")
```

Spell the mediator as a **single hyphenated symbol** — the tokenizer splits
`verified-root-cause` into three tokens, whereas `[(check value)]` parses as
a call (the conditional form). The trace reports the choice as a
`MediatorResolved` event with the winner's index and holonomy percentage.

### `([(cond …)] yes no)` — the conditional square

A deliberately narrow special case: a square with **exactly two branches**
whose bracketed mediator is a **macro call** becomes a lazy conditional. The
bracketed call expands and executes first; its final answer must be exactly
`yes` or `no` (case and surrounding whitespace ignored); only the selected
branch then expands and executes. Anything else — a prompt mediator, one
branch, three branches — keeps normal branch-first mediation.

```rebis
([(settled draft)]
  draft
  (revise draft))
```

The laziness is the point: the unselected branch never expands, which is what
makes recursive macros terminate (see Patterns → the loop). A wrong-shaped
answer produces the diagnostic `conditional mediator must return 'yes' or
'no'`.

### `~` — macro definition

`(~ name (parameter …) body)` defines a named abstraction by **structural
substitution**: arguments are syntax trees, never strings. Parameters are
positional and may be any expression — prompts, arrows, squares, groups,
symbols, other calls.

```rebis
(
  (~ inspect (target)
    (-> target "Analyze the evidence" "Write a report"))
  (inspect "Inspect the parser"))
```

A parameter may occur more than once in the body; each occurrence substitutes
and executes, so compact source multiplies model calls:

```rebis
(~ examine-twice (task)
  (["Compare both attempts"]
    (-> task "Use a static-analysis perspective")
    (-> task "Use a testing perspective")))
```

Renders as `λ function` in the tree, `~[f(x)]` in the mandala; a call site is
`@ call` / `[f]`.

### `'` — quote (macro templates)

`'` marks a macro body as an output **template**: the quoted syntax is returned
by expansion instead of executing during it, and `,` (unquote) splices caller
syntax into it.

```rebis
(~ twice (work)
  '(-> ,work ,work))
```

`(twice "Inspect and improve this code.")` expands to the two-stage arrow with
the prompt duplicated, which then runs. Renders as `' quoted syntax`.

(There is no separate "data string" form. A text constant is just a macro whose
body is a prompt — `(~ topic () "the fall of Rome")` — because `$` interpolates
a macro's text without firing it; see `$` below.)

### `,` — unquote

Inside a quoted template, `,x` splices the caller's syntax for `x` — always a
complete parsed expression, never characters inside a prompt string. An
unquote in call-head position enables higher-order macros:

```rebis
(~ apply (worker value)
  '(,worker ,value))
```

Renders as `, unquote`.

### `$` — string composition

`($ A B …)` **interpolates** its operands into one string and yields that
string. It is pure text construction — nothing inside `$` fires or runs:

- a prompt contributes its characters;
- a symbol contributes its name (after substitution, a bound parameter's value);
- a macro call contributes its **expanded text** — it is *not* fired;
- a nested `$` contributes its assembled text;
- any other operand is a program, not a string, so it contributes nothing
  (a computed value reaches a prompt through `->`, not `$`).

It is an operator, not in-string interpolation — it never looks inside a quoted
string, so a `$` in a prompt (`"it cost $100"`) is literal. The assembled string
is itself a prompt in the position the `$` sits, so it fires there, once.

```rebis
(~ case (self rival)
  ($ "Make the case that " self " beats " rival "."))
```

Variables are macro parameters: `case` binds `self` and `rival`, and a call
supplies the values — reused freely, woven as text, with no extra model call:

```rebis
(["Deliver the verdict on the greater hip-hop legacy."]
  (case "Jay-Z" "Kanye West")
  (case "Kanye West" "Jay-Z"))
```

A text constant is just a macro whose body is a prompt; `$` weaves its text in
without firing it:

```rebis
(~ era () "the streaming era")
($ "Rank the rappers of " (era) ".")   ; one call — (era) is text here, not fired
```

Renders as `$ composition`.

### `(f arg …)` — macro call, including higher-order

Calling a macro substitutes and executes its expansion. A named macro symbol
can itself be an argument and be invoked through a parameter in call-head
position — separating orchestration topology from worker strategy:

```rebis
(
  (~ apply-to-both (worker left right)
    (["Combine both worker results"]
      (worker left)
      (worker right)))
  (~ investigate (task)
    (-> task "Find the root cause" "Propose a safe fix"))
  (apply-to-both
    investigate
    "Inspect the parser failure"
    "Inspect the tokenizer regression"))
```

Rebis 0.5 has named macro symbols as arguments but **no** anonymous
abstractions, closures, currying, partial application, mutable variables, or
pattern matching.

### `#` — module import (hypersigils)

`(# name)` imports a definition-only module: its top-level `~` definitions
join the containing lexical scope; nested imports re-export. Imports never
execute prompts, and module source is cached for the run. The host resolves
the symbolic name — Kaos maps `name` to the saved hypersigil
`~/.kaos/sigils/name.rebis` (`/sigil save name`); qualified paths like
`std/loops` are valid. A resolver may also treat a name as a folder by returning
a definition-only module of child imports. Kaos does this recursively for
saved-sigil folders, while the core resolves `(# std)` to all fourteen embedded
standard-library modules. An exact module takes precedence over a same-named
folder. Cycles, missing modules, executable module bodies, and the 64-module
bound produce typed diagnostics.

```rebis
(
  (# engineering)
  (repair "Fix the cancellation lifecycle"))
```

A module is one `~` definition or a group containing only definitions and
nested imports.

### `nothing` — the refusal word

Not a token but a protocol convention: an oracle answering exactly `nothing`
(case-insensitive) — or not answering — is a refusal. Refusals stay visible in
the firing trace but are dropped by arrows and mediators; they are never
forwarded as answers. A provider failure (auth, transport, timeout) is **not**
a refusal — it surfaces as a typed diagnostic instead.

---

## The value path

Every form has a defined result, which is what `output` follows and what an
enclosing arrow or square consumes:

| Form | Value |
|---|---|
| `"prompt"` | its accepted answer |
| `($ A B …)` | the accepted answer to the composed prompt |
| `(-> A B)` | `B`, the right consumer |
| `(<- B A)` | `B`, the left consumer |
| `([M] A B …)` | the mediator `M`'s result |
| `([symbol] A B …)` deterministic | the best round-tripping branch answer |
| conditional square | the selected branch only |
| `(f …)` | its expansion's value |
| `( … )` group | its final executable form |

---

## Combinations and patterns

### The pipeline — work first, deliverable last

```rebis
(-> "Reproduce" "Diagnose" "Repair" "Verify")
```

Plain sequence. The value is the last stage. Use when the interesting phrasing
is naturally at the end.

### The pinned deliverable — goal first, grounding behind

```rebis
(<- "Deliver a verdict: ship or hold, with the one controlling reason"
  (-> "List the riskiest changes"
      "State the failure each could cause"))
```

`(-> A B) = (<- B A)`, so this is the same routing as the pipeline — but the
deliverable prompt is written (and judged) first-class. The dominant idiom for
anything a mediator will compare.

### Fan out, converge

```rebis
(["Fuse the accounts into one explanation"]
  "Inspect the implementation"
  "Run the relevant tests"
  "Search for related regressions")
```

Independent branches, ordered results, one mediator. Nest squares for
sectioned reports (a square of squares).

### The program-mediator

The judge itself can be an arrow chain ending in a pinned verdict:

```rebis
([(-> "Reconcile the branches"
      (<- "Write the decision as one paragraph"
          "List what would change the decision"))]
  branch-a branch-b)
```

### The diamond — direction as semantics

The same two thoughts flowed in both orientations, then reconciled. Only
expressible because flow direction is first-class syntax:

```rebis
(~ diamond (premise consequence)
  '(["Do both orientations tell the same story? Reconcile them"]
    (-> ,premise ,consequence)
    (<- ,premise ,consequence)))
```

### The reversal guard

A claim asserted only after its own inversion backflows into it:

```rebis
(~ reversible (claim)
  '(<- ,claim
     (-> "Invert the claim"
         "Derive what the inversion would predict")))
```

### The loop — recursion + lazy conditional

There is no loop form and no recursion operator; there are recursive macros
whose unselected branch never expands:

```rebis
(
  (~ improve (current)
    (-> current "Improve this implementation once."))
  (~ finished (current)
    (-> current "Is it finished? Answer exactly yes or no."))
  (~ loop (current step stop)
    ([(stop current)]
      current
      (loop (step current) step stop)))
  (loop "The original implementation" improve finished))
```

`loop` is higher-order: `step` and `stop` are strategies passed by name. The
model's own answer is the termination condition; the 256-expansion budget is
the hard bound behind it.

### Self-similar growth — the graph decides its own depth

Combine the loop with a structure-producing macro and the *whole structure*
becomes the loop variable — a fixed, finite text describing a runtime-decided
topology:

```rebis
(~ anneal (structure)
  ([(settled structure)]
    structure
    (anneal (deepen structure))))
```

Each round, `deepen` wraps the entire lattice built so far; `anneal` re-enters
until the model reports it settled.

### The deterministic gate

Fan out, then let the calculus keep the answer that stays on task — no judge
prompt, no mediation call, unfoolable by rhetorical framing:

```rebis
(
  (~ attempt (task)
    '([staged-rollback-plan]
      (-> ,task "Write the plan, stating each stage explicitly")
      (-> ,task "Write the plan, working backward from the rollback")))
  (attempt "Design the migration for the billing tables"))
```

Both drafts fire; whichever round-trips onto `staged-rollback-plan` more
faithfully is the square's value. Compose with `spread`-style repetition
(repeat the branch) for promptless best-of-k.

### Topology / strategy separation

Keep the shape of collaboration in one macro and the worker's method in
another, and combine them at the call site (`apply-to-both` above). Swap the
worker without touching the topology; swap the topology without touching the
worker.

### The hypersigil library

Factor stable macros into definition-only modules, save them as sigils
(`/sigil save base`), and import per program — here assuming a saved
`~/.kaos/sigils/base.rebis` defining `twice`:

```rebis
(
  (# base)
  (~ verify (work) '(-> ,work "Verify the result"))
  (verify (twice "Draft the summary")))
```

Modules re-export their nested imports, so a project can build a layered
standard library of orchestration patterns. When the host supports folder
resolution, importing the folder name is shorthand for importing all of its
definition-only leaves; `(# std)` is the portable built-in example.

---

## Operational limits

Small source expands into many calls — repeated parameters and higher-order
macros multiply firings, and recursion multiplies them again. `RuntimeLimits`
configures the budgets; the defaults, and the environment overrides Kaos
exposes for them:

| Budget | Default | Kaos override |
|---|---|---|
| macro expansions | 256 | `KAOS_REBIS_MAX_EXPANSIONS` |
| module imports | 64 | `KAOS_REBIS_MAX_MODULES` |
| model calls | 1024 | `KAOS_REBIS_MAX_CALLS` |
| concurrent branches per square | 4 | `KAOS_REBIS_MAX_CONCURRENCY` |

Exhausting a budget produces a typed diagnostic (`ExpansionLimit`, …), never a
silent return.

---

## Mandala and tree alphabet

| Tree glyph | Mandala | Meaning |
|---|---|---|
| `○` | `o "prompt"` | prompt / value terminal |
| `◇` | `◇ symbol` | symbol |
| `◌` | — | group |
| `→` / `←` | `─→─` / `─←─` | forward / backflow answer routing |
| `□` | `[M: code]` | mediator square |
| `λ` | `~[f(x)]` | macro definition (template) |
| `@` | `[f]` | macro call (expanded) |
| `'` | `'` | quoted syntax |
| `,` | `,` | unquote |

`/tree` shows the structural AST; `/mandala` the whiteboard projection; the
firing trace shows prompts, routed values, expansions, branch selections, and
diagnostics as they happen.

---

## Gotchas

- **Comments vanish under `/format`.** `;` comments are whitespace, not
  syntax — the formatter emits canonical source without them. Keep commentary
  you must preserve in module doc headers you don't reformat.
- **`;` inside a prompt is prompt text.** Only a `;` outside quotes starts a
  comment.
- **A group is not a pipeline.** Sibling prompts in a group run independently;
  routing requires an arrow.
- **The conditional is narrow.** Exactly two branches and a *macro-call*
  mediator; a prompt mediator or any other branch count mediates normally.
  The condition's answer must be exactly `yes` or `no`.
- **Multi-word bracket heads are calls, not symbol lists.** `[(parser
  benchmark)]` parses as the call `(parser benchmark)` — the conditional
  form — not as two symbols. For deterministic mediation write one
  hyphenated symbol: `[parser-benchmark]` (the tokenizer splits it).
- **Prompts are opaque.** Parameters substitute expressions around prompts,
  never text inside them.
- **`RESULT n` labels are not variables.** They order the mediator's input;
  source code cannot reference them.
- **Quoted templates need `,`.** In a `'` body a bare parameter name stays a
  literal symbol; only `,name` splices the argument.
- **Budget the blast radius.** `(spread …)` does not exist in Rebis — but a
  repeated parameter is a quiet multiplier, and recursion compounds it; set
  the `KAOS_REBIS_MAX_*` limits in production.
