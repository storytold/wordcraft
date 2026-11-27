//! The displayed form of a calculation: its linear-format (UnicodeMath) text and math tree.
//!
//! The display is generated from the parsed statement, never copied from the source text, so the
//! structure on screen is the structure that was evaluated: wherever UnicodeMath would group the
//! text differently from the calculation parser, the display carries explicit parentheses
//! (`√x²` parses as `(√x)²` and shows as `(√x)^2`). Units are upright text, quoted in the linear
//! form (`1500 "lbf"`). The result follows: `=600 "psi"` when current, a grey `≈600 "psi"` when
//! stale, and a red ` ⚠` instead of any number when the calculation has an error.
//!
//! A vector or matrix result is a bracketed Word matrix of its formatted elements, with their one
//! shared unit after the bracket (`=[■(0.84@1.05)] "in"`) or each element's own unit inside it
//! (`=[■(2 "in"@3 "lbf")]`). Past [`MAX_SHOWN_ROWS`] rows or [`MAX_SHOWN_COLS`] columns it is cut,
//! never silently: a `⋮` row after the rows shown, a `⋯` column after the columns shown (`⋱` where
//! they meet), and the full size after the bracket (`=[■(0@…@49@⋮)] "(60 rows)"`,
//! `"(10000 columns)"`, `"(60 rows × 60 columns)"`).

use std::collections::HashMap;

use craft_units::{Formatted, lookup};
use wordcraft_doc::Rgb;
use wordcraft_doc::calc::CalcState;
use wordcraft_doc::math::{Arg, ColJc, FUNCTION_NAMES, MNode, MRun, MSty, Math, parse_linear};
use wordcraft_doc::math_linear::map_args;

use crate::array::{MAX_ELEMENTS, MAX_TEXT_CHARS};
use crate::{CalcError, Cmp, Expr, Logic, Op, SolveKind, Stmt, funcs, parse};

/// Colour of the error marker.
pub const ERROR_RED: Rgb = Rgb(0xC0, 0x00, 0x00);
/// Colour of a stale result.
pub const STALE_GREY: Rgb = Rgb(0x80, 0x80, 0x80);
/// Colour of the mark after a calculation edited outside WordCraft.
pub const EDITED_AMBER: Rgb = Rgb(0xC0, 0x60, 0x00);
/// The marker shown after a calculation with an error, in place of its result.
const ERROR_MARK: &str = "⚠";
/// The mark after a calculation edited outside WordCraft (amber, where an error's is red).
const EDITED_MARK: &str = "⚠";
/// Longest source text shown for a calculation that doesn't parse, in characters.
const MAX_SHOWN_SOURCE: usize = 2000;
/// First private-use character standing for a unit while the linear text is parsed into a tree.
const SENTINEL_BASE: u32 = 0xE000;
/// How many distinct units and names one display can stand in for (the private-use area's size).
const MAX_SENTINELS: usize = 6400;
/// UnicodeMath's invisible brackets: a group that shows no brackets.
const INVISIBLE_OPEN: &str = "〖";
const INVISIBLE_CLOSE: &str = "〗";
/// The base of an exponential number, grouped so its power is on both digits.
const TEN: &str = "〖10〗";
/// Separates a number from the unit after it in the math tree (spaces in linear text are dropped).
const THIN_SPACE: char = '\u{2009}';
/// Most rows of a vector or matrix result shown; the rest are cut, with a [`CUT`] row after the
/// rows shown and the full size after the bracket.
pub(crate) const MAX_SHOWN_ROWS: usize = 50;
/// Most columns of a result shown; the rest are cut, with a [`COL_CUT`] column after the columns
/// shown and the full size after the bracket.
pub(crate) const MAX_SHOWN_COLS: usize = 50;
/// Each cell of the row standing for the rows cut from a long result.
const CUT: &str = "⋮";
/// Each cell of the column standing for the columns cut from a wide result.
const COL_CUT: &str = "⋯";
/// Where the cut row and the cut column meet.
const CORNER: &str = "⋱";
/// Longest result text [`shown_result`] reads back, bytes: what [`rendered`] writes for the
/// largest grid (every cell shown, the cut row and column, each text as long as a text gets, all
/// of 4-byte characters, with its quotes and separator), and a margin for the unit and size notes.
/// Anything longer came from somewhere else.
const MAX_RESULT_BYTES: usize = (MAX_SHOWN_ROWS + 1) * (MAX_SHOWN_COLS + 1) * (4 * MAX_TEXT_CHARS + 8) + 4096;

/// A result as shown after `=`: formatted, ready to display.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Shown {
    /// One value.
    One(Formatted),
    /// A vector or matrix: its first `rows` rows (at most [`MAX_SHOWN_ROWS`]) of its first `cols`
    /// columns (at most [`MAX_SHOWN_COLS`]), row by row, of `total_rows` × `total_cols` in all.
    /// `unit` is `Some` when every element shares it (never empty), and the items then carry an
    /// empty unit.
    Grid { rows: usize, cols: usize, items: Vec<Formatted>, unit: Option<String>, total_rows: usize, total_cols: usize },
    /// One text.
    Text(String),
    /// A vector or matrix of texts, cut as [`Shown::Grid`] is.
    TextGrid { rows: usize, cols: usize, items: Vec<String>, total_rows: usize, total_cols: usize },
}

/// The linear-format text shown for the calculation `stmt_src`, with its result `shown` (when
/// the statement shows one) in the given state. A source that doesn't parse is shown as typed,
/// followed by the error marker.
pub fn display(stmt_src: &str, shown: Option<&Formatted>, state: &CalcState) -> String {
    let shown = shown.cloned().map(Shown::One);
    rendered(&parse(stmt_src), stmt_src, shown.as_ref(), state).linear
}

/// Render an equality for display outside a solve block, with no result or calculation state.
pub fn standalone_equality(lhs: Expr, rhs: Expr) -> (String, Math) {
    unevaluated(&Stmt::Constraint { lhs, rhs }, true)
}

/// Render a statement as editable math without evaluating it or adding a result.
pub fn unevaluated(stmt: &Stmt, definitions_as_equalities: bool) -> (String, Math) {
    let mut out = Out::default();
    let split = write_stmt(stmt, &mut out, definitions_as_equalities);
    let parse = |text| restore_pieces(parse_linear(text), &out.pieces, &out.nodes);
    let nodes = if let Some(at) = split {
        let (lhs, rhs) = out.parsed.split_at(at);
        let mut nodes = parse(lhs);
        nodes.push(bold_equality());
        nodes.extend(parse(rhs));
        nodes
    } else {
        parse(&out.parsed)
    };
    (out.linear, Math { nodes, ..Math::default() })
}

/// A display: the linear text and the math tree to store on the equation.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Rendered {
    pub linear: String,
    pub math: Arg,
}

/// Whether a statement shows a value after `=`.
pub(crate) fn shows_value(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Eval { .. } => true,
        Stmt::Define { show, .. } | Stmt::DefineMany { show, .. } => show.is_some(),
        Stmt::Given | Stmt::Constraint { .. } => false,
    }
}

