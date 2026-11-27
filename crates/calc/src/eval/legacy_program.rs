//! The bounded subset of legacy worksheet programs expressible by the importer.

use super::{CalcError, Cx, Data, Expr, Frame, Val, array, eval, eval_args, index_text, index_value, range, tick, truth};
use crate::array::{Cell, Grid, MAX_ELEMENTS, MAX_TEXT_CHARS};
use craft_units::Quantity;

type Locals = Vec<(String, Data)>;

enum Flow {
    Value(Option<Data>),
    Break,
    Return(Data),
}

fn invalid() -> CalcError {
    CalcError::Syntax("invalid imported worksheet program".into())
}

fn frame<'a>(outer: &'a Frame<'a>, locals: &'a Locals) -> Frame<'a> {
    Frame { names: &[], values: &[], locals, calls: outer.calls, parent: Some(outer) }
}

fn set(locals: &mut Locals, name: &str, value: Data) {
    if let Some((_, slot)) = locals.iter_mut().find(|(key, _)| key == name) {
        *slot = value;
    } else {
        locals.push((name.to_owned(), value));
    }
}

pub(super) fn run(cx: &Cx<'_>, outer: &Frame<'_>, args: &[Expr], depth: usize) -> Result<Data, CalcError> {
    match sequence(cx, outer, &mut Vec::new(), args, depth)? {
        Flow::Return(value) | Flow::Value(Some(value)) => Ok(value),
        Flow::Value(None) | Flow::Break => Err(CalcError::Syntax("worksheet program did not return a value".into())),
    }
}

fn sequence(cx: &Cx<'_>, outer: &Frame<'_>, locals: &mut Locals, args: &[Expr], depth: usize) -> Result<Flow, CalcError> {
    let mut last = None;
    for expr in args {
        match step(cx, outer, locals, expr, depth + 1)? {
            Flow::Value(Some(value)) => last = Some(value),
            Flow::Value(None) => {}
            flow => return Ok(flow),
        }
    }
    Ok(Flow::Value(last))
}

fn step(cx: &Cx<'_>, outer: &Frame<'_>, locals: &mut Locals, expr: &Expr, depth: usize) -> Result<Flow, CalcError> {
    if depth > super::MAX_EVAL_DEPTH {
        return Err(CalcError::TooComplex);
    }
    tick(cx)?;
    let Expr::Call(name, args) = expr else { return eval(cx, &frame(outer, locals), expr, depth).map(|v| Flow::Value(Some(v))) };
    match (name.as_str(), args.as_slice()) {
        ("worksheetProgram", args) => sequence(cx, outer, locals, args, depth),
        ("worksheetBreak", []) => Ok(Flow::Break),
        ("worksheetReturn", [value]) => eval(cx, &frame(outer, locals), value, depth).map(Flow::Return),
        ("worksheetSet", [Expr::Text(name), value]) => {
            let value = eval(cx, &frame(outer, locals), value, depth)?;
            set(locals, name, value);
            Ok(Flow::Value(None))
        }
        ("worksheetSetCell", [Expr::Text(name), value, at @ ..]) => {
            let scope = frame(outer, locals);
            let value = eval(cx, &scope, value, depth)?;
            let old = scope.param(name);
            let new = write_cell(cx, &scope, old, value, at, depth)?;
            set(locals, name, new);
            Ok(Flow::Value(None))
        }
        ("worksheetWhen" | "worksheetIf", [condition, body, otherwise @ ..]) if otherwise.len() <= 1 => {
            let condition = eval(cx, &frame(outer, locals), condition, depth)?;
            let Data::One(value) = truth(cx, &condition)? else { return Err(CalcError::NotCondition) };
            if value.q.value != 0.0 {
                step(cx, outer, locals, body, depth + 1)
            } else if let [other] = otherwise {
                step(cx, outer, locals, other, depth + 1)
            } else {
                Ok(Flow::Value(None))
            }
        }
        ("worksheetFor", [Expr::Text(name), bounds, body]) => {
            let scope = frame(outer, locals);
            let Data::Grid(values) = eval(cx, &scope, bounds, depth)? else { return Err(invalid()) };
            for value in values.items {
                tick(cx)?;
                set(locals, name, Data::One(value));
                match step(cx, outer, locals, body, depth + 1)? {
                    Flow::Return(value) => return Ok(Flow::Return(value)),
                    Flow::Break => break,
                    Flow::Value(_) => {}
                }
            }
            Ok(Flow::Value(None))
        }
        _ => eval(cx, &frame(outer, locals), expr, depth).map(|v| Flow::Value(Some(v))),
    }
}

