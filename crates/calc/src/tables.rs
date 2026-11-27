//! Document tables as calculation data: reading a marked table's headings and cells, and the
//! definition each column makes.
//!
//! Row 1 holds the headings, `Name (unit)` or `Name` alone for plain numbers; each column defines
//! the column vector `Name` of the values below its heading, in that unit. A cell holds a number, a
//! fraction (`3/4`) or a mixed number (`1-1/2`, `1 1/2`: the pipe-size convention), optionally
//! followed by a unit of the heading's dimension. Cells are not calculations: anything else is an
//! error naming the column and row, never a guess (`1-1/2` is never 0.5, a blank is never 0).
//!
//! A heading `Name (text)` makes a text column instead: each cell is its text, trimmed (a number
//! there is text, `"6"`), and may not be empty, hold a quote mark or run past 1,000 characters.
//! Every other column stays strictly numeric.
//!
//! Table text and structure come from files, so both are hostile: a column holds at most
//! [`MAX_ELEMENTS`] values, a table at most [`MAX_COLS`] columns, and nothing here panics.

use craft_units::parse_unit;
use wordcraft_doc::para::OBJ;
use wordcraft_doc::props::VMerge;
use wordcraft_doc::table::MAX_COLS;
use wordcraft_doc::{Block, Cell, Paragraph, Table};

use crate::array::{MAX_ELEMENTS, MAX_TEXT_CHARS};
use crate::parse::is_keyword;
use crate::{CalcError, Env, Expr, Stmt, check_name, defined_name, parse};

fn syntax(message: impl Into<String>) -> CalcError {
    CalcError::Syntax(message.into())
}

/// What a column holds, by its heading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ColumnKind {
    /// Plain numbers: `t_nom`.
    Plain,
    /// Numbers in a unit: `D_o (in)` is `Unit("in")`.
    Unit(String),
    /// Texts: `Material (text)`, the unit `text` in any case.
    Text,
}

/// A heading read as a name and what its column holds: `"D_o (in)"` is `("D_o", Unit("in"))`,
/// `"t_nom"` is `("t_nom", Plain)` and `"Material (text)"` is `("Material", Text)`. The name must
/// be a valid calculation name that isn't a unit, function or constant, and the unit must read.
/// Whether `g` in the unit means gravity is the caller's check.
pub(crate) fn heading(text: &str) -> Result<(String, ColumnKind), CalcError> {
    let text = text.trim();
    let name = heading_name(text)?;
    let kind = match text.split_once('(') {
        Some((_, rest)) => {
            let unit = rest.strip_suffix(')').map(str::trim).filter(|u| !u.is_empty());
            let Some(unit) = unit else {
                return Err(syntax(format!("the heading {text} needs a unit in brackets at its end, like D_o (in)")));
            };
            if unit.eq_ignore_ascii_case("text") {
                ColumnKind::Text
            } else {
                parse_unit(unit)?;
                ColumnKind::Unit(unit.to_string())
            }
        }
        None => ColumnKind::Plain,
    };
    Ok((name.to_string(), kind))
}

/// The name a heading gives (the text before any `(`), when it is a valid calculation name that
/// isn't a unit, function or constant; whatever the unit after it says.
fn heading_name(text: &str) -> Result<&str, CalcError> {
    let name = text.split_once('(').map_or(text, |(name, _)| name).trim();
    if name.is_empty() {
        return Err(syntax("a calculation data column needs a heading: a name, and a unit in brackets, like D_o (in)"));
    }
    // A keyword is reported as one, not as a name the parser can't read.
    if is_keyword(name) {
        return Err(CalcError::Keyword(name.to_string()));
    }
    // The name as the calculation parser reads it: anything but a single plain name is refused.
    let parsed = parse(&format!("{name} := 0")).ok();
    let plain = matches!(&parsed, Some(stmt @ Stmt::Define { index: None, params, .. }) if params.is_empty() && defined_name(stmt) == Some(name));
    if !plain {
        return Err(syntax(format!("{name} isn't a valid name for a calculation data column")));
    }
    check_name(name)?;
    Ok(name)
}

