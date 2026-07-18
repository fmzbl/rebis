# The Rebis Guide

Rebis is a compact S-expression language for constructing systems of
language-model agents. Its source is simultaneously an execution plan, an agent
hierarchy, and a graph that Kaos can visualize.

Its six central ideas are:

1. Quoted strings are raw model prompts.
2. Bare atoms are Lisp-like symbols.
3. `~` defines named macro abstractions by structural substitution.
4. `->` and `<-` route actual answers between programs.
5. `[M]` runs branches and executes mediator program `M` over their results.
6. `(# module)` imports reusable abstractions without giving the core I/O.

## 1. Grammar

```text
program := expr+ EOF
expr    := prompt | symbol | '\'' expr | ',' expr | '(' form ')'
prompt  := quoted string
form    := '~' symbol '(' symbol* ')' expr
         | '#' module
         | '<' expr '>' expr+
         | '$' expr+
         | '->' expr expr+ | '<-' expr expr+
         | symbol expr* | expr+
```

Whitespace separates forms but otherwise has no meaning. Like a Lisp source
file, a program may contain multiple top-level forms without a surrounding
group. Those forms share one lexical definition scope and execute in source
order. Programs may span as many lines as needed. A `;` outside a quoted prompt starts a comment that runs
to the end of the line and is treated as whitespace; inside a prompt, `;` is
prompt text. Comments are not part of the syntax tree, so the formatter's
canonical output omits them. Rebis has no infix operators. Its structural
tokens include `(`, `)`, `[`, `]`, `~`, `#`, `'`, `,`, `$`, `->`, `<-`, `;`,
and the double quote.

## 2. Prompts and symbols

Only a quoted string fires a model agent:

```rebis
"Inspect the parser and identify the first incorrect state"
```

Spaces and punctuation inside the quotes belong to the prompt:

```rebis
"Fix parser.rs: preserve UTF-8, brackets, and quoted text!"
```

Prompt strings support escaped quotes, backslashes, newlines, tabs, and carriage
returns. When an arrow or mediator supplies data, the runtime appends an
`INPUT:` section to the effective prompt. It never interpolates or rewrites the
quoted source text.

A bare atom is a symbol and is never sent to a model:

```rebis
target
worker
inspect
```

Symbols name macros and lexical parameters. Thus `problem` is a symbol,
whereas `"problem"` is a model prompt.

## 3. Groups and abstraction

Parentheses create a structural and abstraction boundary:

```rebis
("Inspect the code")
```

A group can contain independent expressions:

```rebis
(
  "Inspect the parser"
  "Inspect the tokenizer"
  "Inspect the tests")
```

Children execute one abstraction level above their containing node. Nested
groups remain meaningful even with one expression. Kaos exposes this depth in
its firing trace. A group is not an implicit pipeline; use an arrow when answers
must flow between expressions.

## 4. Arrows

`->` sends actual answers from left to right:

```rebis
(->
  "Reproduce the bug"
  "Trace the failing execution path"
  "Write a root-cause report")
```

The first prompt runs, its accepted answer is labeled as a result and supplied
to the second prompt, and the second result is supplied to the third. The arrow
itself never calls a model.

`<-` expresses the same relationship while keeping the consumer first:

```rebis
(<-
  "Write a root-cause report"
  "Trace the failing execution path")
```

The right side runs first and flows into the left. The governing equivalence is:

```text
(-> A B) = (<- B A)
```

Nested arrows execute normally:

```rebis
(->
  "Inspect the repository"
  (<-
    "Explain the violated assumption"
    (->
      "Read the tests"
      "Compare tests with implementation"))
  "Design the smallest safe fix")
```

## 5. Mediator squares

The square form is `([M] A B ...)`. `M` is executable Rebis code and the
remaining expressions are branches.

```rebis
(["Combine the reports into one verified conclusion"]
  "Inspect the implementation"
  "Run the relevant tests"
  "Search for related regressions")
```

Execution is ordered:

1. Run every branch in source order.
2. Ignore absent answers and the conventional refusal `nothing`.
3. Preserve accepted answers as `RESULT 1`, `RESULT 2`, and so forth.
4. Execute `M` with those results as input.
5. Return the mediator program's result as the square's result.

