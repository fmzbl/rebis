# Rebis test suites

All tests are deterministic and dependency-free. No test contacts a model or
the network. Coverage is split by contract so a failure points at the layer
that regressed:

- `conformance.rs`: end-to-end public-language semantics.
- `syntax_edge_cases.rs`: lexer, parser, formatter, comments, Unicode,
  resource boundaries, and hostile input.
- `runtime_edge_cases.rs`: orchestration, routing, macros, provider failures,
  event ordering, lexical scope, and budgets.
- `module_edge_cases.rs`: resolver, re-export, cache, shadowing, cycles, and
  the module budget.
- `calculus_edge_cases.rs`: the record calculus, reflection, embedding,
  renderers, and tokenization.
- `parallel_squares.rs`: concurrent square branches — determinism, isolation,
  and shared budgets.
- `std_library.rs`: the embedded standard library — build gate, namespace
  reservation, and every module under a scripted oracle.
- `cli_edge_cases.rs`: black-box command, stdin, file, output, and exit-code
  cases.

The crate also carries focused unit tests. Run the same release gate as CI:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked
RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --all-features
```

New language behavior should add a parser/formatter case, its runtime or
calculus behavior, a malformed or bounded counterpart, and an observable event
or CLI assertion when those surfaces are affected.