/// What a text column's cell holds: its text, which `cell_text` has trimmed. Empty, holding a
/// quote mark (`"`, `“` or `”`: a text can't) or too long is an error saying so, to follow the column's name and row.
fn text_cell(text: &str) -> Result<String, String> {
    if text.is_empty() {
        return Err("is empty".to_string());
    }
    // Any of the three: a typed `"` becomes `”` under smart quotes, and no literal could match it.
    if let Some(mark) = text.chars().find(|c| matches!(c, '"' | '“' | '”')) {
        return Err(format!("has a quote mark {mark}, which text can't hold"));
    }
    if text.chars().nth(MAX_TEXT_CHARS).is_some() {
        return Err("is longer than 1,000 characters".to_string());
    }
    Ok(text.to_string())
}

/// What a data cell's text means: a number and the unit written after it, if any. The number is a
/// decimal (`0.840`, `-3`, `1e3`), a fraction (`3/4`) or a mixed number (`1-1/2` or `1 1/2`, both
/// 1.5; `-1-1/2` is -1.5). Anything else is an error saying why (`isn't a number`), to follow the
/// column's name and row. The unit is returned as written and not checked here.
pub(crate) fn cell_value(text: &str) -> Result<(f64, Option<String>), String> {
    const NOT_A_NUMBER: &str = "isn't a number";
    let text = text.trim();
    let (negative, rest) = match text.strip_prefix(['-', '−']) {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (value, rest) = number(rest)?.ok_or(NOT_A_NUMBER)?;
    let unit = rest.trim_start();
    let value = if negative { -value } else { value };
    if !value.is_finite() {
        return Err(NOT_A_NUMBER.to_string());
    }
    if unit.is_empty() {
        return Ok((value, None));
    }
    if unit.starts_with(',') && unit.chars().nth(1).is_some_and(|c| c.is_ascii_digit()) {
        return Err("isn't a number — write it without a thousands separator".to_string());
    }
    // A unit starts with a letter (`in`, `µm`, `Ω`), a degree sign or `%`, or is a difference
    // (`Δ°F`). Anything else after the number — a digit, a sign, a slash — is not a number.
    let starts_unit = unit.chars().next().is_some_and(|c| c.is_alphabetic() || matches!(c, '°' | '%' | '∆'));
    if !starts_unit {
        return Err(NOT_A_NUMBER.to_string());
    }
    Ok((value, Some(unit.to_string())))
}

/// The ASCII digits at the start of `s`, and the rest.
fn digits(s: &str) -> (&str, &str) {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    s.split_at(end)
}

/// The unsigned number at the start of `s` and the rest of `s`; `None` when `s` doesn't start with
/// one. An error for a fraction dividing by zero.
fn number(s: &str) -> Result<Option<(f64, &str)>, String> {
    let (whole, after) = digits(s);
    // A whole number followed by a fraction: `3/4`, or a mixed number `1-1/2` / `1 1/2`.
    if !whole.is_empty() {
        if let Some((fraction, rest)) = after.strip_prefix('/').and_then(|f| fraction_rest(whole, f)) {
            return fraction.map(|v| Some((v, rest)));
        }
        if let Some(tail) = after.strip_prefix(['-', ' ', '\u{a0}']) {
            let (num, slash) = digits(tail);
            if !num.is_empty()
                && let Some((fraction, rest)) = slash.strip_prefix('/').and_then(|f| fraction_rest(num, f))
            {
                let whole: f64 = whole.parse().map_err(|_| "isn't a number".to_string())?;
                return fraction.map(|v| Some((whole + v, rest)));
            }
        }
    }
    // A decimal: digits, then `.` and digits, then an exponent.
    let (frac, after) = match after.strip_prefix('.') {
        Some(f) => digits(f),
        None => ("", after),
    };
    let has_point = s.len() - after.len() > whole.len() + frac.len();
    if (whole.is_empty() && frac.is_empty()) || (has_point && frac.is_empty()) {
        return Ok(None);
    }
    let mut end = s.len() - after.len();
    if let Some(e) = after.strip_prefix(['e', 'E']) {
        let signed = e.strip_prefix(['-', '+']).unwrap_or(e);
        let (exp, _) = digits(signed);
        if !exp.is_empty() {
            end += 1 + (e.len() - signed.len()) + exp.len();
        }
    }
    let (num, rest) = s.split_at(end);
    let value: f64 = num.parse().map_err(|_| "isn't a number".to_string())?;
    Ok(Some((value, rest)))
}

/// The fraction `num/den` where `f` starts with the denominator's digits, and the rest after them;
/// `None` when no digits follow the slash.
fn fraction_rest<'a>(num: &str, f: &'a str) -> Option<(Result<f64, String>, &'a str)> {
    let (den, rest) = digits(f);
    if den.is_empty() {
        return None;
    }
    let value = match (num.parse::<f64>(), den.parse::<f64>()) {
        (Ok(_), Ok(0.0)) => Err("divides by zero".to_string()),
        (Ok(n), Ok(d)) => Ok(n / d),
        _ => Err("isn't a number".to_string()),
    };
    Some((value, rest))
}