/// The display of an already parsed statement (`src` is used only when it didn't parse).
pub(crate) fn rendered(stmt: &Result<Stmt, CalcError>, src: &str, shown: Option<&Shown>, state: &CalcState) -> Rendered {
    let mut out = Out::default();
    let shows = match stmt {
        Ok(stmt) => {
            write_stmt(stmt, &mut out, false);
            shows_value(stmt)
        }
        Err(_) => {
            let text: String = src.chars().take(MAX_SHOWN_SOURCE).collect();
            let ellipsis = if src.chars().nth(MAX_SHOWN_SOURCE).is_some() { "…" } else { "" };
            out.text(&text);
            out.text(ellipsis);
            false
        }
    };
    let mut linear = out.linear;
    let mut math = restore_pieces(parse_linear(&out.parsed), &out.pieces, &out.nodes);
    if stmt.is_err() || matches!(state, CalcState::Error(_)) {
        linear.push(' ');
        linear.push_str(ERROR_MARK);
        math.push(MNode::Run(MRun { text: ERROR_MARK.to_string(), sty: Some(MSty::Plain), color: Some(ERROR_RED), ..MRun::default() }));
    } else if shows {
        let stale = !matches!(state, CalcState::Fresh);
        let rel = if stale { "≈" } else { "=" };
        let mut result: Arg = vec![MNode::Run(MRun::new(rel))];
        linear.push_str(rel);
        match shown {
            None => push_value(&Formatted { number: "?".into(), unit: String::new() }, &mut linear, &mut result),
            Some(Shown::One(f)) => push_value(f, &mut linear, &mut result),
            Some(Shown::Text(t)) => push_text(t, &mut linear, &mut result),
            Some(grid @ (Shown::Grid { .. } | Shown::TextGrid { .. })) => push_grid(grid, &mut linear, &mut result),
        }
        if stale {
            result = paint(result, STALE_GREY);
        }
        math.extend(result);
    }
    Rendered { linear, math }
}

/// A formatted number and its unit, as linear text (`600 "psi"`) and math nodes.
fn push_value(f: &Formatted, linear: &mut String, math: &mut Arg) {
    let number = linear_number(&f.number);
    if number.contains('E') {
        math.push(MNode::Run(unit_run(number.clone())));
    } else if let Some((whole, fraction)) = number.split_once(' ').filter(|(_, part)| part.contains('/')) {
        math.extend(parse_linear(whole));
        math.push(MNode::Run(unit_run(THIN_SPACE.to_string())));
        math.extend(parse_linear(fraction));
    } else {
        math.extend(restore_pieces(parse_linear(&number), &[], &[]));
    }
    linear.push_str(&number);
    push_unit(&f.unit, linear, math);
}

/// Units retain their quoted linear text for result recovery. Built-up math displays the
/// same unit expression with upright symbols, fractions and scripts.
fn push_unit(text: &str, linear: &mut String, math: &mut Arg) {
    if !text.is_empty() {
        linear.push_str(&format!(" \"{text}\""));
        if text.contains(['/', '^', '·', '*', '⋅', '('])
            && craft_units::parse_unit(text).is_ok()
            && let Ok(Stmt::Eval { expr, .. }) = parse(&format!("{text} ="))
        {
            math.push(MNode::Run(unit_run(THIN_SPACE.to_string())));
            math.extend(cell(&expr, &mut String::new()));
        } else {
            math.push(MNode::Run(unit_run(format!("{THIN_SPACE}{text}"))));
        }
    }
}

/// A text in straight quotes, as linear text and as one upright run: the quote marks are part of
/// the run, so the text reads as text and is never read as mathematics.
fn push_text(text: &str, linear: &mut String, math: &mut Arg) {
    linear.push_str(&format!("\"{text}\""));
    math.push(MNode::Run(text_run(text)));
}

/// A vector or matrix result ([`Shown::Grid`], or [`Shown::TextGrid`] of texts): a bracketed matrix
/// of its items, `cols` to a row; a [`COL_CUT`] cell after each row when columns were cut, a
/// [`CUT`] row when rows were (with a [`CORNER`] under the column cut); then the shared unit and,
/// when cut, the full size.
///
/// The tree is built cell by cell rather than parsed from the whole linear text, so every cell
/// shown is drawn (the linear parser stops at a bound on its work) and each costs the same.
fn push_grid(shown: &Shown, linear: &mut String, math: &mut Arg) {
    let (rows, cols, total_rows, total_cols) = match shown {
        Shown::Grid { rows, cols, items, unit, total_rows, total_cols } => {
            push_cells(items, (*rows, *cols), (*total_rows, *total_cols), push_value, linear, math);
            push_unit(unit.as_deref().unwrap_or_default(), linear, math);
            (*rows, *cols, *total_rows, *total_cols)
        }
        Shown::TextGrid { rows, cols, items, total_rows, total_cols } => {
            push_cells(items, (*rows, *cols), (*total_rows, *total_cols), |t: &String, l: &mut String, m: &mut Arg| push_text(t, l, m), linear, math);
            (*rows, *cols, *total_rows, *total_cols)
        }
        Shown::One(_) | Shown::Text(_) => return,
    };
    let cols = cols.max(1);
    let size = match (total_rows > rows, total_cols > cols) {
        (true, true) => format!("({total_rows} rows × {total_cols} columns)"),
        (true, false) => format!("({total_rows} rows)"),
        (false, true) => format!("({total_cols} columns)"),
        (false, false) => String::new(),
    };
    push_unit(&size, linear, math);
}

/// The bracketed matrix of `items` (`shape` is the rows and columns shown, `total` those of the
/// whole result), each item written by `put`; the cut row and column when `total` is larger.
fn push_cells<T>(
    items: &[T],
    shape: (usize, usize),
    total: (usize, usize),
    put: impl Fn(&T, &mut String, &mut Arg),
    linear: &mut String,
    math: &mut Arg,
) {
    let (rows, cols) = (shape.0, shape.1.max(1));
    let (cut_rows, cut_cols) = (total.0 > rows, total.1 > cols);
    let mark = |text: &str| vec![MNode::Run(MRun::new(text))];
    let mut matrix: Vec<Vec<Arg>> = Vec::with_capacity(rows.saturating_add(1));
    linear.push_str("[■(");
    for (r, row) in items.chunks(cols).enumerate() {
        if r > 0 {
            linear.push('@');
        }
        let mut cells = Vec::with_capacity(cols.saturating_add(1));
        for (c, item) in row.iter().enumerate() {
            if c > 0 {
                linear.push('&');
            }
            let mut cell = Vec::new();
            put(item, linear, &mut cell);
            cells.push(cell);
        }
        if cut_cols {
            linear.push('&');
            linear.push_str(COL_CUT);
            cells.push(mark(COL_CUT));
        }
        matrix.push(cells);
    }
    if cut_rows {
        linear.push('@');
        linear.push_str(&vec![CUT; cols].join("&"));
        let mut cells = vec![mark(CUT); cols];
        if cut_cols {
            linear.push('&');
            linear.push_str(CORNER);
            cells.push(mark(CORNER));
        }
        matrix.push(cells);
    }
    linear.push_str(")]");
    math.push(bracketed(matrix));
}

/// A matrix of `rows` of cells in square brackets, as Word's linear format builds `[■(…)]`.
fn bracketed(rows: Vec<Vec<Arg>>) -> MNode {
    let matrix = MNode::Matrix { rows, col_jc: Vec::new() };
    MNode::Delim { beg: Some('['), end: Some(']'), sep: None, grow: true, shp_match: false, elems: vec![vec![matrix]] }
}

/// A short red note after a display, upright (`⚠ not calculated here`).
pub(crate) fn push_note(display: &mut Rendered, note: &str) {
    display.linear.push_str(&format!(" \"{note}\""));
    display.math.push(MNode::Run(MRun { text: format!("{THIN_SPACE}{note}"), sty: Some(MSty::Plain), color: Some(ERROR_RED), ..MRun::default() }));
}

/// The mark run after an equation edited outside WordCraft.
fn edited_run() -> MNode {
    MNode::Run(MRun { text: EDITED_MARK.to_string(), sty: Some(MSty::Plain), color: Some(EDITED_AMBER), ..MRun::default() })
}

fn is_edited_run(n: &MNode) -> bool {
    matches!(n, MNode::Run(r) if r.text == EDITED_MARK && r.color == Some(EDITED_AMBER))
}

/// Whether the equation's nodes end in the edited mark.
pub fn has_edited_mark(math: &Math) -> bool {
    math.nodes.last().is_some_and(is_edited_run)
}

/// The display of an equation edited outside WordCraft (`linear`, `math` as Word left them) with
/// the edited mark after it; `None` when it has the mark already. Nothing of Word's equation is
/// changed: its tree (read from its linear text when it has none yet) gains one run.
pub(crate) fn mark_edited(linear: &str, math: &Math) -> Option<Rendered> {
    if has_edited_mark(math) {
        return None;
    }
    let mut nodes = if math.nodes.is_empty() { parse_linear(linear) } else { math.nodes.clone() };
    nodes.push(edited_run());
    Some(Rendered { linear: format!("{linear} {EDITED_MARK}"), math: nodes })
}

