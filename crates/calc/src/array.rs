//! Grids of values (vectors and matrices) and the element-wise operations on them.
//!
//! A grid is rows × cols values stored row by row; a column vector has one column. Operations work
//! element by element, broadcasting a single value over a grid, so every rule that holds for single
//! values (units, temperatures, finiteness) holds for each element. One element in error makes the
//! whole result an error naming that element, unless the operation is lenient: then the element's
//! error is kept in the grid's `failed` list and the work goes on (for branches that pick per
//! element). Whatever takes a grid whole refuses one with failures ([`Data::strict`]). A grid holds
//! at most [`MAX_ELEMENTS`], checked before its elements are built.

use crate::eval::{Val, text_use};
use crate::{CalcError, Value};
use craft_units::Quantity;

/// How many elements a grid may hold.
pub(crate) const MAX_ELEMENTS: usize = 10_000;

/// How many characters a text may hold.
pub(crate) const MAX_TEXT_CHARS: usize = 1000;

/// `rows` × `cols` values, row by row: numbers (the default) or texts.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Grid<T = Val> {
    pub(crate) rows: usize,
    pub(crate) cols: usize,
    pub(crate) items: Vec<T>,
    /// The elements whose work failed in a lenient operation, with their errors (not yet naming
    /// the element), sorted by element: at most one per element. A failed element's item is a
    /// placeholder that means nothing. Running out of work is never kept here.
    pub(crate) failed: Vec<(usize, CalcError)>,
}

impl<T> Grid<T> {
    /// A grid of `items`, row by row: [`CalcError::TooLarge`] beyond [`MAX_ELEMENTS`], and an
    /// error for no elements or a count that isn't `rows · cols`.
    pub(crate) fn new(rows: usize, cols: usize, items: Vec<T>) -> Result<Grid<T>, CalcError> {
        check_size(rows, cols, items.len())?;
        Ok(Grid { rows, cols, items, failed: Vec::new() })
    }

    /// [`Grid::new`] with the elements in `failed` failed: their errors (not naming the element),
    /// by element, each element at most once and in increasing order, and their items placeholders.
    pub(crate) fn with_failed(rows: usize, cols: usize, items: Vec<T>, failed: Vec<(usize, CalcError)>) -> Result<Grid<T>, CalcError> {
        debug_assert!(failed.windows(2).all(|w| matches!(w, [(a, _), (b, _)] if a < b)), "failed elements out of order");
        debug_assert!(failed.last().is_none_or(|(k, _)| *k < items.len()), "a failed element outside the grid");
        let mut g = Grid::new(rows, cols, items)?;
        g.failed = failed;
        Ok(g)
    }

    /// The error of the first failed element, naming it; `None` when nothing failed.
    fn first_failure(&self, origin: u8) -> Option<CalcError> {
        let (k, e) = self.failed.first()?;
        Some(element_error(self.rows, self.cols, *k, origin, e.clone()))
    }
}

/// What a failed element's item holds: a value that means nothing. Nothing about it guards against
/// its being read (`truth` would even take its NaN as true), so whatever reads a grid's items
/// skips its failed elements ([`Failures`]) or refuses the grid ([`Data::strict`]).
pub(crate) fn placeholder() -> Val {
    Val::plain(Quantity::dimensionless(f64::NAN))
}

/// The failures of a grid's elements, read in element order.
pub(crate) struct Failures<'a> {
    rest: &'a [(usize, CalcError)],
}

impl<'a> Failures<'a> {
    pub(crate) fn of<T>(d: Option<&'a Grid<T>>) -> Failures<'a> {
        Failures { rest: d.map_or(&[], |g| &g.failed) }
    }

    /// The error of element `k`, if it failed; `k` must not go down between calls.
    pub(crate) fn at(&mut self, k: usize) -> Option<&'a CalcError> {
        while let Some(((j, e), rest)) = self.rest.split_first() {
            if *j > k {
                return None;
            }
            self.rest = rest;
            if *j == k {
                return Some(e);
            }
        }
        None
    }
}

