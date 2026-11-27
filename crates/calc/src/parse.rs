//! The calculation language parser: a lexer and a Pratt parser with explicit depth limits.
//!
//! Grammar, loosest binding first:
//!
//! ```text
//! statement := head ":=" body [ show ]                          define (and optionally show)
//!            | expr show                                       evaluate and show
//!            | "Given"                                         opens a block of equations
//!            | expr "==" expr                                  an equation of that block
//! head      := name | name "(" params ")" | name"[" name "]"   variable, function, indexed
//!            | "[" name { ";" name } "]"                       two or more names, one column
//! body      := range | cases | expr
//! range     := expr [ "," expr ] ".." expr                    first, second..last; `..` loosest
//! cases     := case { "," case } [ "," expr "otherwise" ]      the first case that holds wins
//! case      := expr "if" expr                                  value if condition
//! show      := "=" [ "?" ] [ unit text ]                       the rest of the text is the unit
//! expr      := or
//! or        := and { ("or" | "∨") and }
//! and       := not { ("and" | "∧") not }
//! not       := ("not" | "¬") not | compare                     the operand is a whole comparison
//! compare   := sum { ("<" | ">" | "≤" | "<=" | "≥" | ">=" | "==" | "≠" | "!=") sum }
//!                                                               a chain, `a < b ≤ c`, is one comparison
//! sum       := term { ("+" | "-") term }
//! term      := unary { ("*" | "/" | adjacency) unary }
//! unary     := "-" unary | power                               so -x^2 is -(x^2)
//! power     := atom [ "^" unary ]                              right-associative
//! atom      := number | text | name | name"(" args ")" | name"[" args "]" | "(" expr ")"
//!            | "if(" expr "," expr "," expr ")"                  if(condition, then, else)
//!            | "∑" "(" name "=" expr [ "," expr ] ".." expr ")" term
//!                                                               a sum: first..last, or first, second..last
//!            | "[" rows "]" | "|" expr "|" | "√" atom
//! rows      := items { ";" items }                             every row the same length
//! items     := expr { "," expr }
//! postfix   := atom { "²" | "³" | "⁻¹" ... }                   one superscript digit, like `^`
//! ```
//!
//! `and`, `or`, `not`, `if` and `otherwise` are keywords, never names. A `==` is a comparison; a
//! lone `=` still ends the statement. `if` touching a `(` is the function `if(c, a, b)`, which
//! like any call can be a factor of an implied product (`2 if(c, a, b)`); anywhere else `if` and
//! `otherwise` belong to a list of cases, which, like a range, can only be the whole right side of
//! a definition, so its commas are never confused with list or argument commas.
//!
//! `∑` takes one product term, as `·` does: `∑(i = 1..n) a[i]·b[i] + 1` is `(∑ a[i]·b[i]) + 1`;
//! write `∑(i = 1..n) (a[i] + 1)` to sum a sum of terms. Its ends are read like the values of a
//! range, and the `=` after `∑(` and the index is the sum's own, not the one that shows the result.
//! The index belongs to the sum alone. Sums nest, and the ends of a sum count as nesting like a
//! bracket.
//!
//! A text is written between quotes, `"OK"` or `“OK”`, and is at most [`MAX_TEXT_CHARS`] characters.
//! It never multiplies by adjacency (`2 "OK"` is an error).
//!
//! A `(` or `[` right after a name, with no space, is a call or an index (`f(x)`, `D[2]`); after
//! a space it starts a factor (`D [2]` is `D` times a one-element literal). A literal with no `;`
//! is a column (`[1, 2, 3]` is 3 rows × 1). A range (`0..5`, `1, 1.5..3`) can only be the whole
//! right side of a definition, so its commas are never confused with list or argument commas.
//!
//! Adjacency (implicit multiplication) groups exactly like `*`, left to right, so `1/2 x` is
//! `(1/2) x` and not `1/(2 x)` (the Mathcad convention). A number directly after a number, name
//! or closing bracket (`1 500`, `x 2`) is an error rather than a product.
//!
//! The parser knows nothing about units or variables. `5 kN` is `5 * kN`; the evaluator decides
//! what `kN` is. Hostile input is bounded three ways: the source is at most [`MAX_SOURCE_BYTES`],
//! the token list at most [`MAX_TOKENS`], the parser's recursion (nesting) at most [`MAX_DEPTH`] and
//! the height of the tree it builds at most [`MAX_HEIGHT`], so nothing downstream recurses deeply.

use crate::array::MAX_TEXT_CHARS;
use crate::{CalcError, Cmp, Expr, Logic, Op, Show, Stmt};

#[cfg(test)]
mod tests;

/// Largest source text accepted, in bytes.
const MAX_SOURCE_BYTES: usize = 16 * 1024;
/// Most tokens accepted.
const MAX_TOKENS: usize = 10_000;
/// Deepest nesting of expressions (parentheses, calls, literals, indexes, `|…|`, prefix signs and
/// `^` operands) accepted: one more is `TooComplex`.
const MAX_DEPTH: usize = 64;
/// Tallest syntax tree (levels, counting the leaves) accepted. A flat sum of `n` terms is `n`
/// levels tall, so this also bounds how long a sum or product can be.
const MAX_HEIGHT: usize = 1024;

// Binding powers: (left, right) for infix operators; a higher number binds tighter.
const OR_BP: (u8, u8) = (2, 3);
const AND_BP: (u8, u8) = (4, 5);
/// The operand of a prefix `not`: a whole comparison, but no `and` or `or`, so
/// `not a < b and c` is `(not (a < b)) and c`.
const NOT_BP: u8 = 7;
/// Comparisons chain (`a < b ≤ c`), so the right power only decides where each operand ends.
const CMP_BP: (u8, u8) = (8, 9);
const ADD_BP: (u8, u8) = (10, 11);
const MUL_BP: (u8, u8) = (20, 21);
/// Prefix minus: tighter than `*`, looser than `^`, so `-x^2` is `-(x^2)` and `-a*b` is `(-a)*b`.
const NEG_BP: u8 = 30;
/// `^` is right-associative: its right binding power is below its left.
const POW_BP: (u8, u8) = (40, 39);
/// The operand of `√` takes only an atom: `√x^2` is `(√x)^2`.
const SQRT_BP: u8 = 41;