/// What a data cell (or heading) reads as: the trimmed text of its paragraphs with tracked
/// deletions left out. An error, to follow the column's name and row, for a cell holding a
/// table, an object (a calculation, picture, field…) or text in more than one paragraph.
pub(crate) fn cell_text(cell: &Cell) -> Result<String, String> {
    let mut text: Option<String> = None;
    for b in &cell.blocks {
        let p = match &**b {
            Block::Table(_) => return Err("holds a table — a data cell holds a number only".to_string()),
            Block::Para(p) => p,
        };
        if has_live_object(p) {
            return Err("holds an object — a data cell holds a number only".to_string());
        }
        let t = p.final_text();
        let t = t.trim();
        if t.is_empty() {
            continue;
        }
        if text.is_some() {
            return Err("has more than one paragraph — a data cell holds one number".to_string());
        }
        text = Some(t.to_string());
    }
    Ok(text.unwrap_or_default())
}

/// Whether `p` holds an object that isn't tracked-deleted.
fn has_live_object(p: &Paragraph) -> bool {
    if p.objects.is_empty() {
        return false;
    }
    let deleted = p.deleted_ranges();
    p.text.char_indices().any(|(i, c)| c == OBJ && !deleted.iter().any(|r| r.contains(&i)))
}

/// The name column `col`'s heading gives, whenever its name part reads — whatever its unit says
/// and whatever else the cell holds: what a column in error still owns, so the calculations using
/// it say the table has an error (and a later definition of the name is a duplicate). A name that
/// is a unit, function or constant counts: its uses fail rather than quietly meaning the unit.
pub(crate) fn column_name(t: &Table, col: u32) -> Option<String> {
    let cell = t.rows.first()?.cells.get(usize::try_from(col).ok()?)?;
    // The first paragraph with text, its objects and tracked deletions left out.
    let text = cell.blocks.iter().find_map(|b| match &**b {
        Block::Para(p) => {
            let deleted = p.deleted_ranges();
            let text: String = p.text.char_indices().filter(|&(i, c)| c != OBJ && !deleted.iter().any(|r| r.contains(&i))).map(|(_, c)| c).collect();
            Some(text).filter(|t| !t.trim().is_empty())
        }
        Block::Table(_) => None,
    })?;
    match heading_name(text.trim()) {
        Ok(name) => Some(name.to_string()),
        Err(CalcError::ReservedName(name)) => Some(name),
        Err(_) => None,
    }
}

/// Whether `cells` hold text and all of it, at any depth, is tracked-deleted: a row (or table)
/// deleted with tracking on, which the model keeps as text marked deleted.
pub(crate) fn deleted_only<'a>(cells: impl IntoIterator<Item = &'a Cell>) -> bool {
    let mut deleted = false;
    let mut todo: Vec<&Block> = cells.into_iter().flat_map(|c| c.blocks.iter().map(|b| &**b)).collect();
    while let Some(b) = todo.pop() {
        match b {
            Block::Para(p) => {
                for (range, props) in p.run_ranges() {
                    if range.is_empty() {
                        continue;
                    }
                    if props.del.is_none() {
                        return false;
                    }
                    deleted = true;
                }
            }
            Block::Table(t) => todo.extend(t.rows.iter().flat_map(|r| &r.cells).flat_map(|c| c.blocks.iter().map(|b| &**b))),
        }
    }
    deleted
}