There is no hidden mediator prompt. The mediator can itself be a program:

```rebis
([(->
    "Compare every branch result"
    (<-
      "Resolve contradictions"
      "Check each claim against the supplied evidence")
    "Write the final decision")]
  "Inspect the code"
  "Trace the failure"
  "Run the tests")
```

Squares can be nested:

```rebis
(["Produce the final incident report"]
  (["Summarize the technical investigation"]
    "Inspect the parser"
    "Trace the tokenizer regression")
  (["Summarize customer impact"]
    "Inspect support tickets"
    "Check affected releases"))
```

### Deterministic mediation

Quoted prompts describe work; symbols describe structure. A mediator of
**pure symbols** therefore asks for no work — it names what the answers must
be about, and the calculus judges deterministically. No model fires for the
mediation:

```rebis
([verified-root-cause]
  "Trace the failure backward from the symptom"
  "Inspect recent changes for the earliest wrong state")
```

Each accepted answer is reflected to its content tokens and transported back
onto the mediator's tokens through the answers' own record; the best
round-trip wins and becomes the square's result (ties resolve to source
order, and an answer that cannot round-trip at all is refused — if every
answer is refused the square yields nothing). Write the mediator as a single
hyphenated symbol: the tokenizer splits `verified-root-cause` into its three
tokens, while a multi-word head like `(check value)` would parse as a call.
The full contract, including the `MediatorResolved` event, is in the
specification.

## 6. Named macro abstractions

Define a macro abstraction with `(~ name (parameter ...) body)`:

```rebis
(~ inspect (target)
  (->
    target
    "Analyze the evidence"
    "Write a detailed report"))
```

Call it like Lisp:

```rebis
(
  (~ inspect (target)
    (-> target "Analyze the evidence" "Write a report"))
  (inspect "Inspect the parser"))
```

Definitions in a group are registered before that group's executable forms run.
Macro application substitutes syntax trees, not strings. Given:

```rebis
(~ inspect (target) (-> target "Write a report"))
```

this call:

```rebis
(inspect
  (["Choose the strongest finding"]
    "Inspect the parser"
    "Inspect the tokenizer"))
```

expands structurally to:

```rebis
(->
  (["Choose the strongest finding"]
    "Inspect the parser"
    "Inspect the tokenizer")
  "Write a report")
```

Arguments may be any Rebis expression: prompts, arrows, squares, groups,
symbols, or other calls. Parameters are positional. Prompt text is opaque: a
parameter named `target` does not alter the string `"describe target"`.

### Quote and unquote

Quote (`'`) marks a macro body as an output **template**: the quoted syntax is
returned by expansion instead of executing during it, and `,` inserts syntax
supplied by the caller:

```rebis
(
  (~ twice (work)
    '(-> ,work ,work))

  (twice "Inspect and improve this code."))
```

This expands to:

```rebis
(->
  "Inspect and improve this code."
  "Inspect and improve this code.")
```

Quote prevents the template from executing during expansion. Unquote inserts
complete parsed expressions, never characters or fragments inside a prompt. An
unquote may also produce a call head, enabling higher-order macros:

```rebis
(~ apply (worker value)
  '(,worker ,value))
```

Multiple parameters work normally:

```rebis
(
  (~ compare (left right)
    (["Compare the analyses and explain the difference"] left right))
  (compare
    (-> "Inspect version A" "Summarize version A")
    (-> "Inspect version B" "Summarize version B")))
```

A parameter may occur more than once:

```rebis
(
  (~ examine-twice (task)
    (["Compare both attempts"]
      (-> task "Use a static-analysis perspective")
      (-> task "Use a testing perspective")))
  (examine-twice "Inspect the parser"))
```

This executes the substituted argument twice. Compact source can therefore
expand into several model calls.

### Composition and variables

The string is the language's fundamental value, and `($ A B ...)` is the one
operator that transforms it. It **interpolates** its operands into one string
and yields that string — pure text construction, nothing inside `$` fires or
runs. An operand contributes its text: a prompt its characters, a symbol its
bound value, a **macro its expanded text** (it is not fired), a nested `$` its
assembled text. The assembled string is a prompt in the position the `$` sits,
so it fires there, once:

```rebis
(~ case (self rival)
  ($ "Make the case that " self " beats " rival "."))
```

