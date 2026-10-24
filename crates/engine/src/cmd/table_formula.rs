//! Table formulas (Table Layout › Data › Formula): `=SUM(ABOVE)`, cell references and ranges
//! (`=SUM(B2:B4)`, `=A1*2`), arithmetic, and the functions Word offers in tables. A formula field
//! is re-evaluated by Update Field (F9), so its result follows the cells it reads.
//!
//! Cell references count rows from 1 at the top and columns from A at the left, by grid column.
//! ABOVE, LEFT, RIGHT and BELOW sum the first run of numbers in that direction, as Word does: the
//! cells next to the formula's cell are skipped while they hold no number (empty or text), then
//! the numbers are summed up to the next cell that holds none. A function needs its arguments:
//! `=SUM()` is a syntax error, as it is in Word.

use wordcraft_doc::table::{MAX_COLS, MAX_ROWS};
use wordcraft_doc::{Cell, Table};

use crate::CmdError;

/// The longest formula accepted, in bytes.
pub const MAX_FORMULA: usize = 512;
/// The longest numeric picture (`\#` switch) used, in characters.
const MAX_PICTURE: usize = 64;
/// The deepest nesting of parentheses, function calls, unary signs and `^` chains.
const MAX_DEPTH: usize = 32;
/// The most cells one evaluation reads; past it the formula is too complex to work out, so a
/// hostile document full of huge ranges can't make Update Field hang.
pub const MAX_CELL_VISITS: usize = 100_000;

/// Evaluate `formula` (with or without its leading `=`) for the cell in row `r`, cell index `c`.
pub fn evaluate(t: &Table, r: usize, c: usize, formula: &str) -> Result<f64, CmdError> {
    evaluate_with(t, None, None, r, c, formula)
}

pub fn evaluate_with(
    t: &Table,
    doc: Option<&wordcraft_doc::Document>,
    default_lang: Option<&str>,
    r: usize,
    c: usize,
    formula: &str,
) -> Result<f64, CmdError> {
    if formula.len() > MAX_FORMULA {
        return Err(syntax(format!("the formula is longer than {MAX_FORMULA} characters")));
    }
    let body = formula.trim();
    let body = body.strip_prefix('=').unwrap_or(body);
    let mut parser = Parser { toks: tokenize(body)?, pos: 0, depth: 0 };
    let expr = parser.compare()?;
    if parser.pos != parser.toks.len() {
        return Err(syntax("unexpected text after the formula".into()));
    }
    let here = Here { t, doc, default_lang, r, c, g: t.grid_col(r, c), visits: std::cell::Cell::new(0) };
    match eval(&expr, &here)? {
        Val::Num(x) => finite(x),
        Val::List(_) => Err(syntax("a range or ABOVE/LEFT/RIGHT/BELOW needs a function, such as SUM".into())),
    }
}

/// The text a formula result shows: a whole number as is, otherwise two decimals.
pub fn format_result(v: f64) -> String {
    wordcraft_doc::locale_number::format_number(v, None)
}

/// The text a formula field shows: its result under its `\#` numeric picture when it has one (see
/// [`format_picture`]), otherwise as [`format_result`] shows it.
pub fn evaluate_field(t: &Table, r: usize, c: usize, instr: &str) -> Result<String, CmdError> {
    evaluate_field_with(t, None, None, r, c, instr)
}

pub fn evaluate_field_with(
    t: &Table,
    doc: Option<&wordcraft_doc::Document>,
    default_lang: Option<&str>,
    r: usize,
    c: usize,
    instr: &str,
) -> Result<String, CmdError> {
    let (formula, switch) = split_switch(instr);
    let value = evaluate_with(t, doc, default_lang, r, c, formula)?;
    Ok(match switch.and_then(picture_of) {
        Some(picture) => format_picture(value, &picture),
        None => {
            let out_lang = doc
                .and_then(|d| {
                    t.rows.get(r).and_then(|row| row.cells.get(c)).and_then(|cl| {
                        cl.blocks.iter().filter_map(|b| b.as_para()).find(|p| !p.plain_text().trim().is_empty()).and_then(|p| super::para_lang(d, p))
                    })
                })
                .or_else(|| {
                    doc.and_then(|d| {
                        t.rows.iter().flat_map(|row| row.cells.iter()).take(100).find_map(|cl| {
                            cl.blocks
                                .iter()
                                .filter_map(|b| b.as_para())
                                .find(|p| !p.plain_text().trim().is_empty())
                                .and_then(|p| super::para_lang(d, p))
                        })
                    })
                })
                .or_else(|| default_lang.map(str::to_string));
            wordcraft_doc::locale_number::format_number(value, out_lang.as_deref())
        }
    })
}

/// Splits a field's `\#` switch off its formula: `=SUM(ABOVE) \# "0.00"` gives the formula and the
/// switch text (from the `\#` on).
pub fn split_switch(instr: &str) -> (&str, Option<&str>) {
    match instr.find("\\#") {
        Some(at) => (instr[..at].trim_end(), Some(&instr[at..])),
        None => (instr, None),
    }
}