const NO_VALUES: &str = "a calculation data table needs a row of headings and at least one row of values";

/// What is wrong with `t`'s shape as calculation data, if anything: fewer than two rows, a merged
/// cell, more than [`MAX_COLS`] columns, a heading row (repeated on each page) after the first
/// row, or a row without exactly one cell per heading.
fn shape_error(t: &Table) -> Option<CalcError> {
    let head = t.rows.first().map_or(0, |r| r.cells.len());
    if t.rows.len() < 2 || head == 0 {
        return Some(syntax(NO_VALUES));
    }
    let cells = || t.rows.iter().flat_map(|r| &r.cells);
    if cells().any(|c| c.span() > 1 || c.props.vmerge != VMerge::None) {
        return Some(syntax("a calculation data table can't have merged cells"));
    }
    if head > MAX_COLS {
        return Some(syntax(format!("a calculation data table can have at most {MAX_COLS} columns")));
    }
    if t.rows.iter().skip(1).any(|r| r.props.header) {
        return Some(syntax("only the first row of a calculation data table can be a heading row"));
    }
    if t.rows.iter().any(|r| r.cells.len() != head || r.props.grid_before > 0 || r.props.grid_after > 0) {
        return Some(syntax("every row of a calculation data table needs one cell in each column"));
    }
    None
}

/// The definition each column of the data table `t` makes, by column (counted from 0), and an
/// error for the whole table. Column `D_o (in)` over the values 0.840 and 1.050 is
/// `D_o := Matrix{2, 1}` of `Quantity(0.84, "in")` and `Quantity(1.05, "in")`. A cell with its own
/// unit keeps it (`21.3 mm` under `D_o (in)`); one of another dimension is an error. When `g` is
/// gravity, a unit spelled with `g` is refused, naming `gm`.
///
/// A row deleted with tracking on (all its text tracked-deleted) is skipped: it adds no value.
/// Rows are still named by their place in the table, so a message points at the row as shown.
///
/// When the whole table is in error (merged cells, a ragged row, no values), every column is
/// [`CalcError::UpstreamTable`], so what it defines is in error rather than undefined.
pub(crate) fn column_stmts(t: &Table, g_is_gravity: bool) -> (Vec<(u32, Result<Stmt, CalcError>)>, Option<CalcError>) {
    // The data rows that aren't tracked-deleted, by their place in the table.
    let live: Vec<usize> = t.rows.iter().enumerate().skip(1).filter(|(_, r)| !deleted_only(&r.cells)).map(|(k, _)| k).collect();
    let error = shape_error(t).or_else(|| live.is_empty().then(|| syntax(NO_VALUES)));
    let mut env = Env::new();
    env.set_g_is_gravity(g_is_gravity);
    let count = t.rows.first().map_or(0, |r| r.cells.len()).min(MAX_COLS);
    let stmts = (0..count)
        .map(|c| {
            let col = u32::try_from(c).unwrap_or(u32::MAX);
            let stmt = if error.is_some() { Err(CalcError::UpstreamTable) } else { column(t, c, &live, &env) };
            (col, stmt)
        })
        .collect();
    (stmts, error)
}