/// Whether a grid of `rows` × `cols` may be built from `len` elements, so a caller can check
/// before converting them. The cap is checked first (an overflowing shape is too large), then that
/// there is at least one element, then the count.
pub(crate) fn check_size(rows: usize, cols: usize, len: usize) -> Result<(), CalcError> {
    let n = rows.checked_mul(cols).filter(|&n| n <= MAX_ELEMENTS).ok_or(CalcError::TooLarge)?;
    if n == 0 {
        return Err(CalcError::Syntax("an array needs at least one element".to_string()));
    }
    if n != len {
        return Err(CalcError::Syntax(format!("an array of {rows} rows × {cols} needs {n} elements, not {len}")));
    }
    Ok(())
}

/// The element of `g` a quoted value's index (`\i`) picks, counted from `origin`: one index into a
/// vector, or a row and a column; `None` outside the grid, and for one index into a matrix.
pub(crate) fn element_at<T: Clone>(g: &Grid<T>, origin: u8, at: (u32, Option<u32>)) -> Option<T> {
    let place = |i: u32, len: usize| usize::try_from(i).ok()?.checked_sub(usize::from(origin)).filter(|&k| k < len);
    let k = match at {
        (i, None) if g.rows == 1 || g.cols == 1 => place(i, g.items.len())?,
        (_, None) => return None,
        (r, Some(c)) => place(r, g.rows)?.checked_mul(g.cols)?.checked_add(place(c, g.cols)?)?,
    };
    g.items.get(k).cloned()
}

/// An evaluated value: one value, or a grid of them; a number or a text.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Data {
    One(Val),
    Grid(Grid),
    Text(String),
    TextGrid(Grid<String>),
    MixedGrid(Grid<Cell>),
}

/// A cell in imported tabular data. Text that looks numeric remains text.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Cell {
    Number(Val),
    Text(String),
    Nested(Box<Data>),
}

impl Cell {
    pub(crate) fn data(self) -> Data {
        match self {
            Self::Number(value) => Data::One(value),
            Self::Text(text) => Data::Text(text),
            Self::Nested(value) => *value,
        }
    }

    pub(crate) fn from_data(value: Data) -> Self {
        match value {
            Data::One(value) => Self::Number(value),
            Data::Text(text) => Self::Text(text),
            other => Self::Nested(Box::new(other)),
        }
    }

    pub(crate) fn copy_work(&self) -> u64 {
        match self {
            Self::Number(_) => 0,
            Self::Text(text) => text_steps(text),
            Self::Nested(value) => value.copy_work(),
        }
    }

    pub(crate) fn text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            Self::Number(_) | Self::Nested(_) => None,
        }
    }
}

pub(crate) fn cells_to_data(rows: usize, cols: usize, mut cells: Vec<Cell>) -> Result<Data, CalcError> {
    if rows == 1 && cols == 1 && cells.len() == 1 {
        return cells.pop().map(Cell::data).ok_or(CalcError::TooLarge);
    }
    if cells.iter().all(|c| matches!(c, Cell::Number(_))) {
        return Grid::new(
            rows,
            cols,
            cells
                .into_iter()
                .filter_map(|c| match c {
                    Cell::Number(value) => Some(value),
                    Cell::Text(_) | Cell::Nested(_) => None,
                })
                .collect(),
        )
        .map(Data::Grid);
    }
    if cells.iter().all(|c| matches!(c, Cell::Text(_))) {
        return Grid::new(
            rows,
            cols,
            cells
                .into_iter()
                .filter_map(|c| match c {
                    Cell::Text(text) => Some(text),
                    Cell::Number(_) | Cell::Nested(_) => None,
                })
                .collect(),
        )
        .map(Data::TextGrid);
    }
    Grid::new(rows, cols, cells).map(Data::MixedGrid)
}

impl Data {
    /// The data unchanged when no element failed; otherwise the error of the first failed element,
    /// naming it, counted from `origin`. What takes a grid whole (an array function, an index, a
    /// stored or shown value) calls this, so a failed element's placeholder never becomes a value.
    pub(crate) fn strict(self, origin: u8) -> Result<Data, CalcError> {
        match self.failure(origin) {
            Some(e) => Err(e),
            None => Ok(self),
        }
    }