Because it is an operator, `$` never looks inside a quoted string: a `$` written
in a prompt (`"it cost $100"`) is ordinary text. Interpolating text fires no
model, so weaving a value into a prompt costs nothing extra.

**Variables are macro parameters** — there is no separate binding keyword. The
`case` macro above binds `self` and `rival`; a call supplies the values, reused
freely and woven as text:

```rebis
(["Deliver the verdict on the greater hip-hop legacy."]
  (case "Jay-Z" "Kanye West")
  (case "Kanye West" "Jay-Z"))
```

Because the woven-in values are text, reusing them does not multiply model
calls — this fires two advocates and one mediator, no more.

A **text constant** is just a macro whose body is a prompt. Since `$`
interpolates a macro's text without firing it, no quoting is needed:

```rebis
(~ topic () "the fall of Rome")
($ "Write a short explainer on " (topic) ".")
; one model call — (topic) is woven in as text, not fired on its own.
```

To carry a model-*computed* value into a prompt, use `->` (it flows an agent's
answer in as `INPUT:`); `$` builds text, `->` carries results.

## 7. Higher-order macros

A macro can receive a named macro symbol and invoke it through a parameter
in call-head position:

```rebis
(
  (~ apply (worker target)
    (worker target))

  (~ inspect (target)
    (-> target "Write a report"))

  (apply inspect "Inspect the parser"))
```

The call binds `worker` to the symbol `inspect` and `target` to the quoted
prompt. `(worker target)` becomes `(inspect "Inspect the parser")`, which then
expands to the `inspect` body.

A reusable multi-agent combinator:

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

This separates orchestration topology (`apply-to-both`) from worker strategy
(`investigate`). Rebis supports named macro symbols as arguments. It does
not yet define anonymous abstractions, closures, currying, partial application,
mutable variables, or pattern matching.

### Foundational modules and hypersigils

`#` imports a definition-only Rebis module as an S-expression:

```rebis
(
  (# engineering)
  (repair "Fix the cancellation lifecycle"))
```

The host resolves the symbolic name — with one exception: the `std/`
namespace is reserved for the **embedded standard library** (fourteen
documented modules of orchestration macros compiled into the crate; see
`docs/STD.md`). `(# std/spread)` resolves identically under every host and
never consults host storage. `(# std)` imports the complete embedded library in
stable module-name order. Everything else is host policy: Kaos maps
`engineering` to the saved hypersigil `~/.kaos/sigils/engineering.rebis`,
and qualified paths outside `std/` remain ordinary host names. Kaos also
resolves a saved-sigil folder name recursively, so `(# engineering)` can import
every `.rebis` leaf below `~/.kaos/sigils/engineering/`; an exact
`engineering.rebis` module wins when both exist.

A module is either one `~` definition or a group containing only definitions
and nested imports:

```rebis
(
  (# std/base)
  (~ twice (work) '(-> ,work ,work))
  (~ verify (work) '(-> ,work "Verify the result")))
```

Imports never execute prompts. Their top-level macros are added to the
containing lexical scope, nested imports are re-exported, and module source is
cached for the run. Cycles, missing modules, invalid executable module bodies,
storage failures, and the 64-module bound produce typed diagnostics. This
keeps the core deterministic and capability-free: filesystem, embedded
standard-library, and package resolvers remain host policy.

### Loops from macros

There is no recursion operator. A macro may call itself, and a lazy conditional
square decides whether that call executes:

```rebis
(
  (~ improve (current)
    (-> current "Improve this implementation once."))

  (~ finished (current)
    (-> current
        "Is the result at least 25% faster with no regressions?\nAnswer exactly yes or no."))

  (~ loop (current step stop)
    ([(stop current)]
      current
      (loop (step current) step stop)))

  (loop "The original implementation" improve finished))
```

`([(stop current)] yes-branch no-branch)` first expands and executes the
bracketed call. Its final answer must be exactly `yes` or `no`, ignoring case
and surrounding whitespace. Only the corresponding branch executes. This is
what prevents the recursive call from expanding before it is needed.

The conditional interpretation is deliberately narrow: it applies only to a
two-branch square whose bracketed mediator is a macro call. A prompt mediator,
or a square with any other branch count, retains normal branch-first mediation.
The reference runtime caps each run at 256 macro expansions. Exhausting that
budget produces an `ExpansionLimit` diagnostic instead of silently returning.

