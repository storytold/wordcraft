//! Matrix shaping: `submatrix(A, ir, jr, ic, jc)` cuts rows `ir…jr` and columns `ic…jc` out of
//! `A`, `augment(A, B, …)` puts matrices side by side and `stack(A, B, …)` one below the other.
//!
//! They take numbers, each element keeping its own unit, or texts (as `transpose` does); a text
//! with a number is the error [`array::mixed`] gives. Indexes count from the document's index
//! base, and a single value is a 1 × 1 matrix. Each charges the elements it makes to the pass
//! before copying them, and a text its bytes besides, as copying one does elsewhere.

use super::{Ctx, arg_name, charge, shape};
use crate::array::{self, Cell, Data, Grid, MAX_ELEMENTS, text_steps};
use crate::eval::Val;
use crate::{CalcError, Expr};
use craft_units::Dim;

/// How far a number may be from a whole number and still count as it: an index is as exact as
/// one written in `M[i]`.
const WHOLE: f64 = 1e-9;

/// The elements of one argument, row by row.
enum Block<'a> {
    Numbers(&'a [Val]),
    Texts(&'a [String]),
    Mixed(&'a [Cell]),
}

/// An argument as a matrix.
struct Piece<'a> {
    rows: usize,
    cols: usize,
    block: Block<'a>,
}

impl<'a> Piece<'a> {
    fn of(d: &'a Data) -> Piece<'a> {
        let (rows, cols) = shape(d);
        let block = match d {
            Data::One(v) => Block::Numbers(std::slice::from_ref(v)),
            Data::Grid(g) => Block::Numbers(&g.items),
            Data::Text(s) => Block::Texts(std::slice::from_ref(s)),
            Data::TextGrid(g) => Block::Texts(&g.items),
            Data::MixedGrid(g) => Block::Mixed(&g.items),
        };
        Piece { rows, cols, block }
    }

    fn cell(&self, index: usize) -> Option<Cell> {
        match self.block {
            Block::Numbers(values) => values.get(index).copied().map(Cell::Number),
            Block::Texts(values) => values.get(index).cloned().map(Cell::Text),
            Block::Mixed(values) => values.get(index).cloned(),
        }
    }

    fn numbers(&self) -> &'a [Val] {
        match self.block {
            Block::Numbers(v) => v,
            Block::Texts(_) | Block::Mixed(_) => &[],
        }
    }

    fn texts(&self) -> &'a [String] {
        match self.block {
            Block::Texts(v) => v,
            Block::Numbers(_) | Block::Mixed(_) => &[],
        }
    }

    fn is_text(&self) -> bool {
        matches!(self.block, Block::Texts(_))
    }
}

fn missing() -> CalcError {
    CalcError::Syntax("an array is missing an element".to_string())
}

/// `index` as a whole number, if it is a single plain number within [`WHOLE`] of one.
fn whole_index(index: &Data) -> Option<f64> {
    let Data::One(v) = index else { return None };
    let r = v.q.value.round();
    (v.q.dim == Dim::ONE && v.q.value.is_finite() && (v.q.value - r).abs() <= WHOLE).then_some(r)
}

/// A whole number as the author would write it: `3`, `-1`, and `1e300` for one too large to spell.
fn index_text(i: f64) -> String {
    if i.abs() < 1e15 { format!("{}", i as i64) } else { format!("{i:e}") }
}

/// Rows or columns of the matrix a `submatrix` cuts from.
struct Axis<'a> {
    /// `row` or `column`.
    what: &'a str,
    /// The names of its two ends.
    labels: [&'a str; 2],
    /// The number of rows or columns.
    len: usize,
    /// The argument that holds the first end.
    at: usize,
}

