//! Evaluating calculations: variables, functions, and unit-checked arithmetic.
//!
//! Source is hostile input, so evaluation is bounded three ways: the depth of recursion (shared
//! across user-function calls, so a deep body inside deep calls can't overflow the stack), the
//! depth of user-function calls, and the total number of expression nodes evaluated (so a chain of
//! functions that each call the previous twice can't run for hours).
//!
//! A value is one quantity or a grid of them (a vector or matrix, see [`crate::array`]). Operators
//! and built-in functions work on grids element by element, and every element counts as a step of
//! the recalculation's work. A grid is written as a literal (`[1, 2; 3, 4]`), as a range
//! (`i := 0..5`, which may only define a variable), or element by element in an indexed definition
//! (`t[i] := 2·D[i]`); an element is read by index (`D[2]`, `M[1, 0]`), counted from the
//! document's origin. The array functions (`length`, `sum`, `mmul`, `lookup`, `linterp`…, see
//! [`crate::array_funcs`]) take whole grids rather than applying element by element.

use crate::array::{self, Data, Grid, MAX_ELEMENTS};
use crate::array_funcs::{self, nearly_equal};
use crate::funcs::{self, unit};
use crate::parse::is_keyword;
use crate::solve::{Failure, Mode, solve_charged};
use crate::{CalcError, Cmp, Expr, Logic, Op, Show, SolveKind, Stmt, Value};
use craft_units::{Dim, Precision, Quantity, Scale, UnitError, UnitSystem, lookup, parse_unit};
use std::borrow::Cow;
use std::cell::Cell;
use std::collections::{BTreeMap, HashSet};

/// How deep user functions may call each other.
const MAX_CALL_DEPTH: usize = 32;
/// How deep evaluation may recurse in total. The parser caps an expression's height at 1024, so
/// this leaves room for a function body called from the bottom of another expression.
const MAX_EVAL_DEPTH: usize = 2048;
/// How many expression nodes one statement may evaluate.
const MAX_STEPS: u32 = 1_000_000;
/// How many expression nodes one recalculation of a whole document may evaluate: what keeps a
/// hostile document from costing seconds on every edit.
pub(crate) const MAX_PASS_STEPS: u64 = 5_000_000;

/// What a value stands for as a temperature, so that it is shown on an offset scale (`°C`, `°F`)
/// only when that means something: `20 Δ°C` or `T_2 - T_1` shown in `°C` would be a wrong number
/// that looks right (−253.15 °C).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Temp {
    /// Not known to be an absolute temperature: a difference, a scaled temperature, a computed
    /// one, or not a temperature at all.
    #[default]
    Plain,
    /// An absolute temperature: a `°C` or `°F` literal, or one plus or minus a difference.
    Absolute,
    /// A bare `K` or `°R` literal (`300 K`): an absolute temperature or a difference, as it is used.
    Either,
}

/// What an absolute temperature combined with another by `+` or `-` stands for: plus or minus a
/// difference it stays absolute; two absolute temperatures differ by a difference (plain) and
/// have no meaningful sum. Anything else is plain.
fn sum_temp(op: Op, a: Temp, b: Temp) -> Temp {
    match (op, a, b) {
        (_, Temp::Absolute, Temp::Absolute) => Temp::Plain,
        (Op::Add, Temp::Absolute, _) | (Op::Add, _, Temp::Absolute) | (Op::Sub, Temp::Absolute, _) => Temp::Absolute,
        _ => Temp::Plain,
    }
}

/// An evaluated value with what it stands for as a temperature.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Val {
    pub(crate) q: Quantity,
    pub(crate) t: Temp,
}

impl Val {
    pub(crate) fn plain(q: Quantity) -> Val {
        Val { q, t: Temp::Plain }
    }
}

/// How many characters of a text an error shows.
const SHOWN_TEXT_CHARS: usize = 40;

/// The error for the text `text` used where only a number makes sense, at `place` (`in
/// arithmetic`, `with a unit`): the text is shown cut to [`SHOWN_TEXT_CHARS`] characters.
pub(crate) fn text_use(text: &str, place: impl Into<Cow<'static, str>>) -> CalcError {
    CalcError::TextUse { text: cut_text(text), place: place.into() }
}

/// `text` cut to [`SHOWN_TEXT_CHARS`] characters, with `…` when it was cut.
pub(crate) fn cut_text(text: &str) -> String {
    let mut shown: String = text.chars().take(SHOWN_TEXT_CHARS).collect();
    if text.chars().nth(SHOWN_TEXT_CHARS).is_some() {
        shown.push('…');
    }
    shown
}

/// Refuse text among `operands`, which are used at `place`: the first text is named.
fn no_text(operands: &[&Data], place: &'static str) -> Result<(), CalcError> {
    match operands.iter().find_map(|d| d.text()) {
        Some(text) => Err(text_use(text, place)),
        None => Ok(()),
    }
}

/// 1 for true and 0 for false: a dimensionless, plain value.
fn truth_val(holds: bool) -> Val {
    Val::plain(Quantity::dimensionless(if holds { 1.0 } else { 0.0 }))
}

/// What a name means in an [`Env`].
#[derive(Debug, Clone, PartialEq)]
enum Entry {
    Var(Data),
    Func {
        params: Vec<String>,
        body: Expr,
    },
    /// A calculation that defines this name has an error.
    Failed,
}

/// The names defined so far: variables, functions, and names whose calculation failed.
///
/// A name has one meaning: defining it again replaces whatever it was. Whether a second
/// definition is an error is decided by whoever sees the whole document, not here.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Env {
    entries: BTreeMap<String, Entry>,
    /// `g` is standard gravity (Mathcad's convention) rather than the gram; the gram is then `gm`.
    g_is_gravity: bool,
    /// What an array's first element is numbered: 0 (Mathcad's convention) or 1.
    origin: u8,
    /// The unit system values in messages are shown in (`1.6 in is outside NPS …`).
    system: UnitSystem,
    /// The decimal places values in messages are shown with.
    precision: Precision,
}

impl Env {
    /// An empty environment (`g` is the gram until [`Env::set_g_is_gravity`]).
    pub fn new() -> Env {
        Env::default()
    }

    /// Make `g` standard gravity (Mathcad's convention), or the gram.
    pub fn set_g_is_gravity(&mut self, on: bool) {
        self.g_is_gravity = on;
    }

    /// A built-in constant as this environment reads it: `g` is gravity when the document says so.
    fn constant(&self, name: &str) -> Option<Quantity> {
        if self.g_is_gravity && name == "g" { constant("g_0") } else { constant(name) }
    }

    /// When `g` is gravity, a display unit spelled with `g` (`= ? g`, `kg/g`) would read as
    /// gravity: refused, naming `gm`.
    pub(crate) fn check_display_unit(&self, text: &str) -> Result<(), CalcError> {
        let gram = text.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '°')).any(|w| w == "g");
        if self.g_is_gravity && gram {
            return Err(CalcError::Syntax("g is gravity in this document — write gm for grams".into()));
        }
        Ok(())
    }

    pub(crate) fn display_scale(&self, text: &str) -> Result<Scale, CalcError> {
        self.check_display_unit(text)?;
        let text = text.trim();
        if text == "g_0"
            && let Some(gravity) = self.constant(text)
        {
            return Ok(Scale { dim: gravity.dim, factor: gravity.value, offset: 0.0, text: text.to_owned() });
        }
        match parse_unit(text) {
            Ok(scale) => Ok(scale),
            Err(error) => match self.data(text) {
                Some(Data::One(value)) if value.t != Temp::Absolute && value.q.value.is_finite() && value.q.value > 0.0 => {
                    Ok(Scale { dim: value.q.dim, factor: value.q.value, offset: 0.0, text: text.to_owned() })
                }
                Some(_) => Err(CalcError::Syntax(format!("{text} must be a positive scalar scale to use as a display unit"))),
                None => Err(unit(error)),
            },
        }
    }

    /// Number array elements from `origin`: 0 (Mathcad's convention, the default) or 1; anything
    /// else is 0.
    pub fn set_origin(&mut self, origin: u8) {
        self.origin = if origin == 1 { 1 } else { 0 };
    }

    /// What an array's first element is numbered: 0 or 1.
    pub(crate) fn origin(&self) -> u8 {
        self.origin
    }

    /// Show values in messages as the document shows results: in `system`'s units, to
    /// `precision` decimal places (US customary to 3 places until this is called).
    pub fn set_display(&mut self, system: UnitSystem, precision: Precision) {
        self.system = system;
        self.precision = precision.clamped();
    }

    /// Define a variable (not an absolute temperature: see [`Temp`]).
    pub fn define_var(&mut self, name: &str, q: Quantity) {
        self.entries.insert(name.to_string(), Entry::Var(Data::One(Val::plain(q))));
    }

    /// Define an array variable of `rows` × `cols` values given row by row (none of them an
    /// absolute temperature: see [`Temp`]). [`CalcError::TooLarge`] beyond 10,000 elements, and an
    /// error when `items` doesn't fill the shape; nothing is defined then.
    pub fn define_array(&mut self, name: &str, rows: usize, cols: usize, items: Vec<Quantity>) -> Result<(), CalcError> {
        array::check_size(rows, cols, items.len())?;
        let grid = Grid::new(rows, cols, items.into_iter().map(Val::plain).collect())?;
        self.entries.insert(name.to_string(), Entry::Var(Data::Grid(grid)));
        Ok(())
    }

    /// The value of the variable `name`, with what each value stands for as a temperature.
    pub(crate) fn data(&self, name: &str) -> Option<&Data> {
        match self.entries.get(name) {
            Some(Entry::Var(d)) => Some(d),
            _ => None,
        }
    }

    /// Define a function of `params`.
    pub fn define_fn(&mut self, name: &str, params: Vec<String>, body: Expr) {
        self.entries.insert(name.to_string(), Entry::Func { params, body });
    }

    /// Record that the calculation defining `name` has an error: using the name gives
    /// [`CalcError::Upstream`].
    pub fn mark_failed(&mut self, name: &str) {
        self.entries.insert(name.to_string(), Entry::Failed);
    }

    /// Every defined name (variables, functions and failed names), sorted.
    pub fn names(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }
}

/// Refuse a name that is a keyword (`and`, `or`, `not`, `if`, `otherwise`), a unit (with or without
/// an SI prefix), a built-in function, or one of the built-in constants (`π`, `pi`, `e`, `g_0`,
/// `c_0`, `R_u`); and a name with an empty subscript part (`a__b`, `x_`), which would display like a
/// different name (`a_b`, `x`).
pub fn check_name(name: &str) -> Result<(), CalcError> {
    if is_keyword(name) {
        return Err(CalcError::Keyword(name.to_string()));
    }
    if name.contains('_') && name.split('_').any(str::is_empty) {
        return Err(CalcError::Syntax(format!("{name} has an empty subscript — write a name like a_b")));
    }
    if lookup(name).is_some() || funcs::is_builtin(name) || funcs::is_solver_name(name) || is_constant(name) {
        return Err(CalcError::ReservedName(name.to_string()));
    }
    Ok(())
}

/// The built-in constants, with their units: π and e, standard gravity `g_0`, the speed of light
/// `c_0` and the molar gas constant `R_u` (exact SI values). `g` alone is the gram, so gravity
/// carries a subscript.
fn constant(name: &str) -> Option<Quantity> {
    let with = |value: f64, unit: &str| parse_unit(unit).ok().map(|s| Quantity::new(value * s.factor, s.dim));
    match name {
        "π" | "pi" => Some(Quantity::dimensionless(std::f64::consts::PI)),
        "e" => Some(Quantity::dimensionless(std::f64::consts::E)),
        "g_0" => with(9.80665, "m/s^2"),
        "c_0" => with(299_792_458.0, "m/s"),
        "R_u" => with(8.314_462_618_153_24, "J/(mol·K)"),
        _ => None,
    }
}

fn is_constant(name: &str) -> bool {
    constant(name).is_some()
}

/// Evaluate an expression against the defined names.
pub fn eval_expr(env: &Env, e: &Expr) -> Result<Value, CalcError> {
    let pass = Pass::new(u64::MAX);
    // Defensive (see `run_stmt_in`).
    eval_data(env, e, &pass).and_then(|d| d.strict(env.origin)).map(|d| d.value())
}

pub(crate) fn sample_plot(env: &mut Env, plot: &wordcraft_doc::chart::PlotSpec, pass: &Pass) -> Result<Vec<(f64, f64)>, CalcError> {
    let invalid = || CalcError::Syntax("A plot needs a variable, a numeric expression, and a numeric range or vector".into());
    if plot.expression.len() > 4096 || plot.range.len() > 4096 || plot.variable.len() > 80 {
        return Err(invalid());
    }
    if !matches!(crate::parse(&format!("{} =", plot.variable)), Ok(Stmt::Eval { expr: Expr::Name(name), .. }) if name == plot.variable) {
        return Err(invalid());
    }
    check_name(&plot.variable)?;
    let Stmt::Define { body: range, .. } = crate::parse(&format!("plotRange := {}", plot.range))? else { return Err(invalid()) };
    let Stmt::Eval { expr, .. } = crate::parse(&format!("{} =", plot.expression))? else { return Err(invalid()) };
    let data = match &range {
        Expr::Range { first, second, last } => self::range(&Cx::new(env, pass), &Frame::GLOBAL, first, second.as_deref(), last, 0)?,
        _ => eval_data(env, &range, pass)?,
    };
    let Data::Grid(points) = data else { return Err(invalid()) };
    if points.rows != 1 && points.cols != 1 || !(2..=wordcraft_doc::chart::MAX_POINTS).contains(&points.items.len()) {
        return Err(CalcError::Syntax("Use a vector or range of 2 to 4000 points".into()));
    }
    points
        .items
        .iter()
        .map(|point| {
            env.define_var(&plot.variable, point.q);
            let Data::One(y) = eval_data(env, &expr, pass)? else { return Err(invalid()) };
            Ok((point.q.value, y.q.value))
        })
        .collect()
}