## 8. Refusal and result transport

An oracle may return no answer. The exact textual response `nothing`, compared
without case, is also treated as a refusal when arrows and mediators collect
results. Refusals stay visible in the firing trace but are not forwarded as
useful answers.

A provider failure is not a refusal. Production hosts override
`Oracle::try_fire`; authentication, transport, and timeout errors become typed
`OracleFailure` diagnostics.

Accepted answers are labeled in execution order:

```text
RESULT 1:
first answer

RESULT 2:
second answer
```

The orchestration result also exposes `output: Option<String>`. This follows
the program's value path: `->` returns its right consumer, `<-` returns its left
consumer, a normal square returns its mediator, a lazy conditional returns only
the selected branch, a macro call returns its expansion, and a group returns
its final executable form. Hosts should display this separately from the full
firing trace.

The receiver sees those blocks beneath `INPUT:`. Labels preserve ordering but
are not variables accessible from Rebis source.

## 9. A complete bug-fixing program

```rebis
(
  (~ investigate (task)
    (->
      task
      "Identify the earliest incorrect state"
      "Explain the root cause with evidence"))

  (~ validate (proposal)
    ([(->
        "Compare all validation results"
        "List unresolved risks"
        "Return implementation instructions")]
      (-> proposal "Check whether it fixes the root cause")
      (-> proposal "Search for regressions")
      (-> proposal "Design focused regression tests")))

  (~ solve-with (worker problem)
    (->
      (validate
        (->
          (worker problem)
          "Design the smallest safe patch"))
      "Implement the approved patch"))

  (solve-with
    investigate
    "The parser rejects mediator programs containing nested arrows"))
```

This combines quoted prompts, reusable macros, a macro passed as an
argument, nested arrows, executable mediator code, and calls used as pipeline
stages.

## 10. Record compatibility

The crate retains an optional deterministic `Record` calculus used by legacy
Kaos reflection and scoring paths. A record contains evidence lines and their
content-token associations. Expressions can be evaluated against it without
network calls.

This layer is separate from orchestration:

- orchestration chooses prompts and transports model answers;
- record evaluation derives deterministic terms, evidence, and scores;
- model prose never assigns its own deterministic score.

The `score`, `terms`, and `evidence` lines printed by some CLI paths are
compatibility output. New agent programs should primarily inspect their firing
trace and final routed answer.

## 11. Running programs with Kaos

Kaos adapts its selected Claude, OpenAI, OpenRouter, or Ollama model to the Rebis
oracle.

```bash
kaos rebis run --allow-tools program.rebis
kaos rebis run --dry program.rebis
kaos rebis tree '(["Combine"] "Inspect A" "Inspect B")'
kaos rebis mandala '(["Combine"] "Inspect A" "Inspect B")'
```

`run --dry` traverses and expands the real program while every prompt returns
`nothing`. It is useful for inspecting execution order without network access.

Rebis is the default Kaos screen. `/chat` switches to chat mode; `/chat!`
explicitly discards an unsaved Rebis buffer. A specific file can also be opened
with `kaos rebis edit <file>`. The workspace provides:

- Vim-like normal, insert, and command modes;
- automatic pairs for `()`, `<>`, and double quotes;
- highlighting for prompts, symbols, arrows, squares, and `~`;
- `%` matching for parentheses and square brackets;
- `/format` for canonical formatting;
- `/run` for execution through the selected model;
- `/tree` for the expression hierarchy;
- `/mandala` for the `o-[]-o-[]-o` circuit;
- `:w file.rebis` for saving.
- `:q` to return to Kaos (`:q!` discards unsaved edits).

Pressing `/` enters Kaos command mode; pressing `:` enters Vim command mode.
They are deliberately separate. The slash palette filters live while typing;
Up/Down scroll its results, Tab completes, and Enter executes. Use `/graph` to focus the scrollable mandala,
then `hjkl`, arrows, Page Up/Down, `Home`, and `g` to move through it. `Esc`
returns to source focus. `/panel hide`, `/panel show`, and `/panel` control panel
visibility.
The mandala uses real rows for groups, branches, macro templates, calls, mediators,
and arrow stages, so both vertical and horizontal scrolling expose structure.

