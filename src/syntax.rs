//! Pure S-expression syntax with executable mediator squares.
//!
//! ```text
//! expr   := prompt | symbol | '\'' expr | ',' expr | '(' seq ')'
//! seq    := '~' symbol '(' symbol* ')' expr
//!         | '#' module | square expr+
//!         | '$' expr+
//!         | '->' expr expr+ | '<-' expr expr+ | symbol expr* | expr+
//! square := '[' expr ']'
//! ```
//!
//! In `([M] A B)`, `A` and `B` are branches and `M` is the Rebis program that
//! mediates their ordered results. Only quoted strings are model prompts;
//! bare atoms are Lisp-like symbols used for definitions and calls.
//!
//! `($ A B ...)` interpolates its operands to one string — a prompt's
//! characters, a symbol's name, a macro's expanded text — and yields that
//! string, firing it only where it sits. It is pure text construction: nothing
//! inside `$` runs. It is the one operator over the language's fundamental
//! value; variables are macro parameters.

use std::fmt;

/// Maximum accepted source size for one parsed Rebis expression.
pub const MAX_SOURCE_BYTES: usize = 1_048_576;
/// Maximum parenthesis, square, or prefix quote nesting accepted by the parser.
pub const MAX_SYNTAX_DEPTH: usize = 256;

/// A validated, host-neutral module path such as `tools` or `std/loops`.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ModuleName(String);

impl ModuleName {
    /// Borrow the canonical module path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ModuleName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Reason a symbolic module path is invalid.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvalidModuleName;

impl fmt::Display for InvalidModuleName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "module names contain non-empty `/`-separated ASCII letters, numbers, `-`, or `_`",
        )
    }
}

impl std::error::Error for InvalidModuleName {}

impl TryFrom<&str> for ModuleName {
    type Error = InvalidModuleName;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let valid = !value.is_empty()
            && value.split('/').all(|segment| {
                !segment.is_empty()
                    && segment != "."
                    && segment != ".."
                    && segment.chars().all(|character| {
                        character.is_ascii_alphanumeric() || "-_".contains(character)
                    })
            });
        valid
            .then(|| Self(value.to_string()))
            .ok_or(InvalidModuleName)
    }
}

/// One executable Rebis expression.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    /// A Lisp-style sequence of two or more top-level forms. The parser creates
    /// this node implicitly; no surrounding parentheses are required.
    Program(Vec<Expr>),
    /// One quoted raw model prompt.
    Prompt(String),
    /// A Lisp-like name or lexical parameter.
    Symbol(String),
    /// `'expr`: inert Rebis syntax returned by a macro.
    Quote(Box<Expr>),
    /// `,expr`: syntax inserted into the surrounding quote.
    Unquote(Box<Expr>),
    /// An abstraction boundary containing a local prompt and/or subprograms.
    Compose(Vec<Expr>),
    /// `($ A B ...)`: string composition. Each operand is interpolated to text
    /// (a prompt's characters, a symbol's name, a macro's expanded text, a
    /// nested composition) — nothing runs — and the assembled string then fires
    /// like a prompt in the position it occupies.
    Concat(Vec<Expr>),
    /// `([M] A B ...)`: branches execute first; `M` mediates their results.
    Square {
        /// Rebis code executed after all branches, with their ordered results.
        mediator: Box<Expr>,
        /// Independent worker expressions.
        branches: Vec<Expr>,
    },
    /// Left-to-right value flow.
    Forward(Box<Expr>, Box<Expr>),
    /// Right-to-left value flow.
    Backflow(Box<Expr>, Box<Expr>),
    /// `(~ name (parameter ...) body)`: a named structural macro abstraction.
    Function {
        /// Function name.
        name: String,
        /// Lexically scoped parameter names.
        params: Vec<String>,
        /// Function body.
        body: Box<Expr>,
    },
    /// `(name argument ...)`.
    Call {
        /// Function name.
        name: String,
        /// Structural arguments.
        args: Vec<Expr>,
    },
    /// `(# module)`: load top-level macro definitions through the host resolver.
    Import {
        /// Host-defined module name.
        module: ModuleName,
    },
}

/// A syntax error and its source location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    /// Human-readable diagnostic.
    pub message: String,
    /// UTF-8 byte offset, when available.
    pub offset: Option<usize>,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for Error {}
