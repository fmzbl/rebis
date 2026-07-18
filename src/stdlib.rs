//! The embedded standard library: definition-only Rebis modules compiled
//! into the crate and resolved under the reserved `std/` namespace.
//!
//! Every orchestration entry point consults std **before** the host's
//! resolver, and `std/*` never falls through: an unknown std name is
//! `ModuleNotFound`, and a host file mapped to a std name is never read.
//! Everything outside `std/` remains host policy, unchanged.
//!
//! The sources are ordinary Rebis (see `docs/STD.md` for the design):
//! structural modules carry no prompt text; exactly two phrasal modules
//! (`std/canon`, `std/shape`) carry canonical protocol phrasings.

use crate::agents::ModuleResolver;
use crate::syntax::ModuleName;

/// Name → source for every embedded module, in stable alphabetical order.
const STD_MODULES: &[(&str, &str)] = &[
    ("std/canon", include_str!("std/canon.rebis")),
    ("std/committee", include_str!("std/committee.rebis")),
    ("std/debate", include_str!("std/debate.rebis")),
    ("std/dialectic", include_str!("std/dialectic.rebis")),
    ("std/evolve", include_str!("std/evolve.rebis")),
    ("std/flow", include_str!("std/flow.rebis")),
    ("std/gate", include_str!("std/gate.rebis")),
    ("std/loops", include_str!("std/loops.rebis")),
    ("std/map", include_str!("std/map.rebis")),
    ("std/reflexion", include_str!("std/reflexion.rebis")),
    ("std/search", include_str!("std/search.rebis")),
    ("std/shape", include_str!("std/shape.rebis")),
    ("std/spread", include_str!("std/spread.rebis")),
    ("std/tournament", include_str!("std/tournament.rebis")),
];

/// The embedded modules as `(name, source)` pairs, for host tooling
/// (completion, search, documentation).
#[must_use]
pub fn std_modules() -> &'static [(&'static str, &'static str)] {
    STD_MODULES
}

fn embedded(name: &str) -> Option<String> {
    if let Some((_, source)) = STD_MODULES.iter().find(|(module, _)| *module == name) {
        return Some((*source).to_string());
    }

    // A namespace is itself importable. Resolve it to ordinary child imports
    // so each module still goes through cycle detection, caching, events, and
    // the host-configured module budget.
    let prefix = format!("{name}/");
    let children = STD_MODULES
        .iter()
        .map(|(module, _)| *module)
        .filter(|module| module.starts_with(&prefix))
        .collect::<Vec<_>>();
    (!children.is_empty()).then(|| {
        format!(
            "({})",
            children
                .iter()
                .map(|module| format!("(# {module})"))
                .collect::<Vec<_>>()
                .join(" ")
        )
    })
}

fn is_std(name: &str) -> bool {
    name == "std" || name.starts_with("std/")
}

/// Wraps a host resolver with the embedded standard library. `std/*`
/// resolves from the crate and never falls through to the host.
pub(crate) struct WithStd<'a>(pub &'a dyn ModuleResolver);

impl ModuleResolver for WithStd<'_> {
    fn resolve(&self, module: &ModuleName) -> Result<Option<String>, String> {
        let name = module.as_str();
        match embedded(name) {
            Some(source) => Ok(Some(source)),
            None if is_std(name) => Ok(None),
            None => self.0.resolve(module),
        }
    }
}

/// The `Sync` twin of [`WithStd`] for the concurrent evaluation path.
pub(crate) struct WithStdSync<'a>(pub &'a (dyn ModuleResolver + Sync));

impl ModuleResolver for WithStdSync<'_> {
    fn resolve(&self, module: &ModuleName) -> Result<Option<String>, String> {
        let name = module.as_str();
        match embedded(name) {
            Some(source) => Ok(Some(source)),
            None if is_std(name) => Ok(None),
            None => self.0.resolve(module),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn std_namespace_resolves_to_all_child_imports() {
        let source = embedded("std").expect("the std folder is importable");
        assert_eq!(source.matches("(# std/").count(), STD_MODULES.len());
    }
}
