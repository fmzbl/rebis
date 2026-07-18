//! The calculus: evaluating expressions against a record, with or without a
//! model in the square.

use std::collections::BTreeSet;

use crate::record::{content_tokens, Concept, Record};
use crate::syntax::{parse, Error, Expr};

/// Scores are approximate ratios and record collections are memory-bounded in
/// every host; converting their lengths to the public `f32` score is intentional.
#[allow(clippy::cast_precision_loss)]
fn ratio(part: usize, whole: usize) -> f32 {
    part as f32 / whole as f32
}

/// Refine `target` by `by`: keep the part of `target`'s evidence that
/// broadened `by` reaches. The score is the surviving fraction of the target;
/// with no resolvable evidence it degrades to direct term coverage so thin
/// records fail soft.
fn refine(target: &Concept, by: &Concept, record: &Record) -> Concept {
    let tb = record.broaden(&by.terms);
    let eb = record.evidence(&tb);
    let kept: BTreeSet<usize> = target.evidence.intersection(&eb).copied().collect();
    let score = if target.evidence.is_empty() {
        let covered = target.terms.iter().filter(|t| tb.contains(*t)).count();
        if target.terms.is_empty() {
            0.0
        } else {
            ratio(covered, target.terms.len())
        }
    } else {
        ratio(kept.len(), target.evidence.len())
    };
    Concept {
        terms: target.terms.clone(),
        evidence: kept,
        score,
    }
}

/// The square: broaden both occupants and return their common ground —
/// shared evidence, scored by overlap, terms replaced by the bridge.
fn collide(ca: &Concept, cb: &Concept, record: &Record) -> Concept {
    let ta = record.broaden(&ca.terms);
    let tb = record.broaden(&cb.terms);
    let ea = record.evidence(&ta);
    let eb = record.evidence(&tb);
    let shared: BTreeSet<usize> = ea.intersection(&eb).copied().collect();
    let union = ea.union(&eb).count();
    if union == 0 {
        // the record resolves neither occupant: degrade to composition so the
        // atoms survive the collision (thin records fail soft)
        let mut terms = ca.terms.clone();
        terms.extend(cb.terms.iter().cloned());
        return Concept {
            terms,
            evidence: BTreeSet::new(),
            score: 0.0,
        };
    }
    let score = ratio(shared.len(), union);
    let mut bridge = BTreeSet::new();
    for id in &shared {
        if let Some(line) = record.line(*id) {
            for t in ta.union(&tb) {
                if line.contains(t) {
                    bridge.insert(t.clone());
                }
            }
        }
    }
    Concept {
        terms: bridge,
        evidence: shared,
        score,
    }
}

/// Evaluate an expression against the record — the pure calculus.
#[must_use]
pub fn eval(expr: &Expr, record: &Record) -> Concept {
    match expr {
        Expr::Prompt(word) | Expr::Symbol(word) => {
            let mut terms = BTreeSet::new();
            terms.insert(word.clone());
            let evidence = record.evidence(&terms);
            Concept {
                terms,
                evidence,
                score: 1.0,
            }
        }
        // Syntax values are inert in the record calculus. Unquote only has
        // meaning while expanding a quoted macro body.
        Expr::Quote(inner) | Expr::Unquote(inner) => eval(inner, record),
        // A composition's meaning is the union of its operands' — the same
        // shape as a group in the record calculus, since scoring cares about
        // the terms present, not how the text is assembled at runtime.
        Expr::Program(items) | Expr::Compose(items) | Expr::Concat(items) => {
            let mut terms = BTreeSet::new();
            let mut evidence = BTreeSet::new();
            for item in items {
                let c = eval(item, record);
                terms.extend(c.terms);
                evidence.extend(c.evidence);
            }
            Concept {
                terms,
                evidence,
                score: 1.0,
            }
        }
        Expr::Square { mediator, branches } => {
            let mut branch = Concept {
                terms: BTreeSet::new(),
                evidence: BTreeSet::new(),
                score: 1.0,
            };
            for item in branches {
                let concept = eval(item, record);
                branch.terms.extend(concept.terms);
                branch.evidence.extend(concept.evidence);
            }
            collide(&eval(mediator, record), &branch, record)
        }
        // one arrow, two readings: `(-> a b)` refines b by a, `(<- a b)`
        // refines a by b — the law (-> a b) ≡ (<- b a) holds by construction
        Expr::Forward(a, b) => refine(&eval(b, record), &eval(a, record), record),
        Expr::Backflow(a, b) => refine(&eval(a, record), &eval(b, record), record),
        Expr::Function { body, .. } => eval(body, record),
        Expr::Call { args, .. } => eval(&Expr::Compose(args.clone()), record),
        Expr::Import { .. } => Concept {
            terms: BTreeSet::new(),
            evidence: BTreeSet::new(),
            score: 1.0,
        },
    }
}