/// The picture a `\#` switch names: a quoted one (`"#,##0.00"`) or a single word.
fn picture_of(switch: &str) -> Option<String> {
    let rest = switch.strip_prefix("\\#")?.trim_start();
    let picture = match rest.strip_prefix('"') {
        Some(quoted) => quoted.split('"').next().unwrap_or_default(),
        None => rest.split_whitespace().next().unwrap_or_default(),
    };
    let picture: String = picture.chars().take(MAX_PICTURE).collect();
    (!picture.is_empty()).then_some(picture)
}

/// The text of `value` under a numeric picture, as Word's `\#` pictures read: `0` is a digit that
/// always shows, `#` one that shows when it has a value, `,` groups thousands, `.` marks the
/// decimals, `%` multiplies by 100, and anything else shows as it is. Sections split by `;` give the
/// positive, negative and zero forms; with one section a negative value takes a `-` in front.
pub fn format_picture(value: f64, picture: &str) -> String {
    let sections: Vec<&str> = picture.split(';').collect();
    let (section, x, minus) = if value < 0.0 && sections.len() >= 2 {
        (sections[1], -value, false)
    } else if value == 0.0 && sections.len() >= 3 {
        (sections[2], 0.0, false)
    } else {
        (sections[0], value.abs(), value < 0.0)
    };
    let Some(start) = section.find(['#', '0']) else { return section.to_string() };
    let rest = &section[start..];
    let len = rest.find(|c: char| !matches!(c, '#' | '0' | ',' | '.')).unwrap_or(rest.len());
    let (core, suffix) = rest.split_at(len);
    let prefix = &section[..start];
    let x = if prefix.contains('%') || suffix.contains('%') { x * 100.0 } else { x };
    let (int_pic, frac_pic) = match core.split_once('.') {
        Some((int_pic, frac_pic)) => (int_pic, Some(frac_pic)),
        None => (core, None),
    };
    let min_int = int_pic.matches('0').count().min(MAX_PICTURE);
    let max_frac = frac_pic.map_or(0, |f| f.matches(['0', '#']).count()).min(15);
    let min_frac = frac_pic.map_or(0, |f| f.matches('0').count()).min(max_frac);
    let scale = 10f64.powi(max_frac as i32);
    let rounded = (x * scale).round() / scale;
    if !rounded.is_finite() {
        return format_result(value);
    }
    let digits = format!("{:.*}", max_frac, rounded);
    let (whole, decimals) = match digits.split_once('.') {
        Some((whole, decimals)) => (whole.to_string(), decimals.to_string()),
        None => (digits.clone(), String::new()),
    };
    let whole = if whole == "0" && min_int == 0 { String::new() } else { format!("{whole:0>min_int$}") };
    let whole = if int_pic.contains(',') { group_thousands(&whole) } else { whole };
    let mut decimals = decimals;
    while decimals.len() > min_frac && decimals.ends_with('0') {
        decimals.pop();
    }
    let mut body = whole;
    if !decimals.is_empty() {
        body.push('.');
        body.push_str(&decimals);
    }
    let sign = if minus && rounded != 0.0 { "-" } else { "" };
    format!("{sign}{prefix}{body}{suffix}")
}

/// `1234567` → `1,234,567`.
fn group_thousands(digits: &str) -> String {
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// A cell's number as a formula or a sort key reads it: surrounding spaces, currency signs,
/// thousands separators and `%` are ignored, and accounting negatives in parentheses count as
/// negative. Anything else (letters included) makes the cell text, not a number.
pub fn parse_number(text: &str, lang: Option<&str>) -> Option<f64> {
    let t = text.trim();
    let (t, negative) = match t.strip_prefix('(').and_then(|x| x.strip_suffix(')')) {
        Some(inner) => (inner.trim(), true),
        None => (t, false),
    };
    if let Some(x) = wordcraft_doc::locale_number::parse_number(t, lang) {
        let x = if negative { -x.abs() } else { x };
        return x.is_finite().then_some(x);
    }
    let cleaned: String = t.chars().filter(|c| !matches!(c, ',' | '$' | '€' | '£' | '¥' | '৳' | '%') && !c.is_whitespace()).collect();
    if cleaned.is_empty() || !cleaned.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')) {
        return None;
    }
    let x = cleaned.parse::<f64>().ok()?;
    let x = if negative { -x } else { x };
    x.is_finite().then_some(x)
}

/// The number in a cell, if its text is one.
pub(crate) fn cell_number_in(c: &Cell, lang: Option<&str>) -> Option<f64> {
    parse_number(&super::table::cell_text(c), lang)
}

fn syntax(msg: String) -> CmdError {
    CmdError::Failed(format!("syntax error: {msg}"))
}

