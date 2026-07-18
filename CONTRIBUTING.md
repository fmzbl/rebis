# Contributing to Rebis

Rebis is intentionally small. Contributions should strengthen the model
interface without smuggling application policy or provider behavior into the
language.

Before submitting a change, run:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked
RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --all-features
./paper/build.sh
```

Language changes must include all of the following:

1. A normative update to `docs/SPEC.md`.
2. Canonical parse/format fixtures, including nested boundary cases.
3. Malformed-input and resource-limit tests.
4. Graph schema compatibility notes and editor impact.
5. Runtime tests for call order, provider failure, and each affected limit.
6. Security analysis for any new data or capability crossing the provider
   boundary.
7. A `CHANGELOG.md` entry.

The following changes need an explicit language-version proposal rather than a
small patch:

- adding loops, recursion, mutation, variables, imports, or implicit calls;
- changing branch ordering or the input seen by a `[]` branch;
- treating `<-` as an inverse operation;
- flattening singleton scopes from the AST, graph, plan, or trace;
- allowing provider responses to execute tools directly;
- embedding a vendor SDK, secret loader, application schema, or hidden prompt
  into the core crate.

Provider adapters belong outside the language core. They may translate the
Rebis provider contract to local inference or a remote service, but must keep
model selection, credentials, timeouts, retry policy, logging, and response
validation explicit. Tests should use deterministic doubles and must not
contact a live service by default.

Documentation examples must label illustrative provider transcripts as such.
Do not publish invented latency, quality, cost, or benchmark results. The
protocol in `docs/EXPERIMENTS.md` describes how to record real measurements.
