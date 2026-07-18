# Hosting Rebis

Rebis is dependency-free and performs no network I/O. A host owns the record,
model provider, budgets, policy, and persistence. The adapter boundary is the
small `Oracle` trait:

```rust
use rebis_lang::Oracle;

struct HostOracle;

impl Oracle for HostOracle {
    fn fire(&self, prompt: &str) -> Option<String> {
        self.try_fire(prompt).ok().flatten()
    }

    fn try_fire(&self, prompt: &str) -> Result<Option<String>, String> {
        // Return Ok(None) only for an intentional decline. Preserve provider,
        // authentication, and timeout failures as Err messages.
        todo!("provider call for {prompt}")
    }
}
```

Parse before any provider call. Build the initial `Record` only from sources
the user has authorized. Then call `orchestrate(expr, record, oracle)` and keep
the returned output, `Firing` trace, typed events, and diagnostics. Interactive
hosts should use `orchestrate_with_observer` to render events as they occur.

## Modules

`(# name)` is resolved through the separate `ModuleResolver` capability. The
resolver receives a validated `ModuleName`; it returns Rebis source but never
executes it. Use `orchestrate_with_runtime` when a host supports modules:

```rust
use rebis_lang::{ModuleName, ModuleResolver};

struct Modules;

impl ModuleResolver for Modules {
    fn resolve(&self, name: &ModuleName) -> Result<Option<String>, String> {
        // Route `std/...` to an embedded future standard library and other
        // names to an approved user hypersigil store.
        todo!("resolve {name}")
    }
}
```

The core accepts only definition/import module bodies, caches compiled exports,
detects cycles, and limits a run to 64 distinct loads. A resolver must still
confine names to its intended storage root and treat module source as untrusted.

The host should impose time, token, response-size, and concurrency limits;
redact secrets from evidence; make retries explicit; and treat model answers as
untrusted record additions. Rebis never executes tool calls found in an answer.
Use `orchestrate_with_limits` to set the core macro-expansion, module-import,
and model-call budgets. The model-call limit is checked before invoking the
oracle, making it a hard provider-cost boundary.

Atom prompts are the atoms verbatim; hosts must not prepend, rewrite, or expand
them. Parenthesis depth is reported on each firing. Mediator prompts are
issued only for squares that lack common ground. Arrows are local deterministic
judges and never cross the provider boundary. Consequently the syntax tree
gives a conservative call bound before execution.

For deterministic tests, use a scripted oracle that returns fixed answers and
assert both the final score and firing count. The crate's incident-triage test
demonstrates this pattern and pins its documented behavior.