impl Error {
    fn at(message: impl Into<String>, offset: usize) -> Self {
        Self {
            message: message.into(),
            offset: Some(offset),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Open,
    Close,
    OpenSquare,
    CloseSquare,
    Forward,
    Backflow,
    Tilde,
    Quote,
    Unquote,
    Dollar,
    Prompt(String),
    Word(String),
}
struct Spanned {
    tok: Tok,
    offset: usize,
}

/// The lexical class of a token, for both parsing and syntax highlighting.
///
/// A highlighter maps these directly to colors; [`parse`] ignores
/// [`TokenKind::Whitespace`] and [`TokenKind::Comment`] and rejects
/// [`TokenKind::Invalid`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TokenKind {
    /// `(` or `)`.
    Paren,
    /// `[` or `]`.
    Bracket,
    /// `~`.
    Tilde,
    /// `'`.
    Quote,
    /// `,`.
    Unquote,
    /// `$`.
    Dollar,
    /// `->`.
    Forward,
    /// `<-`.
    Backflow,
    /// A `"…"` quoted prompt; the span includes both quotes.
    Prompt,
    /// A bare word or symbol, including the `#` import head.
    Symbol,
    /// A `;` line comment, up to but not including the newline.
    Comment,
    /// A run of whitespace.
    Whitespace,
    /// A malformed run the parser rejects, such as an unterminated prompt.
    Invalid,
}

/// A lexical token and its UTF-8 byte span `start..end` in the source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Token {
    /// The token's lexical class.
    pub kind: TokenKind,
    /// Inclusive start byte offset.
    pub start: usize,
    /// Exclusive end byte offset.
    pub end: usize,
}

/// Decode a `"…"` prompt token's text into its string value. `text` begins with
/// the opening quote; `start` is its byte offset, used to place errors. Walks
/// the token honoring escapes, so it correctly reports an unterminated prompt
/// (no closing quote reached) regardless of how the lexer spanned it.
fn decode_prompt(text: &str, start: usize) -> Result<String, Error> {
    let mut out = String::new();
    let mut chars = text.char_indices();
    chars.next(); // opening quote
    loop {
        match chars.next() {
            None => return Err(Error::at("unterminated quoted prompt", start)),
            Some((_, '"')) => return Ok(out),
            Some((_, '\\')) => match chars.next() {
                None => return Err(Error::at("unterminated escape in prompt", start)),
                Some((escaped_rel, escaped)) => out.push(match escaped {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    '"' => '"',
                    '\\' => '\\',
                    other => {
                        return Err(Error::at(
                            format!("unknown escape `\\{other}` in prompt"),
                            start + escaped_rel,
                        ))
                    }
                }),
            },
            Some((_, ch)) => out.push(ch),
        }
    }
}

/// True for a character that breaks a bare word: whitespace, a structural
/// punctuation mark, or a quote. Shared by the word scan so the lexer and any
/// re-scan agree on symbol boundaries.
fn breaks_word(ch: char) -> bool {
    ch.is_whitespace()
        || matches!(
            ch,
            '(' | ')' | '[' | ']' | '~' | '\'' | ',' | '$' | '"' | ';'
        )
}

/// The single-character structural token a character heads, if any.
fn single_kind(c: char) -> Option<TokenKind> {
    match c {
        '(' | ')' => Some(TokenKind::Paren),
        '[' | ']' => Some(TokenKind::Bracket),
        '~' => Some(TokenKind::Tilde),
        '\'' => Some(TokenKind::Quote),
        ',' => Some(TokenKind::Unquote),
        '$' => Some(TokenKind::Dollar),
        _ => None,
    }
}

/// Advance past a `"…"` prompt at char index `start` (the opening quote),
/// honoring escapes; returns the char index just past it, or the end of input
/// when the prompt is never closed.
fn scan_prompt(chars: &[(usize, char)], start: usize) -> usize {
    let mut i = start + 1;
    while i < chars.len() {
        match chars[i].1 {
            // Skip the escape and its target; the bound stops a trailing `\`.
            '\\' => i += 2,
            '"' => return i + 1,
            _ => i += 1,
        }
    }
    chars.len()
}

/// Advance past a bare word at char index `start`; returns the char index just
/// past it. `->` and `<-` end a word so an arrow is never absorbed into one.
fn scan_word(chars: &[(usize, char)], start: usize) -> usize {
    let mut i = start;
    while i < chars.len() {
        let ch = chars[i].1;
        if breaks_word(ch)
            || (ch == '-' && chars.get(i + 1).map(|x| x.1) == Some('>'))
            || (ch == '<' && chars.get(i + 1).map(|x| x.1) == Some('-'))
        {
            break;
        }
        i += 1;
    }
    i
}

