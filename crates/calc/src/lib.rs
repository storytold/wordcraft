//! WordCraft calculation language.
//!
//! Inline calculations are written the way an engineer writes them on paper: `P := 1500 lbf`,
//! `σ := P/A_b = ? ksi`, `f(x, y) := x^2 + y`. This crate turns that text into a syntax tree
//! ([`parse`]), reports what a statement defines and uses ([`defined_name`], [`free_names`]), and
//! evaluates it against the names defined so far ([`run_stmt`], [`Env`]). [`recalc`] runs every
//! calculation in a document in dependency order and rewrites what each one shows ([`render`]).
//!
//! The parser knows nothing about units or variables: `1500 lbf` is the product of the number 1500
//! and the name `lbf`, and the evaluator decides what each name means. Source text is hostile
//! input, so the parser bounds source size, token count and expression depth and never panics.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod array;
mod array_funcs;
mod ast;
pub mod docx;
mod eval;
mod funcs;
mod number_format;
mod parse;
mod recalc;
pub mod render;
mod solve;
mod tables;
pub mod worksheet_data;

pub use ast::{Cmp, Expr, Logic, Op, Show, SolveKind, Stmt, defined_name, defined_names as names_defined_by, free_names};
pub use eval::{Env, check_name, eval_expr, run_stmt};
pub use parse::{identifier_ranges, parse};
pub use recalc::{
    NOT_CALCULATED_HERE, RecalcReport, body_regions, calc_unchanged, check_new_name, check_quote_index, check_quote_unit, data_tables,
    defined_columns, defined_names, freeze_outside_body, is_calc_object, mark_stale, recalc, sample_plots, worksheet_error_roots,
};

/// What a calculation evaluates to: one value, or a grid of values (a vector or matrix), of
/// numbers or of texts.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// A single value.
    One(craft_units::Quantity),
    /// `rows` × `cols` values, row by row. A column vector has one column.
    Grid {
        /// How many rows.
        rows: usize,
        /// How many columns.
        cols: usize,
        /// The `rows · cols` elements, row-major.
        items: Vec<craft_units::Quantity>,
    },
    /// A text.
    Text(String),
    /// `rows` × `cols` texts, row by row. A column vector has one column.
    TextGrid {
        /// How many rows.
        rows: usize,
        /// How many columns.
        cols: usize,
        /// The `rows · cols` elements, row-major.
        items: Vec<String>,
    },
    /// A grid of imported numbers and text, in row order.
    MixedGrid { rows: usize, cols: usize, items: Vec<CellValue> },
}

/// A cell in a mixed grid.
#[derive(Debug, Clone, PartialEq)]
pub enum CellValue {
    Number(craft_units::Quantity),
    Text(String),
    Nested(Box<Value>),
}