/// Takes the edited mark ([`mark_edited`]) off an equation again: before it is saved for Word,
/// and when the author keeps Word's equation. An equation without it is left as it is.
pub fn strip_edited_mark(linear: &mut String, math: &mut Math) {
    if !has_edited_mark(math) {
        return;
    }
    math.nodes.pop();
    if let Some(rest) = linear.strip_suffix(&format!(" {EDITED_MARK}")) {
        *linear = rest.to_string();
    }
    math.omml.clear();
}

/// `display` with an automatic equation number, the way a numbered plain equation has one
/// (`equation.number`): one equation-array row ending in `#`, which layout numbers and a `.docx`
/// writes as `#(n)`.
pub(crate) fn numbered(mut display: Rendered) -> Rendered {
    let mut row = std::mem::take(&mut display.math);
    row.push(MNode::Run(MRun::new("#")));
    display.math = vec![MNode::EqArr { rows: vec![row] }];
    display.linear.push('#');
    display
}

/// A numbered equation's one row without its number (everything from the `#` on): what its
/// fingerprint is taken of, so the automatic `#` saved and the `#(1)` Word gives back match.
/// `None` when the equation isn't numbered that way.
pub(crate) fn unnumbered(nodes: &[MNode]) -> Option<Arg> {
    let [MNode::EqArr { rows }] = nodes else { return None };
    let [row] = rows.as_slice() else { return None };
    if !wordcraft_doc::math::has_number_mark(row) {
        return None;
    }
    let mut out = Vec::new();
    for n in row {
        match n {
            MNode::Run(r) if !r.lit && !r.nor && r.text.contains('#') => {
                let mut r = r.clone();
                if let Some(k) = r.text.find('#') {
                    r.text.truncate(k);
                }
                if !r.text.is_empty() {
                    out.push(MNode::Run(r));
                }
                break;
            }
            n => out.push(n.clone()),
        }
    }
    Some(out)
}

/// Read a unitless scalar from a calculation's rendered result, at its display precision.
pub fn displayed_unitless_scalar(linear: &str) -> Option<f64> {
    let Shown::One(value) = shown_result(linear)? else { return None };
    if !value.unit.is_empty() {
        return None;
    }
    let numeric = value.number.replace("×10^", "e");
    let number = if let Some((left, denominator)) = numeric.split_once('/') {
        let denominator: f64 = denominator.parse().ok()?;
        if let Some((whole, numerator)) = left.split_once(' ') {
            let whole: f64 = whole.parse().ok()?;
            let fraction = numerator.parse::<f64>().ok()? / denominator;
            whole + fraction.copysign(whole)
        } else {
            left.parse::<f64>().ok()? / denominator
        }
    } else {
        numeric.parse().ok()?
    };
    number.is_finite().then_some(number)
}

/// The result shown in a display made by [`rendered`] (`…=600 "psi"`, `…=[■(0.84@1.05)] "in"`,
/// `…="OK"`, `…=[■("OK"@"NG")]`), read back from its linear text; `None` when the text doesn't end
/// in a result [`rendered`] could have written. The text comes from files, so the work is bounded
/// by its length, and a result longer than any [`rendered`] writes is no result.
///
/// The result follows the last `=` or `≈` outside quotes, found scanning back from the end, so it
/// depends on the result's own quotes alone (whatever the expression before it holds, quoted or
/// not). Units and texts are quoted and have no quote inside, so a text holding an `=` (or an `&`,
/// `@`, `)`) is never taken for the result's start or end.
pub(crate) fn shown_result(linear: &str) -> Option<Shown> {
    let linear = linear.strip_suffix('#').unwrap_or(linear);
    let mut quoted = false;
    let mut start = None;
    for (i, c) in linear.char_indices().rev() {
        if linear.len().saturating_sub(i) > MAX_RESULT_BYTES {
            return None;
        }
        match c {
            '"' => quoted = !quoted,
            // Either half of an `==` is no result marker (`x≔a==1` has lost its result).
            '=' if linear.get(..i).is_some_and(|s| s.ends_with('=')) || linear.get(i + 1..).is_some_and(|s| s.starts_with('=')) => {}
            '=' | '≈' if !quoted => {
                start = Some(i + c.len_utf8());
                break;
            }
            _ => {}
        }
    }
    let tail = linear.get(start?..)?;
    if let Some(grid) = tail.strip_prefix("[■(") {
        return grid_result(grid);
    }
    if tail.starts_with('"') {
        return quoted_text(tail).map(|t| Shown::Text(t.to_string()));
    }
    value_result(tail).map(Shown::One)
}

/// A text in straight quotes (`"OK"`), without them; `None` when there is anything else, or a
/// quote or more characters inside than a text holds.
fn quoted_text(text: &str) -> Option<&str> {
    let inner = text.strip_prefix('"')?.strip_suffix('"')?;
    (!inner.contains('"') && inner.chars().take(MAX_TEXT_CHARS + 1).count() <= MAX_TEXT_CHARS).then_some(inner)
}

/// A number and its unit (`600 "psi"`, `1.5×〖10〗^(-5)`) read back.
fn value_result(text: &str) -> Option<Formatted> {
    let text = text.replace(TEN, "10");
    let text = text.as_str();
    let (number, unit) = match text.strip_suffix('"') {
        Some(rest) => {
            let (number, unit) = rest.rsplit_once(" \"")?;
            (number, unit.to_string())
        }
        None => (text, String::new()),
    };
    let number = number.replace("^(-", "^-").replace(')', "");
    let numeric = !number.is_empty() && number.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '×' | '^' | 'E' | '+' | '/' | ' '));
    (numeric && number.chars().any(|c| c.is_ascii_digit()) && !unit.contains('"')).then_some(Formatted { number, unit })
}

/// A vector or matrix result read back from the text after its `[■(`: the cells to the first `)]`
/// outside a quoted unit (`@` between rows, `&` between cells), then the shared unit and the full
/// size of a cut result, each quoted. Stops as soon as there are more rows or cells than
/// [`rendered`] writes.
fn grid_result(text: &str) -> Option<Shown> {
    // The cells shown, the cut row and the cut column.
    let max_cells = (MAX_SHOWN_ROWS + 1) * (MAX_SHOWN_COLS + 1);
    let mut rows: Vec<Vec<&str>> = vec![Vec::new()];
    let (mut start, mut cells, mut quoted, mut end) = (0, 0usize, false, None);
    for (i, c) in text.char_indices() {
        match c {
            '"' => quoted = !quoted,
            _ if quoted => {}
            '@' | '&' | ')' => {
                let rest = text.get(i..)?;
                if c == ')' && !rest.starts_with(")]") {
                    continue;
                }
                rows.last_mut()?.push(text.get(start..i)?);
                cells += 1;
                if cells > max_cells {
                    return None;
                }
                start = i + c.len_utf8();
                if c == ')' {
                    end = Some(i + ")]".len());
                    break;
                }
                if c == '@' {
                    if rows.len() > MAX_SHOWN_ROWS {
                        return None;
                    }
                    rows.push(Vec::new());
                }
            }
            _ => {}
        }
    }
    let notes = quoted_words(text.get(end?..)?)?;
    let (unit, (total_rows, total_cols)) = match notes.as_slice() {
        [] => (None, (None, None)),
        [only] => size_note(only).map_or((Some(*only), (None, None)), |n| (None, n)),
        [unit, size] => (Some(*unit), size_note(size)?),
        _ => return None,
    };
    if unit.is_some_and(|u| u.is_empty() || size_note(u).is_some()) {
        return None;
    }
    let width = rows.first()?.len();
    if rows.iter().any(|r| r.len() != width) {
        return None;
    }
    // The cut row and column are exactly where the size says.
    let cols = width.checked_sub(usize::from(total_cols.is_some()))?;
    if total_rows.is_some() {
        let cut = rows.pop()?;
        let (marks, corner) = cut.split_at_checked(cols)?;
        if marks.iter().any(|&c| c != CUT) || corner.iter().any(|&c| c != CORNER) {
            return None;
        }
    }
    if total_cols.is_some() {
        for row in &mut rows {
            if row.pop()? != COL_CUT {
                return None;
            }
        }
    }
    let shown = rows.len();
    if shown == 0 || shown > MAX_SHOWN_ROWS || cols == 0 || cols > MAX_SHOWN_COLS {
        return None;
    }
    let total_rows = total_rows.map_or(Some(shown), |n| (n > shown).then_some(n))?;
    let total_cols = total_cols.map_or(Some(cols), |n| (n > cols).then_some(n))?;
    if total_rows.checked_mul(total_cols).is_none_or(|all| all > MAX_ELEMENTS) {
        return None;
    }
    // Cells of text (quoted) take no unit; cells of numbers are all numbers.
    if rows.first().and_then(|r| r.first()).is_some_and(|c| c.starts_with('"')) {
        if unit.is_some() {
            return None;
        }
        let items = rows.into_iter().flatten().map(|c| quoted_text(c).map(str::to_string)).collect::<Option<Vec<_>>>()?;
        return Some(Shown::TextGrid { rows: shown, cols, items, total_rows, total_cols });
    }
    let items = rows.into_iter().flatten().map(value_result).collect::<Option<Vec<_>>>()?;
    if unit.is_some() && items.iter().any(|f| !f.unit.is_empty()) {
        return None;
    }
    Some(Shown::Grid { rows: shown, cols, items, unit: unit.map(str::to_string), total_rows, total_cols })
}