/// The work a whole recalculation may do, shared by every statement it runs.
pub(crate) struct Pass {
    spent: Cell<u64>,
    limit: u64,
}

impl Pass {
    pub(crate) fn new(limit: u64) -> Pass {
        Pass { spent: Cell::new(0), limit }
    }

    /// The steps counted so far.
    #[cfg(test)]
    pub(crate) fn spent(&self) -> u64 {
        self.spent.get()
    }

    /// Count `n` steps of work: [`CalcError::TooMuchWork`], with nothing counted, once that would
    /// pass the limit.
    pub(crate) fn charge(&self, n: u64) -> Result<(), CalcError> {
        let spent = self.spent.get().saturating_add(n);
        if spent > self.limit {
            return Err(CalcError::TooMuchWork);
        }
        self.spent.set(spent);
        Ok(())
    }
}

fn eval_data(env: &Env, e: &Expr, pass: &Pass) -> Result<Data, CalcError> {
    let cx = Cx::new(env, pass);
    eval(&cx, &Frame::GLOBAL, e, 0)
}

/// Run one statement: a definition adds to `env` and an evaluation does not. Returns the value
/// when the statement shows one. A value with a failed element ([`Data::strict`]) is that
/// element's error: it is neither shown nor defined.
///
/// Nothing is defined unless the whole statement succeeds, including a `= ? unit` that does not
/// match the value's dimension. The unit itself is applied by whoever formats the value.
pub fn run_stmt(env: &mut Env, s: &Stmt) -> Result<Option<Value>, CalcError> {
    Ok(run_stmt_in(env, s, &Pass::new(u64::MAX))?.map(|d| d.value()))
}

/// [`run_stmt`], counting its work against `pass`: once the pass has done its limit, every
/// statement is [`CalcError::TooMuchWork`].
pub(crate) fn run_stmt_in(env: &mut Env, s: &Stmt, pass: &Pass) -> Result<Option<Data>, CalcError> {
    // A whole right side that is `Find(…)` would end a block; recalculation solves it there, so
    // here there is none above it.
    if let Stmt::Eval { expr: e, .. } | Stmt::Define { body: e, .. } | Stmt::DefineMany { body: e, .. } = s
        && let Some(kind) = whole_solve(e)
    {
        return Err(unresolved(kind));
    }
    match s {
        Stmt::Eval { expr, show } => {
            // Defensive: evaluated strictly, an element's failure is the error where it happens
            // (`array::build`, a piecewise's `assemble`), so no grid with failed elements gets here
            // today. Should one ever (a value stored with them), its placeholders must never be
            // shown or defined.
            let v = eval_data(env, expr, pass)?.strict(env.origin)?;
            check_show(env, show, &v)?;
            Ok(Some(v))
        }
        Stmt::Define { name, params, body, show, index } => {
            check_name(name)?;
            params.iter().try_for_each(|p| check_name(p))?;
            let is_range = matches!(body, Expr::Range { .. });
            if is_range && (index.is_some() || !params.is_empty()) {
                return Err(range_only_defines_a_variable());
            }
            if index.is_some() {
                // The parser never gives an indexed definition parameters.
                if !params.is_empty() {
                    return Err(CalcError::Syntax("an indexed definition takes one range variable, like t[i] := 2·D[i]".to_string()));
                }
            } else if !params.is_empty() {
                if show.is_some() {
                    return Err(CalcError::Syntax("a function has no single value to show — call it, like f(2) =".to_string()));
                }
                env.define_fn(name, params.clone(), body.clone());
                return Ok(None);
            }
            let v = match (index, body) {
                (Some(i), _) => indexed(env, name, i, body, pass)?,
                (None, Expr::Range { first, second, last }) => {
                    let cx = Cx::new(env, pass);
                    range(&cx, &Frame::GLOBAL, first, second.as_deref(), last, 0)?
                }
                (None, _) => eval_data(env, body, pass)?,
            }
            // Defensive, as for `Stmt::Eval` above.
            .strict(env.origin)?;
            if let Some(show) = show {
                check_show(env, show, &v)?;
            }
            let shown = show.as_ref().map(|_| v.clone());
            env.entries.insert(name.to_string(), Entry::Var(v));
            Ok(shown)
        }
        // The equations of a block are read by recalculation, which solves them where the block
        // ends; run on their own they define nothing.
        Stmt::Given | Stmt::Constraint { .. } => Ok(None),
        Stmt::DefineMany { names, body, show } => {
            names.iter().try_for_each(|n| check_name(n))?;
            let v = eval_data(env, body, pass)?.strict(env.origin)?;
            let parts = split_column(names, &v)?;
            if let Some(show) = show {
                check_show(env, show, &v)?;
            }
            // Every name or none: nothing is defined before the whole statement has succeeded.
            for (name, part) in names.iter().zip(parts) {
                env.entries.insert(name.clone(), Entry::Var(part));
            }
            Ok(show.as_ref().map(|_| v))
        }
    }
}

/// The elements of the column `v`, one value each, for the names of a multiple definition;
/// [`CalcError::Syntax`] naming what the right side is when it isn't a column of exactly that many.
fn split_column(names: &[String], v: &Data) -> Result<Vec<Data>, CalcError> {
    let list = format!("[{}]", names.join("; "));
    let wrong = |is: String| CalcError::Syntax(format!("{list} takes a column of {} values, but the right side {is}", names.len()));
    let (rows, cols) = v.shape().unwrap_or((1, 1));
    match v {
        Data::Grid(g) if cols == 1 && rows == names.len() => Ok(g.items.iter().map(|x| Data::One(*x)).collect()),
        Data::MixedGrid(g) if cols == 1 && rows == names.len() => Ok(g.items.iter().cloned().map(array::Cell::data).collect()),
        Data::TextGrid(g) if cols == 1 && rows == names.len() => Ok(g.items.iter().map(|t| Data::Text(t.clone())).collect()),
        Data::Grid(_) | Data::TextGrid(_) | Data::MixedGrid(_) if cols == 1 => Err(wrong(format!("has {rows}"))),
        Data::Grid(_) | Data::TextGrid(_) | Data::MixedGrid(_) => {
            Err(wrong(format!("is {rows} {} × {cols} {}", plural(rows, "row"), plural(cols, "column"))))
        }
        Data::One(_) | Data::Text(_) => Err(wrong("is one value".to_string())),
    }
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 { word.to_string() } else { format!("{word}s") }
}

/// A `Find` or `Minerr` that no `Given` block above it solves.
pub(crate) fn unresolved(kind: SolveKind) -> CalcError {
    CalcError::Syntax(format!("{} must end a Given block", kind.name()))
}

/// A `Find` or `Minerr` inside an expression, which can never end a block.
pub(crate) fn not_the_whole_side(kind: SolveKind) -> CalcError {
    CalcError::Syntax(format!("{} must be the whole right side of the line that ends a Given block", kind.name()))
}

/// The `Find` or `Minerr` that is the whole of `e`, if it is one.
fn whole_solve(e: &Expr) -> Option<SolveKind> {
    match e {
        Expr::Call(name, _) if name == "Find" => Some(SolveKind::Find),
        Expr::Call(name, _) if name == "Minerr" => Some(SolveKind::Minerr),
        _ => None,
    }
}

/// What a solve that found nothing is, as the error shown to the author.
fn solve_failure(failure: Failure) -> CalcError {
    match failure {
        Failure::NoSolution { worst } => {
            CalcError::Syntax(format!("no solution found near the guesses — the equations are still off by {}; try other guesses", off_by(worst)))
        }
        Failure::NotConverged { steps } => CalcError::Syntax(format!("the solve didn't converge in {steps} steps — try other guesses")),
        Failure::NonFinite => CalcError::NonFinite,
    }
}

/// How far off the equations still are, from the largest miss relative to its equation's scale at
/// the answer, `worst` (at most 2: no miss exceeds both sides): as a percentage with two
/// significant figures from 0.1% up, else as the plain relative amount.
fn off_by(worst: f64) -> String {
    if worst >= 0.001 {
        let percent = worst * 100.0;
        // Two significant figures: one decimal place less than the digits before the point leave.
        let places = (1.0 - percent.log10().floor()).clamp(0.0, 3.0) as usize;
        format!("{percent:.places$}%")
    } else {
        format!("{worst:.1e} of their size")
    }
}

/// A `= ? unit` must be a unit of the value's dimension (of every element's, for a grid).
fn check_show(env: &Env, show: &Show, v: &Data) -> Result<(), CalcError> {
    match &show.unit {
        Some(text) => check_display(env, text, v),
        None => Ok(()),
    }
}

/// Whether `v` can be shown in the unit `text`; for a grid, check every element and name
/// the first that cannot be shown. Text has no unit, and a failed element is its error.
pub(crate) fn check_display(env: &Env, text: &str, v: &Data) -> Result<(), CalcError> {
    if let Some(e) = v.failure(env.origin) {
        return Err(e);
    }
    no_text(&[v], "with a unit")?;
    let scale = env.display_scale(text)?;
    match v {
        Data::One(v) => fits(*v, &scale, text),
        Data::Grid(g) => g
            .items
            .iter()
            .enumerate()
            .try_for_each(|(k, v)| fits(*v, &scale, text).map_err(|e| array::element_error(g.rows, g.cols, k, env.origin, e))),
        // Refused above.
        Data::Text(_) | Data::TextGrid(_) | Data::MixedGrid(_) => no_text(&[v], "with a unit"),
    }
}

/// Check the dimension and require absolute temperatures for offset scales.
fn fits(v: Val, scale: &Scale, text: &str) -> Result<(), CalcError> {
    if !v.q.dim.same_ignoring_angle(scale.dim) {
        return Err(CalcError::Unit(UnitError::Mismatch { left: v.q.dim, right: scale.dim }));
    }
    if scale.offset != 0.0 && v.t == Temp::Plain {
        return Err(CalcError::NotAbsolute(text.trim().to_string()));
    }
    Ok(())
}

/// Shared state of one evaluation.
pub(crate) struct Cx<'a> {
    env: &'a Env,
    /// Expression nodes evaluated so far.
    steps: Cell<u32>,
    /// The whole recalculation's work.
    pass: &'a Pass,
    /// Above 0 while a branch is worked out over whole arrays: an element-wise operation keeps an
    /// element's error in the grid's `failed` list instead of stopping ([`eval_lenient`]).
    lenient: Cell<u32>,
}

impl<'a> Cx<'a> {
    fn new(env: &'a Env, pass: &'a Pass) -> Cx<'a> {
        Cx { env, steps: Cell::new(0), pass, lenient: Cell::new(0) }
    }

    fn lenient(&self) -> bool {
        self.lenient.get() > 0
    }
}

/// `e` evaluated with element failures kept per element (see [`array::zip`]): a branch worked out
/// over whole arrays, of which each element may pick only some. What takes a grid whole still
/// refuses a failed element ([`Data::strict`]). How lenient `cx` was is restored, even on error.
pub(crate) fn eval_lenient(cx: &Cx<'_>, fr: &Frame<'_>, e: &Expr, depth: usize) -> Result<Data, CalcError> {
    leniently(cx, || eval(cx, fr, e, depth))
}

/// `f` run with element failures kept per element, as in [`eval_lenient`]; how lenient `cx` was
/// is restored, even on error.
fn leniently<T>(cx: &Cx<'_>, f: impl FnOnce() -> Result<T, CalcError>) -> Result<T, CalcError> {
    let before = cx.lenient.get();
    cx.lenient.set(before.saturating_add(1));
    let result = f();
    cx.lenient.set(before);
    result
}

/// The parameters in scope, and how many user functions deep we are.
mod legacy_program;

pub(crate) struct Frame<'a> {
    names: &'a [String],
    values: &'a [Data],
    locals: &'a [(String, Data)],
    calls: usize,
    /// The scope this one is inside: a sum's index sees the parameters, and the indexes of the sums
    /// around it. A function's body has none, and sees only its own parameters.
    parent: Option<&'a Frame<'a>>,
}

impl Frame<'_> {
    const GLOBAL: Frame<'static> = Frame { locals: &[], names: &[], values: &[], calls: 0, parent: None };

    /// The value of a parameter or index in scope. The scopes nest no deeper than the sums inside
    /// one another, which the parser bounds.
    fn param(&self, name: &str) -> Option<&Data> {
        let own = self.names.iter().position(|n| n == name).and_then(|i| self.values.get(i));
        self.locals.iter().find(|(key, _)| key == name).map(|(_, value)| value).or(own).or_else(|| self.parent.and_then(|p| p.param(name)))
    }
}

/// Count one expression node against the statement's and the pass's limits.
fn tick(cx: &Cx<'_>) -> Result<(), CalcError> {
    let steps = cx.steps.get() + 1;
    if steps > MAX_STEPS {
        return Err(CalcError::TooComplex);
    }
    cx.steps.set(steps);
    cx.pass.charge(1)
}