/// Tokenize source into a lossless, in-order sequence of spanned tokens.
///
/// Every byte of `src` belongs to exactly one token — whitespace and comments
/// included — so a syntax highlighter can color an entire buffer by mapping
/// each [`TokenKind`] to a style, with no lexer of its own. This never fails:
/// an unterminated prompt still yields a [`TokenKind::Prompt`] span so text
/// being typed stays highlighted. [`parse`] applies the remaining rules —
/// escape validity, prompt termination, nesting depth — that decide whether a
/// lexically spanned program is a syntax error.
#[must_use]
pub fn tokens(src: &str) -> Vec<Token> {
    let chars: Vec<(usize, char)> = src.char_indices().collect();
    let bytes = src.len();
    let end_of = |idx: usize| chars.get(idx).map_or(bytes, |(at, _)| *at);
    let arrow_at = |i: usize, head: char, tail: char| {
        chars[i].1 == head && chars.get(i + 1).map(|x| x.1) == Some(tail)
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let (at, c) = chars[i];
        // Each arm reports the token's kind and the char index just past it; the
        // push and the cursor advance happen once, below.
        let (kind, next) = if let Some(kind) = single_kind(c) {
            (kind, i + 1)
        } else if c.is_whitespace() {
            let mut j = i;
            while j < chars.len() && chars[j].1.is_whitespace() {
                j += 1;
            }
            (TokenKind::Whitespace, j)
        } else if c == ';' {
            // A `;` outside a prompt comments to end of line; inside a prompt it
            // is text, consumed by the prompt scan before reaching here.
            let mut j = i;
            while j < chars.len() && chars[j].1 != '\n' {
                j += 1;
            }
            (TokenKind::Comment, j)
        } else if c == '"' {
            (TokenKind::Prompt, scan_prompt(&chars, i))
        } else if arrow_at(i, '-', '>') {
            (TokenKind::Forward, i + 2)
        } else if arrow_at(i, '<', '-') {
            (TokenKind::Backflow, i + 2)
        } else {
            (TokenKind::Symbol, scan_word(&chars, i))
        };
        out.push(Token {
            kind,
            start: at,
            end: end_of(next),
        });
        i = next;
    }
    out
}

/// Adapt the lossless token stream to the parser's significant-token view:
/// whitespace and comments are dropped, prompt escapes are decoded, and a
/// malformed token becomes a syntax error at its offset.
fn lex(src: &str) -> Result<Vec<Spanned>, Error> {
    let mut out = Vec::new();
    for Token { kind, start, end } in tokens(src) {
        let text = &src[start..end];
        let tok = match kind {
            TokenKind::Whitespace | TokenKind::Comment => continue,
            TokenKind::Paren if text == "(" => Tok::Open,
            TokenKind::Paren => Tok::Close,
            TokenKind::Bracket if text == "[" => Tok::OpenSquare,
            TokenKind::Bracket => Tok::CloseSquare,
            TokenKind::Tilde => Tok::Tilde,
            TokenKind::Quote => Tok::Quote,
            TokenKind::Unquote => Tok::Unquote,
            TokenKind::Dollar => Tok::Dollar,
            TokenKind::Forward => Tok::Forward,
            TokenKind::Backflow => Tok::Backflow,
            TokenKind::Prompt => Tok::Prompt(decode_prompt(text, start)?),
            TokenKind::Symbol => Tok::Word(text.to_string()),
            TokenKind::Invalid => {
                return Err(Error::at(
                    format!(
                        "unexpected character `{}`",
                        text.chars().next().unwrap_or(' ')
                    ),
                    start,
                ))
            }
        };
        out.push(Spanned { tok, offset: start });
    }
    Ok(out)
}