fn coordinates(cx: &Cx<'_>, scope: &Frame<'_>, at: &[Expr], depth: usize) -> Result<Vec<usize>, CalcError> {
    if !(1..=2).contains(&at.len()) {
        return Err(invalid());
    }
    at.iter()
        .map(|i| {
            let n = index_value(cx, scope, i, depth)?;
            let offset = n - f64::from(cx.env.origin);
            if offset < 0.0 || offset >= MAX_ELEMENTS as f64 {
                return Err(CalcError::IndexRange(format!("worksheet matrix index {} is outside the data", index_text(n))));
            }
            Ok(offset as usize)
        })
        .collect()
}

fn write_cell(cx: &Cx<'_>, scope: &Frame<'_>, old: Option<&Data>, value: Data, at: &[Expr], depth: usize) -> Result<Data, CalcError> {
    let coordinates = coordinates(cx, scope, at, depth)?;
    let (r, c) = (coordinates[0], coordinates.get(1).copied().unwrap_or(0));
    let (old_rows, old_cols) = old.map_or((0, 0), |d| d.shape().unwrap_or((1, 1)));
    let (rows, cols) = (old_rows.max(r + 1), old_cols.max(c + 1));
    let size = rows.checked_mul(cols).filter(|n| *n <= MAX_ELEMENTS).ok_or(CalcError::TooLarge)?;
    cx.pass.charge(size as u64 + old.map_or(0, Data::copy_work))?;
    let zero = Cell::Number(Val::plain(Quantity::dimensionless(0.0)));
    let mut cells = vec![zero; size];
    if let Some(old) = old {
        for i in 0..old_rows {
            for j in 0..old_cols {
                cells[i * cols + j] = cell_at(old, i * old_cols + j).ok_or_else(invalid)?;
            }
        }
    }
    cells[r * cols + c] = pack_cell(value)?;
    grid(rows, cols, cells)
}

fn pack_cell(data: Data) -> Result<Cell, CalcError> {
    let mut pending = vec![(&data, 1)];
    while let Some((value, depth)) = pending.pop() {
        if depth > 16 {
            return Err(CalcError::TooComplex);
        }
        if let Data::MixedGrid(grid) = value {
            pending.extend(grid.items.iter().filter_map(|cell| match cell {
                Cell::Nested(value) => Some((value.as_ref(), depth + 1)),
                _ => None,
            }));
        }
    }
    Ok(Cell::from_data(data))
}

fn grid(rows: usize, cols: usize, cells: Vec<Cell>) -> Result<Data, CalcError> {
    if cells.len() == 1 {
        // A one-element array still has an index.
        return match cells.first() {
            Some(Cell::Number(v)) => Grid::new(rows, cols, vec![*v]).map(Data::Grid),
            Some(Cell::Text(t)) => Grid::new(rows, cols, vec![t.clone()]).map(Data::TextGrid),
            _ => Grid::new(rows, cols, cells).map(Data::MixedGrid),
        };
    }
    array::cells_to_data(rows, cols, cells)
}

fn cell_at(data: &Data, i: usize) -> Option<Cell> {
    match data {
        Data::Grid(g) => g.items.get(i).copied().map(Cell::Number),
        Data::TextGrid(g) => g.items.get(i).cloned().map(Cell::Text),
        Data::MixedGrid(g) => g.items.get(i).cloned(),
        Data::One(v) if i == 0 => Some(Cell::Number(*v)),
        Data::Text(t) if i == 0 => Some(Cell::Text(t.clone())),
        _ => None,
    }
}