/// The full size of a cut result: `(60 rows)`, `(10000 columns)` or `(60 rows × 60 columns)`, as
/// (rows, columns), each `None` when not cut. `None` for anything else.
fn size_note(text: &str) -> Option<(Option<usize>, Option<usize>)> {
    let inner = text.strip_prefix('(')?.strip_suffix(')')?;
    let count = |t: &str, what: &str| {
        let n = t.strip_suffix(what)?;
        if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        n.parse::<usize>().ok()
    };
    match inner.split_once(" × ") {
        Some((r, c)) => Some((Some(count(r, " rows")?), Some(count(c, " columns")?))),
        None => match count(inner, " rows") {
            Some(n) => Some((Some(n), None)),
            None => Some((None, Some(count(inner, " columns")?))),
        },
    }
}

/// The quoted words of ` "a" "b"`: `None` for anything else, or more than two.
fn quoted_words(mut text: &str) -> Option<Vec<&str>> {
    let mut words = Vec::new();
    while !text.is_empty() {
        let (word, rest) = text.strip_prefix(" \"")?.split_once('"')?;
        if words.len() == 2 {
            return None;
        }
        words.push(word);
        text = rest;
    }
    Some(words)
}

/// A formatted number as linear text: the base `10` is in invisible brackets (`〖10〗^6`) so the
/// power is on all of it, and a negative exponent is parenthesised (`^(-5)`), so the whole
/// exponent is raised.
fn linear_number(number: &str) -> String {
    let number = number.replace("×10^", &format!("×{TEN}^"));
    match number.split_once("^-") {
        Some((mantissa, exp)) => format!("{mantissa}^(-{exp})"),
        None => number,
    }
}

/// The digits 0–9 raised.
const SUPERSCRIPT_DIGITS: [char; 10] = ['⁰', '¹', '²', '³', '⁴', '⁵', '⁶', '⁷', '⁸', '⁹'];

/// A formatted number as plain text, for a quoted value in a sentence: the exponent of
/// `m×10^e` in superscript characters (`2.9×10⁷`, `1.5×10⁻⁵`), so no markup reaches the text.
pub(crate) fn text_number(number: &str) -> String {
    let Some((mantissa, exp)) = number.split_once("×10^") else { return number.to_string() };
    let raised: Option<String> = exp
        .chars()
        .map(|c| match c {
            '-' => Some('⁻'),
            _ => c.to_digit(10).and_then(|d| SUPERSCRIPT_DIGITS.get(usize::try_from(d).ok()?).copied()),
        })
        .collect();
    match raised {
        Some(raised) if !raised.is_empty() => format!("{mantissa}×10{raised}"),
        _ => number.to_string(),
    }
}

/// A text in straight quotes as one upright run.
fn text_run(text: &str) -> MRun {
    unit_run(format!("\"{text}\""))
}

/// An upright run of unit text.
fn unit_run(text: String) -> MRun {
    MRun { text, sty: Some(MSty::Plain), ..MRun::default() }
}

