//! Table cell formulas (ODF 1.3 §19.646 `table:formula`) and Word's formula fields.
//!
//! LibreOffice writes a text table's formulas in its own syntax (prefix `ooow:`): cells and
//! ranges in angle brackets (`<A1>`, `<A1:B3>`), functions written before their operand
//! (`sum <A1:A3>`, a list joined by `|`) and some operators as words (`add`, `mul`…). WordCraft,
//! as Word, holds a formula as a field (`=SUM(A1:A3)`, `ABOVE`, `LEFT`). Formulas are turned one
//! into the other where both can say them; others are left as their value.

use crate::model::{FBlock, Inline};

/// Longest formula turned, in bytes; most tokens.
const MAX_LEN: usize = 4096;
const MAX_TOKENS: usize = 512;

/// The functions both know: LibreOffice's name, Word's.
const FUNCTIONS: [(&str, &str); 8] = [
    ("sum", "SUM"),
    ("mean", "AVERAGE"),
    ("min", "MIN"),
    ("max", "MAX"),
    ("product", "PRODUCT"),
    ("abs", "ABS"),
    ("sign", "SIGN"),
    ("count", "COUNT"),
];

/// Operators written as words: LibreOffice's, Word's.
const OPERATORS: [(&str, &str); 5] = [("add", "+"), ("sub", "-"), ("mul", "*"), ("div", "/"), ("pow", "^")];

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    /// A cell or range, upper case: `A1`, `A1:B3`.
    Cell(String),
    Num(String),
    Word(String),
    Op(char),
}

/// Whether `s` is a cell (`A1`) or a range of cells (`A1:B3`).
fn is_cells(s: &str) -> bool {
    let cell = |c: &str| {
        let letters = c.chars().take_while(char::is_ascii_alphabetic).count();
        letters > 0 && letters <= 3 && c.len() > letters && c.get(letters..).is_some_and(|d| d.chars().all(|ch| ch.is_ascii_digit()))
    };
    match s.split_once(':') {
        Some((a, b)) => cell(a) && cell(b),
        None => cell(s),
    }
}

/// A formula's tokens: angle-bracketed cells (LibreOffice), words, numbers and operators.
fn tokens(s: &str) -> Option<Vec<Tok>> {
    if s.len() > MAX_LEN {
        return None;
    }
    let mut out = Vec::new();
    let mut it = s.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        if out.len() > MAX_TOKENS {
            return None;
        }
        match c {
            c if c.is_whitespace() => {}
            '<' => {
                let mut cell = String::new();
                loop {
                    match it.next() {
                        Some((_, '>')) => break,
                        Some((_, ch)) if cell.len() < 32 => cell.push(ch),
                        _ => return None,
                    }
                }
                let cell = cell.trim().to_ascii_uppercase();
                // A cell of another table (`<Table2.A1>`) can't be said in Word.
                if !is_cells(&cell) {
                    return None;
                }
                out.push(Tok::Cell(cell));
            }
            '0'..='9' | '.' => {
                let mut end = i + c.len_utf8();
                while let Some((j, ch)) = it.peek().copied() {
                    if ch.is_ascii_digit() || ch == '.' {
                        end = j + ch.len_utf8();
                        it.next();
                    } else {
                        break;
                    }
                }
                let n = s.get(i..end)?;
                n.parse::<f64>().ok()?;
                out.push(Tok::Num(n.to_owned()));
            }
            c if c.is_ascii_alphabetic() => {
                let mut end = i + 1;
                while let Some((j, ch)) = it.peek().copied() {
                    if ch.is_ascii_alphanumeric() || ch == ':' {
                        end = j + 1;
                        it.next();
                    } else {
                        break;
                    }
                }
                out.push(Tok::Word(s.get(i..end)?.to_owned()));
            }
            '+' | '-' | '*' | '/' | '^' | '(' | ')' | '|' | ',' => out.push(Tok::Op(c)),
            _ => return None,
        }
    }
    Some(out)
}

/// Whether `toks`' brackets match.
fn balanced(toks: &[Tok]) -> bool {
    let mut depth = 0usize;
    for t in toks {
        match t {
            Tok::Op('(') => depth += 1,
            Tok::Op(')') => match depth.checked_sub(1) {
                Some(d) => depth = d,
                None => return false,
            },
            _ => {}
        }
    }
    depth == 0
}