/// The span of `axis` that `args` give, as places counted from 0, both ends included. `name` is the
/// matrix as the author wrote it. The ends count from the index base `origin`.
fn span(args: &[Data], exprs: &[Expr], name: &str, axis: &Axis<'_>, origin: u8) -> Result<(usize, usize), CalcError> {
    let Axis { what, labels, len, at } = *axis;
    let o = f64::from(origin);
    let last = len.saturating_sub(1) as f64 + o;
    let mut ends = [0.0; 2];
    for ((k, end), label) in ends.iter_mut().enumerate().zip(labels) {
        let label = arg_name(exprs, at + k, label);
        let i = args.get(at + k).and_then(whole_index).ok_or_else(|| CalcError::Syntax(format!("submatrix needs {label} to be a whole number")))?;
        if i < o || i > last {
            let (shown, to) = (index_text(i), index_text(last));
            return Err(CalcError::IndexRange(format!("submatrix {what} {shown} ({label}) is outside {name} ({what}s {origin} to {to})")));
        }
        *end = i - o;
    }
    let [from, to] = ends;
    if from > to {
        let [a, b] = labels;
        let (from, to) = (index_text(from + o), index_text(to + o));
        return Err(CalcError::Syntax(format!("submatrix needs {a} to be at most {b} ({what}s {from} and {to})")));
    }
    // Whole numbers from 0 to `len`, so the conversions are exact.
    Ok((from as usize, to as usize))
}

/// `submatrix(A, ir, jr, ic, jc)`: rows `ir` to `jr` and columns `ic` to `jc` of `A`, both ends
/// included, counted from the index base. A text matrix gives texts.
pub(super) fn submatrix(args: &[Data], exprs: &[Expr], cx: &Ctx<'_>) -> Result<Data, CalcError> {
    let [a, _, _, _, _] = args else {
        return Err(CalcError::Arity("submatrix".to_string(), 5));
    };
    let name = arg_name(exprs, 0, "A");
    let piece = Piece::of(a);
    let (r0, r1) = span(args, exprs, name, &Axis { what: "row", labels: ["ir", "jr"], len: piece.rows, at: 1 }, cx.origin)?;
    let (c0, c1) = span(args, exprs, name, &Axis { what: "column", labels: ["ic", "jc"], len: piece.cols, at: 3 }, cx.origin)?;
    let size = (r1 - r0 + 1, c1 - c0 + 1);
    let order = (r0..=r1).flat_map(|i| (c0..=c1).map(move |j| (0, i * piece.cols + j))).collect::<Vec<_>>();
    let mixed = matches!(piece.block, Block::Mixed(_));
    finish(&[piece], &order, size, cx, mixed)
}

/// How argument `i` is named in an error about several: its name when it is a bare name, else its
/// place.
fn which(exprs: &[Expr], i: usize) -> String {
    match exprs.get(i) {
        Some(Expr::Name(n)) => n.clone(),
        _ => format!("argument {}", i.saturating_add(1)),
    }
}

/// `augment(A, B, …)`: the matrices side by side. They must have the same number of rows.
pub(super) fn augment(args: &[Data], exprs: &[Expr], cx: &Ctx<'_>) -> Result<Data, CalcError> {
    joined("augment", true, false, args, exprs, cx)
}

/// `stack(A, B, …)`: the matrices one below the other. They must have the same number of columns.
pub(super) fn stack(args: &[Data], exprs: &[Expr], cx: &Ctx<'_>) -> Result<Data, CalcError> {
    joined("stack", false, false, args, exprs, cx)
}