/// Parse one statement.
///
/// # Errors
/// [`CalcError::Syntax`] when the text isn't a valid statement, and [`CalcError::TooComplex`]
/// when it exceeds the size, token or nesting limits.
pub fn parse(src: &str) -> Result<Stmt, CalcError> {
    if src.len() > MAX_SOURCE_BYTES {
        return Err(CalcError::TooComplex);
    }
    // A block opens at a region that is `Given` and nothing else.
    if src.trim() == "Given" {
        return Ok(Stmt::Given);
    }
    let toks = lex(src)?;
    match toks.iter().position(|t| t.kind == Kind::Define) {
        Some(at) => {
            let head = toks.get(..at).unwrap_or_default();
            let mut p = Parser::new(toks.get(at + 1..).unwrap_or_default());
            if matches!(head.first(), Some(Tok { kind: Kind::LBracket, .. })) {
                let names = many_head(src, head)?;
                let body = p.body()?.expr;
                let show = p.finish(src)?;
                return Ok(Stmt::DefineMany { names, body, show });
            }
            let (name, index, params) = define_head(src, head)?;
            let body = p.body()?.expr;
            let show = p.finish(src)?;
            Ok(Stmt::Define { name, index, params, body, show })
        }
        None => {
            let mut p = Parser::new(&toks);
            let expr = p.root()?.expr;
            match p.finish(src)? {
                Some(show) => Ok(Stmt::Eval { expr, show }),
                None => constraint(expr),
            }
        }
    }
}

/// A root expression with no `=` after it: an equation of a solve block when it is one comparison
/// by `==`, else the error for a statement that shows nothing and defines nothing.
fn constraint(expr: Expr) -> Result<Stmt, CalcError> {
    match expr {
        Expr::Compare { first, mut rest } if matches!(rest.as_slice(), [(Cmp::Eq, _)]) => match rest.pop() {
            Some((_, rhs)) => Ok(Stmt::Constraint { lhs: *first, rhs }),
            None => Err(no_result()),
        },
        Expr::Compare { rest, .. } if rest.iter().filter(|(cmp, _)| *cmp == Cmp::Eq).count() > 1 => Err(syntax("an equation has one ==")),
        _ => Err(no_result()),
    }
}

fn no_result() -> CalcError {
    syntax("end with `=` to show the result, or use `:=` to define a name")
}

fn syntax(msg: impl Into<String>) -> CalcError {
    CalcError::Syntax(msg.into())
}

/// An `otherwise` with no case before it, or a case's value with neither `if` nor `otherwise`.
fn no_case_before_otherwise() -> CalcError {
    syntax("each case is a value and a condition, like a if x > 0, and only the last may be b otherwise")
}

/// A `..` anywhere but between the ends of a definition's whole right side.
fn misplaced_range() -> CalcError {
    syntax("a range like 0..5 can only be the whole right side of a definition")
}

/// An `if` or `otherwise` anywhere but in the cases of a definition's whole right side.
fn misplaced_cases() -> CalcError {
    syntax("a list of cases (a if c, b otherwise) can only be the whole right side of a definition")
}

// ---------------------------------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Kind {
    Num(f64),
    /// A text literal's content, without its quotes.
    Text(String),
    Name(String),
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    Bar,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    /// Separates the rows of a literal.
    Semi,
    /// `..`, between the ends of a range.
    DotDot,
    Sqrt,
    /// A superscript exponent such as `²` or `⁻¹`, already decoded.
    Sup(i32),
    /// `:=` or `≔`
    Define,
    /// A lone `=`; the lexer stops here, and the text after it is the unit.
    Eq,
    /// `<`
    Lt,
    /// `>`
    Gt,
    /// `≤` or `<=`
    Le,
    /// `≥` or `>=`
    Ge,
    /// `==`
    EqEq,
    /// `≠` or `!=`
    Ne,
    /// `and` or `∧`
    And,
    /// `or` or `∨`
    Or,
    /// `not` or `¬`
    Not,
    /// `if`
    If,
    /// `otherwise`
    Otherwise,
    /// `∑`
    Sum,
    /// The `=` inside a sum's parentheses, `∑(i = 1..n)`: it separates the index from its range,
    /// and doesn't end the statement.
    SumEq,
}

/// Whether `name` is a keyword (`and`, `or`, `not`, `if`, `otherwise`) rather than a name.
pub(crate) fn is_keyword(name: &str) -> bool {
    keyword(name).is_some()
}

/// The token a whole name is, when it is a keyword.
fn keyword(name: &str) -> Option<Kind> {
    match name {
        "and" => Some(Kind::And),
        "or" => Some(Kind::Or),
        "not" => Some(Kind::Not),
        "if" => Some(Kind::If),
        "otherwise" => Some(Kind::Otherwise),
        _ => None,
    }
}

/// The comparison a token is.
fn comparison(kind: Option<&Kind>) -> Option<Cmp> {
    match kind? {
        Kind::Lt => Some(Cmp::Lt),
        Kind::Gt => Some(Cmp::Gt),
        Kind::Le => Some(Cmp::Le),
        Kind::Ge => Some(Cmp::Ge),
        Kind::EqEq => Some(Cmp::Eq),
        Kind::Ne => Some(Cmp::Ne),
        _ => None,
    }
}

impl Kind {
    fn describe(&self) -> String {
        match self {
            Kind::Num(_) => "a number".into(),
            Kind::Text(_) => "a text".into(),
            Kind::Name(n) => format!("`{n}`"),
            Kind::Plus => "`+`".into(),
            Kind::Minus => "`-`".into(),
            Kind::Star => "`*`".into(),
            Kind::Slash => "`/`".into(),
            Kind::Caret => "`^`".into(),
            Kind::Bar => "`|`".into(),
            Kind::LParen => "`(`".into(),
            Kind::RParen => "`)`".into(),
            Kind::LBracket => "`[`".into(),
            Kind::RBracket => "`]`".into(),
            Kind::Comma => "`,`".into(),
            Kind::Semi => "`;`".into(),
            Kind::DotDot => "`..`".into(),
            Kind::Sqrt => "`√`".into(),
            Kind::Sup(_) => "a superscript".into(),
            Kind::Define => "`:=`".into(),
            Kind::Eq => "`=`".into(),
            Kind::Lt => "`<`".into(),
            Kind::Gt => "`>`".into(),
            Kind::Le => "`≤`".into(),
            Kind::Ge => "`≥`".into(),
            Kind::EqEq => "`==`".into(),
            Kind::Ne => "`≠`".into(),
            Kind::And => "`and`".into(),
            Kind::Or => "`or`".into(),
            Kind::Not => "`not`".into(),
            Kind::If => "`if`".into(),
            Kind::Otherwise => "`otherwise`".into(),
            Kind::Sum => "`∑`".into(),
            Kind::SumEq => "`=`".into(),
        }
    }
}