fn finite(x: f64) -> Result<f64, CmdError> {
    if x.is_finite() { Ok(x) } else { Err(CmdError::Failed("the result is not a number".into())) }
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    /// A name: a function, a keyword or a cell reference (letters then digits), upper-cased.
    Name(String),
    Op(char),
    /// A comparison: `=`, `<>`, `<`, `<=`, `>` or `>=`.
    Cmp(&'static str),
}

fn parse_num_token(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Result<Tok, CmdError> {
    let mut lit = String::new();
    while let Some(&d) = chars.peek() {
        if d.is_ascii_digit() || d == '.' {
            lit.push(d);
            chars.next();
        } else {
            break;
        }
    }
    let x = lit.parse::<f64>().map_err(|_| syntax(format!("`{lit}` isn't a number")))?;
    Ok(Tok::Num(x))
}

fn parse_name_token(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Tok {
    let mut name = String::new();
    while let Some(&d) = chars.peek() {
        if d.is_ascii_alphanumeric() {
            name.push(d.to_ascii_uppercase());
            chars.next();
        } else {
            break;
        }
    }
    Tok::Name(name)
}

fn parse_cmp_token(ch: char, chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Tok {
    chars.next();
    let next = chars.peek().copied();
    let (op, longer) = match (ch, next) {
        ('<', Some('=')) => ("<=", true),
        ('<', Some('>')) => ("<>", true),
        ('>', Some('=')) => (">=", true),
        ('<', _) => ("<", false),
        ('>', _) => (">", false),
        _ => ("=", false),
    };
    if longer {
        chars.next();
    }
    Tok::Cmp(op)
}

fn tokenize(s: &str) -> Result<Vec<Tok>, CmdError> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(&ch) = chars.peek() {
        if ch.is_whitespace() {
            chars.next();
        } else if ch.is_ascii_digit() || ch == '.' {
            out.push(parse_num_token(&mut chars)?);
        } else if ch.is_ascii_alphabetic() {
            out.push(parse_name_token(&mut chars));
        } else if matches!(ch, '+' | '-' | '*' | '/' | '^' | '(' | ')' | ',' | ':') {
            out.push(Tok::Op(ch));
            chars.next();
        } else if matches!(ch, '<'..='>') {
            out.push(parse_cmp_token(ch, &mut chars));
        } else {
            return Err(syntax(format!("`{ch}` isn't part of a formula")));
        }
    }
    Ok(out)
}

/// The keywords that name the cells next to the formula's cell.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Side {
    Above,
    Left,
    Right,
    Below,
}

#[derive(Clone, Debug)]
enum Expr {
    Num(f64),
    /// A cell: (row, grid column).
    Cell(usize, usize, String),
    /// A rectangle of cells: (row0, grid col0, row1, grid col1), inclusive.
    Range(usize, usize, usize, usize),
    Side(Side),
    Neg(Box<Expr>),
    Bin(char, Box<Expr>, Box<Expr>),
    /// A comparison: 1 when it holds, 0 when it doesn't.
    Cmp(&'static str, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
    depth: usize,
}

impl Parser {
    fn peek_op(&self, c: char) -> bool {
        matches!(self.toks.get(self.pos), Some(Tok::Op(x)) if *x == c)
    }

    fn eat_op(&mut self, c: char) -> bool {
        let hit = self.peek_op(c);
        if hit {
            self.pos += 1;
        }
        hit
    }

    fn enter(&mut self) -> Result<(), CmdError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH { Err(syntax("the formula is nested too deeply".into())) } else { Ok(()) }
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// The loosest level: one comparison of two sums (comparisons don't chain).
    fn compare(&mut self) -> Result<Expr, CmdError> {
        let lhs = self.expr()?;
        let Some(Tok::Cmp(op)) = self.toks.get(self.pos).cloned() else { return Ok(lhs) };
        self.pos += 1;
        let rhs = self.expr()?;
        Ok(Expr::Cmp(op, Box::new(lhs), Box::new(rhs)))
    }

    fn expr(&mut self) -> Result<Expr, CmdError> {
        let mut lhs = self.term()?;
        loop {
            let op = if self.eat_op('+') {
                '+'
            } else if self.eat_op('-') {
                '-'
            } else {
                return Ok(lhs);
            };
            let rhs = self.term()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
    }

    fn term(&mut self) -> Result<Expr, CmdError> {
        let mut lhs = self.power()?;
        loop {
            let op = if self.eat_op('*') {
                '*'
            } else if self.eat_op('/') {
                '/'
            } else {
                return Ok(lhs);
            };
            let rhs = self.power()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
    }

    fn power(&mut self) -> Result<Expr, CmdError> {
        let base = self.unary()?;
        if self.eat_op('^') {
            // `^` is right-associative, so each one in a chain nests one level deeper.
            self.enter()?;
            let exp = self.power()?;
            self.leave();
            return Ok(Expr::Bin('^', Box::new(base), Box::new(exp)));
        }
        Ok(base)
    }

    fn unary(&mut self) -> Result<Expr, CmdError> {
        if self.eat_op('-') {
            self.enter()?;
            let inner = self.unary()?;
            self.leave();
            return Ok(Expr::Neg(Box::new(inner)));
        }
        if self.eat_op('+') {
            self.enter()?;
            let inner = self.unary()?;
            self.leave();
            return Ok(inner);
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Expr, CmdError> {
        let Some(tok) = self.toks.get(self.pos).cloned() else {
            return Err(syntax("the formula ends too soon".into()));
        };
        self.pos += 1;
        match tok {
            Tok::Num(x) => Ok(Expr::Num(x)),
            Tok::Op('(') => {
                self.enter()?;
                let inner = self.compare()?;
                if !self.eat_op(')') {
                    return Err(syntax("a `(` is missing its `)`".into()));
                }
                self.leave();
                Ok(inner)
            }
            Tok::Name(name) => {
                if self.eat_op('(') {
                    self.enter()?;
                    let mut args = Vec::new();
                    if !self.eat_op(')') {
                        loop {
                            args.push(self.compare()?);
                            if self.eat_op(')') {
                                break;
                            }
                            if !self.eat_op(',') {
                                return Err(syntax(format!("`{name}` needs `,` or `)` between its arguments")));
                            }
                        }
                    }
                    self.leave();
                    return Ok(Expr::Call(name, args));
                }
                let side = match name.as_str() {
                    "ABOVE" => Some(Side::Above),
                    "LEFT" => Some(Side::Left),
                    "RIGHT" => Some(Side::Right),
                    "BELOW" => Some(Side::Below),
                    _ => None,
                };
                if let Some(side) = side {
                    return Ok(Expr::Side(side));
                }
                // TRUE and FALSE need no parentheses.
                if matches!(name.as_str(), "TRUE" | "FALSE") {
                    return Ok(Expr::Call(name, Vec::new()));
                }
                let (row, col) = cell_ref(&name).ok_or_else(|| syntax(format!("`{name}` isn't a cell reference or a function")))?;
                if self.eat_op(':') {
                    let Some(Tok::Name(other)) = self.toks.get(self.pos).cloned() else {
                        return Err(syntax("a range ends with a cell reference, such as B4".into()));
                    };
                    self.pos += 1;
                    let (row2, col2) = cell_ref(&other).ok_or_else(|| syntax(format!("`{other}` isn't a cell reference")))?;
                    return Ok(Expr::Range(row.min(row2), col.min(col2), row.max(row2), col.max(col2)));
                }
                Ok(Expr::Cell(row, col, name))
            }
            Tok::Op(c) => Err(syntax(format!("`{c}` is in the wrong place"))),
            Tok::Cmp(op) => Err(syntax(format!("`{op}` is in the wrong place"))),
        }
    }
}

/// `B4` → (row 3, column 1): the row counts from 1 and the column from A.
fn cell_ref(name: &str) -> Option<(usize, usize)> {
    let letters: String = name.chars().take_while(char::is_ascii_alphabetic).collect();
    let digits = name.get(letters.len()..)?;
    if letters.is_empty() || digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let col = letters.bytes().try_fold(0usize, |acc, b| acc.checked_mul(26)?.checked_add((b.to_ascii_uppercase() - b'A') as usize + 1))? - 1;
    let row = digits.parse::<usize>().ok()?.checked_sub(1)?;
    (col < MAX_COLS && row < MAX_ROWS).then_some((row, col))
}

/// Where the formula's cell sits.
struct Here<'a> {
    t: &'a Table,
    doc: Option<&'a wordcraft_doc::Document>,
    default_lang: Option<&'a str>,
    r: usize,
    c: usize,
    g: usize,
    /// Cells read so far in this evaluation (see [`MAX_CELL_VISITS`]).
    visits: std::cell::Cell<usize>,
}

impl Here<'_> {
    /// Count `n` more cells read, failing once the evaluation has read too many.
    fn visit(&self, n: usize) -> Result<(), CmdError> {
        let total = self.visits.get().saturating_add(n);
        self.visits.set(total);
        if total > MAX_CELL_VISITS { Err(CmdError::Failed("Formula too complex".into())) } else { Ok(()) }
    }

    fn cell_lang(&self, c: &Cell) -> Option<String> {
        let para_lang = self.doc.and_then(|doc| {
            c.blocks.iter().filter_map(|b| b.as_para()).find(|p| !p.plain_text().trim().is_empty()).and_then(|p| super::para_lang(doc, p))
        });
        para_lang.or_else(|| self.default_lang.map(str::to_string))
    }

    fn cell_number(&self, c: &Cell) -> Option<f64> {
        let lang = self.cell_lang(c);
        let text = super::table::cell_text(c);
        parse_number(&text, lang.as_deref())
    }

    /// The number in the cell covering grid column `g` of row `r`, counted against the budget.
    fn number_at(&self, r: usize, g: usize) -> Result<Option<f64>, CmdError> {
        self.visit(1)?;
        let ci = match self.t.cell_at_grid(r, g) {
            Some(ci) => ci,
            None => return Ok(None),
        };
        let c = match self.t.rows.get(r).and_then(|row| row.cells.get(ci)) {
            Some(c) => c,
            None => return Ok(None),
        };
        Ok(self.cell_number(c))
    }
}

enum Val {
    Num(f64),
    List(Vec<f64>),
}

fn eval(e: &Expr, here: &Here) -> Result<Val, CmdError> {
    Ok(match e {
        Expr::Num(x) => Val::Num(*x),
        Expr::Cell(row, col, name) => {
            let x = here.number_at(*row, *col)?.ok_or_else(|| CmdError::Failed(format!("{name} doesn't hold a number")))?;
            Val::Num(x)
        }
        Expr::Range(r0, g0, r1, g1) => {
            // Only the part of the range inside the table holds cells.
            let mut v = Vec::new();
            let (r_end, g_end) = (here.t.rows.len().min(r1.saturating_add(1)), here.t.cols().min(g1.saturating_add(1)));
            for row in *r0..r_end {
                let mut last_ci = None;
                for g in *g0..g_end {
                    let ci = here.t.cell_at_grid(row, g);
                    if ci.is_some() && ci == last_ci {
                        continue;
                    }
                    last_ci = ci;
                    v.extend(here.number_at(row, g)?);
                }
            }
            Val::List(v)
        }
        Expr::Side(side) => Val::List(side_numbers(here, *side)?),
        Expr::Neg(inner) => Val::Num(-scalar(eval(inner, here)?)?),
        Expr::Bin(op, a, b) => {
            let (x, y) = (scalar(eval(a, here)?)?, scalar(eval(b, here)?)?);
            Val::Num(match op {
                '+' => x + y,
                '-' => x - y,
                '*' => x * y,
                '/' => {
                    if y == 0.0 {
                        return Err(CmdError::Failed("division by zero".into()));
                    }
                    x / y
                }
                _ => {
                    let p = x.powf(y);
                    finite(p)?
                }
            })
        }
        Expr::Cmp(op, a, b) => {
            let (x, y) = (scalar(eval(a, here)?)?, scalar(eval(b, here)?)?);
            let holds = match *op {
                "=" => x == y,
                "<>" => x != y,
                "<" => x < y,
                "<=" => x <= y,
                ">" => x > y,
                _ => x >= y,
            };
            Val::Num(truth(holds))
        }
        Expr::Call(name, args) => Val::Num(call(name, args, here)?),
    })
}

/// A truth value as a formula reads it: 1 for true, 0 for false.
fn truth(holds: bool) -> f64 {
    if holds { 1.0 } else { 0.0 }
}

/// A value that must be one number (arithmetic and single-argument functions).
fn scalar(v: Val) -> Result<f64, CmdError> {
    match v {
        Val::Num(x) => finite(x),
        Val::List(_) => Err(syntax("a range can only be used inside a function".into())),
    }
}

/// The first run of numbers next to the formula's cell on one side (see the module note): the
/// cells are skipped while they hold no number, then summed while they do.
fn side_numbers(here: &Here, side: Side) -> Result<Vec<f64>, CmdError> {
    let mut out = Vec::new();
    let mut started = false;
    let mut take = |value: Option<f64>| -> bool {
        match value {
            Some(x) => {
                out.push(x);
                started = true;
                true
            }
            None => !started,
        }
    };
    match side {
        Side::Above => {
            for row in (0..here.r).rev() {
                if !take(here.number_at(row, here.g)?) {
                    break;
                }
            }
        }
        Side::Below => {
            for row in (here.r + 1)..here.t.rows.len() {
                if !take(here.number_at(row, here.g)?) {
                    break;
                }
            }
        }
        Side::Left | Side::Right => {
            let Some(row) = here.t.rows.get(here.r) else { return Ok(out) };
            here.visit(row.cells.len())?;
            let cells: Vec<Option<f64>> = row.cells.iter().map(|c| here.cell_number(c)).collect();
            let order: Vec<usize> = if side == Side::Left { (0..here.c).rev().collect() } else { ((here.c + 1)..cells.len()).collect() };
            for ci in order {
                if !take(cells.get(ci).copied().flatten()) {
                    break;
                }
            }
        }
    }
    Ok(out)
}

/// Gather a function's arguments: numbers and lists (ranges and keywords) become one list.
fn gather(args: &[Expr], here: &Here) -> Result<Vec<f64>, CmdError> {
    let mut all = Vec::new();
    for a in args {
        match eval(a, here)? {
            Val::Num(x) => all.push(finite(x)?),
            Val::List(v) => all.extend(v),
        }
    }
    Ok(all)
}

/// The one number a single-argument function takes.
fn one(name: &str, args: &[Expr], here: &Here) -> Result<f64, CmdError> {
    match args {
        [a] => scalar(eval(a, here)?),
        _ => Err(CmdError::Failed(format!("{name} takes one argument"))),
    }
}

fn call(name: &str, args: &[Expr], here: &Here) -> Result<f64, CmdError> {
    let needs_one = matches!(
        name,
        "SUM" | "COUNT" | "AVERAGE" | "MAX" | "MIN" | "PRODUCT" | "ABS" | "INT" | "SIGN" | "ROUND" | "MOD" | "AND" | "OR" | "NOT" | "IF" | "DEFINED"
    );
    if args.is_empty() && needs_one {
        return Err(syntax(format!("{name}() needs an argument")));
    }
    match name {
        "TRUE" | "FALSE" if args.is_empty() => Ok(truth(name == "TRUE")),
        "TRUE" | "FALSE" => Err(CmdError::Failed(format!("{name} takes no arguments"))),
        "NOT" => Ok(truth(one(name, args, here)? == 0.0)),
        "AND" | "OR" => {
            let v = gather(args, here)?;
            if v.is_empty() {
                return Err(CmdError::Failed(format!("{name} has no numbers to test")));
            }
            Ok(truth(if name == "AND" { v.iter().all(|x| *x != 0.0) } else { v.iter().any(|x| *x != 0.0) }))
        }
        // Only the branch taken is worked out, so an error in the other one doesn't count.
        "IF" => match args {
            [cond, yes, no] => {
                if scalar(eval(cond, here)?)? != 0.0 {
                    scalar(eval(yes, here)?)
                } else {
                    scalar(eval(no, here)?)
                }
            }
            _ => Err(CmdError::Failed("IF takes three arguments: a condition, the value when it is true, and the value when it is false".into())),
        },
        // DEFINED(cell) is 1 when the cell holds a value the formula can read, 0 when it doesn't.
        "DEFINED" => match args {
            [a] => Ok(truth(eval(a, here).is_ok())),
            _ => Err(CmdError::Failed("DEFINED takes one argument".into())),
        },
        "SUM" => Ok(gather(args, here)?.iter().sum()),
        "COUNT" => Ok(gather(args, here)?.len() as f64),
        "AVERAGE" => {
            let v = gather(args, here)?;
            Ok(if v.is_empty() { 0.0 } else { v.iter().sum::<f64>() / v.len() as f64 })
        }
        "MAX" => {
            let v = gather(args, here)?;
            Ok(if v.is_empty() { 0.0 } else { v.iter().copied().fold(f64::NEG_INFINITY, f64::max) })
        }
        "MIN" => {
            let v = gather(args, here)?;
            Ok(if v.is_empty() { 0.0 } else { v.iter().copied().fold(f64::INFINITY, f64::min) })
        }
        "PRODUCT" => {
            let v = gather(args, here)?;
            Ok(if v.is_empty() { 1.0 } else { v.iter().product() })
        }
        "ABS" => Ok(one(name, args, here)?.abs()),
        "INT" => Ok(one(name, args, here)?.floor()),
        "SIGN" => {
            let x = one(name, args, here)?;
            Ok(if x == 0.0 { 0.0 } else { x.signum() })
        }
        "ROUND" => match args {
            [a, b] => {
                let x = scalar(eval(a, here)?)?;
                let digits = scalar(eval(b, here)?)?.clamp(-15.0, 15.0).trunc();
                let scale = 10f64.powi(digits as i32);
                let scaled = x * scale;
                if !scaled.is_finite() {
                    // Too large to have a fractional part at this precision.
                    return finite(x);
                }
                finite(scaled.round() / scale)
            }
            _ => Err(CmdError::Failed("ROUND takes two arguments".into())),
        },
        "MOD" => match args {
            [a, b] => {
                let (x, y) = (scalar(eval(a, here)?)?, scalar(eval(b, here)?)?);
                if y == 0.0 {
                    return Err(CmdError::Failed("division by zero".into()));
                }
                Ok(x - y * (x / y).floor())
            }
            _ => Err(CmdError::Failed("MOD takes two arguments".into())),
        },
        _ => Err(CmdError::Failed(format!("unknown formula function `{name}`"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(rows: &[&[&str]]) -> Table {
        let cols = rows.iter().map(|r| r.len()).max().unwrap_or(1);
        let mut t = Table::new(rows.len(), cols, 468.0);
        for (r, row) in rows.iter().enumerate() {
            for (c, text) in row.iter().enumerate() {
                if let Some(cl) = t.rows.get_mut(r).and_then(|x| x.cells.get_mut(c)) {
                    cl.blocks = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(text, Default::default()))];
                }
            }
        }
        t
    }

    fn ev(t: &Table, r: usize, c: usize, f: &str) -> Result<f64, CmdError> {
        evaluate(t, r, c, f)
    }

    #[test]
    fn sign_of_zero_is_zero() {
        let t = table(&[&["0", "5", "-5"]]);
        assert_eq!(ev(&t, 0, 0, "=SIGN(0)"), Ok(0.0));
        assert_eq!(ev(&t, 0, 0, "=SIGN(-0)"), Ok(0.0));
        assert_eq!(ev(&t, 0, 1, "=SIGN(A1)"), Ok(0.0), "a cell holding 0");
        assert_eq!(ev(&t, 0, 0, "=SIGN(5)"), Ok(1.0));
        assert_eq!(ev(&t, 0, 0, "=SIGN(-5)"), Ok(-1.0));
    }

    #[test]
    fn cell_references_count_from_one_and_from_a() {
        assert_eq!(cell_ref("A1"), Some((0, 0)));
        assert_eq!(cell_ref("B4"), Some((3, 1)));
        assert_eq!(cell_ref("AA2"), Some((1, 26)));
        assert_eq!(cell_ref("A0"), None);
        assert_eq!(cell_ref("1A"), None);
    }

    #[test]
    fn a_run_of_numbers_above_is_summed_after_skipping_cells_without_numbers() {
        // Observed in Word: the cells above are skipped while empty or text, then the numbers are
        // summed up to the next cell that holds none.
        let t = table(&[&["5"], &["x"], &["2"], &[""]]);
        assert_eq!(ev(&t, 3, 0, "=SUM(ABOVE)"), Ok(2.0), "text stops the run; the 5 above it is not counted");
        assert_eq!(ev(&t, 2, 0, "=SUM(ABOVE)"), Ok(5.0), "text directly above is skipped, so the run is the 5 above it");
        let t = table(&[&["5"], &[""], &[""], &[""]]);
        assert_eq!(ev(&t, 3, 0, "=SUM(ABOVE)"), Ok(5.0), "empty cells are skipped up to the 5");
        let t = table(&[&["5"], &["3"], &[""], &["9"], &[""]]);
        assert_eq!(ev(&t, 4, 0, "=SUM(ABOVE)"), Ok(9.0), "the run is the 9 alone: the blank cuts it off");
        let t = table(&[&["x"], &["5"], &[""]]);
        assert_eq!(ev(&t, 2, 0, "=SUM(ABOVE)"), Ok(5.0));
    }

    #[test]
    fn a_function_needs_its_arguments() {
        let t = table(&[&["1"], &[""]]);
        assert!(ev(&t, 1, 0, "=SUM()").is_err(), "empty parentheses are a syntax error, as in Word");
        assert!(ev(&t, 1, 0, "=SUM").is_err());
    }

    #[test]
    fn unknown_functions_are_errors_not_sums() {
        let t = table(&[&["1"], &["4"], &["7"], &[""]]);
        let err = ev(&t, 3, 0, "=MEDIAN(ABOVE)").err().map(|e| e.to_string()).unwrap_or_default();
        assert!(err.contains("MEDIAN"), "{err}");
    }

    #[test]
    fn round_large_values_return_input() {
        let t = table(&[&["1", "2"], &["3", "4"], &["", ""]]);
        let big = format!("1{}", "0".repeat(300));
        assert_eq!(ev(&t, 2, 0, &format!("=ROUND({big},15)")), Ok(1e300));
        assert_eq!(ev(&t, 2, 0, &format!("=ROUND({big},2)")), Ok(1e300));
    }

    #[test]
    fn ranges_and_references_and_operators() {
        let t = table(&[&["1", "2"], &["3", "4"], &["", ""]]);
        assert_eq!(ev(&t, 2, 0, "=SUM(A1:B2)"), Ok(10.0));
        assert_eq!(ev(&t, 2, 1, "=A1*B2+1"), Ok(5.0));
        assert_eq!(ev(&t, 2, 0, "=(A1+A2)/2"), Ok(2.0));
        assert_eq!(ev(&t, 2, 0, "=2^3-(-1)"), Ok(9.0));
        assert_eq!(ev(&t, 2, 0, "=AVERAGE(A1:B2)"), Ok(2.5));
        assert_eq!(ev(&t, 2, 0, "=MAX(A1:B2)"), Ok(4.0));
        assert_eq!(ev(&t, 2, 0, "=ROUND(2.5, 0)"), Ok(3.0));
        assert_eq!(ev(&t, 2, 0, "=MOD(7, 3)"), Ok(1.0));
    }

    #[test]
    fn range_over_spanning_cells_counts_each_cell_once() {
        let mut t = table(&[&["50", "10"], &["", ""]]);
        t.rows[0].cells[0].props.span = 2;
        assert_eq!(t.cols(), 3);
        assert_eq!(ev(&t, 1, 0, "=SUM(A1:B1)"), Ok(50.0), "spanning cell A1:B1 is evaluated once");
        assert_eq!(ev(&t, 1, 0, "=COUNT(A1:B1)"), Ok(1.0), "spanning cell A1:B1 has count 1");
        assert_eq!(ev(&t, 1, 0, "=SUM(A1:C1)"), Ok(60.0), "spanning cell A1:B1 plus cell C1 = 60");
    }

    #[test]
    fn division_by_zero_and_bad_syntax_are_errors() {
        let t = table(&[&["1"], &["0"]]);
        assert!(ev(&t, 1, 0, "=A1/A2").is_err());
        assert!(ev(&t, 1, 0, "=SUM(A1").is_err());
        assert!(ev(&t, 1, 0, "=A1 A1").is_err());
        assert!(ev(&t, 1, 0, "=A1:").is_err());
        assert!(ev(&t, 1, 0, &format!("={}", "(".repeat(64))).is_err(), "deep nesting is refused");
    }

    #[test]
    fn numbers_read_currency_and_separators_but_not_letters() {
        assert_eq!(parse_number("$1,200", None), Some(1200.0));
        assert_eq!(parse_number(" 12% ", None), Some(12.0));
        assert_eq!(parse_number("(5)", None), Some(-5.0));
        assert_eq!(parse_number("x9", None), None);
        assert_eq!(parse_number("1a", None), None);
        assert_eq!(parse_number("inf", None), None);
        assert_eq!(parse_number("", None), None);
        assert_eq!(parse_number("3,50", Some("pl-PL")), Some(3.5));
    }

    #[test]
    fn comparisons_give_one_or_zero_and_do_not_chain() {
        let t = table(&[&["5"], &["3"]]);
        assert_eq!(ev(&t, 1, 0, "=A1>A2"), Ok(1.0));
        assert_eq!(ev(&t, 1, 0, "=A1<=A2"), Ok(0.0));
        assert_eq!(ev(&t, 1, 0, "=A1<>A2"), Ok(1.0));
        assert_eq!(ev(&t, 1, 0, "=A2>=3"), Ok(1.0));
        assert_eq!(ev(&t, 1, 0, "=A1=5"), Ok(1.0));
        assert_eq!(ev(&t, 1, 0, "=A1<A2<1"), Err(CmdError::Failed("syntax error: unexpected text after the formula".into())));
    }

    #[test]
    fn if_and_or_not_and_the_truth_values() {
        let t = table(&[&["5", "x"], &["3", ""]]);
        assert_eq!(ev(&t, 1, 0, "=IF(A1>A2, 10, 20)"), Ok(10.0));
        assert_eq!(ev(&t, 1, 0, "=IF(A1<A2, 10, 20)"), Ok(20.0));
        assert_eq!(ev(&t, 1, 0, "=IF(A1>A2, 1, B1+1)"), Ok(1.0), "the branch not taken is not worked out");
        assert!(ev(&t, 1, 0, "=IF(A1<A2, 1, B1+1)").is_err(), "the branch taken is");
        assert_eq!(ev(&t, 1, 0, "=AND(A1>1, A2>1)"), Ok(1.0));
        assert_eq!(ev(&t, 1, 0, "=AND(A1>1, A2>4)"), Ok(0.0));
        assert_eq!(ev(&t, 1, 0, "=OR(A1<1, A2>1)"), Ok(1.0));
        assert_eq!(ev(&t, 1, 0, "=OR(A1<1, A2<1)"), Ok(0.0));
        assert_eq!(ev(&t, 1, 0, "=NOT(A1<1)"), Ok(1.0));
        assert_eq!(ev(&t, 1, 0, "=TRUE"), Ok(1.0));
        assert_eq!(ev(&t, 1, 0, "=FALSE()"), Ok(0.0));
        assert!(ev(&t, 1, 0, "=TRUE(1)").is_err());
        assert!(ev(&t, 1, 0, "=IF(A1>A2, 1)").is_err(), "IF takes its three arguments");
        assert!(ev(&t, 1, 0, "=AND()").is_err());
    }

    #[test]
    fn defined_says_whether_a_cell_holds_a_value() {
        let t = table(&[&["5", "x", ""]]);
        assert_eq!(ev(&t, 0, 0, "=DEFINED(A1)"), Ok(1.0));
        assert_eq!(ev(&t, 0, 0, "=DEFINED(B1)"), Ok(0.0), "text isn't a value");
        assert_eq!(ev(&t, 0, 0, "=DEFINED(C1)"), Ok(0.0), "an empty cell isn't");
        assert_eq!(ev(&t, 0, 0, "=IF(DEFINED(C1), C1, 7)"), Ok(7.0));
    }

    #[test]
    fn a_numeric_picture_formats_the_result() {
        assert_eq!(format_picture(1234.5, "#,##0.00"), "1,234.50");
        assert_eq!(format_picture(0.256, "0%"), "26%");
        assert_eq!(format_picture(2.0, "#.##"), "2");
        assert_eq!(format_picture(0.05, "0.0"), "0.1", "halves round away from zero");
        assert_eq!(format_picture(-5.0, "$#,##0"), "-$5", "one section: a minus in front");
        assert_eq!(format_picture(-5.0, "$#,##0.00;($#,##0.00)"), "($5.00)", "the second section is the negative form");
        assert_eq!(format_picture(0.0, "0.00;-;zero"), "zero");
        assert_eq!(format_picture(7.0, "000"), "007");
        assert_eq!(format_picture(-0.001, "0.00"), "0.00", "a rounded zero has no sign");
    }

    #[test]
    fn a_field_switch_picks_the_picture_and_the_formula_stays_apart_from_it() {
        let t = table(&[&["1234.5"], &[""]]);
        assert_eq!(split_switch("=A1 \\# \"0.00\""), ("=A1", Some("\\# \"0.00\"")));
        assert_eq!(split_switch("=A1"), ("=A1", None));
        let shown = |f: &str| evaluate_field(&t, 1, 0, f).unwrap_or_else(|e| panic!("{f}: {e}"));
        assert_eq!(shown("=A1 \\# \"#,##0.00\""), "1,234.50");
        assert_eq!(shown("=A1"), "1234.50");
        assert_eq!(shown("=A1 \\# 0"), "1235", "a bare picture word");
        assert_eq!(shown("=A1 \\# \"\""), "1234.50", "an empty picture is none");
    }

    #[test]
    fn a_huge_range_over_a_small_table_reads_only_the_table() {
        let t = table(&[&["1", "2", "3"], &["4", "5", "6"], &["", "", ""]]);
        // Forty ranges reaching to the last possible row and column would be about 80 million
        // grid positions; clamped to the table they are 6 numbers each, well inside the budget.
        let f = format!("=SUM({})", ["A1:BK32767"; 40].join(","));
        assert!(f.len() <= MAX_FORMULA);
        assert_eq!(ev(&t, 2, 0, &f).unwrap(), 21.0 * 40.0);
    }

    #[test]
    fn an_evaluation_past_the_cell_budget_is_an_error() {
        let mut t = Table::new(1000, 10, 468.0);
        t.rows[0].cells[0] = Cell::with_text("1");
        // Each range reads 10,000 cells, so eleven of them go past the 100,000-cell budget.
        let ok = format!("=SUM({})", ["A1:J1000"; 10].join(","));
        assert_eq!(ev(&t, 0, 1, &ok).unwrap(), 10.0);
        let too_much = format!("=SUM({})", ["A1:J1000"; 11].join(","));
        let err = ev(&t, 0, 1, &too_much).unwrap_err();
        assert_eq!(err, CmdError::Failed("Formula too complex".into()));
        assert_eq!(evaluate_field(&t, 0, 1, &too_much).unwrap_err().to_string(), "Formula too complex");
    }

    #[test]
    fn deep_power_and_sign_chains_are_rejected() {
        let t = table(&[&["1"]]);
        assert_eq!(ev(&t, 0, 0, "=2^3^2").unwrap(), 512.0);
        assert_eq!(ev(&t, 0, 0, "=-+-+2").unwrap(), 2.0);
        for f in [format!("={}1", "1^".repeat(40)), format!("={}1", "+".repeat(40)), format!("={}1", "-+".repeat(20))] {
            let err = ev(&t, 0, 0, &f).unwrap_err();
            assert!(err.to_string().contains("nested too deeply"), "{f}: {err}");
        }
    }
}
