# Rebis `o-[]-o`

```rebis
(~ solve (x) '(-> ,x "Verify the answer"))

(["Ship one result"] (solve "Find the bug") (solve "Design the fix"))
```

In two forms this defines a reusable agent graph, fires two instances, routes
each real answer through verification, and mediates both verified results into
the program's returned value.

Rebis is a pure S-expression language for programming LLM-agent systems.
Quoted strings are raw prompts, bare atoms are Lisp-like symbols, `~` defines
structural macro abstractions, arrows route actual answers, and squares contain
executable mediator code.

```text
program := expr+ EOF
expr := prompt | symbol | '\'' expr | ',' expr | '(' form ')'
form := '~' symbol '(' symbol* ')' expr
      | '#' module | '<' expr '>' expr+
      | '$' expr+
      | '->' expr expr+ | '<-' expr expr+
      | symbol expr* | expr+
```

As in Lisp source files, a program may contain multiple top-level forms without
an extra pair of parentheses. They share one lexical definition scope and
execute in source order; the parser retains that boundary as an implicit
program node.

The central form is `([M] A B ...)`. Branches run first in source order. Their
actual answers become `RESULT 1`, `RESULT 2`, and so on, then enter mediator
program `M`.

```rebis
([(->
    "Compare all reports"
    "Resolve disagreements"
    "Write one verified fix")]
  "Inspect the code and reproduce the failure"
  "Trace the execution and find the root cause")
```

Macro abstractions use `(~ name (parameter ...) body)` and ordinary Lisp-style
calls:

```rebis
(
  (~ inspect (target)
    (-> target "Write a detailed report"))
  (inspect "Inspect the parser"))
```

Application substitutes argument expressions structurally. It never changes
text inside quoted prompts. Only quoted prompts fire agents.

## Composition and variables

The string is the language's fundamental value. `($ A B ...)` is the one
operator that transforms it: it **interpolates** its operands into one string
and yields that string — pure text construction, nothing inside `$` fires or
runs. An operand contributes its text: a prompt its characters, a symbol its
bound value, a macro its expanded text (not fired), a nested `$` its assembled
text. The assembled string is a prompt in the position the `$` sits, so it fires
there, once. Because it is an operator, not in-string interpolation, it never
peeks inside a quoted string — `"it cost $100"` stays literal.

**Variables are macro parameters.** A macro binds names; a call supplies the
values, which `$` weaves in as text — reused freely, with no extra model call
per use:

```rebis
(~ case (self rival)
  ($ "Make the strongest case that " self " beats " rival " in hip-hop."))

(["Deliver the verdict: who has the greater hip-hop legacy, and why?"]
  (case "Jay-Z" "Kanye West")
  (case "Kanye West" "Jay-Z"))
```

This fires exactly two advocates and one mediator: `self` and `rival` are
text, so reusing them does not multiply model calls.

A **text constant** is just a macro whose body is a prompt — `$` interpolates
its text without firing it, so no quoting is needed:

```rebis
(~ topic () "the fall of Rome")
($ "Write a short explainer on " (topic) ".")
; one model call — (topic) is woven in as text, not fired on its own.
```

To carry a model-*computed* value into a prompt, use `->` (it flows the answer
in as `INPUT:`); `$` builds text, `->` carries results.

Macros are higher-order: a named macro symbol can be passed as an
argument and substituted in call-head position.

```rebis
(
  (~ apply (worker target) (worker target))
  (~ inspect (target) (-> target "Write a report"))
  (apply inspect "Inspect the parser"))
```

Repeated parameters may duplicate model work, so hosts should enforce expanded
call, size, token, and time budgets.

For explicit Scheme/Common-Lisp-style construction, quote holds output syntax
and comma splices caller syntax:

```rebis
(~ twice (work) '(-> ,work ,work))
```

Macros may call themselves. A two-branch square whose bracketed expression is
a macro call evaluates that call as an exact `yes`/`no` condition and runs only
the selected branch. This provides loops without a separate recursion operator;
the reference runtime caps expansion at 256 macro calls.

## Modules, abstraction, and expansion

`(# module)` imports the top-level macro definitions of a host-resolved Rebis
module. Kaos resolves modules from saved hypersigils in `~/.kaos/sigils`; names
may be qualified, such as `std/loops`. The crate embeds fourteen modules under
that reserved namespace: import one leaf with `(# std/loops)`, or the complete
folder with `(# std)`. Hosts may apply the same definition-only expansion to
their own folders; Kaos recursively imports a saved folder such as `(# team)`.
Module bodies are definition-only and may re-export other modules with `#`.

For example, save this as the hypersigil `engineering.rebis`:

```rebis
(
  (~ investigate (issue)
    '(->
      ,issue
      "Reproduce the failure"
      (["Choose the strongest causal explanation"]
        (<- "Challenge the trace" "Trace state backward from the symptom")
        (-> "Inspect the relevant code" "Propose the earliest wrong state"))))

  (~ repair (issue)
    '(->
      (investigate ,issue)
      "Implement the smallest root-cause fix"
      (<- "Return the reviewed patch" "Run tests and search for regressions"))))
```

Then a program can import and expand it twice:

```rebis
(
  (# engineering)

  ([(->
      "Compare both repairs"
      "Resolve contradictory evidence"
      "Write the final design and patch plan")]
    (repair "Tower middleware lacks ad-hoc span metadata")
    (repair "Future cancellation can lose the closing event")))
```

The compact call:

```rebis
(repair "Future cancellation can lose the closing event")
```

structurally expands into the full nested `->`, `<-`, mediator, and
`investigate` graph from the module. Arguments remain syntax throughout the
expansion; no prompt text is interpolated or reparsed.

```bash
kaos rebis run --allow-tools examples/incident.rebis
kaos rebis run --dry '(["Combine reports"] "Inspect code" "Trace failure")'
kaos rebis tree '(["synthesize"] "Inspect code" "Trace failure")'
```

Kaos provides a direct editor plus an optional Vim-like mode with
normal/insert/visual modes and `%` bracket matching. `ctrl /` opens Kaos commands
(`/run`, `/tree`, `/mandala`, `/format`,
`/panel`, `/graph`); `:` remains reserved for Vim file commands. See [the specification](docs/SPEC.md),
[guide](docs/GUIDE.md), [symbol reference](docs/REFERENCE.md),
[standard library](docs/STD.md), and [host notes](docs/HOSTS.md).