fn eval(cx: &Cx<'_>, fr: &Frame<'_>, e: &Expr, depth: usize) -> Result<Data, CalcError> {
    if depth > MAX_EVAL_DEPTH {
        return Err(CalcError::TooComplex);
    }
    tick(cx)?;
    match e {
        Expr::Num(v) => finite(*v).map(|q| Data::One(Val::plain(q))),
        Expr::Text(s) => {
            cx.pass.charge(array::text_steps(s))?;
            Ok(Data::Text(s.clone()))
        }
        Expr::Name(n) => resolve(cx, fr, n),
        Expr::Quantity(v, text) => {
            let scale = parse_unit(text).map_err(unit)?;
            let t = literal_temp(text, &scale);
            from_scale(*v, &scale).map(|q| Data::One(Val { q, t }))
        }
        Expr::Unary(Op::Neg, a) => {
            refuse_offset_operand(cx, fr, &[a])?;
            let x = eval(cx, fr, a, depth + 1)?;
            no_text(&[&x], "in arithmetic")?;
            each(cx, &x, |v| v.q.neg().map(Val::plain).map_err(unit))
        }
        Expr::Unary(..) => Err(CalcError::Syntax("unsupported operator".to_string())),
        Expr::Abs(a) => {
            let x = eval(cx, fr, a, depth + 1)?;
            no_text(&[&x], "in arithmetic")?;
            each(cx, &x, |v| finite_q(Quantity::new(v.q.value.abs(), v.q.dim)).map(Val::plain))
        }
        Expr::Binary(op, a, b) => {
            if *op == Op::Mul
                && let Some(q) = offset_literal(cx, fr, a, b)
            {
                return q.map(|q| Data::One(Val { q, t: Temp::Absolute }));
            }
            // Offset literals may be combined with an interval but not scaled (`5 °F/s` would be a
            // wrong number that looks right) or combined with each other.
            if matches!(op, Op::Add | Op::Sub) {
                // An absolute temperature literal may only be combined with a temperature
                // interval literal (`100 °C + 5 K`); `100 °C + 5 °C`, `T + 5 °C` and `100 °C + T`
                // have no meaning, and a variable can't be told apart from an absolute value.
                let (abs_a, abs_b) = (is_offset_literal(cx, fr, a), is_offset_literal(cx, fr, b));
                // Subtracting an absolute temperature from anything (`5 K - 100 °C`) is meaningless.
                let sub_abs = *op == Op::Sub && abs_b;
                if sub_abs || (abs_a && !is_non_offset_temperature_literal(cx, fr, b)) || (abs_b && !is_non_offset_temperature_literal(cx, fr, a)) {
                    return Err(CalcError::Unit(UnitError::AffineTemperature));
                }
            } else {
                refuse_offset_operand(cx, fr, &[a, b])?;
            }
            let x = eval(cx, fr, a, depth + 1)?;
            let y = eval(cx, fr, b, depth + 1)?;
            no_text(&[&x, &y], "in arithmetic")?;
            // `5 K` is a temperature literal: what it stands for comes from its syntax.
            let literal = match op {
                Op::Mul => unit_literal(cx, fr, a, b).map_or(Temp::Plain, |(_, scale, name)| literal_temp(name, &scale)),
                _ => Temp::Plain,
            };
            pair(cx, &x, &y, |x, y| {
                let q = apply(*op, x.q, y.q)?;
                let t = match op {
                    Op::Add | Op::Sub => sum_temp(*op, x.t, y.t),
                    Op::Mul => literal,
                    _ => Temp::Plain,
                };
                Ok(Val { q, t })
            })
        }
        Expr::Compare { first, rest } => compare_chain(cx, fr, first, rest, depth),
        Expr::Logic(logic, a, b) => logic_op(cx, fr, *logic, a, b, depth),
        Expr::Not(a) => {
            let x = eval(cx, fr, a, depth + 1)?;
            let t = truth(cx, &x)?;
            each(cx, &t, |v| Ok(truth_val(v.q.value == 0.0)))
        }
        Expr::Call(name, args) => call(cx, fr, name, args, depth),
        Expr::Matrix { rows, cols, items } => matrix(cx, fr, *rows, *cols, items, depth),
        Expr::Index(name, at) => index(cx, fr, name, at, depth),
        Expr::Cases { cases, otherwise } => cases_expr(cx, fr, cases, otherwise.as_deref(), depth),
        Expr::Sum { index, first, second, last, body } => sum(cx, fr, index, (first, second.as_deref(), last), body, depth),
        // A range is evaluated only as the whole right side of a variable's definition.
        Expr::Range { .. } => Err(range_only_defines_a_variable()),
        Expr::Solve { kind, unknowns, equations } => {
            let equations: Vec<(&Expr, Option<&Expr>)> = equations.iter().map(|(lhs, rhs)| (lhs, Some(rhs))).collect();
            solve_block(cx, fr, *kind, unknowns, &equations, depth)
        }
    }
}

/// How many unknowns one solve may have: the solver's work per step grows with the cube of them.
const MAX_UNKNOWNS: usize = 50;

/// A vector where a solve needs single numbers: a side of an equation, or an unknown's starting
/// value taken from a function's parameter. A block function called with a vector is solved once
/// for each element when this is what stops it ([`call_user`]).
fn vector_in_solve() -> CalcError {
    CalcError::Syntax("a solve works on single numbers — a vector can't be an unknown or a side of an equation".to_string())
}

/// The value of the unknown `name` the solve starts from: the argument when the enclosing function
/// has a parameter of that name (as in Mathcad), else a document-wide definition. One number, with
/// or without a unit (an absolute temperature stays one).
fn guess(cx: &Cx<'_>, fr: &Frame<'_>, name: &str) -> Result<Val, CalcError> {
    if let Some(d) = fr.param(name) {
        return match d {
            Data::One(v) => Ok(*v),
            Data::Grid(_) | Data::TextGrid(_) | Data::MixedGrid(_) => Err(vector_in_solve()),
            Data::Text(_) => Err(CalcError::Syntax(format!("{name}'s starting value must be one number"))),
        };
    }
    match cx.env.entries.get(name) {
        Some(Entry::Var(Data::One(v))) => Ok(*v),
        Some(Entry::Var(_)) => Err(CalcError::Syntax(format!("{name}'s starting value must be one number — define it like {name} := 1"))),
        Some(Entry::Failed) => Err(CalcError::Upstream),
        Some(Entry::Func { .. }) => Err(not_a_value(name)),
        None => Err(CalcError::Syntax(format!("give {name} a starting value"))),
    }
}

/// Solve `equations` for `unknowns`, starting from the unknowns' guesses (global definitions, which
/// are left as they are): one value for one unknown, a column of them in the unknowns' order for
/// several. Each equation is a pair of sides, which must be single numbers of one dimension; a
/// right side of `None` is zero (of the left side's dimension). The equations are worked out in a
/// scope that binds the unknowns to a trial point over the caller's scope `fr`, so they may name a
/// function's parameters. The answer has the dimension of each guess.
///
/// At most [`MAX_UNKNOWNS`] unknowns. Every trial point is a step of the pass's work, besides the
/// nodes of the equations, and so is each step's linear algebra (`n · (n + m)` steps for `n`
/// unknowns and `m` equations, [`solve_charged`]). A trial
/// point where an equation is not a finite number (`√f` for negative `f`) is just a bad trial.
// Not inlined, so the recursive `eval` frame stays small (its depth is bounded by `MAX_EVAL_DEPTH`).
#[inline(never)]
fn solve_block(
    cx: &Cx<'_>,
    fr: &Frame<'_>,
    kind: SolveKind,
    unknowns: &[String],
    equations: &[(&Expr, Option<&Expr>)],
    depth: usize,
) -> Result<Data, CalcError> {
    if unknowns.len() > MAX_UNKNOWNS {
        return Err(CalcError::Syntax(format!("a solve can have at most {MAX_UNKNOWNS} unknowns")));
    }
    let guesses = unknowns.iter().map(|n| guess(cx, fr, n)).collect::<Result<Vec<Val>, _>>()?;
    let start: Vec<f64> = guesses.iter().map(|g| g.q.value).collect();
    let mode = match kind {
        SolveKind::Find => Mode::Find,
        SolveKind::Minerr => Mode::Minerr,
    };
    // The first call is at the guesses: an error there is the equations' own and is shown. Past
    // it, a trial point where an equation can't be worked out (a curve read past its last point,
    // a root of a negative number) is just a bad trial, as is one that isn't finite; only the
    // limits on work and nesting stop the solve.
    let mut at_guess = true;
    let mut sides = |x: &[f64]| -> Result<Vec<(f64, f64)>, CalcError> {
        cx.pass.charge(1)?;
        let trial: Vec<Data> = x.iter().zip(&guesses).map(|(&v, g)| Data::One(Val { q: Quantity::new(v, g.q.dim), t: g.t })).collect();
        let scope = Frame { locals: &[], names: unknowns, values: &trial, calls: fr.calls, parent: Some(fr) };
        let first = std::mem::replace(&mut at_guess, false);
        match equation_sides(cx, &scope, equations, depth) {
            Err(e @ (CalcError::TooMuchWork | CalcError::TooComplex)) => Err(e),
            Err(CalcError::NonFinite) => Ok(vec![(f64::NAN, f64::NAN); equations.len()]),
            Err(_) if !first => Ok(vec![(f64::NAN, f64::NAN); equations.len()]),
            other => other,
        }
    };
    let mut linear_algebra = |steps: u64| cx.pass.charge(steps);
    let found = solve_charged(mode, &start, &mut sides, &mut linear_algebra)?.map_err(solve_failure)?;
    let answers = found
        .x
        .iter()
        .zip(&guesses)
        .map(|(&x, g)| finite_q(Quantity::new(x, g.q.dim)).map(|q| Val { q, t: g.t }))
        .collect::<Result<Vec<Val>, _>>()?;
    match answers.as_slice() {
        [one] => Ok(Data::One(*one)),
        _ => {
            charge_elements(cx, answers.len())?;
            Grid::new(answers.len(), 1, answers).map(Data::Grid)
        }
    }
}

/// Both sides of each of `equations` at the point `fr` binds, as plain SI numbers. The sides of an
/// equation must agree in dimension. A block's equations (each with a right side) are numbered in
/// a unit error, from 1 ([`CalcError::Equation`]); `root`'s one expression isn't.
fn equation_sides(cx: &Cx<'_>, fr: &Frame<'_>, equations: &[(&Expr, Option<&Expr>)], depth: usize) -> Result<Vec<(f64, f64)>, CalcError> {
    equations
        .iter()
        .enumerate()
        .map(|(k, (lhs, rhs))| {
            let sides = || {
                let left = equation_side(cx, fr, lhs, depth)?;
                let Some(rhs) = rhs else { return Ok((left.q.value, 0.0)) };
                let right = equation_side(cx, fr, rhs, depth)?;
                left.q.sub(right.q).map_err(unit)?;
                Ok((left.q.value, right.q.value))
            };
            sides().map_err(|e| match e {
                CalcError::Unit(_) if rhs.is_some() => CalcError::Equation { n: k + 1, error: Box::new(e) },
                e => e,
            })
        })
        .collect()
}

/// One side of an equation: a single number.
fn equation_side(cx: &Cx<'_>, fr: &Frame<'_>, side: &Expr, depth: usize) -> Result<Val, CalcError> {
    match eval(cx, fr, side, depth + 1)? {
        Data::One(v) => Ok(v),
        d @ (Data::Text(_) | Data::TextGrid(_) | Data::MixedGrid(_)) => Err(text_use(d.text().unwrap_or_default(), "in an equation")),
        Data::Grid(_) => Err(vector_in_solve()),
    }
}

/// A solve block called as a function with a vector among its arguments: solved once for each
/// element, as a built-in function is applied element by element (a single value broadcast over a
/// grid, shapes that must match). An element that has no solution fails on its own: strict, the
/// error names the element; leniently (a branch of a piecewise worked out over whole arrays), the
/// element is kept as failed and the others are solved.
// Not inlined, so the recursive `eval` frame stays small (its depth is bounded by `MAX_EVAL_DEPTH`).
#[inline(never)]
fn solve_each(
    cx: &Cx<'_>,
    fr: &Frame<'_>,
    params: &[String],
    values: &[Data],
    body: &Expr,
    unknowns: usize,
    depth: usize,
) -> Result<Data, CalcError> {
    if unknowns != 1 {
        return Err(CalcError::Syntax("a Find with several unknowns takes single values — it can't be called with a vector".to_string()));
    }
    let mut shape: Option<(usize, usize)> = None;
    for s in values.iter().filter_map(Data::shape) {
        match shape {
            Some(left) if left != s => return Err(CalcError::Shape { left, right: s }),
            _ => shape = Some(s),
        }
    }
    let (rows, cols) = shape.unwrap_or((1, 1));
    let count = rows.saturating_mul(cols);
    charge_elements(cx, count)?;
    let o = cx.env.origin;
    let mut inputs: Vec<array::Failures<'_>> = values
        .iter()
        .map(|v| match v {
            Data::Grid(g) => array::Failures::of(Some(g)),
            Data::TextGrid(g) => array::Failures::of(Some(g)),
            Data::MixedGrid(g) => array::Failures::of(Some(g)),
            Data::One(_) | Data::Text(_) => array::Failures::of::<Val>(None),
        })
        .collect();
    let mut items = Vec::with_capacity(count);
    let mut failed = Vec::new();
    for k in 0..count {
        // Every input's failures are read at every element, so none falls behind.
        let broken = inputs.iter_mut().filter_map(|f| f.at(k)).next().cloned();
        let solved = match broken {
            Some(e) => Err(e),
            None => values.iter().map(|v| element_of(cx, v, k)).collect::<Result<Vec<Data>, _>>().and_then(|bound| {
                let inner = Frame { locals: &[], names: params, values: &bound, calls: fr.calls + 1, parent: None };
                match eval(cx, &inner, body, depth + 1)? {
                    Data::One(v) => Ok(v),
                    _ => Err(CalcError::Syntax("a Find with several unknowns takes single values".to_string())),
                }
            }),
        };
        items.push(match solved {
            Ok(v) => v,
            Err(e) => {
                array::keep_failure(&mut failed, k, e, cx.lenient(), |e| array::element_error(rows, cols, k, o, e))?;
                array::placeholder()
            }
        });
    }
    Grid::with_failed(rows, cols, items, failed).map(Data::Grid)
}