    /// The error [`Data::strict`] would give, without taking the data.
    pub(crate) fn failure(&self, origin: u8) -> Option<CalcError> {
        match self {
            Data::One(_) | Data::Text(_) => None,
            Data::Grid(g) => g.first_failure(origin),
            Data::TextGrid(g) => g.first_failure(origin),
            Data::MixedGrid(g) => g.first_failure(origin),
        }
    }

    /// The public form, without what each value stands for as a temperature.
    pub(crate) fn value(&self) -> Value {
        match self {
            Data::One(v) => Value::One(v.q),
            Data::Grid(g) => Value::Grid { rows: g.rows, cols: g.cols, items: g.items.iter().map(|v| v.q).collect() },
            Data::Text(s) => Value::Text(s.clone()),
            Data::TextGrid(g) => Value::TextGrid { rows: g.rows, cols: g.cols, items: g.items.clone() },
            Data::MixedGrid(g) => Value::MixedGrid {
                rows: g.rows,
                cols: g.cols,
                items: g
                    .items
                    .iter()
                    .map(|cell| match cell {
                        Cell::Number(value) => crate::CellValue::Number(value.q),
                        Cell::Text(text) => crate::CellValue::Text(text.clone()),
                        Cell::Nested(value) => crate::CellValue::Nested(Box::new(value.value())),
                    })
                    .collect(),
            },
        }
    }

    /// A grid's rows and columns; `None` for a single value or text.
    pub(crate) fn shape(&self) -> Option<(usize, usize)> {
        match self {
            Data::One(_) | Data::Text(_) => None,
            Data::Grid(g) => Some((g.rows, g.cols)),
            Data::TextGrid(g) => Some((g.rows, g.cols)),
            Data::MixedGrid(g) => Some((g.rows, g.cols)),
        }
    }

    /// The work of building or copying this value, in steps: one per element of a grid, plus one
    /// per 32 bytes of text, so copying text costs in proportion to the memory it takes (a text is
    /// up to [`MAX_TEXT_CHARS`] characters, several times a number's size).
    pub(crate) fn copy_work(&self) -> u64 {
        let elements = |rows: usize, cols: usize| u64::try_from(rows.saturating_mul(cols)).unwrap_or(u64::MAX);
        match self {
            Data::One(_) => 0,
            Data::Grid(g) => elements(g.rows, g.cols),
            Data::Text(s) => text_steps(s),
            Data::TextGrid(g) => g.items.iter().fold(elements(g.rows, g.cols), |n, s| n.saturating_add(text_steps(s))),
            Data::MixedGrid(g) => g.items.iter().fold(elements(g.rows, g.cols), |n, cell| n.saturating_add(cell.copy_work())),
        }
    }

    /// The text itself, or a text grid's first element that did not fail (so an error naming it
    /// never shows a failed element's placeholder); `None` for numbers. A text grid always has
    /// one: a grid whose every element failed is built as numbers.
    pub(crate) fn text(&self) -> Option<&str> {
        match self {
            Data::Text(s) => Some(s),
            Data::TextGrid(g) => {
                let mut failures = Failures::of(Some(g));
                let mut texts = g.items.iter().enumerate().filter(|(k, _)| failures.at(*k).is_none());
                texts.next().or_else(|| g.items.first().map(|s| (0, s))).map(|(_, s)| s.as_str())
            }
            Data::MixedGrid(g) => g.items.iter().find_map(Cell::text),
            Data::One(_) | Data::Grid(_) => None,
        }
    }
}

/// What two texts are compared by: the text trimmed, with every run of whitespace (a non-breaking
/// space included) as one space. Case is kept.
pub(crate) fn text_key(s: &str) -> String {
    let mut key = String::with_capacity(s.len());
    for word in s.split_whitespace() {
        if !key.is_empty() {
            key.push(' ');
        }
        key.push_str(word);
    }
    key
}