/// Every run in `arg`, at any depth, in `colour`.
fn paint(arg: Arg, colour: Rgb) -> Arg {
    arg.into_iter()
        .map(|n| match n {
            MNode::Run(mut r) => {
                r.color = Some(colour);
                MNode::Run(r)
            }
            other => map_args(other, &|a| paint(a, colour)),
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Statement → linear text
// ---------------------------------------------------------------------------------------------

/// What a sentinel stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    /// A unit: upright, set apart from a number before it.
    Unit,
    /// A name or part of one, in the default math style (letters italic).
    Name,
    /// A built-in function name or a multi-letter subscript: upright.
    Upright,
    /// A subtree built apart (a matrix literal, cell by cell): the index of its node in
    /// [`Out::nodes`].
    Node(usize),
    /// A text literal, quotes included: upright, and never read as mathematics.
    Text,
}

/// A piece of text a sentinel stands for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Piece {
    text: String,
    kind: Kind,
}

/// The text being built: `linear` is what is stored and shown; `parsed` is the same text with
/// each unit and each multi-letter name replaced by a private-use sentinel, which
/// [`parse_linear`] keeps as one atom. So a unit like `m/s²` or `lbf·in` never becomes a fraction
/// or product, and a name like `asin`, `xmax` or `f_max` never has a function name (`sin`, `max`)
/// found inside it and applied to what follows.
#[derive(Default)]
struct Out {
    linear: String,
    parsed: String,
    pieces: Vec<Piece>,
    sentinels: HashMap<Piece, char>,
    /// Subtrees built apart, each standing in `parsed` as one [`Kind::Node`] sentinel.
    nodes: Vec<MNode>,
}

impl Out {
    fn text(&mut self, s: &str) {
        self.linear.push_str(s);
        self.parsed.push_str(s);
    }

    fn unit(&mut self, unit: &str) {
        self.linear.push_str(&format!("\"{unit}\""));
        self.sentinel(Piece { text: unit.to_string(), kind: Kind::Unit }, &format!("\"{unit}\""));
    }

    /// A text literal: in straight quotes, one atom for the linear parser so nothing inside it is
    /// read as mathematics.
    fn text_literal(&mut self, text: &str) {
        let quoted = format!("\"{text}\"");
        self.linear.push_str(&quoted);
        self.sentinel(Piece { text: text.to_string(), kind: Kind::Text }, &quoted);
    }

    /// A comparison's `==`: written as two characters in the linear text, drawn as one bold `=` so
    /// it is never mistaken for the `=` that shows a result.
    fn equals(&mut self) {
        self.linear.push_str("==");
        let k = self.nodes.len();
        self.nodes.push(bold_equality());
        self.sentinel(Piece { text: k.to_string(), kind: Kind::Node(k) }, "==");
    }

    /// Name text: a sentinel when UnicodeMath could read part of it as a function name.
    fn name_piece(&mut self, text: &str, kind: Kind) {
        self.linear.push_str(text);
        if text.chars().filter(char::is_ascii_alphabetic).count() >= 2 {
            self.sentinel(Piece { text: text.to_string(), kind }, text);
        } else {
            self.parsed.push_str(text);
        }
    }

    /// Put `piece`'s sentinel into the parsed text, or `fallback` when the sentinels run out.
    fn sentinel(&mut self, piece: Piece, fallback: &str) {
        if let Some(&c) = self.sentinels.get(&piece) {
            self.parsed.push(c);
            return;
        }
        let k = self.pieces.len();
        let sentinel = (k < MAX_SENTINELS).then(|| u32::try_from(k).ok().and_then(|k| char::from_u32(SENTINEL_BASE + k))).flatten();
        match sentinel {
            Some(c) => {
                self.sentinels.insert(piece.clone(), c);
                self.pieces.push(piece);
                self.parsed.push(c);
            }
            None => self.parsed.push_str(fallback),
        }
    }

    /// A matrix literal, `cols` items to a row: a Word matrix in brackets, `&` between columns and
    /// `@` between rows (`[■(1&2@3&4)]`). Its tree is built cell by cell, each cell parsed alone,
    /// and stands in the parsed text as one sentinel: the linear parser bounds its work, and a
    /// long literal parsed whole would lose its last elements to that bound. A literal inside an
    /// element recurses here; the parser bounds that nesting (64 deep).
    fn matrix(&mut self, cols: usize, items: &[Expr]) {
        let mut text = String::from("[■(");
        let mut rows: Vec<Vec<Arg>> = Vec::new();
        for (k, item) in items.iter().enumerate() {
            let new_row = k.checked_rem(cols) == Some(0);
            if k > 0 {
                text.push(if new_row { '@' } else { '&' });
            }
            if new_row || rows.is_empty() {
                rows.push(Vec::new());
            }
            let cell = cell(item, &mut text);
            if let Some(row) = rows.last_mut() {
                row.push(cell);
            }
        }
        text.push_str(")]");
        self.linear.push_str(&text);
        let k = self.nodes.len();
        self.nodes.push(bracketed(rows));
        // Out of sentinels, the literal is parsed with the rest, as text.
        self.sentinel(Piece { text: k.to_string(), kind: Kind::Node(k) }, &text);
    }

    /// The names of a multiple definition: a bracketed column, one name to a row
    /// (`[■(a@b)]`). Like [`Out::matrix`], its tree is built cell by cell and stands in the
    /// parsed text as one sentinel.
    fn name_column(&mut self, names: &[String]) {
        let mut text = String::from("[■(");
        let mut rows: Vec<Vec<Arg>> = Vec::with_capacity(names.len());
        for (k, name) in names.iter().enumerate() {
            if k > 0 {
                text.push('@');
            }
            rows.push(vec![built(|o| o.name(name), &mut text)]);
        }
        text.push_str(")]");
        self.linear.push_str(&text);
        let k = self.nodes.len();
        self.nodes.push(bracketed(rows));
        self.sentinel(Piece { text: k.to_string(), kind: Kind::Node(k) }, &text);
    }

    /// A piecewise: a brace open on the left only, over a matrix of one row per case, its value
    /// then `if` and its condition, and a last row of `otherwise`'s value then `otherwise`
    /// (`{■(0.4&"if "T≤900 "°F"@0.7&"otherwise")┤`). The words are upright. Like
    /// [`Out::matrix`], the tree is built cell by cell and stands in the parsed text as one
    /// sentinel.
    fn cases(&mut self, cases: &[(Expr, Expr)], otherwise: Option<&Expr>) {
        const IF: &str = "if ";
        const OTHERWISE: &str = "otherwise";
        let word = |w: &str| MNode::Run(MRun { text: w.to_string(), sty: Some(MSty::Plain), ..MRun::default() });
        let mut text = String::from("{■(");
        let mut rows: Vec<Vec<Arg>> = Vec::with_capacity(cases.len().saturating_add(1));
        let lines = cases.iter().map(|(value, condition)| (value, Some(condition))).chain(otherwise.map(|value| (value, None)));
        for (k, (value, condition)) in lines.enumerate() {
            if k > 0 {
                text.push('@');
            }
            let value = cell(value, &mut text);
            text.push('&');
            let mut when = Vec::new();
            match condition {
                Some(condition) => {
                    text.push_str(&format!("\"{IF}\""));
                    when.push(word(IF));
                    when.extend(cell(condition, &mut text));
                }
                None => {
                    text.push_str(&format!("\"{OTHERWISE}\""));
                    when.push(word(OTHERWISE));
                }
            }
            rows.push(vec![value, when]);
        }
        text.push_str(")┤");
        self.linear.push_str(&text);
        let k = self.nodes.len();
        let matrix = MNode::Matrix { rows, col_jc: vec![ColJc::Left, ColJc::Left] };
        self.nodes.push(MNode::Delim { beg: Some('{'), end: None, sep: None, grow: true, shp_match: false, elems: vec![vec![matrix]] });
        self.sentinel(Piece { text: k.to_string(), kind: Kind::Node(k) }, &text);
    }

    /// A sum: a real ∑ with its limits (`i=first` or `i=first,second` below, `last` above) and
    /// the summed term grouped after it (`∑_(i=1)^n▒〖a_i·b_i〗`), so what the term reaches is
    /// visible. Like [`Out::matrix`], the tree is built part by part and stands in the parsed
    /// text as one sentinel; a sum inside the term recurses here, and the parser bounds that
    /// nesting.
    fn sum(&mut self, index: &str, ends: (&Expr, Option<&Expr>, &Expr), body: &Expr) {
        let (first, second, last) = ends;
        let mut text = String::from("∑_(");
        let sub = built(
            |o| {
                o.name(index);
                o.text("=");
                write_end(first, o);
                if let Some(second) = second {
                    o.text(",");
                    write_end(second, o);
                }
            },
            &mut text,
        );
        text.push_str(")^");
        let parens = !exponent_simple(last);
        if parens {
            text.push('(');
        }
        let sup = built(|o| write_expr(last, o), &mut text);
        if parens {
            text.push(')');
        }
        text.push_str("▒〖");
        // A term that is a sum of terms, a comparison or looser keeps visible parentheses: the
        // invisible grouping alone would show `∑(i = 1..n) (a[i] + 1)` as `∑ a_i+1`, which reads
        // as `∑(i = 1..n) a[i] + 1`.
        let grouped = prec(body) < PRODUCT;
        let e = built(
            |o| {
                o.text(if grouped { "(" } else { "" });
                write_expr(body, o);
                o.text(if grouped { ")" } else { "" });
            },
            &mut text,
        );
        text.push('〗');
        self.linear.push_str(&text);
        let k = self.nodes.len();
        self.nodes.push(MNode::Nary { chr: '∑', lim_loc: None, grow: false, sub_hide: false, sup_hide: false, sub, sup, e });
        self.sentinel(Piece { text: k.to_string(), kind: Kind::Node(k) }, &text);
    }

    /// A name that a subscript index follows: in invisible brackets when it has a subscript of
    /// its own (`〖D_o〗_i`), so the index lowers the whole name instead of extending its subscript.
    fn index_base(&mut self, name: &str) {
        if name.contains('_') {
            self.text(INVISIBLE_OPEN);
            self.name(name);
            self.text(INVISIBLE_CLOSE);
        } else {
            self.name(name);
        }
    }

    /// A name with its subscripts: `σ_allow` is σ with subscript allow; each further `_`
    /// lowers the rest again, in parentheses (`a_(b_(c))`) so all of it is lowered.
    ///
    /// An empty part (`a__b`, `x_`) shows as nothing: no `_` is left without a subscript after
    /// it, which UnicodeMath would hang on whatever follows (`x_≔3` lowering `≔3`).
    fn name(&mut self, name: &str) {
        let mut parts = name.split('_');
        let base = parts.next().unwrap_or_default();
        self.name_piece(base, Kind::Name);
        let subs: Vec<&str> = parts.filter(|p| !p.is_empty()).collect();
        match subs.as_slice() {
            [] => {}
            [sub] => {
                self.text("_");
                self.name_piece(sub, sub_kind(sub));
            }
            _ => {
                for sub in &subs {
                    self.text("_(");
                    self.name_piece(sub, sub_kind(sub));
                }
                for _ in &subs {
                    self.text(")");
                }
            }
        }
    }
}

/// `e` written alone, its linear text added to `text`: the cell of a matrix or a piecewise, as a
/// tree. An expression inside one recurses into [`Out::matrix`] or [`Out::cases`]; the parser
/// bounds that nesting.
fn cell(e: &Expr, text: &mut String) -> Arg {
    built(|out| write_expr(e, out), text)
}

/// What `write` puts in a fresh [`Out`], as a tree, its linear text added to `text`.
fn built(write: impl FnOnce(&mut Out), text: &mut String) -> Arg {
    let mut part = Out::default();
    write(&mut part);
    text.push_str(&part.linear);
    restore_pieces(parse_linear(&part.parsed), &part.pieces, &part.nodes)
}

/// An end of a sum's range, written before a `,` or a `)` inside its lower limit: a quotient keeps
/// its parentheses, so what follows it can't run on into its denominator.
fn write_end(e: &Expr, out: &mut Out) {
    if range_end_parens(e) {
        out.text("(");
        write_expr(e, out);
        out.text(")");
    } else {
        write_expr(e, out);
    }
}

/// A subscript of two or more characters is upright (`σ_max`); a one-letter one is a symbol.
fn sub_kind(sub: &str) -> Kind {
    if sub.chars().count() >= 2 { Kind::Upright } else { Kind::Name }
}

/// One step of writing an expression, so deep trees are written without recursion.
enum Step<'a> {
    Expr(&'a Expr),
    Text(&'static str),
    Cmp(Cmp),
}

fn bold_equality() -> MNode {
    MNode::Run(MRun { text: "=".to_string(), sty: Some(MSty::Bold), ..MRun::default() })
}

fn write_stmt(stmt: &Stmt, out: &mut Out, definitions_as_equalities: bool) -> Option<usize> {
    match stmt {
        Stmt::Define { name, index, params, body, .. } => {
            match index {
                // `t[i] :=` shows as t with subscript i, like an index in an expression.
                Some(i) => {
                    out.index_base(name);
                    if i.contains('_') {
                        out.text("_(");
                        out.name(i);
                        out.text(")");
                    } else {
                        out.text("_");
                        out.name(i);
                    }
                }
                None => out.name(name),
            }
            if !params.is_empty() {
                out.text("(");
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        out.text(",");
                    }
                    out.name(p);
                }
                out.text(")");
            }
            let split = definition_operator(out, definitions_as_equalities);
            write_expr(body, out);
            return split;
        }
        Stmt::Eval { expr, .. } => write_expr(expr, out),
        Stmt::Given => out.name_piece("Given", Kind::Upright),
        // An equation shows as the comparison it is: the bold `=` of `==`, never mistaken for the
        // `=` that shows a result.
        Stmt::Constraint { lhs, rhs } => {
            write_expr(&Expr::Compare { first: Box::new(lhs.clone()), rest: vec![(Cmp::Eq, rhs.clone())] }, out);
        }
        Stmt::DefineMany { names, body, .. } => {
            out.name_column(names);
            let split = definition_operator(out, definitions_as_equalities);
            write_expr(body, out);
            return split;
        }
    }
    None
}