/// A token and the byte range of the source it came from.
#[derive(Debug, Clone, PartialEq)]
struct Tok {
    kind: Kind,
    start: usize,
    end: usize,
}

/// A name may start with a letter (any script, so Greek works) or one of the unit marks
/// (`°F`, `Δ°F`), so unit names lex as names.
fn is_name_start(c: char) -> bool {
    c.is_alphabetic() || matches!(c, '°' | '∆' | 'Δ')
}

/// A name continues with letters, digits, `_` (the subscript) and the unit marks.
fn is_name_continue(c: char) -> bool {
    (c.is_alphanumeric() && superscript_digit(c).is_none()) || matches!(c, '_' | '°' | '∆' | 'Δ' | 'µ' | 'μ')
}

/// The value of a superscript digit (`⁰`..`⁹`). These are not name characters: `x²` is `x^2`.
fn superscript_digit(c: char) -> Option<i32> {
    match c {
        '⁰' => Some(0),
        '¹' => Some(1),
        '²' => Some(2),
        '³' => Some(3),
        '⁴' => Some(4),
        '⁵' => Some(5),
        '⁶' => Some(6),
        '⁷' => Some(7),
        '⁸' => Some(8),
        '⁹' => Some(9),
        _ => None,
    }
}

/// A run of superscript characters is one optional `⁻` and exactly one digit.
fn decode_superscript(run: &[char]) -> Result<i32, CalcError> {
    let (negative, digits) = match run.split_first() {
        Some(('⁻', rest)) => (true, rest),
        _ => (false, run),
    };
    match digits {
        [d] => match superscript_digit(*d) {
            Some(v) => Ok(if negative { -v } else { v }),
            None => Err(syntax("a superscript needs a digit")),
        },
        _ => Err(syntax("a superscript exponent must be a single digit (use `^` for others)")),
    }
}

/// Source ranges of identifier tokens, excluding quoted text and keywords.
pub fn identifier_ranges(source: &str) -> Vec<std::ops::Range<usize>> {
    lex(source).unwrap_or_default().into_iter().filter_map(|token| matches!(token.kind, Kind::Name(_)).then_some(token.start..token.end)).collect()
}

fn lex(src: &str) -> Result<Vec<Tok>, CalcError> {
    let mut toks: Vec<Tok> = Vec::new();
    let mut chars = src.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        if c.is_whitespace() {
            continue;
        }
        if toks.len() >= MAX_TOKENS {
            return Err(CalcError::TooComplex);
        }
        let mut end = start + c.len_utf8();
        let kind = match c {
            '+' => Kind::Plus,
            '-' | '\u{2212}' | '\u{2013}' => Kind::Minus,
            '*' | '·' | '×' | '\u{22C5}' => Kind::Star,
            '/' | '÷' => Kind::Slash,
            '^' => Kind::Caret,
            '|' => Kind::Bar,
            '(' => Kind::LParen,
            ')' => Kind::RParen,
            '[' => Kind::LBracket,
            ']' => Kind::RBracket,
            ',' => Kind::Comma,
            ';' => Kind::Semi,
            '√' => Kind::Sqrt,
            // The percent unit: a one-character name, so `5 %` is an implied product like `5 m`.
            '%' => Kind::Name("%".to_string()),
            '≔' => Kind::Define,
            '≤' => Kind::Le,
            '≥' => Kind::Ge,
            '≠' => Kind::Ne,
            '∧' => Kind::And,
            '∨' => Kind::Or,
            '¬' => Kind::Not,
            '∑' => Kind::Sum,
            '<' => two_char(&mut chars, &mut end, Kind::Lt, Kind::Le),
            '>' => two_char(&mut chars, &mut end, Kind::Gt, Kind::Ge),
            '!' => match chars.next_if(|&(_, n)| n == '=') {
                Some((i, n)) => {
                    end = i + n.len_utf8();
                    Kind::Ne
                }
                None => return Err(syntax("unexpected character `!` (did you mean `!=`?)")),
            },
            '"' | '“' => {
                let (text, text_end) = lex_text(src, start, c, &mut chars)?;
                end = text_end;
                Kind::Text(text)
            }
            ':' => match chars.next_if(|&(_, n)| n == '=') {
                Some((i, n)) => {
                    end = i + n.len_utf8();
                    Kind::Define
                }
                None => return Err(syntax("unexpected character `:` (did you mean `:=`?)")),
            },
            c if superscript_digit(c).is_some() || c == '⁻' => {
                let mut run = vec![c];
                while let Some((i, n)) = chars.next_if(|&(_, n)| superscript_digit(n).is_some() || n == '⁻') {
                    end = i + n.len_utf8();
                    run.push(n);
                }
                Kind::Sup(decode_superscript(&run)?)
            }
            '=' => {
                // `==` is a comparison; a lone `=` ends the expression, and what follows is the unit,
                // except the one right after `∑(name`, which stays in the sum.
                if chars.next_if(|&(_, n)| n == '=').is_none() {
                    if opens_a_sum(&toks) {
                        toks.push(Tok { kind: Kind::SumEq, start, end });
                        continue;
                    }
                    toks.push(Tok { kind: Kind::Eq, start, end });
                    return Ok(toks);
                }
                end += 1;
                Kind::EqEq
            }
            '.' if chars.peek().is_some_and(|&(_, n)| n == '.') => {
                chars.next();
                end += 1;
                if chars.peek().is_some_and(|&(_, n)| n == '.') {
                    return Err(syntax("`...` isn't valid; a range is written like 0..5"));
                }
                Kind::DotDot
            }
            c if c.is_ascii_digit() || (c == '.' && chars.peek().is_some_and(|&(_, n)| n.is_ascii_digit())) => {
                end = lex_number(src, start, &mut chars);
                let text = src.get(start..end).unwrap_or_default();
                // A second `.` is an error, unless it starts a range's `..` (`1.5..3`).
                if chars.peek().is_some_and(|&(_, n)| n == '.') && !starts_range(src, end) {
                    return Err(syntax(format!("`{text}.` isn't a valid number")));
                }
                match text.parse::<f64>() {
                    Ok(v) if v.is_finite() => Kind::Num(v),
                    _ => return Err(syntax(format!("`{text}` isn't a valid number"))),
                }
            }
            c if is_name_start(c) => {
                while let Some((i, n)) = chars.next_if(|&(_, n)| is_name_continue(n)) {
                    end = i + n.len_utf8();
                }
                let name = src.get(start..end).unwrap_or_default();
                keyword(name).unwrap_or_else(|| Kind::Name(name.to_string()))
            }
            other => return Err(syntax(format!("unexpected character `{}`", other.escape_debug()))),
        };
        toks.push(Tok { kind, start, end });
    }
    Ok(toks)
}