The editor implements normal, insert, character-visual (`v`), and line-visual
(`V`) modes. Visual selections accept standard motions and `y`, `d`, `x`, or
`c`; normal-mode `p` pastes the visual yank.

Normal mode includes `i/a/I/A`, `o/O`, `hjkl`, arrows, `w/b`, `0/$`, `gg/G`,
`%`, `x`, `dd`, `yy`, `D`, `C`, `s`, `p/P`, `u`, and `Ctrl-R`. It intentionally
does not claim complete Vim compatibility: plugins, registers, marks, macros,
counts, search, and the full Ex language remain outside this embedded core.

Terminal bracketed paste is atomic and preserves every line, normalizing CRLF
and CR endings to LF. After a long paste the cursor remains at its end, so the
top of the buffer may be outside the viewport; press `gg` to return there.

The mandala preserves one visual alphabet as macros are added:

```text
o "prompt"       prompt or result terminal
[M: code]        mediator square
~[f(x)]          macro template
[f]              expanded-call square
→ / ←            answer flow
```

A call such as `(inspect "parser")` is rendered as
`(o "parser") ─[inspect]─o`; the definition is rendered as a named `~[inspect]`
template containing its body circuit.

## 12. Formatting and diagnostics

The formatter preserves prompt escapes, macro definitions, call order,
arrows, mediator structure, and significant parentheses. Parser diagnostics
include UTF-8 byte offsets so hosts can highlight the source location.
The parser rejects sources larger than 1 MiB or deeper than 256 structural or
quote levels before recursive descent.

Runtime diagnostics are also typed: `UndefinedMacro`, `ArityMismatch`,
`ExpansionLimit`, `InvalidCondition`, and `OracleFailure`. They are retained in
`Orchestration::diagnostics` and emitted live as `ExecutionEvent::Diagnostic`.
Hosts can use `orchestrate_with_observer` to stream prompt, flow, macro,
mediator, branch-selection, and diagnostic events while execution is active.

A common mistake is writing unquoted prose:

```rebis
(Inspect the parser)
```

This is parsed as a call to symbol `Inspect`; it is not a prompt. Write:

```rebis
"Inspect the parser"
```

A square needs exactly one mediator expression and at least one branch:

```rebis
(["Combine"] "branch")
```

An arrow needs a producer and a consumer; `(-> "only one")` is invalid.

A macro needs a name, parameter list, and one body:

```rebis
(~ inspect (target) (-> target "Write a report"))
```

Calls currently require exact positional arity. Unknown callees and arity
mismatches do not produce model calls and emit typed runtime diagnostics.

## 13. Operational limits

Rebis deliberately leaves provider policy to its host. A production host should
limit:

- expanded expression size and macro-expansion depth;
- imported module count and resolver storage access;
- total and concurrent model firings;
- prompt and answer tokens;
- elapsed time, retries, and spending;
- tool access granted to underlying models;
- persistence and disclosure of firing traces.

Higher-order macros and repeated parameters make these limits important:
small source programs can expand into many calls.
`RuntimeLimits` configures macro, module, model-call, and concurrency budgets.
Convenience entry points default to 256 expansions, 64 distinct module loads,
and 1,024 model calls per orchestration.

Square branches are semantically unordered, so under `orchestrate_parallel`
they evaluate concurrently — up to `with_max_concurrency` at once (default 4),
each against a snapshot of the record with branch-scoped definitions, with
answers and events merged back in source order. A bound of 1 reproduces
sequential evaluation exactly; arrows, group children, and the lazy
conditional are always sequential. See the specification for the full
contract.

## 14. Language equations

```text
"prompt"               one model prompt
(-> A B)               run A and route its answers into B
(<- B A)               the same flow, consumer written first
([M] A B ...)          run branches, then M over ordered answers
(~ f (x ...) 'body)    define a macro with quoted output syntax
,x                     splice syntax x into a quoted macro body
(f argument ...)       expand the macro and execute its result
(apply f x)            higher-order use when f reaches call-head position
```

Quoted prompts describe work. Symbols describe reusable structure. Arrows move
answers. Squares converge branches. Macros turn orchestration patterns into
composable programs.
