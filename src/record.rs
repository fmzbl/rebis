//! The record: what every Rebis expression is evaluated against.
//!
//! In abraxas this is the life record; at a kaos gate it is the candidates'
//! own text; on the CLI it is whatever the caller pipes in. The calculus is
//! the same everywhere: lines of evidence plus their co-occurrence graph.

use std::collections::{BTreeMap, BTreeSet};

/// Words carrying no intent: dropped so connective tissue neither helps nor
/// hurts resolution.
const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "can", "could", "did", "do", "does",
    "for", "from", "had", "has", "have", "how", "i", "if", "in", "into", "is", "it", "its", "me",
    "my", "no", "not", "of", "on", "or", "our", "please", "should", "so", "than", "that", "the",
    "their", "them", "then", "there", "these", "they", "this", "to", "was", "we", "were", "what",
    "when", "where", "which", "who", "why", "will", "with", "would", "you", "your",
];

/// The content tokens of a text: lowercase alphanumeric runs, stopwords and
/// one-character fragments dropped.
#[must_use]
pub fn content_tokens(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut word = String::new();
    for c in text.chars().chain(std::iter::once(' ')) {
        if c.is_alphanumeric() {
            word.extend(c.to_lowercase());
        } else if !word.is_empty() {
            let w = std::mem::take(&mut word);
            if w.chars().count() > 1 && !STOPWORDS.contains(&w.as_str()) {
                out.insert(w);
            }
        }
    }
    out
}

/// Lines of evidence plus their co-occurrence graph. Raw line text is kept
/// alongside the token sets so agent prompts can quote actual evidence.
/// `Clone` supports snapshot isolation: concurrent square branches each
/// evaluate against a copy and merge accepted answers back in source order.
#[derive(Clone)]
pub struct Record {
    lines: Vec<BTreeSet<String>>,
    raw: Vec<String>,
}

impl Record {
    /// Build a record from texts; every non-empty line becomes evidence.
    #[must_use]
    pub fn from_texts<S: AsRef<str>>(texts: &[S]) -> Record {
        let mut record = Record {
            lines: Vec::new(),
            raw: Vec::new(),
        };
        for text in texts {
            record.append_text(text.as_ref());
        }
        record
    }

    /// Whether the record holds no evidence at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Number of evidence lines.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// Append text as new evidence lines. Generation grows the record;
    /// nothing is ever silently replaced.
    pub fn append_text(&mut self, text: &str) {
        for line in text.lines() {
            let toks = content_tokens(line);
            if !toks.is_empty() {
                self.lines.push(toks);
                self.raw.push(line.trim().to_string());
            }
        }
    }

    /// The tokens of one evidence line, by id.
    #[must_use]
    pub fn line(&self, id: usize) -> Option<&BTreeSet<String>> {
        self.lines.get(id)
    }

    /// The raw text of one evidence line, by id.
    #[must_use]
    pub fn raw(&self, id: usize) -> Option<&str> {
        self.raw.get(id).map(String::as_str)
    }

    /// One-hop co-occurrence neighbors of `term`, strongest first, bounded —
    /// the collider's `related()` scoped to this record.
    fn related(&self, term: &str) -> Vec<(String, u32)> {
        let mut weights: BTreeMap<String, u32> = BTreeMap::new();
        for line in &self.lines {
            if line.contains(term) {
                for other in line {
                    if other != term {
                        *weights.entry(other.clone()).or_insert(0) += 1;
                    }
                }
            }
        }
        let mut out: Vec<(String, u32)> = weights.into_iter().collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        out.truncate(4);
        out
    }

    /// Broaden a term set one hop through the co-occurrence graph.
    pub(crate) fn broaden(&self, terms: &BTreeSet<String>) -> BTreeSet<String> {
        let mut out = terms.clone();
        for term in terms {
            for (rel, _w) in self.related(term) {
                out.insert(rel);
            }
        }
        out
    }

    /// The evidence of a term set: line ids mentioning any term.
    pub(crate) fn evidence(&self, terms: &BTreeSet<String>) -> BTreeSet<usize> {
        self.lines
            .iter()
            .enumerate()
            .filter(|(_, line)| terms.iter().any(|t| line.contains(t)))
            .map(|(id, _)| id)
            .collect()
    }
}

/// What an expression evaluates to: terms, the evidence they resolve to, and
/// the score of the operation that produced them.
#[derive(Clone, Debug, PartialEq)]
pub struct Concept {
    /// The surviving terms.
    pub terms: BTreeSet<String>,
    /// Ids of the record lines this concept rests on.
    pub evidence: BTreeSet<usize>,
    /// Score of the producing operation in `0..=1`: overlap for the square,
    /// surviving fraction for the arrows, 1.0 for resolution/composition.
    pub score: f32,
}