/// Whether the tokens so far end in `∑(` and a name, so that a `=` after them is the sum's own.
fn opens_a_sum(toks: &[Tok]) -> bool {
    matches!(
        toks.len().checked_sub(3).and_then(|k| toks.get(k..)),
        Some([Tok { kind: Kind::Sum, .. }, Tok { kind: Kind::LParen, .. }, Tok { kind: Kind::Name(_), .. }])
    )
}

type Chars<'a> = std::iter::Peekable<std::str::CharIndices<'a>>;

/// `alone`, or `with_eq` when a `=` follows (`<` or `<=`); `end` moves past the `=`.
fn two_char(chars: &mut Chars<'_>, end: &mut usize, alone: Kind, with_eq: Kind) -> Kind {
    match chars.next_if(|&(_, n)| n == '=') {
        Some((i, n)) => {
            *end = i + n.len_utf8();
            with_eq
        }
        None => alone,
    }
}

/// The text literal that starts at `start` with the opening quote `open`: its content, and the
/// offset after its closing quote. The content may not hold a quote, so the first `"` or `”` closes
/// it; a `“` inside, or no closing quote, is an unclosed text. At most [`MAX_TEXT_CHARS`]
/// characters, counted as they are read.
fn lex_text(src: &str, start: usize, open: char, chars: &mut Chars<'_>) -> Result<(String, usize), CalcError> {
    let from = start + open.len_utf8();
    let mut count = 0usize;
    for (i, c) in chars.by_ref() {
        match c {
            '"' | '”' => return Ok((src.get(from..i).unwrap_or_default().to_string(), i + c.len_utf8())),
            '“' => break,
            _ => {
                count += 1;
                if count > MAX_TEXT_CHARS {
                    return Err(syntax("a text is at most 1,000 characters"));
                }
            }
        }
    }
    Err(syntax("a text needs a closing quote, like \"OK\""))
}

/// Whether the source at `at` is a range's `..`.
fn starts_range(src: &str, at: usize) -> bool {
    src.get(at..).is_some_and(|rest| rest.starts_with(".."))
}

/// Consume ASCII digits, advancing `end` past them.
fn eat_digits(chars: &mut Chars<'_>, end: &mut usize) {
    while let Some((i, d)) = chars.next_if(|&(_, n)| n.is_ascii_digit()) {
        *end = i + d.len_utf8();
    }
}

/// Consume the rest of a number that starts at `start` (a digit, or a `.` before a digit) and
/// return its end offset: digits, an optional fraction, an optional exponent. An exponent marker
/// only counts when digits follow it, so `2eV` is `2` then the name `eV`. A number stops before a
/// range's `..`, so `0..5` is `0`, `..`, `5` and never `0.` and `.5`.
fn lex_number(src: &str, start: usize, chars: &mut Chars<'_>) -> usize {
    let mut end = start + 1;
    eat_digits(chars, &mut end);
    if !starts_range(src, end) && chars.next_if(|&(_, n)| n == '.').is_some() {
        end += 1;
        eat_digits(chars, &mut end);
    }
    // Look ahead in the source for a complete exponent before consuming any of it.
    let mut look = src.get(end..).unwrap_or_default().chars();
    if matches!(look.next(), Some('e' | 'E')) {
        let digit_follows = match look.next() {
            Some('+' | '-') => look.next().is_some_and(|d| d.is_ascii_digit()),
            Some(d) => d.is_ascii_digit(),
            None => false,
        };
        if digit_follows {
            chars.next(); // the `e`
            end += 1;
            if let Some((i, sign)) = chars.next_if(|&(_, n)| n == '+' || n == '-') {
                end = i + sign.len_utf8();
            }
            eat_digits(chars, &mut end);
        }
    }
    end
}

// ---------------------------------------------------------------------------------------------
// Statement head
// ---------------------------------------------------------------------------------------------

/// The names of the head of a multiple definition, `[a; b; …]`: two or more names, each once,
/// separated by `;` and closed by `]`, with nothing after it.
fn many_head(src: &str, head: &[Tok]) -> Result<Vec<String>, CalcError> {
    let bad = || syntax("a definition of several names is written like [a; b] := Find(x, y)");
    let mut names: Vec<String> = Vec::new();
    let mut it = head.iter().skip(1);
    loop {
        match it.next() {
            Some(Tok { kind: Kind::Name(n), .. }) => {
                if names.contains(n) {
                    return Err(syntax(format!("`{n}` is repeated")));
                }
                names.push(n.clone());
            }
            Some(Tok { kind: Kind::And | Kind::Or | Kind::Not | Kind::If | Kind::Otherwise, start, end }) => {
                return Err(src.get(*start..*end).filter(|w| is_keyword(w)).map_or_else(bad, |w| CalcError::Keyword(w.to_string())));
            }
            _ => return Err(bad()),
        }
        match it.next() {
            Some(Tok { kind: Kind::Semi, .. }) => {}
            Some(Tok { kind: Kind::RBracket, .. }) if it.next().is_none() && names.len() >= 2 => return Ok(names),
            _ => return Err(bad()),
        }
    }
}