/// `augment` (`beside`) or `stack` of at least two matrices.
fn joined(name: &str, beside: bool, mixed: bool, args: &[Data], exprs: &[Expr], cx: &Ctx<'_>) -> Result<Data, CalcError> {
    let pieces: Vec<Piece<'_>> = args.iter().map(Piece::of).collect();
    let (Some(first), true) = (pieces.first(), pieces.len() >= 2) else {
        return Err(CalcError::Arity(name.to_string(), 2));
    };
    // What every matrix has in common, and what adds up.
    let (shared, own): (fn(&Piece<'_>) -> usize, fn(&Piece<'_>) -> usize) = if beside { (|p| p.rows, |p| p.cols) } else { (|p| p.cols, |p| p.rows) };
    if let Some((k, odd)) = pieces.iter().enumerate().find(|(_, p)| shared(p) != shared(first)) {
        let (a, b) = (which(exprs, 0), which(exprs, k));
        let (what, size) = (if beside { "rows" } else { "columns" }, |p: &Piece<'_>| format!("{} × {}", p.rows, p.cols));
        return Err(CalcError::Syntax(format!(
            "{name} needs {a} and {b} to have the same number of {what} ({a} is {}, {b} is {})",
            size(first),
            size(odd)
        )));
    }
    let total = pieces.iter().fold(0usize, |n, p| n.saturating_add(own(p)));
    let size = if beside { (first.rows, total) } else { (total, first.cols) };
    // Refused before the order of its elements is built.
    size.0.checked_mul(size.1).filter(|&n| n <= MAX_ELEMENTS).ok_or(CalcError::TooLarge)?;
    // Each matrix's first element in the result, for the error of text with numbers.
    let mut taken = 0usize;
    let starts = pieces.iter().map(|p| {
        let at = taken;
        taken = taken.saturating_add(own(p));
        if beside { at } else { at.saturating_mul(first.cols) }
    });
    let mixed = mixed || pieces.iter().any(|piece| matches!(piece.block, Block::Mixed(_)));
    if !mixed {
        check_kinds(&pieces, starts, size, cx.origin)?;
    }
    let order: Vec<(usize, usize)> = if beside {
        (0..first.rows).flat_map(|i| pieces.iter().enumerate().flat_map(move |(p, m)| (0..m.cols).map(move |j| (p, i * m.cols + j)))).collect()
    } else {
        pieces.iter().enumerate().flat_map(|(p, m)| (0..m.rows.saturating_mul(m.cols)).map(move |k| (p, k))).collect()
    };
    finish(&pieces, &order, size, cx, mixed)
}

/// An error if `pieces` hold both texts and numbers; it names the first text and the first number
/// as they sit in the result (`size`), given where each matrix's first element falls.
fn check_kinds(pieces: &[Piece<'_>], starts: impl Iterator<Item = usize>, size: (usize, usize), origin: u8) -> Result<(), CalcError> {
    let starts: Vec<usize> = starts.collect();
    let at = |text: bool| pieces.iter().zip(&starts).find(|(p, _)| p.is_text() == text).map(|(_, &k)| k);
    match (at(true), at(false)) {
        (Some(t), Some(n)) => Err(array::mixed(size.0, size.1, t, n, origin)),
        _ => Ok(()),
    }
}

/// The elements `order` names (a matrix and an element of it), from the matrices `from` gives.
fn pick<'a, T>(order: &[(usize, usize)], from: impl Fn(usize) -> &'a [T]) -> Result<Vec<&'a T>, CalcError> {
    order.iter().map(|&(p, k)| from(p).get(k).ok_or_else(missing)).collect()
}