/// How many unknowns `body` solves for when it is a solve (a `Given` block's `Find`/`Minerr`, or a
/// `root`); `None` for any other body.
fn solve_unknowns(body: &Expr) -> Option<usize> {
    match body {
        Expr::Solve { unknowns, .. } => Some(unknowns.len()),
        Expr::Call(name, _) if name == "root" => Some(1),
        _ => None,
    }
}

/// Element `k` of `v` when it is a grid, else `v` itself (a single value broadcast): copied, at a
/// cost by its size.
fn element_of(cx: &Cx<'_>, v: &Data, k: usize) -> Result<Data, CalcError> {
    let missing = || CalcError::Syntax("a vector has fewer elements than its shape says".to_string());
    match v {
        Data::Grid(g) => g.items.get(k).map(|x| Data::One(*x)).ok_or_else(missing),
        Data::TextGrid(g) => {
            let text = g.items.get(k).ok_or_else(missing)?;
            cx.pass.charge(array::text_steps(text))?;
            Ok(Data::Text(text.clone()))
        }
        Data::MixedGrid(g) => {
            let cell = g.items.get(k).ok_or_else(missing)?;
            cx.pass.charge(cell.copy_work())?;
            Ok(cell.clone().data())
        }
        Data::One(_) | Data::Text(_) => copy(cx, v),
    }
}

fn range_only_defines_a_variable() -> CalcError {
    CalcError::Syntax("a range defines a variable, like i := 0..5".to_string())
}

/// How far a number may be from a whole number and still count as it (an index, a range's
/// element count).
const WHOLE: f64 = 1e-9;

/// The most a range's count of steps may be uncertain by, as a fraction of a step, and still be
/// counted: past it, rounding in the values could move where the range ends.
const MAX_RANGE_SLACK: f64 = 0.05;

/// A literal `[…]` of `rows` × `cols` items, row by row. Each item is one value, and they are all
/// numbers or all texts. A text item is moved in, not copied: whatever evaluated it (a literal, a
/// name, an index) already charged for it by size.
// Not inlined, so the recursive `eval` frame stays small (its depth is bounded by `MAX_EVAL_DEPTH`).
#[inline(never)]
fn matrix(cx: &Cx<'_>, fr: &Frame<'_>, rows: usize, cols: usize, items: &[Expr], depth: usize) -> Result<Data, CalcError> {
    array::check_size(rows, cols, items.len())?;
    let (mut numbers, mut texts): (Vec<Val>, Vec<String>) = (Vec::new(), Vec::new());
    let o = cx.env.origin;
    for (k, e) in items.iter().enumerate() {
        // After the first item, one of the two is empty, and that kind has no item before this one.
        match eval(cx, fr, e, depth + 1)? {
            Data::One(v) if texts.is_empty() => numbers.push(v),
            Data::Text(s) if numbers.is_empty() => texts.push(s),
            Data::One(_) => return Err(array::mixed(rows, cols, 0, k, o)),
            Data::Text(_) => return Err(array::mixed(rows, cols, k, 0, o)),
            Data::Grid(_) | Data::TextGrid(_) | Data::MixedGrid(_) => {
                return Err(CalcError::Syntax("a matrix element can't be a vector".to_string()));
            }
        }
    }
    if texts.is_empty() { Grid::new(rows, cols, numbers).map(Data::Grid) } else { Grid::new(rows, cols, texts).map(Data::TextGrid) }
}

/// The element `name[at]`: one index into a vector, or a row and a column. Only the element is
/// copied, so reading one costs one step however large the grid. The name and shape are checked
/// before the indexes are evaluated.
// Not inlined, and the lookup and messages kept in helpers that aren't, so the recursive frames
// (`eval` → `index` → `index_value` → `eval`) stay small: their depth is bounded by `MAX_EVAL_DEPTH`.
#[inline(never)]
fn index(cx: &Cx<'_>, fr: &Frame<'_>, name: &str, at: &[Expr], depth: usize) -> Result<Data, CalcError> {
    let target = indexed_grid(cx, fr, name, at.len())?;
    let mut idx = [0.0; 2];
    for (slot, e) in idx.iter_mut().zip(at) {
        *slot = index_value(cx, fr, e, depth)?;
    }
    let idx = idx.get(..at.len()).unwrap_or_default();
    match target {
        Data::Grid(g) => element(g, name, cx.env.origin, idx).map(Data::One),
        Data::TextGrid(g) => {
            let text = element(g, name, cx.env.origin, idx)?;
            cx.pass.charge(array::text_steps(&text))?;
            Ok(Data::Text(text))
        }
        Data::MixedGrid(g) => {
            let cell = element(g, name, cx.env.origin, idx)?;
            cx.pass.charge(cell.copy_work())?;
            Ok(cell.data())
        }
        Data::One(_) | Data::Text(_) => Err(not_a_vector(name)),
    }
}

/// The grid `name` holds (of numbers or of texts), when it can be read with `count` indexes and
/// none of its elements failed ([`Data::strict`]): a function's parameter may hold failed elements.
#[inline(never)]
fn indexed_grid<'a>(cx: &'a Cx<'_>, fr: &'a Frame<'_>, name: &str, count: usize) -> Result<&'a Data, CalcError> {
    let target = match fr.param(name) {
        Some(d) => d,
        None => match cx.env.entries.get(name) {
            Some(Entry::Var(d)) => d,
            Some(Entry::Failed) => return Err(CalcError::Upstream),
            Some(Entry::Func { .. }) => return Err(not_a_value(name)),
            None => {
                // A constant or unit is a single value; anything else is undefined.
                resolve(cx, fr, name)?;
                return Err(not_a_vector(name));
            }
        },
    };
    let Some((rows, cols)) = target.shape() else {
        return Err(not_a_vector(name));
    };
    index_count(rows, cols, name, count)?;
    match target.failure(cx.env.origin) {
        Some(e) => Err(e),
        None => Ok(target),
    }
}

/// Whether a grid of `rows` × `cols` (named `name`) can be read with `count` indexes: one into a
/// vector, two into any grid.
fn index_count(rows: usize, cols: usize, name: &str, count: usize) -> Result<(), CalcError> {
    match count {
        1 if rows == 1 || cols == 1 => Ok(()),
        1 => Err(CalcError::IndexRange(format!("{name} is a matrix — give a row and a column, like {name}[1, 0]"))),
        2 => Ok(()),
        _ => Err(CalcError::Syntax("an index is one number, or a row and a column, like D[2] or M[1, 0]".to_string())),
    }
}

/// Whether a quoted value's index `at` (`\i`: one index, or a row and a column) picks an element
/// of `data`, the value of `name`, counted from `origin`; the error is the one `name[…]` would
/// give in a calculation.
pub(crate) fn check_element(data: &Data, name: &str, origin: u8, at: (u32, Option<u32>)) -> Result<(), CalcError> {
    let Some((rows, cols)) = data.shape() else {
        return Err(not_a_vector(name));
    };
    let (idx, count) = match at {
        (i, None) => ([f64::from(i), 0.0], 1),
        (r, Some(c)) => ([f64::from(r), f64::from(c)], 2),
    };
    index_count(rows, cols, name, count)?;
    let at = idx.get(..count).unwrap_or_default();
    match data {
        Data::Grid(g) => element(g, name, origin, at).map(|_| ()),
        Data::TextGrid(g) => element(g, name, origin, at).map(|_| ()),
        Data::MixedGrid(g) => element(g, name, origin, at).map(|_| ()),
        Data::One(_) | Data::Text(_) => Err(not_a_vector(name)),
    }
}

/// The element of `g` (named `name`) at the whole-number indexes `at`, counted from `origin`: one
/// index into a vector, or a row and a column.
#[inline(never)]
fn element<T: Clone>(g: &Grid<T>, name: &str, origin: u8, at: &[f64]) -> Result<T, CalcError> {
    let o = origin;
    let last = |n: usize| n.saturating_sub(1).saturating_add(usize::from(o));
    let k = match *at {
        [i] => slot(i, o, g.items.len())
            .ok_or_else(|| CalcError::IndexRange(format!("index {} is outside {name} ({o} to {})", index_text(i), last(g.items.len()))))?,
        [r, c] => match (slot(r, o, g.rows), slot(c, o, g.cols)) {
            (Some(r), Some(c)) => r.saturating_mul(g.cols).saturating_add(c),
            _ => {
                return Err(CalcError::IndexRange(format!(
                    "index [{}, {}] is outside {name} (rows {o} to {}, columns {o} to {})",
                    index_text(r),
                    index_text(c),
                    last(g.rows),
                    last(g.cols)
                )));
            }
        },
        _ => return Err(CalcError::Syntax("an index is one number, or a row and a column, like D[2] or M[1, 0]".to_string())),
    };
    g.items.get(k).cloned().ok_or_else(|| CalcError::IndexRange(format!("{name} has no element {k}")))
}

fn not_a_vector(name: &str) -> CalcError {
    CalcError::IndexRange(format!("{name} isn't a vector"))
}

/// An index: a plain number within [`WHOLE`] of a whole number, which it is rounded to.
// Not inlined, so the recursive `eval` frame stays small (its depth is bounded by `MAX_EVAL_DEPTH`).
#[inline(never)]
fn index_value(cx: &Cx<'_>, fr: &Frame<'_>, e: &Expr, depth: usize) -> Result<f64, CalcError> {
    match eval(cx, fr, e, depth + 1)? {
        Data::One(v) if v.q.dim == Dim::ONE => whole(v.q.value),
        _ => None,
    }
    .ok_or_else(not_whole)
}

#[cold]
fn not_whole() -> CalcError {
    CalcError::Syntax("an index is a whole number, like D[2]".to_string())
}

/// `v` rounded to the whole number it is within [`WHOLE`] of.
fn whole(v: f64) -> Option<f64> {
    let r = v.round();
    (v.is_finite() && (v - r).abs() <= WHOLE).then_some(r)
}

/// Where the whole number `i`, counted from `origin`, falls among `len` places; `None` outside.
fn slot(i: f64, origin: u8, len: usize) -> Option<usize> {
    let k = i - f64::from(origin);
    // `len` is at most `MAX_ELEMENTS`, so the comparison is exact and the cast can't truncate.
    (k >= 0.0 && k < len as f64).then_some(k as usize)
}

/// A whole number as the author would write it: `3`, `-1`, and `1e300` for one too large to spell.
fn index_text(i: f64) -> String {
    if i.abs() < 1e15 { format!("{}", i as i64) } else { format!("{i:e}") }
}

/// `a < b ≤ c …`: the `and` of its links (`a < b and b ≤ c …`), each operand evaluated at most
/// once. As with `and`, a link's right operand is worked out only where every earlier link holds: a
/// single false stops the chain, and for a grid the rest is worked out over whole arrays
/// ([`eval_lenient`]) but taken only by the elements still true, so an element already false
/// ignores a later operand's failures ([`logic_per_element`]). The result is 1 or 0 (a grid of them
/// when an operand is a grid).
// Not inlined, so the recursive `eval` frame stays small (its depth is bounded by `MAX_EVAL_DEPTH`).
#[inline(never)]
fn compare_chain(cx: &Cx<'_>, fr: &Frame<'_>, first: &Expr, rest: &[(Cmp, Expr)], depth: usize) -> Result<Data, CalcError> {
    let mut prev = eval(cx, fr, first, depth + 1)?;
    let mut holds: Option<Data> = None;
    for (cmp, e) in rest {
        let link = |prev: &Data| -> Result<(Data, Data), CalcError> {
            let next = eval(cx, fr, e, depth + 1)?;
            let link = compare(cx, prev, *cmp, &next)?;
            Ok((next, link))
        };
        let (next, all) = match holds {
            // A false so far decides the chain.
            Some(Data::One(v)) if v.q.value == 0.0 => return Ok(Data::One(v)),
            Some(Data::Grid(g)) => {
                let (rows, cols) = (g.rows, g.cols);
                let (mut slots, open) = undecided(&g, false);
                if open.is_empty() {
                    return Ok(Data::Grid(g));
                }
                let (next, link) = leniently(cx, || link(&prev))?;
                same_shape((rows, cols), &link)?;
                take(cx, &link, open, &mut slots)?;
                (next, assemble(cx, rows, cols, slots)?)
            }
            // None so far, or true: the chain is this link.
            Some(Data::One(_)) | None => link(&prev)?,
            // A comparison gives numbers.
            Some(Data::Text(_) | Data::TextGrid(_) | Data::MixedGrid(_)) => return Err(CalcError::NotCondition),
        };
        holds = Some(all);
        prev = next;
    }
    holds.ok_or_else(|| CalcError::Syntax("a comparison needs two values".to_string()))
}

/// `x cmp y`, element by element (a single value broadcast over a grid): 1 where it holds, else 0.
// Not inlined, so the recursive `eval` frame stays small (its depth is bounded by `MAX_EVAL_DEPTH`).
#[inline(never)]
fn compare(cx: &Cx<'_>, x: &Data, cmp: Cmp, y: &Data) -> Result<Data, CalcError> {
    let (xt, yt) = (x.text(), y.text());
    match (xt, yt) {
        (None, None) => pair(cx, x, y, |a, b| compare_values(a, cmp, b).map(truth_val)),
        (Some(_), Some(_)) => compare_texts(cx, x, cmp, y),
        (Some(text), None) | (None, Some(text)) => Err(CalcError::Syntax(format!("can't compare \"{}\" with a number", cut_text(text)))),
    }
}