/// The steps of work a text costs to copy: one per 32 bytes (UTF-8).
pub(crate) fn text_steps(text: &str) -> u64 {
    u64::try_from(text.len() / 32).unwrap_or(u64::MAX)
}

/// The error for a `rows` × `cols` grid that would hold both texts and numbers: it names the later
/// of the element `text_at` (a text) and the element `number_at` (a number), then the earlier.
/// Elements are named as [`element_name`] does, counted from `origin`.
pub(crate) fn mixed(rows: usize, cols: usize, text_at: usize, number_at: usize, origin: u8) -> CalcError {
    let (t, n) = (element_name(rows, cols, text_at, origin), element_name(rows, cols, number_at, origin));
    CalcError::Syntax(if text_at > number_at {
        format!("element {t} is text but element {n} is a number")
    } else {
        format!("element {n} is a number but element {t} is text")
    })
}

/// The shape of combining `a` and `b` element by element: `None` for two single values, a grid's
/// shape when the other is a single value (broadcast) or has the same shape, else
/// [`CalcError::Shape`].
pub(crate) fn result_shape(a: &Data, b: &Data) -> Result<Option<(usize, usize)>, CalcError> {
    match (a.shape(), b.shape()) {
        (Some(l), Some(r)) if l != r => Err(CalcError::Shape { left: l, right: r }),
        (l, r) => Ok(l.or(r)),
    }
}

/// `f` over `a` and `b` element by element, a single value broadcast over a grid. Two single
/// values give `f`'s result (and its error) as it is. An element that failed in either input fails
/// in the result with that error (`a`'s when both did), without calling `f`. When `lenient`, an
/// element's error is kept in the result's `failed` list; otherwise the first is returned, naming
/// the element, counted from `origin`.
pub(crate) fn zip(a: &Data, b: &Data, origin: u8, lenient: bool, mut f: impl FnMut(Val, Val) -> Result<Val, CalcError>) -> Result<Data, CalcError> {
    result_shape(a, b)?;
    if let (Data::One(x), Data::One(y)) = (a, b) {
        return f(*x, *y).map(Data::One);
    }
    let (mut fa, mut fb) = (Failures::of(grid_of(a)), Failures::of(grid_of(b)));
    // Both inputs' failures are read at every element, so neither falls behind.
    let mut step = |k: usize, x: Val, y: Val| {
        let (ea, eb) = (fa.at(k), fb.at(k));
        match ea.or(eb) {
            Some(e) => Err(e.clone()),
            None => f(x, y),
        }
    };
    match (a, b) {
        (Data::Grid(g), Data::One(y)) => build(g, origin, lenient, g.items.iter().enumerate().map(|(k, x)| step(k, *x, *y))),
        (Data::One(x), Data::Grid(g)) => build(g, origin, lenient, g.items.iter().enumerate().map(|(k, y)| step(k, *x, *y))),
        (Data::Grid(g), Data::Grid(h)) => build(g, origin, lenient, g.items.iter().zip(&h.items).enumerate().map(|(k, (x, y))| step(k, *x, *y))),
        // The evaluator refuses text before it gets here, with the place it was used.
        _ => Err(not_numbers(&[a, b])),
    }
}

/// `f` over every element of `a` (or `a` itself when it is a single value, whose error is returned
/// as it is). An element that failed in `a` fails in the result with that error, without calling
/// `f`. When `lenient`, an element's error is kept in the result's `failed` list; otherwise the
/// first is returned, naming the element, counted from `origin`.
pub(crate) fn map(a: &Data, origin: u8, lenient: bool, mut f: impl FnMut(Val) -> Result<Val, CalcError>) -> Result<Data, CalcError> {
    match a {
        Data::One(x) => f(*x).map(Data::One),
        Data::Grid(g) => {
            let mut failures = Failures::of(Some(g));
            let results = g.items.iter().enumerate().map(|(k, x)| match failures.at(k) {
                Some(e) => Err(e.clone()),
                None => f(*x),
            });
            build(g, origin, lenient, results)
        }
        Data::Text(_) | Data::TextGrid(_) | Data::MixedGrid(_) => Err(not_numbers(&[a])),
    }
}