/// The tokens before `:=` must be `name`, `name(param, ...)` or `name[i]`: the name, the range
/// variable of an indexed definition, and the parameters. A keyword in any of those places is a
/// [`CalcError::Keyword`]. `src` is the text the tokens came from.
fn define_head(src: &str, head: &[Tok]) -> Result<(String, Option<String>, Vec<String>), CalcError> {
    let bad = || syntax("`:=` must follow a name, or a function like f(x, y)");
    // A keyword written as a word (`or`), where a name belongs.
    let reserved = |t: &Tok| match t.kind {
        Kind::And | Kind::Or | Kind::Not | Kind::If | Kind::Otherwise => {
            src.get(t.start..t.end).filter(|w| is_keyword(w)).map(|w| CalcError::Keyword(w.to_string()))
        }
        _ => None,
    };
    let mut it = head.iter();
    let name_tok = it.next();
    if let Some(e) = name_tok.and_then(reserved) {
        return Err(e);
    }
    let Some(Tok { kind: Kind::Name(name), end: name_end, .. }) = name_tok else {
        return Err(bad());
    };
    let mut params: Vec<String> = Vec::new();
    match it.next() {
        None => return Ok((name.clone(), None, params)),
        Some(Tok { kind: Kind::LParen, .. }) => {}
        Some(Tok { kind: Kind::LBracket, start, .. }) if start == name_end => {
            return match (it.next(), it.next(), it.next()) {
                (Some(Tok { kind: Kind::Name(i), .. }), Some(Tok { kind: Kind::RBracket, .. }), None) => Ok((name.clone(), Some(i.clone()), params)),
                (Some(t), Some(Tok { kind: Kind::RBracket, .. }), None) if reserved(t).is_some() => Err(reserved(t).unwrap_or_else(bad)),
                _ => Err(syntax("an indexed definition takes one range variable, like t[i] := 2·D[i]")),
            };
        }
        Some(_) => return Err(bad()),
    }
    loop {
        match it.next() {
            Some(Tok { kind: Kind::Name(p), .. }) => {
                if params.contains(p) {
                    return Err(syntax(format!("parameter `{p}` is repeated")));
                }
                params.push(p.clone());
            }
            Some(t) if reserved(t).is_some() => return Err(reserved(t).unwrap_or_else(bad)),
            _ => return Err(syntax("a function's parameters must be names, like f(x, y)")),
        }
        match it.next() {
            Some(Tok { kind: Kind::Comma, .. }) => {}
            Some(Tok { kind: Kind::RParen, .. }) => break,
            _ => return Err(syntax("a function's parameters must be names separated by commas")),
        }
    }
    match it.next() {
        None => Ok((name.clone(), None, params)),
        Some(_) => Err(bad()),
    }
}

// ---------------------------------------------------------------------------------------------
// Expression parser
// ---------------------------------------------------------------------------------------------

/// An expression with the height of its tree.
struct Node {
    expr: Expr,
    height: usize,
}

impl Node {
    fn leaf(expr: Expr) -> Self {
        Node { expr, height: 1 }
    }

    /// A node over `children`, rejecting a tree taller than [`MAX_HEIGHT`].
    fn over(children_height: usize, expr: Expr) -> Result<Self, CalcError> {
        let height = children_height + 1;
        if height > MAX_HEIGHT { Err(CalcError::TooComplex) } else { Ok(Node { expr, height }) }
    }
}

struct Parser<'a> {
    toks: &'a [Tok],
    pos: usize,
    /// Current recursion depth of [`Parser::expr`].
    depth: usize,
    /// Whether a `|` ends the current absolute value instead of starting a new factor.
    bar_closes: bool,
    /// Whether a `..` ends the current expression, as it ends a value of a range at the top of
    /// a definition's right side; anywhere else it is an error.
    in_range_end: bool,
    /// Whether an `if` or `otherwise` ends the current expression, as they end the parts of a
    /// list of cases at the top of a definition's right side; anywhere else they are an error.
    in_cases: bool,
}

impl<'a> Parser<'a> {
    fn new(toks: &'a [Tok]) -> Self {
        Parser { toks, pos: 0, depth: 0, bar_closes: false, in_range_end: false, in_cases: false }
    }

