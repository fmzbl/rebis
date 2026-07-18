//! The o-[]-o tree: rendering an expression as the symbol it is.
//!
//! Prompts are circles, mediators and calls are squares, arrows are arrows. With a record,
//! every node is annotated with its evaluated score and evidence count, so
//! the tree shows the program *and* what the record makes of it.

use crate::eval::eval;
use crate::record::Record;
use crate::syntax::Expr;
use std::fmt::Write;

/// Node glyphs: the drawing alphabet of the symbol.
fn glyph(expr: &Expr) -> (&'static str, String) {
    match expr {
        Expr::Program(_) => ("◆", "program".to_string()),
        Expr::Prompt(w) => ("○", format!("{w:?}")),
        Expr::Symbol(w) => ("◇", w.clone()),
        Expr::Quote(_) => ("'", "quoted syntax".to_string()),
        Expr::Unquote(_) => (",", "unquote".to_string()),
        Expr::Compose(_) => ("◌", "group".to_string()),
        Expr::Concat(_) => ("$", "composition".to_string()),
        Expr::Square { .. } => ("□", "mediator square".to_string()),
        Expr::Forward(..) => ("→", "forward".to_string()),
        Expr::Backflow(..) => ("←", "backflow".to_string()),
        Expr::Function { name, .. } => ("λ", format!("function {name}")),
        Expr::Call { name, .. } => ("@", format!("call {name}")),
        Expr::Import { module } => ("⇲", format!("import {module}")),
    }
}

/// Children with left-associative operator chains flattened, so
/// Arrow chains render flat; a square renders its mediator before its branches.
fn children(expr: &Expr) -> Vec<&Expr> {
    fn spine<'e>(expr: &'e Expr, root: &Expr, out: &mut Vec<&'e Expr>) {
        let same = matches!(
            (expr, root),
            (Expr::Forward(..), Expr::Forward(..)) | (Expr::Backflow(..), Expr::Backflow(..))
        );
        match expr {
            Expr::Forward(a, b) | Expr::Backflow(a, b) if same => {
                spine(a, root, out);
                out.push(b);
            }
            _ => out.push(expr),
        }
    }
    match expr {
        Expr::Prompt(_) | Expr::Symbol(_) | Expr::Import { .. } => Vec::new(),
        Expr::Quote(inner) | Expr::Unquote(inner) => vec![inner],
        Expr::Program(items) | Expr::Compose(items) | Expr::Concat(items) => items.iter().collect(),
        Expr::Square { mediator, branches } => {
            let mut out = Vec::with_capacity(branches.len() + 1);
            out.push(mediator.as_ref());
            out.extend(branches.iter());
            out
        }
        Expr::Forward(..) | Expr::Backflow(..) => {
            let mut out = Vec::new();
            spine(expr, expr, &mut out);
            out
        }
        Expr::Function { body, .. } => vec![body],
        Expr::Call { args, .. } => args.iter().collect(),
    }
}

fn write_node(
    expr: &Expr,
    prefix: &str,
    last: bool,
    root: bool,
    record: Option<&Record>,
    out: &mut String,
) {
    let (glyph, name) = glyph(expr);
    let connector = if root {
        String::new()
    } else if last {
        format!("{prefix}└─ ")
    } else {
        format!("{prefix}├─ ")
    };
    out.push_str(&connector);
    out.push_str(glyph);
    out.push(' ');
    out.push_str(&name);
    if let Some(record) = record {
        let c = eval(expr, record);
        let _ = write!(out, " · {:.3} · {} line(s)", c.score, c.evidence.len());
    }
    out.push('\n');
    let kids = children(expr);
    let next = if root {
        String::new()
    } else if last {
        format!("{prefix}   ")
    } else {
        format!("{prefix}│  ")
    };
    for (i, kid) in kids.iter().enumerate() {
        write_node(kid, &next, i + 1 == kids.len(), false, record, out);
    }
}