/// Parse and evaluate a source string against a record.
///
/// # Errors
///
/// Returns a syntax error when `src` is not a valid Rebis program.
pub fn run(src: &str, record: &Record) -> Result<Concept, Error> {
    Ok(eval(&parse(src)?, record))
}

// ── the model seam ──

/// The model seam used by orchestration. One method: fire a prompt, get
/// text. Hosts adapt their model client to this; tests script it. The
/// calculus in this module never fires it — every prompt a model receives
/// is program source (plus the documented `INPUT:`/`RESULT n:` transport
/// labels); the runtime authors no prompt text of its own.
pub trait Oracle {
    /// Answer one prompt, or decline.
    fn fire(&self, prompt: &str) -> Option<String>;

    /// Answer one prompt while preserving host/provider failures.
    ///
    /// Existing infallible adapters only need to implement [`Oracle::fire`].
    /// Production hosts should override this method so execution can distinguish
    /// an intentional decline (`Ok(None)`) from a failed model call (`Err`).
    ///
    /// # Errors
    ///
    /// Returns a host-defined message when the provider cannot complete the call.
    fn try_fire(&self, prompt: &str) -> Result<Option<String>, String> {
        Ok(self.fire(prompt))
    }
}

// ── the gate application: round-trip fidelity, expressed in the language ──

/// Extract the first well-formed parenthesized Rebis expression embedded in
/// text (a model reply may wrap it in prose). None when nothing parses.
#[must_use]
pub fn parse_embedded(text: &str) -> Option<Expr> {
    for (start, opening) in text.char_indices() {
        if opening != '(' {
            continue;
        }
        let mut depth = 0usize;
        let mut in_prompt = false;
        let mut escaped = false;
        let mut in_comment = false;
        for (relative, character) in text[start..].char_indices() {
            if in_comment {
                if character == '\n' {
                    in_comment = false;
                }
                continue;
            }
            if in_prompt {
                if escaped {
                    escaped = false;
                } else {
                    match character {
                        '\\' => escaped = true,
                        '"' => in_prompt = false,
                        _ => {}
                    }
                }
                continue;
            }
            match character {
                '"' => in_prompt = true,
                // Parentheses inside a `;` comment are not structure.
                ';' => in_comment = true,
                '(' => depth += 1,
                ')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        let end = start + relative + character.len_utf8();
                        if let Ok(expression) = parse(&text[start..end]) {
                            return Some(expression);
                        }
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    None
}

/// Round-trip holonomy of an echo against a task, evaluated on the record —
/// written in the language itself:
///
/// ```text
/// holonomy = 1 − score( (<- task echo) )
/// ```
///
/// how much of the task's evidence the echo fails to explain when flowed
/// backwards. An echo containing no well-formed expression scores 1.0:
/// outside the language there is no round trip.
#[must_use]
pub fn holonomy(task: &str, echo: &str, record: &Record) -> f32 {
    let want = content_tokens(task);
    if want.is_empty() {
        return 0.0;
    }
    let Some(echo_expr) = parse_embedded(echo) else {
        return 1.0;
    };
    let task_expr = Expr::Compose(want.into_iter().map(Expr::Symbol).collect());
    let refined = eval(
        &Expr::Backflow(Box::new(task_expr), Box::new(echo_expr)),
        record,
    );
    1.0 - refined.score
}

/// Deterministic reflection — edge two of the holonomy triangle, with no
/// model and no prompt. The calculus's own tokenizer compresses a candidate
/// answer to the expression of its content tokens.
#[must_use]
pub fn reflect(candidate: &str) -> Expr {
    Expr::Compose(
        content_tokens(candidate)
            .into_iter()
            .map(Expr::Symbol)
            .collect(),
    )
}

/// Promptless holonomy: transport a candidate around the full triangle —
/// task → candidate → [`reflect`] → back onto the task through the record —
/// entirely in the calculus. `0.0` means the loop closes (a faithful answer);
/// `1.0` means it cannot round-trip at all. A candidate with no content
/// tokens scores `1.0`.
#[must_use]
pub fn holonomy_reflected(task: &str, candidate: &str, record: &Record) -> f32 {
    let want = content_tokens(task);
    if want.is_empty() {
        return 0.0;
    }
    let echo = reflect(candidate);
    if matches!(&echo, Expr::Compose(items) if items.is_empty()) {
        return 1.0;
    }
    let task_expr = Expr::Compose(want.into_iter().map(Expr::Symbol).collect());
    let refined = eval(&Expr::Backflow(Box::new(task_expr), Box::new(echo)), record);
    1.0 - refined.score
}