/// An error from parsing or evaluating a calculation. The message is shown to the author next to
/// the calculation, so it is written for them rather than for a programmer.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CalcError {
    /// The text isn't a valid calculation.
    #[error("{0}")]
    Syntax(String),
    /// A name that is used but never defined (`h` is told to use `hr`).
    #[error("{0} isn't defined{hint}", hint = hours_hint(.0))]
    Undefined(String),
    /// A unit or dimension error (an unknown unit `h` is told to use `hr`).
    #[error("{0}{hint}", hint = match .0 { craft_units::UnitError::UnknownUnit(u) => hours_hint(u), _ => "" })]
    Unit(#[from] craft_units::UnitError),
    /// The result is NaN or infinite.
    #[error("result isn't a finite number")]
    NonFinite,
    /// The calculation is larger or more deeply nested than the limits allow.
    #[error("too complex")]
    TooComplex,
    /// The document's calculations together need more work than one recalculation may do; this
    /// one and the rest weren't calculated.
    #[error("too complex to calculate — the document's calculations need more work than one recalculation allows")]
    TooMuchWork,
    /// A function was called with the wrong number of arguments (function name, expected count).
    #[error("{0} expects {1} argument(s)")]
    Arity(String, usize),
    /// A calculation this one depends on has an error.
    #[error("depends on a calculation with an error")]
    Upstream,
    /// The calculation was edited outside WordCraft (in Word): what it shows is no longer what its
    /// source says, so nothing may use either.
    #[error("edited in Word — restore it from the calculation or keep Word's equation")]
    EditedOutside,
    /// A calculation this one depends on (directly or through others) was edited in Word.
    #[error("depends on a calculation edited in Word — resolve it")]
    UpstreamEdited,
    /// Two calculations define the same name.
    #[error("{name} is already defined {} — choose a unique name", .page.map_or("elsewhere in the document".to_string(), |p| format!("on page {p}")))]
    Duplicate {
        /// The name defined twice.
        name: String,
        /// The page of the other definition, when known.
        page: Option<u32>,
    },
    /// A value that isn't an absolute temperature shown on an offset scale (`°C`, `°F`): holds the
    /// unit as written.
    #[error("{0} shows an absolute temperature; use Δ{0} for a difference")]
    NotAbsolute(String),
    /// A name that is a unit, function or constant.
    #[error("{0} is a unit, function or constant name — choose another name")]
    ReservedName(String),
    /// A name that is a keyword of the language (`and`, `or`, `not`, `if`, `otherwise`).
    #[error("{0} is a reserved word — choose another name")]
    Keyword(String),
    /// A condition that is a unit value or a temperature, not a comparison or a plain number.
    #[error("a condition must be a comparison or a plain number")]
    NotCondition,
    /// An absolute temperature compared with a difference or an average.
    #[error("can't compare an absolute temperature with a temperature difference — write an average as T_1 + (T_2 − T_1)/2")]
    TempCompare,
    /// Calculations that depend on each other in a cycle. Holds the names joined with `, `.
    #[error("{0} depend on each other")]
    Circular(String),
    /// Two grids combined element by element have different shapes (rows, columns).
    #[error("shapes don't match: {} rows × {} vs {} rows × {}", .left.0, .left.1, .right.0, .right.1)]
    Shape {
        /// The left operand's shape.
        left: (usize, usize),
        /// The right operand's shape.
        right: (usize, usize),
    },
    /// One element of a grid operation failed: names the element (`2`, or `[1, 2]` in a matrix).
    #[error("element {at}: {error}")]
    Element {
        /// The element, counted from the document's origin.
        at: String,
        /// What went wrong with it.
        error: Box<CalcError>,
    },
    /// An equation of a solve block whose sides don't agree in dimension: names the equation,
    /// counted from 1 in document order.
    #[error("equation {n}: {error}")]
    Equation {
        /// The equation, counted from 1.
        n: usize,
        /// What went wrong with it.
        error: Box<CalcError>,
    },
    /// A grid would hold more elements than allowed.
    #[error("more than 10,000 elements")]
    TooLarge,
    /// An index outside a grid: the message names the valid range.
    #[error("{0}")]
    IndexRange(String),
    /// A data table this calculation depends on has an error.
    #[error("depends on table data with an error")]
    UpstreamTable,
    /// No case of a piecewise definition holds and it has no `otherwise`. Holds `name = value`
    /// for up to three of the variables its conditions name (for one element of a vector, that
    /// element's values).
    #[error("no case applies{}", no_case_values(.0))]
    NoCase(Vec<String>),
    /// A text where only a number makes sense. `text` is cut to 40 characters, and `place` says
    /// what it was used in (`in arithmetic`, `with a unit`).
    #[error("\"{text}\" is text — it can't be used {place}")]
    TextUse {
        /// The text, cut to 40 characters plus `…`.
        text: String,
        /// Where it was used, as the end of the message.
        place: std::borrow::Cow<'static, str>,
    },
}

/// The values a [`CalcError::NoCase`] names, in parentheses after a space; nothing for none.
fn no_case_values(values: &[String]) -> String {
    if values.is_empty() { String::new() } else { format!(" ({})", values.join(", ")) }
}

/// The hint for `name` used as a name or a unit: `h` was the hour, which is `hr` now.
fn hours_hint(name: &str) -> &'static str {
    if name == "h" { " — for hours, use hr" } else { "" }
}