/// Where the bracket opening `toks` closes.
fn closing(toks: &[Tok]) -> Option<usize> {
    let mut depth = 0usize;
    for (i, t) in toks.iter().enumerate() {
        match t {
            Tok::Op('(') => depth += 1,
            Tok::Op(')') => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Word's formula field code for a LibreOffice cell formula (`ooow:sum <A1:A3>` →
/// `=SUM(A1:A3)`), when Word can say it.
pub(super) fn word_of(formula: &str) -> Option<String> {
    let f = formula.trim();
    let f = f.strip_prefix("ooow:").unwrap_or(f);
    let f = f.strip_prefix('=').unwrap_or(f);
    let mut toks = tokens(f).filter(|t| balanced(t))?;
    // Brackets round the whole formula say nothing (they are written round a function).
    while toks.len() >= 2 && toks.first() == Some(&Tok::Op('(')) && closing(&toks) == Some(toks.len() - 1) {
        toks.pop();
        toks.remove(0);
    }
    let mut out = String::from("=");
    let mut i = 0;
    while let Some(t) = toks.get(i) {
        i += 1;
        match t {
            Tok::Cell(c) => out.push_str(c),
            Tok::Num(n) => out.push_str(n),
            Tok::Op('|' | ',') => return None,
            Tok::Op(c) => out.push(*c),
            Tok::Word(w) => {
                let w = w.to_ascii_lowercase();
                if let Some((_, op)) = OPERATORS.iter().find(|(n, _)| *n == w) {
                    out.push_str(op);
                    continue;
                }
                let (_, name) = FUNCTIONS.iter().find(|(n, _)| *n == w)?;
                // Its operand: a cell, number or bracketed expression, or a list of them.
                out.push_str(name);
                out.push('(');
                loop {
                    match toks.get(i)? {
                        Tok::Cell(c) => out.push_str(c),
                        Tok::Num(n) => out.push_str(n),
                        Tok::Op('(') => {
                            // A bracketed expression, its brackets matched.
                            let mut depth = 0usize;
                            let start = i;
                            loop {
                                match toks.get(i)? {
                                    Tok::Op('(') => depth += 1,
                                    Tok::Op(')') => {
                                        depth -= 1;
                                        if depth == 0 {
                                            break;
                                        }
                                    }
                                    _ => {}
                                }
                                i += 1;
                            }
                            let inner: Vec<Tok> = toks.get(start + 1..i)?.to_vec();
                            let inner = word_of_tokens(&inner)?;
                            out.push_str(&inner);
                        }
                        _ => return None,
                    }
                    i += 1;
                    if toks.get(i) == Some(&Tok::Op('|')) {
                        out.push(',');
                        i += 1;
                    } else {
                        break;
                    }
                }
                out.push(')');
            }
        }
    }
    (out.len() > 1).then_some(out)
}

/// Word's form of LibreOffice tokens (an expression in brackets), without the leading `=`.
fn word_of_tokens(toks: &[Tok]) -> Option<String> {
    let text: Vec<String> = toks
        .iter()
        .map(|t| match t {
            Tok::Cell(c) => format!("<{c}>"),
            Tok::Num(n) | Tok::Word(n) => n.clone(),
            Tok::Op(c) => c.to_string(),
        })
        .collect();
    word_of(&text.join(" ")).map(|w| w.trim_start_matches('=').to_owned())
}

/// The column letters of column `c` (from 0): A…Z, AA…
fn column(c: usize) -> String {
    let mut c = c + 1;
    let mut s = Vec::new();
    while c > 0 && s.len() < 4 {
        let d = (c - 1) % 26;
        s.push(b'A' + d as u8);
        c = (c - 1) / 26;
    }
    s.iter().rev().map(|b| char::from(*b)).collect()
}

/// A LibreOffice cell formula for Word's formula field code `instr`, in the cell at row `r`,
/// column `c` (from 0) of a table `rows` × `cols` (`=SUM(ABOVE)` → `ooow:sum <A1:A3>`), when it
/// can say it. Its `\#` picture and `\*` switches are dropped.
pub(super) fn odf_of(instr: &str, (r, c): (usize, usize), (rows, cols): (usize, usize)) -> Option<String> {
    let body = instr.trim().strip_prefix('=')?;
    let body = body.split('\\').next().unwrap_or_default();
    let toks = tokens(body).filter(|t| balanced(t))?;
    let col = column(c);
    // The cells a direction names.
    let dir = |d: &str| -> Option<String> {
        Some(match d {
            "ABOVE" if r > 0 => format!("{col}1:{col}{r}"),
            "BELOW" if r + 1 < rows => format!("{col}{}:{col}{rows}", r + 2),
            "LEFT" if c > 0 => format!("A{0}:{1}{0}", r + 1, column(c - 1)),
            "RIGHT" if c + 1 < cols => format!("{1}{0}:{2}{0}", r + 1, column(c + 1), column(cols - 1)),
            _ => return None,
        })
    };
    let operand = |t: &Tok| -> Option<String> {
        match t {
            Tok::Num(n) => Some(n.clone()),
            Tok::Word(w) => {
                let w = w.to_ascii_uppercase();
                if is_cells(&w) { Some(format!("<{w}>")) } else { dir(&w).map(|d| format!("<{d}>")) }
            }
            _ => None,
        }
    };
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while let Some(t) = toks.get(i) {
        i += 1;
        match t {
            Tok::Op(',' | '|') => return None,
            Tok::Op(c) => parts.push(c.to_string()),
            Tok::Num(_) => parts.push(operand(t)?),
            Tok::Word(w) => {
                let upper = w.to_ascii_uppercase();
                let Some((name, _)) = FUNCTIONS.iter().find(|(_, n)| *n == upper) else {
                    parts.push(operand(t)?);
                    continue;
                };
                // Its arguments: cells, ranges, directions or numbers, joined by `|`.
                if toks.get(i) != Some(&Tok::Op('(')) {
                    return None;
                }
                i += 1;
                let mut args = Vec::new();
                loop {
                    args.push(operand(toks.get(i)?)?);
                    i += 1;
                    match toks.get(i)? {
                        Tok::Op(',') => i += 1,
                        Tok::Op(')') => {
                            i += 1;
                            break;
                        }
                        _ => return None,
                    }
                }
                parts.push(format!("({name} {})", args.join("|")));
            }
            Tok::Cell(_) => return None,
        }
    }
    (!parts.is_empty()).then(|| format!("ooow:{}", parts.join(" ")))
}

/// Turns a cell's content, its value as text, into formula field `instr` showing it, when the
/// cell is one paragraph of text.
pub(super) fn into_field(blocks: &mut [FBlock], instr: String) {
    let [FBlock::Para(p)] = blocks else { return };
    if !p.inlines.iter().all(|i| matches!(i, Inline::Text(..))) {
        return;
    }
    let fmt = p.inlines.iter().find_map(|i| if let Inline::Text(_, f) = i { Some(f.clone()) } else { None }).unwrap_or_default();
    let result: String = p.inlines.iter().filter_map(|i| if let Inline::Text(t, _) = i { Some(t.as_str()) } else { None }).collect();
    p.inlines = vec![Inline::Field { instr, result: result.trim().to_owned(), fmt, locked: false }];
}

/// The formula field a cell is, when it is one: its field code and the result it shows (a cell
/// of one paragraph holding one `=` field, and perhaps spaces).
pub(super) fn of_cell(blocks: &[FBlock]) -> Option<(&str, &str)> {
    let [FBlock::Para(p)] = blocks else { return None };
    let mut field = None;
    for i in &p.inlines {
        match i {
            Inline::Field { instr, result, .. } if field.is_none() && instr.trim_start().starts_with('=') => {
                field = Some((instr.as_str(), result.as_str()))
            }
            Inline::Text(t, _) if t.trim().is_empty() => {}
            _ => return None,
        }
    }
    field
}

/// The number a cell holds, when it is one paragraph of a plain number (digits with perhaps a
/// sign and a decimal point, as LibreOffice reads a number cell's value): what formulas
/// calculate with.
pub(super) fn number_of(blocks: &[FBlock]) -> Option<f64> {
    let [FBlock::Para(p)] = blocks else { return None };
    let mut text = String::new();
    for i in &p.inlines {
        match i {
            Inline::Text(t, _) if text.len() + t.len() <= 64 => text.push_str(t),
            _ => return None,
        }
    }
    let t = text.trim();
    let digits = t.strip_prefix('-').unwrap_or(t);
    let plain = !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit() || c == '.')
        && digits.matches('.').count() <= 1
        && digits.chars().any(|c| c.is_ascii_digit());
    if !plain {
        return None;
    }
    t.parse::<f64>().ok().filter(|v| v.is_finite())
}