/// The grid of numbers `d` holds, if any.
fn grid_of(d: &Data) -> Option<&Grid> {
    match d {
        Data::Grid(g) => Some(g),
        Data::One(_) | Data::Text(_) | Data::TextGrid(_) | Data::MixedGrid(_) => None,
    }
}

/// The error for text among `operands` of an element-wise operation.
fn not_numbers(operands: &[&Data]) -> CalcError {
    match operands.iter().find_map(|d| d.text()) {
        Some(text) => text_use(text, "in arithmetic"),
        None => CalcError::Syntax("an array operation needs numbers".to_string()),
    }
}

/// A grid of `shape`'s shape from per-element results. Strict, it stops at the first error, naming
/// its element. Lenient, an element's error is kept in `failed` with a placeholder item, but
/// running out of work still stops it.
fn build(shape: &Grid, origin: u8, lenient: bool, results: impl Iterator<Item = Result<Val, CalcError>>) -> Result<Data, CalcError> {
    let (rows, cols) = (shape.rows, shape.cols);
    let mut failed = Vec::new();
    let mut items = Vec::with_capacity(shape.items.len());
    for (k, r) in results.enumerate() {
        items.push(match r {
            Ok(v) => v,
            Err(e) => {
                keep_failure(&mut failed, k, e, lenient, |e| element_error(rows, cols, k, origin, e))?;
                placeholder()
            }
        });
    }
    Grid::with_failed(rows, cols, items, failed).map(Data::Grid)
}

/// Keep element `k`'s error `e` in `failed` when `lenient`; otherwise, and for running out of work
/// (never about an element), the error to stop with: `e` as `name` names it.
pub(crate) fn keep_failure(
    failed: &mut Vec<(usize, CalcError)>,
    k: usize,
    e: CalcError,
    lenient: bool,
    name: impl FnOnce(CalcError) -> CalcError,
) -> Result<(), CalcError> {
    match e {
        CalcError::TooMuchWork | CalcError::TooComplex => Err(e),
        e if lenient => {
            failed.push((k, e));
            Ok(())
        }
        e => Err(name(e)),
    }
}

/// `error` as the error of element `k` of a `rows` × `cols` grid. Running out of work isn't about
/// an element, so it is left as it is.
pub(crate) fn element_error(rows: usize, cols: usize, k: usize, origin: u8, error: CalcError) -> CalcError {
    match error {
        CalcError::TooMuchWork | CalcError::TooComplex => error,
        error => CalcError::Element { at: element_name(rows, cols, k, origin), error: Box::new(error) },
    }
}

/// How element `k` (row-major) of a `rows` × `cols` grid is named, counted from `origin`: a
/// vector's element by one index (`2`; a row vector counts along its columns), a matrix's by row
/// and column (`[1, 2]`).
pub(crate) fn element_name(rows: usize, cols: usize, k: usize, origin: u8) -> String {
    let o = usize::from(origin);
    if rows == 1 || cols == 1 {
        k.saturating_add(o).to_string()
    } else {
        let (r, c) = (k.checked_div(cols).unwrap_or(0), k.checked_rem(cols).unwrap_or(0));
        format!("[{}, {}]", r.saturating_add(o), c.saturating_add(o))
    }
}

#[cfg(test)]
mod tests {
    use super::{Data, Grid, map, zip};
    use crate::CalcError;
    use crate::eval::Val;
    use craft_units::Quantity;

    fn num(v: f64) -> Val {
        Val::plain(Quantity::dimensionless(v))
    }

    fn column(values: &[f64]) -> Data {
        Data::Grid(Grid { rows: values.len(), cols: 1, items: values.iter().map(|&v| num(v)).collect(), failed: Vec::new() })
    }

    /// Fails on negative elements, else doubles.
    fn double(v: Val) -> Result<Val, CalcError> {
        if v.q.value < 0.0 { Err(CalcError::NonFinite) } else { Ok(num(2.0 * v.q.value)) }
    }