/// Whether `a cmp b`, for numbers: the dimensions must agree, and the SI values are compared with
/// the tolerance `==` has. An absolute temperature is compared with another absolute temperature or
/// a `K`/`°R` value, but not with a difference or an average.
fn compare_values(a: Val, cmp: Cmp, b: Val) -> Result<bool, CalcError> {
    let (l, r) = (a.q, b.q);
    if !l.dim.same_ignoring_angle(r.dim) {
        return Err(CalcError::Unit(UnitError::Mismatch { left: l.dim, right: r.dim }));
    }
    let plain_temperature = |v: Val| v.t == Temp::Plain && v.q.dim == Dim::TEMPERATURE;
    if (a.t == Temp::Absolute && plain_temperature(b)) || (b.t == Temp::Absolute && plain_temperature(a)) {
        return Err(CalcError::TempCompare);
    }
    let same = nearly_equal(l.value, r.value);
    let less = l.value < r.value;
    Ok(match cmp {
        Cmp::Eq => same,
        Cmp::Ne => !same,
        Cmp::Lt => less && !same,
        Cmp::Le => less || same,
        Cmp::Gt => !less && !same,
        Cmp::Ge => !less || same,
    })
}

/// `x cmp y` for texts, which are only equal or not: after [`array::text_key`] on both. Each
/// element counts as a step of work, and so does the text's size. An element that failed in either
/// side fails in the result, as in [`array::zip`].
#[inline(never)]
fn compare_texts(cx: &Cx<'_>, x: &Data, cmp: Cmp, y: &Data) -> Result<Data, CalcError> {
    let equal = match cmp {
        Cmp::Eq => true,
        Cmp::Ne => false,
        _ => return Err(text_use(x.text().unwrap_or_default(), format!("with {}", cmp.symbol()))),
    };
    let shape = array::result_shape(x, y)?;
    if let Some((rows, cols)) = shape {
        charge_elements(cx, rows.saturating_mul(cols))?;
    }
    let (xs, ys) = (texts_of(x), texts_of(y));
    let n = xs.len().max(ys.len());
    // A single text is broadcast over the other's elements.
    fn at(s: &[String], k: usize) -> &str {
        s.get(if s.len() == 1 { 0 } else { k }).map_or("", String::as_str)
    }
    let (mut fx, mut fy) = (array::Failures::of(text_grid(x)), array::Failures::of(text_grid(y)));
    let (rows, cols) = shape.unwrap_or((1, 1));
    let mut items = Vec::with_capacity(n);
    let mut failed = Vec::new();
    for k in 0..n {
        // Both sides' failures are read at every element, in order; the left side's wins.
        let (ex, ey) = (fx.at(k), fy.at(k));
        if let Some(e) = ex.or(ey) {
            array::keep_failure(&mut failed, k, e.clone(), cx.lenient(), |e| array::element_error(rows, cols, k, cx.env.origin, e))?;
            items.push(array::placeholder());
            continue;
        }
        let (a, b) = (at(xs, k), at(ys, k));
        cx.pass.charge(array::text_steps(a).saturating_add(array::text_steps(b)))?;
        items.push(truth_val((array::text_key(a) == array::text_key(b)) == equal));
    }
    match shape {
        Some((rows, cols)) => Grid::with_failed(rows, cols, items, failed).map(Data::Grid),
        None => items.pop().map(Data::One).ok_or(CalcError::NonFinite),
    }
}

/// The grid of texts `d` holds, if any.
fn text_grid(d: &Data) -> Option<&Grid<String>> {
    match d {
        Data::TextGrid(g) => Some(g),
        Data::One(_) | Data::Grid(_) | Data::Text(_) | Data::MixedGrid(_) => None,
    }
}

/// The texts of a text or a text grid (none for numbers).
fn texts_of(d: &Data) -> &[String] {
    match d {
        Data::Text(s) => std::slice::from_ref(s),
        Data::TextGrid(g) => &g.items,
        Data::One(_) | Data::Grid(_) | Data::MixedGrid(_) => &[],
    }
}

/// `a and b` or `a or b` as 1 or 0. A left value that decides the result (false for `and`, true for
/// `or`) leaves the right side unevaluated. A grid on the left decides element by element
/// ([`logic_per_element`]).
// Not inlined, so the recursive `eval` frame stays small (its depth is bounded by `MAX_EVAL_DEPTH`).
#[inline(never)]
fn logic_op(cx: &Cx<'_>, fr: &Frame<'_>, logic: Logic, a: &Expr, b: &Expr, depth: usize) -> Result<Data, CalcError> {
    let x = eval(cx, fr, a, depth + 1)?;
    let left = truth(cx, &x)?;
    let decides = logic == Logic::Or;
    match &left {
        Data::One(v) if (v.q.value != 0.0) == decides => return Ok(left),
        Data::Grid(g) => return logic_per_element(cx, fr, decides, g, b, depth),
        Data::One(_) | Data::Text(_) | Data::TextGrid(_) | Data::MixedGrid(_) => {}
    }
    let y = eval(cx, fr, b, depth + 1)?;
    let right = truth(cx, &y)?;
    pair(cx, &left, &right, |p, q| {
        let (p, q) = (p.q.value != 0.0, q.q.value != 0.0);
        Ok(truth_val(if decides { p || q } else { p && q }))
    })
}

/// `left and b` or `left or b` for a grid `left` (already 1s and 0s) that decides each element
/// where it is `decides` (true for `or`, false for `and`): elsewhere the element is the truth of
/// the right side's element, which is worked out over whole arrays but only when some element is
/// left to it ([`eval_lenient`]), so a right side that fails at an element the left decides
/// doesn't fail it. The right side is a single value (broadcast) or of `left`'s shape.
#[inline(never)]
fn logic_per_element(cx: &Cx<'_>, fr: &Frame<'_>, decides: bool, left: &Grid, b: &Expr, depth: usize) -> Result<Data, CalcError> {
    let (rows, cols) = (left.rows, left.cols);
    let (mut slots, open) = undecided(left, decides);
    if !open.is_empty() {
        let right = leniently(cx, || {
            let y = eval(cx, fr, b, depth + 1)?;
            truth(cx, &y)
        })?;
        same_shape((rows, cols), &right)?;
        take(cx, &right, open, &mut slots)?;
    }
    assemble(cx, rows, cols, slots)
}

/// The slots of `left and …` or `left or …` for a grid `left` of 1s and 0s that decides each
/// element where it is `decides` (true for `or`, false for `and`), and the elements it leaves open
/// to the right side, in order. An element that failed in `left` keeps its failure.
fn undecided(left: &Grid, decides: bool) -> (Vec<Slot>, Vec<usize>) {
    let mut failures = array::Failures::of(Some(left));
    let slots: Vec<Slot> = left
        .items
        .iter()
        .enumerate()
        .map(|(k, v)| match failures.at(k) {
            Some(e) => Slot::Failed(e.clone()),
            None if (v.q.value != 0.0) == decides => Slot::Num(truth_val(decides)),
            None => Slot::Open,
        })
        .collect();
    let open = slots.iter().enumerate().filter_map(|(k, s)| matches!(s, Slot::Open).then_some(k)).collect();
    (slots, open)
}

/// What one element of a result worked out per element is, as it is worked out.
enum Slot {
    /// Nothing has decided it yet.
    Open,
    /// It takes the branch with this index (a piecewise's `otherwise` is the one after its cases).
    Case(usize),
    /// A number.
    Num(Val),
    /// A text.
    Text(String),
    /// Its error (not naming the element).
    Failed(CalcError),
    /// No case of a piecewise holds for it: [`CalcError::NoCase`], its message not yet written.
    NoCase,
}

/// The piecewise `cases` ([`piecewise`]).
// Not inlined, so the recursive `eval` frame stays small (its depth is bounded by `MAX_EVAL_DEPTH`).
#[inline(never)]
fn cases_expr(cx: &Cx<'_>, fr: &Frame<'_>, cases: &[(Expr, Expr)], otherwise: Option<&Expr>, depth: usize) -> Result<Data, CalcError> {
    let cases: Vec<(&Expr, &Expr)> = cases.iter().map(|(value, condition)| (value, condition)).collect();
    piecewise(cx, fr, &cases, otherwise, depth)
}

/// A piecewise: the value of the first case (`(value, condition)`) whose condition holds, else
/// `otherwise`'s, else [`CalcError::NoCase`]. Only the value picked is evaluated. Conditions are
/// tried in order while they are single values; the first that is a grid decides per element
/// from there on ([`per_element`]). The conditions here are evaluated as leniently as the caller.
// Not inlined, so the recursive `eval` frame stays small (its depth is bounded by `MAX_EVAL_DEPTH`).
#[inline(never)]
fn piecewise(cx: &Cx<'_>, fr: &Frame<'_>, cases: &[(&Expr, &Expr)], otherwise: Option<&Expr>, depth: usize) -> Result<Data, CalcError> {
    for (i, (value, condition)) in cases.iter().enumerate() {
        let c = truth(cx, &eval(cx, fr, condition, depth + 1)?)?;
        match c {
            Data::One(v) if v.q.value != 0.0 => return eval(cx, fr, value, depth + 1),
            Data::One(_) => {}
            Data::Grid(g) => return per_element(cx, fr, cases, i, &g, otherwise, depth),
            // `truth` refuses text.
            Data::Text(_) | Data::TextGrid(_) | Data::MixedGrid(_) => return Err(CalcError::NotCondition),
        }
    }
    match otherwise {
        Some(e) => eval(cx, fr, e, depth + 1),
        None => Err(no_case(cx, &case_vars(cx.env, fr, cases, None), None)),
    }
}

/// A piecewise from its case `first` on, whose condition is the grid `decider` (1s and 0s): each
/// element takes the first case that holds for it. Later conditions are tried only while some
/// element is undecided; elements left undecided take `otherwise`, or fail with
/// [`CalcError::NoCase`]. Every condition after `first` and every value picked by some element is
/// worked out once over whole arrays ([`eval_lenient`]), so `sum(x)` in a branch is the sum of
/// all of `x`, and element k of the result is element k of the value it picks (or that value, when
/// it is a single one). An element fails only when a condition it reached, or the value it picked,
/// failed there; a condition or a picked value that fails as a whole fails the result, and so does
/// one that is neither a single value nor of `decider`'s shape.
///
/// The elements' failures are the first one's error, naming it, unless the caller is lenient: then
/// they are kept in the result's `failed` list.
#[inline(never)]
fn per_element(
    cx: &Cx<'_>,
    fr: &Frame<'_>,
    cases: &[(&Expr, &Expr)],
    first: usize,
    decider: &Grid,
    otherwise: Option<&Expr>,
    depth: usize,
) -> Result<Data, CalcError> {
    let (rows, cols) = (decider.rows, decider.cols);
    let mut slots: Vec<Slot> = std::iter::repeat_with(|| Slot::Open).take(decider.items.len()).collect();
    let mut open = slots.len();
    decide(&mut slots, &mut open, first, decider);
    for (j, (_, condition)) in cases.iter().enumerate().skip(first.saturating_add(1)) {
        if open == 0 {
            break;
        }
        let c = leniently(cx, || {
            let c = eval(cx, fr, condition, depth + 1)?;
            truth(cx, &c)
        })?;
        same_shape((rows, cols), &c)?;
        match c {
            Data::One(v) if v.q.value != 0.0 => {
                slots.iter_mut().filter(|s| matches!(s, Slot::Open)).for_each(|s| *s = Slot::Case(j));
                open = 0;
            }
            Data::One(_) => {}
            Data::Grid(g) => decide(&mut slots, &mut open, j, &g),
            // `truth` refuses text.
            Data::Text(_) | Data::TextGrid(_) | Data::MixedGrid(_) => return Err(CalcError::NotCondition),
        }
    }
    if open > 0 {
        let other = otherwise.map(|_| cases.len());
        for s in slots.iter_mut().filter(|s| matches!(s, Slot::Open)) {
            *s = other.map_or(Slot::NoCase, Slot::Case);
        }
    }
    // Each value some element picks, worked out once; its elements taken in order.
    let mut picks: Vec<(usize, usize)> =
        slots.iter().enumerate().filter_map(|(k, s)| if let Slot::Case(j) = s { Some((*j, k)) } else { None }).collect();
    picks.sort_unstable();
    for group in picks.chunk_by(|a, b| a.0 == b.0) {
        let Some(&(j, _)) = group.first() else { continue };
        let Some(value) = cases.get(j).map(|(value, _)| *value).or(otherwise) else { continue };
        let v = eval_lenient(cx, fr, value, depth + 1)?;
        same_shape((rows, cols), &v)?;
        take(cx, &v, group.iter().map(|&(_, k)| k), &mut slots)?;
    }
    write_no_cases(cx, fr, cases, (rows, cols), &mut slots)?;
    assemble(cx, rows, cols, slots)
}

/// Write the message of each element of `slots` that no case holds for ([`no_case`]), naming the
/// conditions' variables at that element. A strict caller is told of the first failed element
/// only, so only that one is written when it is one of these; for a lenient caller, which keeps
/// them all, each is written. Writing one counts as work (a step, and one per 32 bytes), since a
/// branch that no element picks may hold thousands of them.
fn write_no_cases(cx: &Cx<'_>, fr: &Frame<'_>, cases: &[(&Expr, &Expr)], shape: (usize, usize), slots: &mut [Slot]) -> Result<(), CalcError> {
    let failed = |s: &Slot| matches!(s, Slot::Failed(_) | Slot::NoCase);
    let lenient = cx.lenient();
    let first = slots.iter().position(failed);
    if !first.is_some_and(|k| lenient || matches!(slots.get(k), Some(Slot::NoCase))) {
        return Ok(());
    }
    let vars = case_vars(cx.env, fr, cases, Some(shape));
    for (k, s) in slots.iter_mut().enumerate().filter(|(k, _)| lenient || Some(*k) == first) {
        if matches!(s, Slot::NoCase) {
            let e = no_case(cx, &vars, Some(k));
            let CalcError::NoCase(values) = &e else { continue };
            let bytes: usize = values.iter().map(String::len).sum();
            cx.pass.charge(u64::try_from(bytes / 32).unwrap_or(u64::MAX).saturating_add(1))?;
            *s = Slot::Failed(e);
        }
    }
    Ok(())
}