/// Render an expression as its o-[]-o tree.
#[must_use]
pub fn tree(expr: &Expr) -> String {
    let mut out = String::new();
    write_node(expr, "", true, true, None, &mut out);
    out
}

/// Render the tree with every node scored against a record.
#[must_use]
pub fn tree_scored(expr: &Expr, record: &Record) -> String {
    let mut out = String::new();
    write_node(expr, "", true, true, Some(record), &mut out);
    out
}

/// Render an expression as the horizontal `o-[]-o-[]-o` circuit from the
/// original whiteboard. `o` is a prompt/value terminal, `[M: ...]` is a
/// mediator, `~[f(...)]` is a function template, and `[f]` is a call box.
#[must_use]
pub fn mandala(expr: &Expr) -> String {
    fn inline(expr: &Expr) -> String {
        match expr {
            Expr::Program(_) => "◆ program".to_string(),
            Expr::Prompt(word) => format!("o {word:?}"),
            Expr::Symbol(word) => format!("◇ {word}"),
            Expr::Quote(_) => "' quoted syntax".to_string(),
            Expr::Unquote(_) => ", unquote".to_string(),
            Expr::Function { name, params, .. } => format!("~[{name}({})]", params.join(", ")),
            Expr::Call { name, .. } => format!("[{name}]─o"),
            Expr::Import { module } => format!("⇲[{module}]"),
            Expr::Square { .. } => "[M]─o".to_string(),
            Expr::Forward(..) => "→ flow".to_string(),
            Expr::Backflow(..) => "← flow".to_string(),
            Expr::Compose(_) => "◌ group".to_string(),
            Expr::Concat(_) => "$ composition".to_string(),
        }
    }
    fn indented(lines: Vec<String>, first: &str, rest: &str) -> Vec<String> {
        lines
            .into_iter()
            .enumerate()
            .map(|(i, line)| format!("{}{line}", if i == 0 { first } else { rest }))
            .collect()
    }
    fn block(expr: &Expr) -> Vec<String> {
        match expr {
            Expr::Prompt(_) | Expr::Symbol(_) | Expr::Import { .. } => vec![inline(expr)],
            Expr::Quote(inner) | Expr::Unquote(inner) => {
                let mut out = vec![inline(expr)];
                out.extend(indented(block(inner), "└─ ", "   "));
                out
            }
            Expr::Program(items) | Expr::Compose(items) | Expr::Concat(items) => {
                let mut out = vec![inline(expr)];
                for (i, item) in items.iter().enumerate() {
                    let last = i + 1 == items.len();
                    out.extend(indented(
                        block(item),
                        if last { "└─ " } else { "├─ " },
                        if last { "   " } else { "│  " },
                    ));
                }
                out
            }
            Expr::Forward(left, right) => {
                let mut out = block(left);
                out.extend(["│".to_string(), "▼".to_string()]);
                out.extend(indented(block(right), "→ ", "  "));
                out
            }
            Expr::Backflow(left, right) => {
                let mut out = block(right);
                out.extend(["│".to_string(), "▼ backflow".to_string()]);
                out.extend(indented(block(left), "← ", "  "));
                out
            }
            Expr::Square { mediator, branches } => {
                let mut out = vec!["branches".to_string()];
                for (i, branch) in branches.iter().enumerate() {
                    let last = i + 1 == branches.len();
                    out.extend(indented(
                        block(branch),
                        if last { "└─ " } else { "├─ " },
                        if last { "   " } else { "│  " },
                    ));
                }
                out.extend([
                    "   │".to_string(),
                    "   ▼".to_string(),
                    format!("[M: {}]", inline(mediator)),
                ]);
                out.extend(indented(block(mediator), "└─ ", "   "));
                out.push("   └─o result".to_string());
                out
            }
            Expr::Function { name, params, body } => {
                let mut out = vec![format!("~[{name}({})] template", params.join(", "))];
                out.extend(indented(block(body), "└─ ", "   "));
                out
            }
            Expr::Call { name, args } => {
                let mut out = vec![format!("inputs for [{name}]")];
                for (i, arg) in args.iter().enumerate() {
                    let last = i + 1 == args.len();
                    out.extend(indented(
                        block(arg),
                        if last { "└─ " } else { "├─ " },
                        if last { "   " } else { "│  " },
                    ));
                }
                out.extend(["   │".to_string(), format!("   └─[{name}]─o")]);
                out
            }
        }
    }
    format!("o-[]-o-[]-o\n{}\n", block(expr).join("\n"))
}