fn validate_limits(toks: &[Spanned]) -> Result<(), Error> {
    let mut structural_depth = 0usize;
    let mut prefix_depth = 0usize;
    for token in toks {
        match token.tok {
            Tok::Open | Tok::OpenSquare => {
                structural_depth += 1;
                prefix_depth = 0;
                if structural_depth > MAX_SYNTAX_DEPTH {
                    return Err(Error::at("maximum syntax depth exceeded", token.offset));
                }
            }
            Tok::Close | Tok::CloseSquare => {
                structural_depth = structural_depth.saturating_sub(1);
                prefix_depth = 0;
            }
            Tok::Quote | Tok::Unquote => {
                prefix_depth += 1;
                if prefix_depth > MAX_SYNTAX_DEPTH {
                    return Err(Error::at("maximum syntax depth exceeded", token.offset));
                }
            }
            _ => prefix_depth = 0,
        }
    }
    Ok(())
}

/// Parse one Rebis expression or a Lisp-style sequence of top-level forms.
///
/// # Errors
///
/// Returns a diagnostic with a UTF-8 byte offset when the source is malformed.
/// Multiple top-level forms become one implicit [`Expr::Program`], preserving
/// the lexical scope of definitions without changing what parentheses mean.
pub fn parse(src: &str) -> Result<Expr, Error> {
    if src.len() > MAX_SOURCE_BYTES {
        return Err(Error::at("maximum source size exceeded", MAX_SOURCE_BYTES));
    }
    let toks = lex(src)?;
    validate_limits(&toks)?;
    let mut pos = 0;
    let mut forms = Vec::new();
    while pos < toks.len() {
        forms.push(parse_expr(&toks, &mut pos, src.len())?);
    }
    match forms.len() {
        0 => Err(Error {
            message: "empty expression".into(),
            offset: Some(src.len()),
        }),
        1 => forms.into_iter().next().ok_or_else(|| Error {
            message: "empty expression".into(),
            offset: Some(src.len()),
        }),
        _ => Ok(Expr::Program(forms)),
    }
}

fn items_until(
    toks: &[Spanned],
    pos: &mut usize,
    end: usize,
    close_square: bool,
) -> Result<Vec<Expr>, Error> {
    let mut items = Vec::new();
    loop {
        match toks.get(*pos) {
            Some(Spanned {
                tok: Tok::Close, ..
            }) if !close_square => break,
            Some(Spanned {
                tok: Tok::CloseSquare,
                ..
            }) if close_square => break,
            None => {
                return Err(Error::at(
                    if close_square {
                        "unbalanced `[`"
                    } else {
                        "unbalanced `(`"
                    },
                    end,
                ))
            }
            Some(Spanned {
                tok: Tok::Word(_) | Tok::Prompt(_) | Tok::Quote | Tok::Unquote | Tok::Open,
                ..
            }) => items.push(parse_expr(toks, pos, end)?),
            Some(Spanned { offset, .. }) => {
                return Err(Error::at("unexpected structural token", *offset))
            }
        }
    }
    Ok(items)
}

fn fold_arrow(mut items: Vec<Expr>, forward: bool, at: usize) -> Result<Expr, Error> {
    if items.len() < 2 {
        return Err(Error::at("an arrow needs at least two operands", at));
    }
    let mut acc = items.remove(0);
    for item in items {
        acc = if forward {
            Expr::Forward(Box::new(acc), Box::new(item))
        } else {
            Expr::Backflow(Box::new(acc), Box::new(item))
        };
    }
    Ok(acc)
}

fn parse_function(toks: &[Spanned], pos: &mut usize, end: usize, at: usize) -> Result<Expr, Error> {
    let name = match toks.get(*pos) {
        Some(Spanned {
            tok: Tok::Word(name),
            ..
        }) => name.clone(),
        Some(token) => return Err(Error::at("expected function name", token.offset)),
        None => return Err(Error::at("unfinished function definition", end)),
    };
    if name == "#" {
        return Err(Error::at("`#` is reserved for module imports", at));
    }
    *pos += 1;
    if !matches!(toks.get(*pos).map(|token| &token.tok), Some(Tok::Open)) {
        return Err(Error::at("function parameters must be a list", at));
    }
    *pos += 1;
    let mut params = Vec::new();
    while let Some(token) = toks.get(*pos) {
        match &token.tok {
            Tok::Word(param) => {
                if params.contains(param) {
                    return Err(Error::at(
                        format!("duplicate function parameter `{param}`"),
                        token.offset,
                    ));
                }
                params.push(param.clone());
                *pos += 1;
            }
            Tok::Close => break,
            _ => {
                return Err(Error::at(
                    "function parameters must be symbols",
                    token.offset,
                ))
            }
        }
    }
    if !matches!(toks.get(*pos).map(|token| &token.tok), Some(Tok::Close)) {
        return Err(Error::at("unbalanced function parameter list", end));
    }
    *pos += 1;
    let body = parse_expr(toks, pos, end)?;
    if !matches!(toks.get(*pos).map(|token| &token.tok), Some(Tok::Close)) {
        return Err(Error::at("a function definition has exactly one body", at));
    }
    *pos += 1;
    Ok(Expr::Function {
        name,
        params,
        body: Box::new(body),
    })
}