/// Mark the elements still open in `slots` that the condition `c` (1s and 0s, of their shape)
/// decides: those it holds for take case `j`, and those it failed at fail; `open` counts down.
fn decide(slots: &mut [Slot], open: &mut usize, j: usize, c: &Grid) {
    let mut failures = array::Failures::of(Some(c));
    for (k, (slot, v)) in slots.iter_mut().zip(&c.items).enumerate() {
        // Read at every element, so the cursor never falls behind.
        let failed = failures.at(k);
        if !matches!(slot, Slot::Open) {
            continue;
        }
        if let Some(e) = failed {
            *slot = Slot::Failed(e.clone());
        } else if v.q.value != 0.0 {
            *slot = Slot::Case(j);
        } else {
            continue;
        }
        *open = open.saturating_sub(1);
    }
}

/// [`CalcError::Shape`] unless `d` is a single value or of `shape`.
fn same_shape(shape: (usize, usize), d: &Data) -> Result<(), CalcError> {
    match d.shape() {
        Some(other) if other != shape => Err(CalcError::Shape { left: shape, right: other }),
        _ => Ok(()),
    }
}

/// Set each element `k` of `ks` (in increasing order) in `slots` to element `k` of `v`, or to `v`
/// itself when it is a single value; where `v` failed, to its failure. A text taken is copied, at
/// a cost by its size.
fn take(cx: &Cx<'_>, v: &Data, ks: impl IntoIterator<Item = usize>, slots: &mut [Slot]) -> Result<(), CalcError> {
    let (mut numbers, mut texts) = (array::Failures::of(grid_of(v)), array::Failures::of(text_grid(v)));
    let text = |t: &String| -> Result<Slot, CalcError> {
        cx.pass.charge(array::text_steps(t))?;
        Ok(Slot::Text(t.clone()))
    };
    for k in ks {
        // Only reached with `v` of the slots' shape, so every element exists.
        let missing = || Slot::Failed(CalcError::Shape { left: (slots.len(), 1), right: v.shape().unwrap_or((1, 1)) });
        let slot = match v {
            Data::One(x) => Slot::Num(*x),
            Data::Text(t) => text(t)?,
            Data::Grid(g) => match numbers.at(k) {
                Some(e) => Slot::Failed(e.clone()),
                None => g.items.get(k).map_or_else(missing, |x| Slot::Num(*x)),
            },
            Data::TextGrid(g) => match texts.at(k) {
                Some(e) => Slot::Failed(e.clone()),
                None => match g.items.get(k) {
                    Some(t) => text(t)?,
                    None => missing(),
                },
            },
            Data::MixedGrid(g) => match g.items.get(k) {
                Some(array::Cell::Number(value)) => Slot::Num(*value),
                Some(array::Cell::Text(value)) => text(value)?,
                Some(array::Cell::Nested(_)) => return Err(CalcError::Syntax("a conditional needs scalar matrix cells".into())),
                None => missing(),
            },
        };
        if let Some(s) = slots.get_mut(k) {
            *s = slot;
        }
    }
    Ok(())
}

/// The `rows` × `cols` result of elements worked out one by one: numbers (each keeping what it
/// stands for as a temperature) or texts, never a mix ([`array::mixed`]); a grid whose every
/// element failed is of numbers. Failed elements are the first one's error, naming it, unless `cx`
/// is lenient: then they are kept in the grid's `failed` list.
fn assemble(cx: &Cx<'_>, rows: usize, cols: usize, slots: Vec<Slot>) -> Result<Data, CalcError> {
    charge_elements(cx, slots.len())?;
    let first_text = slots.iter().position(|s| matches!(s, Slot::Text(_)));
    let first_number = slots.iter().position(|s| matches!(s, Slot::Num(_)));
    let o = cx.env.origin;
    if let (Some(t), Some(n)) = (first_text, first_number) {
        return Err(array::mixed(rows, cols, t, n, o));
    }
    // Every element is decided and taken before this, and a no-case message written for every
    // one that can be reported; were one not, it has no value.
    let failure = |s: Slot| match s {
        Slot::Failed(e) => e,
        Slot::NoCase | Slot::Open | Slot::Case(_) | Slot::Num(_) | Slot::Text(_) => CalcError::NoCase(Vec::new()),
    };
    let mut failed: Vec<(usize, CalcError)> = Vec::new();
    let data = if first_text.is_some() {
        let mut items = Vec::with_capacity(slots.len());
        for (k, s) in slots.into_iter().enumerate() {
            items.push(match s {
                Slot::Text(t) => t,
                s => {
                    failed.push((k, failure(s)));
                    String::new()
                }
            });
        }
        Elements::Texts(items)
    } else {
        let mut items = Vec::with_capacity(slots.len());
        for (k, s) in slots.into_iter().enumerate() {
            items.push(match s {
                Slot::Num(v) => v,
                s => {
                    failed.push((k, failure(s)));
                    array::placeholder()
                }
            });
        }
        Elements::Numbers(items)
    };
    if !cx.lenient() && !failed.is_empty() {
        let (k, e) = failed.swap_remove(0);
        return Err(array::element_error(rows, cols, k, o, e));
    }
    match data {
        Elements::Numbers(items) => Grid::with_failed(rows, cols, items, failed).map(Data::Grid),
        Elements::Texts(items) => Grid::with_failed(rows, cols, items, failed).map(Data::TextGrid),
    }
}

/// A grid's elements before it is built: numbers or texts.
enum Elements {
    Numbers(Vec<Val>),
    Texts(Vec<String>),
}

/// The grid of numbers `d` holds, if any.
fn grid_of(d: &Data) -> Option<&Grid> {
    match d {
        Data::Grid(g) => Some(g),
        Data::One(_) | Data::Text(_) | Data::TextGrid(_) | Data::MixedGrid(_) => None,
    }
}

/// How many variables a [`CalcError::NoCase`] names at most.
const MAX_NO_CASE_VALUES: usize = 3;

/// The variables (parameters and defined values, not units, functions or constants) that a
/// piecewise's conditions name, in order of first use, for [`no_case`]: at most
/// [`MAX_NO_CASE_VALUES`], and only those that can be shown, which are single values and texts and,
/// for a piecewise decided per element over `shape`, grids of that shape.
fn case_vars<'a>(env: &'a Env, fr: &'a Frame<'_>, cases: &[(&'a Expr, &'a Expr)], shape: Option<(usize, usize)>) -> Vec<(&'a str, &'a Data)> {
    let mut out = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    // An explicit stack keeps this safe on trees built by hand, deeper than the parser allows.
    let mut stack: Vec<&Expr> = cases.iter().rev().map(|(_, condition)| *condition).collect();
    while let Some(e) = stack.pop() {
        if out.len() >= MAX_NO_CASE_VALUES {
            break;
        }
        match e {
            Expr::Name(n) => {
                if !seen.insert(n.as_str()) {
                    continue;
                }
                let Some(d) = fr.param(n).or_else(|| env.data(n)) else { continue };
                let shown = match d {
                    Data::One(_) | Data::Text(_) => true,
                    Data::Grid(_) | Data::TextGrid(_) | Data::MixedGrid(_) => shape.is_some() && d.shape() == shape,
                };
                if shown {
                    out.push((n.as_str(), d));
                }
            }
            Expr::Num(_) | Expr::Text(_) | Expr::Quantity(..) => {}
            Expr::Unary(_, a) | Expr::Abs(a) | Expr::Not(a) => stack.push(a),
            Expr::Binary(_, a, b) | Expr::Logic(_, a, b) => {
                stack.push(b);
                stack.push(a);
            }
            Expr::Compare { first, rest } => {
                stack.extend(rest.iter().rev().map(|(_, e)| e));
                stack.push(first);
            }
            Expr::Call(_, items) | Expr::Index(_, items) | Expr::Matrix { items, .. } => stack.extend(items.iter().rev()),
            Expr::Range { first, second, last } => {
                stack.push(last);
                stack.extend(second.as_deref());
                stack.push(first);
            }
            Expr::Cases { cases, otherwise } => {
                stack.extend(otherwise.as_deref());
                for (value, condition) in cases.iter().rev() {
                    stack.push(condition);
                    stack.push(value);
                }
            }
            Expr::Sum { first, second, last, body, .. } => {
                stack.push(body);
                stack.push(last);
                stack.extend(second.as_deref());
                stack.push(first);
            }
            Expr::Solve { equations, .. } => stack.extend(equations.iter().rev().flat_map(|(lhs, rhs)| [rhs, lhs])),
        }
    }
    out
}

/// [`CalcError::NoCase`] naming `vars` (from [`case_vars`]) as `name = value`: for an element
/// `at` of a piecewise decided per element, a grid's element there (none when it failed there).
fn no_case(cx: &Cx<'_>, vars: &[(&str, &Data)], at: Option<usize>) -> CalcError {
    let failed_at = |failed: &[(usize, CalcError)], k: usize| failed.binary_search_by_key(&k, |(j, _)| *j).is_ok();
    let values = vars
        .iter()
        .filter_map(|(name, d)| {
            let shown = match (d, at) {
                (Data::One(v), _) => shown_value(cx, *v),
                (Data::Text(t), _) => format!("\"{}\"", cut_text(t)),
                (Data::Grid(g), Some(k)) if !failed_at(&g.failed, k) => shown_value(cx, *g.items.get(k)?),
                (Data::TextGrid(g), Some(k)) if !failed_at(&g.failed, k) => format!("\"{}\"", cut_text(g.items.get(k)?)),
                (Data::Grid(_) | Data::TextGrid(_) | Data::MixedGrid(_), _) => return None,
            };
            Some(format!("{name} = {shown}"))
        })
        .collect();
    CalcError::NoCase(values)
}

/// `v` as the document shows values (`1.6 in`), and an absolute temperature on the document's
/// offset scale (`1000 °F`, `540 °C`).
fn shown_value(cx: &Cx<'_>, v: Val) -> String {
    let (system, precision) = (cx.env.system, cx.env.precision);
    if v.t == Temp::Absolute {
        let unit = if system == UnitSystem::Si { "°C" } else { "°F" };
        if let Ok(scale) = parse_unit(unit)
            && let Ok(f) = craft_units::format(v.q, Some(&scale), system, precision)
        {
            return format!("{} {}", crate::render::text_number(&f.number), f.unit);
        }
    }
    let acx = array_funcs::Ctx { origin: cx.env.origin, pass: cx.pass, system, precision, lenient: false };
    array_funcs::shown(v.q, &acx)
}

/// A condition checked: the plain numbers of `d` (nonzero is true) as 1 and 0, a grid for a grid.
/// A value with a unit is [`CalcError::NotCondition`], and a text can't be one.
pub(crate) fn truth(cx: &Cx<'_>, d: &Data) -> Result<Data, CalcError> {
    if let Some(text) = d.text() {
        return Err(text_use(text, "as a condition"));
    }
    each(cx, d, |v| if v.q.dim == Dim::ONE { Ok(truth_val(v.q.value != 0.0)) } else { Err(CalcError::NotCondition) })
}

/// The range `first..last` or `first, second..last`, both ends included: a column of the values
/// from `first` stepping by `second − first` (by 1 towards `last` when there is no second value)
/// for as long as they don't pass `last`.
///
/// The ends are worked out in the scope `fr`: a definition's are global, a sum's may use the
/// parameters and the indexes around it.
fn range(cx: &Cx<'_>, fr: &Frame<'_>, first: &Expr, second: Option<&Expr>, last: &Expr, depth: usize) -> Result<Data, CalcError> {
    let (a, b, z) = range_ends(cx, fr, first, second, last, depth)?;
    range_values(cx, a, b, z).map(Data::Grid)
}

/// The values at the ends of a range, each a single value: the first, the second when it is given
/// and the last.
fn range_ends(
    cx: &Cx<'_>,
    fr: &Frame<'_>,
    first: &Expr,
    second: Option<&Expr>,
    last: &Expr,
    depth: usize,
) -> Result<(Val, Option<Val>, Val), CalcError> {
    tick(cx)?;
    let end = |e: &Expr| match eval(cx, fr, e, depth + 1)? {
        Data::One(v) => Ok(v),
        Data::Grid(_) => Err(CalcError::Syntax("a range's values are single values, like 0..5".to_string())),
        d @ (Data::Text(_) | Data::TextGrid(_) | Data::MixedGrid(_)) => Err(text_use(d.text().unwrap_or_default(), "in a range")),
    };
    let a = end(first)?;
    let b = second.map(end).transpose()?;
    let z = end(last)?;
    Ok((a, b, z))
}