#[cfg(any())]
mod tests {
    use super::*;
    use crate::syntax::parse;

    #[test]
    fn the_tree_draws_the_symbol() {
        let expr = parse("([m] (-> a (b c)) (<- d e))").unwrap();
        let drawn = tree(&expr);
        assert!(drawn.starts_with("□ square\n"), "{drawn}");
        assert!(drawn.contains("→ forward"));
        assert!(drawn.contains("← backflow"));
        assert!(drawn.contains("○ a"));
        assert!(drawn.contains("◌ group"));
        assert!(drawn.contains("├─"));
        assert!(drawn.contains("└─"));
    }

    #[test]
    fn chains_flatten_into_one_node() {
        let expr = parse("([m] a b c)").unwrap();
        let drawn = tree(&expr);
        assert_eq!(drawn.matches("□ square").count(), 1, "{drawn}");
        assert_eq!(drawn.matches('○').count(), 3);
    }

    #[test]
    fn scored_trees_annotate_every_node() {
        let record = Record::from_texts(&["a and b together", "c alone"]);
        let expr = parse("([m] a b)").unwrap();
        let drawn = tree_scored(&expr, &record);
        for line in drawn.lines() {
            assert!(line.contains("line(s)"), "unscored line: {line}");
        }
    }

    #[test]
    fn mandala_draws_the_whiteboard_circuit() {
        let expr = parse("([m] a ([m] b c))").unwrap();
        let drawn = mandala(&expr);
        assert!(drawn.starts_with("o-[]-o-[]-o\n"));
        assert_eq!(drawn.matches('□').count(), 2);
        assert!(drawn.contains("○ a"));
        assert!(drawn.contains("○ c"));
    }

    #[test]
    fn singleton_groups_remain_visible_as_abstraction() {
        let expr = parse("(hello hello (bye))").unwrap();
        let drawn = tree(&expr);
        assert_eq!(drawn.matches("◌ group").count(), 2, "{drawn}");
        let circuit = mandala(&expr);
        assert_eq!(circuit.matches("↑(").count(), 2, "{circuit}");
    }
}

#[cfg(test)]
mod current_tests {
    use super::*;
    use crate::syntax::parse;

    #[test]
    fn views_show_the_embedded_mediator() {
        let expr = parse("([\"synthesize\"] \"Inspect code\" \"Trace failure\")").unwrap();
        let drawn = tree(&expr);
        assert!(drawn.contains("□ mediator square"));
        assert!(drawn.contains("○ \"synthesize\""));
        assert!(drawn.contains("○ \"Inspect code\""));
        let circuit = mandala(&expr);
        assert!(circuit.contains("[M: o \"synthesize\"]"));
    }

    #[test]
    fn mandala_keeps_functions_inside_the_o_square_o_alphabet() {
        let expr = parse(
            "((~ apply (worker target) (worker target)) \
              (~ inspect (target) (-> target \"Write report\")) \
              (apply inspect \"Inspect parser\"))",
        )
        .unwrap();
        let circuit = mandala(&expr);
        assert!(circuit.contains("~[apply(worker, target)]"));
        assert!(circuit.contains("─[worker]─o"));
        assert!(circuit.contains("~[inspect(target)]"));
        assert!(circuit.contains("─[apply]─o"));
    }
}