/// The definition column `c` of `t` makes from the data rows `live`. `t`'s shape has been checked.
fn column(t: &Table, c: usize, live: &[usize], env: &Env) -> Result<Stmt, CalcError> {
    let head = t.rows.first().and_then(|r| r.cells.get(c)).ok_or(CalcError::UpstreamTable)?;
    let text = cell_text(head).map_err(|e| syntax(format!("the heading of column {} {e}", c.saturating_add(1))))?;
    let (name, kind) = heading(&text)?;
    let unit = match &kind {
        ColumnKind::Unit(unit) => Some(unit.clone()),
        ColumnKind::Plain | ColumnKind::Text => None,
    };
    if let Some(unit) = &unit {
        env.check_display_unit(unit)?;
    }
    let dim = unit.as_deref().map(parse_unit).transpose()?.map(|s| s.dim);
    let values = live.len();
    if values > MAX_ELEMENTS {
        return Err(CalcError::TooLarge);
    }
    let text_column = kind == ColumnKind::Text;
    let mut items = Vec::with_capacity(values);
    for &k in live {
        let at = |message: &str| syntax(format!("{name} row {k} {message}"));
        let cell = t.rows.get(k).and_then(|r| r.cells.get(c)).ok_or_else(|| at("isn't a number"))?;
        let text = cell_text(cell).map_err(|e| at(&e))?;
        if text_column {
            items.push(Expr::Text(text_cell(&text).map_err(|e| at(&e))?));
            continue;
        }
        let (value, own) = cell_value(&text).map_err(|e| at(&e))?;
        let item = match (own, &unit) {
            (None, None) => Expr::Num(value),
            (None, Some(unit)) => Expr::Quantity(value, unit.clone()),
            (Some(own), _) => {
                let unit_error = |e: CalcError| syntax(format!("{name} row {k}: {e}"));
                env.check_display_unit(&own).map_err(unit_error)?;
                let scale = parse_unit(&own).map_err(|e| unit_error(e.into()))?;
                let fits = match dim {
                    Some(dim) => scale.dim == dim,
                    None => scale.dim.is_dimensionless(),
                };
                if !fits {
                    let heading = unit.as_deref().map_or_else(|| "has no unit".to_string(), |u| format!("is in {u}"));
                    return Err(at(&format!("is in {own}, but the column's heading {heading}")));
                }
                Expr::Quantity(value, own)
            }
        };
        items.push(item);
    }
    Ok(Stmt::Define { name, index: None, params: Vec::new(), body: Expr::Matrix { rows: values, cols: 1, items }, show: None })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_values_are_strict() {
        assert_eq!(cell_value("0.840"), Ok((0.84, None)));
        assert_eq!(cell_value("-3"), Ok((-3.0, None)));
        assert_eq!(cell_value("3/4"), Ok((0.75, None)));
        assert_eq!(cell_value("1-1/2"), Ok((1.5, None)));
        assert_eq!(cell_value("1 1/2"), Ok((1.5, None)));
        assert_eq!(cell_value("-1-1/2"), Ok((-1.5, None)));
        assert_eq!(cell_value("1.5 in"), Ok((1.5, Some("in".to_string()))));
        assert_eq!(cell_value("1-1/2 in"), Ok((1.5, Some("in".to_string()))));
        assert_eq!(cell_value("1e3"), Ok((1000.0, None)));
        for bad in ["", "1,500", "1 500", "1-1/2-1", "2+3", "abc", "1/0"] {
            assert!(cell_value(bad).is_err(), "{bad:?} reads as {:?}", cell_value(bad));
        }
    }

    #[test]
    fn text_cells_are_checked() {
        assert_eq!(text_cell("A106 B"), Ok("A106 B".to_string()));
        assert_eq!(text_cell("6"), Ok("6".to_string()));
        assert_eq!(text_cell(""), Err("is empty".to_string()));
        for mark in ['"', '“', '”'] {
            assert_eq!(text_cell(&format!("1/2{mark} pipe")), Err(format!("has a quote mark {mark}, which text can't hold")));
        }
        assert!(text_cell(&"x".repeat(MAX_TEXT_CHARS)).is_ok());
        assert_eq!(text_cell(&"x".repeat(MAX_TEXT_CHARS + 1)), Err("is longer than 1,000 characters".to_string()));
    }

    #[test]
    fn headings() {
        assert_eq!(heading("D_o (in)"), Ok(("D_o".to_string(), ColumnKind::Unit("in".to_string()))));
        assert_eq!(heading("t_nom"), Ok(("t_nom".to_string(), ColumnKind::Plain)));
        assert_eq!(heading("Material (Text)"), Ok(("Material".to_string(), ColumnKind::Text)));
        assert_eq!(heading("Material (TEXT)"), Ok(("Material".to_string(), ColumnKind::Text)));
        for bad in ["2x (in)", "m (kg)", "D_o (furlongs)"] {
            assert!(heading(bad).is_err(), "{bad:?} reads as {:?}", heading(bad));
        }
        assert_eq!(heading("m (kg)"), Err(CalcError::ReservedName("m".into())));
        assert_eq!(heading("or (in)"), Err(CalcError::Keyword("or".into())));
        assert_eq!(heading("if"), Err(CalcError::Keyword("if".into())));
    }
}