/// The column of values of the range from `a` to `z`, stepping by `b − a` when there is a second
/// value `b`.
fn range_values(cx: &Cx<'_>, a: Val, b: Option<Val>, z: Val) -> Result<Grid, CalcError> {
    let span = z.q.sub(a.q).map_err(unit)?;
    let step = match b {
        Some(b) => b.q.sub(a.q).map_err(unit)?,
        None if a.q.dim == Dim::ONE && z.q.dim == Dim::ONE => Quantity::dimensionless(if span.value < 0.0 { -1.0 } else { 1.0 }),
        // Counting by 1 in SI units (1 m from 0 in) would be a wrong number that looks right.
        None => return Err(CalcError::Syntax("a range with units needs a step, like 0 in, 0.5 in..2 in".to_string())),
    };
    if step.value == 0.0 {
        return Err(CalcError::Syntax("a range's step is zero — its second value must differ from the first, like 1, 1.5..3".to_string()));
    }
    let ratio = span.value / step.value;
    if ratio.is_nan() {
        return Err(CalcError::NonFinite);
    }
    // More steps than a grid holds elements (an infinite count too) is too large, however the
    // count rounds.
    if ratio > MAX_ELEMENTS as f64 {
        return Err(CalcError::TooLarge);
    }
    // The values carry rounding error in proportion to their size, and so does a step taken from
    // two of them: `100000.1 − 100000` is 0.1 within about 1.5e-11, so the 20 steps to 100002
    // count 19.9999999997. How far the count may be from a whole number and still be it: a few
    // ulps of the largest value over the step (the span's error), plus, for a step that is the
    // difference of two values, the step's own error once for each step.
    let scale = a.q.value.abs().max(z.q.value.abs()).max(b.map_or(0.0, |b| b.q.value.abs()));
    let per_step = if b.is_some() { ratio.abs() } else { 0.0 };
    let tol = WHOLE.max(8.0 * f64::EPSILON * scale / step.value.abs() * (1.0 + per_step));
    if ratio.is_infinite() || ratio < -tol {
        return Err(CalcError::Syntax("a range's step goes away from its last value — like 1, 1.5..3 or 3, 2.5..1".to_string()));
    }
    // A step so small next to the values that their rounding could move the end by a fair part
    // of a step: where the range ends would be a guess.
    if tol.is_nan() || tol > MAX_RANGE_SLACK {
        return Err(CalcError::Syntax("a range's step is too small next to its values to tell exactly where it ends".to_string()));
    }
    let nearest = ratio.round();
    let lands = (ratio - nearest).abs() <= tol;
    let steps = if lands { nearest.max(0.0) } else { ratio.floor() };
    // At most 10,000 steps (checked above), so the cast below can't truncate.
    let count = steps + 1.0;
    if count > MAX_ELEMENTS as f64 {
        return Err(CalcError::TooLarge);
    }
    let n = count as usize;
    charge_elements(cx, n)?;
    let t = between(a.t, z.t);
    let mut values =
        (0..n).map(|k| a.q.add(Quantity::new(k as f64 * step.value, step.dim)).map(|q| Val { q, t }).map_err(unit)).collect::<Result<Vec<_>, _>>()?;
    // A step that lands on the last value ends on it exactly, not on the step's rounding added up
    // (100000, 100000.1..100002 ends at 100002, not 100002.00000000003). Counting by 1 is exact.
    if lands
        && b.is_some()
        && n > 1
        && let Some(end) = values.last_mut()
    {
        end.q = z.q;
    }
    Grid::new(n, 1, values)
}

/// The sum `∑(index = first..last) body`: `body` worked out once for each value of the range, with
/// the index bound to it, and the terms added element by element (so their units must agree and
/// their shapes match). The index is the sum's own: it can't be a unit, function or constant, nor
/// a name defined in the document, a parameter, or the index of a sum around this one. A range
/// that ends below its first value with no step is empty, and an empty sum is an error, never 0.
/// Each term costs the work of its body, and the range itself is a grid, so at most 10,000 terms.
// Not inlined, so the recursive `eval` frame stays small (its depth is bounded by `MAX_EVAL_DEPTH`).
#[inline(never)]
fn sum(cx: &Cx<'_>, fr: &Frame<'_>, index: &str, ends: (&Expr, Option<&Expr>, &Expr), body: &Expr, depth: usize) -> Result<Data, CalcError> {
    check_name(index)?;
    if fr.param(index).is_some() || cx.env.entries.contains_key(index) {
        return Err(CalcError::Syntax(format!("the sum's index {index} is already defined — choose another name")));
    }
    let empty = || CalcError::Syntax("the sum's range is empty — its last value is below its first".to_string());
    let (first, second, last) = ends;
    let (a, b, z) = range_ends(cx, fr, first, second, last, depth)?;
    if b.is_none() && z.q.sub(a.q).is_ok_and(|span| span.value < 0.0) {
        return Err(empty());
    }
    let values = range_values(cx, a, b, z)?;
    let names = [index.to_string()];
    let mut total: Option<Data> = None;
    for v in &values.items {
        let bound = [Data::One(*v)];
        let frame = Frame { locals: &[], names: &names, values: &bound, calls: fr.calls, parent: Some(fr) };
        let term = eval(cx, &frame, body, depth + 1)?;
        no_text(&[&term], "in a sum")?;
        total = Some(match total {
            None => term,
            Some(so_far) => pair(cx, &so_far, &term, |x, y| Ok(Val { q: apply(Op::Add, x.q, y.q)?, t: sum_temp(Op::Add, x.t, y.t) }))?,
        });
    }
    total.ok_or_else(empty)
}

/// What values between two temperatures stand for: absolute when either end is, a bare `K`
/// temperature when both ends are, and plain otherwise.
fn between(a: Temp, b: Temp) -> Temp {
    match (a, b) {
        (Temp::Plain, _) | (_, Temp::Plain) => Temp::Plain,
        (Temp::Absolute, _) | (_, Temp::Absolute) => Temp::Absolute,
        _ => Temp::Either,
    }
}

/// The indexed definition `name[i] := body`: `body` evaluated once for each element of the range
/// variable `i`, with `i` bound to that element, setting element `i` of `name`. Every element from
/// the origin up to the largest index must be set; a body that is a vector, or an `i` that isn't
/// a vector of whole numbers, is an error. The body is worked out with element failures kept
/// ([`eval_lenient`]), so a body that is a vector is that error even when one of its elements
/// failed, and never an error naming an element of the body inside one naming an element of `name`.
fn indexed(env: &Env, name: &str, i: &str, body: &Expr, pass: &Pass) -> Result<Data, CalcError> {
    let range = match env.entries.get(i) {
        Some(Entry::Var(d)) => d,
        Some(Entry::Failed) => return Err(CalcError::Upstream),
        Some(Entry::Func { .. }) => return Err(not_a_value(i)),
        None => return Err(CalcError::Undefined(i.to_string())),
    };
    let not_a_range = || CalcError::Syntax(format!("{i} must be a range of whole numbers, like {i} := 0..5"));
    let g = match range {
        Data::Grid(g) if g.rows == 1 || g.cols == 1 => g,
        _ => return Err(not_a_range()),
    };
    let o = env.origin;
    // Where each element of `i` puts its value, checked before anything is built.
    let mut places = Vec::with_capacity(g.items.len());
    let mut size = 0.0_f64;
    for v in &g.items {
        let w = (v.q.dim == Dim::ONE).then(|| whole(v.q.value)).flatten().ok_or_else(not_a_range)?;
        let k = w - f64::from(o);
        if k < 0.0 {
            return Err(CalcError::IndexRange(format!("index {} is outside {name} — its elements start at {o}", index_text(w))));
        }
        size = size.max(k + 1.0);
        places.push(k);
    }
    if size > MAX_ELEMENTS as f64 {
        return Err(CalcError::TooLarge);
    }
    // At most `MAX_ELEMENTS`, so the casts are exact.
    let size = size as usize;
    let cx = Cx::new(env, pass);
    charge_elements(&cx, size)?;
    let places: Vec<usize> = places.into_iter().map(|k| k as usize).collect();
    let mut set = vec![false; size];
    for &k in &places {
        if let Some(s) = set.get_mut(k) {
            *s = true;
        }
    }
    let unset =
        |k: usize| CalcError::Syntax(format!("{name}[{}] isn't set by {i} — the range must cover every element", k.saturating_add(usize::from(o))));
    if let Some(k) = set.iter().position(|s| !s) {
        return Err(unset(k));
    }
    let names = [i.to_string()];
    let mut values: Vec<Option<Data>> = vec![None; size];
    for (&k, v) in places.iter().zip(&g.items) {
        let bound = [Data::One(*v)];
        let frame = Frame { locals: &[], names: &names, values: &bound, calls: 0, parent: None };
        let value = match eval_lenient(&cx, &frame, body, 0).map_err(|e| array::element_error(size, 1, k, o, e))? {
            value @ (Data::One(_) | Data::Text(_)) => value,
            Data::Grid(_) | Data::TextGrid(_) | Data::MixedGrid(_) => {
                return Err(CalcError::Syntax(format!("{name}[{i}] is one element — its right side can't be a vector")));
            }
        };
        // Defensive: a single value has no failed elements, so this never fails today; it stays
        // so a value with failures could never be stored. The vector check above comes first, so
        // a vector body with a failed element is reported as a vector.
        let value = value.strict(o).map_err(|e| array::element_error(size, 1, k, o, e))?;
        if let Some(slot) = values.get_mut(k) {
            *slot = Some(value);
        }
    }
    let values = values.into_iter().enumerate().map(|(k, v)| v.ok_or_else(|| unset(k))).collect::<Result<Vec<_>, _>>()?;
    collect_elements(values, o)
}

/// The vector of `values`, one element each: of numbers, or of texts. A mix is an error naming the
/// first element of the kind the first isn't.
fn collect_elements(values: Vec<Data>, origin: u8) -> Result<Data, CalcError> {
    let rows = values.len();
    let (mut numbers, mut texts): (Vec<Val>, Vec<String>) = (Vec::new(), Vec::new());
    for (k, v) in values.into_iter().enumerate() {
        match v {
            Data::One(v) if texts.is_empty() => numbers.push(v),
            Data::Text(s) if numbers.is_empty() => texts.push(s),
            Data::One(_) => return Err(array::mixed(rows, 1, 0, k, origin)),
            Data::Text(_) => return Err(array::mixed(rows, 1, k, 0, origin)),
            Data::Grid(_) | Data::TextGrid(_) | Data::MixedGrid(_) => return Err(CalcError::Syntax("an element is a single value".to_string())),
        }
    }
    if texts.is_empty() { Grid::new(rows, 1, numbers).map(Data::Grid) } else { Grid::new(rows, 1, texts).map(Data::TextGrid) }
}

/// `f` over every element of `x`, each counted as a step of the pass's work.
// Not inlined, so the recursive `eval` frame stays small (its depth is bounded by `MAX_EVAL_DEPTH`).
#[inline(never)]
fn each(cx: &Cx<'_>, x: &Data, f: impl FnMut(Val) -> Result<Val, CalcError>) -> Result<Data, CalcError> {
    if let Some((rows, cols)) = x.shape() {
        charge_elements(cx, rows.saturating_mul(cols))?;
    }
    array::map(x, cx.env.origin, cx.lenient(), f)
}

/// `f` over `x` and `y` element by element (a single value broadcast over a grid), each element
/// counted as a step of the pass's work.
// Not inlined, so the recursive `eval` frame stays small (its depth is bounded by `MAX_EVAL_DEPTH`).
#[inline(never)]
fn pair(cx: &Cx<'_>, x: &Data, y: &Data, f: impl FnMut(Val, Val) -> Result<Val, CalcError>) -> Result<Data, CalcError> {
    if let Some((rows, cols)) = array::result_shape(x, y)? {
        charge_elements(cx, rows.saturating_mul(cols))?;
    }
    array::zip(x, y, cx.env.origin, cx.lenient(), f)
}

/// Count `n` elements' work against the pass.
fn charge_elements(cx: &Cx<'_>, n: usize) -> Result<(), CalcError> {
    cx.pass.charge(u64::try_from(n).unwrap_or(u64::MAX))
}

/// What a literal in the unit `name` stands for as a temperature: absolute on an offset scale,
/// either in a plain temperature unit (`K`, `°R`), a difference in `Δ°C`/`Δ°F`.
fn literal_temp(name: &str, scale: &Scale) -> Temp {
    if scale.dim != Dim::TEMPERATURE {
        Temp::Plain
    } else if scale.offset != 0.0 {
        Temp::Absolute
    } else if name.trim_start().starts_with(['Δ', '∆']) {
        Temp::Plain
    } else {
        Temp::Either
    }
}

fn finite(v: f64) -> Result<Quantity, CalcError> {
    finite_q(Quantity::dimensionless(v))
}

fn finite_q(q: Quantity) -> Result<Quantity, CalcError> {
    if q.value.is_finite() { Ok(q) } else { Err(CalcError::NonFinite) }
}

/// The SI quantity for `v` in `scale`; an offset unit gives the absolute temperature.
fn from_scale(v: f64, scale: &Scale) -> Result<Quantity, CalcError> {
    finite_q(Quantity::new(scale.to_si(v), scale.dim))
}

// Not inlined, so the recursive `eval` frame stays small (its depth is bounded by `MAX_EVAL_DEPTH`).
#[inline(never)]
fn apply(op: Op, x: Quantity, y: Quantity) -> Result<Quantity, CalcError> {
    match op {
        Op::Add => x.add(y),
        Op::Sub => x.sub(y),
        Op::Mul => x.mul(y),
        Op::Div => x.div(y),
        Op::Pow => {
            if !y.dim.is_dimensionless() {
                return Err(CalcError::Unit(UnitError::NotDimensionless(y.dim)));
            }
            x.powf(y.value)
        }
        Op::Neg => return Err(CalcError::Syntax("unsupported operator".to_string())),
    }
    .map_err(unit)
}