fn definition_operator(out: &mut Out, equal: bool) -> Option<usize> {
    if equal {
        out.linear.push_str("==");
        Some(out.parsed.len())
    } else {
        out.text("≔");
        None
    }
}

// Binding strength, loosest first: ranges and cases, `or`, `and`, `not`, comparisons, sums, products,
// negation, powers, atoms (literals and indexes among them).
const RANGE: u8 = 0;
const OR: u8 = 1;
const AND: u8 = 2;
const NOT: u8 = 3;
const COMPARE: u8 = 4;
const SUM: u8 = 5;
const PRODUCT: u8 = 6;
const NEG: u8 = 7;
const POWER: u8 = 8;
const ATOM: u8 = 9;

fn prec(e: &Expr) -> u8 {
    match e {
        Expr::Range { .. } | Expr::Cases { .. } => RANGE,
        Expr::Logic(Logic::Or, ..) => OR,
        Expr::Logic(Logic::And, ..) => AND,
        Expr::Not(_) => NOT,
        Expr::Compare { .. } => COMPARE,
        Expr::Binary(Op::Add | Op::Sub, ..) => SUM,
        Expr::Binary(Op::Mul | Op::Div, ..) | Expr::Quantity(..) => PRODUCT,
        Expr::Unary(..) => NEG,
        Expr::Binary(Op::Pow, ..) => POWER,
        Expr::Binary(Op::Neg, ..) => SUM,
        // The summed term is grouped after the ∑, so a sum is a unit wherever it stands.
        Expr::Num(_)
        | Expr::Text(_)
        | Expr::Name(_)
        | Expr::Call(..)
        | Expr::Abs(_)
        | Expr::Matrix { .. }
        | Expr::Index(..)
        | Expr::Sum { .. }
        | Expr::Solve { .. } => ATOM,
    }
}

fn is_sqrt(e: &Expr) -> bool {
    matches!(e, Expr::Call(n, args) if n == "sqrt" && args.len() == 1)
}

/// A name the evaluator reads as a unit. A variable or parameter can never have a unit's name
/// (`check_name` refuses it), so this needs no knowledge of the document.
fn is_unit(name: &str) -> bool {
    lookup(name).is_some()
}

/// A unit, or a power of one (`in^2`): written after a number with a space, not `·`.
fn is_unitish(e: &Expr) -> bool {
    match e {
        Expr::Name(n) => is_unit(n),
        Expr::Binary(Op::Pow, base, _) => matches!(&**base, Expr::Name(n) if is_unit(n)),
        _ => false,
    }
}

/// The last factor of a product.
fn last_factor(mut e: &Expr) -> &Expr {
    while let Expr::Binary(Op::Mul, _, b) = e {
        e = b;
    }
    e
}

/// A number written without an exponent.
fn plain_num(e: &Expr) -> bool {
    matches!(e, Expr::Num(v) if !exponential(*v))
}

/// Needs no parentheses as the operand of `√`.
fn sqrt_simple(e: &Expr) -> bool {
    plain_num(e) || matches!(e, Expr::Name(n) if !n.contains('_'))
}

/// Needs no parentheses as the base of `^`. A call does: `sin(θ)^2` would raise the argument
/// (UnicodeMath applies the script to a function's operand) and `f(x)^2` only the `(x)`.
fn base_simple(e: &Expr) -> bool {
    plain_num(e) || matches!(e, Expr::Name(_) | Expr::Abs(_))
}

/// Needs no parentheses as an exponent: a whole number or a name without a subscript.
fn exponent_simple(e: &Expr) -> bool {
    match e {
        Expr::Num(v) => !exponential(*v) && v.fract() == 0.0 && *v >= 0.0,
        Expr::Name(n) => !n.contains('_'),
        _ => false,
    }
}

/// Needs no parentheses as the numerator or denominator of `/` (a call gets them, which the
/// fraction drops, so its whole application is the operand).
fn fraction_simple(e: &Expr) -> bool {
    plain_num(e) || matches!(e, Expr::Name(_) | Expr::Abs(_))
}

/// Needs no parentheses as a subscript index: a plain number or a name without a subscript.
fn index_simple(e: &Expr) -> bool {
    match e {
        Expr::Num(v) => !exponential(*v) && *v >= 0.0,
        Expr::Name(n) => !n.contains('_') && !is_unit(n),
        _ => false,
    }
}

/// Needs parentheses as a value of a range. `..` is the loosest operator, so a sum needs none
/// (`0..n-1`); a quotient does, so UnicodeMath can't run its denominator on into the `..`, and so
/// does a range built by hand inside another.
fn range_end_parens(e: &Expr) -> bool {
    matches!(e, Expr::Binary(Op::Div, ..) | Expr::Range { .. })
}