    fn peek(&self) -> Option<&'a Tok> {
        self.toks.get(self.pos)
    }

    fn peek_kind(&self) -> Option<&'a Kind> {
        self.peek().map(|t| &t.kind)
    }

    fn bump(&mut self) -> Option<&'a Tok> {
        let t = self.toks.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    /// After the expression: the end of the statement, or `=` with the unit text after it.
    fn finish(&mut self, src: &str) -> Result<Option<Show>, CalcError> {
        match self.bump() {
            None => Ok(None),
            Some(Tok { kind: Kind::Eq, end, .. }) => {
                let rest = src.get(*end..).unwrap_or_default().trim();
                let unit = rest.strip_prefix('?').unwrap_or(rest).trim();
                Ok(Some(Show { unit: if unit.is_empty() { None } else { Some(unit.to_string()) } }))
            }
            Some(Tok { kind: Kind::If | Kind::Otherwise, .. }) => Err(misplaced_cases()),
            Some(t) => Err(syntax(format!("unexpected {}", t.kind.describe()))),
        }
    }

    fn expect(&mut self, want: &Kind) -> Result<(), CalcError> {
        match self.bump() {
            Some(t) if t.kind == *want => Ok(()),
            Some(t) => Err(syntax(format!("expected {} but found {}", want.describe(), t.kind.describe()))),
            None => Err(syntax(format!("expected {}", want.describe()))),
        }
    }

    /// Parse the whole expression of a statement. It isn't counted as nesting.
    fn root(&mut self) -> Result<Node, CalcError> {
        self.expr_inner(0)
    }

    /// Parse the right side of a definition: a range, a list of cases, or else an ordinary
    /// expression. `..`, `if` and `otherwise` are the loosest and are only accepted here, at the
    /// top: each value of a range, and each value and condition of a case, is a whole expression
    /// (`0..n-1`, `1, 1+0.5..n`, `a + 1 if 0 ≤ x < 1 and y, b otherwise`).
    fn body(&mut self) -> Result<Node, CalcError> {
        let outer = std::mem::replace(&mut self.in_cases, true);
        let r = self.body_inner();
        self.in_cases = outer;
        r
    }

    fn body_inner(&mut self) -> Result<Node, CalcError> {
        let first = self.range_value()?;
        let second = match self.peek_kind() {
            Some(Kind::If) => return self.cases(first),
            Some(Kind::Otherwise) => return Err(no_case_before_otherwise()),
            Some(Kind::Comma) => {
                self.pos += 1;
                let second = self.range_value()?;
                if self.peek_kind() != Some(&Kind::DotDot) {
                    return Err(syntax("a list needs brackets, like [1, 2]; a range with a step is written like 1, 1.5..3"));
                }
                Some(second)
            }
            Some(Kind::DotDot) => None,
            _ => return Ok(first),
        };
        self.pos += 1; // the `..`
        self.range_rhs(first, second)
    }

    /// The rest of a list of cases after its first value, at its first `if`: each condition, then
    /// after a `,` the next value and its `if` or `otherwise`. The cases end at the end of the
    /// statement or its `=`, or after `otherwise`. Each part is a whole expression, read in a loop,
    /// so many cases cost no recursion.
    fn cases(&mut self, first: Node) -> Result<Node, CalcError> {
        let mut height = first.height;
        let mut value = first.expr;
        let mut cases = Vec::new();
        loop {
            match self.bump().map(|t| &t.kind) {
                Some(Kind::If) => {}
                Some(Kind::Otherwise) => {
                    let otherwise = Some(Box::new(value));
                    return Node::over(height, Expr::Cases { cases, otherwise });
                }
                _ => return Err(no_case_before_otherwise()),
            }
            let condition = self.case_part()?;
            height = height.max(condition.height);
            cases.push((value, condition.expr));
            if self.peek_kind() != Some(&Kind::Comma) {
                return Node::over(height, Expr::Cases { cases, otherwise: None });
            }
            self.pos += 1; // the `,`
            let next = self.case_part()?;
            height = height.max(next.height);
            value = next.expr;
            if !matches!(self.peek_kind(), Some(Kind::If | Kind::Otherwise)) {
                return Err(no_case_before_otherwise());
            }
        }
    }

    /// A value or condition of a case: a whole expression (not counted as nesting) that a `,`, an
    /// `if` or an `otherwise` at its top level ends. A `..` there is an error.
    fn case_part(&mut self) -> Result<Node, CalcError> {
        let outer = std::mem::replace(&mut self.in_range_end, false);
        let r = self.expr_inner(0);
        self.in_range_end = outer;
        r
    }

    /// The rest of a range after its `..`: the last value, then the end of the statement or its
    /// `=`, since a range is the whole right side.
    fn range_rhs(&mut self, first: Node, second: Option<Node>) -> Result<Node, CalcError> {
        let last = self.range_value()?;
        if self.peek_kind() == Some(&Kind::DotDot) {
            return Err(misplaced_range());
        }
        let height = first.height.max(last.height).max(second.as_ref().map_or(0, |s| s.height));
        let range = Expr::Range { first: Box::new(first.expr), second: second.map(|s| Box::new(s.expr)), last: Box::new(last.expr) };
        Node::over(height, range)
    }

    /// One value of a range: a whole expression ended by a `..` at its top level (not inside a
    /// bracket). Like the whole expression of a statement, it isn't counted as nesting; with no
    /// `..` after it, it is that expression.
    fn range_value(&mut self) -> Result<Node, CalcError> {
        let outer = std::mem::replace(&mut self.in_range_end, true);
        let r = self.expr_inner(0);
        self.in_range_end = outer;
        r
    }

    /// Run `f` inside a bracket, where a `|` ends an absolute value only when `bar_closes`, and a
    /// `..` never ends a range's value nor an `if` or `otherwise` a case's part.
    fn bracketed<T>(&mut self, bar_closes: bool, f: impl FnOnce(&mut Self) -> Result<T, CalcError>) -> Result<T, CalcError> {
        let outer_bar = std::mem::replace(&mut self.bar_closes, bar_closes);
        let outer_range = std::mem::replace(&mut self.in_range_end, false);
        let outer_cases = std::mem::replace(&mut self.in_cases, false);
        let r = f(self);
        self.bar_closes = outer_bar;
        self.in_range_end = outer_range;
        self.in_cases = outer_cases;
        r
    }

    /// Parse an expression whose operators all bind at least as tightly as `min_bp`.
    fn expr(&mut self, min_bp: u8) -> Result<Node, CalcError> {
        self.depth += 1;
        let r = if self.depth > MAX_DEPTH { Err(CalcError::TooComplex) } else { self.expr_inner(min_bp) };
        self.depth -= 1;
        r
    }

    fn expr_inner(&mut self, min_bp: u8) -> Result<Node, CalcError> {
        let mut lhs = self.prefix()?;
        loop {
            if let Some(&Tok { kind: Kind::Sup(exp), start, .. }) = self.peek() {
                // A superscript applies to the factor just before it, like `^`, and must touch it.
                let touches = self.pos.checked_sub(1).and_then(|i| self.toks.get(i)).is_some_and(|prev| prev.end == start);
                if !touches || POW_BP.0 < min_bp {
                    break;
                }
                self.pos += 1;
                let magnitude = Expr::Num(f64::from(exp.unsigned_abs()));
                let exponent = if exp < 0 { Node::over(1, Expr::Unary(Op::Neg, Box::new(magnitude)))? } else { Node::leaf(magnitude) };
                lhs = Node::over(lhs.height.max(exponent.height), Expr::Binary(Op::Pow, Box::new(lhs.expr), Box::new(exponent.expr)))?;
                continue;
            }
            if let Some(first) = comparison(self.peek_kind()) {
                if CMP_BP.0 < min_bp {
                    break;
                }
                lhs = self.compare(lhs, first)?;
                continue;
            }
            // `if(` touching its parenthesis is the function, a factor like `sqrt(`.
            if matches!(self.peek_kind(), Some(Kind::If | Kind::Otherwise)) && !self.if_call_next() {
                if self.in_cases {
                    break;
                }
                return Err(misplaced_cases());
            }
            let logic = match self.peek_kind() {
                Some(Kind::And) => Some((Logic::And, AND_BP)),
                Some(Kind::Or) => Some((Logic::Or, OR_BP)),
                _ => None,
            };
            if let Some((logic, (left_bp, right_bp))) = logic {
                if left_bp < min_bp {
                    break;
                }
                self.pos += 1;
                let rhs = self.expr(right_bp)?;
                lhs = Node::over(lhs.height.max(rhs.height), Expr::Logic(logic, Box::new(lhs.expr), Box::new(rhs.expr)))?;
                continue;
            }
            let (op, (left_bp, right_bp), implicit) = match self.peek_kind() {
                Some(Kind::Plus) => (Op::Add, ADD_BP, false),
                Some(Kind::Minus) => (Op::Sub, ADD_BP, false),
                Some(Kind::Star) => (Op::Mul, MUL_BP, false),
                Some(Kind::Slash) => (Op::Div, MUL_BP, false),
                Some(Kind::Caret) => (Op::Pow, POW_BP, false),
                Some(Kind::DotDot) if !self.in_range_end => return Err(misplaced_range()),
                _ if self.starts_factor() => (Op::Mul, MUL_BP, true),
                _ => break,
            };
            if left_bp < min_bp {
                break;
            }
            if implicit {
                self.check_adjacent()?;
            } else {
                self.pos += 1;
            }
            let rhs = self.expr(right_bp)?;
            lhs = Node::over(lhs.height.max(rhs.height), Expr::Binary(op, Box::new(lhs.expr), Box::new(rhs.expr)))?;
        }
        Ok(lhs)
    }

    /// A comparison chain that starts with `first` (the next token) after the operand `lhs`:
    /// `a < b ≤ c` is one node, so the whole chain is read iteratively and its height is one more
    /// than its tallest operand's.
    fn compare(&mut self, lhs: Node, first: Cmp) -> Result<Node, CalcError> {
        let mut height = lhs.height;
        let mut rest = Vec::new();
        let mut cmp = first;
        loop {
            self.pos += 1; // the comparison
            let rhs = self.expr(CMP_BP.1)?;
            height = height.max(rhs.height);
            rest.push((cmp, rhs.expr));
            match comparison(self.peek_kind()) {
                Some(next) => cmp = next,
                None => break,
            }
        }
        Node::over(height, Expr::Compare { first: Box::new(lhs.expr), rest })
    }

    /// The token just consumed.
    fn prev(&self) -> Option<&'a Tok> {
        self.pos.checked_sub(1).and_then(|i| self.toks.get(i))
    }

    /// Whether the token just consumed ends a value: a number, text, name, closing bracket or bar, or
    /// a superscript.
    fn after_value(&self) -> bool {
        self.prev().is_some_and(|t| {
            matches!(t.kind, Kind::Num(_) | Kind::Text(_) | Kind::Name(_) | Kind::RParen | Kind::RBracket | Kind::Bar | Kind::Sup(_))
        })
    }

    /// Whether the next token starts a factor, which after a value is an implied product. A text
    /// never does, so `2 "OK"` ends the expression and the text is reported as unexpected.
    fn starts_factor(&self) -> bool {
        match self.peek_kind() {
            Some(Kind::Num(_) | Kind::Name(_) | Kind::LParen | Kind::LBracket | Kind::Sqrt | Kind::Sum) => true,
            Some(Kind::Bar) => !self.bar_closes,
            Some(Kind::If) => self.if_call_next(),
            _ => false,
        }
    }

    /// Whether the next tokens are `if(`, the `(` touching the `if`: the function `if(c, a, b)`.
    fn if_call_next(&self) -> bool {
        match (self.toks.get(self.pos), self.toks.get(self.pos.saturating_add(1))) {
            (Some(Tok { kind: Kind::If, end, .. }), Some(Tok { kind: Kind::LParen, start, .. })) => end == start,
            _ => false,
        }
    }

    /// Before the next factor of an implied product: a number straight after a value is refused,
    /// and so is a `[` touching a closing bracket (`f(x)[2]`, `D[1][2]`), which looks like an
    /// index of something that isn't a name. A `[` touching a name was taken as an index already;
    /// after anything else it starts a literal factor, as `(` does (`2[1, 2]`).
    fn check_adjacent(&self) -> Result<(), CalcError> {
        match self.peek() {
            Some(Tok { kind: Kind::Num(_), .. }) if self.after_value() => Err(syntax("two values in a row \u{2014} did you mean `*`?")),
            Some(Tok { kind: Kind::LBracket, start, .. })
                if self.prev().is_some_and(|p| p.end == *start && matches!(p.kind, Kind::RParen | Kind::RBracket)) =>
            {
                Err(syntax("only a name can be indexed, like D[2]"))
            }
            _ => Ok(()),
        }
    }

    fn prefix(&mut self) -> Result<Node, CalcError> {
        let Some(tok) = self.bump() else {
            return Err(syntax("the expression ends too soon"));
        };
        match &tok.kind {
            Kind::Num(v) => Ok(Node::leaf(Expr::Num(*v))),
            Kind::Text(s) => Ok(Node::leaf(Expr::Text(s.clone()))),
            Kind::Name(n) => {
                // `f(` with no space is a call and `D[` an index; `f (` is `f` times a
                // parenthesised factor and `D [` is `D` times a literal.
                match self.peek() {
                    Some(next) if next.kind == Kind::LParen && next.start == tok.end => {
                        self.pos += 1;
                        self.call(n.clone())
                    }
                    Some(next) if next.kind == Kind::LBracket && next.start == tok.end => {
                        self.pos += 1;
                        self.index(n.clone())
                    }
                    _ => Ok(Node::leaf(Expr::Name(n.clone()))),
                }
            }
            Kind::LParen => {
                let inner = self.bracketed(false, |p| p.expr(0))?;
                self.expect(&Kind::RParen)?;
                Ok(inner)
            }
            Kind::LBracket => self.bracketed(false, Self::literal),
            Kind::Bar => {
                let inner = self.bracketed(true, |p| p.expr(0))?;
                self.expect(&Kind::Bar)?;
                Node::over(inner.height, Expr::Abs(Box::new(inner.expr)))
            }
            Kind::Minus => {
                let operand = self.expr(NEG_BP)?;
                Node::over(operand.height, Expr::Unary(Op::Neg, Box::new(operand.expr)))
            }
            Kind::Plus => self.expr(NEG_BP),
            Kind::Not => {
                let operand = self.expr(NOT_BP)?;
                Node::over(operand.height, Expr::Not(Box::new(operand.expr)))
            }
            Kind::Sqrt => {
                let operand = self.expr(SQRT_BP)?;
                Node::over(operand.height, Expr::Call("sqrt".to_string(), vec![operand.expr]))
            }
            // `if(` with no space is the function `if(c, a, b)`; any other `if` or `otherwise`
            // here starts nothing.
            Kind::If => match self.peek() {
                Some(next) if next.kind == Kind::LParen && next.start == tok.end => {
                    self.pos += 1;
                    self.call("if".to_string())
                }
                _ => Err(misplaced_cases()),
            },
            Kind::Otherwise => Err(misplaced_cases()),
            Kind::Sum => self.sum(),
            other => Err(syntax(format!("expected a value but found {}", other.describe()))),
        }
    }

    /// A sum, after its `∑`: `(i = first..last)` or `(i = first, second..last)`, then the term it
    /// adds up. The term is one product term, like `·` takes (`∑(…) a·b + 1` is `(∑ a·b) + 1`):
    /// a `+`, `-`, comparison, `and` or `or` ends it.
    fn sum(&mut self) -> Result<Node, CalcError> {
        let bad = || syntax("a sum is written like ∑(i = 1..n) a[i]·b[i]");
        if self.bump().map(|t| &t.kind) != Some(&Kind::LParen) {
            return Err(bad());
        }
        let index = match self.bump().map(|t| &t.kind) {
            Some(Kind::Name(index)) => index,
            // A keyword (`and`, or its symbol `∧`) isn't a name, as anywhere a name is defined.
            Some(Kind::And) => return Err(CalcError::Keyword("and".to_string())),
            Some(Kind::Or) => return Err(CalcError::Keyword("or".to_string())),
            Some(Kind::Not) => return Err(CalcError::Keyword("not".to_string())),
            Some(Kind::If) => return Err(CalcError::Keyword("if".to_string())),
            Some(Kind::Otherwise) => return Err(CalcError::Keyword("otherwise".to_string())),
            _ => return Err(syntax("a sum's index must be a name, like ∑(i = 1..n) a[i]")),
        };
        if self.bump().map(|t| &t.kind) != Some(&Kind::SumEq) {
            return Err(bad());
        }
        let (first, second, last) = self.bracketed(false, Self::sum_range)?;
        self.expect(&Kind::RParen)?;
        let body = self.expr(MUL_BP.0)?;
        let height = first.height.max(last.height).max(second.as_ref().map_or(0, |s| s.height)).max(body.height);
        let sum = Expr::Sum {
            index: index.clone(),
            first: Box::new(first.expr),
            second: second.map(|s| Box::new(s.expr)),
            last: Box::new(last.expr),
            body: Box::new(body.expr),
        };
        Node::over(height, sum)
    }

    /// The range of a sum, inside its parentheses: `first..last` or `first, second..last`. The
    /// values are whole expressions read as a range's are, and the whole counts as one level of
    /// nesting, so sums inside the ends of sums are bounded.
    fn sum_range(&mut self) -> Result<(Node, Option<Node>, Node), CalcError> {
        self.depth += 1;
        let r = if self.depth > MAX_DEPTH { Err(CalcError::TooComplex) } else { self.sum_range_inner() };
        self.depth -= 1;
        r
    }

    fn sum_range_inner(&mut self) -> Result<(Node, Option<Node>, Node), CalcError> {
        let bad = || syntax("a sum's range is written like 1..n, or like 1, 3..n with a step");
        let first = self.range_value()?;
        let second = if self.peek_kind() == Some(&Kind::Comma) {
            self.pos += 1;
            Some(self.range_value()?)
        } else {
            None
        };
        if self.bump().map(|t| &t.kind) != Some(&Kind::DotDot) {
            return Err(bad());
        }
        let last = self.range_value()?;
        Ok((first, second, last))
    }

    /// The arguments of a call, after its `(`.
    fn call(&mut self, name: String) -> Result<Node, CalcError> {
        let (args, height) = self.bracketed(false, |p| p.list(&Kind::RParen))?;
        Node::over(height, Expr::Call(name, args))
    }

    /// The indexes of `name[…]`, after its `[`: one, or a row and a column.
    fn index(&mut self, name: String) -> Result<Node, CalcError> {
        let (at, height) = self.bracketed(false, |p| p.list(&Kind::RBracket))?;
        if !(1..=2).contains(&at.len()) {
            return Err(syntax("an index is one number, or a row and a column, like D[2] or M[1, 0]"));
        }
        Node::over(height, Expr::Index(name, at))
    }

    /// Expressions separated by commas up to `close`, which is consumed, and their tallest height.
    fn list(&mut self, close: &Kind) -> Result<(Vec<Expr>, usize), CalcError> {
        let mut items = Vec::new();
        let mut height = 0;
        if self.peek_kind() == Some(close) {
            self.pos += 1;
            return Ok((items, height));
        }
        loop {
            let item = self.expr(0)?;
            height = height.max(item.height);
            items.push(item.expr);
            match self.bump() {
                Some(Tok { kind: Kind::Comma, .. }) => {}
                Some(t) if t.kind == *close => return Ok((items, height)),
                Some(t) => return Err(syntax(format!("expected `,` or {} but found {}", close.describe(), t.kind.describe()))),
                None => return Err(syntax(format!("expected {}", close.describe()))),
            }
        }
    }

    /// A vector or matrix literal, after its `[`: elements separated by `,`, rows by `;`, every
    /// row as long as the first. A single row is a column vector.
    fn literal(&mut self) -> Result<Node, CalcError> {
        if self.peek_kind() == Some(&Kind::RBracket) {
            return Err(syntax("a matrix needs at least one element, like [1, 2]"));
        }
        let mut items = Vec::new();
        let mut height = 0;
        let mut rows = 0;
        let mut cols = 0;
        let mut in_row = 0;
        loop {
            let item = self.expr(0)?;
            if matches!(item.expr, Expr::Matrix { .. }) {
                return Err(syntax("a matrix element can't be another matrix; use `;` to start a new row"));
            }
            height = height.max(item.height);
            items.push(item.expr);
            in_row += 1;
            let closes = match self.bump() {
                Some(Tok { kind: Kind::Comma, .. }) => continue,
                Some(Tok { kind: Kind::Semi, .. }) => false,
                Some(Tok { kind: Kind::RBracket, .. }) => true,
                Some(t) => return Err(syntax(format!("expected `,`, `;` or `]` but found {}", t.kind.describe()))),
                None => return Err(syntax("expected `]`")),
            };
            rows += 1;
            if rows == 1 {
                cols = in_row;
            } else if in_row != cols {
                let noun = if in_row == 1 { "element" } else { "elements" };
                return Err(syntax(format!("row {rows} has {in_row} {noun}; row 1 has {cols}")));
            }
            in_row = 0;
            if closes {
                break;
            }
        }
        let (rows, cols) = if rows == 1 { (cols, 1) } else { (rows, cols) };
        Node::over(height, Expr::Matrix { rows, cols, items })
    }
}