pub(super) fn expression(cx: &Cx<'_>, scope: &Frame<'_>, name: &str, args: &[Expr], depth: usize) -> Option<Result<Data, CalcError>> {
    Some(match name {
        "worksheetIf" => (|| {
            let [test, yes, no] = args else { return Err(invalid()) };
            let test = eval(cx, scope, test, depth + 1)?;
            let Data::One(value) = truth(cx, &test)? else { return Err(CalcError::NotCondition) };
            eval(cx, scope, if value.q.value != 0.0 { yes } else { no }, depth + 1)
        })(),
        "worksheetRange" => match args {
            [first, last] => range(cx, scope, first, None, last, depth),
            [first, second, last] => range(cx, scope, first, Some(second), last, depth),
            _ => Err(invalid()),
        },
        "worksheetEqual" | "worksheetNotEqual" => (|| {
            let [left, right] = args else { return Err(invalid()) };
            let left = eval(cx, scope, left, depth + 1)?;
            let right = eval(cx, scope, right, depth + 1)?;
            let equal = name == "worksheetEqual";
            let zero = |data: &Data| matches!(data, Data::One(value) if value.q.dim == craft_units::Dim::ONE && value.q.value == 0.0);
            for (scalar, value) in [(&left, &right), (&right, &left)] {
                if zero(scalar) {
                    match value {
                        Data::One(value) if value.q.dim != craft_units::Dim::ONE => {
                            return Ok(Data::One(super::truth_val((value.q.value == 0.0) == equal)));
                        }
                        Data::Grid(_) | Data::MixedGrid(_) | Data::TextGrid(_) => {
                            return Ok(Data::One(super::truth_val(!equal)));
                        }
                        _ => {}
                    }
                }
            }
            if matches!((&left, &right), (Data::One(_), Data::Text(_)) | (Data::Text(_), Data::One(_))) {
                return Ok(Data::One(super::truth_val(!equal)));
            }
            super::compare(cx, &left, if equal { crate::Cmp::Eq } else { crate::Cmp::Ne }, &right)
        })(),
        "worksheetCells" => (|| {
            let [Expr::Num(rows), Expr::Num(cols), items @ ..] = args else { return Err(invalid()) };
            if *rows < 1.0 || *cols < 1.0 || rows.fract() != 0.0 || cols.fract() != 0.0 {
                return Err(invalid());
            }
            array::check_size(*rows as usize, *cols as usize, items.len())?;
            let cells = eval_args(cx, scope, items, depth)?.into_iter().map(pack_cell).collect::<Result<Vec<_>, _>>()?;
            grid(*rows as usize, *cols as usize, cells)
        })(),
        "worksheetSetAt" => (|| {
            let [previous, value, at @ ..] = args else { return Err(invalid()) };
            let previous = eval(cx, scope, previous, depth + 1)?;
            let value = eval(cx, scope, value, depth + 1)?;
            write_cell(cx, scope, Some(&previous), value, at, depth)
        })(),
        "worksheetAt" => (|| {
            let [matrix, at @ ..] = args else { return Err(invalid()) };
            let value = eval(cx, scope, matrix, depth + 1)?.strict(cx.env.origin)?;
            let (rows, cols) = value.shape().ok_or_else(invalid)?;
            let at = coordinates(cx, scope, at, depth)?;
            let k = match at.as_slice() {
                [i] if (rows == 1 || cols == 1) && *i < rows.max(cols) => *i,
                [r, c] if *r < rows && *c < cols => r * cols + c,
                _ => return Err(CalcError::IndexRange("worksheet matrix index is outside the data".into())),
            };
            let cell = cell_at(&value, k).ok_or_else(invalid)?;
            cx.pass.charge(cell.copy_work())?;
            Ok(cell.data())
        })(),
        "feetInches" => (|| {
            let [length] = args else { return Err(CalcError::Arity(name.into(), 1)) };
            let value = eval(cx, scope, length, depth + 1)?;
            let Data::One(length) = value else {
                return Err(CalcError::Syntax("feetInches needs a single length".into()));
            };
            let inches = craft_units::parse_unit("in").map_err(super::unit)?;
            if length.q.dim != inches.dim {
                return Err(CalcError::Unit(craft_units::UnitError::Mismatch { left: length.q.dim, right: inches.dim }));
            }
            feet_inches(inches.from_si(length.q.value)).map(Data::Text)
        })(),
        "worksheetConcat" | "worksheetNumberText" => (|| {
            let values = eval_args(cx, scope, args, depth)?;
            let text = if name == "worksheetNumberText" {
                let [Data::One(value)] = values.as_slice() else { return Err(invalid()) };
                value.q.value.to_string()
            } else {
                let mut joined = String::new();
                for value in &values {
                    let Data::Text(text) = value else { return Err(invalid()) };
                    joined.push_str(text);
                }
                joined
            };
            if text.chars().count() > MAX_TEXT_CHARS {
                return Err(CalcError::TooLarge);
            }
            cx.pass.charge(array::text_steps(&text))?;
            Ok(Data::Text(text))
        })(),
        _ => return None,
    })
}

/// Round once to a sixteenth of an inch before splitting the result, so it carries into feet.
fn feet_inches(inches: f64) -> Result<String, CalcError> {
    let sixteenths = (inches.abs() * 16.0).round();
    if !sixteenths.is_finite() || sixteenths >= 9_007_199_254_740_992.0 {
        return Err(CalcError::TooLarge);
    }
    let total = sixteenths as u64;
    let sign = if inches < 0.0 && total != 0 { "-" } else { "" };
    let feet = total / (12 * 16);
    let whole = (total / 16) % 12;
    let remainder = total % 16;
    if remainder == 0 {
        return Ok(format!("{sign}{feet}' {whole}\""));
    }
    let divisor = 1 << remainder.trailing_zeros();
    Ok(format!("{sign}{feet}' {whole} {}/{}\"", remainder / divisor, 16 / divisor))
}