/// Whether `e` is written ending in a sub- or superscript, a square root, a fraction, or anything
/// else whose operand UnicodeMath could run on into a `..` after it: a numeric script takes every
/// letter, digit and `.` that follows (`D_0..5` would lower `0..5`), and so does a denominator
/// (`1+1/2..5` would put `2..5` under the bar). Errs towards yes.
fn ends_in_script(e: &Expr) -> bool {
    let mut e = e;
    loop {
        match e {
            Expr::Num(v) => return exponential(*v),
            Expr::Name(n) => return n.contains('_') && !is_unit(n),
            Expr::Text(_) | Expr::Quantity(..) | Expr::Abs(_) | Expr::Matrix { .. } | Expr::Cases { .. } | Expr::Sum { .. } | Expr::Solve { .. } => {
                return false;
            }
            Expr::Call(..) => return is_sqrt(e),
            Expr::Index(..) | Expr::Binary(Op::Pow | Op::Div, ..) => return true,
            Expr::Unary(_, a) | Expr::Binary(_, _, a) | Expr::Logic(_, _, a) | Expr::Not(a) => e = a,
            Expr::Compare { first, rest } => e = rest.last().map_or(&**first, |(_, last)| last),
            Expr::Range { last, .. } => e = last,
        }
    }
}

/// Push the value written just before a range's `..`: in invisible brackets when it ends in a
/// script or a fraction (`〖D_0〗..5`, `〖1+1/2〗..5`), so the `..` stays out of it.
fn push_before_dots<'a>(todo: &mut Vec<Step<'a>>, e: &'a Expr) {
    if range_end_parens(e) {
        wrap(todo, e, true);
    } else if ends_in_script(e) {
        todo.push(Step::Text(INVISIBLE_CLOSE));
        todo.push(Step::Expr(e));
        todo.push(Step::Text(INVISIBLE_OPEN));
    } else {
        todo.push(Step::Expr(e));
    }
}

/// Push `e`, in parentheses when `parens`.
fn wrap<'a>(todo: &mut Vec<Step<'a>>, e: &'a Expr, parens: bool) {
    if parens {
        todo.push(Step::Text(")"));
        todo.push(Step::Expr(e));
        todo.push(Step::Text("("));
    } else {
        todo.push(Step::Expr(e));
    }
}

fn write_expr(root: &Expr, out: &mut Out) {
    // Steps are pushed last first, so they come off the stack in writing order.
    let mut todo: Vec<Step<'_>> = vec![Step::Expr(root)];
    while let Some(step) = todo.pop() {
        let e = match step {
            Step::Text(t) => {
                out.text(t);
                continue;
            }
            Step::Cmp(Cmp::Eq) => {
                out.equals();
                continue;
            }
            Step::Cmp(cmp) => {
                out.text(cmp.symbol());
                continue;
            }
            Step::Expr(e) => e,
        };
        match e {
            Expr::Num(v) => out.text(&num_text(*v)),
            Expr::Text(s) => out.text_literal(s),
            Expr::Name(n) if is_unit(n) => out.unit(n),
            Expr::Name(n) => out.name(n),
            Expr::Quantity(v, unit) => {
                out.text(&num_text(*v));
                out.text(" ");
                out.unit(unit);
            }
            Expr::Unary(Op::Neg, a) => {
                // A product or sum under `-` keeps its parentheses.
                wrap(&mut todo, a, prec(a) <= NEG);
                todo.push(Step::Text("-"));
            }
            // No other prefix operator exists (the evaluator refuses one); show the operand only.
            Expr::Unary(_, a) => wrap(&mut todo, a, true),
            Expr::Abs(a) => {
                todo.push(Step::Text("|"));
                todo.push(Step::Expr(a));
                todo.push(Step::Text("|"));
            }
            Expr::Call(_, args) if is_sqrt(e) => {
                if let Some(a) = args.first() {
                    wrap(&mut todo, a, !sqrt_simple(a));
                }
                todo.push(Step::Text("√"));
            }
            Expr::Call(name, _) if name == "worksheetData" => out.text("Worksheet data"),
            Expr::Call(name, _) if name == "worksheetProgram" => out.text("Legacy worksheet program"),
            Expr::Call(name, args) => {
                // UnicodeMath builds `sin(…)` into a function applied to its operand and drops
                // the operand's parentheses (shown as `sin θ`), so anything but a lone atom gets a
                // second pair: `sin((θ+1))` shows sin(θ+1), not (sin θ)+1. Other names are
                // sentinels, so their parentheses stay as written.
                let applied = FUNCTION_NAMES.contains(&name.as_str());
                let lone = matches!(args.as_slice(), [a] if plain_num(a) || matches!(a, Expr::Name(n) if !n.contains('_') && !is_unit(n)));
                let doubled = applied && !lone;
                todo.push(Step::Text(if doubled { "))" } else { ")" }));
                for (i, a) in args.iter().enumerate().rev() {
                    todo.push(Step::Expr(a));
                    if i > 0 {
                        todo.push(Step::Text(","));
                    }
                }
                todo.push(Step::Text(if doubled { "((" } else { "(" }));
                if applied {
                    out.text(name);
                } else if funcs::is_builtin(name) || funcs::is_solver_name(name) || name == "if" {
                    out.name_piece(name, Kind::Upright);
                } else {
                    out.name(name);
                }
            }
            Expr::Compare { first, rest } => {
                // Every operand is a sum or tighter; a comparison, `not`, `and` or `or` inside one
                // keeps its parentheses.
                for (cmp, operand) in rest.iter().rev() {
                    wrap(&mut todo, operand, prec(operand) <= COMPARE);
                    todo.push(Step::Cmp(*cmp));
                }
                wrap(&mut todo, first, prec(first) <= COMPARE);
            }
            Expr::Logic(logic, a, b) => {
                // Left to right, like a sum: the same connective on the right keeps its parentheses.
                let level = if *logic == Logic::And { AND } else { OR };
                wrap(&mut todo, b, prec(b) <= level);
                todo.push(Step::Text(if *logic == Logic::And { "∧" } else { "∨" }));
                wrap(&mut todo, a, prec(a) < level);
            }
            Expr::Not(a) => {
                // A comparison keeps its parentheses, though `not` binds looser: `¬a<3` would read
                // as `(¬a)<3`. So does a looser `and` or `or`.
                wrap(&mut todo, a, prec(a) <= COMPARE);
                todo.push(Step::Text("¬"));
            }
            Expr::Matrix { cols, items, .. } => out.matrix(*cols, items),
            Expr::Cases { cases, otherwise } => out.cases(cases, otherwise.as_deref()),
            Expr::Sum { index, first, second, last, body } => out.sum(index, (first, second.as_deref(), last), body),
            // Shown as the call it was written as, `Find(x, y)`: its equations are above it.
            Expr::Solve { kind, unknowns, .. } => write_solve(*kind, unknowns, out),
            Expr::Index(..) | Expr::Range { .. } => push_array_syntax(e, &mut todo, out),
            Expr::Binary(op, a, b) => match op {
                Op::Add | Op::Sub | Op::Neg => {
                    wrap(&mut todo, b, prec(b) <= SUM || matches!(&**b, Expr::Unary(..)));
                    todo.push(Step::Text(if *op == Op::Add { "+" } else { "-" }));
                    // A comparison, `not`, `and` or `or` is looser than a sum, so on the left it
                    // keeps its parentheses too: `(a < b) + 1`.
                    wrap(&mut todo, a, prec(a) < SUM);
                }
                Op::Mul => {
                    // A quotient on the right needs no parentheses: UnicodeMath's numerator stops
                    // at the `·`, so `c·a/b` reads as c·(a/b), as it was evaluated.
                    let right_parens = prec(b) < PRODUCT || matches!(&**b, Expr::Binary(Op::Mul, ..) | Expr::Quantity(..) | Expr::Unary(..));
                    wrap(&mut todo, b, right_parens);
                    let spaced = is_unitish(b) && !is_unitish(last_factor(a)) && !right_parens;
                    todo.push(Step::Text(if spaced { " " } else { "·" }));
                    wrap(&mut todo, a, prec(a) < PRODUCT);
                }
                // The parentheses around a fraction's operands are dropped when it is built up.
                Op::Div => {
                    wrap(&mut todo, b, !fraction_simple(b));
                    todo.push(Step::Text("/"));
                    wrap(&mut todo, a, !fraction_simple(a));
                }
                Op::Pow => {
                    wrap(&mut todo, b, !exponent_simple(b));
                    todo.push(Step::Text("^"));
                    if base_simple(a) && matches!(&**a, Expr::Num(_)) && num_text_len(a) > 1 {
                        // UnicodeMath raises only a number's last digit (`25^2` is 2·5²); invisible
                        // brackets make the whole number the base.
                        todo.push(Step::Text(INVISIBLE_CLOSE));
                        todo.push(Step::Expr(a));
                        todo.push(Step::Text(INVISIBLE_OPEN));
                    } else {
                        wrap(&mut todo, a, !base_simple(a));
                    }
                }
            },
        }
    }
}