fn parse_group(toks: &[Spanned], pos: &mut usize, end: usize, at: usize) -> Result<Expr, Error> {
    if matches!(toks.get(*pos).map(|token| &token.tok), Some(Tok::Tilde)) {
        *pos += 1;
        return parse_function(toks, pos, end, at);
    }
    if matches!(
        toks.get(*pos).map(|token| &token.tok),
        Some(Tok::OpenSquare)
    ) {
        *pos += 1;
        let mediator_items = items_until(toks, pos, end, true)?;
        if mediator_items.len() != 1 {
            return Err(Error::at(
                "a mediator square contains exactly one atom or Rebis block",
                at,
            ));
        }
        *pos += 1;
        let branches = items_until(toks, pos, end, false)?;
        if branches.is_empty() {
            return Err(Error::at("a mediator square needs at least one branch", at));
        }
        *pos += 1;
        return Ok(Expr::Square {
            mediator: Box::new(mediator_items.into_iter().next().expect("length checked")),
            branches,
        });
    }
    if matches!(toks.get(*pos).map(|token| &token.tok), Some(Tok::Dollar)) {
        *pos += 1;
        let operands = items_until(toks, pos, end, false)?;
        if operands.is_empty() {
            return Err(Error::at(
                "a `$` composition needs at least one operand",
                at,
            ));
        }
        *pos += 1;
        return Ok(Expr::Concat(operands));
    }
    let arrow = match toks.get(*pos).map(|token| &token.tok) {
        Some(Tok::Forward) => {
            *pos += 1;
            Some(true)
        }
        Some(Tok::Backflow) => {
            *pos += 1;
            Some(false)
        }
        _ => None,
    };
    let items = items_until(toks, pos, end, false)?;
    *pos += 1;
    if let Some(forward) = arrow {
        fold_arrow(items, forward, at)
    } else if items.is_empty() {
        Err(Error::at("empty group", at))
    } else if matches!(items.first(), Some(Expr::Symbol(name)) if name == "#") {
        match items.as_slice() {
            [Expr::Symbol(_), Expr::Symbol(module)] => ModuleName::try_from(module.as_str())
                .map(|module| Expr::Import { module })
                .map_err(|error| Error::at(error.to_string(), at)),
            _ => Err(Error::at(
                "a `#` import has exactly one bare module name",
                at,
            )),
        }
    } else if let Some(Expr::Symbol(name)) = items.first() {
        Ok(Expr::Call {
            name: name.clone(),
            args: items[1..].to_vec(),
        })
    } else {
        Ok(Expr::Compose(items))
    }
}

fn parse_expr(toks: &[Spanned], pos: &mut usize, end: usize) -> Result<Expr, Error> {
    match toks.get(*pos) {
        Some(Spanned {
            tok: Tok::Quote,
            offset,
        }) => {
            let at = *offset;
            *pos += 1;
            if *pos == toks.len() {
                return Err(Error::at("quote needs an expression", at));
            }
            Ok(Expr::Quote(Box::new(parse_expr(toks, pos, end)?)))
        }
        Some(Spanned {
            tok: Tok::Unquote,
            offset,
        }) => {
            let at = *offset;
            *pos += 1;
            if *pos == toks.len() {
                return Err(Error::at("unquote needs an expression", at));
            }
            Ok(Expr::Unquote(Box::new(parse_expr(toks, pos, end)?)))
        }
        Some(Spanned {
            tok: Tok::Word(word),
            ..
        }) => {
            *pos += 1;
            Ok(Expr::Symbol(word.clone()))
        }
        Some(Spanned {
            tok: Tok::Prompt(prompt),
            ..
        }) => {
            *pos += 1;
            Ok(Expr::Prompt(prompt.clone()))
        }
        Some(Spanned {
            tok: Tok::Open,
            offset,
        }) => {
            let at = *offset;
            *pos += 1;
            parse_group(toks, pos, end, at)
        }
        Some(Spanned { offset, .. }) => Err(Error::at("expected an atom or `(`", *offset)),
        None => Err(Error {
            message: "empty expression".into(),
            offset: Some(end),
        }),
    }
}