/// `number unit` where the unit name is not shadowed by a parameter, variable or constant: the
/// number, the unit's scale and its name.
fn unit_literal<'e>(cx: &Cx<'_>, fr: &Frame<'_>, num: &Expr, unit_name: &'e Expr) -> Option<(f64, Scale, &'e str)> {
    let v = match num {
        Expr::Num(v) => *v,
        Expr::Unary(Op::Neg, inner) => match &**inner {
            Expr::Num(v) => -*v,
            _ => return None,
        },
        _ => return None,
    };
    let Expr::Name(n) = unit_name else {
        return None;
    };
    if fr.param(n).is_some() || cx.env.entries.contains_key(n.as_str()) || cx.env.constant(n).is_some() {
        return None;
    }
    Some((v, lookup(n)?, n))
}

/// `5 °F`: a number times an offset unit that no variable or constant shadows is an absolute
/// temperature. Anything else involving an offset unit is refused in [`resolve`].
fn offset_literal(cx: &Cx<'_>, fr: &Frame<'_>, num: &Expr, unit_name: &Expr) -> Option<Result<Quantity, CalcError>> {
    let (v, scale, _) = unit_literal(cx, fr, num, unit_name)?;
    (scale.offset != 0.0).then(|| from_scale(v, &scale))
}

/// Whether `e` is `number unit` for a temperature unit with no offset (`5 K`, `5 °R`, `5 Δ°C`):
/// a temperature interval.
fn is_non_offset_temperature_literal(cx: &Cx<'_>, fr: &Frame<'_>, e: &Expr) -> bool {
    matches!(e, Expr::Binary(Op::Mul, a, b)
        if unit_literal(cx, fr, a, b).is_some_and(|(_, s, _)| s.offset == 0.0 && s.dim == Dim::TEMPERATURE))
}

/// Whether `e` is exactly a `number offset-unit` literal.
fn is_offset_literal(cx: &Cx<'_>, fr: &Frame<'_>, e: &Expr) -> bool {
    matches!(e, Expr::Binary(Op::Mul, a, b) if offset_literal(cx, fr, a, b).is_some())
}

/// An absolute temperature can't be multiplied, divided or raised to a power.
fn refuse_offset_operand(cx: &Cx<'_>, fr: &Frame<'_>, operands: &[&Expr]) -> Result<(), CalcError> {
    if operands.iter().any(|e| is_offset_literal(cx, fr, e)) { Err(CalcError::Unit(UnitError::AffineTemperature)) } else { Ok(()) }
}

fn not_a_value(name: &str) -> CalcError {
    CalcError::Syntax(format!("{name} is a function — call it like {name}(x)"))
}

/// A name as a value: parameter, variable, constant, unit.
fn resolve(cx: &Cx<'_>, fr: &Frame<'_>, name: &str) -> Result<Data, CalcError> {
    if let Some(v) = fr.param(name) {
        return copy(cx, v);
    }
    match cx.env.entries.get(name) {
        Some(Entry::Var(v)) => return copy(cx, v),
        Some(Entry::Failed) => return Err(CalcError::Upstream),
        Some(Entry::Func { .. }) => return Err(not_a_value(name)),
        None => {}
    }
    if let Some(q) = cx.env.constant(name) {
        return Ok(Data::One(Val::plain(q)));
    }
    // A unit wins over a built-in of the same spelling: `5 min` is minutes, `min(1, 2)` a call.
    if let Some(scale) = lookup(name) {
        if scale.offset != 0.0 {
            return Err(CalcError::Unit(UnitError::AffineTemperature));
        }
        return finite_q(Quantity::new(scale.factor, scale.dim)).map(|q| Data::One(Val::plain(q)));
    }
    if funcs::is_builtin(name) {
        return Err(not_a_value(name));
    }
    Err(CalcError::Undefined(name.to_string()))
}

/// A copy of a name's value. Copying is work ([`Data::copy_work`]): each element of a grid counts
/// as a step, and each 32 bytes of text, so a hostile calculation can't copy large grids or long
/// texts for free.
fn copy(cx: &Cx<'_>, v: &Data) -> Result<Data, CalcError> {
    cx.pass.charge(v.copy_work())?;
    Ok(v.clone())
}

// Not inlined, so the recursive `eval` frame stays small (its depth is bounded by `MAX_EVAL_DEPTH`).
#[inline(never)]
fn call(cx: &Cx<'_>, fr: &Frame<'_>, name: &str, args: &[Expr], depth: usize) -> Result<Data, CalcError> {
    match name {
        "worksheetAdd" | "worksheetSubtract" => {
            let [left, right] = args else { return Err(CalcError::Arity(name.to_owned(), 2)) };
            let left = eval(cx, fr, left, depth + 1)?;
            let right = eval(cx, fr, right, depth + 1)?;
            let op = if name == "worksheetAdd" { Op::Add } else { Op::Sub };
            return pair(cx, &left, &right, |mut a, mut b| {
                if a.q.dim == Dim::ONE && a.q.value == 0.0 && b.q.dim != Dim::TEMPERATURE {
                    a.q.dim = b.q.dim;
                }
                if b.q.dim == Dim::ONE && b.q.value == 0.0 && a.q.dim != Dim::TEMPERATURE {
                    b.q.dim = a.q.dim;
                }
                apply(op, a.q, b.q).map(|q| Val { q, t: sum_temp(op, a.t, b.t) })
            });
        }
        "worksheetTemperature" => {
            let [magnitude, Expr::Text(unit)] = args else {
                return Err(CalcError::Arity(name.to_owned(), 2));
            };
            let scale = parse_unit(unit).map_err(funcs::unit)?;
            if scale.dim != Dim::TEMPERATURE || scale.offset == 0.0 {
                return Err(CalcError::Unit(UnitError::AffineTemperature));
            }
            let magnitude = eval(cx, fr, magnitude, depth + 1)?;
            return each(cx, &magnitude, |v| {
                if v.q.dim != Dim::ONE {
                    return Err(CalcError::Unit(UnitError::NotDimensionless(v.q.dim)));
                }
                from_scale(v.q.value, &scale).map(|q| Val { q, t: Temp::Absolute })
            });
        }
        "worksheetTemperatureDifference" => {
            let [left, right] = args else { return Err(CalcError::Arity(name.to_owned(), 2)) };
            let left = eval(cx, fr, left, depth + 1)?;
            let right = eval(cx, fr, right, depth + 1)?;
            return pair(cx, &left, &right, |a, b| {
                if a.t != Temp::Absolute || b.t != Temp::Absolute {
                    return Err(CalcError::Unit(UnitError::AffineTemperature));
                }
                apply(Op::Sub, a.q, b.q).map(Val::plain)
            });
        }
        _ => {}
    }
    if name == "worksheetProgram" {
        return legacy_program::run(cx, fr, args, depth);
    }
    if let Some(result) = legacy_program::expression(cx, fr, name, args, depth) {
        return result;
    }
    // `if(c, a, b)` is the piecewise `a if c, b otherwise`: only the branch picked is evaluated.
    if name == "if" {
        let [condition, then, otherwise] = args else {
            return Err(CalcError::Arity("if".to_string(), 3));
        };
        return piecewise(cx, fr, &[(then, condition)], Some(otherwise), depth);
    }
    // `root(f(x), x)` finds the `x` that makes the expression zero, from the document's guess for `x`.
    if name == "root" {
        let [expr, Expr::Name(unknown)] = args else {
            return if args.len() == 2 {
                Err(CalcError::Syntax("root's second argument is the unknown, like root(x^2 − 2, x)".to_string()))
            } else {
                Err(CalcError::Arity("root".to_string(), 2))
            };
        };
        return solve_block(cx, fr, SolveKind::Find, std::slice::from_ref(unknown), &[(expr, None)], depth);
    }
    // A solve that recalculation didn't resolve. Run on its own, a statement whose whole right side
    // is the call has no block above it ([`run_stmt_in`]); one inside an expression never ends one.
    match name {
        "Find" => return Err(not_the_whole_side(SolveKind::Find)),
        "Minerr" => return Err(not_the_whole_side(SolveKind::Minerr)),
        _ => {}
    }
    // Resolve what is being called before evaluating the arguments, so a missing name is reported
    // as missing rather than after unrelated errors in its arguments.
    let user = if fr.param(name).is_some() { None } else { cx.env.entries.get(name) };
    match user {
        Some(Entry::Failed) => return Err(CalcError::Upstream),
        Some(Entry::Var(..)) => return Err(not_callable(name)),
        Some(Entry::Func { params, body }) => return call_user(cx, fr, name, params, body, args, depth),
        None => {}
    }
    if fr.param(name).is_some() {
        return Err(not_callable(name));
    }
    if !funcs::is_builtin(name) {
        return if is_constant(name) || lookup(name).is_some() || funcs::is_solver_name(name) {
            Err(not_callable(name))
        } else {
            Err(CalcError::Undefined(name.to_string()))
        };
    }
    let exprs = args;
    let args = eval_args(cx, fr, exprs, depth)?;
    let env = cx.env;
    let acx = array_funcs::Ctx { origin: env.origin, pass: cx.pass, system: env.system, precision: env.precision, lenient: cx.lenient() };
    if let Some(result) = array_funcs::call_array(name, &args, exprs, &acx) {
        return result;
    }
    let values: Vec<Val> = args.iter().filter_map(|a| if let Data::One(v) = a { Some(*v) } else { None }).collect();
    if values.len() < args.len() {
        return call_each(cx, name, &args);
    }
    let quantities: Vec<Quantity> = values.iter().map(|v| v.q).collect();
    let q = funcs::call(name, &quantities).unwrap_or_else(|| Err(CalcError::Undefined(name.to_string())))?;
    // The smaller or larger of absolute temperatures is one of them; any other result is plain.
    let t = if matches!(name, "min" | "max") { picked(&values) } else { Temp::Plain };
    Ok(Data::One(Val { q, t }))
}

/// What a value picked from `values` (their smallest, largest, or one between them) stands for
/// as a temperature: absolute when they are all temperatures and one is absolute, a bare `K`
/// temperature when they all are, and plain otherwise (and for no values).
pub(crate) fn picked(values: &[Val]) -> Temp {
    if values.is_empty() || values.iter().any(|v| v.t == Temp::Plain) {
        Temp::Plain
    } else if values.iter().any(|v| v.t == Temp::Absolute) {
        Temp::Absolute
    } else {
        Temp::Either
    }
}

/// A scalar built-in called with a grid among its arguments: applied element by element over
/// every argument, a single value broadcast. The result of each is not an absolute temperature.
/// (`min`, `max` and the array functions take whole grids, in [`array_funcs::call_array`].)
fn call_each(cx: &Cx<'_>, name: &str, args: &[Data]) -> Result<Data, CalcError> {
    if let Some(text) = args.iter().find_map(Data::text) {
        return Err(text_use(text, format!("in {name}")));
    }
    let builtin = |qs: &[Quantity]| funcs::call(name, qs).unwrap_or_else(|| Err(CalcError::Undefined(name.to_string()))).map(Val::plain);
    // The number of arguments is checked once, before any element: not "element 0: sqrt expects 1".
    let firsts: Vec<Quantity> = args.iter().filter_map(first).map(|v| v.q).collect();
    if let Err(e @ CalcError::Arity(..)) = builtin(&firsts) {
        return Err(e);
    }
    match args {
        [x] => each(cx, x, |x| builtin(&[x.q])),
        [x, y] => pair(cx, x, y, |x, y| builtin(&[x.q, y.q])),
        // Every scalar built-in but `min` and `max` (which never get here) takes one or two
        // arguments, so the check above reported any other count.
        _ => Err(CalcError::Arity(name.to_string(), 2)),
    }
}

/// A single value, or a grid's first element.
fn first(d: &Data) -> Option<Val> {
    match d {
        Data::One(v) => Some(*v),
        Data::Grid(g) => g.items.first().copied(),
        Data::Text(_) | Data::TextGrid(_) | Data::MixedGrid(_) => None,
    }
}

fn not_callable(name: &str) -> CalcError {
    // Mathcad writes the secant `sec(x)`; here `sec` is the second.
    if name == "sec" {
        return CalcError::Syntax("sec isn't a function — for the secant, use secant(x)".to_string());
    }
    CalcError::Syntax(format!("{name} isn't a function — it can't be called with ( )"))
}

fn eval_args(cx: &Cx<'_>, fr: &Frame<'_>, args: &[Expr], depth: usize) -> Result<Vec<Data>, CalcError> {
    args.iter().map(|a| eval(cx, fr, a, depth + 1)).collect()
}

/// Call a user function in a scope holding only its own parameters (and the global names).
fn call_user(cx: &Cx<'_>, fr: &Frame<'_>, name: &str, params: &[String], body: &Expr, args: &[Expr], depth: usize) -> Result<Data, CalcError> {
    if args.len() != params.len() {
        return Err(CalcError::Arity(name.to_string(), params.len()));
    }
    if fr.calls >= MAX_CALL_DEPTH {
        return Err(CalcError::TooComplex);
    }
    let values = eval_args(cx, fr, args, depth)?;
    let inner = Frame { locals: &[], names: params, values: &values, calls: fr.calls + 1, parent: None };
    let whole = eval(cx, &inner, body, depth + 1);
    // A solve called with a vector is first tried with the arguments whole (an equation may take a
    // vector as a vector: `sum(c)`, `c[1]`); only when a vector stands where a single number must
    // (`x^2 == a`) is it solved once for each element.
    if let Err(e) = &whole
        && *e == vector_in_solve()
        && values.iter().any(|v| v.shape().is_some())
        && let Some(unknowns) = solve_unknowns(body)
    {
        return solve_each(cx, fr, params, &values, body, unknowns, depth);
    }
    whole
}

#[cfg(test)]
mod tests;