/// A resolved solve block as the call it came from: the upright `Find` or `Minerr` and its unknowns.
fn write_solve(kind: SolveKind, unknowns: &[String], out: &mut Out) {
    out.name_piece(kind.name(), Kind::Upright);
    out.text("(");
    for (k, unknown) in unknowns.iter().enumerate() {
        if k > 0 {
            out.text(",");
        }
        out.name(unknown);
    }
    out.text(")");
}

/// Push the steps of an index or a range (`e` is one of them). The name of an index is written at
/// once, before the steps.
fn push_array_syntax<'a>(e: &'a Expr, todo: &mut Vec<Step<'a>>, out: &mut Out) {
    match e {
        // An index is a subscript: `D_2`, `M_(i,j)`, `〖D_o〗_i`.
        Expr::Index(name, at) => {
            match at.as_slice() {
                [a] if index_simple(a) => todo.push(Step::Expr(a)),
                _ => {
                    todo.push(Step::Text(")"));
                    push_separated(todo, at, |_| ",");
                    todo.push(Step::Text("("));
                }
            }
            todo.push(Step::Text("_"));
            out.index_base(name);
        }
        Expr::Range { first, second, last } => {
            wrap(todo, last, range_end_parens(last));
            todo.push(Step::Text(".."));
            match second {
                Some(second) => {
                    push_before_dots(todo, second);
                    todo.push(Step::Text(","));
                    wrap(todo, first, range_end_parens(first));
                }
                None => push_before_dots(todo, first),
            }
        }
        _ => {}
    }
}

/// Push `items` with `sep(k)` before each item `k` after the first.
fn push_separated<'a>(todo: &mut Vec<Step<'a>>, items: &'a [Expr], sep: impl Fn(usize) -> &'static str) {
    for (k, item) in items.iter().enumerate().rev() {
        todo.push(Step::Expr(item));
        if k > 0 {
            todo.push(Step::Text(sep(k)));
        }
    }
}

/// Whether a number is written as `m×10^e` (too long otherwise).
fn exponential(v: f64) -> bool {
    let a = v.abs();
    a != 0.0 && (a >= 1e15 || a < 1e-6)
}

/// A number as the source would write it: shortest round-trip digits, `m×10^e` when very large
/// or small.
fn num_text(v: f64) -> String {
    if !exponential(v) {
        return format!("{v}");
    }
    let repr = format!("{v:e}");
    match repr.split_once('e') {
        Some((mantissa, exp)) if exp.starts_with('-') => format!("{mantissa}×{TEN}^({exp})"),
        Some((mantissa, exp)) => format!("{mantissa}×{TEN}^{exp}"),
        None => repr,
    }
}

/// Characters in a number's text.
fn num_text_len(e: &Expr) -> usize {
    match e {
        Expr::Num(v) => num_text(*v).chars().count(),
        _ => 0,
    }
}

// ---------------------------------------------------------------------------------------------
// Sentinels → runs
// ---------------------------------------------------------------------------------------------

fn sentinel_piece(c: char, pieces: &[Piece]) -> Option<&Piece> {
    let k = usize::try_from((c as u32).checked_sub(SENTINEL_BASE)?).ok()?;
    pieces.get(k)
}

fn is_unit_sentinel(c: char, pieces: &[Piece]) -> bool {
    sentinel_piece(c, pieces).is_some_and(|p| p.kind == Kind::Unit)
}

/// Whether a unit after `prev` needs a thin space to stand apart from it (`1500 lbf`, `len m`).
fn needs_space(prev: Option<char>, pieces: &[Piece]) -> bool {
    prev.is_some_and(|c| c.is_alphanumeric() || c == ')' || sentinel_piece(c, pieces).is_some_and(|p| p.kind != Kind::Unit))
}

/// The run a sentinel turns back into, after `prev` (in the same run), coloured like its run.
fn piece_run(piece: &Piece, prev: Option<char>, pieces: &[Piece], template: &MRun) -> MRun {
    match piece.kind {
        Kind::Unit => {
            let space = if needs_space(prev, pieces) { THIN_SPACE.to_string() } else { String::new() };
            MRun { color: template.color, ..unit_run(format!("{space}{}", piece.text)) }
        }
        Kind::Name => MRun { text: piece.text.clone(), ..template.clone() },
        Kind::Upright => MRun { text: piece.text.clone(), sty: Some(MSty::Plain), ..template.clone() },
        // Replaced by its node in `restore_pieces`; never a run.
        Kind::Node(_) => MRun { text: String::new(), ..template.clone() },
        Kind::Text => MRun { color: template.color, ..text_run(&piece.text) },
    }
}

/// Replace each sentinel in the tree with a run of its text, and take a script's base out of the
/// invisible brackets that kept it whole. The tree comes from
/// [`parse_linear`], which caps its depth, so recursing over it is bounded.
fn restore_pieces(arg: Arg, pieces: &[Piece], nodes: &[MNode]) -> Arg {
    let mut out: Arg = Vec::with_capacity(arg.len());
    for node in arg {
        match node {
            MNode::Run(r) if r.text.chars().any(|c| sentinel_piece(c, pieces).is_some()) => {
                let mut buf = String::new();
                // A unit straight after a bracket (`[■(…)] "in"`, `(a+b) "m"`) stands apart from it.
                let mut prev: Option<char> = matches!(out.last(), Some(MNode::Delim { .. })).then_some(')');
                for c in r.text.chars() {
                    match sentinel_piece(c, pieces) {
                        Some(piece) => {
                            if !buf.is_empty() {
                                out.push(MNode::Run(MRun { text: std::mem::take(&mut buf), ..r.clone() }));
                            }
                            match piece.kind {
                                Kind::Node(k) => out.extend(nodes.get(k).cloned()),
                                _ => out.push(MNode::Run(piece_run(piece, prev, pieces, &r))),
                            }
                        }
                        None => buf.push(c),
                    }
                    prev = Some(c);
                }
                if !buf.is_empty() {
                    out.push(MNode::Run(MRun { text: buf, ..r }));
                }
            }
            MNode::Script { kind, base, sub, sup } => {
                // A unit raised to a power right after a number: `2 in^2`.
                let unit_base = matches!(base.first(), Some(MNode::Run(r)) if r.text.chars().next().is_some_and(|c| is_unit_sentinel(c, pieces)));
                let prev = match out.last() {
                    Some(MNode::Run(r)) => r.text.chars().last(),
                    _ => None,
                };
                let mut base = restore_pieces(base, pieces, nodes);
                // A base in invisible brackets (`〖25〗^2`) is the bracketed content itself.
                if let [MNode::Delim { beg: None, end: None, elems, .. }] = base.as_mut_slice()
                    && elems.len() == 1
                    && let Some(inner) = elems.pop()
                {
                    base = inner;
                }
                if unit_base
                    && needs_space(prev, pieces)
                    && let Some(MNode::Run(r)) = base.first_mut()
                {
                    r.text.insert(0, THIN_SPACE);
                }
                out.push(MNode::Script { kind, base, sub: restore_pieces(sub, pieces, nodes), sup: restore_pieces(sup, pieces, nodes) });
            }
            other => out.push(map_args(other, &|a| restore_pieces(a, pieces, nodes))),
        }
    }
    out
}

#[cfg(test)]
mod tests;