/// Write a quoted prompt, escaping the characters the lexer treats specially.
fn write_escaped_prompt(text: &str, out: &mut String) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            ch => out.push(ch),
        }
    }
    out.push('"');
}

/// Render a parsed expression as canonical Rebis source.
#[must_use]
pub fn format(expr: &Expr) -> String {
    fn write(expr: &Expr, out: &mut String) {
        match expr {
            Expr::Program(forms) => {
                for (index, form) in forms.iter().enumerate() {
                    if index > 0 {
                        out.push('\n');
                    }
                    write(form, out);
                }
            }
            Expr::Prompt(text) => write_escaped_prompt(text, out),
            Expr::Symbol(name) => out.push_str(name),
            Expr::Quote(inner) => {
                out.push('\'');
                write(inner, out);
            }
            Expr::Unquote(inner) => {
                out.push(',');
                write(inner, out);
            }
            Expr::Compose(items) => {
                out.push('(');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(' ');
                    }
                    write(item, out);
                }
                out.push(')');
            }
            Expr::Concat(items) => {
                out.push_str("($");
                for item in items {
                    out.push(' ');
                    write(item, out);
                }
                out.push(')');
            }
            Expr::Square { mediator, branches } => {
                out.push_str("([");
                write(mediator, out);
                out.push(']');
                for branch in branches {
                    out.push(' ');
                    write(branch, out);
                }
                out.push(')');
            }
            Expr::Forward(a, b) => {
                out.push_str("(-> ");
                write(a, out);
                out.push(' ');
                write(b, out);
                out.push(')');
            }
            Expr::Backflow(a, b) => {
                out.push_str("(<- ");
                write(a, out);
                out.push(' ');
                write(b, out);
                out.push(')');
            }
            Expr::Function { name, params, body } => {
                out.push_str("(~ ");
                out.push_str(name);
                out.push_str(" (");
                out.push_str(&params.join(" "));
                out.push_str(") ");
                write(body, out);
                out.push(')');
            }
            Expr::Call { name, args } => {
                out.push('(');
                out.push_str(name);
                for arg in args {
                    out.push(' ');
                    write(arg, out);
                }
                out.push(')');
            }
            Expr::Import { module } => {
                out.push_str("(# ");
                out.push_str(module.as_str());
                out.push(')');
            }
        }
    }
    let mut out = String::new();
    write(expr, &mut out);
    out
}