    fn grid(d: Data) -> Grid {
        match d {
            Data::Grid(g) => g,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn lenient_map_keeps_going_past_an_error() {
        let g = grid(map(&column(&[1.0, -1.0, 3.0, -2.0]), 0, true, double).unwrap());
        assert_eq!(g.failed, [(1, CalcError::NonFinite), (3, CalcError::NonFinite)]);
        assert_eq!((g.items[0].q.value, g.items[2].q.value), (2.0, 6.0));
        // Strict, the first error names its element.
        let e = map(&column(&[1.0, -1.0, 3.0]), 1, false, double).unwrap_err();
        assert_eq!(e.to_string(), "element 2: result isn't a finite number");
    }

    #[test]
    fn strict_map_returns_an_input_failure() {
        let mut g = grid(column(&[1.0, 2.0, 3.0]));
        g.failed = vec![(1, CalcError::Upstream)];
        let input = Data::Grid(g);
        let mut calls = 0;
        let e = map(&input, 0, false, |v| {
            calls += 1;
            double(v)
        })
        .unwrap_err();
        assert_eq!(e, CalcError::Element { at: "1".into(), error: Box::new(CalcError::Upstream) });
        assert_eq!(calls, 1, "stops at the failed element");
        // Lenient, the failure passes to the output element without calling `f` on it.
        let mut seen = Vec::new();
        let out = grid(
            map(&input, 0, true, |v| {
                seen.push(v.q.value);
                double(v)
            })
            .unwrap(),
        );
        assert_eq!(seen, [1.0, 3.0]);
        assert_eq!(out.failed, [(1, CalcError::Upstream)]);
        assert_eq!(input.clone().strict(0), Err(CalcError::Element { at: "1".into(), error: Box::new(CalcError::Upstream) }));
        assert_eq!(column(&[1.0]).strict(0), Ok(column(&[1.0])));
    }

    #[test]
    fn zip_merges_failures_in_element_order() {
        let mut a = grid(column(&[1.0, 2.0, 3.0, 4.0]));
        a.failed = vec![(2, CalcError::Upstream)];
        let mut b = grid(column(&[1.0, 2.0, 3.0, 4.0]));
        b.failed = vec![(0, CalcError::TempCompare), (2, CalcError::NotCondition)];
        let (a, b) = (Data::Grid(a), Data::Grid(b));
        let add = |x: Val, y: Val| if x.q.value == 4.0 { Err(CalcError::NonFinite) } else { Ok(num(x.q.value + y.q.value)) };
        let g = grid(zip(&a, &b, 0, true, add).unwrap());
        // `a`'s failure wins where both failed.
        assert_eq!(g.failed, [(0, CalcError::TempCompare), (2, CalcError::Upstream), (3, CalcError::NonFinite)]);
        assert_eq!(g.items[1].q.value, 4.0);
        assert_eq!(
            zip(&a, &b, 0, false, add).unwrap_err().to_string(),
            CalcError::Element { at: "0".into(), error: Box::new(CalcError::TempCompare) }.to_string()
        );
        // A single value broadcast over a grid with failures keeps them.
        let g = grid(zip(&Data::One(num(1.0)), &a, 0, true, add).unwrap());
        assert_eq!(g.failed, [(2, CalcError::Upstream)]);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "failed elements out of order")]
    fn failed_elements_are_kept_in_order() {
        let _ = Grid::with_failed(2, 1, vec![num(1.0), num(2.0)], vec![(1, CalcError::NonFinite), (0, CalcError::NonFinite)]);
    }

    #[test]
    fn running_out_of_work_is_never_kept_per_element() {
        let e = map(&column(&[1.0, 2.0]), 0, true, |_| Err(CalcError::TooMuchWork)).unwrap_err();
        assert_eq!(e, CalcError::TooMuchWork);
        let e = map(&column(&[1.0, 2.0]), 0, true, |_| Err(CalcError::TooComplex)).unwrap_err();
        assert_eq!(e, CalcError::TooComplex);
    }
}