/// The `rows` × `cols` matrix of the elements `order` names (a matrix and an element of it), all
/// numbers or all texts. The elements are charged first, and the bytes of the texts besides.
fn finish(pieces: &[Piece<'_>], order: &[(usize, usize)], (rows, cols): (usize, usize), cx: &Ctx<'_>, mixed: bool) -> Result<Data, CalcError> {
    rows.checked_mul(cols).filter(|&n| n <= MAX_ELEMENTS).ok_or(CalcError::TooLarge)?;
    charge(cx, order.len())?;
    if mixed {
        let cells = order
            .iter()
            .map(|&(p, k)| {
                let cell = pieces.get(p).and_then(|piece| piece.cell(k)).ok_or_else(missing)?;
                if let Some(text) = cell.text() {
                    cx.pass.charge(text_steps(text))?;
                }
                Ok(cell)
            })
            .collect::<Result<Vec<_>, CalcError>>()?;
        return array::cells_to_data(rows, cols, cells);
    }
    if pieces.first().is_some_and(Piece::is_text) {
        let items = pick(order, |p| pieces.get(p).map_or(&[][..], Piece::texts))?;
        cx.pass.charge(items.iter().fold(0, |n: u64, s| n.saturating_add(text_steps(s))))?;
        let mut items: Vec<String> = items.into_iter().cloned().collect();
        return match (items.pop(), items.is_empty()) {
            (Some(s), true) => Ok(Data::Text(s)),
            (last, _) => Grid::new(rows, cols, items.into_iter().chain(last).collect()).map(Data::TextGrid),
        };
    }
    let mut items: Vec<Val> = pick(order, |p| pieces.get(p).map_or(&[][..], Piece::numbers))?.into_iter().copied().collect();
    match (items.pop(), items.is_empty()) {
        (Some(v), true) => Ok(Data::One(v)),
        (last, _) => Grid::new(rows, cols, items.into_iter().chain(last).collect()).map(Data::Grid),
    }
}

/// Mathcad's column selector, preserving the selected cells' types.
pub(super) fn matcol(args: &[Data], exprs: &[Expr], cx: &Ctx<'_>) -> Result<Data, CalcError> {
    let [matrix, column] = args else { return Err(CalcError::Arity("matcol".into(), 2)) };
    let piece = Piece::of(matrix);
    let (rows, cols) = (piece.rows, piece.cols);
    let label = arg_name(exprs, 0, "A");
    let column = whole_index(column)
        .map(|i| i - f64::from(cx.origin))
        .filter(|i| *i >= 0.0 && *i < cols as f64)
        .ok_or_else(|| CalcError::IndexRange(format!("matcol column is outside {label}")))? as usize;
    let order = (0..rows).map(|row| (0, row * cols + column)).collect::<Vec<_>>();
    let mixed = matches!(piece.block, Block::Mixed(_));
    finish(&[piece], &order, (rows, 1), cx, mixed)
}

pub(super) fn worksheet_augment(args: &[Data], exprs: &[Expr], cx: &Ctx<'_>) -> Result<Data, CalcError> {
    joined("augment", true, true, args, exprs, cx)
}

pub(super) fn worksheet_stack(args: &[Data], exprs: &[Expr], cx: &Ctx<'_>) -> Result<Data, CalcError> {
    joined("stack", false, true, args, exprs, cx)
}

/// Match a key in the first column and return the indicated column as a vector, in row order.
pub(super) fn vlookup(args: &[Data], cx: &Ctx<'_>) -> Result<Data, CalcError> {
    let [key, matrix, column] = args else { return Err(CalcError::Arity("vlookup".into(), 3)) };
    let piece = Piece::of(matrix);
    let target = whole_index(column)
        .map(|i| i - f64::from(cx.origin))
        .filter(|i| *i >= 0.0 && *i < piece.cols as f64)
        .ok_or_else(|| CalcError::IndexRange("vlookup column is outside the table".into()))? as usize;
    if !matches!(key, Data::One(_) | Data::Text(_)) {
        return Err(CalcError::Syntax("vlookup needs a single key".into()));
    }
    cx.pass.charge(matrix.copy_work())?;
    let mut found = Vec::new();
    for row in 0..piece.rows {
        let start = row * piece.cols;
        let matches = match (key, piece.cell(start).ok_or_else(missing)?) {
            (Data::Text(a), Cell::Text(b)) => array::text_key(a) == array::text_key(&b),
            (Data::One(a), Cell::Number(b)) => a.q.dim == b.q.dim && super::nearly_equal(a.q.value, b.q.value),
            _ => false,
        };
        if matches {
            found.push(piece.cell(start + target).ok_or_else(missing)?);
        }
    }
    if found.is_empty() {
        return Err(CalcError::Syntax("vlookup found no matching key".into()));
    }
    if let [only] = found.as_slice() {
        return match only {
            Cell::Number(value) => Grid::new(1, 1, vec![*value]).map(Data::Grid),
            Cell::Text(text) => Grid::new(1, 1, vec![text.clone()]).map(Data::TextGrid),
            Cell::Nested(_) => Grid::new(1, 1, found).map(Data::MixedGrid),
        };
    }
    array::cells_to_data(found.len(), 1, found)
}