/// Render a parsed expression as readable, indented Rebis source.
///
/// Unlike [`format()`], this is intended for an interactive editor: structural
/// forms are spread over rows while atoms and prompt contents remain intact.
#[must_use]
pub fn pretty_format(expr: &Expr) -> String {
    fn indent(out: &mut String, depth: usize) {
        out.extend(std::iter::repeat(' ').take(depth * 2));
    }

    fn child(expr: &Expr, out: &mut String, depth: usize) {
        out.push('\n');
        indent(out, depth);
        write(expr, out, depth);
    }

    fn write(expr: &Expr, out: &mut String, depth: usize) {
        match expr {
            Expr::Program(forms) => {
                for (index, form) in forms.iter().enumerate() {
                    if index > 0 {
                        out.push_str("\n\n");
                    }
                    write(form, out, depth);
                }
            }
            Expr::Prompt(text) => write_escaped_prompt(text, out),
            Expr::Symbol(name) => out.push_str(name),
            Expr::Quote(inner) => {
                out.push('\'');
                write(inner, out, depth);
            }
            Expr::Unquote(inner) => {
                out.push(',');
                write(inner, out, depth);
            }
            Expr::Compose(items) => {
                out.push('(');
                for item in items {
                    child(item, out, depth + 1);
                }
                out.push(')');
            }
            // A composition reads best on one line: its operands are text
            // fragments, not sub-programs to spread over rows.
            Expr::Concat(items) => {
                out.push_str("($");
                for item in items {
                    out.push(' ');
                    write(item, out, depth);
                }
                out.push(')');
            }
            Expr::Square { mediator, branches } => {
                out.push_str("([");
                write(mediator, out, depth + 1);
                out.push(']');
                for branch in branches {
                    child(branch, out, depth + 1);
                }
                out.push(')');
            }
            Expr::Forward(a, b) => {
                out.push_str("(->");
                child(a, out, depth + 1);
                child(b, out, depth + 1);
                out.push(')');
            }
            Expr::Backflow(a, b) => {
                out.push_str("(<-");
                child(a, out, depth + 1);
                child(b, out, depth + 1);
                out.push(')');
            }
            Expr::Function { name, params, body } => {
                out.push_str("(~ ");
                out.push_str(name);
                out.push_str(" (");
                out.push_str(&params.join(" "));
                out.push(')');
                child(body, out, depth + 1);
                out.push(')');
            }
            Expr::Call { name, args } => {
                out.push('(');
                out.push_str(name);
                for arg in args {
                    child(arg, out, depth + 1);
                }
                out.push(')');
            }
            Expr::Import { module } => {
                out.push_str("(# ");
                out.push_str(module.as_str());
                out.push(')');
            }
        }
    }

    let mut out = String::new();
    write(expr, &mut out, 0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `(kind, slice)` view of every token, for concise assertions.
    fn token_view(src: &str) -> Vec<(TokenKind, &str)> {
        tokens(src)
            .into_iter()
            .map(|token| (token.kind, &src[token.start..token.end]))
            .collect()
    }

    #[test]
    fn tokens_span_the_whole_source_losslessly() {
        // Every byte belongs to exactly one token, in order, with no gaps — the
        // property a highlighter relies on.
        let src = "(-> a \"hi\") ; note\n($ x)";
        let toks = tokens(src);
        assert_eq!(toks.first().map(|t| t.start), Some(0));
        assert_eq!(toks.last().map(|t| t.end), Some(src.len()));
        for pair in toks.windows(2) {
            assert_eq!(pair[0].end, pair[1].start, "tokens must be contiguous");
        }
        let rebuilt: String = toks.iter().map(|t| &src[t.start..t.end]).collect();
        assert_eq!(rebuilt, src);
    }

    #[test]
    fn tokens_classify_every_operator_and_trivia() {
        assert_eq!(
            token_view("($ a -> b) ; c"),
            vec![
                (TokenKind::Paren, "("),
                (TokenKind::Dollar, "$"),
                (TokenKind::Whitespace, " "),
                (TokenKind::Symbol, "a"),
                (TokenKind::Whitespace, " "),
                (TokenKind::Forward, "->"),
                (TokenKind::Whitespace, " "),
                (TokenKind::Symbol, "b"),
                (TokenKind::Paren, ")"),
                (TokenKind::Whitespace, " "),
                (TokenKind::Comment, "; c"),
            ]
        );
    }

    #[test]
    fn tokens_keep_a_dollar_inside_a_prompt_as_prompt_text() {
        // The lexer never peeks into a string: `$` there is part of the prompt.
        assert_eq!(
            token_view("\"it cost $100\""),
            vec![(TokenKind::Prompt, "\"it cost $100\"")]
        );
    }

    #[test]
    fn an_unterminated_prompt_still_spans_as_a_prompt_token() {
        // Highlighting stays sane mid-typing; parse is what rejects it.
        assert_eq!(token_view("\"open"), vec![(TokenKind::Prompt, "\"open")]);
        assert!(parse("\"open").is_err());
    }

    #[test]
    fn tokens_and_parse_agree_on_arrow_versus_word_boundaries() {
        // `a->b` is three tokens; a lone `-` stays inside a word.
        assert_eq!(
            token_view("a->b a-b"),
            vec![
                (TokenKind::Symbol, "a"),
                (TokenKind::Forward, "->"),
                (TokenKind::Symbol, "b"),
                (TokenKind::Whitespace, " "),
                (TokenKind::Symbol, "a-b"),
            ]
        );
    }

    #[test]
    fn dollar_composition_parses_and_round_trips() {
        let expr = parse("($ \"hi \" name)").unwrap();
        assert_eq!(
            expr,
            Expr::Concat(vec![
                Expr::Prompt("hi ".into()),
                Expr::Symbol("name".into()),
            ])
        );
        assert_eq!(format(&expr), "($ \"hi \" name)");
        assert_eq!(parse(&format(&expr)).unwrap(), expr);
    }

    #[test]
    fn dollar_inside_a_prompt_stays_literal_text() {
        // The operator never peeks inside a string: `$` in a prompt is content.
        assert_eq!(
            parse("\"it cost $100\"").unwrap(),
            Expr::Prompt("it cost $100".into())
        );
    }

    #[test]
    fn empty_composition_is_a_syntax_error() {
        assert!(parse("($)").is_err());
    }

    #[test]
    fn let_is_not_a_keyword_and_stays_an_ordinary_call() {
        // The base language has no `let` sugar: `(let …)` is just a call to a
        // macro named `let`, like any other symbol head.
        assert_eq!(
            parse("(let a b)").unwrap(),
            Expr::Call {
                name: "let".into(),
                args: vec![Expr::Symbol("a".into()), Expr::Symbol("b".into())],
            }
        );
    }

    #[test]
    fn parses_parameterized_square() {
        let expr = parse("([\"Combine reports\"] \"Inspect code\" \"Trace failure\")").unwrap();
        let Expr::Square { mediator, branches } = expr else {
            panic!()
        };
        assert_eq!(*mediator, Expr::Prompt("Combine reports".into()));
        assert_eq!(branches.len(), 2);
    }
    #[test]
    fn prompts_are_quoted_and_symbols_are_distinct() {
        assert_eq!(
            parse("(\"hello hello\" (identity \"bye\"))").unwrap(),
            Expr::Compose(vec![
                Expr::Prompt("hello hello".into()),
                Expr::Call {
                    name: "identity".into(),
                    args: vec![Expr::Prompt("bye".into())]
                }
            ])
        );
    }

    #[test]
    fn parses_lisp_style_top_level_forms_as_an_implicit_program() {
        let source = "(~ investigate (topic) (-> topic \"Investigate deeply\"))\n\n\
                      ([\"Build an app\"]\n\
                        (investigate \"fibonacci\")\n\
                        (investigate \"chaos magic\"))";
        let expression = parse(source).unwrap();
        let Expr::Program(forms) = &expression else {
            panic!("multiple top-level forms must produce a program")
        };
        assert_eq!(forms.len(), 2);
        assert!(matches!(forms[0], Expr::Function { .. }));
        assert!(matches!(forms[1], Expr::Square { .. }));
        assert_eq!(parse(&format(&expression)).unwrap(), expression);
        assert_eq!(parse(&pretty_format(&expression)).unwrap(), expression);
    }

    #[test]
    fn pretty_format_is_multiline_and_round_trips() {
        let expr = parse("((~ pair (a b) '(-> ,a ,b)) (pair \"left\" \"right\"))").unwrap();
        let rendered = pretty_format(&expr);
        assert!(rendered.lines().count() > 4);
        assert_ne!(rendered.lines().last().unwrap().trim(), ")");
        assert!(rendered.ends_with("))"));
        assert_eq!(parse(&rendered).unwrap(), expr);
    }
    #[test]
    fn mediator_may_be_a_whole_program() {
        let src = "([ (-> \"Compare reports\" \"Write result\") ] \"Inspect code\" \"Trace bug\")";
        let expr = parse(src).unwrap();
        assert_eq!(parse(&format(&expr)).unwrap(), expr);
    }
    #[test]
    fn punctuation_is_raw_prompt_text() {
        assert_eq!(
            parse("\"Fix parser.rs: handle UTF-8 correctly!\"").unwrap(),
            Expr::Prompt("Fix parser.rs: handle UTF-8 correctly!".into())
        );
    }
    #[test]
    fn arrows_and_errors() {
        assert!(matches!(
            parse("(-> \"one\" \"two\")").unwrap(),
            Expr::Forward(..)
        ));
        assert!(parse("([] a b)").is_err()); // empty mediator
        assert!(parse("([x])").is_err()); // mediator, no branches
        assert!(parse("([x] \"a\")").is_ok());
        assert!(parse("[a]").is_err()); // a mediator square needs its `(` wrapper
    }
    #[test]
    fn parses_function_definition_and_call() {
        let src = "((~ inspect (target) (-> target \"Write report\")) (inspect \"the parser\"))";
        let expr = parse(src).unwrap();
        assert_eq!(parse(&format(&expr)).unwrap(), expr);
    }

    #[test]
    fn parses_and_formats_quote_and_unquote() {
        let src = "(~ twice (work) '(-> ,work ,work))";
        let expr = parse(src).unwrap();
        assert_eq!(parse(&format(&expr)).unwrap(), expr);
        let Expr::Function { body, .. } = expr else {
            panic!()
        };
        assert!(matches!(body.as_ref(), Expr::Quote(_)));
    }
}
